//! Property-based tests for `MediaIngressStateMachine` — RFC 0014 §3 invariants.
//!
//! These tests use `proptest` to explore arbitrary event sequences and verify
//! that the state machine upholds its correctness invariants under combinatorial
//! input.  They complement the example-based tests in `media_ingress.rs`.
//!
//! # Covered invariants
//!
//! 1. **Terminal absorption** (RFC 0014 §3.1): once the machine reaches
//!    `CLOSED` or `REVOKED`, every subsequent event must produce
//!    `TransitionOutcome::AlreadyTerminal`.
//!
//! 2. **AGENT_REQUEST authority** (RFC 0014 §3.3 §3.3 note): from `PAUSED`,
//!    `AgentResumeRequest` yields `Dropped` unless `pause_trigger ==
//!    AGENT_REQUEST`.  Conversely, it produces a valid `Transitioned` outcome
//!    when the pause was caused by the agent.
//!
//! 3. **SAFE_MODE / POLICY_QUIET_HOURS resume authority**: `SafeModeResume` and
//!    `PolicyQuietHoursResume` are dropped unless the pause trigger matches;
//!    `OperatorResume` is always accepted from `PAUSED` (unconditional).
//!
//! 4. **Pause→Resume cycle state cleanliness**: `pause_trigger` and
//!    `degradation_step` never leak across transition cycles.
//!
//! 5. **stream_epoch == 0 sentinel**: `MediaIngressStateMachine::new(0)` panics.

use std::panic;

use proptest::prelude::*;
use tze_hud_runtime::{
    MediaCloseReason, MediaDegradationTrigger, MediaIngressStateMachine, MediaPauseTrigger,
    MediaSessionEvent, MediaSessionState, TransitionOutcome,
};

// ─── Arbitrary event strategy ─────────────────────────────────────────────────

/// Returns a `BoxedStrategy` that generates arbitrary `MediaSessionEvent`
/// values.  The distribution is flat across all variants (weighted toward
/// simple events; structured variants like `DegradationAdvanced` and
/// `InitiateClose` use their own sub-strategies).
fn arb_event() -> BoxedStrategy<MediaSessionEvent> {
    prop_oneof![
        // Simple unit-like events.
        Just(MediaSessionEvent::TransportEstablished),
        Just(MediaSessionEvent::TransportNegotiationFailed),
        Just(MediaSessionEvent::DegradationRecovered),
        Just(MediaSessionEvent::AgentPauseRequest),
        Just(MediaSessionEvent::OperatorPause),
        Just(MediaSessionEvent::SafeModePause),
        Just(MediaSessionEvent::PolicyQuietHoursPause),
        Just(MediaSessionEvent::AgentResumeRequest),
        Just(MediaSessionEvent::OperatorResume),
        Just(MediaSessionEvent::SafeModeResume),
        Just(MediaSessionEvent::PolicyQuietHoursResume),
        Just(MediaSessionEvent::DrainComplete),
        // DegradationAdvanced — random step 1–7.
        (1u32..=7u32, arb_degradation_trigger()).prop_map(|(step, trigger)| {
            MediaSessionEvent::DegradationAdvanced { step, trigger }
        }),
        // InitiateClose — all close reasons; retry_after_us follows BudgetWatchdog
        // semantics: Some(1) for BudgetWatchdog, None otherwise.
        arb_close_reason().prop_map(|reason| {
            let retry_after_us = if reason == MediaCloseReason::BudgetWatchdog {
                Some(1_000_000)
            } else {
                None
            };
            MediaSessionEvent::InitiateClose {
                reason,
                retry_after_us,
            }
        }),
        // Revoke — all close reasons (machine handles non-revocation ones too).
        arb_close_reason().prop_map(MediaSessionEvent::Revoke),
    ]
    .boxed()
}

fn arb_degradation_trigger() -> BoxedStrategy<MediaDegradationTrigger> {
    prop_oneof![
        Just(MediaDegradationTrigger::RuntimeLadderAdvance),
        Just(MediaDegradationTrigger::WatchdogPerStream),
        Just(MediaDegradationTrigger::OperatorManual),
        Just(MediaDegradationTrigger::CapabilityPolicy),
    ]
    .boxed()
}

