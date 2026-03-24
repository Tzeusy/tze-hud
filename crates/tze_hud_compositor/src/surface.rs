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

/// Stub for a windowed (swapchain) surface.
///
/// In the v1 runtime, the full windowed compositor is wired at the integration
/// layer using `winit` + `wgpu::Surface`. This struct captures the interface
/// contract and provides a no-op implementation suitable for type-checking and
/// future integration.
///
/// ## v1 Status
/// `WindowSurface` is a stub — it does not hold an actual `wgpu::Surface`
/// (that requires a `winit::Window` which is created at runtime startup).
/// Integration with a live window is deferred to the windowed runtime binary.
/// Tests and CI use [`HeadlessSurface`].
///
/// ## Thread model (spec line 54-55)
/// - `acquire_frame()` is called on the compositor thread.
/// - `present()` is called on the main thread (macOS/Metal).
///   The compositor thread sets `FrameReadySignal`; main thread polls it.
pub struct WindowSurface {
    pub width: u32,
    pub height: u32,
}

impl WindowSurface {
    /// Create a new `WindowSurface` stub with the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl CompositorSurface for WindowSurface {
    fn acquire_frame(&self) -> CompositorFrame {
        // Stub: no real swapchain in v1. The windowed runtime integration
        // will replace this with an actual `wgpu::Surface::get_current_texture()` call.
        unimplemented!(
            "WindowSurface::acquire_frame is a stub — use HeadlessSurface for testing, \
             or wire a real wgpu::Surface at runtime startup"
        )
    }

    fn present(&self) {
        // Windowed present() will call SurfaceTexture::present() when swap-chain is wired up.
        // Currently a stub — will be filled in when windowed mode is integrated.
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
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

    /// `WindowSurface::size()` should return configured dimensions without panicking.
    #[test]
    fn test_window_surface_size() {
        let ws = WindowSurface::new(1920, 1080);
        assert_eq!(ws.size(), (1920, 1080));
    }
}
