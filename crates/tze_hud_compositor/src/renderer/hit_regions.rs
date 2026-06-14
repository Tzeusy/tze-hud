//! Hit-region population methods for the compositor.
//!
//! Moved from `renderer/mod.rs` (the "Hit regions" cluster,
//! formerly ~L7346–7899 at plan date) by Step R-8 of the renderer module
//! split (hud-fgryk).  No logic was changed; only the `pub(super)` visibility
//! modifier was added to `populate_drag_handle_hit_regions_from` and
//! `collect_context_menu_vertices` so that callers in sibling modules can
//! access them.
//!
//! ## Methods in this file
//!
//! - `populate_drag_handle_hit_regions` — recompute runtime-internal
//!   drag-handle hit regions for the current frame (calls
//!   `collect_drag_handle_entries` then `populate_drag_handle_hit_regions_from`).
//! - `populate_drag_handle_hit_regions_from` — populate drag-handle hit regions
//!   from a pre-computed entry list (avoids a second collection pass for
//!   callers that already hold a `collect_drag_handle_entries` result).
//! - `collect_context_menu_vertices` — build vertices for the drag-handle reset
//!   context menu popup when one is showing.
//! - `populate_zone_hit_regions` — recompute zone interaction hit regions
//!   (dismiss and action buttons) for the current frame.

use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;

use super::Compositor;
use super::draw_cmds::DragHandleEntry;
use super::token_colors::{is_alert_banner_zone, sort_alert_banner_indices};
use crate::pipeline::{RectVertex, rect_vertices};

impl Compositor {
    /// Recompute runtime-internal drag-handle hit regions for the current frame.
    pub fn populate_drag_handle_hit_regions(&self, scene: &mut SceneGraph, sw: f32, sh: f32) {
        let handles = self.collect_drag_handle_entries(scene, sw, sh);
        self.populate_drag_handle_hit_regions_from(scene, handles);
    }

    /// Populate drag-handle hit regions from a pre-computed entry list.
    ///
    /// Callers that already hold a `collect_drag_handle_entries` result (e.g.
    /// `render_frame_headless`) should use this variant to avoid a second
    /// collection pass.
    pub(super) fn populate_drag_handle_hit_regions_from(
        &self,
        scene: &mut SceneGraph,
        handles: Vec<DragHandleEntry>,
    ) {
        scene.overlay.drag_handle_hit_regions.clear();
        for (tab_order, entry) in (0_u32..).zip(handles.into_iter()) {
            let hit_region = HitRegionNode {
                bounds: entry.bounds,
                interaction_id: entry.interaction_id.clone(),
                accepts_focus: false,
                accepts_pointer: true,
                auto_capture: false,
                ..Default::default()
            };
            scene
                .overlay
                .drag_handle_hit_regions
                .push(DragHandleHitRegion {
                    element_id: entry.element_id,
                    element_kind: entry.element_kind,
                    bounds: entry.bounds,
                    interaction_id: entry.interaction_id,
                    hit_region,
                    tab_order,
                });
        }

        scene.overlay.drag_handle_states.retain(|k, _| {
            scene
                .overlay
                .drag_handle_hit_regions
                .iter()
                .any(|r| &r.interaction_id == k)
        });
    }

    // ── Chrome context menu rendering (hud-zc7f) ─────────────────────────────

    /// Dimensions for the "Reset to default" context menu popup.
    const CONTEXT_MENU_W: f32 = 160.0;
    const CONTEXT_MENU_H: f32 = 32.0;
    const CONTEXT_MENU_PADDING: f32 = 4.0;

