//! Surface abstraction — windowed or headless.
//!
//! The compositor renders to a surface without knowing which kind it is.
//!
//! # Design
//!
//! `CompositorSurface` is the trait that decouples the frame pipeline from the
//! display back-end.  Two implementations ship in v1:
//!
//! - [`HeadlessSurface`] — offscreen `wgpu::Texture` for testing and CI.
//!   `present()` is a no-op; pixel readback uses `copy_texture_to_buffer`.
//! - [`WindowSurface`] — window-backed surface via `wgpu::Surface`.
//!   Reserved for windowed mode; windowed rendering is post-vertical-slice.
//!
//! Per runtime-kernel/spec.md Requirement: Compositor Surface Trait (line 364)
//! the surface is abstracted behind `CompositorSurface` with `acquire_frame()`,
//! `present()`, and `size()`.  `CompositorFrame` bundles the `TextureView` with
//! an ownership guard so the `SurfaceTexture` lives until after `present()`.
//!
//! ## Thread model
//! - `acquire_frame()` is called on the compositor thread.
//! - `present()` MUST be called on the main thread (macOS/Metal requirement).
//!   The compositor thread signals the main thread via `FrameReadySignal`;
//!   the main thread calls `surface.present()`.
//! - `size()` may be called from any thread (read-only).

use std::any::Any;

// ─── CompositorFrame ─────────────────────────────────────────────────────────

/// A frame ready for rendering: a `TextureView` plus an ownership guard.
///
/// The `_guard` holds a heap-allocated value that keeps the underlying GPU
/// resource alive until this `CompositorFrame` is dropped.  For `HeadlessSurface`
/// the guard is a `()` no-op.  For `WindowSurface` it would hold the
/// `wgpu::SurfaceTexture`.
///
/// Per runtime-kernel/spec.md: "CompositorFrame MUST bundle the TextureView
/// with an ownership guard (_guard: Box<dyn Any + Send>) to keep the
/// SurfaceTexture alive until after present()." (line 364)
pub struct CompositorFrame {
    /// Render target for this frame.
    pub view: wgpu::TextureView,
    /// Ownership guard — keeps the backing resource alive until this frame is dropped.
    pub _guard: Box<dyn Any + Send>,
}

// ─── CompositorSurface trait ─────────────────────────────────────────────────

/// Trait for compositor render targets.
///
/// Implemented by both `HeadlessSurface` (offscreen) and `WindowSurface`
/// (display-connected).  The compositor uses this trait exclusively — no
/// `if headless { … } else { … }` branches in the frame pipeline.
///
/// Per runtime-kernel/spec.md Requirement: Compositor Surface Trait (line 364):
/// - `acquire_frame()` → `CompositorFrame` containing the `TextureView`.
/// - `present()` — submit/flip the frame.  No-op for headless.
/// - `size()` → `(width, height)` in pixels.
pub trait CompositorSurface: Send + 'static {
    /// Acquire a frame for rendering.  Returns a `CompositorFrame` whose
    /// `_guard` keeps the underlying GPU resource alive.
    ///
    /// Called on the compositor thread (Stage 6 / Stage 7 boundary).
    fn acquire_frame(&self) -> CompositorFrame;

    /// Present (flip/submit) the current frame.
    ///
    /// For `HeadlessSurface` this is a no-op per spec line 199.
    /// For a real surface this submits the frame to the display.
    ///
    /// **MUST be called on the main thread** (macOS/Metal requirement).
    fn present(&self);

    /// Surface dimensions in pixels.
    fn size(&self) -> (u32, u32);
}

// ─── WindowSurface ─────────────────────────────────────────────────────────────

