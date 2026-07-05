use std::time::Instant;

use tze_hud_compositor::LocalComposerState;
use tze_hud_scene::types::{DragHandleContextMenuState, WidgetParameterValue};

use super::hittest::combined_overlay_hit_regions;
use super::input_dispatch::nanoseconds_since_start;
use super::lifecycle::record_pending_input_latency;
use super::{HitRegion, WinitApp};
use crate::widget_hover::{
    WidgetHoverTracker, build_hover_trackers, hidden_mutations_for_removed, tick_hover_trackers,
};

/// Default empty-draft placeholder hint used when a composer's owning
/// `HitRegionNode` has no `composer_placeholder` override configured
/// (hud-evk0j).
///
/// The portal composer is a chat-message composer (`portal-bottom-chat-composer`),
/// so this matches the existing UX. Rendered — dimmed, from
/// `portal.composer.placeholder_color` — only while the draft is empty; it is
/// never part of the draft buffer and is never submitted.
///
/// Per-composer override (hud-se6hs, follow-up to hud-evk0j): a
/// `HitRegionNode`'s `composer_placeholder` config is threaded through
/// `composer_draft_snapshot` and resolved below — `None` inherits this
/// default, `Some("")` opts the composer out of any placeholder, and
/// `Some(text)` supplies non-chat composers with their own hint copy.
const COMPOSER_DEFAULT_PLACEHOLDER: &str = "Type a message…";

/// Resolve a composer's placeholder hint from its `HitRegionNode` override
/// (as reported by `composer_draft_snapshot`), falling back to
/// [`COMPOSER_DEFAULT_PLACEHOLDER`] when unset.
///
/// Mirrors `HitRegionNode::composer_placeholder`'s three-state convention:
/// `None` (unset) → the global default; `Some("")` (explicit opt-out) →
/// `None` (no placeholder rendered); `Some(text)` (custom) → `Some(text)`.
fn resolve_composer_placeholder(override_config: Option<String>) -> Option<String> {
    match override_config {
        None => Some(COMPOSER_DEFAULT_PLACEHOLDER.to_string()),
        Some(text) if text.is_empty() => None,
        Some(text) => Some(text),
    }
}

impl WinitApp {
    /// Push the current composer draft snapshot to the compositor thread for
    /// local echo rendering (hud-r3ax6).
    ///
    /// Called immediately after every keystroke that mutates the draft buffer.
    /// The snapshot is written to the shared `local_composer_state` slot;
    /// the compositor thread drains it once per frame (no round-trip).
    ///
    /// Also records an input-to-local-ack latency sample so the p99 measurement
    /// required by hud-o9ybl can be produced for the composer path.
    ///
    /// No-ops if the draft manager reports no active draft (safety guard).
    pub(super) fn push_local_composer_echo(&mut self, input_started_at: std::time::Instant) {
        if let Some((text, cursor_byte, selection_anchor, at_capacity, node_id, placeholder_cfg)) =
            self.state.input_processor.composer_draft_snapshot()
        {
            let state = LocalComposerState {
                text,
                cursor_byte,
                selection_anchor,
                at_capacity,
                node_id,
                placeholder: resolve_composer_placeholder(placeholder_cfg),
            };
            if let Ok(mut guard) = self.state.local_composer_state.lock() {
                *guard = Some(Some(state));
            }
            // Measure input-to-local-ack: the "local ack" for a composer
            // keystroke is the moment the snapshot is handed to the compositor.
            // This is the equivalent of `result.local_ack_us` for pointer events.
            let local_ack_us = input_started_at.elapsed().as_micros() as u64;
            record_pending_input_latency(
                &self.state.pending_input_latency,
                input_started_at,
                local_ack_us,
            );
            // Request a redraw so the compositor picks up the new state promptly.
            if let Some(window) = &self.state.window {
                window.request_redraw();
            }
        }
    }

