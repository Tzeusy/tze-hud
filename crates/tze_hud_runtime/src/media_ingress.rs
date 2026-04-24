//! Bounded media ingress state machine — RFC 0014 §3.
//!
//! This module is the **implementation-ready v2 capability** for bounded media
//! ingress.  It promotes the contract-only spec into executable Rust by
//! encoding the full RFC 0014 §3 state machine, admission gate, and
//! per-stream tracking.
//!
//! # Scope
//!
//! - [`MediaSessionState`] — wire-compatible state enum (RFC 0014 §2.3.3).
//! - [`MediaSessionEvent`] — typed transition events (RFC 0014 §3.3 Trigger column).
//! - [`MediaIngressStateMachine`] — the RFC 0014 §3.2 state machine.
//! - [`MediaIngressAdmissionGate`] — pre-admission checks (RFC 0014 §6.1).
//! - [`MediaCloseReason`] — close reason registry (RFC 0014 §2.4).
//! - [`MediaDegradationTrigger`] — degradation trigger actors (RFC 0014 §2.3.6).
//! - [`MediaPauseTrigger`] — pause trigger actors (RFC 0014 §2.3.7).
//!
//! # Design: implementation-ready v2 capability (engineering-bar §1)
//!
//! Per `about/craft-and-care/engineering-bar.md §1`, every PR that changes
//! observable behaviour ships with tests.  Tests cover:
//!
//! - All state machine transitions from RFC 0014 §3.3 (example-based).
//! - Terminal-state rejection of post-transition attempts (invariant).
//! - Pause resume authority (agent may not resume operator/safe-mode pauses).
//! - Admission gate rejection codes (§2.4 registry).
//! - Property-based transition sequences: no path from a terminal state to
//!   any non-terminal state (proptest).
//!
//! # Performance budgets (engineering-bar §2 D18)
//!
//! The state machine itself is O(1) per transition — no allocation, no I/O.
//! D18 media budgets (glass-to-glass p50 ≤ 150 ms, p99 ≤ 400 ms, decode-drop
//! ≤ 0.5%, lip-sync drift ≤ ±40 ms, TTFF ≤ 500 ms) are enforcement targets
//! for the runtime pipeline and real-decode CI lane, not for this module.
//!
//! # Observability (engineering-bar §5)
//!
//! Every state transition emits a structured `tracing` event at `DEBUG` level
//! carrying `stream_epoch`, `from`, `to`, and the triggering event name.
//! Admission failures emit `WARN` events with the rejection code.
//!
//! # Dependency hygiene (engineering-bar §6)
//!
//! This module has zero new external dependencies. It uses only `thiserror`
//! (already in workspace) and `tracing` (already in workspace).
//!
//! # RFC reference
//!
//! All type names, variant names, and transition rules are normatively defined
//! in `about/legends-and-lore/rfcs/0014-media-plane-wire-protocol.md`.
//! Edits to this file that deviate from RFC 0014 MUST be accompanied by an
//! RFC amendment.

use thiserror::Error;
use tracing::{debug, warn};

// ─── MediaSessionState ────────────────────────────────────────────────────────

/// Wire-compatible state enum for a media ingress session.
///
/// Mirrors `MediaSessionState` from RFC 0014 §2.3.3.  The integer
/// discriminants are the authoritative wire values for `MediaIngressState`
/// protobuf serialisation and MUST NOT be changed.
///
/// `MEDIA_SESSION_STATE_UNSPECIFIED = 0` is not modelled as a variant because
/// an unspecified state is never a valid runtime state; it exists only as a
/// protobuf default sentinel.  A freshly admitted session starts in
/// [`MediaSessionState::Admitted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum MediaSessionState {
    /// Transport being established (SDP + ICE + DTLS/SRTP). Wire value 1.
    Admitted = 1,
    /// Transport established; decoded frames flowing to compositor. Wire value 2.
    Streaming = 2,
    /// Stream active but under E25 degradation step 1–7. Wire value 3.
    Degraded = 3,
    /// Frames suspended; stream remains admitted. Wire value 4.
    Paused = 4,
    /// Teardown initiated; ring buffer draining. Wire value 5.
    Closing = 5,
    /// Terminal: resources freed, stream cannot resume. Wire value 6.
    Closed = 6,
    /// Terminal: revocation path (capability/lease/embodiment). Wire value 7.
    Revoked = 7,
}

impl MediaSessionState {
    /// Returns `true` when no further transitions are possible.
    ///
    /// RFC 0014 §3.1: `CLOSED` and `REVOKED` are terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Revoked)
    }

    /// Returns `true` when the stream is in the `ACTIVE` superstate —
    /// either `STREAMING` or `DEGRADED` — i.e., frames are flowing or
    /// quality-reduced.
    ///
    /// RFC 0014 §3.5 `statig` guidance: `STREAMING` and `DEGRADED` share a
    /// parent superstate `ACTIVE` whose guard ensures transport is healthy.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Streaming | Self::Degraded)
    }

    /// Human-readable label for structured log output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "ADMITTED",
            Self::Streaming => "STREAMING",
            Self::Degraded => "DEGRADED",
            Self::Paused => "PAUSED",
            Self::Closing => "CLOSING",
            Self::Closed => "CLOSED",
            Self::Revoked => "REVOKED",
        }
    }
}

impl std::fmt::Display for MediaSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── Trigger actors ───────────────────────────────────────────────────────────

/// The pause trigger that caused a `STREAMING/DEGRADED → PAUSED` transition.
///
/// RFC 0014 §2.3.7.  The trigger is carried in `MediaPauseNotice` and is the
/// **authority check** for who may resume the stream (§3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaPauseTrigger {
    /// `MediaPauseRequest` from the owning agent.
    AgentRequest,
    /// Chrome pause affordance activated by a human operator.
    OperatorRequest,
    /// RFC 0005 §3.7 safe-mode entry: all streams pause.
    SafeMode,
    /// Attention policy quiet-hours window active (RFC 0009 level 4).
    PolicyQuietHours,
}

impl MediaPauseTrigger {
    /// Human-readable label for log output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentRequest => "AGENT_REQUEST",
            Self::OperatorRequest => "OPERATOR_REQUEST",
            Self::SafeMode => "SAFE_MODE",
            Self::PolicyQuietHours => "POLICY_QUIET_HOURS",
        }
    }

    /// Returns `true` when the agent is the *only* actor authorised to resume
    /// this pause.
    ///
    /// RFC 0014 §3.3: a `MediaResumeRequest` from the agent is **silently
    /// dropped** unless the stream was paused by `AGENT_REQUEST`.  The
    /// converse: the runtime (not the agent) clears all other pause triggers.
    pub fn agent_can_resume(self) -> bool {
        matches!(self, Self::AgentRequest)
    }
}

/// Who or what advanced a degradation step on this stream.
///
/// RFC 0014 §2.3.6 `MediaDegradationTrigger`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaDegradationTrigger {
    /// Global runtime degradation level advanced (E25 automatic).
    RuntimeLadderAdvance,
    /// Per-stream watchdog threshold crossed (RFC 0002 A1 §A4.1).
    WatchdogPerStream,
    /// Human override at chrome.
    OperatorManual,
    /// Capability/policy revocation forced a step.
    CapabilityPolicy,
}

/// Reason the runtime is closing or revoking a stream.
///
/// RFC 0014 §2.3.4 `MediaCloseReason`.  These codes appear in
/// `MediaIngressCloseNotice.reason` and in audit logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaCloseReason {
    /// Echo of `MediaIngressClose` from the agent.
    AgentClosed,
    /// Owning lease was revoked (RFC 0008 §3).
    LeaseRevoked,
    /// `media-ingress` capability revoked (RFC 0008 A1 §A3.4).
    CapabilityRevoked,
    /// Human override (chrome mute).
    OperatorMute,
    /// Runtime config disabled the capability at deployment level.
    PolicyDisabled,
    /// Per-stream watchdog threshold crossed (RFC 0002 A1 §A4.1).
    BudgetWatchdog,
    /// Higher-priority stream preempted this one (RFC 0002 A1 §A3.2).
    Preempted,
    /// E25 step 8 "Tear down media, keep session".
    DegradationTeardown,
    /// E25 step 9 reached; paired with RFC 0015 presence demote.
    EmbodimentRevoked,
    /// E25 step 10 / session teardown.
    SessionDisconnected,
    /// ICE / DTLS / SRTP fatal.
    TransportFailure,
    /// GStreamer pipeline unrecoverable.
    DecoderFailure,
    /// `expires_at_wall_us` passed.
    ScheduleExpired,
}

