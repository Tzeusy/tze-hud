//! Video surface methods for the compositor.
//!
//! Moved from `renderer/mod.rs` banner `// ─── Video surface media-plane API (v2 preview) ───`
//! and the video frame texture / widget renderer clusters (formerly ~L1235–1896 pre-split),
//! by Step R-3 of the renderer module split (hud-fgryk).  No logic was changed;
//! only visibility modifiers were added where Rust's module-privacy rules
//! require them (listed in the PR body).

use tze_hud_scene::DegradationLevel;
use tze_hud_scene::graph::SceneGraph;

use crate::widget::WidgetRenderer;

#[cfg(feature = "v2_preview")]
use tze_hud_scene::types::*;

#[cfg(feature = "v2_preview")]
use crate::pipeline::textured_rect_vertices;

#[cfg(feature = "v2_preview")]
use super::image_cache::ImageTextureEntry;

#[cfg(feature = "v2_preview")]
use super::draw_cmds::VideoFrameDrawCmd;

// ─── Video surface media-plane API (v2 preview) ───────────────────────────

impl super::Compositor {
    /// Deliver a media-plane event to a `VideoSurfaceRef` surface.
    ///
    /// The runtime calls this when the media pipeline signals a state change
    /// (admitted, frame decoded, media dropped, reconnected, close, revoke).
    ///
    /// In v1 builds (no `v2_preview` feature) the underlying
    /// [`crate::video_surface::VideoSurfaceMap`] is a zero-cost stub; this
    /// method is still callable and is a no-op.
    ///
    /// # B11 contract (signoff-packet.md §B11)
    ///
    /// When the runtime delivers `MediaEvent::MediaDropped` for a surface,
    /// the compositor transitions that surface to `Paused` state.  On the
    /// next rendered frame, [`super::Compositor::video_render_state`] returns
    /// `VideoRenderState::LastFrameWithBadge`, instructing the render path to
    /// draw the last decoded frame with a disconnection-badge overlay.  The
    /// session (lease + control path) is unaffected.
    #[cfg(feature = "v2_preview")]
    pub fn handle_media_event(
        &mut self,
        surface_id: tze_hud_scene::types::SceneId,
        event: &crate::video_surface::MediaEvent,
    ) {
        self.video_surfaces.handle(surface_id, event);
    }

    /// Query the render state for a video surface.
    ///
    /// Returns [`crate::video_surface::VideoRenderState::Placeholder`] if
    /// the surface is not tracked (unknown `SceneId`) or in a v1 build.
    pub fn video_render_state(
        &self,
        surface_id: &tze_hud_scene::types::SceneId,
    ) -> crate::video_surface::VideoRenderState {
        self.video_surfaces.render_state_for(surface_id)
    }

    /// Remove video surface entries that have reached a terminal state.
    ///
    /// Called automatically by every render path once every 60 frames
    /// (approximately once per second at 60 Hz).  Callers may also invoke
    /// this directly (e.g. after delivering a `Close` / `Revoke` event) to
    /// reclaim memory without waiting for the next scheduled tick.
    ///
    /// Under `v2_preview`, also evicts any GPU textures in `video_frame_cache`
    /// whose surface has transitioned to a terminal state.  Surfaces in
    /// `Placeholder` state (unknown surface_id) are treated as evictable
    /// because they no longer have an active entry in `video_surfaces`.
    pub fn prune_terminal_video_surfaces(&mut self) {
        self.video_surfaces.prune_terminal();
        // v2_preview: after pruning the state-machine map, evict GPU textures
        // whose surface is no longer tracked (render_state_for returns
        // Placeholder for unknown IDs) or has reached a Closed state.
        #[cfg(feature = "v2_preview")]
        {
            use crate::video_surface::VideoRenderState;
            self.video_frame_cache.retain(|sid, _| {
                matches!(
                    self.video_surfaces.render_state_for(sid),
                    VideoRenderState::Streaming | VideoRenderState::LastFrameWithBadge
                )
            });
        }
    }

    // ─── Video frame texture upload (v2 media plane) ──────────────────────────

