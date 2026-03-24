//! Session admission control — enforces session limits and hot-connect.
//!
//! Implements:
//! - §Requirement: Session Limits (line 355): resident (default 16, max 256),
//!   guest (default 64, max 1024), total (default 80, max 1280).
//! - §Requirement: Hot-Connect (line 346): agents connecting while scene active
//!   receive a full scene snapshot. Snapshot delivery is on network threads and
//!   MUST NOT block the compositor thread.
//!
//! # Session limit enforcement
//!
//! When the resident limit is reached, new connections receive `RESOURCE_EXHAUSTED`
//! with a structured body: current capacity + estimated wait hint (spec line 361).
//!
//! # Hot-connect
//!
//! The `HotConnectSnapshot` type carries a serialised scene JSON snapshot that
//! the session server (network thread) delivers in `SessionEstablished`. The
//! compositor thread serialises the snapshot under the scene lock, hands it off
//! to the session, and then continues rendering without being blocked.

use std::collections::HashMap;
use std::time::Instant;

use crate::session::{AgentKind, SessionEnvelope};
use tze_hud_scene::types::{ResourceBudget, SceneId};

// ─── Session limit constants ──────────────────────────────────────────────────

/// Default maximum resident agent sessions (spec line 356).
pub const DEFAULT_MAX_RESIDENT_SESSIONS: usize = 16;

/// Absolute maximum resident agent sessions (spec line 356).
pub const HARD_MAX_RESIDENT_SESSIONS: usize = 256;

/// Default maximum guest agent sessions (spec line 357).
pub const DEFAULT_MAX_GUEST_SESSIONS: usize = 64;

/// Absolute maximum guest agent sessions (spec line 357).
pub const HARD_MAX_GUEST_SESSIONS: usize = 1024;

/// Default maximum total concurrent sessions (spec line 358).
pub const DEFAULT_MAX_TOTAL_SESSIONS: usize = 80;

/// Absolute maximum total concurrent sessions (spec line 358).
pub const HARD_MAX_TOTAL_SESSIONS: usize = 1280;

// ─── Session limits configuration ────────────────────────────────────────────

/// Runtime-configurable session limit thresholds.
///
/// All values are capped to their absolute maximums at construction.
#[derive(Clone, Debug)]
pub struct SessionLimits {
    /// Maximum resident agent sessions (default: 16, hard max: 256).
    pub max_resident: usize,
    /// Maximum guest agent sessions (default: 64, hard max: 1024).
    pub max_guest: usize,
    /// Maximum total concurrent sessions (default: 80, hard max: 1280).
    pub max_total: usize,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_resident: DEFAULT_MAX_RESIDENT_SESSIONS,
            max_guest: DEFAULT_MAX_GUEST_SESSIONS,
            max_total: DEFAULT_MAX_TOTAL_SESSIONS,
        }
    }
}

impl SessionLimits {
    /// Construct `SessionLimits`, capping each value to its absolute maximum.
    pub fn new(max_resident: usize, max_guest: usize, max_total: usize) -> Self {
        Self {
            max_resident: max_resident.min(HARD_MAX_RESIDENT_SESSIONS),
            max_guest: max_guest.min(HARD_MAX_GUEST_SESSIONS),
            max_total: max_total.min(HARD_MAX_TOTAL_SESSIONS),
        }
    }
}

// ─── Admission outcome ────────────────────────────────────────────────────────

/// Outcome of an admission attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum AdmissionOutcome {
    /// Session admitted. Contains the session_id of the newly admitted session.
    Admitted(String), // session_id
    /// Session rejected because a session limit was reached.
    ResourceExhausted(ResourceExhaustedDetail),
    /// Session rejected because a session with the same ID is already admitted.
    DuplicateSessionId,
}

/// Structured detail returned when a session is rejected due to capacity limits.
///
/// The session server delivers this to the connecting agent as
/// `SessionError { code: "RESOURCE_EXHAUSTED", ... }` with a JSON-encoded
/// body (spec line 361).
#[derive(Debug, PartialEq, Eq)]
pub struct ResourceExhaustedDetail {
    /// Which limit was hit.
    pub limit_kind: LimitKind,
    /// Current count of sessions in the relevant pool.
    pub current: usize,
    /// The configured limit for the relevant pool.
    pub limit: usize,
    /// Opaque hint for the client (e.g. "wait ~Ns and retry").
    /// May be empty if no estimate is possible.
    pub estimated_wait_hint: String,
}

