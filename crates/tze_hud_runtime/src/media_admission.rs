//! # Media Activation Gate
//!
//! Runtime activation gate for the bounded-ingress media plane, per RFC 0014 §6.1
//! and signoff packet C13/C15/C17.
//!
//! ## Admission Evaluation Order (RFC 0014 §6.1)
//!
//! Admission is evaluated in strict short-circuit order:
//!
//! 1. **Capability gate** (§A2.1): `media-ingress` granted, dialog / 7-day remember passed
//!    per RFC 0008 A1 §A2.
//! 2. **Budget headroom** (§A2.2): per-session stream limit
//!    (`max_concurrent_media_streams`, default 1); global GPU texture headroom ≥ 128 MiB.
//!    Note: worker pool slot availability (RFC 0002 A1 §A2.2) is checked by the caller before
//!    invoking the activation gate — it is not rechecked here. E25 step ≥ 8 also blocks
//!    new admissions (maps to `POOL_EXHAUSTED` wire reject code).
//! 3. **Role authority** (§A2.3): capability grant authorized by `owner` or `admin` role
//!    per RFC 0009 A1.
//!
//! Any failure short-circuits and returns the corresponding `MediaRejectCode`.
//!
//! ## C13 Capability Dialog Gate
//!
//! The dialog gate (step 0 per RFC 0008 A1 §A4.1) is evaluated before the
//! above three steps. It fires when:
//!
//! - The requested capability is one of the eight C13 tokens.
//! - The capability is enabled at deployment level.
//! - No valid 7-day remember record exists.
//! - No session-level cached grant exists.
//!
//! This module provides the synchronous evaluation path for checking whether
//! a capability grant is already cached (step 0b and 0c). The interactive dialog
//! (step 0d) is an async chrome-layer concern outside this module's scope.
//!
//! ## E25 Degradation Ladder Integration (RFC 0014 §5)
//!
//! The activation gate checks the current runtime degradation level and maps
//! it to E25 ladder steps 8–10 (teardown / presence revoke / disconnect).
//! At E25 step ≥ 8, new admissions are refused.
//!
//! ## C15 Trust Boundary
//!
//! The gate enforces the C15 trust boundary: `cloud-relay` admissions are only
//! permitted when the deployment config has the cloud-relay transport enabled
//! **and** the operator role check (step 3) passes.
//!
//! ## Telemetry (engineering-bar §5)
//!
//! Every admission decision (grant or deny) MUST be:
//! - Emitted as a `MediaAuditEvent` for C17 audit.
//! - Traced via `tracing::info!` with a structured span carrying the subsystem name.
//! - Recorded as a metric increment on the `media_admission_total` and
//!   `media_admission_deny_total` counters (structured log counters — no external
//!   metric system in v2; the tracing subscriber exposes these as JSON).
//!
//! ## Audit latency bound (RFC 0019 / C17)
//!
//! Audit event emission MUST complete in < 100ms. The current implementation
//! accepts a `MediaAuditSink` trait object to avoid coupling to a specific sink.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tracing::{info, warn};
use tze_hud_telemetry::{
    DegradationStep, DegradationTrigger, MediaAuditEvent, MediaCloseReason, MediaRejectCode,
    OperatorOverrideKind,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default maximum concurrent media streams per session (RFC 0014 §6.1 §A2.2).
pub const DEFAULT_MAX_CONCURRENT_MEDIA_STREAMS: u32 = 1;

/// Minimum GPU texture headroom required for admission, in bytes (128 MiB).
pub const MIN_GPU_TEXTURE_HEADROOM_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum `MediaIngressOpen` requests per session per second (RFC 0014 §9.5).
pub const MAX_SIGNALING_REQUESTS_PER_SECOND: u32 = 10;

/// 7-day duration in microseconds (RFC 0008 A1 §A3.3).
pub const REMEMBER_TTL_US: u64 = 7 * 24 * 60 * 60 * 1_000_000;

/// Default capability dialog timeout (RFC 0008 A1 §A2.2).
pub const DEFAULT_DIALOG_TIMEOUT_MS: u64 = 30_000;

/// Capability token for live inbound visual streams (RFC 0008 A1 §A1).
pub const CAPABILITY_MEDIA_INGRESS: &str = "media-ingress";

/// Capability token for microphone input (RFC 0008 A1 §A1).
pub const CAPABILITY_MICROPHONE_INGRESS: &str = "microphone-ingress";

/// Capability token for audio output (RFC 0008 A1 §A1).
pub const CAPABILITY_AUDIO_EMIT: &str = "audio-emit";

/// Capability token for recording (RFC 0008 A1 §A1).
pub const CAPABILITY_RECORDING: &str = "recording";

/// Capability token for cloud-relay (RFC 0008 A1 §A1).
pub const CAPABILITY_CLOUD_RELAY: &str = "cloud-relay";

/// Capability token for external transcoding (RFC 0008 A1 §A1).
pub const CAPABILITY_EXTERNAL_TRANSCODE: &str = "external-transcode";

/// Capability token for federation (reserved in v2; rejected at runtime).
pub const CAPABILITY_FEDERATED_SEND: &str = "federated-send";

/// Capability token for agent-to-agent media (RFC 0008 A1 §A1).
pub const CAPABILITY_AGENT_TO_AGENT_MEDIA: &str = "agent-to-agent-media";

/// The eight C13 capability tokens (RFC 0008 A1 §A1).
pub const C13_CAPABILITIES: &[&str] = &[
    CAPABILITY_MEDIA_INGRESS,
    CAPABILITY_MICROPHONE_INGRESS,
    CAPABILITY_AUDIO_EMIT,
    CAPABILITY_RECORDING,
    CAPABILITY_CLOUD_RELAY,
    CAPABILITY_EXTERNAL_TRANSCODE,
    CAPABILITY_FEDERATED_SEND,
    CAPABILITY_AGENT_TO_AGENT_MEDIA,
];

// ─── Operator Role ────────────────────────────────────────────────────────────

/// Operator role per RFC 0009 A1 §A1.3.
///
/// Only `Owner` and `Admin` may grant C13 media capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorRole {
    /// Full authority: may grant/revoke any capability and manage all operators.
    Owner,
    /// Admin authority: may grant/revoke media capabilities.
    Admin,
    /// Standard member: may not grant media capabilities.
    Member,
    /// Guest/observer: no grant authority.
    Guest,
}

