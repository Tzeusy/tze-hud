//! # Media Plane Audit Events
//!
//! Structured audit events for the media plane, per RFC 0014 §9.6 and signoff
//! packet C17 (mandatory audit events).
//!
//! ## C17 Retention Policy
//!
//! All events: 90-day default retention, operator-configurable, local append-only
//! with daily rotation, schema-versioned. Governed by forthcoming RFC 0019
//! (Audit Log Schema and Retention).
//!
//! ## Event Inventory (RFC 0014 §9.6)
//!
//! | Event | Trigger |
//! |-------|---------|
//! | `media_admission_grant` | `MediaIngressOpenResult(admitted=true)` emitted |
//! | `media_admission_deny` | `MediaIngressOpenResult(admitted=false)` emitted |
//! | `media_stream_close` | `MediaIngressCloseNotice` emitted (any reason) |
//! | `media_stream_revoke` | Stream transitions to `REVOKED` state |
//! | `media_degradation_step` | `MediaDegradationNotice` with non-zero `ladder_step` |
//! | `media_capability_revoke` | `media-ingress` (or related) capability revoked mid-session |
//! | `media_preempt` | Pool preemption (RFC 0002 A1 §A3.2) |
//! | `media_operator_override` | Operator chrome-level mute/pause/revoke |
//!
//! ## Capability Dialog Events (RFC 0008 A1 §A5)
//!
//! | Event | Trigger |
//! |-------|---------|
//! | `capability_dialog_grant` | Operator granted C13 capability through dialog |
//! | `capability_remember` | 7-day remember record written |
//! | `capability_remember_revoke` | Operator revoked a remember record |

use serde::{Deserialize, Serialize};

// ─── Media Admission Reject Code ─────────────────────────────────────────────

/// Stable string codes for media admission rejections.
///
/// Per engineering-bar §5 ("Stable error codes"): append-only — never rename or
/// reuse a code. These appear in `MediaAuditEvent::MediaAdmissionDeny`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MediaRejectCode {
    /// `media-ingress` (or related) capability is not present in the session.
    CapabilityRequired,
    /// Capability is disabled at deployment config level.
    CapabilityNotEnabled,
    /// Operator denied capability via the per-session dialog.
    CapabilityDialogDenied,
    /// No authorized operator responded to dialog within timeout (default 30s).
    CapabilityDialogTimeout,
    /// Capability defined but not active in this runtime version (e.g., `federated-send` in v2).
    CapabilityNotImplemented,
    /// Per-session concurrent stream limit exceeded (default 1 per RFC 0014 §6.1).
    StreamLimitExceeded,
    /// Admission worker pool exhausted; no slot available.
    PoolExhausted,
    /// Global GPU texture headroom below 128 MiB floor (RFC 0014 §6.1 §A2.2).
    GpuTextureHeadroomInsufficient,
    /// Agent role insufficient; `owner` or `admin` required (RFC 0009 A1 §A1.3).
    RoleInsufficient,
    /// Content classification denied for current viewer class.
    ContentClassDenied,
    /// Quiet-hours policy blocked admission.
    PolicyQuietHours,
    /// Trust boundary (C15) blocks the requested transport path.
    TrustBoundaryViolation,
    /// Request rate exceeded: more than 10 `MediaIngressOpen` requests/session/second.
    SignalingRateLimitExceeded,
    /// SDP payload oversize or otherwise invalid.
    InvalidArgument,
}

impl MediaRejectCode {
    /// Return the stable string code for this reject reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CapabilityRequired => "CAPABILITY_REQUIRED",
            Self::CapabilityNotEnabled => "CAPABILITY_NOT_ENABLED",
            Self::CapabilityDialogDenied => "CAPABILITY_DIALOG_DENIED",
            Self::CapabilityDialogTimeout => "CAPABILITY_DIALOG_TIMEOUT",
            Self::CapabilityNotImplemented => "CAPABILITY_NOT_IMPLEMENTED",
            Self::StreamLimitExceeded => "STREAM_LIMIT_EXCEEDED",
            Self::PoolExhausted => "POOL_EXHAUSTED",
            Self::GpuTextureHeadroomInsufficient => "GPU_TEXTURE_HEADROOM_INSUFFICIENT",
            Self::RoleInsufficient => "ROLE_INSUFFICIENT",
            Self::ContentClassDenied => "CONTENT_CLASS_DENIED",
            Self::PolicyQuietHours => "POLICY_QUIET_HOURS",
            Self::TrustBoundaryViolation => "TRUST_BOUNDARY_VIOLATION",
            Self::SignalingRateLimitExceeded => "SIGNALING_RATE_LIMIT_EXCEEDED",
            Self::InvalidArgument => "INVALID_ARGUMENT",
        }
    }
}

