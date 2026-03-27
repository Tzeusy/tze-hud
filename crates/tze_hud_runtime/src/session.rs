//! Per-session resource envelope and memory overhead tracking.
//!
//! Implements the per-agent resource envelope requirements from
//! runtime-kernel/spec.md §Requirement: Per-Agent Resource Envelope (line 307)
//! and §Requirement: Session Memory Overhead (line 298).
//!
//! # Memory Overhead Contract
//!
//! The `SessionEnvelope` struct must remain below 64 KB at steady state
//! (exclusive of content — textures, node data). The struct itself is
//! designed to stay well under this limit. The `assert_memory_overhead`
//! function verifies this at test time.
//!
//! # Default and Hard-Max Envelope Values (spec §Requirement: Per-Agent Resource Envelope)
//!
//! | Dimension         | Default    | Hard max   |
//! |-------------------|------------|------------|
//! | max_tiles         | 8          | 64         |
//! | max_texture_bytes | 256 MiB    | 2 GiB      |
//! | max_update_rate_hz| 30         | 120        |
//! | max_nodes_per_tile| 32         | 64         |
//! | max_active_leases | 8          | 64         |

use std::time::Instant;
use tze_hud_scene::types::{ResourceBudget, SceneId};

// ─── Envelope hard maximums ───────────────────────────────────────────────────

/// Absolute maximum tiles an agent may hold (hard cap, spec line 309).
pub const HARD_MAX_TILES: u32 = 64;

/// Absolute maximum texture bytes an agent may consume (hard cap, spec line 310).
pub const HARD_MAX_TEXTURE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// Absolute maximum update rate in Hz (hard cap, spec line 311).
pub const HARD_MAX_UPDATE_RATE_HZ: f32 = 120.0;

/// Absolute maximum nodes per tile (hard cap, spec line 312).
pub const HARD_MAX_NODES_PER_TILE: u32 = 64;

/// Absolute maximum active leases (hard cap, spec line 313).
pub const HARD_MAX_ACTIVE_LEASES: u32 = 64;

// ─── Envelope defaults ────────────────────────────────────────────────────────

/// Default maximum tiles (spec line 308).
pub const DEFAULT_MAX_TILES: u32 = 8;

/// Default maximum texture bytes (spec line 309): 256 MiB.
pub const DEFAULT_MAX_TEXTURE_BYTES: u64 = 256 * 1024 * 1024;

/// Default maximum update rate in Hz (spec line 310).
pub const DEFAULT_MAX_UPDATE_RATE_HZ: f32 = 30.0;

/// Default maximum nodes per tile (spec line 311).
pub const DEFAULT_MAX_NODES_PER_TILE: u32 = 32;

/// Default maximum active leases (spec line 312).
pub const DEFAULT_MAX_ACTIVE_LEASES: u32 = 8;

// ─── SessionEnvelope ──────────────────────────────────────────────────────────

/// Agent type, determining which session limit pool is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    /// Resident agent (long-lived, high-trust). Counted in resident limit.
    Resident,
    /// Guest agent (transient, lower-trust). Counted in guest limit.
    Guest,
}

/// Per-session resource envelope and metadata.
///
/// Stored in `AdmissionController`'s session table. Memory overhead MUST be
/// < 64 KB per session (exclusive of content — textures, node data). This
/// struct is kept small: fixed-size fields + two small `String` fields.
///
/// # Memory budget analysis
/// - Fixed fields: ~128 bytes
/// - `session_id` string: ≤ 36 bytes (UUID)
/// - `namespace` string: ≤ 255 bytes (agent name limit)
/// - Total per-envelope: ≈ 512 bytes worst case (well under 64 KB)
#[derive(Clone, Debug)]
pub struct SessionEnvelope {
    /// Unique session identifier (UUIDv7 string).
    pub session_id: String,
    /// Agent namespace (agent's registered name).
    pub namespace: String,
    /// The scene-object session ID.
    pub scene_session_id: SceneId,
    /// Whether this is a resident or guest agent.
    pub kind: AgentKind,
    /// Assigned resource budget (capped to hard maximums at construction).
    pub budget: ResourceBudget,
    /// Maximum active leases for this session.
    pub max_active_leases: u32,
    /// Timestamp when the session was established.
    pub established_at: Instant,
    /// Whether this session has been admitted (passed admission checks).
    pub admitted: bool,
}

