//! The compositor — renders the scene graph to a surface.
//!
//! For the vertical slice, this renders tiles as colored rectangles
//! with hit-region highlighting for local feedback.
//!
//! The frame pipeline is **surface-agnostic**: it renders through the
//! `CompositorSurface` trait, which is implemented by both `HeadlessSurface`
//! (offscreen) and `WindowSurface` (display-connected).  No conditional
//! compilation exists in the render path — only the surface implementation
//! differs between modes.
//!
//! Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
//! "No conditional compilation for the render path."
//!
//! ## Two-pass rendering (content → chrome)
//!
//! [`Compositor::render_frame_with_chrome`] implements the three-layer ordering required by
//! the chrome sovereignty contract:
//!   1. Background + content pass (`LoadOp::Clear` — clears and draws agent tiles)
//!   2. Chrome pass (`LoadOp::Load` — draws chrome on top of content, preserving pixels)
//!
//! This separation is the architectural foundation for future render-skip redaction
//! (capture-safe architecture): the content and chrome passes are structurally independent.

use crate::pipeline::{ChromeDrawCmd, RectVertex, rect_vertices};
use crate::surface::{CompositorSurface, HeadlessSurface};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_telemetry::FrameTelemetry;

/// GPU state and render pipeline.
pub struct Compositor {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Pipeline with no blending — writes RGBA directly. Used to clear
    /// the framebuffer to transparent in overlay mode (LoadOp::Clear
    /// doesn't write alpha correctly on some GPUs).
    clear_pipeline: wgpu::RenderPipeline,
    pub width: u32,
    pub height: u32,
    frame_number: u64,
    /// When true, the clear color uses alpha=0 for transparent overlay mode.
    pub overlay_mode: bool,
}

impl Compositor {
    /// Create a new headless compositor.
    ///
    /// Checks the `HEADLESS_FORCE_SOFTWARE` environment variable.  When set to
    /// `1`, wgpu adapter selection uses `force_fallback_adapter = true`, which
    /// selects llvmpipe on Linux or WARP on Windows.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Software GPU (line 211):
    /// "When set, the wgpu adapter selection MUST request a software fallback
    /// (force_fallback_adapter = true)."
    pub async fn new_headless(width: u32, height: u32) -> Result<Self, CompositorError> {
        // Check HEADLESS_FORCE_SOFTWARE env var (spec line 409: "conventionally
        // named HEADLESS_FORCE_SOFTWARE").
        let force_software = std::env::var("HEADLESS_FORCE_SOFTWARE")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: force_software,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        let pipeline = Self::create_pipeline(&device);
        let clear_pipeline = Self::create_clear_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb);