impl std::fmt::Display for MediaRejectCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ─── E25 Degradation Step ────────────────────────────────────────────────────

/// E25 degradation ladder step (1–10), as defined in `failure.md` and RFC 0014 §5.
///
/// Step 0 means no degradation (nominal). Steps 1–10 map to the ordered ladder.
/// This type is used in `media_degradation_step` audit events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DegradationStep(pub u8);

impl DegradationStep {
    /// Nominal state — no active degradation.
    pub const NOMINAL: Self = Self(0);

    /// Construct a degradation step, clamped to the 1–10 range.
    pub fn new(step: u8) -> Self {
        Self(step.min(10))
    }

    /// Returns `true` if this is the nominal (non-degraded) state.
    pub fn is_nominal(self) -> bool {
        self.0 == 0
    }

    /// Human-readable label for this step, per `failure.md`.
    pub fn label(self) -> &'static str {
        match self.0 {
            0 => "nominal",
            1 => "degrade_spatial_audio",
            2 => "reduce_framerate",
            3 => "reduce_resolution",
            4 => "suspend_recording",
            5 => "drop_cloud_relay",
            6 => "drop_second_stream",
            7 => "freeze_and_block_input",
            8 => "teardown_media_keep_session",
            9 => "revoke_embodied_presence",
            10 => "disconnect",
            _ => "unknown",
        }
    }
}

impl std::fmt::Display for DegradationStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "step_{} ({})", self.0, self.label())
    }
}

// ─── Media Close Reason ──────────────────────────────────────────────────────

/// Why a media stream was closed or revoked.
///
/// Mirrors the `reason` discriminants from `MediaIngressCloseNotice` in RFC 0014 §2.4.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaCloseReason {
    /// Agent requested teardown of its own stream.
    AgentClose,
    /// E25 step 8 "tear down media, keep session" reached.
    DegradationTeardown,
    /// E25 step 9 reached; paired with RFC 0015 presence demote.
    EmbodimentRevoked,
    /// E25 step 10 / session teardown.
    SessionDisconnected,
    /// `media-ingress` capability revoked mid-session.
    CapabilityRevoked,
    /// Operator chrome-level mute affordance.
    OperatorMute,
    /// Pool preemption by a higher-priority stream.
    Preempted,
    /// Per-stream watchdog threshold crossed (CPU/GPU/ring-buffer/decoder lifetime).
    WatchdogKilled,
}

impl MediaCloseReason {
    /// Stable string label for this reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentClose => "agent_close",
            Self::DegradationTeardown => "degradation_teardown",
            Self::EmbodimentRevoked => "embodiment_revoked",
            Self::SessionDisconnected => "session_disconnected",
            Self::CapabilityRevoked => "capability_revoked",
            Self::OperatorMute => "operator_mute",
            Self::Preempted => "preempted",
            Self::WatchdogKilled => "watchdog_killed",
        }
    }
}

// ─── Operator Override Kind ──────────────────────────────────────────────────

/// Which chrome-level operator action was taken.
///
/// Used in `media_operator_override` audit events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorOverrideKind {
    /// Operator muted the stream (teardown via mute affordance).
    Mute,
    /// Operator paused the stream (reversible).
    Pause,
    /// Operator resumed a paused stream.
    Resume,
    /// Operator revoked the `media-ingress` capability for this session.
    RevokeCapability,
}

// ─── Degradation Trigger ─────────────────────────────────────────────────────

