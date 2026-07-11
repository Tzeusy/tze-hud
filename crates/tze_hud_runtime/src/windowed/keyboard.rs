use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::time::Instant;

use tze_hud_input::{
    AgentDispatch, AgentDispatchKind, HotkeyResizeAxis, HotkeyResizeDir, KeyboardModifiers,
    PortalFocusTarget, RawCharacterEvent, RawKeyDownEvent, RawKeyUpEvent, ShellReservedShortcut,
};

use super::WinitApp;
use super::input_dispatch::{
    dispatch_focus_event, dispatch_keyboard_event, dispatch_pointer_event,
    dispatch_scroll_offset_event,
};
use super::lifecycle::{INTERACTION_LOCK_BUDGET, spin_acquire, write_windows_clipboard_text};

pub(super) struct ComposerDeliveryContext {
    pub(super) namespace: String,
    pub(super) node_id_bytes: [u8; 16],
    pub(super) tile_id: tze_hud_scene::SceneId,
}

pub(super) enum ComposerDeliveryContextLookup {
    Ready(ComposerDeliveryContext),
    Busy,
    Unavailable,
}

// ── Debug-log preview helpers ─────────────────────────────────────────────────

/// Return a 64-byte-bounded preview of `s` for use in `tracing` fields.
///
/// Mirrors the inline `char_log_preview` block introduced in PR #768 (hud-60hgf)
/// for `raw.character`. Reused here to bound all unbounded string fields in
/// debug-level tracing callsites (key names, namespaces, character payloads).
///
/// Returns a borrowed `&str` for strings that already fit (zero allocation),
/// and an owned `String` with an appended `…` ellipsis for longer inputs.
fn str_preview(s: &str) -> std::borrow::Cow<'_, str> {
    const MAX: usize = 64;
    if s.len() <= MAX {
        std::borrow::Cow::Borrowed(s)
    } else {
        let mut end = MAX;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        std::borrow::Cow::Owned(format!("{}…", &s[..end]))
    }
}

/// Return a bounded string summary of a [`tze_hud_input::KeyboardDispatchKind`]
/// for tracing fields.
///
/// The `Character` variant may carry an arbitrarily large clipboard payload;
/// this helper applies [`str_preview`] to any embedded string fields so that
/// the log line stays bounded regardless of paste size.  `KeyDown`/`KeyUp`
/// key names are also previewed for consistency.
fn keyboard_kind_preview(kind: &tze_hud_input::KeyboardDispatchKind) -> String {
    use tze_hud_input::KeyboardDispatchKind;
    match kind {
        KeyboardDispatchKind::KeyDown {
            key_code,
            key,
            modifiers,
            repeat,
            ..
        } => format!(
            "KeyDown {{ key_code: {:?}, key: {:?}, ctrl: {}, shift: {}, alt: {}, repeat: {} }}",
            str_preview(key_code),
            str_preview(key),
            modifiers.ctrl,
            modifiers.shift,
            modifiers.alt,
            repeat,
        ),
        KeyboardDispatchKind::KeyUp {
            key_code,
            key,
            modifiers,
            ..
        } => format!(
            "KeyUp {{ key_code: {:?}, key: {:?}, ctrl: {}, shift: {}, alt: {} }}",
            str_preview(key_code),
            str_preview(key),
            modifiers.ctrl,
            modifiers.shift,
            modifiers.alt,
        ),
        KeyboardDispatchKind::Character { character, .. } => {
            format!("Character {{ character: {:?} }}", str_preview(character))
        }
    }
}

/// Stable identity for a portal-resize chord, used to pair a consumed `KeyDown`
/// with its later `KeyUp` so the key-up fallback (below) never double-resizes a
/// normal physical key-down/key-up pair.
///
/// Prefer the physical `key_code` (layout- and modifier-independent: `Equal`,
/// `Minus`, `NumpadAdd`, …); fall back to the logical `key` only when the OS
/// did not supply a code.
fn portal_resize_key_code(raw_key_code: &str, raw_key: &str) -> String {
    if raw_key_code.is_empty() {
        raw_key.to_string()
    } else {
        raw_key_code.to_string()
    }
}

/// Resolve a directional whole-portal resize chord (Ctrl+Shift+Arrow, hud-csrmf).
///
/// Returns the `(direction, axis)` pair for the arrow chords, or `None` for any
/// other key or modifier combination:
///
/// | Chord | Result |
/// |-------|--------|
/// | Ctrl+Shift+ArrowRight | grow width |
/// | Ctrl+Shift+ArrowLeft | shrink width |
/// | Ctrl+Shift+ArrowDown | grow height |
/// | Ctrl+Shift+ArrowUp | shrink height |
///
/// Both Ctrl and Shift are required; Alt / Meta must NOT be held (an OS window-
/// management chord, never a portal resize). The physical `key_code` is used so
/// the chord is layout-independent. Growing width toward the right / height
/// toward the bottom matches the top-left anchoring of the pointer and
/// Ctrl+`+`/`-` resize paths.
fn directional_portal_resize(
    key_code: &str,
    modifiers: &KeyboardModifiers,
) -> Option<(HotkeyResizeDir, HotkeyResizeAxis)> {
    if !modifiers.ctrl || !modifiers.shift || modifiers.alt || modifiers.meta {
        return None;
    }
    match key_code {
        "ArrowRight" => Some((HotkeyResizeDir::Grow, HotkeyResizeAxis::Width)),
        "ArrowLeft" => Some((HotkeyResizeDir::Shrink, HotkeyResizeAxis::Width)),
        "ArrowDown" => Some((HotkeyResizeDir::Grow, HotkeyResizeAxis::Height)),
        "ArrowUp" => Some((HotkeyResizeDir::Shrink, HotkeyResizeAxis::Height)),
        _ => None,
    }
}

/// Whether a key event is the bare Tab / Shift+Tab focus-traversal chord
/// (hud-v0cal).
///
/// `true` only for `Tab` with no Ctrl / Alt / Meta held — the exact chord that
/// drives keyboard focus traversal.  Shift is allowed (Shift+Tab = reverse
/// traversal).  Ctrl+Tab / Ctrl+Shift+Tab are shell-reserved (tab switching);
/// Alt+Tab and Win/Cmd+Tab are OS window switchers — all excluded.
///
/// Used to keep the key-down intercept (`dispatch_key_down_event_inner`) and the
/// key-up swallow (`dispatch_key_up_event_inner`) in lockstep: a bare Tab
/// key-down is consumed for traversal, so its matching key-up must be swallowed
/// too, never reaching the composer or agent as a raw release.
fn is_bare_tab_chord(key: &str, modifiers: &KeyboardModifiers) -> bool {
    key == "Tab" && !modifiers.ctrl && !modifiers.alt && !modifiers.meta
}

/// A raw keyboard event deferred from the winit event-loop thread because the
/// shared-state or scene Tokio mutex was busy at dispatch time (hud-2fz34).
///
/// Stored in `WindowedRuntimeState::pending_keyboard_events` and retried once
/// per `about_to_wait` iteration via `drain_pending_keyboard_events`, matching
/// the `pending_input_capture_commands` / `drain_portal_projection` sibling
/// patterns in the same file.
#[derive(Debug)]
pub(super) enum PendingKeyboardEvent {
    KeyDown(RawKeyDownEvent),
    KeyUp(RawKeyUpEvent),
    Character(RawCharacterEvent),
}

/// Outcome of a Tab / Shift+Tab keyboard focus-traversal attempt (hud-v0cal).
///
/// `Done` — the traversal ran (the FocusManager cycle advanced, or was empty)
/// and the Tab event was consumed.
/// `Busy` — a required lock (shared-state / scene) stayed contended past the
/// bounded `INTERACTION_LOCK_BUDGET` spin; the caller defers the Tab event to
/// the next `about_to_wait` drain (the overrun fallback).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TabFocusOutcome {
    Done,
    Busy,
}

/// Outcome of an Escape-clear attempt on a composer-less focus stop (hud-k6yvb).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum EscapeClearOutcome {
    /// Focus was released (a node/tile owner was cleared).
    Cleared,
    /// No owner to clear — the caller proceeds with normal Escape handling.
    NoOwner,
    /// A required lock stayed contended — the caller defers the event.
    Busy,
}

/// Synthetic device id used for keyboard-driven control activation (hud-2v8br).
///
/// Enter / Space on a Tab-focused portal control synthesizes a pointer
/// down+up so the owning agent's existing click handler fires. A distinct,
/// out-of-range id keeps these synthetic events from colliding with real
/// pointer-device state (capture, drag) keyed by device id.
const KEYBOARD_ACTIVATION_DEVICE_ID: u32 = u32::MAX;

/// Result of classifying the active tab's keyboard focus for portal
/// typing-recovery (hud-2v8br). The runtime-local analogue of
/// [`tze_hud_input::PortalFocusTarget`], enriched with the namespace and
/// pointer coordinates needed to synthesize a control activation, plus a `Busy`
/// state for lock contention so the caller can defer the event.
enum PortalKeyFocus {
    /// A required lock stayed contended — defer the event and retry.
    Busy,
    /// Focus is on the composer — normal editing, no recovery.
    Composer,
    /// Focus is anywhere recovery does not apply.
    Other,
    /// Focus is on a non-composer portal control sharing a tile with a composer.
    Control(PortalControlSnapshot),
}

/// Owned snapshot of a focused portal control (hud-2v8br), captured under the
/// scene lock so the runtime can act (refocus / activate) after releasing it.
struct PortalControlSnapshot {
    tile_id: tze_hud_scene::SceneId,
    node_id: tze_hud_scene::SceneId,
    interaction_id: String,
    /// The sibling composer node to redirect typing / Escape into.
    composer_node: tze_hud_scene::SceneId,
    /// Owning agent namespace (for the synthetic activation dispatch).
    namespace: String,
    /// Control center in tile-local coordinates.
    local_x: f32,
    local_y: f32,
    /// Control center in display-space coordinates.
    display_x: f32,
    display_y: f32,
}

/// Returns `true` for the Escape key (recovery-to-composer trigger, hud-2v8br).
fn is_escape_key(key: &str, key_code: &str) -> bool {
    key_code == "Escape" || key == "Escape"
}

