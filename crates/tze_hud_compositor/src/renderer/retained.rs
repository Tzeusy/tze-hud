//! Narrow retained-render path used by the canonical change-efficiency proof.
//!
//! This module deliberately does not consume `SceneDiff` or any reconnect/WAL
//! protocol state.  It owns a private compositor snapshot and accepts only the
//! bounded fifty-tile headless scene used by the Layer 3 evidence lane.  Every
//! other scene stays on the established full-frame renderer.

use std::collections::BTreeSet;
use std::time::Instant;

use wgpu::util::DeviceExt;

use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{NodeData, Rect, SceneId, TextMarkdownNode};
use tze_hud_telemetry::{
    ActualWorkItem, ChangeEfficiencyArtifact, ChangeMeasurementProvenance, ChangeMeasurementStatus,
    ChangeRenderWorkObservation, ClosureWorkItem, EfficiencyPacingIdentity, EfficiencyPacingMode,
    EfficiencyRendererIdentity, EfficiencyRuntimeIdentity, EfficiencyScenarioIdentity,
    EfficiencyViewport, EfficiencyWindowMode, FullSurfaceInvalidation,
    FullSurfaceInvalidationReason, InvalidationCategory, InvalidationClosure,
    InvalidationDependencyReason, NodeWorkItemId, PartialPresentCapability, PixelRect,
    RenderPlanWorkItemId, TextureUploadCategory,
};

use crate::pipeline::rect_vertices;
use crate::surface::{CompositorSurface, HeadlessSurface};
use crate::text::TextItem;

use super::Compositor;

const CANONICAL_TILE_COUNT: usize = 50;

/// Compositor-private retained state for the canonical headless proof lane.
#[derive(Default)]
pub(super) struct RetainedRenderState {
    snapshot: Option<CanonicalSceneSnapshot>,
    pending_full_surface_diagnostic: Option<FullSurfaceInvalidation>,
    latest_capture: Option<RetainedChangeEfficiencyCapture>,
    latest_diagnostic: Option<ChangeEfficiencyArtifact>,
    device_recovery_pending: bool,
}

/// Opaque proof returned only by the compositor after the retained headless
/// renderer has encoded and submitted scoped work. A serialized
/// `ChangeEfficiencyArtifact` cannot manufacture this wrapper.
#[derive(Debug)]
#[must_use = "a retained runtime capture should be validated or inspected"]
pub struct RetainedChangeEfficiencyCapture {
    artifact: ChangeEfficiencyArtifact,
}

impl RetainedChangeEfficiencyCapture {
    fn from_observed_runtime(artifact: ChangeEfficiencyArtifact) -> Self {
        debug_assert_eq!(
            artifact.measurement_provenance,
            ChangeMeasurementProvenance::ObservedRetainedRuntime,
            "only a real retained runtime observation may create a capture"
        );
        Self { artifact }
    }

    /// Inspect the immutable observed-operation artifact recorded by the
    /// compositor. Calling `ChangeEfficiencyArtifact::validate` directly is
    /// intentionally non-certifying; use [`Self::validate`] for this opaque
    /// runtime capture.
    pub fn artifact(&self) -> &ChangeEfficiencyArtifact {
        &self.artifact
    }

    /// Validate the real retained capture and upgrade only a contract-valid,
    /// scoped observation to the certification status.
    pub fn validate(&self) -> tze_hud_telemetry::ChangeEfficiencyValidation {
        let mut validation = self.artifact.validate();
        if validation.contract_satisfied
            && validation.status
                == tze_hud_telemetry::ChangeEfficiencyValidationStatus::PendingRuntimeInstrumentation
            && self.artifact.measurement_provenance
                == ChangeMeasurementProvenance::ObservedRetainedRuntime
        {
            validation.passed = true;
            validation.status =
                tze_hud_telemetry::ChangeEfficiencyValidationStatus::CertifiedRetainedRuntime;
        }
        validation
    }
}

#[derive(Clone, PartialEq)]
struct CanonicalSceneSnapshot {
    viewport: EfficiencyViewport,
    tiles: Vec<CanonicalTextTile>,
}