fn arb_close_reason() -> BoxedStrategy<MediaCloseReason> {
    prop_oneof![
        Just(MediaCloseReason::AgentClosed),
        Just(MediaCloseReason::LeaseRevoked),
        Just(MediaCloseReason::CapabilityRevoked),
        Just(MediaCloseReason::OperatorMute),
        Just(MediaCloseReason::PolicyDisabled),
        Just(MediaCloseReason::BudgetWatchdog),
        Just(MediaCloseReason::Preempted),
        Just(MediaCloseReason::DegradationTeardown),
        Just(MediaCloseReason::EmbodimentRevoked),
        Just(MediaCloseReason::SessionDisconnected),
        Just(MediaCloseReason::TransportFailure),
        Just(MediaCloseReason::DecoderFailure),
        Just(MediaCloseReason::ScheduleExpired),
    ]
    .boxed()
}

/// Pause-trigger events as a strategy (events that lead ACTIVE → PAUSED).
fn arb_pause_event() -> BoxedStrategy<(MediaSessionEvent, MediaPauseTrigger)> {
    prop_oneof![
        Just((
            MediaSessionEvent::AgentPauseRequest,
            MediaPauseTrigger::AgentRequest
        )),
        Just((
            MediaSessionEvent::OperatorPause,
            MediaPauseTrigger::OperatorRequest
        )),
        Just((
            MediaSessionEvent::SafeModePause,
            MediaPauseTrigger::SafeMode
        )),
        Just((
            MediaSessionEvent::PolicyQuietHoursPause,
            MediaPauseTrigger::PolicyQuietHours
        )),
    ]
    .boxed()
}

// ─── Helper: advance machine to STREAMING ─────────────────────────────────────

/// Bring a fresh machine from `ADMITTED` to `STREAMING` by applying
/// `TransportEstablished`.
fn into_streaming(epoch: u64) -> MediaIngressStateMachine {
    let mut m = MediaIngressStateMachine::new(epoch);
    let outcome = m.apply(MediaSessionEvent::TransportEstablished);
    assert!(
        matches!(outcome, TransitionOutcome::Transitioned { .. }),
        "setup: expected ADMITTED→STREAMING, got {outcome:?}"
    );
    assert_eq!(m.state(), MediaSessionState::Streaming);
    m
}

// ─── Proptest 1: Terminal absorption ─────────────────────────────────────────
//
// Invariant: once CLOSED or REVOKED is reached via any event sequence, every
// subsequent event must yield `AlreadyTerminal`.

proptest! {
    /// Reach CLOSED via `InitiateClose` + `DrainComplete`, then verify that
    /// an arbitrary sequence of further events all produce `AlreadyTerminal`.
    #[test]
    fn prop_terminal_closed_absorbs_any_event_sequence(
        events in prop::collection::vec(arb_event(), 0..20)
    ) {
        let mut m = MediaIngressStateMachine::new(1);
        // Drive to CLOSED.
        m.apply(MediaSessionEvent::InitiateClose { reason: MediaCloseReason::AgentClosed, retry_after_us: None });
        m.apply(MediaSessionEvent::DrainComplete);
        assert!(m.state().is_terminal(), "setup: machine must be terminal");

        // Every subsequent event must be absorbed.
        for event in events {
            let outcome = m.apply(event);
            prop_assert!(
                matches!(outcome, TransitionOutcome::AlreadyTerminal(_)),
                "CLOSED must absorb all events; got {outcome:?}"
            );
            prop_assert!(
                m.state().is_terminal(),
                "state must remain terminal after event; now {:?}",
                m.state()
            );
        }
    }

    /// Same invariant via the REVOKED terminal state.
    #[test]
    fn prop_terminal_revoked_absorbs_any_event_sequence(
        events in prop::collection::vec(arb_event(), 0..20)
    ) {
        let mut m = into_streaming(2);
        // Drive to REVOKED.
        m.apply(MediaSessionEvent::Revoke(MediaCloseReason::LeaseRevoked));
        assert!(m.state().is_terminal(), "setup: machine must be terminal");

        for event in events {
            let outcome = m.apply(event);
            prop_assert!(
                matches!(outcome, TransitionOutcome::AlreadyTerminal(_)),
                "REVOKED must absorb all events; got {outcome:?}"
            );
            prop_assert!(
                m.state().is_terminal(),
                "state must remain terminal after event; now {:?}",
                m.state()
            );
        }
    }

    /// Drive through an arbitrary prefix of events until terminal, then verify
    /// absorption.  This finds terminal states that are reachable via arbitrary
    /// (not just canonical) paths.
    #[test]
    fn prop_terminal_absorption_after_arbitrary_prefix(
        prefix in prop::collection::vec(arb_event(), 1..30),
        suffix in prop::collection::vec(arb_event(), 1..20),
    ) {
        let mut m = MediaIngressStateMachine::new(1);
        // Apply the prefix.
        for event in prefix {
            m.apply(event);
        }
        // If terminal, every suffix event must be absorbed.
        if m.state().is_terminal() {
            let terminal_state = m.state();
            for event in suffix {
                let outcome = m.apply(event);
                prop_assert!(
                    matches!(outcome, TransitionOutcome::AlreadyTerminal(_)),
                    "terminal {terminal_state:?} must absorb event; got {outcome:?}"
                );
                prop_assert_eq!(
                    m.state(),
                    terminal_state,
                    "terminal state must be stable"
                );
            }
        }
        // Non-terminal case: the property is trivially satisfied (no assertion
        // to check), so we accept without failing.
    }
}

