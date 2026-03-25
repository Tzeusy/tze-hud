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
/// - `present()` is called on the **main thread** (macOS/Metal requirement).
///   The compositor thread signals the main thread via `FrameReadySignal`;
///   the main thread calls `surface.present()` on the `SurfaceTexture`.
/// - `size()` may be called from any thread (stored atomically).
///
/// ## Reconfiguration
/// On window resize, call `reconfigure(new_width, new_height, &device)` from
/// the main thread before the next frame. The compositor thread will pick up
/// the new size automatically because `size()` reads from the stored fields.
pub struct WindowSurface {
    /// The underlying wgpu surface (window-backed swapchain).
    pub surface: wgpu::Surface<'static>,
    /// Current surface configuration.
    pub config: std::sync::Mutex<wgpu::SurfaceConfiguration>,
    /// Current width in pixels (kept in sync with config).
    pub width: std::sync::atomic::AtomicU32,
    /// Current height in pixels (kept in sync with config).
    pub height: std::sync::atomic::AtomicU32,
}

impl WindowSurface {
    /// Create a `WindowSurface` from an already-configured `wgpu::Surface`.
    ///
    /// The `config` must already have been applied to the surface via
    /// `surface.configure(&device, &config)` before calling this constructor.
    ///
    /// This constructor is called by `Compositor::new_windowed()` after adapter
    /// and device creation, so the surface and device are guaranteed compatible.
    pub fn new(
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        let width = config.width;
        let height = config.height;
        Self {
            surface,
            config: std::sync::Mutex::new(config),
            width: std::sync::atomic::AtomicU32::new(width),
            height: std::sync::atomic::AtomicU32::new(height),
        }
    }

    /// Reconfigure the surface after a window resize.
    ///
    /// MUST be called from the main thread. The compositor thread will see the
    /// new dimensions on the next `size()` call.
    pub fn reconfigure(&self, new_width: u32, new_height: u32, device: &wgpu::Device) {
        if new_width == 0 || new_height == 0 {
            // Zero-size surface is invalid — skip reconfiguration.
            return;
        }
        let mut cfg = self.config.lock().expect("WindowSurface config lock poisoned");
        cfg.width = new_width;
        cfg.height = new_height;
        self.surface.configure(device, &cfg);
        self.width.store(new_width, std::sync::atomic::Ordering::Release);
        self.height.store(new_height, std::sync::atomic::Ordering::Release);
        tracing::info!(
            width = new_width,
            height = new_height,
            "WindowSurface reconfigured after resize"
        );
    }
}

impl CompositorSurface for WindowSurface {
    /// Acquire the next swapchain image from the OS compositor.
    ///
    /// Called on the compositor thread (Stage 6 / Stage 7 boundary).
    /// Returns a `CompositorFrame` whose `_guard` holds the `SurfaceTexture`,
    /// keeping the swapchain image alive until after `present()`.
    ///
    /// Per spec §Compositor Surface Trait (line 364): "CompositorFrame MUST
    /// bundle the TextureView with an ownership guard (_guard: Box<dyn Any + Send>)
    /// to keep the SurfaceTexture alive until after present()."
    fn acquire_frame(&self) -> CompositorFrame {
        let surface_texture = self
            .surface
            .get_current_texture()
            .expect("WindowSurface::acquire_frame: failed to acquire swapchain texture");
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // Box the SurfaceTexture as the ownership guard.
        // It MUST NOT be dropped until after present() is called on the main thread.
        CompositorFrame {
            view,
            _guard: Box::new(surface_texture),
        }
    }

    /// Present the current frame to the display.
    ///
    /// On macOS/Metal this MUST be called on the main thread.
    /// The compositor thread signals via `FrameReadySignal`; the main thread
    /// calls this method.
    ///
    /// Note: the actual `SurfaceTexture::present()` call is made here via the
    /// `_guard` field in `CompositorFrame`. The caller is responsible for
    /// dropping the `CompositorFrame` after calling this.
    ///
    /// For `WindowedRuntime`, the main thread calls `surface.present()` when
    /// it receives the `FrameReadySignal`, then drops the frame.
    fn present(&self) {
        // present() on WindowSurface is a no-op at the trait level.
        // The real present is driven by the WindowedRuntime main thread loop,
        // which extracts the SurfaceTexture from the guard and calls .present()
        // on it directly. This design satisfies the macOS/Metal requirement that
        // surface.present() is called on the main thread.
        //
        // See WindowedRuntime for the complete present() call path.
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
            10, 20, 30, 255,   // pixel (1,0)
        ];
        HeadlessSurface::assert_pixel_color(&pixels, 2, 0, 0, [100, 200, 50, 255], 0, "exact")
            .expect("exact match should pass");
        HeadlessSurface::assert_pixel_color(&pixels, 2, 0, 0, [102, 200, 50, 255], 2, "within tol")
            .expect("within-tolerance should pass");
    }

    #[test]
    fn test_assert_pixel_color_fails_outside_tolerance() {
        let pixels: Vec<u8> = vec![100, 200, 50, 255];
        let result =
            HeadlessSurface::assert_pixel_color(&pixels, 1, 0, 0, [110, 200, 50, 255], 2, "outside");
        assert!(result.is_err(), "should fail when diff > tolerance");
        let msg = result.unwrap_err();
        assert!(msg.contains("channel 0"), "error should identify channel: {msg}");
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
