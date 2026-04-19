//! # tze_hud_media_apple
//!
//! Safe Rust wrapper over `objc2-video-toolbox` for tze_hud iOS media decode.
//!
//! ## Scope
//!
//! This crate provides a safe, lifetime-correct `VtDecodeSession` wrapper over
//! Apple's `VTDecompressionSession` API. It bridges the VideoToolbox C callback
//! model to a `tokio::sync::mpsc` channel so that the rest of the tze_hud media
//! pipeline can consume decoded frames as an async stream.
//!
//! ## Platform Gating
//!
//! All implementation types are `#[cfg(target_vendor = "apple")]`-gated. On
//! Linux and Windows this crate compiles to an empty stub; the workspace remains
//! buildable without an Apple SDK. A future Apple-host worker lifts the stubs to
//! real implementation.
//!
//! ## Design Reference
//!
//! - `docs/audits/ios-videotoolbox-alternative-audit.md` — full API audit and
//!   crate selection rationale (hud-uzqfv).
//! - RFC 0002 §2.8 — Media Worker Boundary and ring-buffer capacity model.
//! - `about/heart-and-soul/media-doctrine.md` — arrival time ≠ presentation time.
//!
//! ## Architecture
//!
//! ```text
//!                           ┌────────────────────────┐
//!   RTP/str0m reassembled   │  VtDecodeSession        │  VT callback thread
//!   NALU bytes ─────────►  │  (safe Rust wrapper)    │ ─────────────────────►
//!                           │                         │  DecodedFrame via
//!   SPS/PPS format desc ──► │  VTDecompressionSession  │  tokio::sync::mpsc
//!                           │  (Apple C API)          │  (capacity = 4)
//!                           └─────────────────────────┘
//!                                                              │
//!                                                              ▼
//!                                               Tokio compositor task
//!                                               wgpu::Queue::write_texture
//! ```
//!
//! ## Tokio Bridge Contract
//!
//! The `VTDecompressionOutputCallback` fires on a VideoToolbox-managed thread —
//! never on a Tokio thread. The bridge uses `try_send` (non-blocking) on a
//! bounded `mpsc` channel with capacity 4 (matching RFC 0002 §2.8 ring-buffer
//! model). Back-pressure: frames are dropped on the VT thread if the compositor
//! Tokio task is behind. This is intentional — the compositor must never stall
//! the VT decode thread.
//!
//! ## Crate structure
//!
//! | Module              | Contents                                                   |
//! |---------------------|------------------------------------------------------------|
//! | `error`             | `VtError` error taxonomy with stable codes                 |
//! | `format`            | `VideoFormat` — codec + SPS/PPS bundle for H.264 / HEVC   |
//! | `frame`             | `DecodedFrame` — decoded pixel buffer + presentation stamp |
//! | `session`           | `VtDecodeSession` — the safe wrapper (Apple-only)          |
//!
//! ## Example (Apple targets only)
//!
//! ```rust,no_run
//! # #[cfg(target_vendor = "apple")]
//! # async fn example() -> Result<(), tze_hud_media_apple::error::VtError> {
//! use tze_hud_media_apple::{
//!     format::VideoFormat,
//!     session::VtDecodeSession,
//! };
//!
//! // Build a format descriptor from SPS/PPS bytes extracted from the SDP offer.
//! let sps: &[u8] = &[/* SPS NAL bytes */];
//! let pps: &[u8] = &[/* PPS NAL bytes */];
//! let format = VideoFormat::h264_from_parameter_sets(sps, pps)?;
//!
//! // Open a session — returns the session handle plus a frame receiver.
//! let (session, mut frame_rx) = VtDecodeSession::open(format)?;
//!
//! // Submit AVCC-format NAL frames as they arrive from str0m.
//! let nalu: &[u8] = &[/* AVCC frame bytes */];
//! let presentation_ts_ns: u64 = 0;
//! session.decode_frame(nalu, presentation_ts_ns)?;
//!
//! // Drain decoded frames in a Tokio task.
//! while let Some(frame) = frame_rx.recv().await {
//!     // `frame.pixel_data()` is NV12 (Y plane then UV plane).
//!     // Upload to wgpu via queue.write_texture(...).
//!     let _ = frame;
//! }
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod format;
pub mod frame;

#[cfg(target_vendor = "apple")]
pub mod session;

// Re-export the public surface for convenience.
pub use error::VtError;
pub use format::VideoFormat;
pub use frame::DecodedFrame;