// ─── Proptest 2: AGENT_REQUEST authority (§3.3) ───────────────────────────────
//
// Invariant: `AgentResumeRequest` from PAUSED is `Dropped` iff the pause
// trigger is NOT `AGENT_REQUEST`.  When the pause trigger IS `AGENT_REQUEST`,
// it must produce a `Transitioned` outcome to STREAMING.

proptest! {
    /// For each pause trigger, applying `AgentResumeRequest` must either
    /// produce `Dropped` (non-agent trigger) or `Transitioned` (agent trigger).
    #[test]
    fn prop_agent_resume_authority_matches_pause_trigger(
        (pause_event, expected_trigger) in arb_pause_event()
    ) {
        let mut m = into_streaming(42);
        m.apply(pause_event);
        assert_eq!(m.state(), MediaSessionState::Paused, "setup: must be PAUSED");

        let outcome = m.apply(MediaSessionEvent::AgentResumeRequest);

        if expected_trigger == MediaPauseTrigger::AgentRequest {
            // Agent-caused pause: resume must succeed.
            prop_assert!(
                matches!(outcome, TransitionOutcome::Transitioned {
                    from: MediaSessionState::Paused,
                    to: MediaSessionState::Streaming,
                }),
                "AgentResumeRequest on AGENT_REQUEST pause must transition to STREAMING; got {outcome:?}"
            );
            prop_assert_eq!(m.state(), MediaSessionState::Streaming);
            prop_assert!(m.pause_trigger().is_none(), "pause_trigger must be cleared after resume");
        } else {
            // Non-agent pause: resume must be silently dropped.
            prop_assert!(
                outcome == TransitionOutcome::Dropped,
                "AgentResumeRequest on {:?} pause must be Dropped; got {:?}",
                expected_trigger, outcome
            );
            prop_assert!(
                m.state() == MediaSessionState::Paused,
                "state must remain PAUSED; got {:?}", m.state()
            );
            prop_assert!(
                m.pause_trigger() == Some(expected_trigger),
                "pause_trigger must be unchanged after Dropped AgentResumeRequest; got {:?}",
                m.pause_trigger()
            );
        }
    }
}

// ─── Proptest 3: SAFE_MODE / POLICY_QUIET_HOURS resume authority ──────────────
//
// `SafeModeResume` is Dropped unless pause_trigger == SAFE_MODE.
// `PolicyQuietHoursResume` is Dropped unless pause_trigger == POLICY_QUIET_HOURS.
// `OperatorResume` always succeeds from PAUSED (unconditional authority).