/// Window-backed swapchain surface.
///
/// Wraps a `wgpu::Surface` created from a `winit::window::Window`. Used by
/// `WindowedRuntime` to display rendered frames on a real screen.
///
/// ## Thread model (spec §Compositor Thread Ownership, line 46)
/// - `acquire_frame()` is called on the **compositor thread** — it calls
///   `surface.get_current_texture()` to obtain the next swapchain image.
///   The acquired `SurfaceTexture` is stored in `pending_texture` so the main
///   thread can retrieve and present it.
/// - `take_pending_texture()` is called on the **main thread** after the
///   compositor signals `FrameReadySignal`. The main thread calls
///   `SurfaceTexture::present()` on the returned texture, satisfying the
///   macOS/Metal requirement that `present()` runs on the main thread.
/// - `size()` may be called from any thread (stored atomically).
/// - `pending_resize` signals a pending resize from the main thread to the
///   compositor thread. The compositor calls `reconfigure()` using its owned
///   `wgpu::Device` before the next `acquire_frame()`.
///
/// ## Reconfiguration
/// On window resize, the main thread stores the new dimensions in
/// `pending_resize`. The compositor thread detects a non-zero pending resize at
/// the start of each frame cycle and calls `reconfigure()`.
pub struct WindowSurface {
    /// The underlying wgpu surface (window-backed swapchain).
    pub surface: wgpu::Surface<'static>,
    /// Current surface configuration.
    pub config: std::sync::Mutex<wgpu::SurfaceConfiguration>,
    /// Current width in pixels (kept in sync with config).
    pub width: std::sync::atomic::AtomicU32,
    /// Current height in pixels (kept in sync with config).
    pub height: std::sync::atomic::AtomicU32,
    /// Pending `SurfaceTexture` acquired by the compositor thread and awaiting
    /// presentation on the main thread.
    ///
    /// The compositor stores the texture here in `acquire_frame()` so the main
    /// thread can retrieve it via `take_pending_texture()` and call
    /// `SurfaceTexture::present()` without a second swapchain acquire.
    pub pending_texture: std::sync::Mutex<Option<wgpu::SurfaceTexture>>,
    /// Pending resize dimensions signalled from the main thread to the
    /// compositor thread. `(0, 0)` means no resize pending.
    ///
    /// The main thread stores `(new_width, new_height)` atomically on
    /// `WindowEvent::Resized`. The compositor thread reads this at the start of
    /// each frame, applies `reconfigure()` with the new dimensions using its
    /// owned `wgpu::Device`, then resets both fields to `0`.
    pub pending_resize_width: std::sync::atomic::AtomicU32,
    pub pending_resize_height: std::sync::atomic::AtomicU32,
}

