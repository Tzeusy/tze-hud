//! GStreamer appsink-backed [`MediaDecodePipeline`] for tze_hud_runtime.
//!
//! This module is compiled only when the `gstreamer` feature flag is active.
//! Without the feature the file is present but the entire module is gated, so
//! `cargo check --workspace` succeeds on hosts that do not have GStreamer
//! development headers installed.
//!
//! # Activation
//!
//! ```bash
//! # Host must have:  libgstreamer1.0-dev  libgstreamer-plugins-base1.0-dev
//! cargo build -p tze_hud_runtime --features gstreamer
//! cargo test  -p tze_hud_runtime --features gstreamer gst_decode_pipeline
//! ```
//!
//! # Pipeline description
//!
//! The initial test pipeline is:
//!
//! ```text
//! videotestsrc ! videoconvert ! video/x-raw,format=RGBA ! appsink
//! ```
//!
//! A real media source can be substituted by passing an arbitrary
//! `gst-launch`-style description string to [`GstDecodePipeline::new`].
//!
//! # Colour-space note (hud-ndo7o)
//!
//! `upload_video_frame` in the compositor binds the resulting texture with
//! `wgpu::TextureFormat::Rgba8UnormSrgb`, which applies an sRGB EOTF on GPU
//! sampling.  GStreamer's `videoconvert` element outputs linear BT.709 data.
//! This means sampled colours undergo the sRGB EOTF even though the source is
//! BT.709, producing a mild gamma mismatch — colours will appear slightly
//! brighter than a reference display would show.  The approximation is
//! acceptable for initial integration; a proper fix would either:
//!
//! 1. switch the texture format to `Rgba8Unorm` (skips gamma) and handle
//!    colour management in the render pipeline, or
//! 2. insert a GStreamer `videobalance` or custom GLSL element that converts
//!    BT.709 → sRGB before the appsink.
//!
//! Tracking issue: hud-ndo7o.
//!
//! # Spec references
//!
//! * CLAUDE.md — GStreamer is the mandated media stack
//! * `crates/tze_hud_compositor/src/video_surface.rs` — [`MediaDecodePipeline`]
//!   trait definition (lives in compositor to keep it FFI-free)

#![cfg(feature = "gstreamer")]

use std::sync::OnceLock;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use tze_hud_compositor::video_surface::{MediaDecodePipeline, VideoFrame};

// ── One-time GStreamer init ────────────────────────────────────────────────────

/// Initialise GStreamer exactly once per process.
///
/// Multiple calls are safe and cheap — `OnceLock` ensures the `gst::init()`
/// call happens at most once.  Returns an error if GStreamer initialisation
/// fails (missing plugins, bad environment, etc.).
fn ensure_gst_init() -> Result<(), gst::glib::Error> {
    static GST_INIT: OnceLock<Result<(), gst::glib::Error>> = OnceLock::new();
    // `gst::init()` returns `Result<(), glib::Error>`.  Clone the stored
    // result so each caller gets the same outcome.
    GST_INIT
        .get_or_init(|| gst::init())
        .as_ref()
        .map(|_| ())
        .map_err(|e| e.clone())
}

// ── GstDecodePipeline ─────────────────────────────────────────────────────────

/// GStreamer appsink-backed implementation of [`MediaDecodePipeline`].
///
/// Owns a `gst::Pipeline` and an `AppSink` element.  Decoded RGBA8 frames are
/// pulled from the appsink in [`next_frame`][Self::next_frame] using
/// `try_pull_sample` (non-blocking).
///
/// # Thread safety
///
/// `GstDecodePipeline` is `Send + Sync` because:
/// - `gst::Pipeline` is `Send + Sync` (GStreamer objects are refcounted and
///   thread-safe after construction).
/// - `gst_app::AppSink` is likewise `Send + Sync`.
/// - `current_dimensions` is `Option<(u32, u32)>`, which is `Send + Sync`.
pub struct GstDecodePipeline {
    /// The running GStreamer pipeline.
    pipeline: gst::Pipeline,
    /// The appsink element that receives decoded RGBA8 frames.
    appsink: gst_app::AppSink,
    /// Dimensions negotiated from caps after the first sample arrives.
    /// `None` until the pipeline has produced at least one frame.
    current_dimensions: Option<(u32, u32)>,
}