    /// Upload a decoded [`crate::video_surface::VideoFrame`] to a wgpu texture
    /// and cache the bind group for use in the next render pass.
    ///
    /// Reuses the existing `texture_rect_pipeline` and
    /// `texture_rect_bind_group_layout` so no new GPU pipeline is required.
    /// The texture format is `Rgba8UnormSrgb`, matching the image-texture
    /// pipeline (source frames are sRGB-encoded RGBA8 from the decode path).
    ///
    /// # Upload policy
    ///
    /// Each call **replaces** any previously cached texture for this
    /// `surface_id`.  The old `wgpu::Texture` is dropped immediately (wgpu
    /// reference-counts GPU resources; the GPU will not free the memory until
    /// any in-flight draw commands referencing the old bind group complete).
    ///
    /// # Error conditions
    ///
    /// Returns `false` and logs a warning if:
    /// - `frame.rgba.len() != frame.width * frame.height * 4` (corrupt frame)
    /// - `frame.width == 0 || frame.height == 0` (degenerate frame)
    ///
    /// On validation failure the previous cached texture (if any) is kept
    /// intact so the last good frame continues to be displayed.
    ///
    /// # ENV gate
    ///
    /// This method is always callable; the caller (runtime or integration test)
    /// decides whether to invoke it.  The `TZE_HUD_SYNTHETIC_DECODE=1`
    /// environment variable activates the synthetic pipeline wiring in the
    /// runtime test harness; regular render frames are unaffected.
    ///
    /// # v2_preview gate
    ///
    /// Available only when the `v2_preview` feature is active.  In v1 builds
    /// this method does not exist; the render path always shows the dark
    /// placeholder quad.
    ///
    /// # Colour-space contract — sRGB vs BT.709 (hud-ndo7o)
    ///
    /// The texture is created with `wgpu::TextureFormat::Rgba8UnormSrgb`, which
    /// causes the GPU sampler to apply the **sRGB EOTF** (gamma ≈ 2.2 curve) to
    /// every texel on read.  However, GStreamer's
    /// `videoconvert ! video/x-raw,format=RGBA` pipeline outputs pixels encoded
    /// with the **BT.709 transfer function**, which is close to sRGB but not
    /// identical (BT.709 uses a linear segment below 0.018 and a power exponent
    /// of 0.45, versus sRGB's 0.0031308 breakpoint and 1/2.4 exponent).  The
    /// result is a mild gamma mismatch — sampled colours will appear slightly
    /// brighter than a reference display would show.
    ///
    /// This approximation is acceptable for initial integration.  Two forward-fix
    /// paths are available:
    ///
    /// 1. **GStreamer side** — insert a BT.709 → sRGB gamma conversion in the
    ///    decode stage (e.g. a `videobalance` element or custom GLSL kernel)
    ///    before the appsink, so the bytes written into `VideoFrame::rgba` are
    ///    already sRGB-encoded.
    /// 2. **Compositor side** — switch to `Rgba8Unorm` (bypasses the GPU sRGB
    ///    EOTF) and apply the BT.709 inverse EOTF in a fragment shader, keeping
    ///    the data in linear light for downstream compositing.
    ///
    /// See also: `crates/tze_hud_runtime/src/gst_decode_pipeline.rs` module-level
    /// rustdoc, which describes the same mismatch from the decode-pipeline side.
    /// Tracking: hud-ndo7o.
    #[cfg(feature = "v2_preview")]
    pub fn upload_video_frame(
        &mut self,
        surface_id: tze_hud_scene::types::SceneId,
        frame: &crate::video_surface::VideoFrame,
    ) -> bool {
        use crate::video_surface::{MediaEvent, VideoRenderState};

        match self.video_surfaces.render_state_for(&surface_id) {
            VideoRenderState::Closed => {
                self.evict_video_frame_texture(&surface_id);
                tracing::warn!(
                    surface_id = %surface_id,
                    "upload_video_frame: terminal surface rejected"
                );
                return false;
            }
            VideoRenderState::Placeholder => {}
            VideoRenderState::Streaming | VideoRenderState::LastFrameWithBadge => {}
        }

        // Validate dimensions — fail fast with a diagnostic message.
        if frame.width == 0 || frame.height == 0 {
            tracing::warn!(
                surface_id = %surface_id,
                width = frame.width,
                height = frame.height,
                "upload_video_frame: degenerate frame dimensions — skipped"
            );
            return false;
        }
        let expected_bytes = (frame.width as usize)
            .saturating_mul(frame.height as usize)
            .saturating_mul(4);
        if frame.rgba.len() != expected_bytes {
            tracing::warn!(
                surface_id = %surface_id,
                expected = expected_bytes,
                actual = frame.rgba.len(),
                "upload_video_frame: byte count mismatch — skipped"
            );
            return false;
        }

        // Create GPU texture (TEXTURE_BINDING | COPY_DST so wgpu can blit
        // the CPU bytes in and the shader can sample from it).
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video_frame_tex"),
            size: wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // RGBA8 sRGB matches the image-texture pipeline so no shader
            // changes are needed.
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload CPU-side RGBA8 bytes into the texture.
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );

        // Build a bind group reusing the shared bind group layout and sampler.
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video_frame_bg"),
            layout: &self.texture_rect_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
                },
            ],
        });

        tracing::debug!(
            surface_id = %surface_id,
            width = frame.width,
            height = frame.height,
            presented_at_us = frame.presented_at_us,
            "upload_video_frame: texture uploaded to GPU"
        );

        // Replace any existing cache entry (old texture is dropped here; wgpu
        // ref-counts the GPU resource until any in-flight draw completes).
        self.video_frame_cache.insert(
            surface_id,
            ImageTextureEntry {
                _texture: texture,
                bind_group,
                width: frame.width,
                height: frame.height,
            },
        );
        if matches!(
            self.video_surfaces.render_state_for(&surface_id),
            VideoRenderState::Placeholder
        ) {
            self.video_surfaces.ensure(surface_id);
            self.video_surfaces
                .handle(surface_id, &MediaEvent::Admitted);
        }
        self.video_surfaces
            .handle_decoded_frame(surface_id, frame.clone());

        true
    }

    /// Evict the GPU texture for a video surface that has reached a terminal
    /// state or been revoked.
    ///
    /// Drops the `wgpu::Texture` and its associated bind group.  The wgpu
    /// runtime will release the GPU allocation once no in-flight command
    /// buffers reference the old bind group.
    ///
    /// No-op if no texture is cached for `surface_id`.
    #[cfg(feature = "v2_preview")]
    pub fn evict_video_frame_texture(&mut self, surface_id: &tze_hud_scene::types::SceneId) {
        if self.video_frame_cache.remove(surface_id).is_some() {
            tracing::debug!(
                surface_id = %surface_id,
                "evict_video_frame_texture: texture evicted"
            );
        }
    }

    /// Run [`Self::prune_terminal_video_surfaces`] every 60 frames.
    ///
    /// Called at the end of each render path.  The modulo gate amortises the
    /// cost of the HashMap scan across frames; once per second at 60 Hz is
    /// frequent enough to prevent unbounded accumulation of closed/revoked
    /// entries without adding measurable per-frame overhead.
    #[inline]
    pub(super) fn maybe_prune_terminal_video_surfaces(&mut self) {
        if self.frame_number % 60 == 0 {
            self.prune_terminal_video_surfaces();
        }
    }

    // ─── Video frame draw commands (v2 media plane) ───────────────────────────

    /// Collect draw commands for all `VideoSurfaceRef` zones that have a cached
    /// GPU texture (i.e. `upload_video_frame` has been called for them and the
    /// surface is in `Streaming` or `LastFrameWithBadge` state).
    ///
    /// The returned commands are consumed by [`super::Compositor::encode_video_frame_pass`]
    /// which renders them after the main color pass so the texture overwrites
    /// the dark placeholder quad emitted by `render_zone_content`.
    ///
    /// Returns an empty `Vec` in v1 builds (feature gate handled at call site).
    #[cfg(feature = "v2_preview")]
    pub(super) fn collect_video_frame_cmds(
        &self,
        scene: &SceneGraph,
        sw: f32,
        sh: f32,
    ) -> Vec<VideoFrameDrawCmd> {
        use crate::video_surface::VideoRenderState;

        let mut cmds = Vec::new();
        let Some(publishes) = scene
            .zone_registry
            .active_publishes
            .get(tze_hud_scene::config::APPROVED_MEDIA_ZONE)
        else {
            return cmds;
        };
        if publishes.is_empty() {
            return cmds;
        }

        // Check whether any publication in this zone is a VideoSurfaceRef.
        let surface_id = publishes.iter().rev().find_map(|record| {
            if let ZoneContent::VideoSurfaceRef(sid) = &record.content {
                Some(*sid)
            } else {
                None
            }
        });
        let surface_id = match surface_id {
            Some(sid) => sid,
            None => return cmds,
        };

        // Only draw the real texture when a GPU texture is cached (upload_video_frame
        // was called) AND the surface is in a frame-visible render state.
        let render_state = self.video_surfaces.render_state_for(&surface_id);
        let has_texture = self.video_frame_cache.contains_key(&surface_id);
        if !has_texture {
            return cmds; // no frame uploaded yet — dark placeholder from render_zone_content
        }
        match render_state {
            VideoRenderState::Streaming | VideoRenderState::LastFrameWithBadge => {}
            VideoRenderState::Placeholder | VideoRenderState::Closed => return cmds,
        }

        // Resolve zone geometry.
        let zone_def = match scene
            .zone_registry
            .zones
            .get(tze_hud_scene::config::APPROVED_MEDIA_ZONE)
        {
            Some(z) => z,
            None => return cmds,
        };
        let (x, y, w, h) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);
        if w <= 0.0 || h <= 0.0 {
            return cmds;
        }

        cmds.push(VideoFrameDrawCmd {
            surface_id,
            x,
            y,
            w,
            h,
            tint: [1.0, 1.0, 1.0, 1.0],
        });
        cmds
    }

    /// Render decoded video frames for all collected [`VideoFrameDrawCmd`]s.
    ///
    /// Runs as a separate render pass after the main color + image pass so
    /// the video texture overwrites the dark placeholder quad.  Uses
    /// `LoadOp::Load` to preserve previously drawn content.
    ///
    /// No-op if `cmds` is empty (avoids a GPU pass entirely).
    #[cfg(feature = "v2_preview")]
    pub(super) fn encode_video_frame_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_view: &wgpu::TextureView,
        cmds: &[VideoFrameDrawCmd],
        sw: f32,
        sh: f32,
    ) {
        if cmds.is_empty() {
            return;
        }

        use wgpu::util::DeviceExt;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video_frame_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame_view,
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

        pass.set_pipeline(&self.texture_rect_pipeline);

        for cmd in cmds {
            let entry = match self.video_frame_cache.get(&cmd.surface_id) {
                Some(e) => e,
                None => continue, // race: evicted since collection
            };

            let verts = textured_rect_vertices(
                cmd.x,
                cmd.y,
                cmd.w,
                cmd.h,
                sw,
                sh,
                [0.0, 0.0, 1.0, 1.0], // full UV rect
                cmd.tint,
            );
            let vertex_buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("video_quad_buf"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });

            pass.set_bind_group(0, &entry.bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buf.slice(..));
            pass.draw(0..6, 0..1);
        }
    }

    // ─── Widget renderer ──────────────────────────────────────────────────────

    /// Initialize (or re-initialize) the widget renderer for the given surface format.
    ///
    /// Must be called once before widget textures can be composited. For headless
    /// compositors, `format` should be `Rgba8UnormSrgb`. For windowed compositors,
    /// use the negotiated swapchain format.
    ///
    /// Calling this multiple times replaces the existing renderer (e.g. on surface
    /// reconfiguration or format change). Any cached textures are discarded.
    pub fn init_widget_renderer(&mut self, format: wgpu::TextureFormat) {
        let mut renderer = WidgetRenderer::new(&self.device, format);
        if let Some(ledger) = &self.resident_ledger {
            renderer.set_resident_ledger(ledger.clone());
        }
        self.widget_renderer = Some(renderer);
        tracing::debug!(format = ?format, "widget renderer initialized");
    }

    pub fn set_resident_ledger(&mut self, ledger: tze_hud_resource::ResidentLedger) {
        if let Some(renderer) = &mut self.widget_renderer {
            renderer.set_resident_ledger(ledger.clone());
        }
        self.resident_ledger = Some(ledger);
    }

    /// Get a mutable reference to the widget renderer, if initialized.
    pub fn widget_renderer_mut(&mut self) -> Option<&mut WidgetRenderer> {
        self.widget_renderer.as_mut()
    }

    /// Get a reference to the widget renderer, if initialized.
    pub fn widget_renderer(&self) -> Option<&WidgetRenderer> {
        self.widget_renderer.as_ref()
    }

    /// Ensure widget instances have up-to-date cached textures for all widget
    /// instances in the registry.
    ///
    /// For each widget instance:
    /// - If no texture entry exists (first frame), rasterizes with default params.
    /// - If the instance has a `dirty` flag set, re-rasterizes with current params.
    /// - If an animation is active, resolves interpolated params and re-rasterizes.
    ///
    /// Under degradation level [`DegradationLevel::Significant`] or higher, active
    /// transitions are snapped to their final values immediately, reducing
    /// re-rasterization to at most once per parameter change during transitions.
    ///
    /// This should be called once per frame before `render_frame`.
    pub fn sync_widget_textures(
        &mut self,
        scene: &SceneGraph,
        degradation_level: DegradationLevel,
    ) {
        let wr = match &mut self.widget_renderer {
            Some(r) => r,
            None => return,
        };

        let registry = &scene.widget_registry;

        // Collect instances that need texture updates. Widgets without active
        // publications are not visible; clear their cached texture so clear/TTL
        // removal takes effect on the next frame instead of rendering defaults.
        let instance_names: Vec<String> = registry.instances.keys().cloned().collect();

        // Reclaim every safely inactive texture before admitting any new or
        // replacement raster for this frame. Active publications form the
        // current-frame guard set and are never evicted by this pass.
        for instance_name in &instance_names {
            let has_active_publication = registry
                .active_publishes
                .get(instance_name)
                .is_some_and(|publishes| !publishes.is_empty());
            if !has_active_publication {
                wr.remove_texture(instance_name);
            }
        }

        for instance_name in instance_names {
            let has_active_publication = registry
                .active_publishes
                .get(&instance_name)
                .is_some_and(|publishes| !publishes.is_empty());
            if !has_active_publication {
                continue;
            }

            let instance = match registry.instances.get(&instance_name) {
                Some(i) => i.clone(),
                None => continue,
            };
            let def = match registry.definitions.get(&instance.widget_type_name) {
                Some(d) => d.clone(),
                None => continue,
            };

            // Determine pixel geometry from the instance's geometry policy.
            // Fall back to a sensible default if not set.
            let (pw, ph) =
                super::resolve_widget_pixel_size(&instance, &def, self.width, self.height);
            if pw == 0 || ph == 0 {
                continue;
            }

            // Check if this instance needs an initial texture (no entry yet).
            let needs_initial = wr.texture_entry(&instance_name).is_none();

            // Resolve animated or static params, applying degradation-aware snapping.
            let current_params = &instance.current_params;
            let (effective_params, still_animating) =
                wr.resolve_animated_params(&instance_name, current_params, degradation_level);

            let params_changed = wr
                .texture_entry(&instance_name)
                .map(|e| e.last_rendered_params != effective_params)
                .unwrap_or(false);

            let dirty = needs_initial
                || still_animating
                || params_changed
                || wr
                    .texture_entry(&instance_name)
                    .map(|e| e.dirty)
                    .unwrap_or(false);

            if dirty {
                wr.rasterize_and_upload(
                    &self.device,
                    &self.queue,
                    &instance_name,
                    &def,
                    &effective_params,
                    pw,
                    ph,
                );
                // Record what params were rendered so we can detect future changes.
                if let Some(entry) = wr.texture_entry_mut(&instance_name) {
                    entry.last_rendered_params = effective_params;
                    entry.dirty = false;
                }
            }
        }
    }
}
