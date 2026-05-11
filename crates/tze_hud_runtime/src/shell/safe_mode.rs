//! Safe mode state machine — suspend and resume per system-shell/spec.md.
//!
//! # Ownership contract
//!
//! The `SafeModeController` is the **SOLE** owner of safe mode and freeze state transitions.
//! - It is the only writer of `ShellOverrideState.safe_mode_active` and `freeze_active`.
//! - Policy arbitration evaluates triggers (Level 0/Level 1) but does NOT transition shell state.
//! - On any safe mode activation: **freeze MUST be cancelled before any other entry steps**.
//!
//! # State invariant
//!
//! `safe_mode = true` implies `freeze_active = false`.  This invariant is enforced atomically
//! inside `enter_safe_mode()` and is never violated regardless of entry trigger.
//!
//! # Entry protocol (spec lines 89–101)
//!
//! 1. Cancel active freeze and discard freeze queue (`freeze_active = false` first).
//! 2. Suspend all ACTIVE leases (NOT revoke — identity preserved).
//! 3. Set `SharedState.safe_mode_active = true` → mutation intake rejects batches.
//! 4. Broadcast `SessionSuspended` to all connected sessions via `ServerMessage` channel.
//! 5. Set `ChromeState.safe_mode_active = true` → overlay renders on next frame.
//!
//! # Exit protocol (spec lines 115–123)
//!
//! 1. Dismiss overlay: `ChromeState.safe_mode_active = false`.
//! 2. Resume all SUSPENDED leases → ACTIVE; compute TTL adjustments.
//! 3. Set `SharedState.safe_mode_active = false` → mutations accepted again.
//! 4. Broadcast `SessionResumed` to all connected sessions.
//! 5. After safe mode exit, freeze remains inactive.
//!
//! # Spec references
//!
//! - system-shell/spec.md §Safe Mode Entry Protocol (line 89)
//! - system-shell/spec.md §Safe Mode Overlay (line 102)
//! - system-shell/spec.md §Safe Mode Exit (line 115)
//! - system-shell/spec.md §Safe Mode and Freeze Interaction (line 124)
//! - lease-governance/spec.md §Safe Mode Suspends Leases (line 92)
//! - lease-governance/spec.md §Safe Mode Resume (line 105)

use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use tze_hud_protocol::proto::session::{
    ServerMessage, SessionResumed, SessionSuspended, server_message::Payload as ServerPayload,
};
use tze_hud_protocol::session::SharedState;
use tze_hud_scene::types::LeaseState;
use tze_hud_scene::types::SceneId;

use super::chrome::{
    AuditPayload, AuditTrigger, ChromeState, NoopAuditSink, SafeModeEntryReason, ShellAuditEvent,
    ShellAuditSink,
};

// ─── Shell-owned override state ───────────────────────────────────────────────

/// Shell-owned override state.
///
/// Written exclusively by `SafeModeController`.  Policy arbitration only reads a snapshot.
/// This is the authoritative source for `safe_mode_active` and `freeze_active`.
#[derive(Clone, Debug, Default)]
pub struct ShellOverrideState {
    /// Scene is currently frozen (Ctrl+Shift+F active).
    pub freeze_active: bool,
    /// Safe mode is currently active.
    pub safe_mode_active: bool,
    /// Wall-clock milliseconds when safe mode was entered (0 if inactive).
    pub safe_mode_entered_at_ms: u64,
    /// The reason safe mode was entered.
    pub safe_mode_entry_reason: Option<SafeModeEntryReason>,
}

impl ShellOverrideState {
    /// Assert the state invariant: `safe_mode = true ⟹ freeze_active = false`.
    ///
    /// Panics in debug builds if the invariant is violated.
    #[inline]
    pub fn assert_invariant(&self) {
        debug_assert!(
            !(self.safe_mode_active && self.freeze_active),
            "shell invariant violated: safe_mode and freeze_active are both true"
        );
    }
}

// ─── Lease resume info ────────────────────────────────────────────────────────

/// Per-lease TTL adjustment information emitted on safe mode exit.
///
/// Corresponds to `LeaseResume` fields defined in RFC 0008 §7.3.
#[derive(Clone, Debug)]
pub struct LeaseResumeInfo {
    /// The lease ID being resumed.
    pub lease_id: SceneId,
    /// Agent namespace that owns the lease.
    pub namespace: String,
    /// Adjusted wall-clock expiry in UTC microseconds.
    /// `None` for indefinite-TTL leases (`ttl_ms = 0`).
    pub adjusted_expires_at_wall_us: Option<u64>,
    /// How long the lease was suspended, in microseconds.
    pub suspension_duration_us: u64,
}

// ─── Entry / exit result types ────────────────────────────────────────────────

/// Result of a safe mode entry operation.
#[derive(Debug)]
pub struct SafeModeEntryResult {
    /// Number of leases that were suspended.
    pub leases_suspended: usize,
    /// Number of sessions that received `SessionSuspended`.
    pub sessions_notified: usize,
    /// Whether freeze was active and had to be cancelled on entry.
    pub freeze_was_cancelled: bool,
}

/// Result of a safe mode exit operation.
#[derive(Debug)]
pub struct SafeModeExitResult {
    /// Number of leases that were resumed.
    pub leases_resumed: usize,
    /// TTL adjustments for each resumed lease (for `LeaseResume` messages).
    pub lease_resumes: Vec<LeaseResumeInfo>,
    /// Number of sessions that received `SessionResumed`.
    pub sessions_notified: usize,
    /// Suspension duration in microseconds (from entry to exit).
    pub suspension_duration_us: u64,
}

// ─── Input handling ───────────────────────────────────────────────────────────

