//! Concrete implementation of the `LeaseStateMachine` trait.
//!
//! `LeaseImpl<C>` holds per-lease state and implements every transition and
//! query defined in the trait contract.  Clock injection via `C: Clock`
//! enables deterministic testing.
//!
//! ## Audit events
//!
//! Every successful state transition appends a [`LeaseAuditEvent`] to an
//! internal buffer.  Callers drain that buffer via [`LeaseStateMachine::drain_events`]
//! and route the events to `lease_changes` subscribers.  Failed transitions
//! (those returning `Err`) produce **no** event.

use super::types::{
    DenyReason, LeaseAuditEvent, LeaseEventKind, LeaseId, LeaseIdentity,
    RevokeReason as AuditRevokeReason,
};
use super::{
    BudgetTier, LeaseState, LeaseStateMachine, RenewalPolicy, RevokeReason, TransitionError,
};
use crate::clock::Clock;
use crate::types::SceneId;

// ─── Budget tier time thresholds ─────────────────────────────────────────────

/// Warning → Throttle transition if warning unresolved for 5 seconds.
const THROTTLE_AFTER_MS: u64 = 5_000;
/// Throttle → Revocation transition if throttle sustained for 30 seconds.
const REVOCATION_AFTER_MS: u64 = 30_000;
/// Default grace period for orphaned leases (ms).
const DEFAULT_GRACE_PERIOD_MS: u64 = 30_000;

// ─── LeaseImpl ───────────────────────────────────────────────────────────────

/// Concrete state machine for a single lease.
///
/// Implements `LeaseStateMachine<C>`.  All time-dependent queries go through
/// the injected `clock`, enabling deterministic tests via `TestClock`.
pub struct LeaseImpl<C: Clock> {
    clock: C,
    state: LeaseState,
    renewal_policy: RenewalPolicy,

    /// Lease identifier.  Nil until the caller assigns an identity via
    /// [`set_lease_id`][LeaseImpl::set_lease_id].  Events carry whatever value
    /// is stored here at transition time.
    lease_id: LeaseId,

    /// When the lease was activated (ms since epoch).  Used for TTL accounting.
    activated_at_ms: u64,
    /// Original TTL in ms (0 = indefinite).
    ttl_ms: u64,
    /// Accumulated suspension time (ms) deducted from effective TTL.
    total_suspension_ms: u64,

    /// Timestamp when current suspension started, if currently SUSPENDED.
    suspended_at_ms: Option<u64>,
    /// TTL remaining at the moment of suspension (ms).  Used to restore TTL on resume.
    ttl_remaining_at_suspend_ms: Option<u64>,

    /// Timestamp when the lease entered ORPHANED state (ms).
    orphaned_at_ms: Option<u64>,
    /// Grace period duration (ms).
    grace_period_ms: u64,

    // ── Budget enforcement ──────────────────────────────────────────────────
    budget_tier: BudgetTier,
    /// Timestamp when budget first entered Warning tier (ms).
    warning_started_ms: Option<u64>,
    /// Timestamp when budget entered Throttle tier (ms).
    throttle_started_ms: Option<u64>,

    // ── Audit event buffer ──────────────────────────────────────────────────
    /// Pending audit events produced by successful transitions.
    ///
    /// Drained by callers via `drain_events()`.  Never grows unboundedly because
    /// the caller is expected to drain after each transition.
    pending_events: Vec<LeaseAuditEvent>,
}

impl<C: Clock> LeaseImpl<C> {
    /// Current wall-clock time from the injected clock.
    fn now_ms(&self) -> u64 {
        self.clock.now_millis()
    }

    /// Current wall-clock time in microseconds from the injected clock.
    fn now_us(&self) -> u64 {
        self.clock.now_us()
    }

    /// Returns the renewal policy for this lease.
    ///
    /// Used by the session layer to arm the AUTO_RENEW timer at 75% TTL elapsed
    /// and to determine behavior on TTL expiry.
    pub fn renewal_policy(&self) -> RenewalPolicy {
        self.renewal_policy
    }

    /// Assign a `LeaseId` to this state machine.
    ///
    /// Must be called before `activate()` so that the emitted `Granted` event
    /// carries the correct identity.  The lease ID is assigned by the runtime
    /// at grant time (UUIDv7, time-ordered).
    pub fn set_lease_id(&mut self, id: LeaseId) {
        self.lease_id = id;
    }