    /// Build vertices for the drag-handle reset context menu, if one is showing.
    ///
    /// Returns an empty `Vec` when `scene.overlay.drag_handle_context_menu` is `None`.
    /// The menu is rendered as two rects:
    /// - A semi-transparent dark background (the popup container).
    /// - A lighter "Reset to default" button inside it.
    pub(super) fn collect_context_menu_vertices(
        &self,
        scene: &SceneGraph,
        sw: f32,
        sh: f32,
    ) -> Vec<RectVertex> {
        let Some(ref menu) = scene.overlay.drag_handle_context_menu else {
            return Vec::new();
        };

        let mut vertices = Vec::with_capacity(12); // 2 quads × 6 vertices

        // Menu background.
        let bg_rgba = [0.12_f32, 0.12, 0.12, 0.92];
        let bg_color = self.gpu_color_raw(bg_rgba);
        let bg_verts = rect_vertices(
            menu.anchor_x,
            menu.anchor_y,
            Self::CONTEXT_MENU_W,
            Self::CONTEXT_MENU_H,
            sw,
            sh,
            bg_color,
        );
        vertices.extend_from_slice(&bg_verts);

        // "Reset to default" button.
        let btn_rgba = [0.22_f32, 0.22, 0.22, 0.95];
        let btn_color = self.gpu_color_raw(btn_rgba);
        let p = Self::CONTEXT_MENU_PADDING;
        let btn_verts = rect_vertices(
            menu.anchor_x + p,
            menu.anchor_y + p,
            Self::CONTEXT_MENU_W - p * 2.0,
            Self::CONTEXT_MENU_H - p * 2.0,
            sw,
            sh,
            btn_color,
        );
        vertices.extend_from_slice(&btn_verts);

        vertices
    }