/// An input event relevant to safe mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeModeInput {
    /// Click/tap "Resume" button, or Enter/Space while it is focused.
    ResumeAction,
    /// `Ctrl+Shift+Escape` — toggles safe mode both in and out.
    CtrlShiftEscape,
    /// Any other input (captured during safe mode; discarded / not routed to agents).
    Other,
}

/// Result of processing an input event in safe mode context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeModeInputResult {
    /// Input triggered a safe mode exit; caller should call `exit_safe_mode()`.
    ExitSafeMode,
    /// Input was captured by safe mode and MUST NOT be routed to agents.
    Captured,
    /// Safe mode is not active; input should be dispatched normally.
    PassThrough,
}

/// Classify a raw input event for safe mode processing.
///
/// During safe mode all input is captured by the chrome layer.  Only the exit
/// gestures (`ResumeAction` and `CtrlShiftEscape`) produce `ExitSafeMode`;
/// everything else is `Captured`.
pub fn classify_safe_mode_input(
    input: SafeModeInput,
    safe_mode_active: bool,
) -> SafeModeInputResult {
    if !safe_mode_active {
        return SafeModeInputResult::PassThrough;
    }
    match input {
        SafeModeInput::ResumeAction | SafeModeInput::CtrlShiftEscape => {
            SafeModeInputResult::ExitSafeMode
        }
        SafeModeInput::Other => SafeModeInputResult::Captured,
    }
}

// ─── SafeModeController ───────────────────────────────────────────────────────

/// Controls the safe mode state machine.
///
/// This is the SOLE writer of `ShellOverrideState.safe_mode_active` and
/// `ShellOverrideState.freeze_active`.
///
/// `override_state` is kept private to enforce the ownership contract:
/// no external code can mutate `safe_mode_active`/`freeze_active` and
/// violate the state invariant. Use the read-only accessor `override_state()`
/// or the public convenience methods (`is_safe_mode_active`, `is_freeze_active`).
/// Tests that need to set up initial state (e.g., `freeze_active = true`) must
/// use `set_freeze_active_for_test` (cfg(test) only).
pub struct SafeModeController {
    /// Shared protocol state (scene graph + sessions).
    pub shared_state: Arc<Mutex<SharedState>>,
    /// Chrome rendering state (for overlay visibility).
    pub chrome_state: Arc<RwLock<ChromeState>>,
    /// Shell-owned override state — authoritative source.
    /// Private: only `SafeModeController` methods may write this.
    override_state: ShellOverrideState,
    /// Audit sink for shell events (never routed to agents).
    audit_sink: Arc<dyn ShellAuditSink>,
}

impl SafeModeController {
    /// Create a new controller.
    pub fn new(
        shared_state: Arc<Mutex<SharedState>>,
        chrome_state: Arc<RwLock<ChromeState>>,
        audit_sink: Arc<dyn ShellAuditSink>,
    ) -> Self {
        Self {
            shared_state,
            chrome_state,
            override_state: ShellOverrideState::default(),
            audit_sink,
        }
    }

    /// Create with a no-op audit sink (for headless / test use).
    pub fn new_headless(
        shared_state: Arc<Mutex<SharedState>>,
        chrome_state: Arc<RwLock<ChromeState>>,
    ) -> Self {
        Self::new(shared_state, chrome_state, Arc::new(NoopAuditSink))
    }

    /// Whether safe mode is currently active.
    pub fn is_safe_mode_active(&self) -> bool {
        self.override_state.safe_mode_active
    }

    /// Whether freeze is currently active.
    pub fn is_freeze_active(&self) -> bool {
        self.override_state.freeze_active
    }

    /// Read-only snapshot of the current override state.
    /// Used by policy arbitration — never written outside the shell.
    pub fn override_state(&self) -> &ShellOverrideState {
        &self.override_state
    }

    // ── Entry ─────────────────────────────────────────────────────────────────