proptest! {
    #[test]
    fn prop_safe_mode_resume_authority(
        (pause_event, expected_trigger) in arb_pause_event()
    ) {
        let mut m = into_streaming(7);
        m.apply(pause_event);
        assert_eq!(m.state(), MediaSessionState::Paused);

        let outcome = m.apply(MediaSessionEvent::SafeModeResume);

        if expected_trigger == MediaPauseTrigger::SafeMode {
            prop_assert!(
                matches!(outcome, TransitionOutcome::Transitioned {
                    from: MediaSessionState::Paused,
                    to: MediaSessionState::Streaming,
                }),
                "SafeModeResume on SAFE_MODE pause must transition to STREAMING; got {outcome:?}"
            );
            prop_assert_eq!(m.state(), MediaSessionState::Streaming);
            prop_assert!(m.pause_trigger().is_none());
        } else {
            prop_assert!(
                outcome == TransitionOutcome::Dropped,
                "SafeModeResume on {:?} pause must be Dropped; got {:?}",
                expected_trigger, outcome
            );
            prop_assert_eq!(m.state(), MediaSessionState::Paused);
            prop_assert!(
                m.pause_trigger() == Some(expected_trigger),
                "pause_trigger must be unchanged; got {:?}", m.pause_trigger()
            );
        }
    }

    #[test]
    fn prop_policy_quiet_hours_resume_authority(
        (pause_event, expected_trigger) in arb_pause_event()
    ) {
        let mut m = into_streaming(8);
        m.apply(pause_event);
        assert_eq!(m.state(), MediaSessionState::Paused);

        let outcome = m.apply(MediaSessionEvent::PolicyQuietHoursResume);

        if expected_trigger == MediaPauseTrigger::PolicyQuietHours {
            prop_assert!(
                matches!(outcome, TransitionOutcome::Transitioned {
                    from: MediaSessionState::Paused,
                    to: MediaSessionState::Streaming,
                }),
                "PolicyQuietHoursResume on POLICY_QUIET_HOURS pause must succeed; got {outcome:?}"
            );
            prop_assert_eq!(m.state(), MediaSessionState::Streaming);
            prop_assert!(m.pause_trigger().is_none());
        } else {
            prop_assert!(
                outcome == TransitionOutcome::Dropped,
                "PolicyQuietHoursResume on {:?} pause must be Dropped; got {:?}",
                expected_trigger, outcome
            );
            prop_assert_eq!(m.state(), MediaSessionState::Paused);
            prop_assert!(
                m.pause_trigger() == Some(expected_trigger),
                "pause_trigger must be unchanged; got {:?}", m.pause_trigger()
            );
        }
    }

    /// OperatorResume is unconditional: from PAUSED it always produces
    /// `Transitioned` regardless of which trigger caused the pause.
    #[test]
    fn prop_operator_resume_is_unconditional(
        (pause_event, _trigger) in arb_pause_event()
    ) {
        let mut m = into_streaming(9);
        m.apply(pause_event);
        assert_eq!(m.state(), MediaSessionState::Paused);

        let outcome = m.apply(MediaSessionEvent::OperatorResume);
        prop_assert!(
            matches!(outcome, TransitionOutcome::Transitioned {
                from: MediaSessionState::Paused,
                to: MediaSessionState::Streaming,
            }),
            "OperatorResume must always transition PAUSED→STREAMING; got {outcome:?}"
        );
        prop_assert_eq!(m.state(), MediaSessionState::Streaming);
        prop_assert!(
            m.pause_trigger().is_none(),
            "pause_trigger must be cleared after OperatorResume"
        );
    }
}

// ─── Proptest 4: Pause→Resume cycle state cleanliness ────────────────────────
//
// After any pause→resume cycle, `pause_trigger` must be `None` in STREAMING,
// and `degradation_step` must reflect only the current state (0 in STREAMING).

proptest! {
    /// Multiple pause→resume cycles must not leak state.
    ///
    /// Repeats `n` full agent-pause / agent-resume cycles and verifies
    /// `pause_trigger` and `degradation_step` after each cycle.
    #[test]
    fn prop_pause_resume_cycles_leave_no_state_leakage(n in 1usize..=8) {
        let mut m = into_streaming(100);

        for cycle in 0..n {
            // Pre-condition: in STREAMING, no pause_trigger, step=0.
            prop_assert!(
                m.state() == MediaSessionState::Streaming,
                "before cycle {}: must be STREAMING; got {:?}", cycle, m.state()
            );
            prop_assert!(
                m.pause_trigger().is_none(),
                "before cycle {}: pause_trigger must be None in STREAMING; got {:?}",
                cycle, m.pause_trigger()
            );
            prop_assert!(
                m.degradation_step() == 0,
                "before cycle {}: degradation_step must be 0 in STREAMING; got {}",
                cycle, m.degradation_step()
            );

            // Pause.
            let pause_outcome = m.apply(MediaSessionEvent::AgentPauseRequest);
            prop_assert!(
                matches!(pause_outcome, TransitionOutcome::Transitioned { .. }),
                "cycle {}: AgentPauseRequest must transition; got {:?}", cycle, pause_outcome
            );
            prop_assert_eq!(m.state(), MediaSessionState::Paused);
            prop_assert_eq!(m.pause_trigger(), Some(MediaPauseTrigger::AgentRequest));

            // Resume.
            let resume_outcome = m.apply(MediaSessionEvent::AgentResumeRequest);
            prop_assert!(
                matches!(resume_outcome, TransitionOutcome::Transitioned { .. }),
                "cycle {}: AgentResumeRequest must transition; got {:?}", cycle, resume_outcome
            );
            prop_assert_eq!(m.state(), MediaSessionState::Streaming);
            prop_assert!(
                m.pause_trigger().is_none(),
                "after cycle {}: pause_trigger must be None; got {:?}", cycle, m.pause_trigger()
            );
        }
    }

    /// Degrade→recover→pause→resume cycle: degradation_step and pause_trigger
    /// must each be 0 / None at the end of their respective cycles.
    #[test]
    fn prop_degradation_and_pause_fields_dont_leak(
        step in 1u32..=7u32,
        (pause_event, _trigger) in arb_pause_event(),
    ) {
        let mut m = into_streaming(200);

        // Degrade.
        m.apply(MediaSessionEvent::DegradationAdvanced {
            step,
            trigger: MediaDegradationTrigger::RuntimeLadderAdvance,
        });
        prop_assert_eq!(m.state(), MediaSessionState::Degraded);
        prop_assert_eq!(m.degradation_step(), step);

        // Recover.
        m.apply(MediaSessionEvent::DegradationRecovered);
        prop_assert_eq!(m.state(), MediaSessionState::Streaming);
        prop_assert_eq!(
            m.degradation_step(),
            0,
            "degradation_step must be 0 after recovery"
        );
        prop_assert!(m.pause_trigger().is_none());

        // Pause with an arbitrary trigger.
        m.apply(pause_event);
        prop_assert_eq!(m.state(), MediaSessionState::Paused);
        prop_assert!(m.pause_trigger().is_some());
        prop_assert_eq!(
            m.degradation_step(),
            0,
            "degradation_step must remain 0 in PAUSED"
        );

        // OperatorResume is unconditional — always works regardless of trigger.
        m.apply(MediaSessionEvent::OperatorResume);
        prop_assert_eq!(m.state(), MediaSessionState::Streaming);
        prop_assert!(
            m.pause_trigger().is_none(),
            "pause_trigger must be None after OperatorResume"
        );
        prop_assert_eq!(m.degradation_step(), 0, "degradation_step must still be 0");
    }
}

