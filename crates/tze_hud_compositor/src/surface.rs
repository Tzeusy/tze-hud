//! Surface abstraction — windowed or headless.
//!
//! The compositor renders to a surface without knowing which kind it is.
//! Two implementations are provided:
//!
//! - [`HeadlessSurface`] — offscreen texture for testing and CI. `present()` is a no-op.
//! - [`WindowSurface`] — stub for a real winit/wgpu swapchain surface. In v1 the windowed
//!   compositor is wired at the integration layer; this type captures the interface contract.
//!
//! ## Spec: CompositorSurface Trait (RFC 0002 §8.1, spec.md line 364)
//!
//! ```text
//! CompositorSurface:
//!   acquire_frame() -> CompositorFrame
//!   present(frame: CompositorFrame)
//!   size() -> (u32, u32)
//! ```
//!
//! `CompositorFrame` bundles the `TextureView` with an ownership guard that keeps
//! the `SurfaceTexture` alive until `present()` is called (headless surfaces use
//! a no-op guard).

/// A frame acquired from the surface.
///
/// Bundles the `TextureView` with an optional ownership guard. The guard keeps
/// any associated `SurfaceTexture` alive until this frame is presented or dropped.
///
/// For [`HeadlessSurface`], `guard` is `None` (no swapchain texture to manage).
/// For a real [`WindowSurface`], the guard holds the `wgpu::SurfaceTexture`.
pub struct CompositorFrame {
    /// The texture view to render into.
    pub view: wgpu::TextureView,
    /// Optional guard keeping the underlying surface texture alive until present.
    pub guard: Option<Box<dyn std::any::Any + Send>>,
}

/// Trait for compositor render targets.
///
/// Implemented by both [`HeadlessSurface`] and [`WindowSurface`].
///
/// ## Thread model
/// - `acquire_frame()` is called on the compositor thread.
/// - `present()` MUST be called on the main thread (macOS/Metal requirement).
///   The compositor thread signals the main thread via `FrameReadySignal`;
///   the main thread calls `surface.present()`.
/// - `size()` may be called from any thread (read-only).
pub trait CompositorSurface: Send + 'static {
    /// Acquire a frame for rendering.
    ///
    /// Returns a [`CompositorFrame`] containing the texture view and an optional
    /// ownership guard. The guard keeps the underlying surface texture alive until
    /// `present()` is called.
    ///
    /// Called on the compositor thread (Stage 6 / Stage 7 boundary).
    fn acquire_frame(&self) -> CompositorFrame;

    /// Present the rendered frame.
    ///
    /// For [`HeadlessSurface`] this is a no-op.
    /// For a real surface this submits the frame to the display.
    ///
    /// **MUST be called on the main thread** (macOS/Metal requirement).
    fn present(&self);

    /// Returns (width, height) in physical pixels.
    fn size(&self) -> (u32, u32);

    /// Returns the current texture view for the surface.
    ///
    /// Convenience accessor used by [`crate::renderer::Compositor::render_frame`].
    /// The default implementation acquires a frame and returns a reference to its view.
    ///
    /// For [`HeadlessSurface`] this returns a reference to the pre-allocated view.
    fn current_texture_view(&self) -> &wgpu::TextureView;
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
        // Stub: no-op until real window integration is wired.
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn current_texture_view(&self) -> &wgpu::TextureView {
        unimplemented!(
            "WindowSurface::current_texture_view is a stub — use HeadlessSurface for testing"
        )
    }
}

/// Headless offscreen surface for testing and CI.
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

impl CompositorSurface for HeadlessSurface {
    /// Acquire a frame for rendering into the headless texture.
    ///
    /// Returns a `CompositorFrame` with the pre-allocated texture view and
    /// no ownership guard (the texture is owned by `HeadlessSurface` itself).
    fn acquire_frame(&self) -> CompositorFrame {
        // HeadlessSurface owns the texture; we create a new view for this frame.
        // The guard is None because there is no swapchain texture to release.
        let view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        CompositorFrame { view, guard: None }
    }

    /// No-op — headless surfaces don't present to a display.
    fn present(&self) {
        // Headless: present() is intentionally a no-op (spec line 139).
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns a reference to the pre-allocated texture view.
    ///
    /// The renderer uses this for the render pass attachment. This avoids
    /// re-creating the view per frame in the hot path.
    fn current_texture_view(&self) -> &wgpu::TextureView {
        &self.view
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
}