impl ResourceExhaustedDetail {
    /// Encode as a compact JSON string for embedding in `SessionError.hint`.
    ///
    /// The `estimated_wait_hint` is escaped to avoid JSON injection from
    /// characters such as `"`, `\`, or control characters.
    pub fn to_json_hint(&self) -> String {
        let hint_escaped = self
            .estimated_wait_hint
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!(
            r#"{{"limit":"{}","current":{},"capacity":{},"hint":"{}"}}"#,
            self.limit_kind.as_str(),
            self.current,
            self.limit,
            hint_escaped,
        )
    }
}

/// Which session limit pool was exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    ResidentSessionLimit,
    GuestSessionLimit,
    TotalSessionLimit,
}

impl LimitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LimitKind::ResidentSessionLimit => "resident_session_limit",
            LimitKind::GuestSessionLimit => "guest_session_limit",
            LimitKind::TotalSessionLimit => "total_session_limit",
        }
    }
}

// ─── Hot-connect snapshot ─────────────────────────────────────────────────────

/// Scene snapshot delivered to a newly-connected agent (§Requirement: Hot-Connect).
///
/// Produced by the compositor/scene thread under the scene lock, then handed
/// to the network/session thread for delivery in `SessionEstablished`. The
/// compositor thread is never blocked by snapshot delivery.
#[derive(Clone, Debug)]
pub struct HotConnectSnapshot {
    /// Serialised scene JSON (as returned by `SceneGraph::snapshot_json()`).
    pub scene_json: String,
    /// Scene version at the time the snapshot was taken.
    pub scene_version: u64,
    /// Monotonic timestamp when the snapshot was captured.
    pub captured_at: Instant,
}

impl HotConnectSnapshot {
    /// Construct a snapshot from a raw JSON string and scene version.
    pub fn new(scene_json: String, scene_version: u64) -> Self {
        Self {
            scene_json,
            scene_version,
            captured_at: Instant::now(),
        }
    }

    /// Returns `true` if this snapshot is "fresh" — captured within `max_age_ms`
    /// milliseconds of `now`.
    ///
    /// A stale snapshot may be re-captured if the caller tracks freshness.
    pub fn is_fresh(&self, now: Instant, max_age_ms: u64) -> bool {
        now.duration_since(self.captured_at).as_millis() as u64 <= max_age_ms
    }
}

// ─── AdmissionController ─────────────────────────────────────────────────────

/// Session admission controller.
///
/// Enforces session limits, tracks active sessions by kind, and supports
/// hot-connect snapshot handoff.
///
/// # Thread model
/// This struct is single-threaded (compositor/control-plane thread).
/// Network threads hold `SessionEnvelope` values passed by reference; they do
/// not mutate the admission controller.
pub struct AdmissionController {
    /// Configured session limits.
    limits: SessionLimits,
    /// Active sessions keyed by session_id.
    sessions: HashMap<String, SessionEnvelope>,
    /// Monotonically increasing admission counter (used for wait-hint estimation).
    admit_counter: u64,
}

impl AdmissionController {
    /// Construct with default session limits.
    pub fn new() -> Self {
        Self::with_limits(SessionLimits::default())
    }

    /// Construct with custom session limits.
    pub fn with_limits(limits: SessionLimits) -> Self {
        Self {
            limits,
            sessions: HashMap::new(),
            admit_counter: 0,
        }
    }

    // ── Admission ──────────────────────────────────────────────────────────

    /// Attempt to admit a new session.
    ///
    /// Checks all applicable limits (total, then kind-specific). If any limit
    /// is reached, returns `ResourceExhausted` with structured detail.
    ///
    /// On success, registers the session and returns `Admitted(session_id)`.
    pub fn admit(
        &mut self,
        session_id: String,
        namespace: String,
        scene_session_id: SceneId,
        kind: AgentKind,
        budget: ResourceBudget,
        max_active_leases: u32,
    ) -> AdmissionOutcome {
        // Check total session limit first.
        if self.sessions.len() >= self.limits.max_total {
            return AdmissionOutcome::ResourceExhausted(ResourceExhaustedDetail {
                limit_kind: LimitKind::TotalSessionLimit,
                current: self.sessions.len(),
                limit: self.limits.max_total,
                estimated_wait_hint: self.estimate_wait_hint(),
            });
        }

        // Check kind-specific limit.
        let kind_count = self.sessions.values().filter(|s| s.kind == kind).count();
        let kind_limit = match kind {
            AgentKind::Resident => self.limits.max_resident,
            AgentKind::Guest => self.limits.max_guest,
        };
        let limit_kind = match kind {
            AgentKind::Resident => LimitKind::ResidentSessionLimit,
            AgentKind::Guest => LimitKind::GuestSessionLimit,
        };
        if kind_count >= kind_limit {
            return AdmissionOutcome::ResourceExhausted(ResourceExhaustedDetail {
                limit_kind,
                current: kind_count,
                limit: kind_limit,
                estimated_wait_hint: self.estimate_wait_hint(),
            });
        }

        // Reject duplicate session IDs — HashMap::insert would silently replace
        // an existing session and corrupt pool counts.
        if self.sessions.contains_key(&session_id) {
            return AdmissionOutcome::DuplicateSessionId;
        }

        // Admit the session.
        let mut envelope = SessionEnvelope::new(
            session_id.clone(),
            namespace,
            scene_session_id,
            kind,
            budget,
            max_active_leases,
        );
        envelope.mark_admitted();
        self.sessions.insert(session_id.clone(), envelope);
        self.admit_counter += 1;

        AdmissionOutcome::Admitted(session_id)
    }