// ─── Proptest 5: stream_epoch == 0 sentinel ───────────────────────────────────
//
// RFC 0014 §2.3.2: `stream_epoch 0` is the rejection sentinel; constructing
// a state machine with epoch 0 must panic.

#[test]
fn prop_zero_epoch_always_panics() {
    let result = panic::catch_unwind(|| {
        MediaIngressStateMachine::new(0);
    });
    assert!(
        result.is_err(),
        "MediaIngressStateMachine::new(0) must panic (stream_epoch 0 is the rejection sentinel)"
    );
}

proptest! {
    /// Any non-zero epoch must be accepted (no panic).
    #[test]
    fn prop_nonzero_epoch_always_accepted(epoch in 1u64..=u64::MAX) {
        let m = MediaIngressStateMachine::new(epoch);
        prop_assert_eq!(m.stream_epoch(), epoch);
        prop_assert_eq!(m.state(), MediaSessionState::Admitted);
    }
}

// ─── Proptest 6: pause_trigger invariant across arbitrary event sequences ─────
//
// `pause_trigger` must be Some(_) iff `state == PAUSED`.

proptest! {
    #[test]
    fn prop_pause_trigger_some_iff_paused(
        events in prop::collection::vec(arb_event(), 0..30)
    ) {
        let mut m = MediaIngressStateMachine::new(1);
        for event in events {
            m.apply(event);
            let state = m.state();
            let trigger = m.pause_trigger();
            if state == MediaSessionState::Paused {
                prop_assert!(
                    trigger.is_some(),
                    "pause_trigger must be Some when state is PAUSED; state={state:?}, trigger={trigger:?}"
                );
            } else {
                prop_assert!(
                    trigger.is_none(),
                    "pause_trigger must be None when state is not PAUSED; state={state:?}, trigger={trigger:?}"
                );
            }
        }
    }

    /// `degradation_step` must be non-zero iff `state == DEGRADED`; all other
    /// states must have `degradation_step == 0`.
    ///
    /// Pausing from DEGRADED resets the E25 ladder step to 0 so that PAUSED,
    /// CLOSING, CLOSED, REVOKED, and STREAMING are always clean.
    #[test]
    fn prop_degradation_step_nonzero_iff_degraded(
        events in prop::collection::vec(arb_event(), 0..30)
    ) {
        let mut m = MediaIngressStateMachine::new(1);
        for event in events {
            m.apply(event);
            let state = m.state();
            let step = m.degradation_step();
            if state == MediaSessionState::Degraded {
                prop_assert!(
                    step > 0,
                    "degradation_step must be > 0 in DEGRADED; got {}", step
                );
            } else {
                prop_assert!(
                    step == 0,
                    "degradation_step must be 0 in {:?}; got {}", state, step
                );
            }
        }
    }
}