    /// Returns the current lease id.
    pub fn lease_id(&self) -> LeaseId {
        self.lease_id
    }

    /// Effective remaining TTL accounting for all suspension pauses.
    ///
    /// When `ttl_ms == 0` (indefinite) this always returns `u64::MAX` (no expiry).
    fn effective_remaining_ms_at(&self, now_ms: u64) -> u64 {
        if self.ttl_ms == 0 {
            return u64::MAX; // indefinite
        }
        match self.state {
            LeaseState::Suspended => {
                // TTL frozen: return what was saved at suspension entry.
                self.ttl_remaining_at_suspend_ms.unwrap_or(0)
            }
            _ => {
                // effective_elapsed = (now - activated_at) - total_suspension
                let elapsed = now_ms.saturating_sub(self.activated_at_ms);
                let effective_elapsed = elapsed.saturating_sub(self.total_suspension_ms);
                self.ttl_ms.saturating_sub(effective_elapsed)
            }
        }
    }

    /// Re-evaluate budget tier time transitions (Warning→Throttle→Revocation).
    fn advance_budget_tier(&mut self, now_ms: u64) {
        match self.budget_tier {
            BudgetTier::Normal | BudgetTier::Revocation => {
                // Nothing to advance.
            }
            BudgetTier::Warning => {
                if let Some(warn_start) = self.warning_started_ms {
                    if now_ms.saturating_sub(warn_start) >= THROTTLE_AFTER_MS {
                        self.budget_tier = BudgetTier::Throttle;
                        self.throttle_started_ms = Some(now_ms);
                    }
                }
            }
            BudgetTier::Throttle => {
                if let Some(throttle_start) = self.throttle_started_ms {
                    if now_ms.saturating_sub(throttle_start) >= REVOCATION_AFTER_MS {
                        self.budget_tier = BudgetTier::Revocation;
                    }
                }
            }
        }
    }

    /// Push an audit event with the current lease_id and clock timestamp.
    fn push_event(&mut self, kind: LeaseEventKind) {
        self.pending_events.push(LeaseAuditEvent {
            lease_id: self.lease_id,
            event_at_wall_us: self.now_us(),
            kind,
        });
    }

    /// Compute optional `expires_at_wall_us` given a TTL and activation time.
    ///
    /// Returns `None` for indefinite leases (`ttl_ms == 0`).
    fn compute_expires_at_wall_us(&self, activated_at_ms: u64) -> Option<u64> {
        if self.ttl_ms == 0 {
            None
        } else {
            // expires_at = activated_at + ttl, in microseconds
            Some((activated_at_ms + self.ttl_ms) * 1_000)
        }
    }
}

impl<C: Clock> LeaseStateMachine<C> for LeaseImpl<C> {
    fn new_requested(ttl_ms: u64, policy: RenewalPolicy, clock: C) -> Self {
        LeaseImpl {
            clock,
            state: LeaseState::Requested,
            renewal_policy: policy,
            lease_id: SceneId::null(),
            activated_at_ms: 0,
            ttl_ms,
            total_suspension_ms: 0,
            suspended_at_ms: None,
            ttl_remaining_at_suspend_ms: None,
            orphaned_at_ms: None,
            grace_period_ms: DEFAULT_GRACE_PERIOD_MS,
            budget_tier: BudgetTier::Normal,
            warning_started_ms: None,
            throttle_started_ms: None,
            pending_events: Vec::new(),
        }
    }

    // ── Transitions ──────────────────────────────────────────────────────────