impl GstDecodePipeline {
    /// Create a new pipeline from an arbitrary `gst-launch`-style description.
    ///
    /// The description must end with an `appsink` element named `"appsink0"`.
    /// Example:
    ///
    /// ```text
    /// videotestsrc ! videoconvert ! video/x-raw,format=RGBA ! appsink name=appsink0
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if GStreamer initialisation fails, the pipeline cannot
    /// be parsed, or the `appsink0` element is not found.
    pub fn new(description: &str) -> Result<Self, gst::glib::Error> {
        ensure_gst_init()?;

        let pipeline = gst::parse::launch(description)
            .map_err(|e| {
                // `gst::parse::launch` returns a `glib::Error` on failure.
                e
            })?
            .downcast::<gst::Pipeline>()
            .map_err(|_| {
                gst::glib::Error::new(
                    gst::glib::FileError::Failed,
                    "pipeline description did not produce a gst::Pipeline",
                )
            })?;

        let appsink = pipeline
            .by_name("appsink0")
            .ok_or_else(|| {
                gst::glib::Error::new(
                    gst::glib::FileError::Failed,
                    "pipeline must contain an appsink element named \"appsink0\"",
                )
            })?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| {
                gst::glib::Error::new(
                    gst::glib::FileError::Failed,
                    "element \"appsink0\" is not an AppSink",
                )
            })?;

        // Set the appsink to drop old samples rather than blocking the decoder.
        appsink.set_max_buffers(1);
        appsink.set_drop(true);
        appsink.set_sync(false);

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| {
                gst::glib::Error::new(
                    gst::glib::FileError::Failed,
                    "failed to set pipeline to PLAYING state",
                )
            })?;

        Ok(Self {
            pipeline,
            appsink,
            current_dimensions: None,
        })
    }

    /// Convenience constructor using a synthetic `videotestsrc` source.
    ///
    /// Pipeline:
    /// ```text
    /// videotestsrc ! videoconvert ! video/x-raw,format=RGBA ! appsink name=appsink0
    /// ```
    ///
    /// This is the preferred entry point for unit tests and smoke validation
    /// because it does not require any media file or network source.
    ///
    /// # Errors
    ///
    /// Returns an error if GStreamer initialisation fails or any required plugin
    /// (`videotestsrc`, `videoconvert`, `appsink`) is not installed.
    pub fn new_from_test_src() -> Result<Self, gst::glib::Error> {
        Self::new(
            "videotestsrc ! videoconvert ! video/x-raw,format=RGBA ! appsink name=appsink0",
        )
    }

    /// Extract `(width, height)` from a GStreamer `Sample`'s caps.
    ///
    /// Returns `None` if the caps are absent or do not carry `width`/`height`
    /// fields (which should never happen for a negotiated RGBA pipeline, but is
    /// handled defensively).
    fn dimensions_from_sample(sample: &gst::Sample) -> Option<(u32, u32)> {
        let caps = sample.caps()?;
        let structure = caps.structure(0)?;
        let width = structure.get::<i32>("width").ok()? as u32;
        let height = structure.get::<i32>("height").ok()? as u32;
        Some((width, height))
    }
}