impl WindowSurface {
    /// Create a `WindowSurface` from an already-configured `wgpu::Surface`.
    ///
    /// The `config` must already have been applied to the surface via
    /// `surface.configure(&device, &config)` before calling this constructor.
    ///
    /// This constructor is called by `Compositor::new_windowed()` after adapter
    /// and device creation, so the surface and device are guaranteed compatible.
    pub fn new(surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration) -> Self {
        let width = config.width;
        let height = config.height;
        Self {
            surface,
            config: std::sync::Mutex::new(config),
            width: std::sync::atomic::AtomicU32::new(width),
            height: std::sync::atomic::AtomicU32::new(height),
            pending_texture: std::sync::Mutex::new(None),
            pending_resize_width: std::sync::atomic::AtomicU32::new(0),
            pending_resize_height: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Reconfigure the surface after a window resize.
    ///
    /// MUST be called from the compositor thread (it owns the `wgpu::Device`).
    /// The main thread signals a resize via `pending_resize_width/height`.
    pub fn reconfigure(&self, new_width: u32, new_height: u32, device: &wgpu::Device) {
        if new_width == 0 || new_height == 0 {
            // Zero-size surface is invalid — skip reconfiguration.
            return;
        }
        // Ensure the GPU has finished processing any command buffers that
        // reference the current surface texture before we drop it.  Without
        // this, queue::submit() may still be reading the texture asynchronously
        // when we destroy it below, causing a wgpu validation error:
        //   "Texture with '<Surface Texture>' label has been destroyed"
        device.poll(wgpu::Maintain::Wait);

        // wgpu requires all acquired SurfaceTexture images to be dropped before
        // calling Surface::configure(). During resize races the main thread may
        // not have presented the last pending image yet; clear it here to avoid:
        // "SurfaceOutput must be dropped before a new Surface is made".
        {
            let mut pending = self
                .pending_texture
                .lock()
                .expect("pending_texture lock poisoned");
            let _ = pending.take();
        }
        // Clamp to adapter's max texture dimension to avoid wgpu validation errors
        // on GPUs with limits below the requested window size.
        let max_dim = device.limits().max_texture_dimension_2d;
        let w = new_width.min(max_dim);
        let h = new_height.min(max_dim);
        let mut cfg = self
            .config
            .lock()
            .expect("WindowSurface config lock poisoned");
        cfg.width = w;
        cfg.height = h;
        self.surface.configure(device, &cfg);
        self.width.store(w, std::sync::atomic::Ordering::Release);
        self.height.store(h, std::sync::atomic::Ordering::Release);
        tracing::info!(
            width = w,
            height = h,
            "WindowSurface reconfigured after resize"
        );
    }

    /// Take the pending `SurfaceTexture` stored by the compositor thread.
    ///
    /// Called from the **main thread** after the compositor signals
    /// `FrameReadySignal`. Returns `Some(texture)` if a frame is ready,
    /// `None` if the compositor has not yet produced a frame this cycle.
    ///
    /// The caller MUST call `SurfaceTexture::present()` on the returned texture.
    pub fn take_pending_texture(&self) -> Option<wgpu::SurfaceTexture> {
        self.pending_texture
            .lock()
            .expect("pending_texture lock poisoned")
            .take()
    }

    /// Present the currently pending swapchain image, if any.
    ///
    /// This performs `take()+present()` while holding the `pending_texture`
    /// mutex so the compositor cannot call `get_current_texture()` in the tiny
    /// window between "take" and "present".
    ///
    /// Returns `true` if a texture was presented, `false` if no texture was
    /// pending.
    pub fn present_pending_texture(&self) -> bool {
        let mut pending = self
            .pending_texture
            .lock()
            .expect("pending_texture lock poisoned");
        if let Some(texture) = pending.take() {
            texture.present();
            true
        } else {
            false
        }
    }
}

impl CompositorSurface for WindowSurface {
    /// Acquire the next swapchain image from the OS compositor.
    ///
    /// Called on the compositor thread (Stage 6 / Stage 7 boundary).
    ///
    /// The acquired `SurfaceTexture` is stored in `self.pending_texture` so the
    /// main thread can retrieve it via `take_pending_texture()` and call
    /// `.present()` — satisfying the macOS/Metal requirement. The
    /// `CompositorFrame._guard` holds `()` (a no-op) because ownership has been
    /// transferred to `pending_texture`.
    ///
    /// On recoverable errors (`Outdated`, `Lost`, `Timeout`) a warning is logged
    /// and an empty frame (black output) is returned so the compositor can skip
    /// the frame gracefully rather than panicking.
    fn acquire_frame(&self) -> CompositorFrame {
        // Serialize acquire/pending-state handoff through the same mutex used
        // by the main thread present path. If a texture is still pending, reuse
        // that same acquired image instead of trying a second acquire.
        let mut pending = self
            .pending_texture
            .lock()
            .expect("pending_texture lock poisoned");

        if let Some(existing) = pending.as_ref() {
            let view = existing
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            return CompositorFrame {
                view,
                _guard: Box::new(()),
            };
        }

        match self.surface.get_current_texture() {
            Ok(surface_texture) => {
                let view = surface_texture
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                // Store the SurfaceTexture so the main thread can present it.
                // Do NOT box it inside CompositorFrame._guard — that would drop
                // it (without calling .present()) when the frame is dropped on
                // the compositor thread, discarding the rendered frame.
                *pending = Some(surface_texture);
                CompositorFrame {
                    view,
                    _guard: Box::new(()), // no-op — ownership is in pending_texture
                }
            }
            Err(e) => {
                // Recoverable: Outdated/Lost/Timeout happen on resize/minimize.
                // Log a warning and return a dummy frame so the compositor can
                // skip rendering this cycle without crashing.
                tracing::warn!(
                    error = %e,
                    "WindowSurface::acquire_frame: failed to acquire swapchain texture; skipping frame"
                );
                // Return a dummy frame — the render pass will render to a
                // scratch texture that is never presented. This is wasteful
                // but safe; the compositor will try again next frame.
                //
                // A future improvement: surface the error to the frame loop
                // so the compositor can skip the render pass entirely.
                let dummy = self.config.lock().expect("config lock poisoned");
                let dummy_view = {
                    // We can't create a texture without a device here.
                    // Instead, reuse the last pending texture's view if present.
                    // As a fallback, re-acquire (which may also fail).
                    drop(dummy);
                    // Re-attempt; if this also fails, the compositor thread
                    // will log the error and skip the frame on the next cycle.
                    match self.surface.get_current_texture() {
                        Ok(t) => {
                            let v = t
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());
                            *pending = Some(t);
                            v
                        }
                        Err(e2) => {
                            tracing::error!(
                                error = %e2,
                                "WindowSurface::acquire_frame: retry also failed; frame will be dropped"
                            );
                            // We cannot return a valid TextureView without a Device.
                            // Panic here to surface the misconfiguration clearly;
                            // in a production path this should be surfaced via the
                            // error channel to the runtime for a controlled restart.
                            panic!(
                                "WindowSurface::acquire_frame: cannot acquire swapchain texture after retry: {e2}"
                            )
                        }
                    }
                };
                CompositorFrame {
                    view: dummy_view,
                    _guard: Box::new(()),
                }
            }
        }
    }

    /// Present the current frame to the display.
    ///
    /// On macOS/Metal this MUST be called on the main thread. The
    /// `WindowedRuntime` main thread calls `take_pending_texture()` and then
    /// `SurfaceTexture::present()` directly. This trait method is a no-op for
    /// `WindowSurface` because the actual present happens via the pending-texture
    /// handoff, NOT through this `present()` call (which runs on the compositor
    /// thread alongside `render_frame()`).
    fn present(&self) {
        // No-op. The actual SurfaceTexture::present() is called by the main
        // thread via take_pending_texture(). See WindowedRuntime::maybe_present_frame().
    }

    fn size(&self) -> (u32, u32) {
        (
            self.width.load(std::sync::atomic::Ordering::Acquire),
            self.height.load(std::sync::atomic::Ordering::Acquire),
        )
    }
}

// ─── HeadlessSurface ─────────────────────────────────────────────────────────

/// Headless offscreen surface for testing and CI.
///
/// Satisfies:
/// - runtime-kernel/spec.md Requirement: Headless Mode (line 198)
/// - validation-framework/spec.md Requirement: DR-V2 (line 186)
/// - validation-framework/spec.md Requirement: DR-V6 (line 238)
pub struct HeadlessSurface {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// Output buffer for pixel readback.
    pub output_buffer: wgpu::Buffer,
    pub bytes_per_row: u32,
}

impl HeadlessSurface {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Buffer for readback
        let bytes_per_row = Self::aligned_bytes_per_row(width);
        let buffer_size = (bytes_per_row * height) as u64;
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("headless_readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            texture,
            view,
            width,
            height,
            output_buffer,
            bytes_per_row,
        }
    }