    fn activate(&mut self) -> Result<(), TransitionError> {
        if self.state != LeaseState::Requested {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        let now_ms = self.now_ms();
        let now_us = self.now_us();
        self.activated_at_ms = now_ms;
        self.state = LeaseState::Active;

        // Build a minimal LeaseIdentity from locally available fields.
        // Callers that need fully-populated identity fields (namespace, session_id,
        // capability_scope, resource_budget) should set those on a higher-level
        // wrapper and not rely on this auto-built identity for wire protocol.
        let expires_at = self.compute_expires_at_wall_us(now_ms);
        // Convert lease-module RenewalPolicy to types::RenewalPolicy for LeaseIdentity.
        let identity_renewal_policy = match self.renewal_policy {
            RenewalPolicy::Manual => crate::types::RenewalPolicy::Manual,
            RenewalPolicy::AutoRenew => crate::types::RenewalPolicy::AutoRenew,
            RenewalPolicy::OneShot => crate::types::RenewalPolicy::OneShot,
        };
        let identity = LeaseIdentity {
            lease_id: self.lease_id,
            namespace: String::new(),
            session_id: SceneId::null(),
            granted_at_wall_us: now_us,
            ttl_ms: self.ttl_ms,
            renewal_policy: identity_renewal_policy,
            capability_scope: Vec::new(),
            resource_budget: crate::types::ResourceBudget::default(),
            lease_priority: 2, // normal
        };
        self.push_event(LeaseEventKind::Granted {
            identity,
            expires_at_wall_us: expires_at,
        });
        Ok(())
    }

    fn suspend(&mut self) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        if self.state != LeaseState::Active {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Suspended,
            });
        }
        let now_ms = self.now_ms();
        let now_us = self.now_us();
        let remaining = self.effective_remaining_ms_at(now_ms);
        self.suspended_at_ms = Some(now_ms);
        self.ttl_remaining_at_suspend_ms = Some(remaining);
        self.state = LeaseState::Suspended;

        self.push_event(LeaseEventKind::Suspended {
            suspended_at_wall_us: now_us,
            ttl_remaining_ms: remaining,
        });
        Ok(())
    }

    fn resume(&mut self) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        if self.state != LeaseState::Suspended {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        let now_ms = self.now_ms();

        // Compute suspension duration before mutating state.
        let suspension_duration_us = self
            .suspended_at_ms
            .map(|at| now_ms.saturating_sub(at) * 1_000)
            .unwrap_or(0);

        // Accumulate suspension duration
        if let Some(susp_at) = self.suspended_at_ms {
            self.total_suspension_ms += now_ms.saturating_sub(susp_at);
        }
        // Restore TTL remaining: adjust activated_at_ms so that
        // effective_remaining_ms_at(now_ms) == ttl_remaining_at_suspend_ms.
        //
        // effective_elapsed = (now - activated_at) - total_suspension
        // remaining = ttl_ms - effective_elapsed
        // => activated_at = now - (ttl_ms - remaining) - total_suspension
        //                  = now - ttl_ms + remaining - total_suspension
        // But simpler: just reset activated_at so:
        //   (now - activated_at) - total_suspension = ttl_ms - remaining_at_suspend
        if let Some(remaining_at_suspend) = self.ttl_remaining_at_suspend_ms {
            if self.ttl_ms > 0 {
                let effective_elapsed_desired = self.ttl_ms.saturating_sub(remaining_at_suspend);
                // (now - activated_at) - total_suspension = effective_elapsed_desired
                // activated_at = now - effective_elapsed_desired - total_suspension
                self.activated_at_ms = now_ms
                    .saturating_sub(effective_elapsed_desired)
                    .saturating_sub(self.total_suspension_ms);
            }
        }
        self.suspended_at_ms = None;
        self.ttl_remaining_at_suspend_ms = None;
        self.state = LeaseState::Active;

        // Compute new expires_at after TTL adjustment.
        let adjusted_expires_at_wall_us = self.compute_expires_at_wall_us(self.activated_at_ms);
        self.push_event(LeaseEventKind::Resumed {
            adjusted_expires_at_wall_us,
            suspension_duration_us,
        });
        Ok(())
    }

    fn orphan(&mut self) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        if self.state != LeaseState::Active {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Orphaned,
            });
        }
        let now_ms = self.now_ms();
        self.orphaned_at_ms = Some(now_ms);
        self.state = LeaseState::Orphaned;

        let grace_expires_at_wall_us = (now_ms + self.grace_period_ms) * 1_000;
        self.push_event(LeaseEventKind::Orphaned {
            grace_expires_at_wall_us,
        });
        Ok(())
    }

    fn reconnect(&mut self) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        if self.state != LeaseState::Orphaned {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        // Verify still within grace period.
        let now_ms = self.now_ms();
        if let Some(orphaned_at) = self.orphaned_at_ms {
            if now_ms.saturating_sub(orphaned_at) >= self.grace_period_ms {
                return Err(TransitionError::InvalidTransition {
                    from: self.state,
                    to: LeaseState::Active,
                });
            }
        }
        self.orphaned_at_ms = None;
        self.state = LeaseState::Active;

        self.push_event(LeaseEventKind::Reconnected);
        Ok(())
    }

    fn expire(&mut self) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        match self.state {
            LeaseState::Active | LeaseState::Orphaned => {
                self.state = LeaseState::Expired;
                self.push_event(LeaseEventKind::Expired);
                Ok(())
            }
            _ => Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Expired,
            }),
        }
    }

    fn revoke(&mut self, reason: RevokeReason) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        // REVOKED can be reached from Active, Suspended, or Orphaned.
        match self.state {
            LeaseState::Active | LeaseState::Suspended | LeaseState::Orphaned => {
                self.state = LeaseState::Revoked;
                // Map internal RevokeReason to the audit types RevokeReason.
                let audit_reason = match reason {
                    RevokeReason::ViewerDismissed => AuditRevokeReason::ViewerDismissed,
                    RevokeReason::BudgetPolicy => AuditRevokeReason::BudgetPolicy,
                    RevokeReason::SuspensionTimeout => AuditRevokeReason::SuspensionTimeout,
                    RevokeReason::CapabilityRevoked => AuditRevokeReason::CapabilityRevoked,
                    RevokeReason::Other => AuditRevokeReason::Other,
                };
                self.push_event(LeaseEventKind::Revoked {
                    reason: audit_reason,
                });
                Ok(())
            }
            _ => Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Revoked,
            }),
        }
    }

    fn release(&mut self) -> Result<(), TransitionError> {
        if self.state.is_terminal() {
            return Err(TransitionError::TerminalState);
        }
        if self.state != LeaseState::Active {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Released,
            });
        }
        self.state = LeaseState::Released;
        self.push_event(LeaseEventKind::Released);
        Ok(())
    }

    fn deny(&mut self, reason: DenyReason) -> Result<(), TransitionError> {
        if self.state != LeaseState::Requested {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: LeaseState::Denied,
            });
        }
        self.state = LeaseState::Denied;
        self.push_event(LeaseEventKind::Denied { reason });
        Ok(())
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    fn state(&self) -> LeaseState {
        self.state
    }

    fn ttl_remaining_ms(&self) -> Option<u64> {
        if self.ttl_ms == 0 {
            return None; // indefinite
        }
        if self.state.is_terminal() {
            return None;
        }
        let now_ms = self.now_ms();
        Some(self.effective_remaining_ms_at(now_ms))
    }

    fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    fn can_transition_to(&self, target: LeaseState) -> bool {
        use LeaseState::*;
        if self.state.is_terminal() {
            return false;
        }
        match (self.state, target) {
            (Requested, Active) => true,
            (Requested, Denied) => true,
            (Active, Suspended) => true,
            (Active, Orphaned) => true,
            (Active, Expired) => true,
            (Active, Revoked) => true,
            (Active, Released) => true,
            (Suspended, Active) => true,
            (Suspended, Revoked) => true,
            (Orphaned, Active) => true,
            (Orphaned, Expired) => true,
            _ => false,
        }
    }

    fn budget_tier(&self) -> BudgetTier {
        self.budget_tier
    }

    fn suspension_duration_ms(&self) -> u64 {
        match self.state {
            LeaseState::Suspended => {
                let now_ms = self.now_ms();
                self.suspended_at_ms
                    .map(|at| now_ms.saturating_sub(at))
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn update_budget_usage(&mut self, usage_fraction: f64) -> Result<(), TransitionError> {
        let now_ms = self.now_ms();

        if usage_fraction >= 1.0 {
            // Hard limit: set Revocation tier immediately per spec §Budget Hard Limit at 100%.
            self.budget_tier = BudgetTier::Revocation;
            self.warning_started_ms = None;
            self.throttle_started_ms = None;
            return Err(TransitionError::BudgetHardLimitExceeded);
        }

        if usage_fraction >= 0.80 {
            // Enter Warning if not already in warning or higher.
            if self.budget_tier == BudgetTier::Normal {
                self.budget_tier = BudgetTier::Warning;
                self.warning_started_ms = Some(now_ms);
            } else {
                // Already in Warning or Throttle — check for tier advancement.
                self.advance_budget_tier(now_ms);
            }
        } else {
            // Below 80%: reset to Normal if not yet in Throttle/Revocation.
            if self.budget_tier == BudgetTier::Warning {
                self.budget_tier = BudgetTier::Normal;
                self.warning_started_ms = None;
            }
            // Throttle and Revocation are sticky (require explicit resolution by the runtime).
        }

        Ok(())
    }

    // ── Audit event channel ───────────────────────────────────────────────────

    fn drain_events(&mut self) -> Vec<LeaseAuditEvent> {
        std::mem::take(&mut self.pending_events)
    }
}