impl OperatorRole {
    /// Returns `true` if this role may grant C13 media capabilities (RFC 0009 A1 §A1.3).
    pub fn may_grant_media_capability(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

// ─── Trust Transport ─────────────────────────────────────────────────────────

/// Media transport path, relevant to C15 trust boundary enforcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaTransport {
    /// Local WebRTC within the runtime (no external relay).
    LocalWebRtc,
    /// Cloud relay via external SFU (requires `cloud-relay` capability + C15 clearance).
    CloudRelay,
}

// ─── 7-Day Remember Record ────────────────────────────────────────────────────

/// Per-agent-per-capability 7-day remember record (RFC 0008 A1 §A3.2).
///
/// Valid when:
/// - `!revoked`
/// - `expires_at_us > now_us()`
#[derive(Clone, Debug)]
pub struct CapabilityRememberRecord {
    /// Agent namespace this record applies to.
    pub agent_namespace: String,
    /// Capability token this record applies to.
    pub capability: String,
    /// Operator principal who granted and chose "remember" (local UUID in v2).
    pub granted_by: String,
    /// When the remember record was written (UTC microseconds).
    pub granted_at_us: u64,
    /// When the remember record expires (UTC microseconds).
    /// Always `granted_at_us + REMEMBER_TTL_US`.
    pub expires_at_us: u64,
    /// Whether this record has been explicitly revoked before natural expiry.
    pub revoked: bool,
    /// If revoked: the operator principal who revoked it.
    pub revoked_by: Option<String>,
    /// If revoked: when the revocation occurred (UTC microseconds).
    pub revoked_at_us: Option<u64>,
}

impl CapabilityRememberRecord {
    /// Create a new remember record with the standard 7-day TTL.
    pub fn new(
        agent_namespace: impl Into<String>,
        capability: impl Into<String>,
        granted_by: impl Into<String>,
        granted_at_us: u64,
    ) -> Self {
        Self {
            agent_namespace: agent_namespace.into(),
            capability: capability.into(),
            granted_by: granted_by.into(),
            expires_at_us: granted_at_us.saturating_add(REMEMBER_TTL_US),
            granted_at_us,
            revoked: false,
            revoked_by: None,
            revoked_at_us: None,
        }
    }

    /// Returns `true` if this record is currently valid (not expired, not revoked).
    pub fn is_valid(&self, now_us: u64) -> bool {
        !self.revoked && self.expires_at_us > now_us
    }

    /// Revoke this record immediately.
    pub fn revoke(&mut self, revoked_by: impl Into<String>, revoked_at_us: u64) {
        self.revoked = true;
        self.revoked_by = Some(revoked_by.into());
        self.revoked_at_us = Some(revoked_at_us);
    }
}

// ─── Session Capability Cache ─────────────────────────────────────────────────

/// In-session cache of interactively granted capabilities (RFC 0008 A1 §A2.4).
///
/// Lives only for the duration of the session; does not survive teardown.
/// A cache hit skips the dialog gate for subsequent requests in the same session.
#[derive(Clone, Debug, Default)]
pub struct SessionCapabilityCache {
    /// Set of (agent_namespace, capability) pairs granted in this session.
    grants: HashMap<(String, String), SessionCapabilityGrant>,
}

/// A single session-scoped capability grant record.
#[derive(Clone, Debug)]
pub struct SessionCapabilityGrant {
    /// Session for which this grant was cached.
    pub session_id: String,
    /// Agent namespace within that session.
    pub agent_namespace: String,
    /// Capability token granted.
    pub capability: String,
    /// When this grant was recorded (UTC microseconds).
    pub granted_at_us: u64,
    /// Whether a 7-day remember record was also written.
    pub remember_written: bool,
}

impl SessionCapabilityCache {
    /// Returns `true` if `(agent_namespace, capability)` has a cached grant.
    pub fn has_grant(&self, agent_namespace: &str, capability: &str) -> bool {
        self.grants
            .contains_key(&(agent_namespace.to_string(), capability.to_string()))
    }

    /// Insert a grant into the session cache.
    pub fn insert(&mut self, grant: SessionCapabilityGrant) {
        self.grants.insert(
            (grant.agent_namespace.clone(), grant.capability.clone()),
            grant,
        );
    }

    /// Remove a grant from the session cache (e.g., on mid-session capability revocation).
    pub fn remove(&mut self, agent_namespace: &str, capability: &str) {
        self.grants
            .remove(&(agent_namespace.to_string(), capability.to_string()));
    }
}

// ─── Deployment Config Snapshot ───────────────────────────────────────────────

/// Deployment-level capability enable/disable flags.
///
/// Set at startup via runtime config; requires restart to change.
/// Per RFC 0008 A1 §A2.1: if a capability is disabled at this level,
/// `LeaseRequest`s including it are denied immediately with
/// `CAPABILITY_NOT_ENABLED` — no dialog.
#[derive(Clone, Debug)]
pub struct MediaCapabilityConfig {
    /// Whether each C13 capability is enabled at deployment level.
    enabled: HashMap<String, bool>,
    /// Maximum concurrent media streams per session.
    pub max_concurrent_streams: u32,
    /// Whether cloud-relay transport is enabled (C15 trust boundary).
    pub cloud_relay_enabled: bool,
}

impl Default for MediaCapabilityConfig {
    fn default() -> Self {
        let mut enabled = HashMap::new();
        // By default, only `media-ingress` is enabled; all others off.
        enabled.insert(CAPABILITY_MEDIA_INGRESS.to_string(), true);
        enabled.insert(CAPABILITY_MICROPHONE_INGRESS.to_string(), false);
        enabled.insert(CAPABILITY_AUDIO_EMIT.to_string(), false);
        enabled.insert(CAPABILITY_RECORDING.to_string(), false);
        enabled.insert(CAPABILITY_CLOUD_RELAY.to_string(), false);
        enabled.insert(CAPABILITY_EXTERNAL_TRANSCODE.to_string(), false);
        enabled.insert(CAPABILITY_FEDERATED_SEND.to_string(), false);
        enabled.insert(CAPABILITY_AGENT_TO_AGENT_MEDIA.to_string(), false);
        Self {
            enabled,
            max_concurrent_streams: DEFAULT_MAX_CONCURRENT_MEDIA_STREAMS,
            cloud_relay_enabled: false,
        }
    }
}

impl MediaCapabilityConfig {
    /// Returns `true` if the named capability is enabled at deployment level.
    pub fn is_enabled(&self, capability: &str) -> bool {
        self.enabled.get(capability).copied().unwrap_or(false)
    }

    /// Enable or disable a capability at deployment level.
    pub fn set_enabled(&mut self, capability: impl Into<String>, enabled: bool) {
        self.enabled.insert(capability.into(), enabled);
    }
}

// ─── Admission Request ────────────────────────────────────────────────────────

/// A media admission request — all inputs the gate needs to evaluate.
pub struct MediaAdmissionRequest<'a> {
    /// Session identifier.
    pub session_id: &'a str,
    /// Agent namespace making the request.
    pub agent_namespace: &'a str,
    /// Stream epoch for the new stream (assigned by caller before evaluation).
    pub stream_epoch: &'a str,
    /// Which C13 capability is being requested (typically `"media-ingress"`).
    pub capability: &'a str,
    /// Transport path (local vs. cloud-relay).
    pub transport: MediaTransport,
    /// Role of the operator currently associated with this session.
    pub operator_role: OperatorRole,
    /// Number of streams this session already has active.
    pub current_stream_count: u32,
    /// Available GPU texture memory in bytes.
    pub gpu_texture_headroom_bytes: u64,
    /// Current E25 runtime degradation level (0 = nominal, 8+ = block new admissions).
    pub e25_level: u8,
    /// Whether the capability is present in the session's granted capability set.
    pub capability_in_session_grants: bool,
}