    /// Enter safe mode.
    ///
    /// # Protocol order (must match spec)
    ///
    /// 1. Cancel freeze (`freeze_active = false` BEFORE any other steps).
    /// 2. Suspend all ACTIVE leases — NOT revoke.
    /// 3. Set `SharedState.safe_mode_active = true` — mutation intake rejects batches.
    /// 4. Broadcast `SessionSuspended` to all connected sessions.
    /// 5. Set `ChromeState.safe_mode_active = true` — overlay on next frame.
    ///
    /// Returns a summary of what happened. If safe mode is already active, returns
    /// a no-op result without double-entering.
    pub async fn enter_safe_mode(
        &mut self,
        reason: SafeModeEntryReason,
        trigger: AuditTrigger,
        _error_detail: Option<String>,
    ) -> SafeModeEntryResult {
        // Guard: idempotent — already active is a no-op.
        if self.override_state.safe_mode_active {
            return SafeModeEntryResult {
                leases_suspended: 0,
                sessions_notified: 0,
                freeze_was_cancelled: false,
            };
        }

        let now_ms = now_wall_ms();
        let now_us = now_ms.saturating_mul(1_000);

        // Step 1: Cancel freeze BEFORE any other safe mode entry steps.
        // The state invariant safe_mode=true ⟹ freeze_active=false must hold.
        let freeze_was_cancelled = if self.override_state.freeze_active {
            self.override_state.freeze_active = false;
            // Note: freeze queue discard is managed by the freeze module (bead #3).
            // Setting freeze_active=false here signals that the queue should be discarded.
            true
        } else {
            false
        };

        // Steps 2–4: acquire SharedState, suspend leases, signal safe mode, broadcast.
        let (leases_suspended, sessions_notified) = {
            let mut st = self.shared_state.lock().await;

            // Step 2: Suspend all ACTIVE leases (NOT revoke — spec §Safe Mode Suspends Leases).
            let leases_suspended = {
                let mut scene = st.scene.lock().await;
                scene.suspend_all_leases(now_ms);
                scene
                    .leases
                    .values()
                    .filter(|l| l.state == LeaseState::Suspended)
                    .count()
            };

            // Step 3: Signal safe mode active so mutation intake rejects new batches.
            st.safe_mode_active = true;

            // Step 4: Broadcast SessionSuspended to all connected sessions.
            // Sessions that are subscribed receive this via their server_message_tx.
            //
            // sequence = 0: the protocol assigns per-session monotonically increasing
            // sequence numbers to server messages.  The session handler is responsible
            // for stamping the correct sequence before sending to a client.  Broadcasting
            // a shared `ServerMessage` means we cannot assign per-session sequences here;
            // callers that care about sequencing (e.g., integration tests) must rewrite
            // the field when delivering to individual sessions.  This is a known limitation
            // tracked as a follow-up: the session registry should wrap each message with
            // a per-session sequence before delivery instead of broadcasting a shared struct.
            let suspended_msg = ServerMessage {
                sequence: 0, // see comment above — to be fixed when per-session sequencing lands
                timestamp_wall_us: now_us,
                payload: Some(ServerPayload::SessionSuspended(SessionSuspended {
                    reason: "safe_mode_entered".to_string(),
                    timestamp_wall_us: now_us,
                })),
            };
            let sessions_notified = st.sessions.broadcast_server_message(suspended_msg);

            (leases_suspended, sessions_notified)
        };

        // Step 5: Set ChromeState → overlay visible on next compositor frame.
        // Recover from a poisoned lock rather than panicking: safe mode is a
        // failure-recovery path and must remain resilient after a prior panic.
        {
            let mut chrome = self
                .chrome_state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            chrome.safe_mode_active = true;
        }

        // Update local override state atomically.
        self.override_state.safe_mode_active = true;
        self.override_state.safe_mode_entered_at_ms = now_ms;
        self.override_state.safe_mode_entry_reason = Some(reason);
        self.override_state.assert_invariant();

        // Emit audit event (telemetry thread only — never routed to agents).
        // Note: `timestamp_mono_us` is populated with a wall-clock value here.
        // `ShellAuditEvent` calls this field "monotonic", but the safe mode controller
        // does not yet hold an `Instant`-based epoch.  A future refactor should inject
        // a monotonic clock source to prevent backward-going timestamps on NTP adjustments.
        self.audit_sink.emit(ShellAuditEvent {
            timestamp_mono_us: now_us,
            trigger,
            payload: AuditPayload::SafeModeEntered { reason },
        });

        // If freeze was cancelled, emit a FreezeDeactivated event.
        if freeze_was_cancelled {
            self.audit_sink.emit(ShellAuditEvent {
                timestamp_mono_us: now_us,
                trigger: AuditTrigger::Auto,
                payload: AuditPayload::FreezeDeactivated,
            });
        }

        SafeModeEntryResult {
            leases_suspended,
            sessions_notified,
            freeze_was_cancelled,
        }
    }

    // ── Exit ──────────────────────────────────────────────────────────────────