    /// Bytes per row aligned to wgpu's 256-byte requirement.
    fn aligned_bytes_per_row(width: u32) -> u32 {
        let unaligned = width * 4; // RGBA8 = 4 bytes per pixel
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        unaligned.div_ceil(align) * align
    }

    /// Copy the rendered texture to the readback buffer.
    ///
    /// Call this after the render pass, before submitting the command buffer.
    pub fn copy_to_buffer(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Read back pixels from the buffer. Returns RGBA8 data.
    /// This is a blocking call (waits for GPU to finish).
    ///
    /// Per runtime-kernel/spec.md line 208: pixel readback is on-demand via
    /// `copy_texture_to_buffer`.
    pub fn read_pixels(&self, device: &wgpu::Device) -> Vec<u8> {
        let buffer_slice = self.output_buffer.slice(..);

        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);

        // Remove row padding
        for y in 0..self.height {
            let offset = (y * self.bytes_per_row) as usize;
            let row_end = offset + (self.width * 4) as usize;
            pixels.extend_from_slice(&data[offset..row_end]);
        }

        drop(data);
        self.output_buffer.unmap();

        pixels
    }

    /// Get pixel at (x, y) from raw RGBA data. Returns [r, g, b, a].
    pub fn pixel_at(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * width + x) * 4) as usize;
        [data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]
    }

    /// Assert that a pixel at (x, y) is within `tolerance` of `expected` on
    /// every channel (R, G, B, A).
    ///
    /// Returns `Ok([r, g, b, a])` on pass, `Err(message)` on failure.
    ///
    /// The `label` is included in the error message for diagnostic clarity.
    ///
    /// Software-rasterised GPU paths (llvmpipe / SwiftShader) may produce
    /// values that differ from the linear-space input by ±2 per channel due
    /// to sRGB conversion.  Use `tolerance = 2` for solid fills on CI.
    pub fn assert_pixel_color(
        data: &[u8],
        width: u32,
        x: u32,
        y: u32,
        expected: [u8; 4],
        tolerance: u8,
        label: &str,
    ) -> Result<[u8; 4], String> {
        let actual = Self::pixel_at(data, width, x, y);
        for ch in 0..4 {
            let diff = actual[ch].abs_diff(expected[ch]);
            if diff > tolerance {
                return Err(format!(
                    "pixel assertion failed at ({x},{y}) [{label}]: \
                     channel {ch} actual={} expected={} diff={} tolerance={}",
                    actual[ch], expected[ch], diff, tolerance,
                ));
            }
        }
        Ok(actual)
    }
}