impl SessionEnvelope {
    /// Construct a new `SessionEnvelope` for a resident agent using default budgets.
    ///
    /// Budget values are capped to their respective hard maximums.
    pub fn new_resident(session_id: String, namespace: String, scene_session_id: SceneId) -> Self {
        Self::new(
            session_id,
            namespace,
            scene_session_id,
            AgentKind::Resident,
            ResourceBudget::default(),
            DEFAULT_MAX_ACTIVE_LEASES,
        )
    }

    /// Construct a new `SessionEnvelope` for a guest agent using default budgets.
    pub fn new_guest(session_id: String, namespace: String, scene_session_id: SceneId) -> Self {
        Self::new(
            session_id,
            namespace,
            scene_session_id,
            AgentKind::Guest,
            ResourceBudget::default(),
            DEFAULT_MAX_ACTIVE_LEASES,
        )
    }

    /// Construct a `SessionEnvelope` with explicit budget (capped to hard maxima).
    pub fn new(
        session_id: String,
        namespace: String,
        scene_session_id: SceneId,
        kind: AgentKind,
        requested_budget: ResourceBudget,
        requested_max_leases: u32,
    ) -> Self {
        let budget = ResourceBudget {
            max_tiles: requested_budget.max_tiles.min(HARD_MAX_TILES),
            max_texture_bytes: requested_budget
                .max_texture_bytes
                .min(HARD_MAX_TEXTURE_BYTES),
            max_update_rate_hz: requested_budget
                .max_update_rate_hz
                .min(HARD_MAX_UPDATE_RATE_HZ),
            max_nodes_per_tile: requested_budget
                .max_nodes_per_tile
                .min(HARD_MAX_NODES_PER_TILE),
        };
        let max_active_leases = requested_max_leases.min(HARD_MAX_ACTIVE_LEASES);
        Self {
            session_id,
            namespace,
            scene_session_id,
            kind,
            budget,
            max_active_leases,
            established_at: Instant::now(),
            admitted: false,
        }
    }

    /// Mark this session as fully admitted (handshake + budget negotiation complete).
    pub fn mark_admitted(&mut self) {
        self.admitted = true;
    }

    /// Compute the approximate memory overhead (bytes) of this session envelope,
    /// excluding content (textures, node data).
    ///
    /// Used to assert the < 64 KB session overhead requirement.
    pub fn memory_overhead_bytes(&self) -> usize {
        // Stack-size of the struct plus the heap allocations for the two String fields.
        // String::len() returns the *used* byte count; String::capacity() would return
        // the *allocated* byte count. We use len() here because the spec requirement
        // is for steady-state overhead, where capacity ≈ len for these small strings.
        //
        // Per-session state NOT counted here (held in separate structs):
        // - BudgetEnforcer::AgentResourceState: update-rate VecDeque of ~60 Instants ≈ 480 B
        // - Event subscription buffers: 0–4 pending messages × ~64 B = negligible at steady state
        //
        // Combined upper bound for all session state remains well under 64 KB.
        std::mem::size_of::<SessionEnvelope>() + self.session_id.len() + self.namespace.len()
    }
}