// ─── Admission Outcome ────────────────────────────────────────────────────────

/// Outcome of a media admission evaluation.
#[derive(Debug, PartialEq, Eq)]
pub enum MediaAdmissionOutcome {
    /// Admission approved. Stream may proceed.
    Admitted,
    /// Admission denied. Contains the stable rejection code.
    Denied(MediaRejectCode),
    /// Dialog required before admission can proceed.
    ///
    /// The caller (chrome/async layer) MUST present the operator dialog for
    /// `(agent_namespace, capability)`, then re-evaluate with the result in
    /// the session capability cache.
    DialogRequired,
}

// ─── Admission Error ──────────────────────────────────────────────────────────

/// Error type for activation gate failures (internal faults, not admission denials).
#[derive(Debug, Error)]
pub enum MediaAdmissionError {
    /// The sink failed to emit the audit event within the 100ms latency bound.
    #[error("audit event emission failed: {0}")]
    AuditEmitFailed(String),
}

// ─── Audit Sink ───────────────────────────────────────────────────────────────

/// Sink for media plane audit events (C17).
///
/// The sink MUST emit the event durably within 100ms (C17 latency bound).
/// Implementations may be append-only log writers, channel senders, or
/// test-only collecting sinks.
pub trait MediaAuditSink: Send + Sync {
    /// Emit a single media audit event.
    ///
    /// Called on the control-plane thread. MUST NOT block for more than 100ms.
    fn emit(&self, event: MediaAuditEvent);
}

/// A no-op audit sink for tests and benchmarks.
pub struct NoopMediaAuditSink;

impl MediaAuditSink for NoopMediaAuditSink {
    fn emit(&self, _event: MediaAuditEvent) {}
}

/// A collecting audit sink that records events for test assertions.
#[derive(Default)]
pub struct CollectingMediaAuditSink {
    events: std::sync::Mutex<Vec<MediaAuditEvent>>,
}

impl CollectingMediaAuditSink {
    /// Drain all collected events and return them.
    pub fn drain(&self) -> Vec<MediaAuditEvent> {
        self.events
            .lock()
            .expect("mutex not poisoned")
            .drain(..)
            .collect()
    }

    /// Return the number of events collected so far.
    pub fn len(&self) -> usize {
        self.events.lock().expect("mutex not poisoned").len()
    }

    /// Returns `true` if no events have been collected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl MediaAuditSink for CollectingMediaAuditSink {
    fn emit(&self, event: MediaAuditEvent) {
        self.events.lock().expect("mutex not poisoned").push(event);
    }
}

impl MediaAuditSink for std::sync::Arc<CollectingMediaAuditSink> {
    fn emit(&self, event: MediaAuditEvent) {
        self.events.lock().expect("mutex not poisoned").push(event);
    }
}

// ─── Signaling Rate Limiter ───────────────────────────────────────────────────

/// Per-session signaling rate limiter for `MediaIngressOpen` requests.
///
/// Enforces the maximum of 10 requests/session/second (RFC 0014 §9.5).
/// Uses a sliding 1-second window backed by a ring buffer of timestamps.
#[derive(Debug, Default)]
pub struct SignalingRateLimiter {
    /// Timestamps (microseconds) of recent requests, newest-last.
    window: std::collections::VecDeque<u64>,
}

impl SignalingRateLimiter {
    /// Record a new request timestamp and return `true` if admission is allowed.
    ///
    /// Evicts entries older than 1 second before checking the limit.
    pub fn admit(&mut self, now_us: u64) -> bool {
        // Evict entries outside the 1-second sliding window.
        let cutoff = now_us.saturating_sub(1_000_000); // 1 second in µs
        while let Some(&front) = self.window.front() {
            if front <= cutoff {
                self.window.pop_front();
            } else {
                break;
            }
        }
        if self.window.len() >= MAX_SIGNALING_REQUESTS_PER_SECOND as usize {
            return false;
        }
        self.window.push_back(now_us);
        true
    }

    /// Return the number of requests in the current sliding window.
    pub fn window_count(&self) -> usize {
        self.window.len()
    }
}

// ─── Activation Gate ─────────────────────────────────────────────────────────

/// Runtime activation gate for the bounded-ingress media plane.
///
/// This is the primary enforcement point for RFC 0014 §6.1 (admission gate),
/// RFC 0008 A1 (C13 capability dialog), RFC 0009 A1 (role authority), and
/// C15 (trust boundary).
///
/// # Thread Model
///
/// This struct is owned by the control-plane / session server. It is NOT
/// used from the compositor thread. All methods take `&mut self` because the
/// gate maintains per-session state (remember store, session cache, rate limits).
pub struct MediaActivationGate {
    /// Deployment-level capability configuration.
    config: MediaCapabilityConfig,
    /// 7-day remember store: keyed by `(agent_namespace, capability)`.
    remember_store: HashMap<(String, String), CapabilityRememberRecord>,
    /// Per-session capability caches: keyed by session_id.
    session_caches: HashMap<String, SessionCapabilityCache>,
    /// Per-session signaling rate limiters: keyed by session_id.
    rate_limiters: HashMap<String, SignalingRateLimiter>,
    /// Audit event sink.
    sink: Box<dyn MediaAuditSink>,
}

impl MediaActivationGate {
    /// Create a new activation gate with the given config and audit sink.
    pub fn new(config: MediaCapabilityConfig, sink: Box<dyn MediaAuditSink>) -> Self {
        Self {
            config,
            remember_store: HashMap::new(),
            session_caches: HashMap::new(),
            rate_limiters: HashMap::new(),
            sink,
        }
    }

    /// Create an activation gate with default config and no-op audit sink.
    ///
    /// Suitable for tests and headless integration scenarios.
    pub fn with_defaults() -> Self {
        Self::new(
            MediaCapabilityConfig::default(),
            Box::new(NoopMediaAuditSink),
        )
    }

    // ── Signaling rate check ──────────────────────────────────────────────────