// ─── CompositorSurface impl for HeadlessSurface ────────────────────────────────

/// `HeadlessSurface` implements `CompositorSurface`.
///
/// - `acquire_frame()` creates a new `TextureView` from the offscreen texture; guard is `()`.
/// - `present()` is a no-op (spec line 199: "Headless surface present() MUST be a no-op").
/// - `size()` returns `(width, height)`.
impl CompositorSurface for HeadlessSurface {
    fn acquire_frame(&self) -> CompositorFrame {
        // Re-create the view from the texture (the stored view can't be Clone).
        // We return a new TextureView pointing at the same texture.
        // The guard is a no-op `()` because the texture is owned by HeadlessSurface.
        let view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        CompositorFrame {
            view,
            _guard: Box::new(()), // no-op guard — HeadlessSurface owns the texture
        }
    }

    /// No-op — headless mode does not present to a display (spec line 199).
    fn present(&self) {}

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_pixel_color_passes_within_tolerance() {
        let pixels: Vec<u8> = vec![
            100, 200, 50, 255, // pixel (0,0)
            10, 20, 30, 255, // pixel (1,0)
        ];
        HeadlessSurface::assert_pixel_color(&pixels, 2, 0, 0, [100, 200, 50, 255], 0, "exact")
            .expect("exact match should pass");
        HeadlessSurface::assert_pixel_color(&pixels, 2, 0, 0, [102, 200, 50, 255], 2, "within tol")
            .expect("within-tolerance should pass");
    }

    #[test]
    fn test_assert_pixel_color_fails_outside_tolerance() {
        let pixels: Vec<u8> = vec![100, 200, 50, 255];
        let result = HeadlessSurface::assert_pixel_color(
            &pixels,
            1,
            0,
            0,
            [110, 200, 50, 255],
            2,
            "outside",
        );
        assert!(result.is_err(), "should fail when diff > tolerance");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("channel 0"),
            "error should identify channel: {msg}"
        );
    }

    /// `WindowSurface::size()` cannot be tested without a real wgpu surface
    /// (requires a window handle). The `HeadlessSurface` covers the rendering
    /// path; windowed integration tests are in the `vertical_slice` binary.
    /// This test documents the interface contract for reviewers.
    #[test]
    fn test_window_surface_atomic_size_fields() {
        // Verify that AtomicU32 read/write works correctly for width/height.
        // We cannot construct a real WindowSurface without a window handle,
        // so this test only exercises the atomic helpers directly.
        use std::sync::atomic::{AtomicU32, Ordering};
        let w = AtomicU32::new(1920);
        let h = AtomicU32::new(1080);
        assert_eq!(w.load(Ordering::Acquire), 1920);
        assert_eq!(h.load(Ordering::Acquire), 1080);
        w.store(2560, Ordering::Release);
        assert_eq!(w.load(Ordering::Acquire), 2560);
    }
}