    /// Admit with default budget (convenience wrapper for resident sessions).
    ///
    /// Used when the session protocol does not carry custom budget parameters.
    pub fn admit_resident_default(
        &mut self,
        session_id: String,
        namespace: String,
        scene_session_id: SceneId,
    ) -> AdmissionOutcome {
        self.admit(
            session_id,
            namespace,
            scene_session_id,
            AgentKind::Resident,
            ResourceBudget::default(),
            crate::session::DEFAULT_MAX_ACTIVE_LEASES,
        )
    }

    /// Remove a session from the admission table (on disconnect/revocation).
    ///
    /// Returns the evicted envelope, or `None` if not found.
    pub fn evict(&mut self, session_id: &str) -> Option<SessionEnvelope> {
        self.sessions.remove(session_id)
    }

    // ── Session queries ────────────────────────────────────────────────────

    /// Number of currently admitted sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Number of admitted sessions of the given kind.
    pub fn session_count_by_kind(&self, kind: AgentKind) -> usize {
        self.sessions.values().filter(|s| s.kind == kind).count()
    }

    /// Retrieve a session envelope by session ID.
    pub fn get(&self, session_id: &str) -> Option<&SessionEnvelope> {
        self.sessions.get(session_id)
    }

    /// Retrieve a mutable session envelope by session ID.
    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut SessionEnvelope> {
        self.sessions.get_mut(session_id)
    }

    /// Returns `true` if the resident pool is at or over its limit.
    pub fn resident_pool_full(&self) -> bool {
        self.session_count_by_kind(AgentKind::Resident) >= self.limits.max_resident
    }

    /// Returns `true` if the guest pool is at or over its limit.
    pub fn guest_pool_full(&self) -> bool {
        self.session_count_by_kind(AgentKind::Guest) >= self.limits.max_guest
    }

    /// Returns `true` if the total session pool is at or over its limit.
    pub fn total_pool_full(&self) -> bool {
        self.sessions.len() >= self.limits.max_total
    }

    /// Configured session limits (read-only view).
    pub fn limits(&self) -> &SessionLimits {
        &self.limits
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    /// Produce a human-readable wait hint for the agent (spec line 361).
    ///
    /// For v1 this is a static estimate; a production implementation would
    /// track session churn rate and produce a time-based estimate.
    fn estimate_wait_hint(&self) -> String {
        // v1: heuristic only — sessions last an unknown duration, so we return
        // a generic hint rather than a false specific estimate.
        "retry after a few seconds; capacity may free as active sessions disconnect".to_string()
    }
}

impl Default for AdmissionController {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::ResourceBudget;

    fn make_controller() -> AdmissionController {
        AdmissionController::new()
    }

    fn admit_resident(controller: &mut AdmissionController, name: &str) -> AdmissionOutcome {
        controller.admit_resident_default(
            format!("session-{}", name),
            name.to_string(),
            SceneId::new(),
        )
    }

    // ─── Default limits ────────────────────────────────────────────────────

    #[test]
    fn test_default_limits() {
        let c = make_controller();
        assert_eq!(c.limits().max_resident, DEFAULT_MAX_RESIDENT_SESSIONS);
        assert_eq!(c.limits().max_guest, DEFAULT_MAX_GUEST_SESSIONS);
        assert_eq!(c.limits().max_total, DEFAULT_MAX_TOTAL_SESSIONS);
    }

    // ─── Happy-path admission ──────────────────────────────────────────────