    /// Exit safe mode.
    ///
    /// # Protocol order (must match spec)
    ///
    /// 1. Dismiss overlay: `ChromeState.safe_mode_active = false`.
    /// 2. Resume all SUSPENDED leases → ACTIVE; compute TTL adjustments.
    /// 3. Set `SharedState.safe_mode_active = false` — mutations accepted again.
    /// 4. Broadcast `SessionResumed` to all connected sessions.
    ///
    /// Returns TTL adjustment information per lease for `LeaseResume` delivery.
    ///
    /// **Agents MUST NOT re-request leases** — identity, capability scope, and resource
    /// budget are preserved across the ACTIVE → SUSPENDED → ACTIVE cycle.
    pub async fn exit_safe_mode(&mut self) -> SafeModeExitResult {
        // Guard: idempotent — not active is a no-op.
        if !self.override_state.safe_mode_active {
            return SafeModeExitResult {
                leases_resumed: 0,
                lease_resumes: Vec::new(),
                sessions_notified: 0,
                suspension_duration_us: 0,
            };
        }

        let now_ms = now_wall_ms();
        let now_us = now_ms.saturating_mul(1_000);
        let entered_at_us = self
            .override_state
            .safe_mode_entered_at_ms
            .saturating_mul(1_000);
        let suspension_duration_us = now_us.saturating_sub(entered_at_us);

        // Step 1: Dismiss overlay immediately so the next compositor frame has no overlay.
        {
            let mut chrome = self
                .chrome_state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            chrome.safe_mode_active = false;
        }

        // Steps 2–4: acquire SharedState, resume leases, clear safe mode flag, broadcast.
        let (leases_resumed, lease_resumes, sessions_notified) = {
            let mut st = self.shared_state.lock().await;

            // Step 2a: Collect suspension info and resume leases within the scene lock.
            let safe_mode_entered_ms = self.override_state.safe_mode_entered_at_ms;
            let (suspended_info, leases_resumed) = {
                let mut scene = st.scene.lock().await;
                let suspended_info: Vec<(SceneId, String, u64, u64)> = scene
                    .leases
                    .values()
                    .filter(|l| l.state == LeaseState::Suspended)
                    .map(|l| {
                        // suspension start time (use recorded suspended_at_ms or safe mode entry).
                        let susp_at_ms = l.suspended_at_ms.unwrap_or(safe_mode_entered_ms);
                        let susp_dur_us = now_ms.saturating_sub(susp_at_ms).saturating_mul(1_000);
                        // Adjusted expiry = now_us + remaining TTL (in us).
                        // For indefinite TTL (ttl_ms == 0), we use 0 as "no expiry" sentinel.
                        let adjusted_expires_wall_us = if l.ttl_ms == 0 {
                            0 // sentinel for indefinite
                        } else {
                            let remaining_ms = l.ttl_remaining_at_suspend_ms.unwrap_or(l.ttl_ms);
                            now_us.saturating_add(remaining_ms.saturating_mul(1_000))
                        };
                        (
                            l.id,
                            l.namespace.clone(),
                            adjusted_expires_wall_us,
                            susp_dur_us,
                        )
                    })
                    .collect();
                // Step 2b: Resume all SUSPENDED leases → ACTIVE; TTL adjusted.
                scene.resume_all_leases(now_ms);
                let leases_resumed = suspended_info.len();
                (suspended_info, leases_resumed)
            };

            // Build LeaseResume descriptors for the caller.
            let lease_resumes: Vec<LeaseResumeInfo> = suspended_info
                .into_iter()
                .map(
                    |(lease_id, namespace, adjusted_expires_wall_us, susp_dur_us)| {
                        LeaseResumeInfo {
                            lease_id,
                            namespace,
                            adjusted_expires_at_wall_us: if adjusted_expires_wall_us == 0 {
                                None // indefinite TTL
                            } else {
                                Some(adjusted_expires_wall_us)
                            },
                            suspension_duration_us: susp_dur_us,
                        }
                    },
                )
                .collect();

            // Step 3: Clear safe mode flag — mutation intake accepts new batches.
            st.safe_mode_active = false;

            // Step 4: Broadcast SessionResumed to all connected sessions.
            // sequence = 0: same known limitation as SessionSuspended above — per-session
            // sequencing must be assigned by the session handler, not the broadcaster.
            let resumed_msg = ServerMessage {
                sequence: 0,
                timestamp_wall_us: now_us,
                payload: Some(ServerPayload::SessionResumed(SessionResumed {
                    timestamp_wall_us: now_us,
                })),
            };
            let sessions_notified = st.sessions.broadcast_server_message(resumed_msg);

            (leases_resumed, lease_resumes, sessions_notified)
        };

        // Update local override state.
        self.override_state.safe_mode_active = false;
        self.override_state.safe_mode_entered_at_ms = 0;
        self.override_state.safe_mode_entry_reason = None;
        // Spec §Safe Mode Exit: after exit, freeze is inactive.
        // freeze_active was cleared on entry; do not re-enable.
        self.override_state.assert_invariant();

        // Emit audit event.
        self.audit_sink.emit(ShellAuditEvent {
            timestamp_mono_us: now_us,
            trigger: AuditTrigger::PointerGesture, // exit is always an explicit viewer action
            payload: AuditPayload::SafeModeExited,
        });

        SafeModeExitResult {
            leases_resumed,
            lease_resumes,
            sessions_notified,
            suspension_duration_us,
        }
    }

    // ── Input routing ─────────────────────────────────────────────────────────

    /// Route an input event through the safe mode filter.
    ///
    /// During safe mode ALL input is captured by the chrome layer; only the three
    /// "exit" gestures yield `ExitSafeMode`.
    pub fn route_input(&self, input: SafeModeInput) -> SafeModeInputResult {
        classify_safe_mode_input(input, self.override_state.safe_mode_active)
    }

    // ── Convenience constructors for common triggers ──────────────────────────

    /// Automatic safe mode entry on `wgpu::DeviceError::Lost`.
    ///
    /// Per spec §Auto safe mode on GPU loss: `SafeModeEntryReason = CriticalError`.
    pub async fn enter_safe_mode_on_gpu_loss(
        &mut self,
        error_detail: Option<String>,
    ) -> SafeModeEntryResult {
        self.enter_safe_mode(
            SafeModeEntryReason::CriticalError,
            AuditTrigger::Auto,
            error_detail,
        )
        .await
    }

    /// Automatic safe mode entry on scene graph corruption or unrecoverable render failure.
    pub async fn enter_safe_mode_on_critical_error(
        &mut self,
        error_detail: Option<String>,
    ) -> SafeModeEntryResult {
        self.enter_safe_mode(
            SafeModeEntryReason::CriticalError,
            AuditTrigger::Auto,
            error_detail,
        )
        .await
    }

    /// Manual safe mode entry via `Ctrl+Shift+Escape` or "Dismiss All" chrome control.
    pub async fn enter_safe_mode_viewer_action(&mut self) -> SafeModeEntryResult {
        self.enter_safe_mode(
            SafeModeEntryReason::ExplicitViewerAction,
            AuditTrigger::KeyboardShortcut,
            None,
        )
        .await
    }

    /// Directly set freeze state for testing only.
    ///
    /// This exists solely to set up pre-condition state in unit tests (e.g.,
    /// `freeze_active = true` before entering safe mode).  Production code
    /// MUST use the freeze module (bead #3) to manage freeze state.
    #[cfg(test)]
    pub fn set_freeze_active_for_test(&mut self, active: bool) {
        self.override_state.freeze_active = active;
    }