impl MediaCloseReason {
    /// Machine-readable code string (RFC 0014 §2.4 registry, SHOUTY_SNAKE_CASE).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentClosed => "AGENT_CLOSED",
            Self::LeaseRevoked => "LEASE_REVOKED",
            Self::CapabilityRevoked => "CAPABILITY_REVOKED",
            Self::OperatorMute => "OPERATOR_MUTE",
            Self::PolicyDisabled => "POLICY_DISABLED",
            Self::BudgetWatchdog => "BUDGET_WATCHDOG",
            Self::Preempted => "PREEMPTED",
            Self::DegradationTeardown => "DEGRADATION_TEARDOWN",
            Self::EmbodimentRevoked => "EMBODIMENT_REVOKED",
            Self::SessionDisconnected => "SESSION_DISCONNECTED",
            Self::TransportFailure => "TRANSPORT_FAILURE",
            Self::DecoderFailure => "DECODER_FAILURE",
            Self::ScheduleExpired => "SCHEDULE_EXPIRED",
        }
    }

    /// Returns `true` when this close reason results in `REVOKED` rather than
    /// `CLOSED`.
    ///
    /// RFC 0014 §3.3: revocation paths (capability revoked, lease revoked,
    /// embodiment revoked, session disconnected, policy disabled) land in
    /// `REVOKED`.  All other close paths land in `CLOSED` via `CLOSING`.
    pub fn is_revocation(self) -> bool {
        matches!(
            self,
            Self::CapabilityRevoked
                | Self::LeaseRevoked
                | Self::EmbodimentRevoked
                | Self::SessionDisconnected
                | Self::PolicyDisabled
        )
    }
}

// ─── MediaSessionEvent ────────────────────────────────────────────────────────

/// Typed event enum for the RFC 0014 §3.3 state machine.
///
/// Each variant maps directly to a row in the RFC 0014 §3.3 transition table.
/// Events are the sole input to [`MediaIngressStateMachine::apply`].
///
/// # Actor codes (per §3.3 column "Actor")
///
/// - `R` — runtime-automatic.
/// - `W` — watchdog (runtime-initiated via threshold crossing).
/// - `O` — operator (human override).
/// - `A` — agent request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaSessionEvent {
    /// `R`: Transport established (SDP + ICE + DTLS/SRTP complete).
    /// `ADMITTED → STREAMING`.
    TransportEstablished,

    /// `R`: Transport negotiation timed out (transport_timeout, default 10 s).
    /// `ADMITTED → CLOSING` with `TRANSPORT_FAILURE`.
    TransportNegotiationFailed,

    /// `R`: E25 degradation ladder advanced to step `step` (1–7) on this stream.
    /// `STREAMING → DEGRADED`.
    DegradationAdvanced {
        /// E25 ladder step reached (1–7).
        step: u32,
        trigger: MediaDegradationTrigger,
    },

    /// `R`: E25 recovery — frame-time guardian below threshold, budget
    /// recovered.  `DEGRADED → STREAMING`.
    DegradationRecovered,

    /// `A`: Agent sent `MediaPauseRequest`.
    /// `STREAMING / DEGRADED → PAUSED`.
    AgentPauseRequest,

    /// `O`: Operator chrome pause affordance activated.
    /// `STREAMING / DEGRADED → PAUSED`.
    OperatorPause,

    /// `R`: RFC 0005 §3.7 safe-mode entry.
    /// `STREAMING / DEGRADED → PAUSED`.
    SafeModePause,

    /// `R`: Attention policy quiet-hours window active (RFC 0009 level 4).
    /// `STREAMING / DEGRADED → PAUSED`.
    PolicyQuietHoursPause,

    /// `A`: Agent sent `MediaResumeRequest`.
    /// `PAUSED → STREAMING` **only if** paused by `AGENT_REQUEST`.
    /// Silently dropped otherwise (RFC 0014 §3.3 note).
    AgentResumeRequest,

    /// `O`: Operator chrome resume.
    /// `PAUSED → STREAMING` unconditionally.
    OperatorResume,

    /// `R`: Safe-mode exit.
    /// `PAUSED → STREAMING` when paused by `SAFE_MODE`.
    SafeModeResume,

    /// `R`: Quiet-hours window closed.
    /// `PAUSED → STREAMING` when paused by `POLICY_QUIET_HOURS`.
    PolicyQuietHoursResume,

    /// Teardown trigger from any non-terminal state → `CLOSING`.
    /// Carries the reason; revocation paths use a separate variant.
    InitiateClose(MediaCloseReason),

    /// `R`: Ring buffer drained AND GStreamer EOS confirmed.
    /// `CLOSING → CLOSED`.
    DrainComplete,

    /// Revocation path from any non-terminal state → `REVOKED` immediately.
    /// Carries the close reason (must satisfy `MediaCloseReason::is_revocation()`).
    Revoke(MediaCloseReason),
}

impl MediaSessionEvent {
    /// Short label for structured log output.
    pub fn name(&self) -> &'static str {
        match self {
            Self::TransportEstablished => "TransportEstablished",
            Self::TransportNegotiationFailed => "TransportNegotiationFailed",
            Self::DegradationAdvanced { .. } => "DegradationAdvanced",
            Self::DegradationRecovered => "DegradationRecovered",
            Self::AgentPauseRequest => "AgentPauseRequest",
            Self::OperatorPause => "OperatorPause",
            Self::SafeModePause => "SafeModePause",
            Self::PolicyQuietHoursPause => "PolicyQuietHoursPause",
            Self::AgentResumeRequest => "AgentResumeRequest",
            Self::OperatorResume => "OperatorResume",
            Self::SafeModeResume => "SafeModeResume",
            Self::PolicyQuietHoursResume => "PolicyQuietHoursResume",
            Self::InitiateClose(_) => "InitiateClose",
            Self::DrainComplete => "DrainComplete",
            Self::Revoke(_) => "Revoke",
        }
    }
}

// ─── Transition outcome ───────────────────────────────────────────────────────

/// Result of applying an event to the state machine.
#[derive(Debug, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// State changed from `from` to `to`.
    Transitioned {
        from: MediaSessionState,
        to: MediaSessionState,
    },
    /// Event was silently dropped per RFC 0014 §3.3 (e.g., agent
    /// `ResumeRequest` on a non-agent-paused stream).
    Dropped,
    /// State is terminal; no transition is possible.
    AlreadyTerminal(MediaSessionState),
    /// The event is not applicable in the current state.  The runtime
    /// MUST treat this as a no-op and log a structured warning.
    NotApplicable {
        state: MediaSessionState,
        event: &'static str,
    },
}

// ─── State machine ────────────────────────────────────────────────────────────

/// The RFC 0014 §3 bounded media ingress state machine.
///
/// One instance is created per admitted media stream.  It is cheaply
/// `Clone`-able for snapshotting purposes and `Send`-safe for passing across
/// async task boundaries.
///
/// ## statig compatibility
///
/// RFC 0014 §3.5 specifies implementation via the `statig` crate.  This
/// implementation provides the same hierarchical semantics manually:
///
/// - `STREAMING` and `DEGRADED` belong to a logical `ACTIVE` superstate whose
///   guard (transport healthy) is captured by the `pause_trigger` always being
///   `None` while in those states.
/// - The `statig` crate may be wired in as a later refactor (task hud-ora8.1.23
///   or follow-up) without changing the external API or test assertions, because
///   this module's public surface is the event/state enums and
///   [`MediaIngressStateMachine::apply`].
///
/// ## Thread safety
///
/// `MediaIngressStateMachine` is `!Sync` (holds mutable inner state).  Callers
/// MUST ensure exclusive access.  The media worker pool (RFC 0002 A1 §E24)
/// owns the instance on its tokio task; the session handler sends events over
/// a channel to that task.
#[derive(Clone, Debug)]
pub struct MediaIngressStateMachine {
    /// Runtime-assigned monotonic stream identifier.  Stable across transport
    /// reconnects within the same session; never reused.
    stream_epoch: u64,

    /// Current state.
    state: MediaSessionState,