    #[test]
    fn test_admit_single_session() {
        let mut c = make_controller();
        let outcome = admit_resident(&mut c, "agent-a");
        assert!(
            matches!(outcome, AdmissionOutcome::Admitted(_)),
            "should admit first session"
        );
        assert_eq!(c.session_count(), 1);
    }

    #[test]
    fn test_session_evict() {
        let mut c = make_controller();
        admit_resident(&mut c, "agent-a");
        let evicted = c.evict("session-agent-a");
        assert!(evicted.is_some());
        assert_eq!(c.session_count(), 0);
    }

    // ─── Resident limit enforcement (spec line 355, 361) ───────────────────

    #[test]
    fn test_resident_limit_rejects_17th_session() {
        let mut c = AdmissionController::with_limits(SessionLimits::new(
            16, // max_resident = 16 (spec default)
            64,
            80,
        ));

        // Admit 16 resident sessions — all should succeed.
        for i in 0..16 {
            let outcome = admit_resident(&mut c, &format!("agent-{i}"));
            assert!(
                matches!(outcome, AdmissionOutcome::Admitted(_)),
                "session {i} should be admitted"
            );
        }

        // 17th must be rejected with RESOURCE_EXHAUSTED.
        let outcome = admit_resident(&mut c, "agent-16");
        match outcome {
            AdmissionOutcome::ResourceExhausted(detail) => {
                assert_eq!(detail.limit_kind, LimitKind::ResidentSessionLimit);
                assert_eq!(detail.current, 16);
                assert_eq!(detail.limit, 16);
                assert!(
                    !detail.estimated_wait_hint.is_empty(),
                    "should include a wait hint"
                );
            }
            other => panic!("expected ResourceExhausted, got {:?}", other),
        }
    }

    #[test]
    fn test_resource_exhausted_detail_json_hint() {
        let detail = ResourceExhaustedDetail {
            limit_kind: LimitKind::ResidentSessionLimit,
            current: 16,
            limit: 16,
            estimated_wait_hint: "retry in 5s".to_string(),
        };
        let json = detail.to_json_hint();
        assert!(json.contains("resident_session_limit"));
        assert!(json.contains("\"current\":16"));
        assert!(json.contains("\"capacity\":16"));
        assert!(json.contains("retry in 5s"));
    }

    // ─── Guest limit enforcement ───────────────────────────────────────────

    #[test]
    fn test_guest_limit_enforced() {
        let mut c = AdmissionController::with_limits(SessionLimits::new(2, 2, 10));

        // Admit 2 guests.
        for i in 0..2 {
            let outcome = c.admit(
                format!("g-session-{i}"),
                format!("guest-{i}"),
                SceneId::new(),
                AgentKind::Guest,
                ResourceBudget::default(),
                crate::session::DEFAULT_MAX_ACTIVE_LEASES,
            );
            assert!(
                matches!(outcome, AdmissionOutcome::Admitted(_)),
                "guest {i} should be admitted"
            );
        }

        // 3rd guest rejected.
        let outcome = c.admit(
            "g-session-3".into(),
            "guest-3".into(),
            SceneId::new(),
            AgentKind::Guest,
            ResourceBudget::default(),
            crate::session::DEFAULT_MAX_ACTIVE_LEASES,
        );
        assert!(
            matches!(outcome, AdmissionOutcome::ResourceExhausted(ref d) if d.limit_kind == LimitKind::GuestSessionLimit),
            "expected guest limit exhaustion"
        );
    }

    // ─── Total session limit ────────────────────────────────────────────────

    #[test]
    fn test_total_session_limit_blocks_admission() {
        // max_total=2 < max_resident=5 + max_guest=5
        let mut c = AdmissionController::with_limits(SessionLimits::new(5, 5, 2));

        // First 2 admissions succeed.
        for i in 0..2 {
            let outcome = admit_resident(&mut c, &format!("agent-{i}"));
            assert!(
                matches!(outcome, AdmissionOutcome::Admitted(_)),
                "session {i} should be admitted"
            );
        }

        // 3rd is blocked by total limit.
        let outcome = admit_resident(&mut c, "agent-2");
        assert!(
            matches!(outcome, AdmissionOutcome::ResourceExhausted(ref d) if d.limit_kind == LimitKind::TotalSessionLimit),
            "expected total limit exhaustion"
        );
    }

    // ─── Limit capping ────────────────────────────────────────────────────

    #[test]
    fn test_limits_capped_to_hard_maxima() {
        let limits = SessionLimits::new(9999, 9999, 9999);
        assert_eq!(limits.max_resident, HARD_MAX_RESIDENT_SESSIONS);
        assert_eq!(limits.max_guest, HARD_MAX_GUEST_SESSIONS);
        assert_eq!(limits.max_total, HARD_MAX_TOTAL_SESSIONS);
    }