    /// Clear the local composer echo (called on blur, submit, cancel).
    ///
    /// Stores `Some(None)` (explicit deactivation) in the shared slot so the
    /// compositor clears the overlay on the next frame.
    pub(super) fn clear_local_composer_echo(&mut self) {
        if let Ok(mut guard) = self.state.local_composer_state.lock() {
            *guard = Some(None);
        }
        if let Some(window) = &self.state.window {
            window.request_redraw();
        }
    }

    /// Rebuild runtime-managed widget hover trackers and refresh overlay hit-regions.
    pub(super) fn refresh_widget_hover_tracking(&mut self) {
        let (surf_w, surf_h) = if let Some(window) = &self.state.window {
            let size = window.inner_size();
            (size.width as f32, size.height as f32)
        } else {
            (
                self.state.config.window.width as f32,
                self.state.config.window.height as f32,
            )
        };

        let mut next_trackers: Option<std::collections::HashMap<String, WidgetHoverTracker>> = None;
        let mut dynamic_hit_regions: Option<Vec<HitRegion>> = None;
        let mut removed_mutations = Vec::new();
        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(scene) = state.scene.try_lock() {
                let next =
                    build_hover_trackers(&scene, surf_w, surf_h, &self.state.widget_hover_trackers);
                removed_mutations =
                    hidden_mutations_for_removed(&self.state.widget_hover_trackers, &next);
                dynamic_hit_regions = Some(combined_overlay_hit_regions(
                    &self.state.static_hit_regions,
                    &scene,
                ));
                next_trackers = Some(next);
            }
        }
        if let Some(next) = next_trackers {
            self.state.widget_hover_trackers = next;
        }
        self.apply_widget_hover_mutations(removed_mutations);