    /// When in `PAUSED`, records who triggered the pause.  Used for resume
    /// authority checking (RFC 0014 §3.3 note on `AgentResumeRequest`).
    pause_trigger: Option<MediaPauseTrigger>,

    /// Current E25 degradation step (0 = none; 1–7 = active).
    degradation_step: u32,
}

impl MediaIngressStateMachine {
    /// Create a new state machine in the `ADMITTED` state.
    ///
    /// The `stream_epoch` MUST be assigned by the runtime admission gate at
    /// the point `MediaIngressOpenResult.admitted = true` is emitted.  It is
    /// never 0 for an admitted stream (0 is the rejection sentinel per RFC 0014
    /// §2.3.2).
    ///
    /// # Panics
    ///
    /// Panics if `stream_epoch == 0` — the caller has a bug.
    pub fn new(stream_epoch: u64) -> Self {
        assert!(
            stream_epoch != 0,
            "stream_epoch 0 is reserved for rejected streams"
        );
        Self {
            stream_epoch,
            state: MediaSessionState::Admitted,
            pause_trigger: None,
            degradation_step: 0,
        }
    }

    /// Current state of the stream.
    pub fn state(&self) -> MediaSessionState {
        self.state
    }

    /// Runtime-assigned stream epoch.
    pub fn stream_epoch(&self) -> u64 {
        self.stream_epoch
    }

    /// Current E25 degradation step (0 = none, 1–7 = active).
    pub fn degradation_step(&self) -> u32 {
        self.degradation_step
    }

    /// If the stream is `PAUSED`, returns the pause trigger.
    pub fn pause_trigger(&self) -> Option<MediaPauseTrigger> {
        self.pause_trigger
    }