    // ─── Hot-connect snapshot ──────────────────────────────────────────────

    #[test]
    fn test_hot_connect_snapshot_construction() {
        let snap = HotConnectSnapshot::new(r#"{"scene":{}}"#.to_string(), 42);
        assert_eq!(snap.scene_version, 42);
        assert!(snap.scene_json.contains("scene"));
    }

    #[test]
    fn test_hot_connect_snapshot_freshness() {
        let snap = HotConnectSnapshot::new("{}".to_string(), 0);
        let now = Instant::now();
        // Snapshot was just captured — should be fresh up to any reasonable max_age.
        assert!(snap.is_fresh(now, 1000), "snapshot should be fresh immediately");
    }

    #[test]
    fn test_hot_connect_snapshot_staleness() {
        // Manually construct a snapshot with a captured_at in the "past" by
        // using a snapshot captured 200ms ago, then testing with max_age=100ms.
        let snap = HotConnectSnapshot {
            scene_json: "{}".to_string(),
            scene_version: 0,
            captured_at: Instant::now() - std::time::Duration::from_millis(200),
        };
        let now = Instant::now();
        assert!(
            !snap.is_fresh(now, 100),
            "200ms-old snapshot should be stale at 100ms max_age"
        );
        assert!(
            snap.is_fresh(now, 300),
            "200ms-old snapshot should be fresh at 300ms max_age"
        );
    }

    // ─── Session pool queries ──────────────────────────────────────────────

    #[test]
    fn test_resident_pool_full_detection() {
        let mut c = AdmissionController::with_limits(SessionLimits::new(1, 64, 80));
        assert!(!c.resident_pool_full());
        admit_resident(&mut c, "agent-a");
        assert!(c.resident_pool_full());
    }

    #[test]
    fn test_total_pool_full_detection() {
        let mut c = AdmissionController::with_limits(SessionLimits::new(5, 5, 1));
        assert!(!c.total_pool_full());
        admit_resident(&mut c, "agent-a");
        assert!(c.total_pool_full());
    }

    // ─── Session count by kind ─────────────────────────────────────────────

    #[test]
    fn test_session_count_by_kind() {
        let mut c = make_controller();

        // Admit 2 residents.
        admit_resident(&mut c, "r1");
        admit_resident(&mut c, "r2");

        // Admit 1 guest.
        c.admit(
            "g-session-1".into(),
            "guest-1".into(),
            SceneId::new(),
            AgentKind::Guest,
            ResourceBudget::default(),
            crate::session::DEFAULT_MAX_ACTIVE_LEASES,
        );

        assert_eq!(c.session_count_by_kind(AgentKind::Resident), 2);
        assert_eq!(c.session_count_by_kind(AgentKind::Guest), 1);
        assert_eq!(c.session_count(), 3);
    }

    // ─── Duplicate session_id rejection ───────────────────────────────────

    #[test]
    fn test_duplicate_session_id_rejected() {
        let mut c = make_controller();

        // First admission with "dup-session" succeeds.
        let outcome = c.admit_resident_default(
            "dup-session".to_string(),
            "agent-dup".to_string(),
            SceneId::new(),
        );
        assert!(matches!(outcome, AdmissionOutcome::Admitted(_)));
        assert_eq!(c.session_count(), 1);

        // Second admission with the same session_id must be rejected.
        let outcome = c.admit_resident_default(
            "dup-session".to_string(),
            "agent-dup-2".to_string(),
            SceneId::new(),
        );
        assert!(
            matches!(outcome, AdmissionOutcome::DuplicateSessionId),
            "duplicate session_id must be rejected"
        );
        // Session count must not have changed (no silent overwrite).
        assert_eq!(c.session_count(), 1);
    }

    // ─── JSON hint escaping ────────────────────────────────────────────────

    #[test]
    fn test_json_hint_escapes_special_characters() {
        let detail = ResourceExhaustedDetail {
            limit_kind: LimitKind::ResidentSessionLimit,
            current: 16,
            limit: 16,
            estimated_wait_hint: r#"wait "5s" or retry\n"#.to_string(),
        };
        let json = detail.to_json_hint();
        // The hint must not break the JSON structure (no unescaped quotes or backslashes).
        assert!(!json.contains(r#"wait "5s""#), "unescaped quote should not appear in JSON");
        assert!(json.contains(r#"wait \"5s\""#), "quote should be escaped");
    }
}