        // Pointer capture includes explicit static regions plus compositor-managed
        // zone interaction regions (notification dismiss/action affordances).
        //
        // If the scene lock is briefly unavailable, keep the last known dynamic
        // regions. Dropping to the usually-empty static set makes overlay
        // hit-testing flicker to passthrough during mutation bursts.
        if let Some(dynamic_hit_regions) = dynamic_hit_regions {
            self.state.hit_regions = dynamic_hit_regions;
        } else if self.state.hit_regions.is_empty() {
            self.state.hit_regions = self.state.static_hit_regions.clone();
        }
    }

    /// Tick widget hover trackers and apply local parameter mutations.
    pub(super) fn tick_widget_hover_tracking(&mut self) {
        if self.state.widget_hover_trackers.is_empty() {
            return;
        }
        let mutations = tick_hover_trackers(
            &mut self.state.widget_hover_trackers,
            self.state.cursor_x,
            self.state.cursor_y,
            Instant::now(),
        );
        self.apply_widget_hover_mutations(mutations);
    }

    /// Apply runtime-local hover mutations to widget instance params.
    fn apply_widget_hover_mutations(
        &mut self,
        mutations: Vec<crate::widget_hover::WidgetHoverMutation>,
    ) {
        if mutations.is_empty() {
            return;
        }

        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                for mutation in mutations {
                    if let Err(e) = scene.set_widget_param_local(
                        &mutation.instance_name,
                        &mutation.param_name,
                        WidgetParameterValue::F32(mutation.value),
                    ) {
                        tracing::debug!(
                            error = %e,
                            widget = %mutation.instance_name,
                            param = %mutation.param_name,
                            "widget hover: failed to apply local hover mutation"
                        );
                    }
                }
            }
        }
    }

    // ── Chrome context menu (drag handle reset gesture) ────────────────────

    /// Show the chrome context menu anchored at the cursor position if the
    /// cursor is currently over a drag handle.
    ///
    /// Called on right-click (desktop).  No-op if the cursor is not on a
    /// drag handle.
    pub(super) fn handle_right_click_on_drag_handle(&mut self) {
        let cx = self.state.cursor_x;
        let cy = self.state.cursor_y;

        // Find the drag handle under the cursor.
        let element_id = {
            let Ok(state) = self.state.shared_state.try_lock() else {
                return;
            };
            let Ok(scene) = state.scene.try_lock() else {
                return;
            };
            scene
                .overlay
                .drag_handle_hit_regions
                .iter()
                .find(|r| {
                    cx >= r.bounds.x
                        && cx < r.bounds.x + r.bounds.width
                        && cy >= r.bounds.y
                        && cy < r.bounds.y + r.bounds.height
                })
                .map(|r| r.element_id)
        };

        let Some(element_id) = element_id else {
            return; // Cursor is not on a drag handle — nothing to show.
        };

        // Anchor the menu to the right-click position.
        // Pre-compute the reset button rect (constant throughout the menu's lifetime).
        const MENU_W: f32 = 160.0;
        const MENU_H: f32 = 32.0;
        const PADDING: f32 = 4.0;
        let menu = DragHandleContextMenuState {
            element_id,
            anchor_x: cx,
            anchor_y: cy,
            shown_at_ns: nanoseconds_since_start(),
            reset_button_rect: Some(tze_hud_scene::Rect::new(
                cx + PADDING,
                cy + PADDING,
                MENU_W - PADDING * 2.0,
                MENU_H - PADDING * 2.0,
            )),
        };

        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                scene.overlay.drag_handle_context_menu = Some(menu);
                tracing::debug!(
                    element_id = %element_id,
                    x = cx,
                    y = cy,
                    "chrome context menu shown for drag handle"
                );
            }
        }
    }

    /// Handle a left-click when the chrome context menu is showing.
    ///
    /// - If the click lands on the "Reset to default" button rect → trigger reset.
    /// - Otherwise → dismiss the menu (click-outside).
    ///
    /// Called synchronously on `MouseButton::Left` release, before the normal
    /// `enqueue_pointer_event` path.
    pub(super) fn handle_left_click_with_context_menu(&mut self) {
        let cx = self.state.cursor_x;
        let cy = self.state.cursor_y;

        // Extract context menu state, then immediately drop the scene lock.
        let menu_state = {
            let Ok(state) = self.state.shared_state.try_lock() else {
                return;
            };
            let Ok(scene) = state.scene.try_lock() else {
                return;
            };
            scene.overlay.drag_handle_context_menu.clone()
        };

        let Some(menu) = menu_state else {
            return; // Menu not showing — nothing to do.
        };

        // Check if the click landed on the Reset button.
        let hit_reset = menu
            .reset_button_rect
            .is_some_and(|r| cx >= r.x && cx < r.x + r.width && cy >= r.y && cy < r.y + r.height);

        // Dismiss the menu in all cases.
        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                scene.overlay.drag_handle_context_menu = None;
            }
        }

        if !hit_reset {
            tracing::debug!("chrome context menu dismissed (click-outside)");
            return;
        }

        // Reset the element geometry.
        self.perform_reset_element_geometry(menu.element_id);
    }

    /// Auto-dismiss the context menu after 3 seconds.
    ///
    /// Called each frame from `RedrawRequested`.  No-op when no menu is showing.
    pub(super) fn tick_context_menu_auto_dismiss(&mut self) {
        const AUTO_DISMISS_NS: u64 = 3_000_000_000; // 3 seconds

        let now_ns = nanoseconds_since_start();

        let should_dismiss = {
            let Ok(state) = self.state.shared_state.try_lock() else {
                return;
            };
            let Ok(scene) = state.scene.try_lock() else {
                return;
            };
            scene
                .overlay
                .drag_handle_context_menu
                .as_ref()
                .is_some_and(|m| now_ns.saturating_sub(m.shown_at_ns) >= AUTO_DISMISS_NS)
        };

        if should_dismiss {
            if let Ok(state) = self.state.shared_state.try_lock() {
                if let Ok(mut scene) = state.scene.try_lock() {
                    scene.overlay.drag_handle_context_menu = None;
                    tracing::debug!("chrome context menu auto-dismissed after 3s");
                }
            }
        }
    }

    /// Synchronously reset the geometry override for `element_id` and broadcast
    /// an `ElementRepositionedEvent` (hud-zc7f).
    ///
    /// This is the sync chrome-layer path for the "Reset to default" context
    /// menu action.  It mirrors the logic in `HudSessionImpl::reset_element_geometry`
    /// but runs on the main thread without async, using the stored broadcast sender.
    ///
    /// No-op if the element has no user override.
    fn perform_reset_element_geometry(&mut self, element_id: tze_hud_scene::SceneId) {
        // Collect previous override, fallback geometry, and optional persist path.
        let (previous_override, fallback_geometry, store_snapshot, persist_path) = {
            let Ok(mut state) = self.state.shared_state.try_lock() else {
                tracing::warn!("perform_reset_element_geometry: could not acquire shared state");
                return;
            };
            // Hand geometry authority back to the adapter FIRST (hud-lyqun),
            // before the override no-op short-circuit below: a whole-portal
            // move/resize takes the viewer lock but does not always write an
            // element-store override, so the lock must be released here even when
            // there is no override to clear. A portal member's reset releases the
            // WHOLE portal group so every constituent surface becomes
            // adapter-controlled again — not just the one the reset menu was
            // opened on.
            if let Ok(mut scene) = state.scene.try_lock() {
                match super::portal::resolve_portal_group(&scene, element_id) {
                    Some(group) => {
                        for member_id in group.member_ids {
                            scene.unlock_viewer_geometry(member_id);
                        }
                    }
                    None => scene.unlock_viewer_geometry(element_id),
                }
            }

            // Clear the override.
            let previous = state.element_store.reset_geometry_override(element_id);
            let Some(previous) = previous else {
                tracing::debug!(
                    element_id = %element_id,
                    "perform_reset_element_geometry: no override — geometry lock released, no override to clear"
                );
                return;
            };
            // Resolve fallback geometry (agent bounds → config → default policy).
            let fallback = {
                let Ok(scene) = state.scene.try_lock() else {
                    return;
                };
                state
                    .element_store
                    .entries
                    .get(&element_id)
                    .map(|entry| {
                        tze_hud_scene::element_store::fallback_geometry_for_element(
                            element_id, entry, &scene,
                        )
                    })
                    .unwrap_or(tze_hud_scene::ZERO_GEOMETRY_POLICY)
            };
            let store_snapshot = state.element_store.clone();
            let persist_path = state.element_store_path.clone();
            (previous, fallback, store_snapshot, persist_path)
        };

        // Persist the updated store on a background thread to avoid blocking the
        // Winit event loop with sync disk I/O (atomic write + fsync).
        if let Some(path) = persist_path {
            std::thread::spawn(move || {
                if let Err(e) =
                    crate::element_store::persist_element_store_to_path(&store_snapshot, &path)
                {
                    tracing::warn!(error = %e, "perform_reset_element_geometry: persist failed");
                }
            });
        }

        // Broadcast ElementRepositionedEvent.
        if let Some(ref tx) = self.state.element_repositioned_tx {
            let event = tze_hud_protocol::proto::ElementRepositionedEvent {
                // Use big-endian UUID bytes to match scene_id_to_bytes wire contract.
                element_id: element_id.as_uuid().as_bytes().to_vec(),
                new_geometry: Some(tze_hud_protocol::convert::geometry_policy_to_proto(
                    &fallback_geometry,
                )),
                previous_geometry: Some(tze_hud_protocol::convert::geometry_policy_to_proto(
                    &previous_override,
                )),
            };
            // Errors (no receivers, channel lagged) are silently ignored —
            // ElementRepositionedEvent is an ephemeral-realtime notification;
            // subscribers that are not present at the moment of a geometry reset
            // will learn the restored geometry on next subscription refresh.
            tx.send(event).unwrap_or_default();
            tracing::debug!(
                element_id = %element_id,
                "ElementRepositionedEvent broadcast after reset-to-default"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::graph::SceneGraph;

    // ─── resolve_composer_placeholder (hud-se6hs) ─────────────────────────

    /// Unset override (`None`, i.e. no `composer_placeholder` configured on
    /// the owning `HitRegionNode`) falls back to the global default —
    /// backward-compatible with every existing chat-composer caller.
    #[test]
    fn resolve_composer_placeholder_falls_back_to_default_when_unset() {
        assert_eq!(
            resolve_composer_placeholder(None),
            Some(COMPOSER_DEFAULT_PLACEHOLDER.to_string())
        );
    }

    /// A custom per-composer hint overrides the global default verbatim.
    #[test]
    fn resolve_composer_placeholder_uses_custom_override() {
        assert_eq!(
            resolve_composer_placeholder(Some("Search…".to_string())),
            Some("Search…".to_string())
        );
    }

    /// An explicit opt-out (`Some("")`) suppresses the placeholder entirely —
    /// distinct from the unset case above, which still shows the default.
    #[test]
    fn resolve_composer_placeholder_empty_string_opts_out() {
        assert_eq!(resolve_composer_placeholder(Some(String::new())), None);
    }

    #[test]
    fn hover_tracker_region_resolves_from_widget_geometry() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("create tab");

        scene
            .widget_registry
            .register_definition(tze_hud_scene::types::WidgetDefinition {
                id: "status-indicator".to_string(),
                name: "status-indicator".to_string(),
                description: "test".to_string(),
                parameter_schema: Vec::new(),
                layers: Vec::new(),
                default_geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 1.0,
                },
                default_rendering_policy: tze_hud_scene::types::RenderingPolicy::default(),
                default_contention_policy: tze_hud_scene::types::ContentionPolicy::LatestWins,
                max_publishers: tze_hud_scene::types::WidgetDefinition::default_max_publishers(),
                ephemeral: false,
                hover_behavior: Some(tze_hud_scene::types::WidgetHoverBehavior {
                    trigger_rect: tze_hud_scene::types::WidgetNormalizedRect {
                        x_pct: 0.88,
                        y_pct: 0.06,
                        width_pct: 0.08,
                        height_pct: 0.22,
                    },
                    delay_ms: 3_000,
                    visibility_param: "tooltip_visible".to_string(),
                    hidden_value: 0.0,
                    visible_value: 1.0,
                }),
            });
        scene
            .widget_registry
            .register_instance(tze_hud_scene::types::WidgetInstance {
                id: tze_hud_scene::SceneId::new(),
                widget_type_name: "status-indicator".to_string(),
                tab_id,
                geometry_override: Some(tze_hud_scene::types::GeometryPolicy::Relative {
                    x_pct: 1660.0 / 1920.0,
                    y_pct: 8.0 / 1080.0,
                    width_pct: 252.0 / 1920.0,
                    height_pct: 96.0 / 1080.0,
                }),
                contention_override: None,
                instance_name: "main-status".to_string(),
                current_params: std::collections::HashMap::new(),
            });

        let trackers =
            build_hover_trackers(&scene, 1920.0, 1080.0, &std::collections::HashMap::new());
        let region = trackers
            .get("main-status")
            .map(|t| t.region.clone())
            .expect("main-status hover tracker must resolve");
        assert!(
            region.x >= 1880.0 && region.x <= 1905.0,
            "x should sit near the icon trigger region, got {}",
            region.x
        );
        assert!(
            region.y >= 12.0 && region.y <= 18.0,
            "y should sit near the top trigger margin, got {}",
            region.y
        );
        assert!(
            region.width >= 19.0 && region.width <= 21.0,
            "width should match normalized trigger width"
        );
    }
}