/// What triggered the degradation step.
///
/// Per RFC 0014 §5.3.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationTrigger {
    /// Global runtime degradation level advanced (frame-time guardian).
    RuntimeLadderAdvance,
    /// Per-stream watchdog threshold crossed.
    WatchdogPerStream,
    /// Operator manual override.
    OperatorManual,
    /// Capability revoked or quiet-hours policy fired.
    CapabilityPolicy,
    /// Recovery — degradation level receded.
    Recovery,
}

// ─── Audit Event ─────────────────────────────────────────────────────────────

/// A single media plane audit event per RFC 0014 §9.6 + RFC 0008 A1 §A5.
///
/// All variants include the mandatory common fields: `session_id`,
/// `agent_namespace`, `stream_epoch` (where applicable), `timestamp_us`.
/// The `event` field always holds a stable, lowercase-underscored string name —
/// never rename an existing value.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MediaAuditEvent {
    // ── RFC 0014 §9.6 events ────────────────────────────────────────────────
    /// `MediaIngressOpenResult(admitted=true)` emitted.
    MediaAdmissionGrant {
        /// Session that admitted the stream.
        session_id: String,
        /// Agent namespace that requested admission.
        agent_namespace: String,
        /// Stream epoch assigned to the newly admitted stream.
        stream_epoch: String,
        /// Capability token that was granted (e.g. `"media-ingress"`).
        capability: String,
        /// UTC microseconds when the grant was recorded.
        timestamp_us: u64,
    },

    /// `MediaIngressOpenResult(admitted=false)` emitted.
    MediaAdmissionDeny {
        /// Session that denied the request.
        session_id: String,
        /// Agent namespace that made the request.
        agent_namespace: String,
        /// Stable rejection code.
        reject_code: MediaRejectCode,
        /// UTC microseconds when the denial was recorded.
        timestamp_us: u64,
    },

    /// `MediaIngressCloseNotice` emitted (any reason).
    MediaStreamClose {
        /// Session the stream belonged to.
        session_id: String,
        /// Agent namespace that owned the stream.
        agent_namespace: String,
        /// Stream epoch that closed.
        stream_epoch: String,
        /// Why the stream closed.
        reason: MediaCloseReason,
        /// UTC microseconds when the close was recorded.
        timestamp_us: u64,
    },

    /// Stream transitioned to `REVOKED` state.
    MediaStreamRevoke {
        /// Session the stream belonged to.
        session_id: String,
        /// Agent namespace that owned the stream.
        agent_namespace: String,
        /// Stream epoch that was revoked.
        stream_epoch: String,
        /// Why the stream was revoked.
        reason: MediaCloseReason,
        /// UTC microseconds when the revocation was recorded.
        timestamp_us: u64,
    },

    /// `MediaDegradationNotice` with non-zero `ladder_step`.
    MediaDegradationStep {
        /// Session the stream belongs to.
        session_id: String,
        /// Agent namespace that owns the stream.
        agent_namespace: String,
        /// Stream epoch on which the step was applied.
        stream_epoch: String,
        /// E25 ladder step reached (1–10; 0 = recovery).
        ladder_step: DegradationStep,
        /// What triggered the step.
        trigger: DegradationTrigger,
        /// UTC microseconds when the step was recorded.
        timestamp_us: u64,
    },

    /// `media-ingress` (or related) capability revoked mid-session.
    MediaCapabilityRevoke {
        /// Session from which the capability was revoked.
        session_id: String,
        /// Agent namespace whose capability was revoked.
        agent_namespace: String,
        /// Capability token that was revoked.
        capability: String,
        /// UTC microseconds when the revocation was recorded.
        timestamp_us: u64,
    },

    /// Pool preemption (RFC 0002 A1 §A3.2).
    MediaPreempt {
        /// Session the preempted stream belonged to.
        session_id: String,
        /// Agent namespace that owned the preempted stream.
        agent_namespace: String,
        /// Stream epoch that was preempted.
        stream_epoch: String,
        /// UTC microseconds when the preemption was recorded.
        timestamp_us: u64,
    },

    /// Operator chrome-level mute/pause/revoke.
    MediaOperatorOverride {
        /// Session the action was applied to.
        session_id: String,
        /// Agent namespace that owned the affected stream.
        agent_namespace: String,
        /// Stream epoch affected (empty string if the action is session-level).
        stream_epoch: String,
        /// Which operator action was taken.
        action: OperatorOverrideKind,
        /// UTC microseconds when the action was recorded.
        timestamp_us: u64,
    },

    // ── RFC 0008 A1 §A5 capability dialog events ─────────────────────────────
    /// Operator granted a C13 capability through the per-session dialog.
    CapabilityDialogGrant {
        /// Session in which the dialog occurred.
        session_id: String,
        /// Agent namespace that requested the capability.
        agent_namespace: String,
        /// Capability token granted (one of the eight C13 tokens).
        capability: String,
        /// Operator principal who granted (local UUID in v2).
        granted_by: String,
        /// UTC microseconds when the grant was recorded.
        granted_at_us: u64,
        /// Whether a 7-day remember record was also written.
        remember_written: bool,
        /// Expiry of the remember record, if `remember_written` is true.
        #[serde(skip_serializing_if = "Option::is_none")]
        remember_expires_at_us: Option<u64>,
    },

    /// 7-day per-agent-per-capability remember record written.
    CapabilityRemember {
        /// Agent namespace the remember record applies to.
        agent_namespace: String,
        /// Capability token remembered.
        capability: String,
        /// Operator principal who granted and chose "remember".
        granted_by: String,
        /// UTC microseconds when the record was written.
        granted_at_us: u64,
        /// UTC microseconds when the record expires (granted_at_us + 7 days).
        expires_at_us: u64,
    },

    /// Operator manually revoked a 7-day remember record.
    CapabilityRememberRevoke {
        /// Agent namespace whose remember record was revoked.
        agent_namespace: String,
        /// Capability token whose remember record was revoked.
        capability: String,
        /// Operator principal who revoked the record.
        revoked_by: String,
        /// UTC microseconds when the revocation was recorded.
        revoked_at_us: u64,
        /// Human-readable reason for revocation.
        reason: String,
    },
}