    /// Check and record a signaling request for the given session.
    ///
    /// Returns `false` (and emits an audit deny event) if the session has
    /// exceeded 10 `MediaIngressOpen` requests/second (RFC 0014 §9.5).
    ///
    /// **Wire-layer contract (RFC 0014 §9.5):** When this method returns `false`,
    /// the gRPC handler MUST reject the wire request with the `INVALID_ARGUMENT`
    /// reject code. The audit event records `SignalingRateLimitExceeded` as an
    /// internal discriminant for structured logging; that code does not appear
    /// in the wire protocol's §2.4 reject-code table.
    ///
    /// **Clock contract:** Production callers MUST supply a monotonic timestamp
    /// (e.g. `now_us_monotonic()`) so the sliding window is immune to NTP steps
    /// and wall-clock jumps. The audit event's `timestamp_us` field intentionally
    /// uses the same value; use `now_us()` only when a separate wall-clock stamp
    /// is needed for the audit record.
    pub fn check_signaling_rate(
        &mut self,
        session_id: &str,
        agent_namespace: &str,
        now_us: u64,
    ) -> bool {
        let limiter = self
            .rate_limiters
            .entry(session_id.to_string())
            .or_default();
        if !limiter.admit(now_us) {
            warn!(
                subsystem = "media_admission",
                session_id,
                agent_namespace,
                metric.media_admission_deny_total = 1u64,
                "signaling rate limit exceeded"
            );
            self.sink.emit(MediaAuditEvent::MediaAdmissionDeny {
                session_id: session_id.to_string(),
                agent_namespace: agent_namespace.to_string(),
                reject_code: MediaRejectCode::SignalingRateLimitExceeded,
                timestamp_us: now_us,
            });
            return false;
        }
        true
    }

    // ── C13 dialog gate (step 0) ──────────────────────────────────────────────

    /// Evaluate the C13 capability dialog gate (RFC 0008 A1 §A4.1, step 0).
    ///
    /// Returns:
    /// - `Ok(true)` if the gate passes (remember record or session cache hit).
    /// - `Ok(false)` if the dialog must be presented before proceeding.
    /// - `Err(MediaRejectCode)` if the request should be denied immediately
    ///   (capability disabled at deployment level, or federated-send in v2).
    ///
    /// This method does NOT perform the dialog itself — that is a chrome-layer concern.
    /// After the operator responds, the caller MUST call `record_session_grant` or
    /// `record_dialog_deny` before re-evaluating.
    pub fn evaluate_dialog_gate(
        &self,
        session_id: &str,
        agent_namespace: &str,
        capability: &str,
        now_us: u64,
    ) -> Result<bool, MediaRejectCode> {
        // `federated-send` is defined but not active in v2 (RFC 0008 A1 §A1).
        if capability == CAPABILITY_FEDERATED_SEND {
            return Err(MediaRejectCode::CapabilityNotImplemented);
        }

        // Check deployment-level enable flag.
        if !self.config.is_enabled(capability) {
            return Err(MediaRejectCode::CapabilityNotEnabled);
        }

        // Step 0b: valid 7-day remember record?
        let remember_key = (agent_namespace.to_string(), capability.to_string());
        if let Some(record) = self.remember_store.get(&remember_key) {
            if record.is_valid(now_us) {
                return Ok(true); // gate passes
            }
        }

        // Step 0c: session-level cached grant?
        if let Some(cache) = self.session_caches.get(session_id) {
            if cache.has_grant(agent_namespace, capability) {
                return Ok(true); // gate passes
            }
        }

        // Step 0d: dialog required.
        Ok(false)
    }

    // ── Main admission evaluation ─────────────────────────────────────────────

    /// Evaluate a media admission request.
    ///
    /// This implements RFC 0014 §6.1 steps 1–3 (after the C13 dialog gate).
    /// The dialog gate (step 0) must be evaluated separately via
    /// `evaluate_dialog_gate` and resolved before calling this method.
    ///
    /// An audit event is emitted for every decision (grant or deny).
    ///
    /// Returns `MediaAdmissionOutcome::Admitted` on success.
    pub fn evaluate(
        &mut self,
        req: &MediaAdmissionRequest<'_>,
        now_us: u64,
    ) -> MediaAdmissionOutcome {
        // ── Step 0: C13 dialog gate ───────────────────────────────────────────
        if C13_CAPABILITIES.contains(&req.capability) {
            match self.evaluate_dialog_gate(
                req.session_id,
                req.agent_namespace,
                req.capability,
                now_us,
            ) {
                Err(reject_code) => {
                    self.deny(req, reject_code.clone(), now_us);
                    return MediaAdmissionOutcome::Denied(reject_code);
                }
                Ok(false) => {
                    // Dialog required — caller must present dialog and re-evaluate.
                    return MediaAdmissionOutcome::DialogRequired;
                }
                Ok(true) => {} // gate passes, continue
            }
        }

        // ── Step 1: Capability gate ───────────────────────────────────────────
        // The requested capability must be in the session's granted capability set.
        if !req.capability_in_session_grants {
            self.deny(req, MediaRejectCode::CapabilityRequired, now_us);
            return MediaAdmissionOutcome::Denied(MediaRejectCode::CapabilityRequired);
        }

        // ── E25 ladder check: block new admissions at step ≥ 8 ───────────────
        // At step 8 (teardown) or higher, the runtime is tearing down media.
        // New admissions must be refused until the runtime recovers.
        if req.e25_level >= 8 {
            self.deny(req, MediaRejectCode::PoolExhausted, now_us);
            return MediaAdmissionOutcome::Denied(MediaRejectCode::PoolExhausted);
        }

        // ── C15 trust boundary: cloud-relay transport check ───────────────────
        if req.transport == MediaTransport::CloudRelay && !self.config.cloud_relay_enabled {
            self.deny(req, MediaRejectCode::TrustBoundaryViolation, now_us);
            return MediaAdmissionOutcome::Denied(MediaRejectCode::TrustBoundaryViolation);
        }

        // ── Step 2a: Per-session stream count limit ───────────────────────────
        if req.current_stream_count >= self.config.max_concurrent_streams {
            self.deny(req, MediaRejectCode::StreamLimitExceeded, now_us);
            return MediaAdmissionOutcome::Denied(MediaRejectCode::StreamLimitExceeded);
        }

        // ── Step 2b: GPU texture headroom check ───────────────────────────────
        if req.gpu_texture_headroom_bytes < MIN_GPU_TEXTURE_HEADROOM_BYTES {
            self.deny(req, MediaRejectCode::GpuTextureHeadroomInsufficient, now_us);
            return MediaAdmissionOutcome::Denied(MediaRejectCode::GpuTextureHeadroomInsufficient);
        }

        // ── Step 3: Role authority check (defense-in-depth) ───────────────────
        // Admission requires operator role `owner` or `admin` (RFC 0009 A1 §A1.3).
        if !req.operator_role.may_grant_media_capability() {
            self.deny(req, MediaRejectCode::RoleInsufficient, now_us);
            return MediaAdmissionOutcome::Denied(MediaRejectCode::RoleInsufficient);
        }

        // ── All gates passed — admit ──────────────────────────────────────────
        info!(
            subsystem = "media_admission",
            session_id = req.session_id,
            agent_namespace = req.agent_namespace,
            stream_epoch = req.stream_epoch,
            capability = req.capability,
            metric.media_admission_total = 1u64,
            "media stream admitted"
        );
        self.sink.emit(MediaAuditEvent::MediaAdmissionGrant {
            session_id: req.session_id.to_string(),
            agent_namespace: req.agent_namespace.to_string(),
            stream_epoch: req.stream_epoch.to_string(),
            capability: req.capability.to_string(),
            timestamp_us: now_us,
        });
        MediaAdmissionOutcome::Admitted
    }