#[derive(Clone, PartialEq)]
struct CanonicalTextTile {
    tile_id: SceneId,
    node_id: SceneId,
    tile_bounds: Rect,
    text: TextMarkdownNode,
}

struct CanonicalTextChange {
    changed: CanonicalTextTile,
    next_snapshot: CanonicalSceneSnapshot,
}

enum RetainedPlan {
    Partial(CanonicalTextChange),
    FullFrame {
        diagnostic: Option<FullSurfaceInvalidation>,
    },
}

/// Typed, compositor-private planner for the exact retained evidence envelope.
///
/// It has no SceneDiff/WAL input: planning is derived solely from the prior
/// private snapshot and the live scene about to be rendered.
struct InvalidationPlanner;

impl InvalidationPlanner {
    fn plan_headless(
        state: &RetainedRenderState,
        scene: &SceneGraph,
        width: u32,
        height: u32,
    ) -> RetainedPlan {
        if state.device_recovery_pending {
            return RetainedPlan::FullFrame {
                diagnostic: Some(full_surface_invalidation(
                    FullSurfaceInvalidationReason::DeviceRecovery,
                    PartialPresentCapability::Supported,
                )),
            };
        }

        match (&state.snapshot, canonical_snapshot(scene, width, height)) {
            (None, Some(_)) => RetainedPlan::FullFrame {
                diagnostic: Some(full_surface_invalidation(
                    FullSurfaceInvalidationReason::SurfaceCreation,
                    PartialPresentCapability::Supported,
                )),
            },
            // This lane intentionally does not instrument arbitrary scenes
            // before it has established a canonical baseline. hud-f670c.1 owns
            // broader retained-planner coverage.
            (None, None) => RetainedPlan::FullFrame { diagnostic: None },
            (Some(_), None) => RetainedPlan::FullFrame {
                diagnostic: Some(full_surface_invalidation(
                    FullSurfaceInvalidationReason::UnsupportedRetainedSceneChange,
                    PartialPresentCapability::Supported,
                )),
            },
            (Some(previous_snapshot), Some(next_snapshot)) => {
                if previous_snapshot.viewport != next_snapshot.viewport {
                    return RetainedPlan::FullFrame {
                        diagnostic: Some(full_surface_invalidation(
                            FullSurfaceInvalidationReason::Resize,
                            PartialPresentCapability::Supported,
                        )),
                    };
                }
                if previous_snapshot == &next_snapshot {
                    return RetainedPlan::FullFrame { diagnostic: None };
                }
                let Some(changed) = changed_canonical_tile(previous_snapshot, &next_snapshot)
                else {
                    return RetainedPlan::FullFrame {
                        diagnostic: Some(full_surface_invalidation(
                            FullSurfaceInvalidationReason::UnsupportedRetainedSceneChange,
                            PartialPresentCapability::Supported,
                        )),
                    };
                };
                RetainedPlan::Partial(CanonicalTextChange {
                    changed,
                    next_snapshot,
                })
            }
        }
    }
}

impl RetainedRenderState {
    fn plan_headless(&self, scene: &SceneGraph, width: u32, height: u32) -> RetainedPlan {
        InvalidationPlanner::plan_headless(self, scene, width, height)
    }

    fn remember_full_headless_scene(&mut self, scene: &SceneGraph, width: u32, height: u32) {
        self.snapshot = canonical_snapshot(scene, width, height);
        self.device_recovery_pending = false;
    }

    fn forget_snapshot(&mut self) {
        self.snapshot = None;
    }

    fn complete_partial(&mut self, next_snapshot: CanonicalSceneSnapshot) {
        self.snapshot = Some(next_snapshot);
    }

    fn set_pending_full_surface_diagnostic(&mut self, diagnostic: Option<FullSurfaceInvalidation>) {
        self.pending_full_surface_diagnostic = diagnostic;
    }