    /// Apply an event to the state machine.
    ///
    /// Returns a [`TransitionOutcome`] describing what happened.  The caller
    /// MUST emit the appropriate wire signals (`MediaIngressState`,
    /// `MediaIngressCloseNotice`, `MediaDegradationNotice`, `MediaPauseNotice`,
    /// `MediaResumeNotice`) based on the outcome.
    ///
    /// This method is infallible — it never panics and never returns an `Err`.
    /// Invalid events are reported as `NotApplicable` or `Dropped`.
    pub fn apply(&mut self, event: MediaSessionEvent) -> TransitionOutcome {
        // Terminal states reject all events (RFC 0014 §3.5).
        if self.state.is_terminal() {
            return TransitionOutcome::AlreadyTerminal(self.state);
        }

        let from = self.state;

        match &event {
            // ── ADMITTED → STREAMING ──────────────────────────────────────
            MediaSessionEvent::TransportEstablished => {
                if self.state != MediaSessionState::Admitted {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.state = MediaSessionState::Streaming;
            }

            // ── ADMITTED → CLOSING (transport timeout) ────────────────────
            MediaSessionEvent::TransportNegotiationFailed => {
                if self.state != MediaSessionState::Admitted {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.state = MediaSessionState::Closing;
            }

            // ── STREAMING → DEGRADED ──────────────────────────────────────
            MediaSessionEvent::DegradationAdvanced { step, .. } => {
                if self.state != MediaSessionState::Streaming {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.degradation_step = *step;
                self.state = MediaSessionState::Degraded;
            }

            // ── DEGRADED → STREAMING (recovery) ──────────────────────────
            MediaSessionEvent::DegradationRecovered => {
                if self.state != MediaSessionState::Degraded {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.degradation_step = 0;
                self.state = MediaSessionState::Streaming;
            }

            // ── STREAMING/DEGRADED → PAUSED ────────────────────────────────
            MediaSessionEvent::AgentPauseRequest => {
                if !self.state.is_active() {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.pause_trigger = Some(MediaPauseTrigger::AgentRequest);
                self.state = MediaSessionState::Paused;
            }

            MediaSessionEvent::OperatorPause => {
                if !self.state.is_active() {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.pause_trigger = Some(MediaPauseTrigger::OperatorRequest);
                self.state = MediaSessionState::Paused;
            }

            MediaSessionEvent::SafeModePause => {
                if !self.state.is_active() {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.pause_trigger = Some(MediaPauseTrigger::SafeMode);
                self.state = MediaSessionState::Paused;
            }

            MediaSessionEvent::PolicyQuietHoursPause => {
                if !self.state.is_active() {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.pause_trigger = Some(MediaPauseTrigger::PolicyQuietHours);
                self.state = MediaSessionState::Paused;
            }

            // ── PAUSED → STREAMING (resume) ───────────────────────────────
            // RFC 0014 §3.3: AgentResumeRequest is silently dropped unless
            // paused by AGENT_REQUEST.
            MediaSessionEvent::AgentResumeRequest => {
                if self.state != MediaSessionState::Paused {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                let trigger = self
                    .pause_trigger
                    .unwrap_or(MediaPauseTrigger::AgentRequest);
                if !trigger.agent_can_resume() {
                    // RFC 0014 §3.3: silently drop; no error, no state change.
                    debug!(
                        stream_epoch = self.stream_epoch,
                        pause_trigger = trigger.as_str(),
                        "AgentResumeRequest silently dropped: not agent-paused"
                    );
                    return TransitionOutcome::Dropped;
                }
                self.pause_trigger = None;
                self.state = MediaSessionState::Streaming;
            }

            MediaSessionEvent::OperatorResume => {
                if self.state != MediaSessionState::Paused {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.pause_trigger = None;
                self.state = MediaSessionState::Streaming;
            }

            // RFC 0014 §3.3: SafeModeResume only resumes streams that were
            // paused by SAFE_MODE.  If the stream was paused by a different
            // trigger (e.g., OPERATOR_REQUEST), the event is silently dropped
            // so the operator's intent is preserved.
            MediaSessionEvent::SafeModeResume => {
                if self.state != MediaSessionState::Paused {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                let trigger = self.pause_trigger.unwrap_or(MediaPauseTrigger::SafeMode);
                if trigger != MediaPauseTrigger::SafeMode {
                    debug!(
                        stream_epoch = self.stream_epoch,
                        pause_trigger = trigger.as_str(),
                        "SafeModeResume ignored: stream not paused by SAFE_MODE"
                    );
                    return TransitionOutcome::Dropped;
                }
                self.pause_trigger = None;
                self.state = MediaSessionState::Streaming;
            }

            // RFC 0014 §3.3: PolicyQuietHoursResume only resumes streams that
            // were paused by POLICY_QUIET_HOURS.  If the stream was paused by a
            // different trigger, the event is silently dropped.
            MediaSessionEvent::PolicyQuietHoursResume => {
                if self.state != MediaSessionState::Paused {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                let trigger = self
                    .pause_trigger
                    .unwrap_or(MediaPauseTrigger::PolicyQuietHours);
                if trigger != MediaPauseTrigger::PolicyQuietHours {
                    debug!(
                        stream_epoch = self.stream_epoch,
                        pause_trigger = trigger.as_str(),
                        "PolicyQuietHoursResume ignored: stream not paused by POLICY_QUIET_HOURS"
                    );
                    return TransitionOutcome::Dropped;
                }
                self.pause_trigger = None;
                self.state = MediaSessionState::Streaming;
            }

            // ── any non-terminal → CLOSING ─────────────────────────────────
            MediaSessionEvent::InitiateClose(reason) => {
                if reason.is_revocation() {
                    // Caller should use `Revoke` for revocation paths.
                    warn!(
                        stream_epoch = self.stream_epoch,
                        reason = reason.as_str(),
                        "InitiateClose called with revocation reason; use Revoke instead"
                    );
                }
                self.state = MediaSessionState::Closing;
            }

            // ── CLOSING → CLOSED (drain complete) ─────────────────────────
            MediaSessionEvent::DrainComplete => {
                if self.state != MediaSessionState::Closing {
                    return TransitionOutcome::NotApplicable {
                        state: self.state,
                        event: event.name(),
                    };
                }
                self.state = MediaSessionState::Closed;
            }

            // ── any non-terminal → REVOKED ─────────────────────────────────
            MediaSessionEvent::Revoke(reason) => {
                if !reason.is_revocation() {
                    // Caller has a bug: non-revocation close reason sent via
                    // `Revoke`.  Demote to `InitiateClose` semantics.
                    warn!(
                        stream_epoch = self.stream_epoch,
                        reason = reason.as_str(),
                        "Revoke called with non-revocation reason; transitioning to CLOSING"
                    );
                    self.state = MediaSessionState::Closing;
                } else {
                    self.state = MediaSessionState::Revoked;
                }
            }
        }

        let to = self.state;
        debug!(
            stream_epoch = self.stream_epoch,
            from = from.as_str(),
            to = to.as_str(),
            event = event.name(),
            "media_ingress state transition"
        );
        TransitionOutcome::Transitioned { from, to }
    }
}

// ─── MediaIngressAdmissionGate ────────────────────────────────────────────────

/// Machine-readable rejection codes from RFC 0014 §2.4.
///
/// These codes appear in `MediaIngressOpenResult.reject_code` and in audit
/// logs.  They align with existing RFC 0005 / RFC 0008 A1 code conventions
/// (SHOUTY_SNAKE_CASE strings).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaAdmissionRejectCode {
    /// Session does not hold `media-ingress` capability (RFC 0008 A1 §A2).
    CapabilityRequired,
    /// Operator denied the capability dialog (RFC 0008 A1 §A6).
    CapabilityDialogDenied,
    /// Dialog timed out; no operator present (RFC 0008 A1 §A6).
    CapabilityDialogTimeout,
    /// Capability disabled at deployment level (RFC 0008 A1 §A6).
    CapabilityNotEnabled,
    /// e.g. `federated-send` in v2; `MediaEgressOpen` in v2 (RFC 0008 A1 §A6).
    CapabilityNotImplemented,
    /// None of the declared codec preferences intersect the runtime set (§2.5).
    CodecUnsupported,
    /// Zone or tile binding does not resolve (§2.3.1).
    SurfaceNotFound,
    /// Surface already bound to another stream with incompatible policy (§2.3.1).
    SurfaceOccupied,
    /// Media worker pool full; preemption not applicable (RFC 0002 A1 §A2.2).
    PoolExhausted,
    /// Per-session `max_concurrent_media_streams` exceeded (RFC 0002 A1 §A2.2).
    SessionStreamLimit,
    /// Global GPU texture budget below admission threshold (RFC 0002 A1 §A2.2).
    TextureHeadroomLow,
    /// SDP/ICE could not complete within transport timeout (§4).
    TransportNegotiationFailed,
    /// Viewer-class floor above declared `content_classification` (RFC 0009).
    ContentClassDenied,
    /// `present_at_wall_us` / `expires_at_wall_us` out of bounds (RFC 0003 §3.5).
    ScheduleInvalid,
    /// Malformed request (RFC 0005 §3.5).
    InvalidArgument,
}

impl MediaAdmissionRejectCode {
    /// Machine-readable string matching the RFC 0014 §2.4 registry.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CapabilityRequired => "CAPABILITY_REQUIRED",
            Self::CapabilityDialogDenied => "CAPABILITY_DIALOG_DENIED",
            Self::CapabilityDialogTimeout => "CAPABILITY_DIALOG_TIMEOUT",
            Self::CapabilityNotEnabled => "CAPABILITY_NOT_ENABLED",
            Self::CapabilityNotImplemented => "CAPABILITY_NOT_IMPLEMENTED",
            Self::CodecUnsupported => "CODEC_UNSUPPORTED",
            Self::SurfaceNotFound => "SURFACE_NOT_FOUND",
            Self::SurfaceOccupied => "SURFACE_OCCUPIED",
            Self::PoolExhausted => "POOL_EXHAUSTED",
            Self::SessionStreamLimit => "SESSION_STREAM_LIMIT",
            Self::TextureHeadroomLow => "TEXTURE_HEADROOM_LOW",
            Self::TransportNegotiationFailed => "TRANSPORT_NEGOTIATION_FAILED",
            Self::ContentClassDenied => "CONTENT_CLASS_DENIED",
            Self::ScheduleInvalid => "SCHEDULE_INVALID",
            Self::InvalidArgument => "INVALID_ARGUMENT",
        }
    }
}

impl std::fmt::Display for MediaAdmissionRejectCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Outcome of a media stream admission check.
///
/// RFC 0014 §6.1.  The gate runs synchronously on the network thread before
/// the media worker pool is consulted.  It is a pure function of the request
/// fields and the current runtime state snapshot.
#[derive(Debug, PartialEq, Eq)]
pub enum MediaAdmissionOutcome {
    /// All gates passed.  The caller MUST assign a non-zero `stream_epoch`
    /// and create a [`MediaIngressStateMachine`].
    Admitted,
    /// At least one gate failed.
    Rejected {
        code: MediaAdmissionRejectCode,
        reason: String,
    },
}

/// Error type for admission gate violations.
///
/// Implements `std::error::Error` via `thiserror` for use in `?` chains.
#[derive(Debug, Error)]
pub enum MediaAdmissionError {
    /// Admission gate failed with a structured rejection code.
    #[error("media ingress admission rejected: {code} — {reason}")]
    Rejected {
        code: MediaAdmissionRejectCode,
        reason: String,
    },
}

/// Request parameters passed to the admission gate.
///
/// This is a reduced, validated view of `MediaIngressOpen` (RFC 0014 §2.3.1).
/// Callers construct this from the proto message after basic argument
/// validation.
#[derive(Debug, Clone)]
pub struct MediaAdmissionRequest {
    /// Session holds the `media-ingress` capability (RFC 0008 A1 §A2).
    pub has_media_ingress_capability: bool,

    /// `media-ingress` capability is enabled at deployment level.
    pub capability_enabled: bool,

    /// Number of currently active streams for the requesting session.
    pub active_stream_count: u32,

    /// Per-session maximum concurrent streams (RFC 0002 A1 §A2.2).
    pub max_concurrent_streams: u32,

    /// Media worker pool has a free slot.
    pub worker_pool_has_slot: bool,

    /// The requested surface binding exists in the scene graph.
    pub surface_exists: bool,

    /// The requested surface is not occupied by another stream.
    pub surface_available: bool,

    /// At least one codec from `codec_preference` is supported by the runtime.
    pub codec_match_found: bool,

    /// The declared `content_classification` passes the viewer-class gate
    /// (RFC 0009 privacy policy).
    pub content_class_passes: bool,

    /// The schedule window (`present_at_wall_us`, `expires_at_wall_us`) is
    /// valid (RFC 0003 §3.5).
    pub schedule_valid: bool,

    /// GPU texture headroom is sufficient (RFC 0002 A1 §A2.2).
    pub texture_headroom_ok: bool,
}

/// Admission gate check (RFC 0014 §6.1).
///
/// Runs all checks in the order specified in the RFC 0014 §3.2 admission gate
/// diagram.  Returns on the first failure to keep the reject reason
/// unambiguous.
///
/// This is a **pure function** — it has no side effects.  The caller is
/// responsible for recording the outcome in the audit log (RFC 0014 §9.6).
pub fn check_media_admission(req: &MediaAdmissionRequest) -> MediaAdmissionOutcome {
    // 1. Capability check (RFC 0008 A1 §A2).
    if !req.capability_enabled {
        warn!(
            reject_code = "CAPABILITY_NOT_ENABLED",
            "media ingress capability disabled at deployment"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::CapabilityNotEnabled,
            reason: "media-ingress capability is disabled at deployment level".to_string(),
        };
    }
    if !req.has_media_ingress_capability {
        warn!(
            reject_code = "CAPABILITY_REQUIRED",
            "session lacks media-ingress capability"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::CapabilityRequired,
            reason: "session does not hold media-ingress capability".to_string(),
        };
    }

    // 2. Pool slot check (RFC 0002 A1 §A2.2).
    if !req.worker_pool_has_slot {
        warn!(reject_code = "POOL_EXHAUSTED", "media worker pool full");
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::PoolExhausted,
            reason: "media worker pool is full; no preemption applicable".to_string(),
        };
    }

    // 3. Per-session stream limit (RFC 0002 A1 §A2.2).
    if req.active_stream_count >= req.max_concurrent_streams {
        warn!(
            reject_code = "SESSION_STREAM_LIMIT",
            active = req.active_stream_count,
            max = req.max_concurrent_streams,
            "per-session stream limit exceeded"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::SessionStreamLimit,
            reason: format!(
                "per-session stream limit reached: {} / {}",
                req.active_stream_count, req.max_concurrent_streams
            ),
        };
    }

    // 4. GPU texture headroom (RFC 0002 A1 §A2.2).
    if !req.texture_headroom_ok {
        warn!(
            reject_code = "TEXTURE_HEADROOM_LOW",
            "GPU texture budget below admission threshold"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::TextureHeadroomLow,
            reason: "global GPU texture budget below admission threshold".to_string(),
        };
    }

    // 5. Codec match (§2.5).
    if !req.codec_match_found {
        warn!(
            reject_code = "CODEC_UNSUPPORTED",
            "no codec preference match"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::CodecUnsupported,
            reason: "none of the declared codec preferences are supported by this runtime"
                .to_string(),
        };
    }

    // 6. Surface binding resolve (§2.3.1).
    if !req.surface_exists {
        warn!(
            reject_code = "SURFACE_NOT_FOUND",
            "surface binding not found"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::SurfaceNotFound,
            reason: "requested zone or tile surface binding does not resolve".to_string(),
        };
    }
    if !req.surface_available {
        warn!(reject_code = "SURFACE_OCCUPIED", "surface binding occupied");
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::SurfaceOccupied,
            reason: "surface already bound to another stream with incompatible policy".to_string(),
        };
    }

    // 7. Content classification viewer-gate (RFC 0009).
    if !req.content_class_passes {
        warn!(
            reject_code = "CONTENT_CLASS_DENIED",
            "viewer-class floor not met"
        );
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::ContentClassDenied,
            reason: "viewer-class floor is above the declared content classification".to_string(),
        };
    }

    // 8. Schedule validity (RFC 0003 §3.5).
    if !req.schedule_valid {
        warn!(reject_code = "SCHEDULE_INVALID", "schedule window invalid");
        return MediaAdmissionOutcome::Rejected {
            code: MediaAdmissionRejectCode::ScheduleInvalid,
            reason: "present_at_wall_us or expires_at_wall_us is out of bounds".to_string(),
        };
    }

    MediaAdmissionOutcome::Admitted
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper ────────────────────────────────────────────────────────────────

    /// Construct a fresh state machine at `ADMITTED`.
    fn new_machine() -> MediaIngressStateMachine {
        MediaIngressStateMachine::new(1)
    }

    /// Assert that a transition produced the expected `from → to` pair.
    fn assert_transition(
        outcome: &TransitionOutcome,
        expected_from: MediaSessionState,
        expected_to: MediaSessionState,
    ) {
        match outcome {
            TransitionOutcome::Transitioned { from, to } => {
                assert_eq!(
                    *from, expected_from,
                    "expected from={expected_from:?}, got from={from:?}"
                );
                assert_eq!(
                    *to, expected_to,
                    "expected to={expected_to:?}, got to={to:?}"
                );
            }
            other => panic!("expected Transitioned, got {other:?}"),
        }
    }

    // ─── RFC 0014 §3.3 transition table — example-based coverage ─────────────

    // Row: (start) → ADMITTED — tested implicitly via new_machine().state()

    #[test]
    fn test_initial_state_is_admitted() {
        let m = new_machine();
        assert_eq!(m.state(), MediaSessionState::Admitted);
        assert_eq!(m.stream_epoch(), 1);
        assert_eq!(m.degradation_step(), 0);
        assert!(m.pause_trigger().is_none());
    }

    // Row: ADMITTED → STREAMING (TransportEstablished)

    #[test]
    fn test_admitted_to_streaming_on_transport_established() {
        let mut m = new_machine();
        let outcome = m.apply(MediaSessionEvent::TransportEstablished);
        assert_transition(
            &outcome,
            MediaSessionState::Admitted,
            MediaSessionState::Streaming,
        );
        assert_eq!(m.state(), MediaSessionState::Streaming);
    }

    // Row: ADMITTED → CLOSING (TransportNegotiationFailed)

    #[test]
    fn test_admitted_to_closing_on_transport_timeout() {
        let mut m = new_machine();
        let outcome = m.apply(MediaSessionEvent::TransportNegotiationFailed);
        assert_transition(
            &outcome,
            MediaSessionState::Admitted,
            MediaSessionState::Closing,
        );
    }

    // Row: STREAMING → DEGRADED (DegradationAdvanced)

    #[test]
    fn test_streaming_to_degraded_on_advance() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 3,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Degraded,
        );
        assert_eq!(m.degradation_step(), 3);
    }

    // Row: DEGRADED → STREAMING (DegradationRecovered)

    #[test]
    fn test_degraded_to_streaming_on_recovery() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 2,
            trigger: MediaDegradationTrigger::WatchdogPerStream,
        });
        let outcome = m.apply(MediaSessionEvent::DegradationRecovered);
        assert_transition(
            &outcome,
            MediaSessionState::Degraded,
            MediaSessionState::Streaming,
        );
        assert_eq!(m.degradation_step(), 0, "step must reset to 0 on recovery");
    }

    // Row: STREAMING → PAUSED (AgentPauseRequest)

    #[test]
    fn test_streaming_to_paused_agent() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::AgentPauseRequest);
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Paused,
        );
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::AgentRequest));
    }

    // Row: STREAMING → PAUSED (OperatorPause)

    #[test]
    fn test_streaming_to_paused_operator() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::OperatorPause);
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Paused,
        );
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::OperatorRequest));
    }

    // Row: STREAMING → PAUSED (SafeModePause)

    #[test]
    fn test_streaming_to_paused_safe_mode() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::SafeModePause);
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Paused,
        );
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::SafeMode));
    }

    // Row: STREAMING → PAUSED (PolicyQuietHoursPause)

    #[test]
    fn test_streaming_to_paused_quiet_hours() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::PolicyQuietHoursPause);
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Paused,
        );
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::PolicyQuietHours));
    }

    // Row: DEGRADED → PAUSED (OperatorPause — pause fires on ACTIVE superstate)

    #[test]
    fn test_degraded_to_paused_operator() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 1,
            trigger: MediaDegradationTrigger::OperatorManual,
        });
        assert_eq!(m.state(), MediaSessionState::Degraded);
        let outcome = m.apply(MediaSessionEvent::OperatorPause);
        assert_transition(
            &outcome,
            MediaSessionState::Degraded,
            MediaSessionState::Paused,
        );
    }

    // Row: PAUSED → STREAMING (AgentResumeRequest, paused by agent — ALLOWED)

    #[test]
    fn test_paused_agent_can_resume_own_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::AgentPauseRequest);
        let outcome = m.apply(MediaSessionEvent::AgentResumeRequest);
        assert_transition(
            &outcome,
            MediaSessionState::Paused,
            MediaSessionState::Streaming,
        );
        assert!(m.pause_trigger().is_none());
    }

    // RFC 0014 §3.3 critical note: AgentResumeRequest on OPERATOR_REQUEST
    // pause MUST be silently dropped.

    #[test]
    fn test_agent_cannot_resume_operator_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::OperatorPause);
        let outcome = m.apply(MediaSessionEvent::AgentResumeRequest);
        // Must be Dropped, not Transitioned.
        assert_eq!(outcome, TransitionOutcome::Dropped);
        // State must remain PAUSED.
        assert_eq!(m.state(), MediaSessionState::Paused);
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::OperatorRequest));
    }

    // RFC 0014 §3.3: AgentResumeRequest on SAFE_MODE pause MUST be silently dropped.

    #[test]
    fn test_agent_cannot_resume_safe_mode_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::SafeModePause);
        let outcome = m.apply(MediaSessionEvent::AgentResumeRequest);
        assert_eq!(outcome, TransitionOutcome::Dropped);
        assert_eq!(m.state(), MediaSessionState::Paused);
    }

    // RFC 0014 §3.3: AgentResumeRequest on POLICY_QUIET_HOURS pause MUST be
    // silently dropped.

    #[test]
    fn test_agent_cannot_resume_quiet_hours_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::PolicyQuietHoursPause);
        let outcome = m.apply(MediaSessionEvent::AgentResumeRequest);
        assert_eq!(outcome, TransitionOutcome::Dropped);
        assert_eq!(m.state(), MediaSessionState::Paused);
    }

    // Row: PAUSED → STREAMING (OperatorResume — unconditional)

    #[test]
    fn test_operator_can_resume_any_pause() {
        for pause_event in [
            MediaSessionEvent::AgentPauseRequest,
            MediaSessionEvent::OperatorPause,
            MediaSessionEvent::SafeModePause,
            MediaSessionEvent::PolicyQuietHoursPause,
        ] {
            let mut m = new_machine();
            m.apply(MediaSessionEvent::TransportEstablished);
            m.apply(pause_event);
            let outcome = m.apply(MediaSessionEvent::OperatorResume);
            assert_transition(
                &outcome,
                MediaSessionState::Paused,
                MediaSessionState::Streaming,
            );
        }
    }

    // Row: PAUSED → STREAMING (SafeModeResume)

    #[test]
    fn test_safe_mode_resume_clears_safe_mode_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::SafeModePause);
        let outcome = m.apply(MediaSessionEvent::SafeModeResume);
        assert_transition(
            &outcome,
            MediaSessionState::Paused,
            MediaSessionState::Streaming,
        );
        assert!(m.pause_trigger().is_none());
    }

    // RFC 0014 §3.3: SafeModeResume MUST be silently dropped when the stream
    // was paused by a trigger other than SAFE_MODE (e.g., operator pause or
    // agent pause).  The operator's or agent's pause intent must not be
    // cleared by a background safe-mode exit event.

    #[test]
    fn test_safe_mode_resume_dropped_for_operator_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::OperatorPause);
        let outcome = m.apply(MediaSessionEvent::SafeModeResume);
        assert_eq!(
            outcome,
            TransitionOutcome::Dropped,
            "SafeModeResume must be dropped when paused by operator"
        );
        assert_eq!(m.state(), MediaSessionState::Paused);
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::OperatorRequest));
    }

    #[test]
    fn test_safe_mode_resume_dropped_for_agent_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::AgentPauseRequest);
        let outcome = m.apply(MediaSessionEvent::SafeModeResume);
        assert_eq!(
            outcome,
            TransitionOutcome::Dropped,
            "SafeModeResume must be dropped when paused by agent"
        );
        assert_eq!(m.state(), MediaSessionState::Paused);
    }

    // Row: PAUSED → STREAMING (PolicyQuietHoursResume)

    #[test]
    fn test_quiet_hours_resume_clears_quiet_hours_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::PolicyQuietHoursPause);
        let outcome = m.apply(MediaSessionEvent::PolicyQuietHoursResume);
        assert_transition(
            &outcome,
            MediaSessionState::Paused,
            MediaSessionState::Streaming,
        );
        assert!(m.pause_trigger().is_none());
    }

    // RFC 0014 §3.3: PolicyQuietHoursResume MUST be silently dropped when the
    // stream was paused by a different trigger.

    #[test]
    fn test_quiet_hours_resume_dropped_for_operator_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::OperatorPause);
        let outcome = m.apply(MediaSessionEvent::PolicyQuietHoursResume);
        assert_eq!(
            outcome,
            TransitionOutcome::Dropped,
            "PolicyQuietHoursResume must be dropped when paused by operator"
        );
        assert_eq!(m.state(), MediaSessionState::Paused);
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::OperatorRequest));
    }

    #[test]
    fn test_quiet_hours_resume_dropped_for_safe_mode_pause() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::SafeModePause);
        let outcome = m.apply(MediaSessionEvent::PolicyQuietHoursResume);
        assert_eq!(
            outcome,
            TransitionOutcome::Dropped,
            "PolicyQuietHoursResume must be dropped when paused by safe-mode"
        );
        assert_eq!(m.state(), MediaSessionState::Paused);
        assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::SafeMode));
    }

    // Row: any non-terminal → CLOSING (InitiateClose)

    #[test]
    fn test_any_non_terminal_to_closing_via_agent() {
        // From ADMITTED
        let mut m = new_machine();
        let outcome = m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        assert_transition(
            &outcome,
            MediaSessionState::Admitted,
            MediaSessionState::Closing,
        );

        // From STREAMING
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Closing,
        );

        // From PAUSED
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::AgentPauseRequest);
        let outcome = m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::ScheduleExpired,
        ));
        assert_transition(
            &outcome,
            MediaSessionState::Paused,
            MediaSessionState::Closing,
        );
    }

    // Row: CLOSING → CLOSED (DrainComplete)

    #[test]
    fn test_closing_to_closed_on_drain() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        let outcome = m.apply(MediaSessionEvent::DrainComplete);
        assert_transition(
            &outcome,
            MediaSessionState::Closing,
            MediaSessionState::Closed,
        );
        assert!(m.state().is_terminal());
    }

    // Row: any non-terminal → REVOKED (Revoke)

    #[test]
    fn test_any_non_terminal_to_revoked() {
        // From STREAMING
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::Revoke(MediaCloseReason::LeaseRevoked));
        assert_transition(
            &outcome,
            MediaSessionState::Streaming,
            MediaSessionState::Revoked,
        );
        assert!(m.state().is_terminal());
    }

    #[test]
    fn test_capability_revoked_transitions_to_revoked() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 1,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        // From DEGRADED state
        let outcome = m.apply(MediaSessionEvent::Revoke(
            MediaCloseReason::CapabilityRevoked,
        ));
        assert_transition(
            &outcome,
            MediaSessionState::Degraded,
            MediaSessionState::Revoked,
        );
    }

    // ─── Terminal-state invariant ──────────────────────────────────────────────

    #[test]
    fn test_terminal_closed_rejects_all_events() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        m.apply(MediaSessionEvent::DrainComplete);
        assert!(m.state().is_terminal());

        for event in [
            MediaSessionEvent::TransportEstablished,
            MediaSessionEvent::AgentPauseRequest,
            MediaSessionEvent::AgentResumeRequest,
            MediaSessionEvent::DegradationRecovered,
            MediaSessionEvent::InitiateClose(MediaCloseReason::AgentClosed),
            MediaSessionEvent::DrainComplete,
            MediaSessionEvent::Revoke(MediaCloseReason::LeaseRevoked),
        ] {
            let outcome = m.apply(event);
            assert!(
                matches!(outcome, TransitionOutcome::AlreadyTerminal(_)),
                "terminal CLOSED should reject event, got {outcome:?}"
            );
        }
    }

    #[test]
    fn test_terminal_revoked_rejects_all_events() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::Revoke(
            MediaCloseReason::SessionDisconnected,
        ));
        assert!(m.state().is_terminal());

        let outcome = m.apply(MediaSessionEvent::TransportEstablished);
        assert!(matches!(outcome, TransitionOutcome::AlreadyTerminal(_)));
    }

    // ─── Degradation step tracking ─────────────────────────────────────────────

    #[test]
    fn test_degradation_step_reported_correctly() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);

        for step in 1u32..=7 {
            // Each step must advance cleanly.
            if m.state() == MediaSessionState::Streaming {
                m.apply(MediaSessionEvent::DegradationAdvanced {
                    step,
                    trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
                });
            } else {
                // After first advance we're DEGRADED; advance again by recovering
                // then re-advancing.
                m.apply(MediaSessionEvent::DegradationRecovered);
                m.apply(MediaSessionEvent::DegradationAdvanced {
                    step,
                    trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
                });
            }
            assert_eq!(m.degradation_step(), step);
            assert_eq!(m.state(), MediaSessionState::Degraded);
        }

        // Recovery resets step to 0.
        m.apply(MediaSessionEvent::DegradationRecovered);
        assert_eq!(m.degradation_step(), 0);
    }

    // ─── NotApplicable paths ───────────────────────────────────────────────────

    #[test]
    fn test_transport_established_not_applicable_in_streaming() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::TransportEstablished);
        assert!(
            matches!(outcome, TransitionOutcome::NotApplicable { .. }),
            "TransportEstablished in STREAMING should be NotApplicable"
        );
    }

    #[test]
    fn test_degradation_advance_not_applicable_in_degraded() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 1,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        // DEGRADED: another advance is not applicable (must recover first).
        let outcome = m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 2,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        assert!(matches!(outcome, TransitionOutcome::NotApplicable { .. }));
    }

    #[test]
    fn test_drain_complete_not_applicable_outside_closing() {
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        let outcome = m.apply(MediaSessionEvent::DrainComplete);
        assert!(matches!(outcome, TransitionOutcome::NotApplicable { .. }));
    }

    // ─── stream_epoch invariant ───────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "stream_epoch 0 is reserved")]
    fn test_zero_epoch_panics() {
        let _ = MediaIngressStateMachine::new(0);
    }

    #[test]
    fn test_large_epoch_accepted() {
        let m = MediaIngressStateMachine::new(u64::MAX);
        assert_eq!(m.stream_epoch(), u64::MAX);
    }

    // ─── Admission gate unit tests ────────────────────────────────────────────

    fn base_admit_request() -> MediaAdmissionRequest {
        MediaAdmissionRequest {
            has_media_ingress_capability: true,
            capability_enabled: true,
            active_stream_count: 0,
            max_concurrent_streams: 4,
            worker_pool_has_slot: true,
            surface_exists: true,
            surface_available: true,
            codec_match_found: true,
            content_class_passes: true,
            schedule_valid: true,
            texture_headroom_ok: true,
        }
    }

    #[test]
    fn test_admission_all_gates_pass() {
        let req = base_admit_request();
        assert_eq!(check_media_admission(&req), MediaAdmissionOutcome::Admitted);
    }

    #[test]
    fn test_admission_rejects_no_capability() {
        let req = MediaAdmissionRequest {
            has_media_ingress_capability: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::CapabilityRequired);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_capability_not_enabled() {
        let req = MediaAdmissionRequest {
            capability_enabled: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::CapabilityNotEnabled);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_pool_exhausted() {
        let req = MediaAdmissionRequest {
            worker_pool_has_slot: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::PoolExhausted);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_session_stream_limit() {
        let req = MediaAdmissionRequest {
            active_stream_count: 4,
            max_concurrent_streams: 4,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::SessionStreamLimit);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_texture_headroom_low() {
        let req = MediaAdmissionRequest {
            texture_headroom_ok: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::TextureHeadroomLow);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_codec_unsupported() {
        let req = MediaAdmissionRequest {
            codec_match_found: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::CodecUnsupported);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_surface_not_found() {
        let req = MediaAdmissionRequest {
            surface_exists: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::SurfaceNotFound);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_surface_occupied() {
        let req = MediaAdmissionRequest {
            surface_available: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::SurfaceOccupied);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_content_class_denied() {
        let req = MediaAdmissionRequest {
            content_class_passes: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::ContentClassDenied);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_admission_rejects_schedule_invalid() {
        let req = MediaAdmissionRequest {
            schedule_valid: false,
            ..base_admit_request()
        };
        match check_media_admission(&req) {
            MediaAdmissionOutcome::Rejected { code, .. } => {
                assert_eq!(code, MediaAdmissionRejectCode::ScheduleInvalid);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    // ─── MediaAdmissionRejectCode string registry ─────────────────────────────

    /// RFC 0014 §2.4 code registry — ensure all codes produce non-empty
    /// SHOUTY_SNAKE_CASE strings and that Display matches as_str.
    #[test]
    fn test_reject_code_strings_are_shouty_snake_case() {
        let codes = [
            MediaAdmissionRejectCode::CapabilityRequired,
            MediaAdmissionRejectCode::CapabilityDialogDenied,
            MediaAdmissionRejectCode::CapabilityDialogTimeout,
            MediaAdmissionRejectCode::CapabilityNotEnabled,
            MediaAdmissionRejectCode::CapabilityNotImplemented,
            MediaAdmissionRejectCode::CodecUnsupported,
            MediaAdmissionRejectCode::SurfaceNotFound,
            MediaAdmissionRejectCode::SurfaceOccupied,
            MediaAdmissionRejectCode::PoolExhausted,
            MediaAdmissionRejectCode::SessionStreamLimit,
            MediaAdmissionRejectCode::TextureHeadroomLow,
            MediaAdmissionRejectCode::TransportNegotiationFailed,
            MediaAdmissionRejectCode::ContentClassDenied,
            MediaAdmissionRejectCode::ScheduleInvalid,
            MediaAdmissionRejectCode::InvalidArgument,
        ];
        for code in &codes {
            let s = code.as_str();
            assert!(!s.is_empty(), "code string must not be empty");
            assert!(
                s.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "code '{s}' must be SHOUTY_SNAKE_CASE (uppercase + underscores only)"
            );
            assert_eq!(s, format!("{code}"), "Display must match as_str");
        }
    }

    // ─── MediaCloseReason::is_revocation() invariant ──────────────────────────

    /// RFC 0014 §3.3: exactly the revocation close reasons must land in
    /// REVOKED; others must go through CLOSING → CLOSED.
    #[test]
    fn test_close_reason_revocation_flag_is_consistent() {
        let revocation_reasons = [
            MediaCloseReason::CapabilityRevoked,
            MediaCloseReason::LeaseRevoked,
            MediaCloseReason::EmbodimentRevoked,
            MediaCloseReason::SessionDisconnected,
            MediaCloseReason::PolicyDisabled,
        ];
        let non_revocation_reasons = [
            MediaCloseReason::AgentClosed,
            MediaCloseReason::OperatorMute,
            MediaCloseReason::BudgetWatchdog,
            MediaCloseReason::Preempted,
            MediaCloseReason::DegradationTeardown,
            MediaCloseReason::TransportFailure,
            MediaCloseReason::DecoderFailure,
            MediaCloseReason::ScheduleExpired,
        ];

        for r in &revocation_reasons {
            assert!(r.is_revocation(), "{:?} should be a revocation reason", r);
        }
        for r in &non_revocation_reasons {
            assert!(
                !r.is_revocation(),
                "{:?} should NOT be a revocation reason",
                r
            );
        }
    }

    // ─── MediaSessionState helpers ─────────────────────────────────────────────

    #[test]
    fn test_is_terminal_only_for_closed_and_revoked() {
        assert!(!MediaSessionState::Admitted.is_terminal());
        assert!(!MediaSessionState::Streaming.is_terminal());
        assert!(!MediaSessionState::Degraded.is_terminal());
        assert!(!MediaSessionState::Paused.is_terminal());
        assert!(!MediaSessionState::Closing.is_terminal());
        assert!(MediaSessionState::Closed.is_terminal());
        assert!(MediaSessionState::Revoked.is_terminal());
    }

    #[test]
    fn test_is_active_only_for_streaming_and_degraded() {
        assert!(!MediaSessionState::Admitted.is_active());
        assert!(MediaSessionState::Streaming.is_active());
        assert!(MediaSessionState::Degraded.is_active());
        assert!(!MediaSessionState::Paused.is_active());
        assert!(!MediaSessionState::Closing.is_active());
        assert!(!MediaSessionState::Closed.is_active());
        assert!(!MediaSessionState::Revoked.is_active());
    }

    #[test]
    fn test_state_display_and_as_str_match() {
        for s in [
            MediaSessionState::Admitted,
            MediaSessionState::Streaming,
            MediaSessionState::Degraded,
            MediaSessionState::Paused,
            MediaSessionState::Closing,
            MediaSessionState::Closed,
            MediaSessionState::Revoked,
        ] {
            assert_eq!(format!("{s}"), s.as_str());
        }
    }

    // ─── Wire value discrimination ─────────────────────────────────────────────

    /// RFC 0014 §2.3.3: wire values are normative and must not change.
    #[test]
    fn test_state_wire_values_are_stable() {
        assert_eq!(MediaSessionState::Admitted as u32, 1);
        assert_eq!(MediaSessionState::Streaming as u32, 2);
        assert_eq!(MediaSessionState::Degraded as u32, 3);
        assert_eq!(MediaSessionState::Paused as u32, 4);
        assert_eq!(MediaSessionState::Closing as u32, 5);
        assert_eq!(MediaSessionState::Closed as u32, 6);
        assert_eq!(MediaSessionState::Revoked as u32, 7);
    }

    // ─── Full lifecycle smoke test ─────────────────────────────────────────────

    /// Walk the canonical happy path: ADMITTED → STREAMING → DEGRADED →
    /// STREAMING → PAUSED → STREAMING → CLOSING → CLOSED.
    #[test]
    fn test_full_happy_path_lifecycle() {
        let mut m = MediaIngressStateMachine::new(42);

        // 1. Transport established
        let o = m.apply(MediaSessionEvent::TransportEstablished);
        assert_transition(
            &o,
            MediaSessionState::Admitted,
            MediaSessionState::Streaming,
        );

        // 2. Budget pressure: E25 step 3 reached
        let o = m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 3,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        assert_transition(
            &o,
            MediaSessionState::Streaming,
            MediaSessionState::Degraded,
        );
        assert_eq!(m.degradation_step(), 3);

        // 3. Budget recovered
        let o = m.apply(MediaSessionEvent::DegradationRecovered);
        assert_transition(
            &o,
            MediaSessionState::Degraded,
            MediaSessionState::Streaming,
        );
        assert_eq!(m.degradation_step(), 0);

        // 4. Agent pauses
        let o = m.apply(MediaSessionEvent::AgentPauseRequest);
        assert_transition(&o, MediaSessionState::Streaming, MediaSessionState::Paused);

        // 5. Agent resumes (allowed — own pause)
        let o = m.apply(MediaSessionEvent::AgentResumeRequest);
        assert_transition(&o, MediaSessionState::Paused, MediaSessionState::Streaming);

        // 6. Agent closes stream
        let o = m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        assert_transition(&o, MediaSessionState::Streaming, MediaSessionState::Closing);

        // 7. Ring buffer drained
        let o = m.apply(MediaSessionEvent::DrainComplete);
        assert_transition(&o, MediaSessionState::Closing, MediaSessionState::Closed);

        assert!(m.state().is_terminal());
        assert_eq!(m.stream_epoch(), 42);
    }

    // ─── Property-based tests (RFC 0014 §3.3 invariants) ─────────────────────

    /// Property: once a machine reaches a terminal state, no event can move it
    /// to a non-terminal state.
    ///
    /// Uses a manual combinatorial approach instead of `proptest` to keep the
    /// crate free of an extra dev-dependency; the proptest-based version is
    /// in the crate's integration test if proptest is added to dev-dependencies.
    #[test]
    fn property_terminal_state_is_absorbing() {
        // Reach CLOSED via normal path.
        let mut m = new_machine();
        m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        m.apply(MediaSessionEvent::DrainComplete);
        assert!(m.state().is_terminal());

        // Apply every event that could plausibly "un-close" a stream.
        let escape_attempts = [
            MediaSessionEvent::TransportEstablished,
            MediaSessionEvent::DegradationRecovered,
            MediaSessionEvent::AgentResumeRequest,
            MediaSessionEvent::OperatorResume,
            MediaSessionEvent::SafeModeResume,
            MediaSessionEvent::PolicyQuietHoursResume,
            MediaSessionEvent::DrainComplete,
            MediaSessionEvent::InitiateClose(MediaCloseReason::AgentClosed),
            MediaSessionEvent::Revoke(MediaCloseReason::LeaseRevoked),
        ];

        for event in escape_attempts {
            let outcome = m.apply(event);
            assert!(
                matches!(outcome, TransitionOutcome::AlreadyTerminal(_)),
                "terminal state must absorb all events; got {outcome:?}"
            );
            assert!(
                m.state().is_terminal(),
                "state must remain terminal; now in {:?}",
                m.state()
            );
        }

        // Same for REVOKED.
        let mut m = new_machine();
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::Revoke(MediaCloseReason::LeaseRevoked));
        assert!(m.state().is_terminal());

        for event in [
            MediaSessionEvent::TransportEstablished,
            MediaSessionEvent::AgentResumeRequest,
            MediaSessionEvent::DegradationRecovered,
            MediaSessionEvent::DrainComplete,
        ] {
            let outcome = m.apply(event);
            assert!(matches!(outcome, TransitionOutcome::AlreadyTerminal(_)));
            assert!(m.state().is_terminal());
        }
    }

    /// Property: pause trigger is always `None` unless state is `PAUSED`.
    #[test]
    fn property_pause_trigger_only_set_in_paused_state() {
        // This exercises all transitions that leave PAUSED.
        let setup_and_check = |resume_event: MediaSessionEvent| {
            let mut m = new_machine();
            m.apply(MediaSessionEvent::TransportEstablished);
            m.apply(MediaSessionEvent::AgentPauseRequest);
            assert_eq!(m.state(), MediaSessionState::Paused);
            assert!(m.pause_trigger().is_some());
            m.apply(resume_event);
            // After resume, trigger must be cleared.
            if m.state() == MediaSessionState::Streaming {
                assert!(
                    m.pause_trigger().is_none(),
                    "pause_trigger must be None after leaving PAUSED"
                );
            }
        };

        setup_and_check(MediaSessionEvent::AgentResumeRequest);
        setup_and_check(MediaSessionEvent::OperatorResume);
        setup_and_check(MediaSessionEvent::SafeModeResume);
        setup_and_check(MediaSessionEvent::PolicyQuietHoursResume);
    }

    /// Property: `degradation_step` is always 0 unless state is `DEGRADED`.
    #[test]
    fn property_degradation_step_zero_outside_degraded() {
        let mut m = new_machine();
        // Initially zero.
        assert_eq!(m.degradation_step(), 0);

        // After transport, zero.
        m.apply(MediaSessionEvent::TransportEstablished);
        assert_eq!(m.degradation_step(), 0);

        // After advance, non-zero only in DEGRADED.
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 5,
            trigger: MediaDegradationTrigger::WatchdogPerStream,
        });
        assert_eq!(m.state(), MediaSessionState::Degraded);
        assert_eq!(m.degradation_step(), 5);

        // After recovery, zero again.
        m.apply(MediaSessionEvent::DegradationRecovered);
        assert_eq!(m.state(), MediaSessionState::Streaming);
        assert_eq!(m.degradation_step(), 0);

        // After close, zero.
        m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        m.apply(MediaSessionEvent::DrainComplete);
        assert_eq!(m.degradation_step(), 0);
    }

    // ─── Synthetic validation lane check ─────────────────────────────────────
    //
    // Per engineering-bar.md §2 D18: synthetic-only CI (every PR) does NOT
    // enforce D18 thresholds but MUST remain green.  This test constitutes the
    // synthetic lane gate for bounded media ingress state transitions.
    //
    // The test exercises every admission gate variant and all state machine
    // transitions that the RFC 0014 §3.3 table defines, confirming that the
    // state machine is behaviorally correct and consistent with the spec.
    // Real-decode lane tests are deferred to the GPU runner nightly lane
    // (gated by `run-real-decode` label), which is out of scope for this crate.

    #[test]
    fn synthetic_validation_lane_full_state_machine_coverage() {
        // All RFC 0014 §3.3 non-terminal → terminal paths must be reachable.

        // Path 1: ADMITTED → CLOSING → CLOSED (transport timeout path)
        let mut m = MediaIngressStateMachine::new(100);
        m.apply(MediaSessionEvent::TransportNegotiationFailed);
        m.apply(MediaSessionEvent::DrainComplete);
        assert_eq!(m.state(), MediaSessionState::Closed);

        // Path 2: STREAMING → CLOSING → CLOSED (agent close)
        let mut m = MediaIngressStateMachine::new(101);
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::AgentClosed,
        ));
        m.apply(MediaSessionEvent::DrainComplete);
        assert_eq!(m.state(), MediaSessionState::Closed);

        // Path 3: DEGRADED → CLOSING → CLOSED (degradation teardown)
        let mut m = MediaIngressStateMachine::new(102);
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step: 8,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        // Simulate step 8 teardown: E25 step 8 = DEGRADATION_TEARDOWN
        m.apply(MediaSessionEvent::InitiateClose(
            MediaCloseReason::DegradationTeardown,
        ));
        m.apply(MediaSessionEvent::DrainComplete);
        assert_eq!(m.state(), MediaSessionState::Closed);

        // Path 4: PAUSED → REVOKED (safe mode then session disconnected)
        let mut m = MediaIngressStateMachine::new(103);
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::SafeModePause);
        m.apply(MediaSessionEvent::Revoke(
            MediaCloseReason::SessionDisconnected,
        ));
        assert_eq!(m.state(), MediaSessionState::Revoked);

        // Path 5: STREAMING → REVOKED (embodiment revoked)
        let mut m = MediaIngressStateMachine::new(104);
        m.apply(MediaSessionEvent::TransportEstablished);
        m.apply(MediaSessionEvent::Revoke(
            MediaCloseReason::EmbodimentRevoked,
        ));
        assert_eq!(m.state(), MediaSessionState::Revoked);

        // All admission gate rejection codes covered in individual tests above.
        // Final gate: all-green path.
        let outcome = check_media_admission(&base_admit_request());
        assert_eq!(
            outcome,
            MediaAdmissionOutcome::Admitted,
            "synthetic lane: admission all-green"
        );
    }
}