/// Assert that a `SessionEnvelope`'s memory overhead is below the 64 KB limit.
///
/// The 64 KB limit covers session metadata and event subscription buffers
/// (exclusive of content — textures, node data). See spec line 298–304.
///
/// # Panics
/// Panics if `envelope.memory_overhead_bytes()` ≥ 65,536.
pub fn assert_memory_overhead_within_budget(envelope: &SessionEnvelope) {
    let overhead = envelope.memory_overhead_bytes();
    assert!(
        overhead < 65_536,
        "session '{}' memory overhead {}B exceeds 64KB limit",
        envelope.namespace,
        overhead,
    );
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::ResourceBudget;

    fn make_envelope() -> SessionEnvelope {
        SessionEnvelope::new_resident(
            "test-session-id".to_string(),
            "test-agent".to_string(),
            SceneId::new(),
        )
    }

    // ─── Default values ────────────────────────────────────────────────────

    #[test]
    fn test_default_envelope_uses_spec_defaults() {
        let e = make_envelope();
        assert_eq!(e.budget.max_tiles, DEFAULT_MAX_TILES);
        assert_eq!(e.budget.max_texture_bytes, DEFAULT_MAX_TEXTURE_BYTES);
        assert_eq!(e.budget.max_update_rate_hz, DEFAULT_MAX_UPDATE_RATE_HZ);
        assert_eq!(e.budget.max_nodes_per_tile, DEFAULT_MAX_NODES_PER_TILE);
        assert_eq!(e.max_active_leases, DEFAULT_MAX_ACTIVE_LEASES);
    }

    #[test]
    fn test_new_session_is_not_admitted() {
        let e = make_envelope();
        assert!(!e.admitted, "new session should start as not admitted");
    }

    #[test]
    fn test_mark_admitted() {
        let mut e = make_envelope();
        e.mark_admitted();
        assert!(e.admitted);
    }

    // ─── Hard-max capping ──────────────────────────────────────────────────

    #[test]
    fn test_budget_capped_to_hard_max_tiles() {
        let e = SessionEnvelope::new(
            "id".into(),
            "ns".into(),
            SceneId::new(),
            AgentKind::Resident,
            ResourceBudget {
                max_tiles: 9999,
                max_texture_bytes: DEFAULT_MAX_TEXTURE_BYTES,
                max_update_rate_hz: DEFAULT_MAX_UPDATE_RATE_HZ,
                max_nodes_per_tile: DEFAULT_MAX_NODES_PER_TILE,
            },
            DEFAULT_MAX_ACTIVE_LEASES,
        );
        assert_eq!(e.budget.max_tiles, HARD_MAX_TILES);
    }

    #[test]
    fn test_budget_capped_to_hard_max_texture_bytes() {
        let e = SessionEnvelope::new(
            "id".into(),
            "ns".into(),
            SceneId::new(),
            AgentKind::Resident,
            ResourceBudget {
                max_tiles: DEFAULT_MAX_TILES,
                max_texture_bytes: 100 * 1024 * 1024 * 1024, // 100 GiB >> 2 GiB hard max
                max_update_rate_hz: DEFAULT_MAX_UPDATE_RATE_HZ,
                max_nodes_per_tile: DEFAULT_MAX_NODES_PER_TILE,
            },
            DEFAULT_MAX_ACTIVE_LEASES,
        );
        assert_eq!(e.budget.max_texture_bytes, HARD_MAX_TEXTURE_BYTES);
    }

    #[test]
    fn test_budget_capped_to_hard_max_update_rate() {
        let e = SessionEnvelope::new(
            "id".into(),
            "ns".into(),
            SceneId::new(),
            AgentKind::Resident,
            ResourceBudget {
                max_tiles: DEFAULT_MAX_TILES,
                max_texture_bytes: DEFAULT_MAX_TEXTURE_BYTES,
                max_update_rate_hz: 9999.0,
                max_nodes_per_tile: DEFAULT_MAX_NODES_PER_TILE,
            },
            DEFAULT_MAX_ACTIVE_LEASES,
        );
        assert_eq!(e.budget.max_update_rate_hz, HARD_MAX_UPDATE_RATE_HZ);
    }

    #[test]
    fn test_budget_capped_to_hard_max_nodes_per_tile() {
        let e = SessionEnvelope::new(
            "id".into(),
            "ns".into(),
            SceneId::new(),
            AgentKind::Resident,
            ResourceBudget {
                max_tiles: DEFAULT_MAX_TILES,
                max_texture_bytes: DEFAULT_MAX_TEXTURE_BYTES,
                max_update_rate_hz: DEFAULT_MAX_UPDATE_RATE_HZ,
                max_nodes_per_tile: 9999,
            },
            DEFAULT_MAX_ACTIVE_LEASES,
        );
        assert_eq!(e.budget.max_nodes_per_tile, HARD_MAX_NODES_PER_TILE);
    }

    #[test]
    fn test_max_active_leases_capped() {
        let e = SessionEnvelope::new(
            "id".into(),
            "ns".into(),
            SceneId::new(),
            AgentKind::Resident,
            ResourceBudget::default(),
            9999,
        );
        assert_eq!(e.max_active_leases, HARD_MAX_ACTIVE_LEASES);
    }

    // ─── Memory overhead ───────────────────────────────────────────────────

    #[test]
    fn test_memory_overhead_under_64kb() {
        let e = make_envelope();
        assert_memory_overhead_within_budget(&e);
    }

    #[test]
    fn test_memory_overhead_struct_size_reasonable() {
        // The struct itself (excluding heap allocations for Strings) should
        // be well under 1 KiB.
        let base_size = std::mem::size_of::<SessionEnvelope>();
        assert!(
            base_size < 1024,
            "SessionEnvelope struct is unexpectedly large: {base_size} bytes"
        );
    }

    // ─── Agent kind ────────────────────────────────────────────────────────

    #[test]
    fn test_resident_kind() {
        let e = SessionEnvelope::new_resident("id".into(), "ns".into(), SceneId::new());
        assert_eq!(e.kind, AgentKind::Resident);
    }

    #[test]
    fn test_guest_kind() {
        let e = SessionEnvelope::new_guest("id".into(), "ns".into(), SceneId::new());
        assert_eq!(e.kind, AgentKind::Guest);
    }
}