    /// Start a new observed frame. Evidence slots are single-frame drains, not
    /// a history: a later fallback must never leave a prior passing capture
    /// available to be misattributed to the new frame.
    fn begin_observation_frame(&mut self) {
        self.clear_observed_evidence();
        // A previous planner decline that never reached the full-frame observer
        // must not leak its diagnostic into the next observed frame.
        self.pending_full_surface_diagnostic = None;
    }

    fn clear_observed_evidence(&mut self) {
        self.latest_capture = None;
        self.latest_diagnostic = None;
    }

    fn take_pending_full_surface_diagnostic(&mut self) -> Option<FullSurfaceInvalidation> {
        self.pending_full_surface_diagnostic.take()
    }

    fn set_latest_capture(&mut self, capture: RetainedChangeEfficiencyCapture) {
        self.latest_capture = Some(capture);
    }

    fn take_latest_capture(&mut self) -> Option<RetainedChangeEfficiencyCapture> {
        self.latest_capture.take()
    }

    fn set_latest_diagnostic(&mut self, artifact: ChangeEfficiencyArtifact) {
        self.latest_diagnostic = Some(artifact);
    }

    fn take_latest_diagnostic(&mut self) -> Option<ChangeEfficiencyArtifact> {
        self.latest_diagnostic.take()
    }

    #[cfg(test)]
    fn note_device_recovery(&mut self) {
        self.device_recovery_pending = true;
        self.snapshot = None;
    }
}

impl Compositor {
    /// Try the retained canonical headless path before falling back to the
    /// ordinary full-frame renderer.  `None` always means the caller must run
    /// the existing path; it never treats a proxy counter as evidence.
    pub(super) fn try_render_retained_headless(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
    ) -> Option<tze_hud_telemetry::FrameTelemetry> {
        self.retained_render_state.begin_observation_frame();
        let (width, height) = surface.size();
        let plan = self
            .retained_render_state
            .plan_headless(scene, width, height);
        let RetainedPlan::Partial(change) = plan else {
            let RetainedPlan::FullFrame { diagnostic } = plan else {
                unreachable!("retained plan has only partial or full-frame variants");
            };
            self.retained_render_state
                .set_pending_full_surface_diagnostic(diagnostic);
            return None;
        };

        let (telemetry, interval_duration_ms) =
            match self.render_retained_text_change(scene, surface, &change.changed) {
                Some(result) => result,
                None => {
                    self.retained_render_state.forget_snapshot();
                    self.retained_render_state
                        .set_pending_full_surface_diagnostic(Some(full_surface_invalidation(
                            FullSurfaceInvalidationReason::UnsupportedRetainedSceneChange,
                            PartialPresentCapability::Supported,
                        )));
                    return None;
                }
            };

        let capture = RetainedChangeEfficiencyCapture::from_observed_runtime(
            self.observed_change_artifact(&change.changed, width, height, interval_duration_ms),
        );
        self.retained_render_state
            .complete_partial(change.next_snapshot);
        self.retained_render_state.set_latest_capture(capture);
        Some(telemetry)
    }

    /// Record the result of an ordinary headless frame.  This seeds (or
    /// refreshes) the private retained snapshot only after the real full frame
    /// was submitted, and preserves structured diagnostics for resize/device
    /// recovery rather than certifying them.
    pub(super) fn observe_full_headless_frame(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
    ) {
        // `render_frame_headless` normally calls the retained planner first,
        // but keep this observer independently fail-closed for callers that
        // invoke a full frame directly.
        self.retained_render_state.clear_observed_evidence();
        let (width, height) = surface.size();
        let diagnostic = self
            .retained_render_state
            .take_pending_full_surface_diagnostic();
        self.retained_render_state
            .remember_full_headless_scene(scene, width, height);
        if let Some(diagnostic) = diagnostic {
            self.retained_render_state
                .set_latest_diagnostic(full_surface_artifact(
                    self.renderer_identity(),
                    scene.visible_tiles().len() as u32,
                    width,
                    height,
                    diagnostic,
                ));
        }
    }