impl Drop for GstDecodePipeline {
    fn drop(&mut self) {
        // Best-effort teardown: move the pipeline to NULL state to release
        // hardware resources and GStreamer threads.  Errors are silently
        // ignored because `drop` cannot propagate them.
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl MediaDecodePipeline for GstDecodePipeline {
    /// Poll for the next decoded RGBA8 frame.
    ///
    /// Uses `try_pull_sample` with a zero-duration timeout so the call never
    /// blocks.  Returns `None` if no new frame is available yet.
    ///
    /// On each successful pull the frame dimensions are updated from the
    /// sample's caps so [`frame_dimensions`][Self::frame_dimensions] converges
    /// to the negotiated resolution.
    fn next_frame(&mut self) -> Option<VideoFrame> {
        // Non-blocking pull.  `None` means no sample is ready yet.
        let sample = self
            .appsink
            .try_pull_sample(gst::ClockTime::ZERO)?;

        // Update cached dimensions from this sample's caps.
        if let Some(dims) = Self::dimensions_from_sample(&sample) {
            self.current_dimensions = Some(dims);
        }

        let (width, height) = self.current_dimensions?;

        let buffer = sample.buffer()?;
        let map = buffer.map_readable().ok()?;

        let expected_len = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        if map.len() != expected_len {
            tracing::warn!(
                actual_len = map.len(),
                expected_len,
                "GstDecodePipeline: buffer size mismatch (possible row padding); skipping frame"
            );
            return None;
        }

        // Copy the RGBA bytes out of the GStreamer buffer into an owned Vec so
        // the buffer map (and the underlying GstBuffer) can be released.
        let rgba = map.to_vec();

        // Use the GStreamer presentation-time-stamp (PTS) if available;
        // fall back to 0 if the buffer carries no PTS.
        let presented_at_us = buffer
            .pts()
            .map(|pts| pts.useconds())
            .unwrap_or(0);

        Some(VideoFrame {
            rgba,
            width,
            height,
            presented_at_us,
        })
    }

    /// Return the frame dimensions negotiated from the most recent appsink sample.
    ///
    /// Returns `None` until the pipeline has produced at least one frame and
    /// caps negotiation has completed.
    fn frame_dimensions(&self) -> Option<(u32, u32)> {
        self.current_dimensions
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Helper: spin until `pipeline.next_frame()` returns `Some`, up to `timeout`.
    ///
    /// GStreamer pipelines take a short time to enter PLAYING state and produce
    /// the first decoded sample.  This helper hides the latency from the tests.
    fn wait_for_frame(pipeline: &mut GstDecodePipeline, timeout: Duration) -> Option<VideoFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = pipeline.next_frame() {
                return Some(frame);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// GstDecodePipeline::new_from_test_src constructs successfully.
    ///
    /// If this test fails with an error message about missing plugins,
    /// install `gstreamer1.0-plugins-good` (provides `videotestsrc`).
    #[test]
    fn gst_decode_pipeline_constructs_from_test_src() {
        let result = GstDecodePipeline::new_from_test_src();
        assert!(
            result.is_ok(),
            "GstDecodePipeline::new_from_test_src() failed: {:?}",
            result.err()
        );
    }

    /// Pull at least one frame and verify non-zero dimensions.
    ///
    /// Validates the end-to-end path from GStreamer decode → appsink pull →
    /// VideoFrame.  Dimensions must be > 0 in both axes after a successful pull.
    #[test]
    fn gst_decode_pipeline_pulls_frame_with_nonzero_dimensions() {
        let mut pipeline = GstDecodePipeline::new_from_test_src()
            .expect("GstDecodePipeline::new_from_test_src() must succeed");

        let frame = wait_for_frame(&mut pipeline, Duration::from_secs(5))
            .expect("must pull at least one frame within 5 seconds");

        assert!(frame.width > 0, "frame width must be > 0");
        assert!(frame.height > 0, "frame height must be > 0");
    }

    /// Frame byte length equals width × height × 4 (RGBA8).
    ///
    /// This invariant must hold for every produced frame — the compositor's
    /// `upload_video_frame` path assumes exactly `width × height × 4` bytes.
    #[test]
    fn gst_decode_pipeline_frame_byte_length_matches_rgba8() {
        let mut pipeline = GstDecodePipeline::new_from_test_src()
            .expect("GstDecodePipeline::new_from_test_src() must succeed");

        let frame = wait_for_frame(&mut pipeline, Duration::from_secs(5))
            .expect("must pull at least one frame within 5 seconds");

        let expected = (frame.width as usize)
            .saturating_mul(frame.height as usize)
            .saturating_mul(4);

        assert_eq!(
            frame.rgba.len(),
            expected,
            "frame byte length must equal width({}) × height({}) × 4 = {}",
            frame.width,
            frame.height,
            expected
        );
    }

    /// frame_dimensions() converges to the negotiated resolution after a pull.
    ///
    /// Before any frame is pulled `frame_dimensions` returns `None`.  After at
    /// least one successful pull it must return `Some((w, h))` matching the
    /// frame.
    #[test]
    fn gst_decode_pipeline_frame_dimensions_converges_after_pull() {
        let mut pipeline = GstDecodePipeline::new_from_test_src()
            .expect("GstDecodePipeline::new_from_test_src() must succeed");

        let frame = wait_for_frame(&mut pipeline, Duration::from_secs(5))
            .expect("must pull at least one frame within 5 seconds");

        let dims = pipeline.frame_dimensions();
        assert!(
            dims.is_some(),
            "frame_dimensions() must return Some after a successful pull"
        );
        let (w, h) = dims.unwrap();
        assert_eq!(w, frame.width, "frame_dimensions width must match frame.width");
        assert_eq!(h, frame.height, "frame_dimensions height must match frame.height");
    }
}