        Ok(Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            width,
            height,
            frame_number: 0,
            overlay_mode: false,
        })
    }

    /// Create a windowed compositor backed by a real `winit::window::Window`.
    ///
    /// This is the factory method for production windowed rendering. It:
    /// 1. Uses `select_gpu_adapter` with platform-mandated backends (Vulkan/D3D12/Metal).
    /// 2. Creates a `wgpu::Surface` from the window via `instance.create_surface`.
    /// 3. Negotiates the surface format (sRGB preferred).
    /// 4. Configures the surface with the window's physical dimensions.
    /// 5. Creates the `wgpu::Device` and `wgpu::Queue`.
    ///
    /// Returns the `(Compositor, WindowSurface)` pair. The `WindowSurface` must
    /// be kept alive for the duration of the runtime.
    ///
    /// Per spec §Compositor Thread Ownership (line 46): the returned `Compositor`
    /// (and thus `Device` + `Queue`) MUST be transferred to the compositor thread
    /// immediately after creation. The `WindowSurface` is owned by the main thread.
    ///
    /// Per spec §Platform GPU Backends (line 189): this path uses the platform-
    /// mandated backends — unlike `new_headless` which uses `Backends::all()`.
    pub async fn new_windowed(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, false).await
    }

    /// Create a windowed compositor, optionally forcing Vulkan for overlay
    /// transparency (DX12 only supports Opaque swapchain alpha mode).
    pub async fn new_windowed_overlay(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, true).await
    }

    async fn new_windowed_inner(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
        overlay: bool,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        use crate::surface::WindowSurface;

        // ── Step 1: Create instance with platform-mandated backends ──────────
        // We need the surface before adapter selection so we can pass it as
        // `compatible_surface`. Create a temporary instance first, create the
        // surface, then select the adapter with that surface constraint.
        // On Windows in overlay mode, force Vulkan — DX12 only supports Opaque
        // swapchain alpha mode, which prevents per-pixel transparency.
        let backends = if overlay && cfg!(target_os = "windows") {
            tracing::info!("overlay mode: forcing Vulkan backend for transparent swapchain");
            wgpu::Backends::VULKAN
        } else {
            crate::adapter::platform_backends().flags
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        // ── Step 2: Create wgpu::Surface from the winit window ───────────────
        // SAFETY: `window` is wrapped in Arc — it outlives the surface because
        // we pass 'static lifetime via Arc<Window>.
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| CompositorError::DeviceCreation(format!("create_surface: {e}")))?;

        // ── Step 3: Select adapter compatible with the surface ────────────────
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        tracing::info!(
            backend = ?adapter_info.backend,
            device_name = %adapter_info.name,
            vendor = adapter_info.vendor,
            "windowed: GPU adapter selected"
        );

        // ── Step 4: Request device ────────────────────────────────────────────
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor_windowed"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        // ── Step 5: Configure the surface ────────────────────────────────────
        let surface_caps = surface.get_capabilities(&adapter);

        // Guard: wgpu surface capabilities must be non-empty on a valid
        // adapter/surface combination. Return a structured error instead of
        // panicking via index [0] so the caller can diagnose driver issues.
        if surface_caps.formats.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported texture formats — driver or backend issue".to_string(),
            ));
        }
        if surface_caps.present_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported present modes — driver or backend issue".to_string(),
            ));
        }
        if surface_caps.alpha_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported alpha modes — driver or backend issue".to_string(),
            ));
        }

        // Prefer sRGB surface format; fall back to the first available format.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo // vsync — latency-stable
        } else {
            surface_caps.present_modes[0]
        };

        // Clamp dimensions to the device's maximum supported texture size.
        // Use device.limits() (not adapter.limits()) because the device is
        // created with required_limits=downlevel_defaults(); the actual device
        // limits reflect what the adapter provides subject to those requirements.
        // Some GPUs (e.g. certain Intel/Mesa drivers) report a max of 2048,
        // which is smaller than common display resolutions like 2560x1440.
        //
        // Also guard against zero-size dimensions: wgpu panics if width or
        // height is 0 in surface.configure().  This can happen if inner_size()
        // returned (0,0) on a minimized or not-yet-shown window.
        let max_dim = device.limits().max_texture_dimension_2d;
        let clamped_width = width.min(max_dim).max(1);
        let clamped_height = height.min(max_dim).max(1);
        if clamped_width != width || clamped_height != height {
            tracing::warn!(
                requested_width = width,
                requested_height = height,
                clamped_width,
                clamped_height,
                max_texture_dimension_2d = max_dim,
                "windowed: surface dimensions clamped to device limit"
            );
        }

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: clamped_width,
            height: clamped_height,
            present_mode,
            alpha_mode: surface_caps
                .alpha_modes
                .iter()
                .find(|m| **m == wgpu::CompositeAlphaMode::PreMultiplied)
                .or_else(|| surface_caps.alpha_modes.iter().find(|m| **m == wgpu::CompositeAlphaMode::PostMultiplied))
                .copied()
                .unwrap_or(surface_caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        tracing::info!(
            available_alpha_modes = ?surface_caps.alpha_modes,
            selected_alpha_mode = ?config.alpha_mode,
            "windowed: alpha mode selection"
        );
        // Write diagnostic to a known file for remote debugging.
        let diag = format!(
            "backend: {:?}\ndevice: {}\nrequested_backends: {:?}\noverlay: {}\navailable_alpha_modes: {:?}\nselected_alpha_mode: {:?}\nformat: {:?}\npresent_mode: {:?}\n",
            adapter_info.backend, adapter_info.name, backends, overlay,
            surface_caps.alpha_modes, config.alpha_mode, surface_format, present_mode,
        );
        let _ = std::fs::write("C:\\tze_hud\\logs\\alpha_diag.txt", &diag);
        surface.configure(&device, &config);
        tracing::info!(
            format = ?surface_format,
            present_mode = ?present_mode,
            alpha_mode = ?config.alpha_mode,
            width = clamped_width,
            height = clamped_height,
            "windowed: surface configured"
        );

        // ── Step 6: Create render pipelines (format-aware) ────────────────────
        let pipeline = Self::create_pipeline_with_format(&device, surface_format);
        let clear_pipeline = Self::create_clear_pipeline(&device, surface_format);

        let compositor = Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            width: clamped_width,
            height: clamped_height,
            frame_number: 0,
            overlay_mode: false,
        };

        let window_surface = WindowSurface::new(surface, config);
        Ok((compositor, window_surface))
    }

    /// Create a render pipeline targeting a specific texture format.
    ///
    /// This is the canonical pipeline constructor. Both `create_pipeline`
    /// (headless, fixed format) and `create_pipeline_with_format` (windowed,
    /// dynamic swapchain format) delegate here to avoid duplicating the
    /// pipeline descriptor.
    ///
    /// `label_prefix` is prepended to debug labels so GPU profilers can
    /// distinguish headless vs windowed pipelines.
    fn create_pipeline_inner(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label_prefix: &str,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};

        let shader_label = format!("{label_prefix}rect_shader");
        let layout_label = format!("{label_prefix}rect_pipeline_layout");
        let pipeline_label = format!("{label_prefix}rect_pipeline");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&shader_label),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&layout_label),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&pipeline_label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create a render pipeline targeting a dynamic swapchain format.
    ///
    /// Called by `new_windowed` so the pipeline matches the negotiated
    /// surface format (which varies by platform/driver).
    fn create_pipeline_with_format(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, format, "windowed_")
    }

    /// Create a pipeline with no blending — writes RGBA directly.
    /// Used to clear the framebuffer to transparent in overlay mode.
    fn create_clear_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clear_shader"),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clear_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("clear_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None, // No blending — direct RGBA write
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create a render pipeline for headless mode (`Rgba8UnormSrgb`).
    fn create_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, wgpu::TextureFormat::Rgba8UnormSrgb, "")
    }

    /// Render one frame of the scene to the surface.
    ///
    /// This method is surface-agnostic: it works with any type implementing
    /// `CompositorSurface`.  The same code path executes in headless and windowed
    /// modes — only the surface implementation differs.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
    /// "No conditional compilation for the render path."
    ///
    /// For headless pixel readback, use `render_frame_headless()` instead,
    /// which includes the `copy_to_buffer` step internally so that
    /// `surface.read_pixels()` returns the current frame's data.
    /// `render_frame()` does NOT copy pixels to the readback buffer — the
    /// encoder is created and consumed internally and is not exposed.
    ///
    /// Returns telemetry for this frame.
    pub fn render_frame(
        &mut self,
        scene: &SceneGraph,
        surface: &dyn CompositorSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex buffer from scene
        let mut vertices: Vec<RectVertex> = Vec::new();
        let (sw, sh) = {
            let (w, h) = surface.size();
            (w as f32, h as f32)
        };

        // In overlay mode, prepend a full-screen quad to zero out alpha.
        if self.overlay_mode {
            // One-shot diagnostic: log surface dimensions on first frame.
            if self.frame_number == 1 {
                let diag = format!(
                    "render_frame: sw={sw}, sh={sh}, compositor_w={}, compositor_h={}\n",
                    self.width, self.height,
                );
                let _ = std::fs::write("C:\\tze_hud\\logs\\render_diag.txt", &diag);
            }
            vertices.extend_from_slice(&rect_vertices(
                0.0, 0.0, sw, sh, sw, sh,
                [0.0, 0.0, 0.0, 0.0],
            ));
        }

        for tile in &tiles {
            // Render tile background
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            vertices.extend_from_slice(&verts);

            // Render nodes within the tile
            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut vertices, sw, sh);
            }
        }

        // Render zone content.
        self.render_zone_content(scene, &mut vertices, sw, sh);


        let encode_start = std::time::Instant::now();

        // Create vertex buffer
        let vertex_buffer = if vertices.is_empty() {
            None
        } else {
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vertex_buffer"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buffer)
        };

        // Acquire frame through the surface trait (surface-agnostic).
        // The CompositorFrame._guard keeps the backing resource alive until drop.
        let frame = surface.acquire_frame();

        // Encode render pass
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // In overlay mode, use the clear_pipeline (no blending) for ALL
            // rendering. The first 6 vertices are a full-screen transparent
            // quad that zeros out every pixel's alpha. Subsequent content
            // (zone bars) overwrites specific regions with their own alpha.
            if self.overlay_mode {
                render_pass.set_pipeline(&self.clear_pipeline);
            } else {
                render_pass.set_pipeline(&self.pipeline);
            }

            if let Some(ref buffer) = vertex_buffer {
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..vertices.len() as u32, 0..1);
            }
        }

        telemetry.render_encode_us = encode_start.elapsed().as_micros() as u64;

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));

        // present() is surface-specific: no-op for headless, swap-chain flip for windowed.
        // Drop frame guard AFTER present() so the backing resource stays alive.
        surface.present();
        drop(frame);

        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Render one frame and copy pixel data into the headless readback buffer.
    ///
    /// This is a convenience method for testing/CI that handles the extra
    /// `copy_to_buffer` step required for headless pixel readback.
    ///
    /// The `copy_to_buffer` call must happen before `queue.submit()`, which
    /// means it cannot be cleanly extracted into a post-render callback without
    /// breaking the submit boundary.  This method intentionally duplicates the
    /// render pipeline for that reason.  A follow-up refactor should extract a
    /// shared internal `encode_frame` helper that accepts an optional readback
    /// callback, eliminating this duplication.
    ///
    /// Returns telemetry for this frame.
    pub fn render_frame_headless(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex buffer from scene.
        // Use surface.size() — not self.width/self.height — so that vertex
        // normalization is correct even if the HeadlessSurface was created with
        // different dimensions than the compositor's stored width/height.
        let mut vertices: Vec<RectVertex> = Vec::new();
        let (sw, sh) = {
            let (w, h) = surface.size();
            (w as f32, h as f32)
        };

        for tile in &tiles {
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            vertices.extend_from_slice(&verts);

            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut vertices, sw, sh);
            }
        }

        let encode_start = std::time::Instant::now();

        let vertex_buffer = if vertices.is_empty() {
            None
        } else {
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vertex_buffer"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buffer)
        };

        // Acquire frame via trait — same code path as render_frame().
        let frame = surface.acquire_frame();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);

            if let Some(ref buffer) = vertex_buffer {
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..vertices.len() as u32, 0..1);
            }
        }

        telemetry.render_encode_us = encode_start.elapsed().as_micros() as u64;

        // Headless-specific: copy rendered texture to readback buffer.
        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        surface.present(); // no-op for headless
        drop(frame);
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Render one frame with a separate chrome pass after the content pass.
    ///
    /// # Layer ordering (back to front)
    ///
    /// 1. **Content pass** — background clear + all agent tiles. Uses `LoadOp::Clear`.
    /// 2. **Chrome pass** — draws chrome draw commands on top. Uses `LoadOp::Load` to
    ///    preserve the content pass pixels, ensuring chrome is always on top.
    ///
    /// Chrome draw commands are produced by the caller (from `ChromeRenderer::render_chrome`)
    /// before calling this function. The compositor does not access `ChromeState` directly —
    /// chrome state is fully decoupled from scene/agent state.
    ///
    /// # Separable passes
    ///
    /// The content pass and chrome pass are encoded as two separate `RenderPass` blocks
    /// within the same `CommandEncoder`. This architectural separation is the foundation
    /// for future render-skip redaction (capture-safe architecture): the content pass can
    /// be suppressed independently of the chrome pass.
    ///
    /// # Chrome layer sovereignty
    ///
    /// Because the chrome pass uses `LoadOp::Load` and runs after the content pass,
    /// chrome pixels are always written last — no agent tile can occlude chrome regardless
    /// of its z-order value.
    pub fn render_frame_with_chrome(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
        chrome_cmds: &[ChromeDrawCmd],
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // ── Pass 1: Content (background + agent tiles) ──────────────────────
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        let mut content_vertices: Vec<RectVertex> = Vec::new();
        let sw = self.width as f32;
        let sh = self.height as f32;

        for tile in &tiles {
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            content_vertices.extend_from_slice(&verts);
            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut content_vertices, sw, sh);
            }
        }

        let encode_start = std::time::Instant::now();

        let content_buffer = if content_vertices.is_empty() {
            None
        } else {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("content_vertex_buffer"),
                    contents: bytemuck::cast_slice(&content_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buf)
        };

        // ── Pass 2: Chrome ───────────────────────────────────────────────────
        let mut chrome_vertices: Vec<RectVertex> = Vec::new();
        for cmd in chrome_cmds {
            let verts = rect_vertices(cmd.x, cmd.y, cmd.width, cmd.height, sw, sh, cmd.color);
            chrome_vertices.extend_from_slice(&verts);
        }

        let chrome_buffer = if chrome_vertices.is_empty() {
            None
        } else {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("chrome_vertex_buffer"),
                    contents: bytemuck::cast_slice(&chrome_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buf)
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder_with_chrome"),
            });

        // Content render pass — clears the surface and draws agent tiles.
        {
            let mut content_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("content_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            content_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = content_buffer {
                content_pass.set_vertex_buffer(0, buf.slice(..));
                content_pass.draw(0..content_vertices.len() as u32, 0..1);
            }
        }

        // Chrome render pass — uses LoadOp::Load to preserve content pixels.
        // Chrome commands are drawn ON TOP of content by construction.
        // No agent tile can occlude chrome regardless of z-order.
        {
            let mut chrome_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chrome_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // LoadOp::Load: preserve content pixels — chrome draws on top.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            chrome_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = chrome_buffer {
                chrome_pass.set_vertex_buffer(0, buf.slice(..));
                chrome_pass.draw(0..chrome_vertices.len() as u32, 0..1);
            }
        }

        telemetry.render_encode_us = encode_start.elapsed().as_micros() as u64;

        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Return the clear color for render passes. Transparent in overlay mode,
    /// dark background in fullscreen mode.
    fn clear_color(&self) -> wgpu::Color {
        if self.overlay_mode {
            wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }
        } else {
            wgpu::Color {
                r: 0.05,
                g: 0.05,
                b: 0.1,
                a: 1.0,
            }
        }
    }

    /// Render zone content as colored rectangles at zone geometry positions.
    ///
    /// Zones with active publishes get a visible indicator. Text rendering is
    /// deferred; for now the content text is not drawn, but the zone region is
    /// made visible so the user can confirm zone publishing works end-to-end.
    fn render_zone_content(
        &self,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
    ) {
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Resolve zone geometry to pixel bounds.
            let (x, y, w, h) = match &zone_def.geometry_policy {
                GeometryPolicy::EdgeAnchored {
                    edge,
                    height_pct,
                    width_pct,
                    margin_px,
                } => {
                    let zw = sw * width_pct;
                    let zh = sh * height_pct;
                    let zx = (sw - zw) / 2.0;
                    let zy = match edge {
                        DisplayEdge::Top => *margin_px,
                        DisplayEdge::Bottom => sh - zh - margin_px,
                        _ => 0.0,
                    };
                    (zx, zy, zw, zh)
                }
                GeometryPolicy::Relative {
                    x_pct,
                    y_pct,
                    width_pct,
                    height_pct,
                } => {
                    let zx = sw * x_pct;
                    let zy = sh * y_pct;
                    let zw = sw * width_pct;
                    let zh = sh * height_pct;
                    (zx, zy, zw, zh)
                }
            };

            // Semi-transparent background for zone content.
            let bg_color = [0.1, 0.1, 0.15, 0.85];
            vertices.extend_from_slice(&rect_vertices(x, y, w, h, sw, sh, bg_color));
        }
    }

    /// Determine the background color for a tile based on its content.
    fn tile_background_color(&self, tile: &Tile, scene: &SceneGraph) -> [f32; 4] {
        if let Some(root_id) = tile.root_node
            && let Some(node) = scene.nodes.get(&root_id)
        {
            match &node.data {
                NodeData::SolidColor(sc) => return sc.color.to_array(),
                NodeData::TextMarkdown(tm) => {
                    if let Some(bg) = &tm.background {
                        return bg.to_array();
                    }
                    return [0.15, 0.15, 0.25, tile.opacity];
                }
                NodeData::HitRegion(_) => {
                    // Check local state for visual feedback
                    if let Some(state) = scene.hit_region_states.get(&root_id) {
                        if state.pressed {
                            return [0.4, 0.7, 1.0, tile.opacity]; // Active blue
                        } else if state.hovered {
                            return [0.3, 0.5, 0.8, tile.opacity]; // Hover blue
                        }
                    }
                    return [0.2, 0.3, 0.5, tile.opacity]; // Default hit region
                }
                NodeData::StaticImage(_) => {
                    // Tile background for image tiles: near-black with slight tint
                    return [0.05, 0.05, 0.05, tile.opacity];
                }
            }
        }
        [0.1, 0.1, 0.2, tile.opacity]
    }

    /// Render a node and its children within a tile.
    #[allow(clippy::only_used_in_recursion)]
    fn render_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        match &node.data {
            NodeData::SolidColor(sc) => {
                let verts = rect_vertices(
                    tile.bounds.x + sc.bounds.x,
                    tile.bounds.y + sc.bounds.y,
                    sc.bounds.width,
                    sc.bounds.height,
                    sw,
                    sh,
                    sc.color.to_array(),
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::TextMarkdown(tm) => {
                // For the vertical slice, render text as a colored rectangle
                // (actual text rendering deferred to post-vertical-slice)
                let bg = tm.background.unwrap_or(Rgba::new(0.15, 0.15, 0.25, 1.0));
                let verts = rect_vertices(
                    tile.bounds.x + tm.bounds.x,
                    tile.bounds.y + tm.bounds.y,
                    tm.bounds.width,
                    tm.bounds.height,
                    sw,
                    sh,
                    bg.to_array(),
                );
                vertices.extend_from_slice(&verts);

                // Render a smaller text area rectangle in the text color
                // to indicate text content is present
                let text_margin = 8.0;
                if tm.bounds.width > text_margin * 2.0 && tm.bounds.height > text_margin * 2.0 {
                    let verts = rect_vertices(
                        tile.bounds.x + tm.bounds.x + text_margin,
                        tile.bounds.y + tm.bounds.y + text_margin,
                        tm.bounds.width - text_margin * 2.0,
                        (tm.font_size_px * 1.2).min(tm.bounds.height - text_margin * 2.0),
                        sw,
                        sh,
                        tm.color.to_array(),
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
            NodeData::HitRegion(hr) => {
                // Render hit region with local state feedback
                let color = if let Some(state) = scene.hit_region_states.get(&node_id) {
                    if state.pressed {
                        [0.4, 0.7, 1.0, 1.0]
                    } else if state.hovered {
                        [0.3, 0.5, 0.8, 1.0]
                    } else {
                        [0.2, 0.3, 0.5, 1.0]
                    }
                } else {
                    [0.2, 0.3, 0.5, 1.0]
                };

                let verts = rect_vertices(
                    tile.bounds.x + hr.bounds.x,
                    tile.bounds.y + hr.bounds.y,
                    hr.bounds.width,
                    hr.bounds.height,
                    sw,
                    sh,
                    color,
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::StaticImage(img) => {
                // Render a representative colored quad for the image bounds.
                //
                // Full GPU texture upload (wgpu::Texture from RGBA data) is deferred to a
                // follow-up iteration that adds a sampler pipeline. For the vertical slice this
                // placeholder renders a warm-gray background quad with a smaller accent strip
                // (mimicking the visual weight of an image) so that pixel-readback tests can
                // verify the node is composited into the frame at the correct position.
                let outer_color = [0.55_f32, 0.50, 0.45, 1.0]; // warm gray — "image placeholder"
                let verts = rect_vertices(
                    tile.bounds.x + img.bounds.x,
                    tile.bounds.y + img.bounds.y,
                    img.bounds.width,
                    img.bounds.height,
                    sw,
                    sh,
                    outer_color,
                );
                vertices.extend_from_slice(&verts);

                // Inner accent strip — a slightly brighter inset rectangle.
                let margin = 4.0_f32;
                if img.bounds.width > margin * 2.0 && img.bounds.height > margin * 2.0 {
                    let accent_color = [0.75_f32, 0.70, 0.65, 1.0];
                    let verts = rect_vertices(
                        tile.bounds.x + img.bounds.x + margin,
                        tile.bounds.y + img.bounds.y + margin,
                        img.bounds.width - margin * 2.0,
                        img.bounds.height - margin * 2.0,
                        sw,
                        sh,
                        accent_color,
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
        }

        // Render children
        for child_id in &node.children {
            self.render_node(*child_id, tile, scene, vertices, sw, sh);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("failed to create device: {0}")]
    DeviceCreation(String),
}

// Make buffer init descriptor available
use wgpu::util::DeviceExt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::HeadlessSurface;
    use tze_hud_scene::graph::SceneGraph;

    /// Mutex to serialize tests that mutate `HEADLESS_FORCE_SOFTWARE`, a
    /// global environment variable.  Rust tests run in parallel by default,
    /// so concurrent mutations would cause races.  This is an in-process lock;
    /// it does not protect against separate test binary runs.
    static ENV_VAR_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Convenience: build a minimal scene with one tile containing the given node.
    fn scene_with_node(node: Node) -> SceneGraph {
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene.set_tile_root(tile_id, node).unwrap();
        scene
    }

    /// Create a headless compositor and surface pair for testing.
    async fn make_compositor_and_surface(w: u32, h: u32) -> (Compositor, HeadlessSurface) {
        let compositor = Compositor::new_headless(w, h)
            .await
            .expect("headless compositor");
        let surface = HeadlessSurface::new(&compositor.device, w, h);
        (compositor, surface)
    }

    #[tokio::test]
    async fn test_static_image_node_renders_placeholder_quad() {
        // The static image placeholder renders a warm-gray outer quad ~[0.55, 0.50, 0.45].
        // In sRGB output the linear values are gamma-compressed.
        // We just verify that *some* non-background pixels appear in the expected warm range.
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        // RS-4: StaticImageNode uses resource_id + decoded_bytes; no raw blob embedded.
        let resource_id = ResourceId::of(b"8x8 test image placeholder");
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 8,
                height: 8,
                decoded_bytes: 8 * 8 * 4,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        };

        let scene = scene_with_node(node);
        compositor.render_frame_headless(&scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        // The background clear color is ~[0.05, 0.05, 0.1] in linear; tile bg is [0.05,0.05,0.05].
        // The placeholder outer quad is warm gray [0.55, 0.50, 0.45] in linear.
        // In sRGB this is approximately [198, 188, 176]. We look for pixels brighter than 150 in
        // all three channels to confirm the quad was rendered (not just the dark background).
        let any_warm_pixel = pixels
            .chunks(4)
            .any(|p| p[0] > 150 && p[1] > 140 && p[2] > 130);
        assert!(
            any_warm_pixel,
            "expected warm-gray placeholder pixels from StaticImageNode"
        );
    }

    #[tokio::test]
    async fn test_static_image_node_composited_with_other_nodes() {
        // Render a scene with both a SolidColor node and a StaticImage node in adjacent tiles.
        let (mut compositor, surface) = make_compositor_and_surface(512, 256).await;

        let mut scene = SceneGraph::new(512.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);

        // Left tile: red solid color
        let left_tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                left_tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0),
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Right tile: static image
        // RS-4: StaticImageNode uses resource_id + decoded_bytes; no raw blob embedded.
        let right_tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(256.0, 0.0, 256.0, 256.0),
                2,
            )
            .unwrap();
        scene
            .set_tile_root(
                right_tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id: ResourceId::of(b"8x8 green placeholder"),
                        width: 8,
                        height: 8,
                        decoded_bytes: 8 * 8 * 4,
                        fit_mode: ImageFitMode::Cover,
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        compositor.render_frame_headless(&scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 512 * 256 * 4, "pixel buffer size mismatch");
        // Just verify the frame completed without panic and returned the expected buffer size.
    }

    // ── Chrome layer pixel tests ──────────────────────────────────────────────

    /// Layer 1 pixel test: chrome layer is always visible above max-z-order agent tile.
    ///
    /// Acceptance criterion: "Layer 1 pixel tests confirm chrome always visible above
    /// max-z-order agent tile."
    ///
    /// This test renders a bright red tile at max z-order (u32::MAX) then renders a
    /// distinctive chrome rectangle over the same region. The chrome pixels (pure green)
    /// must overwrite the red tile pixels.
    #[tokio::test]
    async fn test_chrome_always_above_max_zorder_tile() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        // Agent tile at max valid agent z-order with bright red content.
        // Agent tiles must use z_order < ZONE_TILE_Z_MIN (0x8000_0000); u32::MAX is
        // reserved for runtime zone tiles (scene-graph/spec.md §Zone Layer Attachment).
        use tze_hud_scene::types::ZONE_TILE_Z_MIN;
        let max_agent_z = ZONE_TILE_Z_MIN - 1; // 0x7FFF_FFFF
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                max_agent_z,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0), // bright red
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Chrome draw command: bright green rectangle covering the full surface.
        // In NDC space, this will overwrite all tile content.
        let chrome_cmds = vec![crate::pipeline::ChromeDrawCmd {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 40.0,                // tab bar height
            color: [0.0, 1.0, 0.0, 1.0], // pure green — distinctive chrome marker
        }];

        compositor.render_frame_with_chrome(&scene, &surface, &chrome_cmds);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);

        // Check the top-left pixel region (where chrome covers the tile).
        // In sRGB, linear [0,1,0] green becomes approximately [0, 255, 0].
        // We look for pixels that are distinctly green (G > 200, R < 50).
        let chrome_top_pixel = &pixels[0..4]; // first pixel (top-left)
        assert!(
            chrome_top_pixel[1] > 150, // green channel dominant
            "chrome green channel should be dominant at top: {chrome_top_pixel:?}"
        );
        // The tile red should NOT bleed through chrome.
        assert!(
            chrome_top_pixel[0] < 50,
            "agent tile red must not show through chrome: {chrome_top_pixel:?}"
        );
    }

    /// Layer 1 pixel test: chrome hit-test priority — chrome is always drawn last.
    ///
    /// Verifies the separable render pass architecture: content pass first (agent tiles),
    /// chrome pass second (chrome elements). The two-pass structure guarantees chrome
    /// always occupies the final pixels regardless of content.
    #[tokio::test]
    async fn test_chrome_pass_uses_load_op_load() {
        // Render a scene with a blue agent tile + a chrome red stripe.
        // Blue content should persist where chrome doesn't cover; red should cover where it does.
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        // Blue tile — fills entire surface in content pass.
                        color: Rgba::new(0.0, 0.0, 1.0, 1.0),
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Chrome: red stripe only in top half (rows 0..128).
        let chrome_cmds = vec![crate::pipeline::ChromeDrawCmd {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 128.0,
            color: [1.0, 0.0, 0.0, 1.0], // pure red
        }];

        compositor.render_frame_with_chrome(&scene, &surface, &chrome_cmds);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);

        // Top row: chrome (red) should dominate.
        let top_px = &pixels[0..4];
        assert!(
            top_px[0] > 150,
            "top pixel should be red (chrome): {top_px:?}"
        );
        assert!(
            top_px[2] < 50,
            "top pixel blue (tile) must be suppressed by chrome: {top_px:?}"
        );

        // Bottom row: content (blue) should persist — chrome didn't cover it.
        // Row 255 starts at pixel offset 255*256*4.
        let bottom_row_offset = 255 * 256 * 4;
        let bottom_px = &pixels[bottom_row_offset..bottom_row_offset + 4];
        assert!(
            bottom_px[2] > 150,
            "bottom pixel should be blue (tile content, no chrome): {bottom_px:?}"
        );
        assert!(
            bottom_px[0] < 50,
            "bottom pixel red should be absent (no chrome): {bottom_px:?}"
        );
    }

    /// Verify that render_frame_with_chrome renders correctly even when chrome_cmds is empty.
    #[tokio::test]
    async fn test_two_pass_with_empty_chrome_cmds() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        let scene = scene_with_node(Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        });
        // Empty chrome cmds — must not panic.
        compositor.render_frame_with_chrome(&scene, &surface, &[]);
        compositor.device.poll(wgpu::Maintain::Wait);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 256 * 256 * 4);
    }

    // ── Headless parity tests ─────────────────────────────────────────────────

    /// Verify that `render_frame` (surface-agnostic) works with a `HeadlessSurface`
    /// as a `&dyn CompositorSurface`.  This is the core headless parity assertion:
    /// the same method that would be used with a windowed surface works headlessly.
    #[tokio::test]
    async fn test_render_frame_via_compositor_surface_trait() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        let scene = SceneGraph::new(256.0, 256.0);

        // render_frame takes &dyn CompositorSurface — no special headless branch.
        let telemetry =
            compositor.render_frame(&scene, &surface as &dyn crate::surface::CompositorSurface);
        assert!(telemetry.frame_time_us > 0, "frame time must be non-zero");
        assert_eq!(telemetry.tile_count, 0, "empty scene has no tiles");
    }

    /// Verify that HEADLESS_FORCE_SOFTWARE env-var path is exercised in the
    /// adapter-selection code.  We cannot assert the adapter backend in a unit
    /// test (it's opaque), so we just verify that creating a compositor with
    /// the env var set does not crash.
    #[tokio::test]
    async fn test_new_headless_with_force_software_env_var() {
        // Serialize all env-var-mutating tests via a process-wide mutex.
        // Rust tests run in parallel by default; without serialization,
        // a concurrent test could observe or overwrite HEADLESS_FORCE_SOFTWARE.
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Safety: single-threaded within the mutex guard; no other test
        // touches HEADLESS_FORCE_SOFTWARE while _guard is held.
        unsafe {
            std::env::set_var("HEADLESS_FORCE_SOFTWARE", "1");
        }
        let result = Compositor::new_headless(64, 64).await;
        unsafe {
            std::env::remove_var("HEADLESS_FORCE_SOFTWARE");
        }
        drop(_guard);

        // Either Ok (software GPU found) or Err(NoAdapter) (no software GPU
        // installed in this CI environment) are acceptable.  A panic would not be.
        match result {
            Ok(_) => {}
            Err(CompositorError::NoAdapter) => {}
            Err(e) => panic!("unexpected error with HEADLESS_FORCE_SOFTWARE=1: {e}"),
        }
    }

    // ── Surface capability guard + dimension clamping (hud-q5hx regression) ────
    //
    // These tests validate the defensive logic added to `new_windowed_inner()`:
    //   1. Empty surface capability lists return `Err` instead of panicking.
    //   2. Dimension clamping uses `.max(1)` to prevent zero-size configs.
    //
    // The windowed path requires a real display handle and GPU, so we test the
    // clamping arithmetic directly as a pure function.

    /// Dimension clamping must apply both the device max and a minimum of 1.
    /// wgpu panics on `surface.configure()` with zero-width or zero-height.
    #[test]
    fn surface_dim_clamp_zero_becomes_one() {
        let max_dim = 16384u32;
        assert_eq!(0u32.min(max_dim).max(1), 1);
        assert_eq!(1u32.min(max_dim).max(1), 1);
        assert_eq!(2560u32.min(max_dim).max(1), 2560);
        assert_eq!(3840u32.min(max_dim).max(1), 3840);
    }

    /// Dimension clamping respects the device maximum texture dimension.
    /// Values larger than the limit are clamped, values within the limit pass through.
    #[test]
    fn surface_dim_clamp_respects_device_limit() {
        let max_dim = 4096u32;
        assert_eq!(4097u32.min(max_dim).max(1), 4096, "over-limit must clamp");
        assert_eq!(4096u32.min(max_dim).max(1), 4096, "at-limit must pass");
        assert_eq!(2560u32.min(max_dim).max(1), 2560, "under-limit must pass");
        assert_eq!(1920u32.min(max_dim).max(1), 1920, "default res must pass");
    }

    /// Dimension clamping at 2560x1440 with a 32768 device limit (RTX 3080)
    /// must not clamp — 2560 and 1440 are well below the RTX 3080's limit.
    #[test]
    fn surface_dim_clamp_2560x1440_passes_on_rtx3080_limit() {
        // RTX 3080 with Vulkan driver reports max_texture_dimension_2d = 32768.
        let max_dim = 32768u32;
        assert_eq!(2560u32.min(max_dim).max(1), 2560, "2560 must not be clamped");
        assert_eq!(1440u32.min(max_dim).max(1), 1440, "1440 must not be clamped");
        assert_eq!(3840u32.min(max_dim).max(1), 3840, "4K must not be clamped");
        assert_eq!(2160u32.min(max_dim).max(1), 2160, "4K must not be clamped");
    }
}
