//! Decoded video frame type delivered by the Tokio bridge.
//!
//! `DecodedFrame` carries the CPU-side pixel data (NV12 / YpCbCr biplanar 420)
//! produced by the VideoToolbox output callback, together with the presentation
//! timestamp that the compositor uses for timing.
//!
//! Pixel data ownership: the frame owns a `Vec<u8>` copy of the pixel bytes
//! produced from `CVPixelBufferLockBaseAddress` / `memcpy` / `CVPixelBufferUnlockBaseAddress`
//! inside the VT callback. This is the safe path — `CVPixelBuffer` must not
//! escape the callback scope without explicit retain, and passing raw pointers
//! across the Tokio channel boundary would be unsound.
//!
//! Future optimization (post-v2): `CVMetalTextureCache` zero-copy GPU upload
//! eliminates the `memcpy` entirely by mapping the `CVPixelBuffer` as a Metal
//! texture. Filed as a follow-up in hud-l0h6t.

/// A single decoded video frame, delivered to the Tokio compositor task.
///
/// `DecodedFrame` is produced by `VtDecodeSession` (Apple targets only) and
/// received via `tokio::sync::mpsc::Receiver<DecodedFrame>`. The pixel data is
/// in **NV12** format (also called `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange`
/// on Apple platforms), which is the native output of the VideoToolbox
/// hardware decode path.
///
/// ## NV12 Layout
///
/// ```text
///   [Y plane bytes: width × height]
///   [UV plane bytes: width × (height / 2), interleaved U0 V0 U1 V1 ...]
/// ```
///
/// Total byte count: `width * height * 3 / 2`.
///
/// ## Uploading to wgpu
///
/// ```text
/// // Two-plane upload: Y plane then UV plane.
/// // (Illustrative; real implementation uses wgpu::Texture with two planes or
/// //  a RGBA conversion step, depending on the wgpu texture format in use.)
/// let (y, uv) = frame.as_nv12_planes();
/// // hand y and uv to wgpu::Queue::write_texture for the two planes
/// ```
#[derive(Debug)]
pub struct DecodedFrame {
    /// Frame width in pixels.
    width: u32,
    /// Frame height in pixels.
    height: u32,
    /// Presentation timestamp in nanoseconds, as supplied by the caller on
    /// `VtDecodeSession::decode_frame`. The compositor uses this for timing.
    ///
    /// The tze_hud runtime doctrine: arrival time ≠ presentation time.
    /// This timestamp is the *presentation* stamp — the compositor must not
    /// present before `presentation_ts_ns`.
    presentation_ts_ns: u64,
    /// Pixel data in NV12 format. Length = `width * height * 3 / 2`.
    pixels: Vec<u8>,
}

impl DecodedFrame {
    /// Construct a `DecodedFrame`.
    ///
    /// `pixels` must be exactly `width * height * 3 / 2` bytes of NV12 data.
    /// This constructor is `pub(crate)` — callers outside this crate receive
    /// frames from the `VtDecodeSession` channel, they do not construct them.
    ///
    /// On non-Apple targets the session module is cfg-gated away; suppress the
    /// dead-code lint so the struct still compiles cleanly on Linux.
    #[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
    pub(crate) fn new(width: u32, height: u32, presentation_ts_ns: u64, pixels: Vec<u8>) -> Self {
        debug_assert_eq!(
            pixels.len(),
            nv12_byte_count(width, height),
            "NV12 pixel buffer size mismatch: expected {}, got {}",
            nv12_byte_count(width, height),
            pixels.len()
        );
        Self {
            width,
            height,
            presentation_ts_ns,
            pixels,
        }
    }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Presentation timestamp in nanoseconds.
    ///
    /// The compositor must not present this frame before this timestamp
    /// (tze_hud media doctrine: arrival time ≠ presentation time).
    pub fn presentation_ts_ns(&self) -> u64 {
        self.presentation_ts_ns
    }

    /// Returns the full NV12 pixel data as a byte slice.
    ///
    /// Layout: `[Y plane][UV plane]` where the Y plane is `width × height`
    /// bytes and the UV plane is `width × (height / 2)` bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// Returns `(y_plane, uv_plane)` slices for two-plane wgpu upload.
    ///
    /// The Y plane is `width × height` bytes. The UV plane is `width × (height / 2)` bytes.
    pub fn as_nv12_planes(&self) -> (&[u8], &[u8]) {
        let y_len = (self.width * self.height) as usize;
        let (y, uv) = self.pixels.split_at(y_len);
        (y, uv)
    }

    /// Total byte count for NV12 pixel data at this frame's dimensions.
    pub fn byte_count(&self) -> usize {
        nv12_byte_count(self.width, self.height)
    }
}

/// Computes the expected byte count for an NV12 buffer of the given dimensions.
///
/// NV12: Y plane (width × height) + UV plane (width × height/2) = 3/2 × w × h.
/// For odd heights the UV plane is `width × ceil(height/2)` bytes; this
/// function uses integer arithmetic that matches the VideoToolbox output.
pub(crate) fn nv12_byte_count(width: u32, height: u32) -> usize {
    let y = (width as usize) * (height as usize);
    let uv = (width as usize) * (height as usize).div_ceil(2);
    y + uv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_byte_count_720p() {
        // 1280×720: Y=921600, UV=460800, total=1382400.
        assert_eq!(nv12_byte_count(1280, 720), 1_382_400);
    }

    #[test]
    fn nv12_byte_count_1080p() {
        // 1920×1080: Y=2073600, UV=1036800, total=3110400.
        assert_eq!(nv12_byte_count(1920, 1080), 3_110_400);
    }

    #[test]
    fn nv12_byte_count_odd_height() {
        // Width=4, height=3: Y=12, UV = 4 * ceil(3/2) = 4*2 = 8. Total=20.
        assert_eq!(nv12_byte_count(4, 3), 20);
    }

    #[test]
    fn decoded_frame_planes_split_correctly() {
        let w = 4u32;
        let h = 2u32;
        let total = nv12_byte_count(w, h);
        let y_len = (w * h) as usize;
        let pixels: Vec<u8> = (0..total as u8).collect();
        let frame = DecodedFrame::new(w, h, 12345, pixels);
        let (y, uv) = frame.as_nv12_planes();
        assert_eq!(y.len(), y_len);
        assert_eq!(uv.len(), total - y_len);
        assert_eq!(y[0], 0);
        assert_eq!(uv[0], y_len as u8);
    }

    #[test]
    fn decoded_frame_accessors() {
        let pixels = vec![0u8; nv12_byte_count(16, 16)];
        let frame = DecodedFrame::new(16, 16, 999_999_999, pixels);
        assert_eq!(frame.width(), 16);
        assert_eq!(frame.height(), 16);
        assert_eq!(frame.presentation_ts_ns(), 999_999_999);
    }
}