    /// Expose the override state for read-only use in tests.
    #[cfg(test)]
    pub fn override_state_for_test(&self) -> &ShellOverrideState {
        &self.override_state
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Current wall-clock time in milliseconds since Unix epoch.
fn now_wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::chrome::{
        AuditPayload, ChromeState, CollectingAuditSink, SafeModeEntryReason,
    };
    use super::*;
    use std::sync::{Arc, RwLock};
    use tokio::sync::Mutex;
    use tze_hud_protocol::session::{SessionRegistry, SharedState};
    use tze_hud_protocol::token::TokenStore;
    use tze_hud_scene::graph::SceneGraph;
    use tze_hud_scene::types::LeaseState;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_shared_state() -> Arc<Mutex<SharedState>> {
        use std::sync::Arc;
        use tze_hud_protocol::session::RuntimeDegradationLevel;
        Arc::new(Mutex::new(SharedState {
            scene: Arc::new(Mutex::new(SceneGraph::new(1920.0, 1080.0))),
            sessions: SessionRegistry::new("test-key"),
            resource_store: tze_hud_resource::ResourceStore::new(
                tze_hud_resource::ResourceStoreConfig::default(),
            ),
            widget_asset_store: tze_hud_protocol::session::WidgetAssetStore::default(),
            runtime_widget_store: None,
            element_store: tze_hud_scene::element_store::ElementStore::default(),
            element_store_path: None,
            safe_mode_active: false,
            freeze_active: false,
            token_store: TokenStore::new(),
            degradation_level: RuntimeDegradationLevel::Normal,
            media_ingress_active: None,
            input_capture_tx: None,
        }))
    }

    fn make_controller_with_sink(sink: Arc<CollectingAuditSink>) -> SafeModeController {
        let shared = make_shared_state();
        let chrome = Arc::new(RwLock::new(ChromeState::new()));
        SafeModeController::new(shared, chrome, sink)
    }

    fn make_controller() -> SafeModeController {
        let shared = make_shared_state();
        let chrome = Arc::new(RwLock::new(ChromeState::new()));
        SafeModeController::new_headless(shared, chrome)
    }

    /// Grant an active lease in the scene graph; returns the lease ID.
    async fn grant_active_lease(ctrl: &SafeModeController, namespace: &str) -> SceneId {
        let st = ctrl.shared_state.lock().await;
        st.scene.lock().await.grant_lease(namespace, 60_000, vec![])
    }

    // ── 1. Entry protocol ─────────────────────────────────────────────────────

    /// Scenario: Manual safe mode entry (spec line 94)
    /// WHEN viewer presses Ctrl+Shift+Escape
    /// THEN all active leases are suspended, agents receive SessionSuspended,
    ///      safe mode overlay appears, and all input routes to chrome.
    #[tokio::test]
    async fn test_enter_safe_mode_suspends_active_leases() {
        let mut ctrl = make_controller();
        let lease_id = grant_active_lease(&ctrl, "agent.alpha").await;

        // Verify lease is ACTIVE before entry.
        {
            let st = ctrl.shared_state.lock().await;
            assert_eq!(
                st.scene.lock().await.leases[&lease_id].state,
                LeaseState::Active
            );
        }

        let result = ctrl.enter_safe_mode_viewer_action().await;

        assert!(
            ctrl.is_safe_mode_active(),
            "safe mode must be active after entry"
        );
        assert_eq!(result.leases_suspended, 1, "one lease should be suspended");
        assert!(
            !ctrl.is_freeze_active(),
            "freeze must be inactive after safe mode entry"
        );

        // Lease must be SUSPENDED — NOT revoked.
        {
            let st = ctrl.shared_state.lock().await;
            assert_eq!(
                st.scene.lock().await.leases[&lease_id].state,
                LeaseState::Suspended,
                "lease must be SUSPENDED not REVOKED — identity preserved"
            );
            assert!(
                st.safe_mode_active,
                "SharedState.safe_mode_active must be true"
            );
        }

        // ChromeState reflects safe mode → overlay will render on next frame.
        {
            let chrome = ctrl.chrome_state.read().unwrap();
            assert!(
                chrome.safe_mode_active,
                "ChromeState.safe_mode_active must be true"
            );
        }
    }

    /// Scenario: Auto safe mode on GPU loss (spec line 98)
    /// WHEN compositor detects wgpu::DeviceError::Lost
    /// THEN runtime enters safe mode with SafeModeEntryReason = CRITICAL_ERROR.
    #[tokio::test]
    async fn test_auto_safe_mode_on_gpu_loss() {
        let mut ctrl = make_controller();

        ctrl.enter_safe_mode_on_gpu_loss(Some("wgpu::DeviceError::Lost".into()))
            .await;

        assert!(ctrl.is_safe_mode_active());
        assert_eq!(
            ctrl.override_state_for_test().safe_mode_entry_reason,
            Some(SafeModeEntryReason::CriticalError)
        );
    }

    /// Scenario: Safe mode overlay rendering (spec line 107)
    /// WHEN safe mode entered THEN ChromeState drives overlay — reads exclusively from it.
    #[tokio::test]
    async fn test_safe_mode_overlay_set_in_chrome_state() {
        let mut ctrl = make_controller();

        assert!(!ctrl.chrome_state.read().unwrap().safe_mode_active);
        ctrl.enter_safe_mode_viewer_action().await;
        assert!(ctrl.chrome_state.read().unwrap().safe_mode_active);
    }

    // ── 2. Freeze-safe mode invariant (spec lines 124–139) ───────────────────

    /// Scenario: Safe mode cancels freeze (manual trigger) (spec line 133)
    /// WHEN scene frozen and viewer triggers safe mode via Ctrl+Shift+Escape
    /// THEN freeze cancelled, queue discarded, freeze_active=false, overlay appears.
    #[tokio::test]
    async fn test_safe_mode_cancels_freeze_manual_trigger() {
        let mut ctrl = make_controller();
        ctrl.set_freeze_active_for_test(true);

        let result = ctrl.enter_safe_mode_viewer_action().await;

        assert!(ctrl.is_safe_mode_active());
        assert!(
            !ctrl.is_freeze_active(),
            "freeze MUST be false after safe mode entry"
        );
        assert!(result.freeze_was_cancelled);
        ctrl.override_state_for_test().assert_invariant();
    }

    /// Scenario: Safe mode cancels freeze (automatic trigger) (spec line 129)
    /// WHEN scene frozen and GPU failure triggers safe mode automatically
    /// THEN freeze cancelled, queue discarded, freeze_active=false, overlay appears.
    #[tokio::test]
    async fn test_safe_mode_cancels_freeze_auto_trigger() {
        let mut ctrl = make_controller();
        ctrl.set_freeze_active_for_test(true);

        let result = ctrl
            .enter_safe_mode_on_gpu_loss(Some("GPU error".into()))
            .await;

        assert!(ctrl.is_safe_mode_active());
        assert!(!ctrl.is_freeze_active());
        assert!(result.freeze_was_cancelled);
        ctrl.override_state_for_test().assert_invariant();
    }

    /// State invariant enforced: safe_mode=true ⟹ freeze_active=false.
    #[tokio::test]
    async fn test_shell_state_invariant_enforced() {
        let mut ctrl = make_controller();
        ctrl.set_freeze_active_for_test(true);
        ctrl.enter_safe_mode_viewer_action().await;

        // Both: safe_mode must be true and freeze must be false.
        assert!(ctrl.is_safe_mode_active());
        assert!(!ctrl.is_freeze_active());
        ctrl.override_state_for_test().assert_invariant();
    }

    /// Scenario: Freeze ignored during safe mode (spec line 137)
    /// WHEN viewer presses Ctrl+Shift+F while safe mode active THEN no effect.
    #[test]
    fn test_freeze_attempt_during_safe_mode_is_captured() {
        // Ctrl+Shift+F (freeze toggle) is NOT one of the exit gestures —
        // it should be captured (not passed to agents, not triggering freeze).
        let result = classify_safe_mode_input(SafeModeInput::Other, true);
        assert_eq!(
            result,
            SafeModeInputResult::Captured,
            "all non-exit inputs (including Ctrl+Shift+F) must be captured during safe mode"
        );
    }

    /// After safe mode exit, freeze is inactive.
    #[tokio::test]
    async fn test_freeze_inactive_after_safe_mode_exit() {
        let mut ctrl = make_controller();
        ctrl.enter_safe_mode_viewer_action().await;
        ctrl.exit_safe_mode().await;

        assert!(!ctrl.is_safe_mode_active());
        assert!(
            !ctrl.is_freeze_active(),
            "freeze must be inactive after safe mode exit"
        );
        ctrl.override_state_for_test().assert_invariant();
    }

    // ── 3. Exit protocol (spec lines 115–123) ─────────────────────────────────

    /// Scenario: Resume from safe mode (spec line 120)
    /// WHEN viewer clicks Resume
    /// THEN overlay dismissed, SUSPENDED leases → ACTIVE, SessionResumed sent,
    ///      LeaseResume with adjusted expiry sent, staleness badges clear,
    ///      mutations accepted without re-request.
    #[tokio::test]
    async fn test_exit_safe_mode_resumes_suspended_leases() {
        let mut ctrl = make_controller();
        let lease_id = grant_active_lease(&ctrl, "agent.alpha").await;

        // Enter: lease becomes SUSPENDED.
        ctrl.enter_safe_mode_viewer_action().await;
        {
            let st = ctrl.shared_state.lock().await;
            assert_eq!(
                st.scene.lock().await.leases[&lease_id].state,
                LeaseState::Suspended
            );
        }

        // Exit: lease should return to ACTIVE.
        let result = ctrl.exit_safe_mode().await;

        assert!(
            !ctrl.is_safe_mode_active(),
            "safe mode must be inactive after exit"
        );
        assert_eq!(result.leases_resumed, 1, "one lease should be resumed");
        assert!(
            !result.lease_resumes.is_empty(),
            "must have lease resume info"
        );

        {
            let st = ctrl.shared_state.lock().await;
            assert_eq!(
                st.scene.lock().await.leases[&lease_id].state,
                LeaseState::Active,
                "lease must return to ACTIVE — agents do not re-request"
            );
            assert!(
                !st.safe_mode_active,
                "SharedState.safe_mode_active must be false"
            );
        }

        // Overlay dismissed.
        assert!(!ctrl.chrome_state.read().unwrap().safe_mode_active);
    }

    /// Agents do NOT re-request leases — identity, capability scope, and budget preserved.
    #[tokio::test]
    async fn test_lease_identity_preserved_across_suspend_resume() {
        let mut ctrl = make_controller();
        let lease_id = grant_active_lease(&ctrl, "agent.alpha").await;

        let (ns_before, priority_before) = {
            let st = ctrl.shared_state.lock().await;
            let scene = st.scene.lock().await;
            let l = &scene.leases[&lease_id];
            (l.namespace.clone(), l.priority)
        };

        ctrl.enter_safe_mode_viewer_action().await;
        ctrl.exit_safe_mode().await;

        {
            let st = ctrl.shared_state.lock().await;
            let scene = st.scene.lock().await;
            let l = &scene.leases[&lease_id];
            assert_eq!(l.namespace, ns_before, "namespace preserved across cycle");
            assert_eq!(
                l.priority, priority_before,
                "priority preserved across cycle"
            );
        }
    }

    /// LeaseResume fields: adjusted_expires_at_wall_us and suspension_duration_us populated.
    #[tokio::test]
    async fn test_lease_resume_info_fields_populated() {
        let mut ctrl = make_controller();
        grant_active_lease(&ctrl, "agent.alpha").await;

        ctrl.enter_safe_mode_viewer_action().await;
        let result = ctrl.exit_safe_mode().await;

        assert!(!result.lease_resumes.is_empty());
        let info = &result.lease_resumes[0];
        // Finite TTL lease (60_000ms) should have a populated adjusted_expires_at_wall_us.
        assert!(
            info.adjusted_expires_at_wall_us.is_some(),
            "adjusted_expires_at_wall_us must be Some for finite-TTL leases"
        );
    }

    /// TTL pause: suspension time excluded from TTL accounting.
    #[tokio::test]
    async fn test_ttl_excluded_during_suspension() {
        let mut ctrl = make_controller();
        let lease_id = grant_active_lease(&ctrl, "agent.alpha").await;

        let original_ttl = {
            let st = ctrl.shared_state.lock().await;
            st.scene.lock().await.leases[&lease_id].ttl_ms
        };

        ctrl.enter_safe_mode_viewer_action().await;
        ctrl.exit_safe_mode().await;

        let post_resume_ttl = {
            let st = ctrl.shared_state.lock().await;
            st.scene.lock().await.leases[&lease_id].ttl_ms
        };

        // The TTL after resume should be close to original (very little real time elapsed
        // in the test). Allow 5000ms tolerance for test overhead.
        assert!(
            post_resume_ttl >= original_ttl.saturating_sub(5_000),
            "TTL after resume ({post_resume_ttl}ms) should be ≈ original ({original_ttl}ms)"
        );
    }

    // ── 4. Mutations rejected during safe mode ────────────────────────────────

    /// WHEN safe mode active THEN SharedState.safe_mode_active = true,
    /// which causes the session server to reject MutationBatch with SAFE_MODE_ACTIVE.
    #[tokio::test]
    async fn test_mutations_rejected_via_shared_state_flag() {
        let mut ctrl = make_controller();

        ctrl.enter_safe_mode_viewer_action().await;
        {
            let st = ctrl.shared_state.lock().await;
            assert!(
                st.safe_mode_active,
                "SharedState.safe_mode_active must be true — session server uses this flag"
            );
        }

        ctrl.exit_safe_mode().await;
        {
            let st = ctrl.shared_state.lock().await;
            assert!(
                !st.safe_mode_active,
                "SharedState.safe_mode_active must be false after exit"
            );
        }
    }

    // ── 5. Input routing ──────────────────────────────────────────────────────

    /// Resume button / Enter / Space triggers safe mode exit.
    #[test]
    fn test_resume_action_exits_safe_mode() {
        assert_eq!(
            classify_safe_mode_input(SafeModeInput::ResumeAction, true),
            SafeModeInputResult::ExitSafeMode
        );
    }

    /// Ctrl+Shift+Escape toggle exits safe mode.
    #[test]
    fn test_ctrl_shift_escape_exits_safe_mode() {
        assert_eq!(
            classify_safe_mode_input(SafeModeInput::CtrlShiftEscape, true),
            SafeModeInputResult::ExitSafeMode
        );
    }

    /// All other inputs are captured during safe mode (not routed to agents).
    #[test]
    fn test_other_inputs_captured_during_safe_mode() {
        assert_eq!(
            classify_safe_mode_input(SafeModeInput::Other, true),
            SafeModeInputResult::Captured
        );
    }

    /// Input passes through when safe mode is inactive.
    #[test]
    fn test_inputs_pass_through_when_inactive() {
        for input in [
            SafeModeInput::ResumeAction,
            SafeModeInput::CtrlShiftEscape,
            SafeModeInput::Other,
        ] {
            assert_eq!(
                classify_safe_mode_input(input, false),
                SafeModeInputResult::PassThrough,
                "input {input:?} must pass through when safe mode is inactive"
            );
        }
    }

    // ── 6. Idempotency ────────────────────────────────────────────────────────

    /// Entering safe mode when already active is a no-op.
    #[tokio::test]
    async fn test_enter_idempotent() {
        let mut ctrl = make_controller();
        let lease_id = grant_active_lease(&ctrl, "agent.alpha").await;

        ctrl.enter_safe_mode_viewer_action().await;
        let result2 = ctrl.enter_safe_mode_viewer_action().await;

        assert_eq!(result2.leases_suspended, 0, "second entry is a no-op");
        assert!(ctrl.is_safe_mode_active());

        // Lease still SUSPENDED, not double-touched.
        let st = ctrl.shared_state.lock().await;
        assert_eq!(
            st.scene.lock().await.leases[&lease_id].state,
            LeaseState::Suspended
        );
    }

    /// Exiting safe mode when not active is a no-op.
    #[tokio::test]
    async fn test_exit_idempotent() {
        let mut ctrl = make_controller();

        let result = ctrl.exit_safe_mode().await;
        assert_eq!(result.leases_resumed, 0, "exit when not active is a no-op");
        assert!(!ctrl.is_safe_mode_active());
    }

    // ── 7. Audit events ───────────────────────────────────────────────────────

    /// Audit events emitted on entry and exit.
    #[tokio::test]
    async fn test_audit_events_emitted_on_entry_and_exit() {
        let sink = Arc::new(CollectingAuditSink::new());
        let mut ctrl = make_controller_with_sink(Arc::clone(&sink));

        ctrl.enter_safe_mode_viewer_action().await;
        ctrl.exit_safe_mode().await;

        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| matches!(e.payload, AuditPayload::SafeModeEntered { .. })),
            "SafeModeEntered audit event must be emitted"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e.payload, AuditPayload::SafeModeExited)),
            "SafeModeExited audit event must be emitted"
        );
    }

    /// Freeze cancellation emits FreezeDeactivated audit event.
    #[tokio::test]
    async fn test_freeze_cancel_emits_audit_event() {
        let sink = Arc::new(CollectingAuditSink::new());
        let mut ctrl = make_controller_with_sink(Arc::clone(&sink));

        ctrl.set_freeze_active_for_test(true);
        ctrl.enter_safe_mode_viewer_action().await;

        let events = sink.drain();
        assert!(
            events
                .iter()
                .any(|e| matches!(e.payload, AuditPayload::FreezeDeactivated)),
            "FreezeDeactivated audit event must be emitted when freeze is cancelled"
        );
    }

    // ── 8. policy_matrix_basic: safe mode overrides all policy levels ─────────

    /// Acceptance criterion: test scenes: policy_matrix_basic (safe mode overrides all policy levels).
    /// WHEN safe mode entered THEN all leases at all priority levels are suspended.
    #[tokio::test]
    async fn test_safe_mode_overrides_all_policy_levels_policy_matrix_basic() {
        let shared = make_shared_state();
        let chrome = Arc::new(RwLock::new(ChromeState::new()));
        let mut ctrl = SafeModeController::new_headless(shared.clone(), chrome);

        // Grant leases at multiple priorities (simulating policy_matrix_basic agents).
        {
            let st = shared.lock().await;
            let mut scene = st.scene.lock().await;
            for ns in [
                "system.agent",
                "high.priority.agent",
                "normal.agent",
                "low.agent",
            ] {
                scene.grant_lease(ns, 60_000, vec![]);
            }
        }

        // Enter safe mode.
        let result = ctrl.enter_safe_mode_viewer_action().await;
        assert!(result.leases_suspended > 0);

        // All leases must be SUSPENDED regardless of priority.
        {
            let st = shared.lock().await;
            let scene = st.scene.lock().await;
            let all_suspended = scene
                .leases
                .values()
                .all(|l| l.state == LeaseState::Suspended || l.state.is_terminal());
            assert!(
                all_suspended,
                "ALL leases must be SUSPENDED — safe mode overrides all policy levels"
            );
        }
    }

    // ── 9. Overlay renders from ChromeState only (no scene graph) ────────────

    /// Scenario: Overlay with corrupted scene graph (spec line 111)
    /// WHEN safe mode entered due to scene graph corruption
    /// THEN overlay still renders correctly (reads only from ChromeState).
    #[tokio::test]
    async fn test_overlay_renders_from_chrome_state_only_after_critical_error() {
        use super::super::chrome::{ChromeRenderer, ViewerClass};

        let shared = make_shared_state();
        let chrome = Arc::new(RwLock::new({
            let mut cs = ChromeState::new();
            cs.viewer_class = ViewerClass::Owner;
            cs
        }));
        let mut ctrl = SafeModeController::new_headless(shared, Arc::clone(&chrome));

        // Enter safe mode simulating scene graph corruption (critical error).
        ctrl.enter_safe_mode_on_critical_error(Some("scene graph corrupted".into()))
            .await;

        assert!(
            chrome.read().unwrap().safe_mode_active,
            "ChromeState.safe_mode_active must be true"
        );

        // Chrome renderer must produce overlay commands from ChromeState alone
        // — no scene graph access required.
        let mut renderer = ChromeRenderer::new_headless(chrome);
        let cmds = renderer.render_chrome(1920.0, 1080.0);
        assert!(
            !cmds.is_empty(),
            "chrome renderer must produce commands in safe mode"
        );

        // Full-viewport dimming overlay must be present.
        let has_full_overlay = cmds
            .iter()
            .any(|c| c.x == 0.0 && c.y == 0.0 && c.width == 1920.0 && c.height == 1080.0);
        assert!(has_full_overlay, "full-viewport overlay must be present");
    }

    // ── 10. Session notification path ─────────────────────────────────────────

    /// Safe mode controller broadcasts SessionSuspended and SessionResumed to sessions
    /// that have a registered `server_message_tx`.
    ///
    /// This exercises the out-of-band broadcast mechanism end-to-end:
    ///   1. Register a `server_message_tx` for a session.
    ///   2. Enter safe mode → assert `SessionSuspended` is received.
    ///   3. Exit safe mode → assert `SessionResumed` is received.
    #[tokio::test]
    async fn test_session_notification_broadcast() {
        use tokio::sync::mpsc;
        use tze_hud_protocol::proto::session::server_message::Payload as ServerPayload;

        let shared = make_shared_state();
        let chrome = Arc::new(RwLock::new(ChromeState::new()));
        let mut ctrl = SafeModeController::new_headless(Arc::clone(&shared), chrome);

        // Authenticate a session and register a server_message_tx.
        let (tx, mut rx) = mpsc::channel(8);
        {
            let mut st = shared.lock().await;
            let session = st
                .sessions
                .authenticate("agent.notify_test", "test-key", &[])
                .expect("auth should succeed");
            let registered = st
                .sessions
                .register_server_message_tx(&session.session_id, tx);
            assert!(
                registered,
                "register_server_message_tx must return true for known session"
            );
        }

        // Enter safe mode — expect SessionSuspended.
        let entry = ctrl.enter_safe_mode_viewer_action().await;
        assert_eq!(
            entry.sessions_notified, 1,
            "one session should receive SessionSuspended"
        );
        let msg = rx.try_recv().expect("SessionSuspended must be in channel");
        let msg = msg.expect("message must be Ok");
        assert!(
            matches!(msg.payload, Some(ServerPayload::SessionSuspended(_))),
            "payload must be SessionSuspended"
        );

        // Exit safe mode — expect SessionResumed.
        let exit_result = ctrl.exit_safe_mode().await;
        assert_eq!(
            exit_result.sessions_notified, 1,
            "one session should receive SessionResumed"
        );
        let msg = rx.try_recv().expect("SessionResumed must be in channel");
        let msg = msg.expect("message must be Ok");
        assert!(
            matches!(msg.payload, Some(ServerPayload::SessionResumed(_))),
            "payload must be SessionResumed"
        );
    }
}