/// Returns `true` for Enter / Space — the activation keys for a Tab-focused
/// portal control (hud-2v8br), mirroring a pointer click.
fn is_activation_key(key: &str, key_code: &str) -> bool {
    matches!(key_code, "Enter" | "NumpadEnter" | "Space")
        || matches!(key, "Enter" | "\r" | "\n" | " ")
}

fn restore_front_requeued_event(
    pending_keyboard_events: &mut VecDeque<PendingKeyboardEvent>,
    len_after_pop: usize,
) -> bool {
    if pending_keyboard_events.len() <= len_after_pop {
        return false;
    }

    if let Some(requeued_event) = pending_keyboard_events.pop_back() {
        pending_keyboard_events.push_front(requeued_event);
    }
    true
}

/// Bounded FIFO keyboard event drain helper (hud-dwcr7).
///
/// Calls `dispatch` at most `limit` times — where `limit` is the queue length
/// at the drain call-site, computed once by the caller.  Each call to `dispatch`
/// may return:
///
/// - `ControlFlow::Continue(())` — the event was processed; keep draining.
/// - `ControlFlow::Break(())` — stop immediately: either the active-tab mirror
///   was momentarily busy (before the pop), or an inner-dispatch busy-defer pushed
///   the event to the tail and `restore_front_requeued_event` moved it back to the
///   front (so no later event can overtake it).
///
/// The `limit` bound (`for _ in 0..limit`, **not** `while !queue.is_empty()`) is
/// the primary fix for the hud-dwcr7 livelock: events that arrive *during* the
/// drain — pushed by inner dispatch or from the OS event path — are deferred to the
/// *next* `about_to_wait` cycle rather than processed in the same pass.  Without
/// the bound, a producer that continuously enqueues events could prevent the drain
/// from ever returning, turning a single `about_to_wait` tick into an unbounded
/// dispatch storm.
///
/// Extracted from `drain_pending_keyboard_events` so the bounding invariant is
/// independently testable without a full `WinitApp` state machine (hud-b09ag).
fn drain_keyboard_queue_bounded<F>(limit: usize, mut dispatch: F)
where
    F: FnMut() -> ControlFlow<()>,
{
    for _ in 0..limit {
        if dispatch().is_break() {
            break;
        }
    }
}

impl WinitApp {
    /// Enqueue and process a keyboard-originated scroll event (PgUp / PgDn).
    ///
    /// Uses the current cursor position for hit-testing, exactly like wheel
    /// scroll.  Delegates to
    /// [`InputProcessor::process_keyboard_scroll`] which applies the same
    /// local-first coalescing and clamping as `process_scroll_event`.
    ///
    /// Dispatches a `ScrollOffsetChangedEvent` to the tile-owning agent via the
    /// `INPUT_EVENTS` channel when the scroll changes the tile offset.
    pub(super) fn enqueue_keyboard_scroll_event(&mut self, delta_y: f32) {
        let x = self.state.cursor_x;
        let y = self.state.cursor_y;

        if let Ok(state) = self.state.shared_state.try_lock()
            && let Ok(mut scene) = state.scene.try_lock()
        {
            if let Some(ev) = self
                .state
                .input_processor
                .process_keyboard_scroll(x, y, delta_y, &mut scene)
            {
                dispatch_scroll_offset_event(&self.state.input_event_tx, &scene, ev);
            }
        }
    }

    /// Snap the input-pane history's scroll offset back to the tail
    /// (hud-qbcp8).
    ///
    /// Called on every accepted composer keystroke: unlike ordinary content
    /// growth (which deliberately leaves a `ScrolledBack` viewport alone, spec
    /// §3.3), a viewer who is actively typing into their own composer should
    /// not be stranded scrolled away from their own live draft. No-op (and no
    /// scene write) once the tile is already at the tail.
    pub(super) fn reset_input_history_scroll_to_tail(&mut self, tile_id: tze_hud_scene::SceneId) {
        if let Ok(state) = self.state.shared_state.try_lock()
            && let Ok(mut scene) = state.scene.try_lock()
        {
            self.state
                .input_processor
                .reset_tile_scroll_to_tail(tile_id, &mut scene);
        }
    }

    // ── Keyboard drain helpers ────────────────────────────────────────────

    /// Translate a raw key-down event through the `KeyboardProcessor`, log it,
    /// and broadcast the resulting `KeyboardDispatch` over the `INPUT_EVENTS`
    /// gRPC channel via `input_event_tx`.
    ///
    /// If `current_owner` is `FocusOwner::None` (no focused agent session),
    /// `KeyboardProcessor::process_key_down` returns `None` and the event is
    /// silently dropped — there is no recipient to deliver to.
    ///
    /// Delivery is best-effort (fire-and-forget): if the channel has no
    /// receivers (gRPC disabled, agent not subscribed) the broadcast error is
    /// silently ignored, consistent with the transactional keyboard-event
    /// contract where dropped delivery is an infrastructure gap, not a
    /// data-loss policy.
    ///
    /// # Composer interception (§4.4)
    ///
    /// When a composer region is focused (`accepts_composer_input = true`), the
    /// event is first offered to the `ComposerDraftManager` via
    /// `route_key_down_to_composer`.  If the manager consumes the event
    /// (`consumed = true`), it is NOT forwarded to the agent as a raw
    /// `KeyDownEvent`.  Any transactional batch returned (submit / cancel) is
    /// handed to `deliver_composer_batch` for future downstream delivery.
    /// Public Stage-1 entry for an OS key-down event.
    ///
    /// Applies the early gates that MUST run on the OS-event path — safe-mode
    /// capture, the FIFO ordering guard, and the active-tab availability check —
    /// then delegates the actual routing to [`Self::dispatch_key_down_event_inner`].
    ///
    /// FIFO ordering invariant (hud-2fz34): when events are already queued in
    /// `pending_keyboard_events`, a freshly-arriving Stage-1 event must NOT jump
    /// ahead of them, so it is appended to the queue and processed later by
    /// `drain_pending_keyboard_events`.  The drain calls the *inner* directly
    /// (bypassing this guard) so a queued event is actually consumed instead of
    /// being rotated to the back forever (the livelock fixed in hud-dwcr7).
    pub(super) fn dispatch_key_down_event(&mut self, raw: &RawKeyDownEvent) {
        // ── Priority 1: Safe-mode capture ─────────────────────────────────
        // (See the inner fn / historical comments for the full precedence
        // rationale.)  Lock-free AtomicBool mirror read; never fails under
        // contention.  Safe mode owns ALL input — drop before anything else.
        if self
            .state
            .safe_mode_atomic
            .load(std::sync::atomic::Ordering::Acquire)
        {
            tracing::debug!(
                key = %str_preview(&raw.key),
                "safe-mode capture: key dropped (safe mode active — chrome layer owns input)"
            );
            return;
        }

        // FIFO guard: if earlier events are still pending, queue this one
        // immediately so it cannot bypass them even when the lock is free.
        if !self.state.pending_keyboard_events.is_empty() {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
            return;
        }
        // Resolve the active tab via the lock-free mirror (hud-dwcr7).
        // None = mirror momentarily busy → defer to the next about_to_wait.
        let Some(active_tab) = self.active_tab_for_keyboard_dispatch() else {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
            return;
        };
        self.dispatch_key_down_event_inner(raw, active_tab);
    }

