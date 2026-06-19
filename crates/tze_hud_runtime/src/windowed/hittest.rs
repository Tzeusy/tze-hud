use tze_hud_compositor::{Compositor, CompositorSurface};
use tze_hud_scene::graph::SceneGraph;

use super::WinitApp;
use crate::window::{HitRegion, WindowMode, should_capture_pointer_event};

pub(super) fn sync_scene_display_area(scene: &mut SceneGraph, width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    scene.display_area = tze_hud_scene::Rect::new(0.0, 0.0, width as f32, height as f32);
}

fn zone_hit_regions_to_overlay_regions(scene: &SceneGraph) -> Vec<HitRegion> {
    scene
        .overlay
        .zone_hit_regions
        .iter()
        .map(|region| {
            HitRegion::new(
                region.bounds.x,
                region.bounds.y,
                region.bounds.width,
                region.bounds.height,
            )
        })
        .collect()
}

/// Collect overlay capture regions from content-layer tiles that contain at
/// least one `HitRegionNode` with `accepts_pointer = true`.
///
/// In overlay mode, the OS routes all pointer events to the desktop unless the
/// window has set `cursor_hittest(true)`. We flip to capture only when the
/// cursor is inside a known interactive region. Zone-owned affordances are
/// covered by `zone_hit_regions_to_overlay_regions`; this function handles
/// agent-owned content tiles that carry a `HitRegionNode`.
///
/// **Granularity**: we use the tile's display-space bounding box as the OS
/// capture region (not individual node bounds). The OS capture decision must be
/// made before the event is dispatched, so a coarser region is correct here.
/// Precise hit-testing against individual `HitRegionNode` bounds still happens
/// in Stage 2 (`crates/tze_hud_input/src/hit_test.rs`) after the event arrives.
///
/// Tiles with `InputMode::Passthrough` are excluded: they are transparent by
/// design and must not block underlying desktop events.
fn content_tile_hit_regions_from_scene(scene: &SceneGraph) -> Vec<HitRegion> {
    let mut regions = Vec::new();
    for tile in scene.tiles.values() {
        if tile.input_mode == tze_hud_scene::InputMode::Passthrough {
            continue;
        }
        if let Some(root_id) = tile.root_node {
            if tile_has_pointer_hit_region(scene, root_id) {
                regions.push(HitRegion::new(
                    tile.bounds.x,
                    tile.bounds.y,
                    tile.bounds.width,
                    tile.bounds.height,
                ));
            }
        }
    }
    regions
}

/// Returns `true` if the node subtree rooted at `node_id` contains at least
/// one `HitRegionNode` with `accepts_pointer = true`.
fn tile_has_pointer_hit_region(scene: &SceneGraph, node_id: tze_hud_scene::SceneId) -> bool {
    let Some(node) = scene.nodes.get(&node_id) else {
        return false;
    };
    if let tze_hud_scene::NodeData::HitRegion(hr) = &node.data {
        if hr.accepts_pointer {
            return true;
        }
    }
    node.children
        .iter()
        .any(|&child_id| tile_has_pointer_hit_region(scene, child_id))
}

pub(super) fn combined_overlay_hit_regions(
    static_regions: &[HitRegion],
    scene: &SceneGraph,
) -> Vec<HitRegion> {
    let mut regions = static_regions.to_vec();
    regions.extend(zone_hit_regions_to_overlay_regions(scene));
    regions.extend(content_tile_hit_regions_from_scene(scene));
    regions
}

pub(super) fn refresh_interaction_hit_regions_after_render(
    compositor: &Compositor,
    scene: &mut SceneGraph,
    surface: &dyn CompositorSurface,
) {
    let (surf_w, surf_h) = surface.size();
    compositor.populate_zone_hit_regions(scene, surf_w as f32, surf_h as f32);
}

impl WinitApp {
    /// Refresh cursor position from OS state when passthrough is active.
    ///
    /// In overlay mode on Windows, `set_cursor_hittest(false)` can prevent
    /// `CursorMoved` delivery to winit. Polling global cursor position ensures
    /// hit-testing can flip back to capture when the cursor enters an active
    /// widget hover region.
    pub(super) fn refresh_cursor_position_from_os(&mut self) {
        if self.state.effective_mode == WindowMode::Overlay {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::Foundation::POINT;
                use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

                let Some(window) = &self.state.window else {
                    return;
                };
                let window_pos = window
                    .outer_position()
                    .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0));