    /// Drain the most recent opaque capture from the canonical fifty-tile,
    /// headless evidence lane.
    ///
    /// This is a Rust-only Layer 3 validation hook.  It does not alter scene,
    /// gRPC, protobuf, reconnect-diff, or WAL contracts. `None` means this
    /// deliberately narrow producer did not observe its canonical scenario;
    /// it does not make a claim about arbitrary scene changes.
    pub fn take_change_efficiency_capture(&mut self) -> Option<RetainedChangeEfficiencyCapture> {
        self.retained_render_state.take_latest_capture()
    }

    /// Drain the most recent structured full-frame diagnostic from the
    /// canonical retained evidence lane. Diagnostics are deliberately raw
    /// artifacts and can never certify proportional rendering.
    pub fn take_change_efficiency_diagnostic(&mut self) -> Option<ChangeEfficiencyArtifact> {
        self.retained_render_state.take_latest_diagnostic()
    }

    fn render_retained_text_change(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
        changed: &CanonicalTextTile,
    ) -> Option<(tze_hud_telemetry::FrameTelemetry, u64)> {
        let frame_start = Instant::now();
        let (width, height) = surface.size();
        let damage = pixel_rect_for_bounds(changed.tile_bounds, width, height)?;
        let current_tile = scene.tiles.get(&changed.tile_id)?;
        let current_node = scene.nodes.get(&changed.node_id)?;
        let NodeData::TextMarkdown(current_text) = &current_node.data else {
            return None;
        };
        if current_text != &changed.text || !current_node.children.is_empty() {
            return None;
        }
        let background_color = self.tile_background_color(current_tile, scene)?;
        let background_vertices = rect_vertices(
            changed.tile_bounds.x,
            changed.tile_bounds.y,
            changed.tile_bounds.width,
            changed.tile_bounds.height,
            width as f32,
            height as f32,
            self.gpu_color_raw(background_color),
        );
        let background_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retained_change_background"),
                contents: bytemuck::cast_slice(&background_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        // The normal render pipeline commit-primes this cache before reaching
        // the retained path. Decline to the full renderer if that invariant is
        // unavailable; this proof lane must not reintroduce lossy markdown or
        // an unbounded parse on its evidence frame.
        let content_key = self.node_key_cache.get(&changed.node_id).copied()?;
        let markdown_cache = self.markdown_cache();
        let parsed = markdown_cache.get_by_key(&content_key)?;
        let text_item = TextItem::from_text_markdown_cached(
            current_text,
            changed.tile_bounds.x,
            changed.tile_bounds.y,
            parsed,
        );
        let inline_backdrops = {
            let text_rasterizer = self.text_rasterizer.as_mut()?;
            text_rasterizer.update_viewport(&self.queue, width, height);
            text_rasterizer
                .prepare_text_items(&self.device, &self.queue, &[text_item])
                .ok()?
        };
        // The canonical lane admits plain text only; inline markdown backdrops
        // would require an extra scoped pass, so fail closed to the full path.
        if !inline_backdrops.is_empty() {
            return None;
        }

        let frame = surface.acquire_frame()?;
        let encode_start = Instant::now();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("retained_change_encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("retained_change_background_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Retain every unaffected pixel from the submitted baseline.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_scissor_rect(damage.x, damage.y, damage.width, damage.height);
            if self.use_opaque_rect_pipeline() {
                pass.set_pipeline(&self.clear_pipeline);
            } else {
                pass.set_pipeline(&self.pipeline);
            }
            pass.set_vertex_buffer(0, background_buffer.slice(..));
            pass.draw(0..background_vertices.len() as u32, 0..1);
        }
        {
            let text_rasterizer = self.text_rasterizer.as_ref()?;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("retained_change_text_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_scissor_rect(damage.x, damage.y, damage.width, damage.height);
            text_rasterizer.render_text_pass(&mut pass).ok()?;
        }
        if let Some(text_rasterizer) = self.text_rasterizer.as_mut() {
            text_rasterizer.trim_atlas();
        }

        // The readback copy preserves HeadlessRuntime's normal contract.  It is
        // not a render encode or texture upload and does not expand the damage.
        surface.copy_to_buffer(&mut encoder);
        let stage6_render_encode_us = encode_start.elapsed().as_micros().max(1) as u64;
        let submit_start = Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        surface.present();
        drop(frame);
        self.device.poll(wgpu::Maintain::Wait);
        let stage7_gpu_submit_us = submit_start.elapsed().as_micros().max(1) as u64;

        let mut telemetry = tze_hud_telemetry::FrameTelemetry::new(self.frame_number);
        telemetry.tile_count = CANONICAL_TILE_COUNT as u32;
        telemetry.node_count = CANONICAL_TILE_COUNT as u32;
        telemetry.active_leases = scene.leases.len() as u32;
        telemetry.tiles_layout_recomputed = 1;
        telemetry.stage6_render_encode_us = stage6_render_encode_us;
        telemetry.stage7_gpu_submit_us = stage7_gpu_submit_us;
        telemetry.frame_time_us = frame_start.elapsed().as_micros().max(1) as u64;
        let interval_duration_ms = frame_start.elapsed().as_millis().max(1) as u64;
        Some((telemetry, interval_duration_ms))
    }

    fn observed_change_artifact(
        &self,
        changed: &CanonicalTextTile,
        width: u32,
        height: u32,
        interval_duration_ms: u64,
    ) -> ChangeEfficiencyArtifact {
        let node = NodeWorkItemId {
            tile_id: changed.tile_id.to_string(),
            node_id: changed.node_id.to_string(),
        };
        let render_plan = RenderPlanWorkItemId {
            tile_id: changed.tile_id.to_string(),
            plan_id: format!("{}:retained-text", changed.node_id),
        };
        let damage = tze_hud_telemetry::DamageWorkItemId {
            tile_id: changed.tile_id.to_string(),
            region_id: format!("{}:bounds", changed.tile_id),
            bounds: pixel_rect_for_bounds(changed.tile_bounds, width, height)
                .expect("canonical planner already validated changed tile damage"),
        };
        ChangeEfficiencyArtifact {
            schema_version: tze_hud_telemetry::CHANGE_EFFICIENCY_SCHEMA_VERSION,
            scenario: EfficiencyScenarioIdentity {
                name: tze_hud_telemetry::ONE_NODE_FIFTY_TILE_SCENARIO_NAME.into(),
                version: tze_hud_telemetry::ONE_NODE_FIFTY_TILE_SCENARIO_VERSION,
            },
            runtime: EfficiencyRuntimeIdentity {
                build: format!("tze_hud_compositor-{}", env!("CARGO_PKG_VERSION")),
                window_mode: EfficiencyWindowMode::Headless,
            },
            pacing: EfficiencyPacingIdentity {
                mode: EfficiencyPacingMode::EventDriven,
                requested_cadence_hz: None,
            },
            renderer: self.renderer_identity(),
            viewport: EfficiencyViewport { width, height },
            constrained_profile: None,
            settling_duration_ms: 0,
            interval_duration_ms,
            status: ChangeMeasurementStatus::Complete,
            measurement_provenance: ChangeMeasurementProvenance::ObservedRetainedRuntime,
            scene_tile_count: CANONICAL_TILE_COUNT as u32,
            closure: InvalidationClosure {
                layout: one_item_category(node.clone()),
                raster: one_item_category(node),
                texture_upload: TextureUploadCategory {
                    closure_items: vec![],
                    actual_work: vec![],
                },
                render_encoding: one_item_category(render_plan),
                composition_damage: one_item_category(damage),
            },
            render_observation: ChangeRenderWorkObservation {
                full_surface_clear_operations: 0,
                full_frame_encode_operations: 0,
                scoped_render_encode_operations: 1,
            },
            // The scoped background and single prepared text item are the two
            // logical draws in this encoder.  The closure's render-plan work
            // remains one item because both belong to the changed tile plan.
            encoded_draw_calls: 2,
            full_surface_invalidation: None,
        }
    }

    fn renderer_identity(&self) -> EfficiencyRendererIdentity {
        EfficiencyRendererIdentity {
            backend: nonempty_identity(&self.adapter_info.backend, "unknown-backend"),
            adapter: nonempty_identity(&self.adapter_info.name, "unknown-adapter"),
            software: self.adapter_info.device_type.eq_ignore_ascii_case("cpu"),
        }
    }
}

fn canonical_snapshot(
    scene: &SceneGraph,
    width: u32,
    height: u32,
) -> Option<CanonicalSceneSnapshot> {
    if width == 0
        || height == 0
        || !scene.zone_registry.active_publishes.is_empty()
        || !scene.widget_registry.active_publishes.is_empty()
        || !scene.overlay.tile_scroll_configs.is_empty()
        || !scene.overlay.tile_scroll_offsets.is_empty()
        || !scene.overlay.displayed_tile_scroll_offsets.is_empty()
        || !scene.overlay.tile_follow_tail_at_tail.is_empty()
        || !scene.overlay.tile_unread_counts.is_empty()
        || !scene.overlay.tile_lifecycle_accents.is_empty()
        || !scene.overlay.tile_composer_interactions.is_empty()
        || !scene.overlay.drag_active_elements.is_empty()
        || scene.overlay.drag_handle_context_menu.is_some()
        || !scene.overlay.portal_surfaces.is_empty()
        || !scene.overlay.tile_font_scale.is_empty()
    {
        return None;
    }

    let visible_tiles = scene.visible_tiles();
    if visible_tiles.len() != CANONICAL_TILE_COUNT || scene.nodes.len() != CANONICAL_TILE_COUNT {
        return None;
    }

    let mut tiles = Vec::with_capacity(CANONICAL_TILE_COUNT);
    for tile in visible_tiles {
        if tile.opacity != 1.0 || !rect_is_inside_viewport(tile.bounds, width, height) {
            return None;
        }
        let root_id = tile.root_node?;
        let node = scene.nodes.get(&root_id)?;
        if !node.children.is_empty() {
            return None;
        }
        let NodeData::TextMarkdown(text) = &node.data else {
            return None;
        };
        if text.content.is_empty()
            || !text.content.is_ascii()
            || text.background.is_some()
            || !text.color_runs.is_empty()
            || !matches!(text.overflow, tze_hud_scene::types::TextOverflow::Clip)
            || !rect_is_inside_tile(text.bounds, tile.bounds)
        {
            return None;
        }
        tiles.push(CanonicalTextTile {
            tile_id: tile.id,
            node_id: root_id,
            tile_bounds: tile.bounds,
            text: text.clone(),
        });
    }
    tiles.sort_by_key(|tile| tile.tile_id);
    if tiles.iter().enumerate().any(|(index, tile)| {
        tiles[index + 1..]
            .iter()
            .any(|other| tile.tile_bounds.intersects(&other.tile_bounds))
    }) {
        return None;
    }
    Some(CanonicalSceneSnapshot {
        viewport: EfficiencyViewport { width, height },
        tiles,
    })
}

fn changed_canonical_tile(
    previous: &CanonicalSceneSnapshot,
    current: &CanonicalSceneSnapshot,
) -> Option<CanonicalTextTile> {
    if previous.tiles.len() != current.tiles.len() {
        return None;
    }
    let mut changed = None;
    for (before, after) in previous.tiles.iter().zip(&current.tiles) {
        if before.tile_id != after.tile_id
            || before.node_id != after.node_id
            || before.tile_bounds != after.tile_bounds
        {
            return None;
        }
        if before.text.content == after.text.content {
            if before.text != after.text {
                return None;
            }
            continue;
        }
        let mut before_with_new_content = before.text.clone();
        before_with_new_content.content = after.text.content.clone();
        if before_with_new_content != after.text
            || !same_ascii_glyph_inventory(&before.text.content, &after.text.content)
            || changed.replace(after.clone()).is_some()
        {
            return None;
        }
    }
    changed
}

fn same_ascii_glyph_inventory(before: &str, after: &str) -> bool {
    let before_glyphs: BTreeSet<_> = before.bytes().collect();
    let after_glyphs: BTreeSet<_> = after.bytes().collect();
    before_glyphs == after_glyphs
}

fn rect_is_inside_viewport(rect: Rect, width: u32, height: u32) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
        && rect.x >= 0.0
        && rect.y >= 0.0
        && rect.x + rect.width <= width as f32
        && rect.y + rect.height <= height as f32
}

fn rect_is_inside_tile(node_bounds: Rect, tile_bounds: Rect) -> bool {
    node_bounds.x.is_finite()
        && node_bounds.y.is_finite()
        && node_bounds.width.is_finite()
        && node_bounds.height.is_finite()
        && node_bounds.width > 0.0
        && node_bounds.height > 0.0
        && node_bounds.x >= 0.0
        && node_bounds.y >= 0.0
        && node_bounds.x + node_bounds.width <= tile_bounds.width
        && node_bounds.y + node_bounds.height <= tile_bounds.height
}

fn pixel_rect_for_bounds(bounds: Rect, width: u32, height: u32) -> Option<PixelRect> {
    if !rect_is_inside_viewport(bounds, width, height) {
        return None;
    }
    let left = bounds.x.floor() as u32;
    let top = bounds.y.floor() as u32;
    let right = (bounds.x + bounds.width).ceil() as u32;
    let bottom = (bounds.y + bounds.height).ceil() as u32;
    let pixel_rect = PixelRect {
        x: left,
        y: top,
        width: right.checked_sub(left)?,
        height: bottom.checked_sub(top)?,
    };
    (pixel_rect.width > 0 && pixel_rect.height > 0).then_some(pixel_rect)
}

fn one_item_category<T>(identity: T) -> InvalidationCategory<T>
where
    T: Clone,
{
    InvalidationCategory {
        closure_items: vec![ClosureWorkItem {
            identity: identity.clone(),
            dependency_reason: InvalidationDependencyReason::DirectChange,
        }],
        actual_work: vec![ActualWorkItem {
            identity,
            operations: 1,
        }],
    }
}

fn full_surface_invalidation(
    reason: FullSurfaceInvalidationReason,
    partial_present_capability: PartialPresentCapability,
) -> FullSurfaceInvalidation {
    FullSurfaceInvalidation {
        reason,
        partial_present_capability,
    }
}

fn full_surface_artifact(
    renderer: EfficiencyRendererIdentity,
    scene_tile_count: u32,
    width: u32,
    height: u32,
    full_surface_invalidation: FullSurfaceInvalidation,
) -> ChangeEfficiencyArtifact {
    let surface_damage = tze_hud_telemetry::DamageWorkItemId {
        tile_id: "runtime-surface".into(),
        region_id: "full-surface".into(),
        bounds: PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        },
    };
    ChangeEfficiencyArtifact {
        schema_version: tze_hud_telemetry::CHANGE_EFFICIENCY_SCHEMA_VERSION,
        scenario: EfficiencyScenarioIdentity {
            name: tze_hud_telemetry::ONE_NODE_FIFTY_TILE_SCENARIO_NAME.into(),
            version: tze_hud_telemetry::ONE_NODE_FIFTY_TILE_SCENARIO_VERSION,
        },
        runtime: EfficiencyRuntimeIdentity {
            build: format!("tze_hud_compositor-{}", env!("CARGO_PKG_VERSION")),
            window_mode: EfficiencyWindowMode::Headless,
        },
        pacing: EfficiencyPacingIdentity {
            mode: EfficiencyPacingMode::EventDriven,
            requested_cadence_hz: None,
        },
        renderer,
        viewport: EfficiencyViewport { width, height },
        constrained_profile: None,
        settling_duration_ms: 0,
        interval_duration_ms: 1,
        status: ChangeMeasurementStatus::Complete,
        measurement_provenance: ChangeMeasurementProvenance::ObservedFullFrameRuntime,
        scene_tile_count,
        closure: InvalidationClosure {
            layout: InvalidationCategory {
                closure_items: vec![],
                actual_work: vec![],
            },
            raster: InvalidationCategory {
                closure_items: vec![],
                actual_work: vec![],
            },
            texture_upload: TextureUploadCategory {
                closure_items: vec![],
                actual_work: vec![],
            },
            render_encoding: InvalidationCategory {
                closure_items: vec![],
                actual_work: vec![],
            },
            composition_damage: one_item_category(surface_damage),
        },
        render_observation: ChangeRenderWorkObservation {
            full_surface_clear_operations: 1,
            full_frame_encode_operations: 1,
            scoped_render_encode_operations: 0,
        },
        encoded_draw_calls: 0,
        full_surface_invalidation: Some(full_surface_invalidation),
    }
}