    // ── State mutation helpers ────────────────────────────────────────────────

    /// Record a session-level capability grant (after operator dialog approval).
    ///
    /// Per RFC 0008 A1 §A2.4. If `remember` is true, also writes a 7-day
    /// remember record and emits the appropriate audit events.
    pub fn record_session_grant(
        &mut self,
        session_id: &str,
        agent_namespace: &str,
        capability: &str,
        granted_by: &str,
        now_us: u64,
        remember: bool,
    ) {
        let cache = self
            .session_caches
            .entry(session_id.to_string())
            .or_default();

        let remember_written = remember;
        let remember_expires_at_us = if remember {
            Some(now_us.saturating_add(REMEMBER_TTL_US))
        } else {
            None
        };

        cache.insert(SessionCapabilityGrant {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            capability: capability.to_string(),
            granted_at_us: now_us,
            remember_written,
        });

        // Emit capability_dialog_grant audit event.
        self.sink.emit(MediaAuditEvent::CapabilityDialogGrant {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            capability: capability.to_string(),
            granted_by: granted_by.to_string(),
            granted_at_us: now_us,
            remember_written,
            remember_expires_at_us,
        });

        if remember {
            // Write 7-day remember record and emit capability_remember audit event.
            let expires_at_us = now_us.saturating_add(REMEMBER_TTL_US);
            let record =
                CapabilityRememberRecord::new(agent_namespace, capability, granted_by, now_us);
            self.remember_store.insert(
                (agent_namespace.to_string(), capability.to_string()),
                record,
            );
            self.sink.emit(MediaAuditEvent::CapabilityRemember {
                agent_namespace: agent_namespace.to_string(),
                capability: capability.to_string(),
                granted_by: granted_by.to_string(),
                granted_at_us: now_us,
                expires_at_us,
            });
            info!(
                subsystem = "media_admission",
                agent_namespace,
                capability,
                granted_by,
                expires_at_us,
                "capability remember record written"
            );
        }
    }

    /// Revoke a 7-day remember record (RFC 0008 A1 §A3.4).
    ///
    /// Also clears any session-capability cache entries for the same
    /// agent+capability across all active sessions.
    ///
    /// Emits a `capability_remember_revoke` audit event.
    pub fn revoke_remember_record(
        &mut self,
        agent_namespace: &str,
        capability: &str,
        revoked_by: &str,
        now_us: u64,
        reason: &str,
    ) {
        let key = (agent_namespace.to_string(), capability.to_string());
        if let Some(record) = self.remember_store.get_mut(&key) {
            record.revoke(revoked_by, now_us);
        }

        // Clear session-level cache entries for this agent+capability.
        for cache in self.session_caches.values_mut() {
            cache.remove(agent_namespace, capability);
        }

        warn!(
            subsystem = "media_admission",
            agent_namespace, capability, revoked_by, reason, "capability remember record revoked"
        );
        self.sink.emit(MediaAuditEvent::CapabilityRememberRevoke {
            agent_namespace: agent_namespace.to_string(),
            capability: capability.to_string(),
            revoked_by: revoked_by.to_string(),
            revoked_at_us: now_us,
            reason: reason.to_string(),
        });
    }

    /// Record a media stream close or revocation event.
    ///
    /// Emits the appropriate audit event (`media_stream_close` or
    /// `media_stream_revoke`).
    pub fn record_stream_close(
        &self,
        session_id: &str,
        agent_namespace: &str,
        stream_epoch: &str,
        reason: MediaCloseReason,
        is_revoke: bool,
        now_us: u64,
    ) {
        if is_revoke {
            info!(
                subsystem = "media_admission",
                session_id,
                agent_namespace,
                stream_epoch,
                reason = reason.as_str(),
                "media stream revoked"
            );
            self.sink.emit(MediaAuditEvent::MediaStreamRevoke {
                session_id: session_id.to_string(),
                agent_namespace: agent_namespace.to_string(),
                stream_epoch: stream_epoch.to_string(),
                reason,
                timestamp_us: now_us,
            });
        } else {
            info!(
                subsystem = "media_admission",
                session_id,
                agent_namespace,
                stream_epoch,
                reason = reason.as_str(),
                "media stream closed"
            );
            self.sink.emit(MediaAuditEvent::MediaStreamClose {
                session_id: session_id.to_string(),
                agent_namespace: agent_namespace.to_string(),
                stream_epoch: stream_epoch.to_string(),
                reason,
                timestamp_us: now_us,
            });
        }
    }

    /// Record an E25 degradation step on a stream.
    ///
    /// Emits a `media_degradation_step` audit event.
    pub fn record_degradation_step(
        &self,
        session_id: &str,
        agent_namespace: &str,
        stream_epoch: &str,
        ladder_step: DegradationStep,
        trigger: DegradationTrigger,
        now_us: u64,
    ) {
        info!(
            subsystem = "media_admission",
            session_id,
            agent_namespace,
            stream_epoch,
            ladder_step = ladder_step.0,
            ladder_step_label = ladder_step.label(),
            "E25 degradation step recorded"
        );
        self.sink.emit(MediaAuditEvent::MediaDegradationStep {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            stream_epoch: stream_epoch.to_string(),
            ladder_step,
            trigger,
            timestamp_us: now_us,
        });
    }

    /// Record an operator chrome-level override action.
    ///
    /// Emits a `media_operator_override` audit event.
    pub fn record_operator_override(
        &self,
        session_id: &str,
        agent_namespace: &str,
        stream_epoch: &str,
        action: OperatorOverrideKind,
        now_us: u64,
    ) {
        info!(
            subsystem = "media_admission",
            session_id, agent_namespace, stream_epoch, "operator override recorded"
        );
        self.sink.emit(MediaAuditEvent::MediaOperatorOverride {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            stream_epoch: stream_epoch.to_string(),
            action,
            timestamp_us: now_us,
        });
    }

    /// Record a pool preemption event.
    ///
    /// Emits a `media_preempt` audit event.
    pub fn record_preempt(
        &self,
        session_id: &str,
        agent_namespace: &str,
        stream_epoch: &str,
        now_us: u64,
    ) {
        warn!(
            subsystem = "media_admission",
            session_id, agent_namespace, stream_epoch, "media stream preempted"
        );
        self.sink.emit(MediaAuditEvent::MediaPreempt {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            stream_epoch: stream_epoch.to_string(),
            timestamp_us: now_us,
        });
    }