impl MediaAuditEvent {
    /// Return the stable string event name for this event variant.
    ///
    /// These names are stable and append-only — never rename an existing value.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::MediaAdmissionGrant { .. } => "media_admission_grant",
            Self::MediaAdmissionDeny { .. } => "media_admission_deny",
            Self::MediaStreamClose { .. } => "media_stream_close",
            Self::MediaStreamRevoke { .. } => "media_stream_revoke",
            Self::MediaDegradationStep { .. } => "media_degradation_step",
            Self::MediaCapabilityRevoke { .. } => "media_capability_revoke",
            Self::MediaPreempt { .. } => "media_preempt",
            Self::MediaOperatorOverride { .. } => "media_operator_override",
            Self::CapabilityDialogGrant { .. } => "capability_dialog_grant",
            Self::CapabilityRemember { .. } => "capability_remember",
            Self::CapabilityRememberRevoke { .. } => "capability_remember_revoke",
        }
    }

    /// Return the session_id for events that carry one, or `None` for session-agnostic events.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::MediaAdmissionGrant { session_id, .. }
            | Self::MediaAdmissionDeny { session_id, .. }
            | Self::MediaStreamClose { session_id, .. }
            | Self::MediaStreamRevoke { session_id, .. }
            | Self::MediaDegradationStep { session_id, .. }
            | Self::MediaCapabilityRevoke { session_id, .. }
            | Self::MediaPreempt { session_id, .. }
            | Self::MediaOperatorOverride { session_id, .. }
            | Self::CapabilityDialogGrant { session_id, .. } => Some(session_id.as_str()),
            Self::CapabilityRemember { .. } | Self::CapabilityRememberRevoke { .. } => None,
        }
    }

    /// Return the timestamp_us for this event.
    pub fn timestamp_us(&self) -> u64 {
        match self {
            Self::MediaAdmissionGrant { timestamp_us, .. }
            | Self::MediaAdmissionDeny { timestamp_us, .. }
            | Self::MediaStreamClose { timestamp_us, .. }
            | Self::MediaStreamRevoke { timestamp_us, .. }
            | Self::MediaDegradationStep { timestamp_us, .. }
            | Self::MediaCapabilityRevoke { timestamp_us, .. }
            | Self::MediaPreempt { timestamp_us, .. }
            | Self::MediaOperatorOverride { timestamp_us, .. } => *timestamp_us,
            Self::CapabilityDialogGrant { granted_at_us, .. } => *granted_at_us,
            Self::CapabilityRemember { granted_at_us, .. } => *granted_at_us,
            Self::CapabilityRememberRevoke { revoked_at_us, .. } => *revoked_at_us,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── MediaRejectCode stable string codes ───────────────────────────────────

    #[test]
    fn test_reject_code_stable_strings() {
        assert_eq!(
            MediaRejectCode::CapabilityRequired.as_str(),
            "CAPABILITY_REQUIRED"
        );
        assert_eq!(
            MediaRejectCode::StreamLimitExceeded.as_str(),
            "STREAM_LIMIT_EXCEEDED"
        );
        assert_eq!(
            MediaRejectCode::CapabilityDialogDenied.as_str(),
            "CAPABILITY_DIALOG_DENIED"
        );
        assert_eq!(
            MediaRejectCode::TrustBoundaryViolation.as_str(),
            "TRUST_BOUNDARY_VIOLATION"
        );
        assert_eq!(
            MediaRejectCode::SignalingRateLimitExceeded.as_str(),
            "SIGNALING_RATE_LIMIT_EXCEEDED"
        );
    }

    // ── DegradationStep invariants ─────────────────────────────────────────────

    #[test]
    fn test_degradation_step_nominal() {
        assert!(DegradationStep::NOMINAL.is_nominal());
        assert!(!DegradationStep::new(1).is_nominal());
    }

    #[test]
    fn test_degradation_step_clamped_to_10() {
        assert_eq!(DegradationStep::new(11).0, 10);
        assert_eq!(DegradationStep::new(255).0, 10);
    }

    #[test]
    fn test_degradation_step_labels() {
        assert_eq!(DegradationStep::new(1).label(), "degrade_spatial_audio");
        assert_eq!(DegradationStep::new(2).label(), "reduce_framerate");
        assert_eq!(DegradationStep::new(3).label(), "reduce_resolution");
        assert_eq!(DegradationStep::new(4).label(), "suspend_recording");
        assert_eq!(DegradationStep::new(5).label(), "drop_cloud_relay");
        assert_eq!(DegradationStep::new(6).label(), "drop_second_stream");
        assert_eq!(DegradationStep::new(7).label(), "freeze_and_block_input");
        assert_eq!(
            DegradationStep::new(8).label(),
            "teardown_media_keep_session"
        );
        assert_eq!(DegradationStep::new(9).label(), "revoke_embodied_presence");
        assert_eq!(DegradationStep::new(10).label(), "disconnect");
    }

    // ── MediaAuditEvent serialization ─────────────────────────────────────────

    #[test]
    fn test_media_admission_grant_serializes_correctly() {
        let event = MediaAuditEvent::MediaAdmissionGrant {
            session_id: "sess-abc".to_string(),
            agent_namespace: "agent-x".to_string(),
            stream_epoch: "epoch-1".to_string(),
            capability: "media-ingress".to_string(),
            timestamp_us: 1_000_000,
        };
        let json = serde_json::to_string(&event).expect("serialize should succeed");
        assert!(
            json.contains(r#""event":"media_admission_grant""#),
            "event tag missing: {json}"
        );
        assert!(json.contains(r#""session_id":"sess-abc""#));
        assert!(json.contains(r#""capability":"media-ingress""#));
        assert_eq!(event.event_name(), "media_admission_grant");
    }

    #[test]
    fn test_media_admission_deny_serializes_with_reject_code() {
        let event = MediaAuditEvent::MediaAdmissionDeny {
            session_id: "sess-def".to_string(),
            agent_namespace: "agent-y".to_string(),
            reject_code: MediaRejectCode::StreamLimitExceeded,
            timestamp_us: 2_000_000,
        };
        let json = serde_json::to_string(&event).expect("serialize should succeed");
        assert!(json.contains(r#""event":"media_admission_deny""#), "{json}");
        assert!(json.contains("STREAM_LIMIT_EXCEEDED"), "{json}");
        assert_eq!(event.event_name(), "media_admission_deny");
    }

    #[test]
    fn test_media_degradation_step_event() {
        let event = MediaAuditEvent::MediaDegradationStep {
            session_id: "sess-1".to_string(),
            agent_namespace: "agent-z".to_string(),
            stream_epoch: "epoch-3".to_string(),
            ladder_step: DegradationStep::new(2),
            trigger: DegradationTrigger::RuntimeLadderAdvance,
            timestamp_us: 3_000_000,
        };
        let json = serde_json::to_string(&event).expect("serialize should succeed");
        assert!(
            json.contains(r#""event":"media_degradation_step""#),
            "{json}"
        );
        assert_eq!(event.event_name(), "media_degradation_step");
        assert_eq!(event.timestamp_us(), 3_000_000);
    }

    #[test]
    fn test_capability_dialog_grant_with_remember() {
        let event = MediaAuditEvent::CapabilityDialogGrant {
            session_id: "sess-g".to_string(),
            agent_namespace: "agent-a".to_string(),
            capability: "media-ingress".to_string(),
            granted_by: "operator-uuid".to_string(),
            granted_at_us: 1_234_567_890_000_000,
            remember_written: true,
            remember_expires_at_us: Some(1_235_172_690_000_000),
        };
        let json = serde_json::to_string(&event).expect("serialize should succeed");
        assert!(
            json.contains(r#""event":"capability_dialog_grant""#),
            "{json}"
        );
        assert!(json.contains("remember_expires_at_us"), "{json}");
        assert!(json.contains(r#""remember_written":true"#), "{json}");
        assert_eq!(event.event_name(), "capability_dialog_grant");
        assert_eq!(event.session_id(), Some("sess-g"));
    }

    #[test]
    fn test_capability_dialog_grant_without_remember_omits_expiry_field() {
        let event = MediaAuditEvent::CapabilityDialogGrant {
            session_id: "sess-h".to_string(),
            agent_namespace: "agent-b".to_string(),
            capability: "audio-emit".to_string(),
            granted_by: "operator-uuid-2".to_string(),
            granted_at_us: 1_234_567_890_000_000,
            remember_written: false,
            remember_expires_at_us: None,
        };
        let json = serde_json::to_string(&event).expect("serialize should succeed");
        assert!(
            !json.contains("remember_expires_at_us"),
            "expiry field should be omitted when not written: {json}"
        );
        assert!(json.contains(r#""remember_written":false"#), "{json}");
    }

    #[test]
    fn test_capability_remember_revoke_event() {
        let event = MediaAuditEvent::CapabilityRememberRevoke {
            agent_namespace: "agent-c".to_string(),
            capability: "cloud-relay".to_string(),
            revoked_by: "operator-uuid-3".to_string(),
            revoked_at_us: 5_000_000,
            reason: "operator_manual_revoke".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize should succeed");
        assert!(
            json.contains(r#""event":"capability_remember_revoke""#),
            "{json}"
        );
        // session_id is None for session-agnostic events
        assert_eq!(event.session_id(), None);
    }

    #[test]
    fn test_event_name_matches_serde_tag() {
        // Spot-check that event_name() is consistent with the serde tag value.
        let events = [
            MediaAuditEvent::MediaStreamRevoke {
                session_id: "s".to_string(),
                agent_namespace: "a".to_string(),
                stream_epoch: "e".to_string(),
                reason: MediaCloseReason::CapabilityRevoked,
                timestamp_us: 0,
            },
            MediaAuditEvent::MediaPreempt {
                session_id: "s".to_string(),
                agent_namespace: "a".to_string(),
                stream_epoch: "e".to_string(),
                timestamp_us: 0,
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let name = event.event_name();
            assert!(
                json.contains(&format!(r#""event":"{name}""#)),
                "event_name '{name}' not found in JSON: {json}"
            );
        }
    }
}