                let mut pt = POINT { x: 0, y: 0 };
                // SAFETY: GetCursorPos writes to the provided POINT and has no
                // additional safety preconditions.
                let ok = unsafe { GetCursorPos(&mut pt).is_ok() };
                if ok {
                    self.state.cursor_x = (pt.x - window_pos.x) as f32;
                    self.state.cursor_y = (pt.y - window_pos.y) as f32;
                }
            }
        }
    }

    /// Update overlay passthrough/capture state from current cursor+regions.
    pub(super) fn update_overlay_cursor_hittest(&mut self) {
        if self.state.effective_mode != WindowMode::Overlay {
            return;
        }
        let should_capture = should_capture_pointer_event(
            WindowMode::Overlay,
            self.state.cursor_x,
            self.state.cursor_y,
            &self.state.hit_regions,
        ) || self.state.left_button_down;
        if let Some(window) = &self.state.window {
            if let Err(e) = window.set_cursor_hittest(should_capture) {
                tracing::trace!(
                    error = %e,
                    capture = should_capture,
                    "overlay: set_cursor_hittest failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{scene_with_capture_tile, scene_with_drag_handle_tile};
    use super::*;
    use tze_hud_compositor::HeadlessSurface;

    #[test]
    fn combined_overlay_hit_regions_includes_zone_interaction_regions() {
        let static_regions = vec![HitRegion::new(10.0, 20.0, 30.0, 40.0)];
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene
            .overlay
            .zone_hit_regions
            .push(tze_hud_scene::types::ZoneHitRegion {
                zone_name: "notification-area".to_string(),
                published_at_wall_us: 123,
                publisher_namespace: "test-agent".to_string(),
                bounds: tze_hud_scene::types::Rect::new(100.0, 200.0, 20.0, 20.0),
                kind: tze_hud_scene::types::ZoneInteractionKind::Dismiss,
                interaction_id: "zone:notification-area:dismiss:123:test-agent".to_string(),
                tab_order: 0,
            });

        let combined = combined_overlay_hit_regions(&static_regions, &scene);

        assert_eq!(combined.len(), 2, "static + zone capture regions expected");
        assert_eq!(
            combined[0], static_regions[0],
            "static region must be preserved"
        );
        assert_eq!(
            combined[1],
            HitRegion::new(100.0, 200.0, 20.0, 20.0),
            "zone hit region must be exposed to overlay click-capture"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn windowed_post_render_refresh_updates_drag_handles_before_snapshot() {
        let _runtime_guard = crate::test_support::lock_headless_runtime().await;
        let mut compositor = match Compositor::new_headless(1920, 1080).await {
            Ok(compositor) => compositor,
            Err(e) => {
                eprintln!("skipping GPU-dependent drag-handle refresh test: {e}");
                return;
            }
        };
        let surface = HeadlessSurface::new(&compositor.device, 1920, 1080);
        let (mut scene, tile_id, _element_id, _interaction_id) =
            scene_with_drag_handle_tile(400.0, 300.0, 600.0, 200.0);

        let tile = scene
            .tiles
            .get_mut(&tile_id)
            .expect("drag-handle test tile must exist");
        tile.bounds.x = 500.0;
        tile.bounds.y = 330.0;

        let stale_snapshot = crate::pipeline::HitTestSnapshot::from_scene(&scene);
        assert!(
            !stale_snapshot.hit_test_drag_handle(800.0, 330.0),
            "pre-refresh snapshot still carries the previous frame's drag-handle bounds"
        );

        compositor.prime_markdown_cache(&scene);
        compositor.prime_truncation_cache(&scene);
        compositor.render_frame(&mut scene, &surface);
        refresh_interaction_hit_regions_after_render(&compositor, &mut scene, &surface);
        let refreshed_snapshot = crate::pipeline::HitTestSnapshot::from_scene(&scene);

        assert!(
            refreshed_snapshot.hit_test_drag_handle(800.0, 330.0),
            "snapshot built after the windowed post-render refresh must match the newly displayed drag-handle geometry"
        );
    }

    #[test]
    fn content_tile_with_hit_region_node_registers_capture_region() {
        let (scene, tile_id) = scene_with_capture_tile();
        let tile_bounds = scene.tiles[&tile_id].bounds;

        let regions = content_tile_hit_regions_from_scene(&scene);

        assert_eq!(
            regions.len(),
            1,
            "exactly one capture region for one capture-mode tile with HitRegionNode"
        );
        assert_eq!(
            regions[0],
            HitRegion::new(
                tile_bounds.x,
                tile_bounds.y,
                tile_bounds.width,
                tile_bounds.height
            ),
            "capture region must match the tile's display-space bounds"
        );
    }

    #[test]
    fn passthrough_tile_with_hit_region_does_not_register_capture_region() {
        use tze_hud_scene::{Capability, HitRegionNode, InputMode, Node, NodeData, Rect, SceneId};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "overlay",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "overlay",
                lease_id,
                Rect::new(0.0, 0.0, 300.0, 300.0),
                1,
            )
            .unwrap();
        scene.tiles.get_mut(&tile_id).unwrap().input_mode = InputMode::Passthrough;
        let node_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: node_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 100.0, 50.0),
                        interaction_id: "passthrough-btn".to_string(),
                        accepts_pointer: true,
                        accepts_focus: false,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let regions = content_tile_hit_regions_from_scene(&scene);
        assert!(
            regions.is_empty(),
            "passthrough tile must not register a capture region"
        );
    }

    #[test]
    fn tile_with_accepts_pointer_false_does_not_register_capture_region() {
        use tze_hud_scene::{Capability, HitRegionNode, Node, NodeData, Rect, SceneId};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 100.0),
                1,
            )
            .unwrap();
        let node_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: node_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
                        interaction_id: "focus-only".to_string(),
                        accepts_pointer: false,
                        accepts_focus: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let regions = content_tile_hit_regions_from_scene(&scene);
        assert!(
            regions.is_empty(),
            "tile with accepts_pointer=false node must not register a capture region"
        );
    }

    #[test]
    fn combined_overlay_hit_regions_includes_content_tile_regions() {
        let (scene, tile_id) = scene_with_capture_tile();
        let tile_bounds = scene.tiles[&tile_id].bounds;

        let static_regions = vec![HitRegion::new(10.0, 10.0, 50.0, 50.0)];
        let combined = combined_overlay_hit_regions(&static_regions, &scene);

        assert_eq!(
            combined.len(),
            2,
            "static + content-tile capture regions expected"
        );
        assert_eq!(combined[0], static_regions[0], "static region preserved");
        assert_eq!(
            combined[1],
            HitRegion::new(
                tile_bounds.x,
                tile_bounds.y,
                tile_bounds.width,
                tile_bounds.height
            ),
            "content-tile capture region must cover tile display-space bounds"
        );
    }

    #[test]
    fn fullscreen_captures_pointer_outside_any_hit_region() {
        let capture = should_capture_pointer_event(WindowMode::Fullscreen, 9000.0, 9000.0, &[]);
        assert!(capture, "fullscreen must capture all pointer events");
    }

    #[test]
    fn fullscreen_captures_pointer_even_with_regions_present() {
        let regions = vec![HitRegion::new(0.0, 0.0, 100.0, 100.0)];
        let capture =
            should_capture_pointer_event(WindowMode::Fullscreen, 9000.0, 9000.0, &regions);
        assert!(capture);
    }

    #[test]
    fn overlay_no_hit_regions_passes_through_all_events() {
        let capture = should_capture_pointer_event(WindowMode::Overlay, 500.0, 500.0, &[]);
        assert!(
            !capture,
            "overlay with no hit-regions must pass all events through"
        );
    }

    #[test]
    fn overlay_cursor_inside_hit_region_is_captured() {
        let regions = vec![HitRegion::new(100.0, 100.0, 200.0, 150.0)];
        let capture = should_capture_pointer_event(WindowMode::Overlay, 150.0, 150.0, &regions);
        assert!(capture, "cursor inside hit-region must be captured");
    }

    #[test]
    fn overlay_cursor_outside_all_hit_regions_passes_through() {
        let regions = vec![HitRegion::new(100.0, 100.0, 200.0, 150.0)];
        let capture = should_capture_pointer_event(WindowMode::Overlay, 50.0, 50.0, &regions);
        assert!(!capture, "cursor outside all hit-regions must pass through");
    }

    #[test]
    fn overlay_multiple_hit_regions_union_semantics() {
        let regions = vec![
            HitRegion::new(0.0, 0.0, 100.0, 100.0),
            HitRegion::new(500.0, 500.0, 100.0, 100.0),
        ];
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            50.0,
            50.0,
            &regions
        ));
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            550.0,
            550.0,
            &regions
        ));
        assert!(!should_capture_pointer_event(
            WindowMode::Overlay,
            300.0,
            300.0,
            &regions
        ));
    }

    #[test]
    fn scene_display_area_sync_uses_actual_surface_size() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        sync_scene_display_area(&mut scene, 2560, 1440);

        assert_eq!(scene.display_area.width, 2560.0);
        assert_eq!(scene.display_area.height, 1440.0);
    }

    #[test]
    fn scene_display_area_sync_ignores_zero_dimensions() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        sync_scene_display_area(&mut scene, 0, 1440);

        assert_eq!(scene.display_area.width, 1920.0);
        assert_eq!(scene.display_area.height, 1080.0);
    }
}