fn nonempty_identity(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.into()
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> EfficiencyRendererIdentity {
        EfficiencyRendererIdentity {
            backend: "test-backend".into(),
            adapter: "test-adapter".into(),
            software: true,
        }
    }

    #[test]
    fn full_surface_reasons_are_structured_non_passing_diagnostics() {
        for (reason, capability) in [
            (
                FullSurfaceInvalidationReason::Resize,
                PartialPresentCapability::Supported,
            ),
            (
                FullSurfaceInvalidationReason::DeviceRecovery,
                PartialPresentCapability::Supported,
            ),
            (
                FullSurfaceInvalidationReason::UnsupportedPartialPresentBackend,
                PartialPresentCapability::Unsupported,
            ),
            (
                FullSurfaceInvalidationReason::UnsupportedRetainedSceneChange,
                PartialPresentCapability::Supported,
            ),
        ] {
            let artifact = full_surface_artifact(
                renderer(),
                CANONICAL_TILE_COUNT as u32,
                1_000,
                500,
                full_surface_invalidation(reason.clone(), capability),
            );
            let report = artifact.validate();
            assert!(!report.passed, "{report:#?}");
            assert_eq!(
                report.status,
                tze_hud_telemetry::ChangeEfficiencyValidationStatus::DiagnosticFullSurface,
                "{report:#?}"
            );
            assert_eq!(artifact.full_surface_invalidation.unwrap().reason, reason);
        }
    }

    #[test]
    fn new_observation_clears_prior_frame_capture_and_diagnostic() {
        let mut state = RetainedRenderState::default();
        let mut prior_capture_artifact = full_surface_artifact(
            renderer(),
            CANONICAL_TILE_COUNT as u32,
            1_000,
            500,
            full_surface_invalidation(
                FullSurfaceInvalidationReason::SurfaceCreation,
                PartialPresentCapability::Supported,
            ),
        );
        prior_capture_artifact.measurement_provenance =
            ChangeMeasurementProvenance::ObservedRetainedRuntime;
        state.set_latest_capture(RetainedChangeEfficiencyCapture::from_observed_runtime(
            prior_capture_artifact,
        ));
        state.set_latest_diagnostic(full_surface_artifact(
            renderer(),
            CANONICAL_TILE_COUNT as u32,
            1_000,
            500,
            full_surface_invalidation(
                FullSurfaceInvalidationReason::Resize,
                PartialPresentCapability::Supported,
            ),
        ));
        state.set_pending_full_surface_diagnostic(Some(full_surface_invalidation(
            FullSurfaceInvalidationReason::DeviceRecovery,
            PartialPresentCapability::Supported,
        )));

        state.begin_observation_frame();

        assert!(state.take_latest_capture().is_none());
        assert!(state.take_latest_diagnostic().is_none());
        assert!(state.take_pending_full_surface_diagnostic().is_none());
    }

    #[test]
    fn device_recovery_invalidates_the_private_snapshot() {
        let mut state = RetainedRenderState {
            snapshot: Some(CanonicalSceneSnapshot {
                viewport: EfficiencyViewport {
                    width: 1_000,
                    height: 500,
                },
                tiles: vec![],
            }),
            ..RetainedRenderState::default()
        };

        state.note_device_recovery();

        assert!(state.snapshot.is_none());
        assert!(state.device_recovery_pending);
    }
}