    /// FIFO-ordered inner routing for a key-down event.
    ///
    /// Called by [`Self::dispatch_key_down_event`] (Stage-1) with the active tab
    /// already resolved, and by `drain_pending_keyboard_events` for queued
    /// events.  This MUST NOT re-apply the FIFO guard (`pending_keyboard_events`
    /// non-empty), since the drain processes the queue in order and a guard here
    /// would rotate the front event to the back forever (hud-dwcr7 livelock).
    ///
    /// `active_tab` is the value already read from the mirror: `Some(tab)` to
    /// route, `None` to drop (no active tab → no composer / agent target).
    pub(super) fn dispatch_key_down_event_inner(
        &mut self,
        raw: &RawKeyDownEvent,
        active_tab: Option<tze_hud_scene::SceneId>,
    ) {
        // No active tab → nothing to route to.  Drop (do not re-queue / spin).
        let Some(tab_id) = active_tab else { return };

        // ── Priority 2: Shell/chrome-reserved shortcuts ───────────────────
        //
        // Shell-reserved shortcuts (Ctrl+Tab, Ctrl+1..9, Ctrl+Shift+M, etc.)
        // MUST win over portal resize hotkeys.  A reserved key is never
        // consumed by a portal.  The portal-resize intercept is skipped so
        // the reserved key is never consumed by a portal, but normal routing
        // still runs so the key reaches the agent (e.g. chrome handles Ctrl+Tab
        // at a higher layer, but the event is not suppressed here).
        //
        // Note: Ctrl+Shift+F8/F9 (monitor cycling) is handled even earlier —
        // in the OS event path (Stage 1, `WindowEvent::KeyboardInput`) — so it
        // never reaches this function at all.  The `is_reserved` check below
        // handles the remaining reserved set that does reach here.
        if ShellReservedShortcut::is_reserved(
            &raw.key,
            raw.modifiers.ctrl,
            raw.modifiers.shift,
            raw.modifiers.alt,
        ) {
            tracing::debug!(
                key = %str_preview(&raw.key),
                ctrl = raw.modifiers.ctrl,
                shift = raw.modifiers.shift,
                "shell-reserved shortcut: portal resize skipped (chrome layer handles)"
            );
            // Fall through to normal routing — the key may still need to be
            // delivered to the agent (e.g. Ctrl+Tab dispatched as a chrome
            // event, not suppressed entirely).  The important invariant is
            // that portal resize DOES NOT consume it.
        } else {
            // ── Priority 4: Portal resize hotkey intercept (§6b.2) ────────
            //
            // Ctrl+`+`/Ctrl+`=` (grow) and Ctrl+`-` (shrink) resize the
            // focused portal tile.  The hotkey is focus-scoped: only the
            // portal that holds keyboard focus consumes it.
            //
            // Composer focus is still portal-surface focus.  Run this before
            // composer draft routing so Ctrl resize chords can resize a
            // focused text-stream portal without leaking to the agent.
            //
            // The hotkey is consumed (returns early) when applied so it does
            // NOT propagate to the composer or the agent's raw KeyDown path.
            // Resolve the resize direction from the logical key first, then fall
            // back to the **physical** KeyCode. The logical match is fragile
            // under Ctrl on Windows (winit may not resolve bare `=`/`-`/`+`, and
            // `+` needs Shift), which is the root cause of hud-v4k1h — the
            // physical fallback (`Equal`/`Minus`/numpad) makes the chord
            // deterministic regardless of layout or held modifiers.
            if let Some(dir) = HotkeyResizeDir::from_key(&raw.key, raw.modifiers.ctrl)
                .or_else(|| HotkeyResizeDir::from_key_code(&raw.key_code, raw.modifiers.ctrl))
            {
                if self.apply_portal_resize_hotkey(tab_id, dir, HotkeyResizeAxis::Both) {
                    // Remember this chord so its matching `KeyUp` is swallowed by
                    // the key-up fallback instead of resizing a second time
                    // (hud-v4k1h: live Windows can deliver key-up-only streams, so
                    // the key-up path also resizes — this dedup keeps a normal
                    // down/up pair to exactly one resize).
                    self.state
                        .consumed_portal_resize_keydowns
                        .insert(portal_resize_key_code(&raw.key_code, &raw.key));
                    tracing::debug!(
                        key = %str_preview(&raw.key),
                        key_code = %str_preview(&raw.key_code),
                        "portal resize: Ctrl hotkey consumed (resize applied)"
                    );
                    return;
                }
            }
            // ── Priority 4b: Directional whole-portal resize (Ctrl+Shift+Arrow) ─
            //
            // A per-axis whole-portal resize affordance (hud-csrmf): Ctrl+Shift+
            // Left/Right steps WIDTH, Up/Down steps HEIGHT. It takes the same
            // viewer-geometry-lock as the pointer-drag and Ctrl+`+`/`-` paths, so
            // a width step reflows the transcript and drives the dynamic hud-rpmwt
            // reconcile-on-republish path WITHOUT pointer injection — the axis the
            // autopilot live-verify needs on a multi-monitor console where the
            // 22px pointer resize handle is unreachable.
            //
            // Gated on `!is_composer_active()` so it never steals the composer's
            // Ctrl+Shift+Arrow word/line-selection while the viewer is typing; a
            // keyboard viewer (or the autopilot) reaches it by focusing a
            // non-composer surface of the portal (click transcript / Tab to a
            // control), matching the click-to-focus and Tab focus semantics.
            else if !self.state.input_processor.is_composer_active()
                && let Some((dir, axis)) = directional_portal_resize(&raw.key_code, &raw.modifiers)
                && self.apply_portal_resize_hotkey(tab_id, dir, axis)
            {
                self.state
                    .consumed_portal_resize_keydowns
                    .insert(portal_resize_key_code(&raw.key_code, &raw.key));
                tracing::debug!(
                    key_code = %str_preview(&raw.key_code),
                    ?axis,
                    "portal resize: Ctrl+Shift+Arrow directional hotkey consumed (resize applied)"
                );
                return;
            }
        }

        // ── Keyboard focus traversal: Tab / Shift+Tab (hud-v0cal) ─────────
        //
        // Tab advances keyboard focus to the next focusable affordance and
        // Shift+Tab to the previous one, so the composer (and any focusable
        // region) is reachable WITHOUT a pointer — the no-pointer surfaces this
        // contract targets (smart glasses / Mobile Presence Node).  Spec change
        // `portal-composer-interaction-completeness`, "Transcript Interaction
        // Contract", scenario "composer is focusable without a pointer".
        //
        // Scoping precedence: this runs AFTER the safe-mode capture (outer fn),
        // the shell/chrome-reserved set, and the portal-resize hotkey, and
        // BEFORE composer/agent routing.  Ctrl+Tab / Ctrl+Shift+Tab are
        // shell-reserved (tab switching, handled at the chrome layer) and are
        // excluded here by the `!ctrl` guard; only the bare Tab / Shift+Tab
        // chord drives focus traversal.  Alt+Tab (OS window switcher) and
        // Meta+Tab (Win+Tab on Windows / Cmd+Tab on macOS — also OS window
        // switchers) are likewise excluded.  The Tab key is always consumed so
        // it is never forwarded to the composer draft or the agent as raw input.
        if is_bare_tab_chord(&raw.key, &raw.modifiers) {
            if self.navigate_portal_focus(tab_id, raw.modifiers.shift) == TabFocusOutcome::Busy {
                // A required lock was busy — defer and retry on the next drain,
                // mirroring the composer/namespace busy-defer paths below.
                self.state
                    .pending_keyboard_events
                    .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
            }
            return;
        }

        // ── Portal control keyboard recovery / activation (hud-2v8br) ──────
        //
        // When Tab has parked focus on a non-composer portal control (minimize /
        // restore / submit), a keyboard-only viewer must never be stranded:
        //   - Escape recovers focus to the composer (explicit recovery),
        //   - Enter / Space activate the control (mirroring a pointer click),
        //   - any other key is consumed here so it never leaks to the agent as
        //     raw input; printable text recovers to the composer via the
        //     following Character event (dispatch_character_event_inner).
        // Gated on `!is_composer_active()` so normal composer editing (where the
        // composer is the focus sink) is untouched — Enter submits, Space types,
        // Escape cancels there, all handled by the intercept below.
        if !self.state.input_processor.is_composer_active() {
            match self.portal_key_focus(tab_id) {
                PortalKeyFocus::Busy => {
                    self.state
                        .pending_keyboard_events
                        .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
                    return;
                }
                PortalKeyFocus::Control(control) => {
                    if is_escape_key(&raw.key, &raw.key_code) {
                        if self.refocus_composer_from_control(
                            tab_id,
                            control.tile_id,
                            control.composer_node,
                        ) == TabFocusOutcome::Busy
                        {
                            self.state
                                .pending_keyboard_events
                                .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
                        }
                        return;
                    }
                    if is_activation_key(&raw.key, &raw.key_code) {
                        self.activate_portal_control(&control);
                        return;
                    }
                    // Consume: printable text recovers via the Character event.
                    return;
                }
                PortalKeyFocus::Other => {
                    // Escape on a composer-less focus stop (a focusable node with
                    // no composer sibling, or a tile-level stop) clears focus so
                    // the keyboard user is never stranded on a control they cannot
                    // type into — a later Tab re-enters the cycle (hud-k6yvb).
                    if is_escape_key(&raw.key, &raw.key_code) {
                        match self.try_escape_clear_focus(tab_id) {
                            EscapeClearOutcome::Cleared => return,
                            EscapeClearOutcome::Busy => {
                                self.state
                                    .pending_keyboard_events
                                    .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
                                return;
                            }
                            EscapeClearOutcome::NoOwner => {}
                        }
                    }
                }
                PortalKeyFocus::Composer => {}
            }
        }

        // ── Composer draft intercept (§4.4) ──────────────────────────────
        if self.state.input_processor.is_composer_active() {
            let delivery_context = match self.composer_delivery_context_for_tab(tab_id) {
                ComposerDeliveryContextLookup::Ready(context) => Some(context),
                ComposerDeliveryContextLookup::Busy => {
                    self.state
                        .pending_keyboard_events
                        .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
                    return;
                }
                ComposerDeliveryContextLookup::Unavailable => None,
            };
            // Captured before `delivery_context` is moved into
            // `route_and_deliver_composer_batch` below (hud-qbcp8: input-
            // history reset-to-tail needs the tile id after that move).
            let history_tile_id = delivery_context.as_ref().map(|c| c.tile_id);
            // Soft-wrap vertical caret movement (hud-21o6x): for ArrowUp/ArrowDown,
            // hand the input layer the compositor's latest wrapped-line layout so
            // the caret can step between visual rows. Read the reverse channel only
            // for the vertical keys (avoids cloning the layout on every keystroke);
            // `None`/stale simply falls back to hard-newline movement.
            if matches!(raw.key_code.as_str(), "ArrowUp" | "ArrowDown") {
                let layout = self
                    .state
                    .composer_visual_layout
                    .lock()
                    .ok()
                    .and_then(|g| g.clone());
                self.state
                    .input_processor
                    .set_composer_visual_layout(layout);
            }
            // Ctrl+C / Ctrl+X (copy / cut): snapshot the current selection to the
            // OS clipboard BEFORE routing the keystroke — a cut mutates the draft
            // in `route_key_down_to_composer` below, so the copy must be read
            // first. The input layer then consumes the keystroke (and, for cut,
            // deletes the selection), so it never leaks to the agent. Clipboard
            // write is a no-op off Windows; an empty selection writes nothing
            // (hud-hxhnt). Ctrl+Alt is excluded so it cannot shadow AltGr.
            if raw.modifiers.ctrl
                && !raw.modifiers.alt
                && matches!(raw.key_code.as_str(), "KeyC" | "KeyX")
            {
                if let Some(selected) = self.state.input_processor.composer_selected_text() {
                    write_windows_clipboard_text(&selected);
                }
            }
            // Capture the input-started-at instant for local-ack latency
            // measurement on the composer keystroke path (hud-r3ax6 / hud-o9ybl).
            let composer_input_started = Instant::now();
            let (consumed, batch) = self.state.input_processor.route_key_down_to_composer(
                &raw.key_code,
                &raw.key,
                raw.modifiers.shift,
                raw.modifiers.ctrl,
                raw.modifiers.alt,
            );
            // Track whether this keystroke submitted or cancelled the composer so
            // we can suppress the push below (clear must win over push; hud-r3ax6).
            let mut key_down_is_terminal = false;
            if let Some(b) = batch {
                let is_submission = b.submission.is_some();
                key_down_is_terminal = b.cancel.is_some() || is_submission;
                if let Some(context) = delivery_context {
                    self.route_and_deliver_composer_batch(context, b);
                }
                if key_down_is_terminal {
                    // Submit or cancel: clear the local echo overlay.
                    self.clear_local_composer_echo();
                }
                // hud-npcdf: a submission sets `key_down_is_terminal`, so the
                // reset-to-tail in the `!key_down_is_terminal` branch below is
                // skipped — and a ProjectionAuthority-attached tile echoes its own
                // submission async via `portal_projection_driver`'s
                // `notify_tile_content_appended`, which honors `ScrolledBack` and
                // does not reset. A scrolled-back viewer's just-submitted reply
                // would then stay off-screen. Snap the input-pane history back to
                // the tail on SUBMIT (not cancel) so the reply the viewer just
                // sent is revealed. The raw-tile path already resets inside
                // `append_raw_tile_viewer_echo`; `reset_input_history_scroll_to_tail`
                // is idempotent, so the redundant call there is harmless.
                if is_submission {
                    if let Some(tile_id) = history_tile_id {
                        self.reset_input_history_scroll_to_tail(tile_id);
                    }
                }
            }
            if consumed {
                tracing::debug!(
                    key_code = %str_preview(&raw.key_code),
                    "composer: KeyDown consumed by draft manager"
                );
                // Push updated draft snapshot for local echo rendering (hud-r3ax6).
                // Guard: do NOT push after a terminal batch — clear must win.
                if !key_down_is_terminal {
                    self.push_local_composer_echo(composer_input_started);
                    // hud-qbcp8: typing should not leave the input-pane history
                    // scrolled away from the viewer's own live draft.
                    if let Some(tile_id) = history_tile_id {
                        self.reset_input_history_scroll_to_tail(tile_id);
                    }
                }
                return;
            }
        }

        let focus_owner = self.state.focus_manager.current_owner(tab_id).clone();

        // Build a namespace-resolver closure: given a tile_id, return its
        // agent namespace from the scene.  A Cell is used to propagate a
        // lock-busy signal out of the closure so the caller can defer the
        // event (hud-2fz34).
        let ns_lock_busy = std::cell::Cell::new(false);
        let namespace_fn = |tile_id: tze_hud_scene::SceneId| -> Option<String> {
            match self.namespace_for_keyboard_tile(tile_id) {
                None => {
                    ns_lock_busy.set(true);
                    None
                }
                Some(ns) => ns,
            }
        };
        if let Some(dispatch) =
            self.state
                .keyboard_processor
                .process_key_down(raw, &focus_owner, namespace_fn)
        {
            tracing::debug!(
                namespace = %str_preview(&dispatch.namespace),
                tile_id = ?dispatch.tile_id,
                node_id = ?dispatch.node_id,
                kind = %keyboard_kind_preview(&dispatch.kind),
                "keyboard: KeyDown dispatched to agent"
            );
            dispatch_keyboard_event(&self.state.input_event_tx, dispatch);
        } else if ns_lock_busy.get() {
            // Namespace lock was busy inside the closure; defer the whole event.
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::KeyDown(raw.clone()));
        }
    }

    /// Advance keyboard focus by one step for a Tab / Shift+Tab key-down
    /// (hud-v0cal) and broadcast the resulting focus transition.
    ///
    /// Drives [`tze_hud_input::InputProcessor::navigate_focus`] — the no-pointer
    /// analogue of the pointer click-to-focus path — so the composer (and any
    /// focusable region) is reachable without a pointer.  The composer
    /// activation / hit-region-focus bookkeeping is handled inside
    /// `navigate_focus`; this method mirrors the pointer path's transition
    /// handling in `lifecycle.rs` (FocusGained/FocusLost broadcast over the
    /// FOCUS_EVENTS channel, plus the composer blur delivery-context capture and
    /// local-echo clear).
    ///
    /// Returns [`TabFocusOutcome::Busy`] when the shared-state or scene lock
    /// stays contended past [`INTERACTION_LOCK_BUDGET`], so the caller can defer
    /// the Tab event to the next drain (the overrun fallback).
    fn navigate_portal_focus(
        &mut self,
        tab_id: tze_hud_scene::SceneId,
        reverse: bool,
    ) -> TabFocusOutcome {
        // A Tab / Shift+Tab focus traversal is a discrete, deliberate user action
        // that must produce visible feedback this frame, so acquire with a bounded
        // spin (INTERACTION_LOCK_BUDGET) rather than a single try_lock — the same
        // strategy `apply_portal_resize_hotkey` uses for the resize hotkey, also a
        // discrete keyboard action.  A single contended try_lock (compositor /
        // streaming publish holding the scene) would otherwise force a full
        // about_to_wait round-trip before the focus moves.  navigate_focus needs
        // &mut scene to update hit_region_states focus flags; on a true overrun we
        // fall back to deferring the event via TabFocusOutcome::Busy.
        let transition = {
            let Some(state) = spin_acquire(&self.state.shared_state, INTERACTION_LOCK_BUDGET)
            else {
                tracing::trace!("tab focus traversal deferred: shared_state lock budget elapsed");
                return TabFocusOutcome::Busy;
            };
            let Some(mut scene) = spin_acquire(&state.scene, INTERACTION_LOCK_BUDGET) else {
                tracing::trace!("tab focus traversal deferred: scene lock budget elapsed");
                return TabFocusOutcome::Busy;
            };
            self.state.input_processor.navigate_focus(
                &mut self.state.focus_manager,
                &mut scene,
                tab_id,
                reverse,
            )
        };

        // Mirror the pointer path's transition handling (lifecycle.rs
        // process_with_focus broadcast block).  The locks are already released.
        let mut composer_focus_lost = false;
        if let Some((ev, ns)) = &transition.gained {
            tracing::debug!(
                namespace = %str_preview(ns),
                tile_id = ?ev.tile_id,
                node_id = ?ev.node_id,
                source = ?ev.source,
                reverse,
                "tab focus traversal: focus gained"
            );
            // A new focus-gain clears any stale blur delivery context.
            self.state.pending_blur_delivery_context = None;
        }
        if let Some((ev, ns)) = &transition.lost {
            tracing::debug!(
                namespace = %str_preview(ns),
                tile_id = ?ev.tile_id,
                node_id = ?ev.node_id,
                reason = ?ev.reason,
                "tab focus traversal: focus lost"
            );
            // Capture the composer delivery context while namespace + node_id +
            // tile_id are all still known, so a blur-triggered terminal draft
            // batch can still be delivered at the next settle (§4.3 flush
            // guarantee on blur).
            if let Some(node_id) = ev.node_id {
                self.state.pending_blur_delivery_context = Some(ComposerDeliveryContext {
                    namespace: ns.clone(),
                    node_id_bytes: *node_id.as_uuid().as_bytes(),
                    tile_id: ev.tile_id,
                });
            }
            composer_focus_lost = true;
        }

        dispatch_focus_event(&self.state.input_event_tx, transition);

        // Clear the local composer echo overlay if focus left a composer region.
        if composer_focus_lost {
            self.clear_local_composer_echo();
        }

        TabFocusOutcome::Done
    }

    /// Classify the active tab's keyboard focus for portal typing-recovery
    /// (hud-2v8br) under a bounded scene lock.
    ///
    /// Returns [`PortalKeyFocus::Busy`] when the shared-state / scene lock stays
    /// contended past [`INTERACTION_LOCK_BUDGET`] (caller defers the event);
    /// otherwise the classification, with a [`PortalControlSnapshot`] (namespace
    /// + activation coordinates resolved from the scene) for the `Control` case.
    fn portal_key_focus(&self, tab_id: tze_hud_scene::SceneId) -> PortalKeyFocus {
        let Some(state) = spin_acquire(&self.state.shared_state, INTERACTION_LOCK_BUDGET) else {
            return PortalKeyFocus::Busy;
        };
        let Some(scene) = spin_acquire(&state.scene, INTERACTION_LOCK_BUDGET) else {
            return PortalKeyFocus::Busy;
        };
        let owner = self.state.focus_manager.current_owner(tab_id).clone();
        match self
            .state
            .input_processor
            .classify_portal_focus(&owner, &scene)
        {
            PortalFocusTarget::Composer => PortalKeyFocus::Composer,
            PortalFocusTarget::Other => PortalKeyFocus::Other,
            PortalFocusTarget::Control {
                tile_id,
                node_id,
                interaction_id,
                composer_node,
            } => {
                let (Some(tile), Some(node)) =
                    (scene.tiles.get(&tile_id), scene.nodes.get(&node_id))
                else {
                    return PortalKeyFocus::Other;
                };
                let tze_hud_scene::types::NodeData::HitRegion(hr) = &node.data else {
                    return PortalKeyFocus::Other;
                };
                let local_x = hr.bounds.x + hr.bounds.width / 2.0;
                let local_y = hr.bounds.y + hr.bounds.height / 2.0;
                PortalKeyFocus::Control(PortalControlSnapshot {
                    tile_id,
                    node_id,
                    interaction_id,
                    composer_node,
                    namespace: tile.namespace.clone(),
                    local_x,
                    local_y,
                    display_x: tile.bounds.x + local_x,
                    display_y: tile.bounds.y + local_y,
                })
            }
        }
    }

    /// Redirect keyboard focus from a portal control to its sibling composer
    /// (typing / Escape recovery, hud-2v8br) and broadcast the transition.
    ///
    /// Mirrors [`Self::navigate_portal_focus`]'s lock + broadcast handling.
    /// Returns [`TabFocusOutcome::Busy`] on lock contention so the caller defers.
    fn refocus_composer_from_control(
        &mut self,
        tab_id: tze_hud_scene::SceneId,
        tile_id: tze_hud_scene::SceneId,
        composer_node: tze_hud_scene::SceneId,
    ) -> TabFocusOutcome {
        let transition = {
            let Some(state) = spin_acquire(&self.state.shared_state, INTERACTION_LOCK_BUDGET)
            else {
                return TabFocusOutcome::Busy;
            };
            let Some(mut scene) = spin_acquire(&state.scene, INTERACTION_LOCK_BUDGET) else {
                return TabFocusOutcome::Busy;
            };
            self.state.input_processor.recover_composer_focus(
                &mut self.state.focus_manager,
                &mut scene,
                tab_id,
                tile_id,
                composer_node,
            )
        };

        // Mirror the pointer / Tab transition broadcast (locks already released).
        let mut composer_focus_lost = false;
        if transition.gained.is_some() {
            self.state.pending_blur_delivery_context = None;
        }
        if let Some((ev, ns)) = &transition.lost {
            if let Some(node_id) = ev.node_id {
                self.state.pending_blur_delivery_context = Some(ComposerDeliveryContext {
                    namespace: ns.clone(),
                    node_id_bytes: *node_id.as_uuid().as_bytes(),
                    tile_id: ev.tile_id,
                });
            }
            composer_focus_lost = true;
        }
        tracing::debug!("portal recovery: focus redirected to composer");
        dispatch_focus_event(&self.state.input_event_tx, transition);
        if composer_focus_lost {
            self.clear_local_composer_echo();
        }
        TabFocusOutcome::Done
    }

    /// Escape recovery for a composer-less focus stop (hud-k6yvb): a focusable
    /// node whose tile has no composer, or a tile-level stop. Clears focus to
    /// `None` so a keyboard user is never stranded on a control they cannot type
    /// into; a subsequent Tab re-enters the cycle at the first stop.
    ///
    /// Returns [`EscapeClearOutcome::Cleared`] when it released a real owner,
    /// [`EscapeClearOutcome::NoOwner`] when there was nothing to clear (caller
    /// proceeds normally), or [`EscapeClearOutcome::Busy`] on lock contention
    /// (caller defers the event).
    fn try_escape_clear_focus(&mut self, tab_id: tze_hud_scene::SceneId) -> EscapeClearOutcome {
        let transition = {
            let Some(state) = spin_acquire(&self.state.shared_state, INTERACTION_LOCK_BUDGET)
            else {
                return EscapeClearOutcome::Busy;
            };
            let Some(scene) = spin_acquire(&state.scene, INTERACTION_LOCK_BUDGET) else {
                return EscapeClearOutcome::Busy;
            };
            // Nothing to clear when focus is already absent / on chrome.
            if matches!(
                self.state.focus_manager.current_owner(tab_id),
                tze_hud_input::FocusOwner::None | tze_hud_input::FocusOwner::ChromeElement(_)
            ) {
                return EscapeClearOutcome::NoOwner;
            }
            self.state.focus_manager.clear_focus(tab_id, &scene)
        };
        tracing::debug!("portal recovery: Escape cleared composer-less focus stop");
        dispatch_focus_event(&self.state.input_event_tx, transition);
        EscapeClearOutcome::Cleared
    }

    /// Activate a Tab-focused portal control by synthesizing a pointer
    /// down+up to the owning agent (hud-2v8br), so Enter / Space fire the same
    /// handler a pointer click does. The control's behavior (minimize / restore /
    /// submit) is agent-owned and keyed off `interaction_id`, so a synthetic
    /// press+release on the same node is the faithful keyboard analogue.
    fn activate_portal_control(&mut self, control: &PortalControlSnapshot) {
        let base = AgentDispatch {
            namespace: control.namespace.clone(),
            tile_id: control.tile_id,
            node_id: control.node_id,
            interaction_id: control.interaction_id.clone(),
            local_x: control.local_x,
            local_y: control.local_y,
            display_x: control.display_x,
            display_y: control.display_y,
            device_id: KEYBOARD_ACTIVATION_DEVICE_ID,
            kind: AgentDispatchKind::PointerDown,
            capture_released_reason: None,
        };
        tracing::debug!(
            interaction_id = %str_preview(&control.interaction_id),
            "portal control activated via keyboard (synthetic pointer down+up)"
        );
        dispatch_pointer_event(&self.state.input_event_tx, base.clone());
        dispatch_pointer_event(
            &self.state.input_event_tx,
            AgentDispatch {
                kind: AgentDispatchKind::PointerUp,
                ..base
            },
        );
    }

    /// Translate a raw key-up event through the `KeyboardProcessor`, log it,
    /// and broadcast it over the `INPUT_EVENTS` gRPC channel.
    ///
    /// Events are dropped silently when `current_owner` is `FocusOwner::None`.
    ///
    /// Safe-mode capture applies here as well: when safe mode is active, key-up
    /// events are dropped so agents never see a key-release for a key-down that
    /// was already captured by the chrome layer.
    pub(super) fn dispatch_key_up_event(&mut self, raw: &RawKeyUpEvent) {
        // ── Safe-mode capture ──────────────────────────────────────────────
        // Mirror the key-down safe-mode guard: if safe mode is active, chrome
        // owns ALL input including key-release events.
        // Lock-free read via the AtomicBool mirror — see `dispatch_key_down_event`
        // Priority 1 comment for the memory-ordering rationale.
        if self
            .state
            .safe_mode_atomic
            .load(std::sync::atomic::Ordering::Acquire)
        {
            tracing::debug!(
                key = %str_preview(&raw.key),
                "safe-mode capture: KeyUp dropped (safe mode active — chrome layer owns input)"
            );
            return;
        }

        // FIFO guard: if earlier events are still pending, queue this one
        // immediately so it cannot bypass them even when the lock is free.
        if !self.state.pending_keyboard_events.is_empty() {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::KeyUp(raw.clone()));
            return;
        }
        // Resolve the active tab via the lock-free mirror (hud-dwcr7).
        let Some(active_tab) = self.active_tab_for_keyboard_dispatch() else {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::KeyUp(raw.clone()));
            return;
        };
        self.dispatch_key_up_event_inner(raw, active_tab);
    }

    /// FIFO-ordered inner routing for a key-up event.  See
    /// [`Self::dispatch_key_down_event_inner`] for why this MUST NOT re-apply
    /// the FIFO guard (hud-dwcr7 livelock).
    pub(super) fn dispatch_key_up_event_inner(
        &mut self,
        raw: &RawKeyUpEvent,
        active_tab: Option<tze_hud_scene::SceneId>,
    ) {
        let Some(tab_id) = active_tab else { return };

        // ── Keyboard focus traversal: swallow the matching Tab KeyUp (hud-v0cal) ──
        //
        // A bare Tab / Shift+Tab KeyDown is unconditionally consumed by focus
        // traversal in `dispatch_key_down_event_inner` (never forwarded as raw
        // input).  Its matching KeyUp must therefore also be swallowed: otherwise
        // the agent that just gained focus would receive a raw Tab release with no
        // preceding KeyDown — an impossible key sequence that contradicts the
        // "Tab is never forwarded as raw input" contract.  The modifier guard
        // matches the KeyDown intercept exactly (Ctrl / Alt / Meta excluded), so a
        // shell-reserved Ctrl+Tab or OS Alt/Meta+Tab release still routes normally.
        if is_bare_tab_chord(&raw.key, &raw.modifiers) {
            tracing::debug!("tab focus traversal: matching KeyUp swallowed (KeyDown consumed)");
            return;
        }

        // ── Portal resize hotkey: KeyUp fallback (hud-v4k1h) ──────────────
        //
        // On live Windows, SendInput-driven Ctrl chords can deliver ONLY the
        // `KeyUp` for `=`/`-`/`+` (the `KeyDown` never arrives while Ctrl is
        // held), so a key-down-only resize intercept silently does nothing.
        // We therefore also resize on key-up.
        //
        // Two cases, in order:
        //   1. The matching `KeyDown` already resized → swallow this `KeyUp`
        //      (dedup) so a normal physical down/up pair resizes exactly once.
        //   2. No `KeyDown` was consumed → treat this `KeyUp` as the resize
        //      trigger (the release-only live stream).
        //
        // Direction resolution mirrors the key-down path: logical key first,
        // then the physical `KeyCode` fallback (PR #937) so the chord is
        // deterministic regardless of layout or held modifiers.
        let resize_key_code = portal_resize_key_code(&raw.key_code, &raw.key);
        if self
            .state
            .consumed_portal_resize_keydowns
            .remove(&resize_key_code)
        {
            tracing::debug!(
                key = %str_preview(&raw.key),
                key_code = %str_preview(&raw.key_code),
                "portal resize: matching KeyUp consumed after KeyDown resize"
            );
            return;
        }
        if let Some(dir) = HotkeyResizeDir::from_key(&raw.key, raw.modifiers.ctrl)
            .or_else(|| HotkeyResizeDir::from_key_code(&raw.key_code, raw.modifiers.ctrl))
            && self.apply_portal_resize_hotkey(tab_id, dir, HotkeyResizeAxis::Both)
        {
            tracing::debug!(
                key = %str_preview(&raw.key),
                key_code = %str_preview(&raw.key_code),
                "portal resize: Ctrl KeyUp fallback consumed (resize applied)"
            );
            return;
        }
        // Directional Ctrl+Shift+Arrow whole-portal resize: same KeyUp fallback as
        // the Ctrl+`+`/`-` chord (hud-csrmf), so a SendInput-driven release-only
        // stream still resizes. Gated on `!is_composer_active()` to mirror the
        // key-down intercept (never steal composer word/line-select).
        if !self.state.input_processor.is_composer_active()
            && let Some((dir, axis)) = directional_portal_resize(&raw.key_code, &raw.modifiers)
            && self.apply_portal_resize_hotkey(tab_id, dir, axis)
        {
            tracing::debug!(
                key_code = %str_preview(&raw.key_code),
                ?axis,
                "portal resize: Ctrl+Shift+Arrow directional KeyUp fallback consumed (resize applied)"
            );
            return;
        }

        let focus_owner = self.state.focus_manager.current_owner(tab_id).clone();

        let ns_lock_busy = std::cell::Cell::new(false);
        let namespace_fn = |tile_id: tze_hud_scene::SceneId| -> Option<String> {
            match self.namespace_for_keyboard_tile(tile_id) {
                None => {
                    ns_lock_busy.set(true);
                    None
                }
                Some(ns) => ns,
            }
        };
        if let Some(dispatch) =
            self.state
                .keyboard_processor
                .process_key_up(raw, &focus_owner, namespace_fn)
        {
            tracing::debug!(
                namespace = %str_preview(&dispatch.namespace),
                tile_id = ?dispatch.tile_id,
                node_id = ?dispatch.node_id,
                kind = %keyboard_kind_preview(&dispatch.kind),
                "keyboard: KeyUp dispatched to agent"
            );
            dispatch_keyboard_event(&self.state.input_event_tx, dispatch);
        } else if ns_lock_busy.get() {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::KeyUp(raw.clone()));
        }
    }

    /// Translate a raw post-IME character event through the `KeyboardProcessor`,
    /// log it, and broadcast it over the `INPUT_EVENTS` gRPC channel.
    ///
    /// Called both from `WindowEvent::Ime(Ime::Commit)` (IME path) and from
    /// `Key::Character` in `WindowEvent::KeyboardInput` (direct input path), as
    /// well as the paste-shortcut path (Ctrl+V clipboard text).
    ///
    /// Events are dropped silently when `current_owner` is `FocusOwner::None`.
    ///
    /// # Safe-mode capture
    ///
    /// When safe mode is active, character events (including paste and IME commits)
    /// are dropped so agents never receive character input while chrome owns input.
    ///
    /// # Composer interception (§4.1)
    ///
    /// When a composer region is focused, the character is routed into the
    /// `ComposerDraftManager` draft buffer instead of being forwarded to the
    /// agent as a raw `CharacterEvent`.  Only `EditOutcome::Unchanged` (no
    /// active composer) allows the normal dispatch path.
    pub(super) fn dispatch_character_event(&mut self, raw: &RawCharacterEvent) {
        // ── Safe-mode capture ──────────────────────────────────────────────
        // All character input (Key::Character, paste shortcut, IME commits) is
        // captured by the chrome layer when safe mode is active.
        // Lock-free read via the AtomicBool mirror — see `dispatch_key_down_event`
        // Priority 1 comment for the memory-ordering rationale.
        if self
            .state
            .safe_mode_atomic
            .load(std::sync::atomic::Ordering::Acquire)
        {
            tracing::debug!(
                "safe-mode capture: CharacterEvent dropped (safe mode active — chrome layer owns input)"
            );
            return;
        }

        // FIFO guard: if earlier events are still pending, queue this one
        // immediately so it cannot bypass them even when the lock is free.
        if !self.state.pending_keyboard_events.is_empty() {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::Character(raw.clone()));
            return;
        }
        // Resolve the active tab via the lock-free mirror (hud-dwcr7).
        let Some(active_tab) = self.active_tab_for_keyboard_dispatch() else {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::Character(raw.clone()));
            return;
        };
        self.dispatch_character_event_inner(raw, active_tab);
    }

    /// FIFO-ordered inner routing for a character event.  See
    /// [`Self::dispatch_key_down_event_inner`] for why this MUST NOT re-apply
    /// the FIFO guard (hud-dwcr7 livelock).
    pub(super) fn dispatch_character_event_inner(
        &mut self,
        raw: &RawCharacterEvent,
        active_tab: Option<tze_hud_scene::SceneId>,
    ) {
        let Some(tab_id) = active_tab else { return };

        // ── Portal control typing recovery (hud-2v8br) ──────────────────────
        //
        // A printable character typed while a non-composer portal control holds
        // focus recovers to the composer so the keystroke lands in the input box
        // instead of being dispatched to the agent as a raw CharacterEvent. After
        // the redirect the composer is active, so the intercept below inserts the
        // character. Whitespace-only characters (e.g. the Space that just
        // activated a control) are consumed without typing, matching the
        // Enter/Space activation semantics in dispatch_key_down_event_inner.
        if !self.state.input_processor.is_composer_active() {
            match self.portal_key_focus(tab_id) {
                PortalKeyFocus::Busy => {
                    self.state
                        .pending_keyboard_events
                        .push_back(PendingKeyboardEvent::Character(raw.clone()));
                    return;
                }
                PortalKeyFocus::Control(control) => {
                    if raw.character.trim().is_empty() {
                        // Space/whitespace on a control is an activation key, not
                        // text — consume it so it neither types nor leaks.
                        return;
                    }
                    if self.refocus_composer_from_control(
                        tab_id,
                        control.tile_id,
                        control.composer_node,
                    ) == TabFocusOutcome::Busy
                    {
                        self.state
                            .pending_keyboard_events
                            .push_back(PendingKeyboardEvent::Character(raw.clone()));
                        return;
                    }
                    // Composer now active — fall through to the intercept below,
                    // which inserts raw.character into the draft.
                }
                PortalKeyFocus::Composer | PortalKeyFocus::Other => {}
            }
        }

        // ── Composer draft intercept (§4.4) ──────────────────────────────
        //
        // Spec §4.4: NO character input reaches the agent while a composer
        // region is focused — regardless of what route_character_to_composer
        // returns.  In particular, when the clipboard text is ENTIRELY control
        // characters, route_character_to_composer sanitises it to an empty
        // string and paste("") returns EditOutcome::Unchanged (nothing was
        // mutated in the draft).  Without an unconditional early-return here,
        // the Unchanged case would fall through to the agent dispatch path
        // below, leaking input to the agent while the composer is focused
        // (hud-60hgf).
        if self.state.input_processor.is_composer_active() {
            let delivery_context = match self.composer_delivery_context_for_tab(tab_id) {
                ComposerDeliveryContextLookup::Ready(context) => Some(context),
                ComposerDeliveryContextLookup::Busy => {
                    self.state
                        .pending_keyboard_events
                        .push_back(PendingKeyboardEvent::Character(raw.clone()));
                    return;
                }
                ComposerDeliveryContextLookup::Unavailable => None,
            };
            // Captured before `delivery_context` is moved into
            // `route_and_deliver_composer_batch` below (hud-qbcp8: input-
            // history reset-to-tail needs the tile id after that move).
            let history_tile_id = delivery_context.as_ref().map(|c| c.tile_id);
            // Capture the input-started-at instant for local-ack latency
            // measurement (hud-r3ax6 / hud-o9ybl).
            let composer_input_started = Instant::now();
            let (outcome, batch) = self
                .state
                .input_processor
                .route_character_to_composer(&raw.character);
            // Track whether this character event submitted or cancelled the composer
            // so we can suppress the push below (clear must win; hud-r3ax6).
            let mut char_is_terminal = false;
            if let Some(b) = batch {
                let is_submission = b.submission.is_some();
                char_is_terminal = b.cancel.is_some() || is_submission;
                if char_is_terminal {
                    // Submit or cancel: clear the local echo overlay.
                    self.clear_local_composer_echo();
                }
                if let Some(context) = delivery_context {
                    self.route_and_deliver_composer_batch(context, b);
                }
                // hud-npcdf: mirror the KeyDown submit-terminal reset so a
                // submission arriving on the character path (e.g. an
                // authority-attached tile) also snaps the input-pane history back
                // to the tail — the async authority echo honors `ScrolledBack` and
                // would otherwise strand the just-submitted reply off-screen.
                // Idempotent alongside the raw-tile reset in
                // `append_raw_tile_viewer_echo`.
                if is_submission {
                    if let Some(tile_id) = history_tile_id {
                        self.reset_input_history_scroll_to_tail(tile_id);
                    }
                }
            }
            // Truncate for debug logs: raw.character carries clipboard text and
            // can be arbitrarily large.  Formatting is lazy (tracing skips it
            // below info in production), but defensive truncation avoids
            // surprises in debug builds with large paste payloads.
            let char_log_preview = str_preview(&raw.character);
            if outcome != tze_hud_input::EditOutcome::Unchanged {
                tracing::debug!(
                    character = %char_log_preview,
                    outcome = ?outcome,
                    "composer: Character consumed by draft manager"
                );
                // Push the updated draft snapshot for local echo rendering
                // (hud-r3ax6).  This is the Stage 2 "local feedback" path;
                // no adapter round-trip.
                // Guard: do NOT push after a terminal batch — clear must win.
                if !char_is_terminal {
                    self.push_local_composer_echo(composer_input_started);
                    // hud-qbcp8: typing should not leave the input-pane history
                    // scrolled away from the viewer's own live draft.
                    if let Some(tile_id) = history_tile_id {
                        self.reset_input_history_scroll_to_tail(tile_id);
                    }
                }
            } else {
                // EditOutcome::Unchanged: the draft was not mutated (e.g. the
                // clipboard contained only control characters that sanitised to
                // empty, or the paste arrived while the composer was at
                // capacity and already at its limit).  No echo push needed.
                // Unconditional early-return below ensures the event still
                // never reaches the agent path (§4.4).
                tracing::debug!(
                    character = %char_log_preview,
                    "composer: Character absorbed (Unchanged — all-control or no-op paste); not forwarded to agent (§4.4)"
                );
            }
            // §4.4 hard gate: the composer is active, so we MUST NOT fall
            // through to the agent dispatch path below under any outcome.
            return;
        }

        let focus_owner = self.state.focus_manager.current_owner(tab_id).clone();

        let ns_lock_busy = std::cell::Cell::new(false);
        let namespace_fn = |tile_id: tze_hud_scene::SceneId| -> Option<String> {
            match self.namespace_for_keyboard_tile(tile_id) {
                None => {
                    ns_lock_busy.set(true);
                    None
                }
                Some(ns) => ns,
            }
        };
        if let Some(dispatch) =
            self.state
                .keyboard_processor
                .process_character(raw, &focus_owner, namespace_fn)
        {
            tracing::debug!(
                namespace = %str_preview(&dispatch.namespace),
                tile_id = ?dispatch.tile_id,
                node_id = ?dispatch.node_id,
                kind = %keyboard_kind_preview(&dispatch.kind),
                "keyboard: Character dispatched to agent"
            );
            dispatch_keyboard_event(&self.state.input_event_tx, dispatch);
        } else if ns_lock_busy.get() {
            self.state
                .pending_keyboard_events
                .push_back(PendingKeyboardEvent::Character(raw.clone()));
        }
    }

    /// Flush coalesced composer draft notifications at the frame settle point.
    ///
    /// Should be called once per frame / per settle window after all key events
    /// for the current batch have been drained.  Guarantees the terminal draft
    /// state is delivered even when keystrokes arrived in a burst (spec §4.3
    /// flush guarantee).
    ///
    /// The `DraftNotificationBatch` returned by the input processor is encoded
    /// as proto messages and broadcast on the `INPUT_EVENTS` channel to the
    /// owning adapter namespace.
    pub(super) fn flush_composer_draft_at_settle(&mut self) {
        // Resolve delivery context.  Two cases:
        //
        // 1. Normal path (keystroke / timer settle): the composer node is still
        //    focused, so composer_delivery_context() resolves namespace + node_id
        //    from the live focus state.
        //
        // 2. Blur path: a focus-lost transition happened earlier this frame
        //    (process_with_focus cleared focused_node and stored the terminal
        //    batch in pending_flushed_batch).  composer_delivery_context()
        //    returns None because focused_node is None.  We fall back to
        //    pending_blur_delivery_context which was captured at blur time and
        //    consume it here so it is not reused across frames.
        //
        // This two-path resolution upholds the §4.3 flush guarantee on blur.
        let ctx = match self.composer_delivery_context() {
            ComposerDeliveryContextLookup::Ready(context) => Some(context),
            ComposerDeliveryContextLookup::Busy => {
                match self.state.pending_blur_delivery_context.take() {
                    Some(context) => Some(context),
                    None => return,
                }
            }
            ComposerDeliveryContextLookup::Unavailable => {
                self.state.pending_blur_delivery_context.take()
            }
        };
        if let Some(batch) = self.state.input_processor.try_flush_composer_draft() {
            if let Some(context) = ctx {
                self.route_and_deliver_composer_batch(context, batch);
            }
        }
    }

    /// Retry keyboard events that were deferred in the previous iteration(s)
    /// because the active-tab mirror was momentarily busy (hud-2fz34).
    ///
    /// Called from `about_to_wait` once per event-loop iteration, matching the
    /// `drain_input_capture_commands` sibling pattern.  Each event is popped
    /// from the front of `pending_keyboard_events` and routed through the
    /// **inner** dispatch fns (`dispatch_*_event_inner`), NOT the public
    /// Stage-1 entry.
    ///
    /// This bypass is the fix for the hud-dwcr7 livelock: the public entry
    /// re-queues any event when the queue is non-empty (the FIFO guard).  If the
    /// drain called the public entry, a freshly-popped event would see the
    /// remaining queued events and immediately re-queue itself to the back —
    /// the queue would rotate front→back forever and never shrink, freezing
    /// composer echo.  The inner fns skip the FIFO guard, so the drain (which is
    /// itself the FIFO-ordered consumer) actually consumes each event.
    ///
    /// FIFO guarantee: the active tab is resolved once per pop.  If the mirror
    /// is momentarily busy, the entire drain stops immediately — no later event
    /// is allowed to skip ahead of an earlier one.
    ///
    /// Ordering: called after `flush_composer_draft_at_settle` and before
    /// `drain_portal_projection` so deferred keystrokes are retried before
    /// portal-projection geometry is refreshed (no observable ordering
    /// difference under normal operation, but consistent with event-arrival
    /// order).
    pub(super) fn drain_pending_keyboard_events(&mut self) {
        // Drain at most the number of events that were pending at entry so we
        // don't loop forever if a genuine lock-busy defer re-grows the queue
        // (e.g. the agent-routing namespace try_lock inside an inner fn).
        // The bound lives inside `drain_keyboard_queue_bounded` (hud-b09ag).
        let limit = self.state.pending_keyboard_events.len();
        drain_keyboard_queue_bounded(limit, || {
            // Resolve the active tab before popping: if the mirror is busy, stop
            // draining entirely to preserve strict FIFO order.  A later event
            // must not be dispatched before an earlier one that is still blocked.
            let Some(active_tab) = self.active_tab_for_keyboard_dispatch() else {
                return ControlFlow::Break(());
            };
            let Some(event) = self.state.pending_keyboard_events.pop_front() else {
                return ControlFlow::Break(());
            };
            let len_after_pop = self.state.pending_keyboard_events.len();
            // Route through the inner fns (no FIFO guard) — see the doc comment.
            // Calling the public entry here would re-queue the just-popped event
            // (the remaining queue is non-empty), rotating front→back forever
            // (the hud-dwcr7 livelock).
            match event {
                PendingKeyboardEvent::KeyDown(raw) => {
                    self.dispatch_key_down_event_inner(&raw, active_tab)
                }
                PendingKeyboardEvent::KeyUp(raw) => {
                    self.dispatch_key_up_event_inner(&raw, active_tab)
                }
                PendingKeyboardEvent::Character(raw) => {
                    self.dispatch_character_event_inner(&raw, active_tab)
                }
            }
            // Inner dispatch can still defer the popped event if a required
            // namespace/composer delivery-context lookup is busy. Those paths
            // append to the tail; move that retry back to the front and stop so
            // later events cannot overtake it.
            if restore_front_requeued_event(&mut self.state.pending_keyboard_events, len_after_pop)
            {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
    }

    /// Read the active tab without blocking the event-loop thread on the scene
    /// mutex (hud-2fz34, hud-dwcr7).
    ///
    /// Returns:
    /// - `None` — the (tiny, dedicated) mirror lock was momentarily contended;
    ///   caller may defer to the next iteration.
    /// - `Some(None)` — resolved; no active tab.
    /// - `Some(Some(id))` — resolved; active tab found.
    ///
    /// This reads `SharedState.active_tab_mirror` — a lock-free-by-design
    /// `std::sync::Mutex<Option<SceneId>>` that is updated whenever a writer
    /// holding the scene changes `active_tab` (gRPC mutation apply, event-loop
    /// tab switch).  It deliberately does NOT `try_lock` the scene Tokio mutex:
    /// under sustained gRPC portal streaming that lock is held across mutation
    /// batches, so the old try_lock kept failing and every composer keystroke
    /// deferred — freezing the local echo and violating the "Local feedback
    /// first" doctrine + the 4 ms input-to-local-ack budget.  Composer
    /// keystroke echo (the composer intercept in `dispatch_key_down_event`)
    /// depends only on the lock-free `InputProcessor` plus this `tab_id`, so
    /// removing the scene-lock dependency here fully unblocks it.
    ///
    /// A one-frame lag on the mirror across a tab switch is acceptable: the
    /// scene remains the source of truth and the mirror reconverges on the next
    /// refresh.
    pub(super) fn active_tab_for_keyboard_dispatch(
        &self,
    ) -> Option<Option<tze_hud_scene::SceneId>> {
        match self.state.active_tab_mirror.try_lock() {
            Ok(guard) => Some(*guard),
            Err(std::sync::TryLockError::WouldBlock) => {
                tracing::trace!("keyboard dispatch deferred: active_tab mirror busy");
                None
            }
            // A poisoned mirror still yields a valid Copy value — recover it.
            Err(std::sync::TryLockError::Poisoned(p)) => Some(*p.into_inner()),
        }
    }

    /// Try to read the agent namespace for a keyboard tile without blocking the
    /// event-loop thread (hud-2fz34).
    ///
    /// Returns:
    /// - `None` — shared-state or scene lock is busy; caller must defer to the
    ///   next iteration.
    /// - `Some(None)` — lock acquired; tile not found in scene.
    /// - `Some(Some(ns))` — lock acquired; namespace resolved.
    ///
    /// See `active_tab_for_keyboard_dispatch` for the rationale.
    pub(super) fn namespace_for_keyboard_tile(
        &self,
        tile_id: tze_hud_scene::SceneId,
    ) -> Option<Option<String>> {
        let Ok(state) = self.state.shared_state.try_lock() else {
            tracing::trace!("keyboard dispatch deferred: shared_state lock busy");
            return None;
        };
        let Ok(scene) = state.scene.try_lock() else {
            tracing::trace!("keyboard dispatch deferred: scene lock busy");
            return None;
        };
        Some(scene.tiles.get(&tile_id).map(|tile| tile.namespace.clone()))
    }

    /// Resolve the delivery context for the currently focused composer region.
    ///
    /// `Busy` means a required lock was momentarily unavailable; callers must
    /// retry later without consuming the draft event.
    ///
    /// Used by `dispatch_key_down_event`, `dispatch_character_event`, and
    /// `flush_composer_draft_at_settle` to supply the delivery context to
    /// `deliver_composer_batch` and the projection-authority input bridge.
    /// Resolve the scene `tile_id` owning the currently focused composer,
    /// without resolving the agent namespace (hud-sq2ss).
    ///
    /// Mirrors the `composer_focused_node()` + `focus_manager` lookup half of
    /// [`Self::composer_delivery_context_for_tab`], but skips the
    /// `namespace_for_keyboard_tile` scene lock that lookup also does — callers
    /// that only need the tile id for a reset-to-tail (not for routing a batch
    /// to an agent) should use this instead of paying for a namespace resolve
    /// they will not use.
    ///
    /// Returns `None` if no composer is focused, there is no active tab, or the
    /// focus owner is not a tile (all "nothing to reset" cases, not errors).
    pub(super) fn composer_focused_tile_id(&self) -> Option<tze_hud_scene::SceneId> {
        self.state.input_processor.composer_focused_node()?;
        let tab_id = self.active_tab_for_keyboard_dispatch().flatten()?;
        self.state.focus_manager.current_owner(tab_id).tile_id()
    }

    pub(super) fn composer_delivery_context(&self) -> ComposerDeliveryContextLookup {
        match self.active_tab_for_keyboard_dispatch() {
            Some(Some(tab_id)) => self.composer_delivery_context_for_tab(tab_id),
            Some(None) => ComposerDeliveryContextLookup::Unavailable,
            None => ComposerDeliveryContextLookup::Busy,
        }
    }

    pub(super) fn composer_delivery_context_for_tab(
        &self,
        tab_id: tze_hud_scene::SceneId,
    ) -> ComposerDeliveryContextLookup {
        let Some(node_id) = self.state.input_processor.composer_focused_node() else {
            return ComposerDeliveryContextLookup::Unavailable;
        };
        let node_id_bytes = *node_id.as_uuid().as_bytes();

        // The focus manager's active tab holds the authoritative FocusOwner.
        // For a composer region the owner is FocusOwner::Node { tile_id, .. };
        // from the tile_id we can look up the agent namespace.
        let Some(tile_id) = self.state.focus_manager.current_owner(tab_id).tile_id() else {
            return ComposerDeliveryContextLookup::Unavailable;
        };
        match self.namespace_for_keyboard_tile(tile_id) {
            Some(Some(namespace)) => {
                ComposerDeliveryContextLookup::Ready(ComposerDeliveryContext {
                    namespace,
                    node_id_bytes,
                    tile_id,
                })
            }
            Some(None) => ComposerDeliveryContextLookup::Unavailable,
            None => ComposerDeliveryContextLookup::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::ops::ControlFlow;

    use tze_hud_input::{KeyboardModifiers, RawKeyDownEvent};
    use tze_hud_scene::MonoUs;

    use super::{
        PendingKeyboardEvent, drain_keyboard_queue_bounded, is_bare_tab_chord,
        restore_front_requeued_event,
    };

    /// hud-v0cal: the bare Tab / Shift+Tab focus-traversal chord is recognised,
    /// and the modifier-laden Tab variants (shell-reserved Ctrl+Tab, OS Alt/Meta
    /// window switchers) are NOT — for BOTH the key-down intercept and the key-up
    /// swallow, which share this predicate.
    ///
    /// This guards the key-up contract: because a bare Tab key-down is always
    /// consumed for traversal (never forwarded), `dispatch_key_up_event_inner`
    /// must early-return on the same predicate so the matching bare Tab key-UP is
    /// never delivered to the composer or agent as a raw release.  A regression
    /// that narrowed/loosened either guard would diverge them and surface here.
    #[test]
    fn bare_tab_chord_predicate_matches_keydown_and_keyup_intercepts() {
        let bare = KeyboardModifiers::NONE;
        let shift = KeyboardModifiers {
            shift: true,
            ..KeyboardModifiers::NONE
        };
        let ctrl = KeyboardModifiers {
            ctrl: true,
            ..KeyboardModifiers::NONE
        };
        let ctrl_shift = KeyboardModifiers {
            ctrl: true,
            shift: true,
            ..KeyboardModifiers::NONE
        };
        let alt = KeyboardModifiers {
            alt: true,
            ..KeyboardModifiers::NONE
        };
        let meta = KeyboardModifiers {
            meta: true,
            ..KeyboardModifiers::NONE
        };

        // Bare Tab and Shift+Tab ARE focus traversal → consumed on key-down and
        // swallowed on key-up (never delivered to composer/agent).
        assert!(is_bare_tab_chord("Tab", &bare), "Tab → traversal");
        assert!(
            is_bare_tab_chord("Tab", &shift),
            "Shift+Tab → reverse traversal"
        );

        // Modifier-laden Tab variants are NOT traversal → routed normally
        // (Ctrl+Tab/Ctrl+Shift+Tab shell-reserved; Alt/Meta+Tab OS switchers).
        assert!(
            !is_bare_tab_chord("Tab", &ctrl),
            "Ctrl+Tab is shell-reserved"
        );
        assert!(
            !is_bare_tab_chord("Tab", &ctrl_shift),
            "Ctrl+Shift+Tab is shell-reserved"
        );
        assert!(
            !is_bare_tab_chord("Tab", &alt),
            "Alt+Tab is the OS switcher"
        );
        assert!(
            !is_bare_tab_chord("Tab", &meta),
            "Win/Cmd+Tab is the OS switcher"
        );

        // Non-Tab keys are never affected by the traversal guards.
        assert!(!is_bare_tab_chord("a", &bare), "plain character is not Tab");
        assert!(!is_bare_tab_chord("Enter", &bare), "Enter is not Tab");
    }

    fn key_down(key: &str, timestamp_mono_us: u64) -> PendingKeyboardEvent {
        PendingKeyboardEvent::KeyDown(RawKeyDownEvent {
            key_code: format!("Key{}", key.to_ascii_uppercase()),
            key: key.to_string(),
            modifiers: KeyboardModifiers::NONE,
            repeat: false,
            timestamp_mono_us: MonoUs(timestamp_mono_us),
        })
    }

    fn assert_key_down(event: &PendingKeyboardEvent, expected: &str) {
        match event {
            PendingKeyboardEvent::KeyDown(raw) => assert_eq!(
                raw.key, expected,
                "pending keyboard event order must preserve FIFO"
            ),
            other => panic!("expected KeyDown({expected:?}), got {other:?}"),
        }
    }

    #[test]
    fn restore_front_requeued_event_keeps_blocked_event_ahead_of_later_events() {
        let mut queue: VecDeque<PendingKeyboardEvent> =
            [key_down("b", 2_000), key_down("c", 3_000)]
                .into_iter()
                .collect();
        let len_after_pop = queue.len();

        // Simulate the real drain path after "a" was popped from the front and
        // the inner dispatch had to retry because a required delivery-context or
        // namespace lookup was busy. The inner dispatch appends the same event to
        // the tail; the drain must restore it to the front before returning.
        queue.push_back(key_down("a", 1_000));

        assert!(
            restore_front_requeued_event(&mut queue, len_after_pop),
            "helper must detect that the popped event was requeued"
        );
        assert_eq!(queue.len(), 3, "restoration must not drop any event");
        assert_key_down(&queue[0], "a");
        assert_key_down(&queue[1], "b");
        assert_key_down(&queue[2], "c");
    }

    // ── Regression guards for the hud-dwcr7 kbd-livelock dispatch-storm ─────────
    //
    // The storm (docs/evidence/text-stream-portals/kbd-livelock-20260617-223504.log)
    // was caused by `drain_pending_keyboard_events` calling the public Stage-1 dispatch
    // functions. Those re-queued any event to the back when `pending_keyboard_events`
    // was non-empty (the FIFO guard), rotating front→back forever: the queue never
    // shrank; composer echo froze; the event-loop thread spun indefinitely.
    //
    // The fix extracts the bounded loop into `drain_keyboard_queue_bounded` (hud-dwcr7).
    // The three tests below exercise the extracted helper directly so each invariant is
    // independently guarded. Cross-linked: hud-dwcr7 (fix, closed), hud-b09ag (guard).

    /// AC #2 guard: `drain_keyboard_queue_bounded` must stop after exactly `limit`
    /// iterations even when new events arrive during the drain.
    ///
    /// Scenario: 4 events are queued.  Each dispatch iteration pops one event AND
    /// pushes a new "concurrent arrival" (simulating the OS event path or an inner
    /// dispatch re-enqueue racing with the drain).  With the `for _ in 0..limit`
    /// bound the drain stops after 4 iterations; the 4 new arrivals remain queued
    /// for the next `about_to_wait` cycle.
    ///
    /// **This test fails if the bound is removed** (e.g. changed to `loop` or
    /// `while !queue.is_empty()`): without the bound the drain processes the 4 new
    /// events too, making `iters ≠ 4` and `queue.len() ≠ 4`.
    ///
    /// AC #2 verified manually: temporarily changed `for _ in 0..limit` to `loop`
    /// in `drain_keyboard_queue_bounded`; the test hit the `pop_front() == None`
    /// branch at iteration 9 (assertion `iters == 4` failed with iters=9, and
    /// `queue.len() == 4` failed with queue.len()=0).  Restored the bound: test
    /// passes (iters=4, queue.len()=4).
    ///
    /// Cross-linked: hud-dwcr7 (fix), hud-b09ag (guard).
    #[test]
    fn drain_bounded_helper_stops_at_initial_limit_when_new_events_arrive_during_drain() {
        let initial_events: usize = 4;
        let mut queue: VecDeque<PendingKeyboardEvent> = (0..initial_events)
            .map(|i| key_down(["a", "b", "c", "d"][i], (i as u64 + 1) * 1_000))
            .collect();

        let mut iters = 0usize;
        let mut arrivals = 0usize;
        let limit = queue.len();

        drain_keyboard_queue_bounded(limit, || {
            iters += 1;
            let Some(_event) = queue.pop_front() else {
                // Queue unexpectedly empty — only reachable if the bound was removed
                // and the drain ran past the initial events.
                return ControlFlow::Break(());
            };
            // Simulate a concurrent OS-event arrival while the drain is running.
            // Without the `0..limit` bound the drain processes these too, looping
            // until arrivals is exhausted (2×initial_events total iterations).
            if arrivals < initial_events {
                queue.push_back(key_down("x", arrivals as u64 * 9_000));
                arrivals += 1;
            }
            ControlFlow::Continue(())
        });

        // With the `0..limit` bound: exactly `initial_events` iterations.
        // Without the bound: would be 2×initial_events (drains originals + arrivals).
        assert_eq!(
            iters, initial_events,
            "drain must stop after {initial_events} iterations (the initial queue \
             length); got {iters} — bound may have been removed"
        );
        // Newly-arrived events must still be queued (deferred to next cycle).
        assert_eq!(
            queue.len(),
            initial_events,
            "newly-arrived events must be deferred; queue.len()={} (expected \
             {initial_events})",
            queue.len()
        );
    }

    /// Guards the `restore_front_requeued_event` break inside `drain_keyboard_queue_bounded`.
    ///
    /// When inner dispatch defers an event (lock-busy) it pushes to the tail.
    /// `restore_front_requeued_event` detects this (queue grew) and the closure returns
    /// `Break`, stopping the drain after exactly 1 iteration with FIFO order intact.
    ///
    /// Cross-linked: hud-dwcr7 (fix), hud-b09ag (guard).
    #[test]
    fn drain_bounded_helper_re_queue_path_breaks_immediately_and_preserves_fifo() {
        let mut queue: VecDeque<PendingKeyboardEvent> = [
            key_down("a", 1_000),
            key_down("b", 2_000),
            key_down("c", 3_000),
        ]
        .into_iter()
        .collect();

        let mut iters = 0usize;
        let limit = queue.len();

        drain_keyboard_queue_bounded(limit, || {
            iters += 1;
            let event = queue
                .pop_front()
                .expect("queue must not be empty within limit");
            let len_after_pop = queue.len();
            // Simulate inner dispatch hitting a lock-busy condition → defers to back.
            queue.push_back(event);
            if restore_front_requeued_event(&mut queue, len_after_pop) {
                // Re-queue detected; "a" is back at front; caller stops this drain.
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });

        // Must stop after exactly 1 iteration (re-queue detected → Break).
        assert_eq!(
            iters, 1,
            "drain must break immediately on first re-queue; \
             spinning {limit} iterations without breaking is the rotation livelock"
        );
        // Queue integrity: all 3 events preserved, none dropped or duplicated.
        assert_eq!(queue.len(), 3, "re-queue must not drop or multiply events");
        // FIFO order: the originally-first event ("a") is back at the front.
        assert_key_down(&queue[0], "a");
        assert_key_down(&queue[1], "b");
        assert_key_down(&queue[2], "c");
    }

    /// Sanity / happy-path: all events dispatch successfully → queue drains to zero.
    ///
    /// Cross-linked: hud-dwcr7 (fix), hud-b09ag (guard).
    #[test]
    fn drain_bounded_helper_full_success_path_drains_queue_to_zero() {
        let mut queue: VecDeque<PendingKeyboardEvent> = [
            key_down("a", 1_000),
            key_down("b", 2_000),
            key_down("c", 3_000),
        ]
        .into_iter()
        .collect();

        let limit = queue.len();
        drain_keyboard_queue_bounded(limit, || {
            let _ = queue
                .pop_front()
                .expect("queue must not be empty within limit");
            // Inner dispatch succeeds: nothing pushed to back.
            ControlFlow::Continue(())
        });

        assert_eq!(
            queue.len(),
            0,
            "all events must be consumed when dispatch always succeeds"
        );
    }
}