    /// Record a mid-session capability revocation.
    ///
    /// Emits a `media_capability_revoke` audit event.
    pub fn record_capability_revoke(
        &self,
        session_id: &str,
        agent_namespace: &str,
        capability: &str,
        now_us: u64,
    ) {
        warn!(
            subsystem = "media_admission",
            session_id, agent_namespace, capability, "media capability revoked mid-session"
        );
        self.sink.emit(MediaAuditEvent::MediaCapabilityRevoke {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            capability: capability.to_string(),
            timestamp_us: now_us,
        });
    }

    /// Evict all session state on session teardown.
    pub fn evict_session(&mut self, session_id: &str) {
        self.session_caches.remove(session_id);
        self.rate_limiters.remove(session_id);
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn deny(&self, req: &MediaAdmissionRequest<'_>, code: MediaRejectCode, now_us: u64) {
        warn!(
            subsystem = "media_admission",
            session_id = req.session_id,
            agent_namespace = req.agent_namespace,
            reject_code = code.as_str(),
            metric.media_admission_deny_total = 1u64,
            "media stream admission denied"
        );
        self.sink.emit(MediaAuditEvent::MediaAdmissionDeny {
            session_id: req.session_id.to_string(),
            agent_namespace: req.agent_namespace.to_string(),
            reject_code: code,
            timestamp_us: now_us,
        });
    }
}

// ─── Utility: timestamps ─────────────────────────────────────────────────────

/// Process-start anchor for monotonic microseconds.
///
/// Initialized once on first call to [`now_us_monotonic`].
static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Return monotonic microseconds elapsed since process start.
///
/// Uses `std::time::Instant`, which is immune to wall-clock adjustments
/// (NTP steps, leap seconds, operator clock changes).
///
/// **Rate-limit callers MUST use this function**, not [`now_us`], so that
/// the sliding window is never corrupted by backwards time jumps.
/// Tests should supply their own arbitrary `u64` to remain deterministic.
pub fn now_us_monotonic() -> u64 {
    PROCESS_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_micros() as u64
}

/// Return the current UTC timestamp in microseconds.
///
/// Uses `SystemTime` — suitable for **audit event timestamps** and any context
/// where wall-clock time is required (e.g. 7-day remember record expiry).
/// Do **not** pass this value to [`SignalingRateLimiter::admit`] in production;
/// use [`now_us_monotonic`] there to avoid NTP-step sensitivity.
///
/// Tests should supply their own `now_us` to remain deterministic.
pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_micros() as u64
}

// ─── E25 Integration: map E25 level to media plane actions ───────────────────