    /// Recompute the zone interaction hit regions for the current frame.
    ///
    /// Clears `scene.overlay.zone_hit_regions` then repopulates it with dismiss (×)
    /// buttons and action buttons for every visible notification slot in every
    /// Stack zone that contains `ZoneContent::Notification` publications.
    ///
    /// # Layout
    ///
    /// For each notification slot (height = `stack_slot_height(policy)`):
    ///
    /// - **Dismiss button**: a square in the top-right corner of the slot.
    ///   Size: `DISMISS_BUTTON_SIZE × DISMISS_BUTTON_SIZE` px.
    ///   Position: `(slot_right - DISMISS_BUTTON_SIZE, slot_y)`.
    ///
    /// - **Action buttons**: a horizontal row at the bottom of the slot.
    ///   Each button is `ACTION_BUTTON_H` px tall and
    ///   `(slot_w - inset * 2) / n_actions` px wide (where `n_actions` is
    ///   capped at `MAX_NOTIFICATION_ACTIONS`).
    ///
    /// Tab order within a slot: dismiss button first, then actions left-to-right.
    /// Slots are ordered top-to-bottom (slot 0 = newest, matching rendering order).
    ///
    /// # Called by
    ///
    /// Called after a frame render completes to refresh `scene.overlay.zone_hit_regions`
    /// for the next frame's hit-testing.  This prepares `SceneGraph::hit_test` to
    /// return `ZoneInteraction` for zone affordances based on the most recently
    /// rendered layout.
    pub fn populate_zone_hit_regions(&self, scene: &mut SceneGraph, sw: f32, sh: f32) {
        /// Side length of the dismiss (×) button in pixels.
        const DISMISS_BUTTON_SIZE: f32 = 20.0;
        /// Height of each action button row in pixels.
        const ACTION_BUTTON_H: f32 = 22.0;
        /// Horizontal inset used to position action buttons (matches notification inset).
        const ACTION_INSET: f32 = 9.0;

        scene.overlay.zone_hit_regions.clear();
        let mut tab_order: u32 = 0;

        // Sort zone names for deterministic tab-order assignment across frames.
        // HashMap iteration order is nondeterministic; sorting ensures keyboard
        // focus cycling is stable when multiple interactive zones are present.
        let mut zone_names: Vec<_> = scene.zone_registry.active_publishes.keys().collect();
        zone_names.sort_unstable();

        for zone_name in zone_names {
            let publishes = match scene.zone_registry.active_publishes.get(zone_name) {
                Some(p) => p,
                None => continue,
            };
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Only Stack zones with Notification content get interactive regions.
            if !matches!(zone_def.contention_policy, ContentionPolicy::Stack { .. }) {
                continue;
            }

            let policy = &zone_def.rendering_policy;
            let (zx, zy, zw, zh) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);
            let slot_h = Self::stack_slot_height(policy);

            // alert-banner uses dynamic height; other Stack zones use configured zh.
            let effective_zh = if is_alert_banner_zone(zone_name) {
                publishes.len() as f32 * slot_h
            } else {
                zh
            };

            // Ordered as in render_zone_content: newest-first for regular zones,
            // severity-descending for alert-banner.
            let ordered: Vec<&ZonePublishRecord> = if is_alert_banner_zone(zone_name) {
                sort_alert_banner_indices(publishes)
                    .into_iter()
                    .map(|idx| &publishes[idx])
                    .collect()
            } else {
                publishes.iter().rev().collect()
            };

            for (slot_idx, record) in ordered.iter().enumerate() {
                let slot_y = zy + slot_idx as f32 * slot_h;
                if slot_y >= zy + effective_zh {
                    break;
                }
                let effective_slot_h = slot_h.min((zy + effective_zh) - slot_y);

                let n_payload = match &record.content {
                    ZoneContent::Notification(n) => n,
                    _ => continue, // Only notifications get interactive affordances.
                };

                // ── Dismiss (×) button ────────────────────────────────────────
                // Top-right corner of the slot, DISMISS_BUTTON_SIZE square.
                let dismiss_bounds = Rect::new(
                    zx + zw - DISMISS_BUTTON_SIZE,
                    slot_y,
                    DISMISS_BUTTON_SIZE,
                    DISMISS_BUTTON_SIZE.min(effective_slot_h),
                );
                let dismiss_id = format!(
                    "zone:{}:dismiss:{}:{}",
                    zone_name, record.published_at_wall_us, record.publisher_namespace,
                );
                scene.overlay.zone_hit_regions.push(ZoneHitRegion {
                    zone_name: zone_name.clone(),
                    published_at_wall_us: record.published_at_wall_us,
                    publisher_namespace: record.publisher_namespace.clone(),
                    bounds: dismiss_bounds,
                    kind: ZoneInteractionKind::Dismiss,
                    interaction_id: dismiss_id,
                    tab_order,
                });
                tab_order += 1;

                // ── Action buttons ────────────────────────────────────────────
                let n_actions = n_payload.actions.len().min(MAX_NOTIFICATION_ACTIONS);
                if n_actions > 0 {
                    let avail_w = (zw - ACTION_INSET * 2.0).max(1.0);
                    let btn_w = avail_w / n_actions as f32;
                    let action_y = slot_y + effective_slot_h - ACTION_BUTTON_H;

                    for (btn_idx, action) in n_payload.actions.iter().take(n_actions).enumerate() {
                        let btn_x = zx + ACTION_INSET + btn_idx as f32 * btn_w;
                        let action_bounds = Rect::new(
                            btn_x,
                            action_y.max(slot_y),
                            btn_w,
                            ACTION_BUTTON_H.min(effective_slot_h),
                        );
                        let action_id = format!(
                            "zone:{}:action:{}:{}:{}",
                            zone_name,
                            record.published_at_wall_us,
                            record.publisher_namespace,
                            action.callback_id,
                        );
                        scene.overlay.zone_hit_regions.push(ZoneHitRegion {
                            zone_name: zone_name.clone(),
                            published_at_wall_us: record.published_at_wall_us,
                            publisher_namespace: record.publisher_namespace.clone(),
                            bounds: action_bounds,
                            kind: ZoneInteractionKind::Action {
                                callback_id: action.callback_id.clone(),
                            },
                            interaction_id: action_id,
                            tab_order,
                        });
                        tab_order += 1;
                    }
                }
            }
        }
    }
}