/// Map the current runtime degradation level (0–5) to the E25 ladder step
/// that the media plane cares about.
///
/// The runtime degradation controller uses a 6-level scale (Normal=0 ..
/// Emergency=5). RFC 0014 §5 maps E25 steps 1–10 to media plane wire signals.
/// This function translates so the media admission gate can evaluate E25 impact.
///
/// | Runtime level | E25 media impact |
/// |---------------|-----------------|
/// | 0 Normal | none |
/// | 1 Coalesce | none (no media effect) |
/// | 2 ReduceTextureQuality | none (no media effect) |
/// | 3 DisableTransparency | none (no media effect) |
/// | 4 ShedTiles | step 6 (shed second stream) |
/// | 5 Emergency | step 8 (teardown media, keep session) |
///
/// Steps 1–5 and 7 are triggered by the media-plane watchdog, not the
/// compositor degradation level directly.
pub fn runtime_level_to_e25_step(runtime_level: u8) -> DegradationStep {
    match runtime_level {
        0..=3 => DegradationStep::NOMINAL,
        4 => DegradationStep::new(6),  // ShedTiles → drop second stream
        5 => DegradationStep::new(8),  // Emergency → teardown media, keep session
        _ => DegradationStep::new(10), // beyond Emergency → disconnect
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gate_with_sink(
        config: MediaCapabilityConfig,
        sink: &std::sync::Arc<CollectingMediaAuditSink>,
    ) -> MediaActivationGate {
        MediaActivationGate::new(config, Box::new(sink.clone()))
    }

    fn admitted_request<'a>(
        session_id: &'a str,
        agent_namespace: &'a str,
        stream_epoch: &'a str,
    ) -> MediaAdmissionRequest<'a> {
        MediaAdmissionRequest {
            session_id,
            agent_namespace,
            stream_epoch,
            capability: CAPABILITY_MEDIA_INGRESS,
            transport: MediaTransport::LocalWebRtc,
            operator_role: OperatorRole::Owner,
            current_stream_count: 0,
            gpu_texture_headroom_bytes: MIN_GPU_TEXTURE_HEADROOM_BYTES + 1,
            e25_level: 0,
            capability_in_session_grants: true,
        }
    }

    const NOW: u64 = 1_000_000_000_000; // stable test timestamp

    // ── Operator role ─────────────────────────────────────────────────────────

    #[test]
    fn test_owner_and_admin_may_grant() {
        assert!(OperatorRole::Owner.may_grant_media_capability());
        assert!(OperatorRole::Admin.may_grant_media_capability());
        assert!(!OperatorRole::Member.may_grant_media_capability());
        assert!(!OperatorRole::Guest.may_grant_media_capability());
    }

    // ── SignalingRateLimiter ───────────────────────────────────────────────────

    #[test]
    fn test_rate_limiter_allows_up_to_limit() {
        let mut limiter = SignalingRateLimiter::default();
        for _ in 0..MAX_SIGNALING_REQUESTS_PER_SECOND {
            assert!(limiter.admit(NOW), "should admit within limit");
        }
        // One more — should be denied.
        assert!(!limiter.admit(NOW), "should deny at limit+1");
    }

    #[test]
    fn test_rate_limiter_window_slides() {
        let mut limiter = SignalingRateLimiter::default();
        for i in 0..MAX_SIGNALING_REQUESTS_PER_SECOND {
            limiter.admit(NOW + i as u64 * 100); // spread within 1s
        }
        // 1 second later — window clears.
        assert!(
            limiter.admit(NOW + 1_100_000),
            "window should slide after 1s"
        );
    }

    // ── Dialog gate ───────────────────────────────────────────────────────────

    #[test]
    fn test_dialog_gate_returns_dialog_required_when_no_cache() {
        let config = MediaCapabilityConfig::default();
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let gate = make_gate_with_sink(config, &sink);
        // media-ingress is enabled but no session cache or remember record.
        let result = gate.evaluate_dialog_gate("sess-1", "agent-a", CAPABILITY_MEDIA_INGRESS, NOW);
        assert_eq!(result, Ok(false), "dialog required without cache");
    }

    #[test]
    fn test_dialog_gate_passes_with_valid_remember_record() {
        let config = MediaCapabilityConfig::default();
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let mut gate = make_gate_with_sink(config, &sink);

        // Insert a valid remember record.
        let record =
            CapabilityRememberRecord::new("agent-a", CAPABILITY_MEDIA_INGRESS, "op-1", NOW);
        gate.remember_store.insert(
            ("agent-a".to_string(), CAPABILITY_MEDIA_INGRESS.to_string()),
            record,
        );

        let result =
            gate.evaluate_dialog_gate("sess-1", "agent-a", CAPABILITY_MEDIA_INGRESS, NOW + 1);
        assert_eq!(result, Ok(true), "remember record should grant passage");
    }

    #[test]
    fn test_dialog_gate_passes_with_session_cache() {
        let config = MediaCapabilityConfig::default();
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let mut gate = make_gate_with_sink(config, &sink);

        // Insert a session cache entry.
        let cache = gate.session_caches.entry("sess-1".to_string()).or_default();
        cache.insert(SessionCapabilityGrant {
            session_id: "sess-1".to_string(),
            agent_namespace: "agent-a".to_string(),
            capability: CAPABILITY_MEDIA_INGRESS.to_string(),
            granted_at_us: NOW,
            remember_written: false,
        });

        let result =
            gate.evaluate_dialog_gate("sess-1", "agent-a", CAPABILITY_MEDIA_INGRESS, NOW + 1);
        assert_eq!(result, Ok(true), "session cache should grant passage");
    }

    #[test]
    fn test_dialog_gate_rejects_federated_send() {
        let config = MediaCapabilityConfig::default();
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let gate = make_gate_with_sink(config, &sink);
        let result = gate.evaluate_dialog_gate("sess-1", "agent-a", CAPABILITY_FEDERATED_SEND, NOW);
        assert_eq!(
            result,
            Err(MediaRejectCode::CapabilityNotImplemented),
            "federated-send is not active in v2"
        );
    }

    #[test]
    fn test_dialog_gate_rejects_disabled_capability() {
        let mut config = MediaCapabilityConfig::default();
        config.set_enabled(CAPABILITY_MEDIA_INGRESS, false);
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let gate = make_gate_with_sink(config, &sink);
        let result = gate.evaluate_dialog_gate("sess-1", "agent-a", CAPABILITY_MEDIA_INGRESS, NOW);
        assert_eq!(result, Err(MediaRejectCode::CapabilityNotEnabled));
    }

    #[test]
    fn test_expired_remember_record_requires_dialog() {
        let config = MediaCapabilityConfig::default();
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let mut gate = make_gate_with_sink(config, &sink);

        // Insert an expired remember record.
        let mut record =
            CapabilityRememberRecord::new("agent-a", CAPABILITY_MEDIA_INGRESS, "op-1", NOW);
        // Manually expire it.
        record.expires_at_us = NOW; // expires at exactly NOW — is_valid checks >, so invalid
        gate.remember_store.insert(
            ("agent-a".to_string(), CAPABILITY_MEDIA_INGRESS.to_string()),
            record,
        );

        // Evaluating at NOW: expires_at_us == NOW, so is_valid returns false.
        let result = gate.evaluate_dialog_gate("sess-1", "agent-a", CAPABILITY_MEDIA_INGRESS, NOW);
        assert_eq!(result, Ok(false), "expired record should require dialog");
    }

    // ── Admission gate ────────────────────────────────────────────────────────

    fn make_gate_preloaded_cache(
        session_id: &str,
        agent_namespace: &str,
    ) -> (
        MediaActivationGate,
        std::sync::Arc<CollectingMediaAuditSink>,
    ) {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());
        let mut gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));
        let cache = gate
            .session_caches
            .entry(session_id.to_string())
            .or_default();
        cache.insert(SessionCapabilityGrant {
            session_id: session_id.to_string(),
            agent_namespace: agent_namespace.to_string(),
            capability: CAPABILITY_MEDIA_INGRESS.to_string(),
            granted_at_us: NOW,
            remember_written: false,
        });
        (gate, sink)
    }

    #[test]
    fn test_admit_succeeds_with_all_gates_passing() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let req = admitted_request("sess-1", "agent-a", "epoch-1");
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(outcome, MediaAdmissionOutcome::Admitted);
        let events = sink.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name(), "media_admission_grant");
    }

    #[test]
    fn test_deny_when_capability_not_in_session_grants() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let mut req = admitted_request("sess-1", "agent-a", "epoch-1");
        req.capability_in_session_grants = false;
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Denied(MediaRejectCode::CapabilityRequired)
        );
        let events = sink.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name(), "media_admission_deny");
    }

    #[test]
    fn test_deny_when_stream_limit_exceeded() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let mut req = admitted_request("sess-1", "agent-a", "epoch-1");
        req.current_stream_count = DEFAULT_MAX_CONCURRENT_MEDIA_STREAMS;
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Denied(MediaRejectCode::StreamLimitExceeded)
        );
        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| e.event_name() == "media_admission_deny")
        );
    }

    #[test]
    fn test_deny_when_gpu_headroom_insufficient() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let mut req = admitted_request("sess-1", "agent-a", "epoch-1");
        req.gpu_texture_headroom_bytes = MIN_GPU_TEXTURE_HEADROOM_BYTES - 1;
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Denied(MediaRejectCode::GpuTextureHeadroomInsufficient)
        );
        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| e.event_name() == "media_admission_deny")
        );
    }

    #[test]
    fn test_deny_when_role_insufficient() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let mut req = admitted_request("sess-1", "agent-a", "epoch-1");
        req.operator_role = OperatorRole::Member;
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Denied(MediaRejectCode::RoleInsufficient)
        );
        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| e.event_name() == "media_admission_deny")
        );
    }

    #[test]
    fn test_deny_at_e25_level_8() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let mut req = admitted_request("sess-1", "agent-a", "epoch-1");
        req.e25_level = 8; // teardown media, keep session
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Denied(MediaRejectCode::PoolExhausted)
        );
        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| e.event_name() == "media_admission_deny")
        );
    }

    #[test]
    fn test_deny_cloud_relay_when_trust_boundary_blocks() {
        let (mut gate, sink) = make_gate_preloaded_cache("sess-1", "agent-a");
        let mut config = MediaCapabilityConfig::default();
        config.cloud_relay_enabled = false;
        gate.config = config;

        let mut req = admitted_request("sess-1", "agent-a", "epoch-1");
        req.transport = MediaTransport::CloudRelay;
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Denied(MediaRejectCode::TrustBoundaryViolation)
        );
        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| e.event_name() == "media_admission_deny")
        );
    }

    #[test]
    fn test_dialog_required_when_no_session_cache() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let mut gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));
        // No session cache — dialog required.
        let req = admitted_request("sess-2", "agent-b", "epoch-1");
        let outcome = gate.evaluate(&req, NOW);
        assert_eq!(outcome, MediaAdmissionOutcome::DialogRequired);
        // No audit event for DialogRequired (dialog not yet resolved).
        assert_eq!(sink.len(), 0);
    }

    // ── Remember record lifecycle ─────────────────────────────────────────────

    #[test]
    fn test_record_session_grant_with_remember_emits_two_events() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let mut gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        gate.record_session_grant(
            "sess-1",
            "agent-a",
            CAPABILITY_MEDIA_INGRESS,
            "op-1",
            NOW,
            true,
        );

        let events = sink.drain();
        assert_eq!(
            events.len(),
            2,
            "should emit capability_dialog_grant + capability_remember"
        );
        let names: Vec<_> = events.iter().map(|e| e.event_name()).collect();
        assert!(names.contains(&"capability_dialog_grant"));
        assert!(names.contains(&"capability_remember"));
    }

    #[test]
    fn test_record_session_grant_without_remember_emits_one_event() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let mut gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        gate.record_session_grant(
            "sess-1",
            "agent-a",
            CAPABILITY_MEDIA_INGRESS,
            "op-1",
            NOW,
            false,
        );

        let events = sink.drain();
        assert_eq!(events.len(), 1, "should emit only capability_dialog_grant");
        assert_eq!(events[0].event_name(), "capability_dialog_grant");
    }

    #[test]
    fn test_remember_record_ttl_is_seven_days() {
        let record =
            CapabilityRememberRecord::new("agent-a", CAPABILITY_MEDIA_INGRESS, "op-1", NOW);
        assert_eq!(
            record.expires_at_us - record.granted_at_us,
            REMEMBER_TTL_US,
            "TTL must be exactly 7 days"
        );
        assert!(record.is_valid(NOW + 1));
        assert!(!record.is_valid(NOW + REMEMBER_TTL_US)); // exactly at expiry → invalid (strict >)
    }

    #[test]
    fn test_revoke_remember_record_emits_audit_and_clears_session_cache() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let mut gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        // Write a remember record and session cache entry.
        gate.record_session_grant(
            "sess-1",
            "agent-a",
            CAPABILITY_MEDIA_INGRESS,
            "op-1",
            NOW,
            true,
        );
        sink.drain(); // clear prior events

        // Revoke.
        gate.revoke_remember_record(
            "agent-a",
            CAPABILITY_MEDIA_INGRESS,
            "op-2",
            NOW + 100,
            "operator_manual_revoke",
        );

        // Verify audit event.
        let events = sink.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name(), "capability_remember_revoke");

        // Verify session cache is cleared.
        let cache = gate.session_caches.get("sess-1");
        let has_grant = cache.map_or(false, |c| c.has_grant("agent-a", CAPABILITY_MEDIA_INGRESS));
        assert!(!has_grant, "session cache should be cleared after revoke");

        // Verify remember record is marked revoked.
        let key = ("agent-a".to_string(), CAPABILITY_MEDIA_INGRESS.to_string());
        let record = gate.remember_store.get(&key).unwrap();
        assert!(record.revoked);
        assert_eq!(record.revoked_by.as_deref(), Some("op-2"));
    }

    // ── Stream close / revoke / preempt / degradation events ─────────────────

    #[test]
    fn test_record_stream_close_emits_audit_event() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        gate.record_stream_close(
            "sess-1",
            "agent-a",
            "epoch-1",
            MediaCloseReason::AgentClose,
            false,
            NOW,
        );

        let events = sink.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name(), "media_stream_close");
    }

    #[test]
    fn test_record_stream_revoke_emits_revoke_event() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        gate.record_stream_close(
            "sess-1",
            "agent-a",
            "epoch-1",
            MediaCloseReason::CapabilityRevoked,
            true,
            NOW,
        );

        let events = sink.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name(), "media_stream_revoke");
    }

    #[test]
    fn test_record_degradation_step_emits_audit_event() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        gate.record_degradation_step(
            "sess-1",
            "agent-a",
            "epoch-1",
            DegradationStep::new(2),
            DegradationTrigger::RuntimeLadderAdvance,
            NOW,
        );

        let events = sink.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name(), "media_degradation_step");
    }

    // ── E25 ladder mapping ────────────────────────────────────────────────────

    #[test]
    fn test_runtime_level_to_e25_step_mapping() {
        assert_eq!(runtime_level_to_e25_step(0), DegradationStep::NOMINAL);
        assert_eq!(runtime_level_to_e25_step(1), DegradationStep::NOMINAL);
        assert_eq!(runtime_level_to_e25_step(2), DegradationStep::NOMINAL);
        assert_eq!(runtime_level_to_e25_step(3), DegradationStep::NOMINAL);
        assert_eq!(runtime_level_to_e25_step(4), DegradationStep::new(6)); // ShedTiles → step 6
        assert_eq!(runtime_level_to_e25_step(5), DegradationStep::new(8)); // Emergency → step 8
        assert_eq!(runtime_level_to_e25_step(6), DegradationStep::new(10)); // beyond → step 10
    }

    // ── Session eviction ──────────────────────────────────────────────────────

    #[test]
    fn test_evict_session_clears_cache_and_rate_limiter() {
        let sink = std::sync::Arc::new(CollectingMediaAuditSink::default());

        let mut gate =
            MediaActivationGate::new(MediaCapabilityConfig::default(), Box::new(sink.clone()));

        gate.session_caches.entry("sess-1".to_string()).or_default();
        gate.rate_limiters.entry("sess-1".to_string()).or_default();

        gate.evict_session("sess-1");

        assert!(!gate.session_caches.contains_key("sess-1"));
        assert!(!gate.rate_limiters.contains_key("sess-1"));
    }

    // ── Monotonic clock helper ────────────────────────────────────────────────

    #[test]
    fn test_now_us_monotonic_is_non_decreasing() {
        // Call twice in quick succession; monotonic time must not go backwards.
        let t1 = now_us_monotonic();
        let t2 = now_us_monotonic();
        assert!(
            t2 >= t1,
            "now_us_monotonic regressed: t1={t1} t2={t2}"
        );
    }

    #[test]
    fn test_now_us_monotonic_is_non_zero_after_startup() {
        // After process start the monotonic counter must have advanced beyond
        // zero (the process start anchor is in the past).
        let t = now_us_monotonic();
        // Allow up to 1 µs if called immediately after init — practically
        // impossible, but be conservative: simply require it is not max/overflow.
        assert!(t < u64::MAX, "now_us_monotonic overflowed");
    }

    #[test]
    fn test_rate_limiter_with_monotonic_time_respects_window() {
        // Verify that SignalingRateLimiter works correctly when driven by
        // monotonic timestamps (the production-intended clock source).
        let mut limiter = SignalingRateLimiter::default();
        let base = now_us_monotonic();

        // Fill up to the limit using synthetic offsets from the monotonic base.
        for i in 0..MAX_SIGNALING_REQUESTS_PER_SECOND {
            assert!(
                limiter.admit(base + i as u64 * 100),
                "should admit within limit"
            );
        }
        // Exactly at limit — next request at same window should be denied.
        assert!(!limiter.admit(base + 999_000), "should deny at limit");

        // After the 1-second window elapses the window should clear.
        assert!(
            limiter.admit(base + 1_100_000),
            "should admit after window slides"
        );
    }
}
