//! Safe `VtDecodeSession` wrapper over `VTDecompressionSession`.
//!
//! This module is `#[cfg(target_vendor = "apple")]`-gated. It does not compile
//! on Linux or Windows. The workspace-level `lib.rs` re-exports this module
//! only on Apple targets.
//!
//! ## Safety Model
//!
//! `VTDecompressionSession` is a CoreFoundation `CFTypeRef`-based object. The
//! `objc2-video-toolbox` crate exposes it as a raw C API. This module wraps it
//! in a safe Rust struct with the following invariants:
//!
//! 1. **Single owner.** `VtDecodeSession` is `!Clone`. Only one Rust value
//!    holds the `VTDecompressionSessionRef` at a time.
//! 2. **Drop invalidates.** `Drop` calls `VTDecompressionSessionInvalidate`
//!    then `CFRelease`. No double-invalidate is possible because the session
//!    ref is moved into the struct and nulled on drop.
//! 3. **Callback lifetime.** The `VTDecompressionOutputCallback` is a raw C
//!    function pointer. The callback state (the `mpsc::Sender<DecodedFrame>`)
//!    is heap-allocated via `Box` and its raw pointer is stored in the
//!    `decompressionOutputRefCon` field. The `Box` is reclaimed in `Drop`
//!    after `VTDecompressionSessionInvalidate` ensures no further callbacks
//!    will fire.
//! 4. **Non-blocking callback.** The callback calls `try_send` — it never
//!    blocks. If the Tokio consumer is behind, frames are dropped on the VT
//!    thread. This is intentional (media doctrine: compositor must not stall).
//! 5. **Send safety.** `VTDecompressionSessionRef` is safe to send across
//!    threads when properly retained (CoreFoundation objects are thread-safe
//!    for retain/release). The `VtDecodeSession` is `Send` but not `Sync`.
//!
//! ## Tokio Bridge
//!
//! Channel capacity = 4, matching the RFC 0002 §2.8 ring-buffer model. The
//! sender lives in the callback (VT thread). The receiver is returned to the
//! caller and consumed by a Tokio task in the compositor.
//!
//! ## Stub Notice
//!
//! The current implementation is a **design skeleton**. The unsafe FFI calls
//! to `objc2-video-toolbox` types are present as commented-out pseudocode with
//! `// TODO(hud-l0h6t):` markers. A future Apple-host worker replaces the
//! pseudocode with real bindings.
//!
//! The skeleton compiles on Apple targets (the `objc2-video-toolbox` dependency
//! is in scope) but the `open()` constructor returns
//! [`VtError::UnsupportedCodec`] with a `"stub — not yet implemented"` message
//! until the TODO stubs are filled in.

#![cfg(target_vendor = "apple")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::error::VtError;
use crate::format::{Codec, VideoFormat};
use crate::frame::{DecodedFrame, nv12_byte_count};

// ---------------------------------------------------------------------------
// Channel capacity
// ---------------------------------------------------------------------------

/// Bounded channel capacity for decoded frames delivered from the VT callback
/// to the Tokio compositor task.
///
/// Value = 4, per RFC 0002 §2.8 ring-buffer model. Frames are dropped in the
/// callback if the compositor is behind; the compositor must never block the VT
/// decode thread.
const FRAME_CHANNEL_CAPACITY: usize = 4;

// ---------------------------------------------------------------------------
// Callback state — heap-allocated, erased through `*mut ()` in the refCon
// ---------------------------------------------------------------------------

/// State threaded through `decompressionOutputRefCon`. Heap-allocated and
/// leaked into the C callback; reclaimed in `VtDecodeSession::drop`.
struct CallbackState {
    sender: mpsc::Sender<DecodedFrame>,
    width: u32,
    height: u32,
    invalidated: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// VtDecodeSession
// ---------------------------------------------------------------------------

/// Safe wrapper over a `VTDecompressionSession`.
///
/// Create via [`VtDecodeSession::open`]. The returned
/// `mpsc::Receiver<DecodedFrame>` delivers decoded frames to the Tokio
/// compositor task. Drop the `VtDecodeSession` to invalidate the underlying
/// session and stop decode.
///
/// `VtDecodeSession` is `Send` but not `Sync`. Move it to the media worker
/// task that submits NAL frames; keep the `Receiver` in the compositor task.
pub struct VtDecodeSession {
    // TODO(hud-l0h6t): Replace `*mut ()` with the real `VTDecompressionSessionRef`
    // type from `objc2-video-toolbox` once building on an Apple host.
    //
    //   session_ref: objc2_video_toolbox::VTDecompressionSessionRef,
    _session_ref: *mut (),

    /// Raw pointer to the heap-allocated `CallbackState`. Reclaimed in `Drop`.
    callback_state_ptr: *mut CallbackState,

    /// Shared flag set to `true` after `VTDecompressionSessionInvalidate`.
    /// Guards against use-after-invalidate in `decode_frame`.
    invalidated: Arc<AtomicBool>,
}

// SAFETY: `VTDecompressionSession` is a CoreFoundation object; CoreFoundation
// objects are thread-safe for retain/release. `VtDecodeSession` is the sole
// owner and does not expose the raw ref externally. `Send` is sound here.
// `Sync` is NOT derived — callers must not call `decode_frame` concurrently.
unsafe impl Send for VtDecodeSession {}

impl VtDecodeSession {
    /// Open a `VTDecompressionSession` for the given `VideoFormat`.
    ///
    /// Returns a `(VtDecodeSession, Receiver<DecodedFrame>)` pair. The session
    /// submits NAL frames via [`decode_frame`][Self::decode_frame]; decoded
    /// frames arrive on the `Receiver`.
    ///
    /// ## Errors
    ///
    /// - [`VtError::UnsupportedCodec`] if the codec is VP9 (not supported by
    ///   VideoToolbox on iOS — see audit §1.2).
    /// - [`VtError::SessionCreateFailed`] if `VTDecompressionSessionCreate`
    ///   returns a non-zero `OSStatus`.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use tze_hud_media_apple::{format::VideoFormat, session::VtDecodeSession};
    /// # async fn run() -> Result<(), tze_hud_media_apple::error::VtError> {
    /// let format = VideoFormat::h264_from_parameter_sets(
    ///     &[0x67, 0x42, 0xc0, 0x1f][..],
    ///     &[0x68, 0xce, 0x38, 0x80][..],
    ///     1280,
    ///     720,
    /// )?;
    /// let (session, mut rx) = VtDecodeSession::open(format)?;
    /// // submit frames with session.decode_frame(nalu, pts_ns)?;
    /// // drain rx in a Tokio task
    /// # Ok(())
    /// # }
    /// ```
    pub fn open(format: VideoFormat) -> Result<(Self, mpsc::Receiver<DecodedFrame>), VtError> {
        // VP9 is not supported by VideoToolbox on iOS.
        // See: docs/audits/ios-videotoolbox-alternative-audit.md §1.2
        //
        // VP9 software decode via libvpx is a separate path (hud-l0h6t follow-up).
        // We gate it here so callers get an explicit, actionable error rather than
        // a cryptic OSStatus -12906 from VideoToolbox.
        //
        // VP9 is not a Codec variant in this crate — the check is left as a
        // comment for the Apple-host implementer who adds VP9 codec routing.

        debug!(
            codec = format.codec().name(),
            width = format.width(),
            height = format.height(),
            "opening VTDecompressionSession"
        );

        let (sender, receiver) = mpsc::channel::<DecodedFrame>(FRAME_CHANNEL_CAPACITY);
        let invalidated = Arc::new(AtomicBool::new(false));

        let callback_state = Box::new(CallbackState {
            sender,
            width: format.width(),
            height: format.height(),
            invalidated: Arc::clone(&invalidated),
        });
        let callback_state_ptr = Box::into_raw(callback_state);

        // TODO(hud-l0h6t): Replace the stub below with real VideoToolbox calls.
        //
        // --- Apple-host implementation sketch ---
        //
        // Step 1: Build CMVideoFormatDescription from SPS/PPS.
        //
        //   let format_desc = match format.codec() {
        //       Codec::H264 => {
        //           let CodecParameters::H264 { ref sps, ref pps } = format.parameters;
        //           let param_sets = [sps.as_ptr(), pps.as_ptr()];
        //           let param_sizes = [sps.len(), pps.len()];
        //           let mut desc: CMVideoFormatDescriptionRef = ptr::null_mut();
        //           let status = CMVideoFormatDescriptionCreateFromH264ParameterSets(
        //               kCFAllocatorDefault,
        //               2,
        //               param_sets.as_ptr(),
        //               param_sizes.as_ptr(),
        //               4, // NAL unit header length (AVCC)
        //               &mut desc,
        //           );
        //           if status != 0 {
        //               // Reclaim state before returning error.
        //               drop(unsafe { Box::from_raw(callback_state_ptr) });
        //               return Err(VtError::InvalidFormat {
        //                   detail: format!("CMVideoFormatDescriptionCreateFromH264ParameterSets \
        //                                   returned OSStatus {status}"),
        //               });
        //           }
        //           desc
        //       }
        //       Codec::Hevc => { /* analogous for HEVC */ todo!() }
        //   };
        //
        // Step 2: Configure decoder pixel buffer attributes.
        //   Request NV12 (kCVPixelFormatType_420YpCbCr8BiPlanarFullRange = 875704422).
        //
        //   let pixel_fmt = CFNumber::from(875704422i32);
        //   let attrs = CFDictionary::from_pairs(&[
        //       (kCVPixelBufferPixelFormatTypeKey, pixel_fmt.as_CFType()),
        //   ]);
        //
        // Step 3: Set up VTDecompressionOutputCallbackRecord.
        //
        //   let callback_record = VTDecompressionOutputCallbackRecord {
        //       decompressionOutputCallback: Some(vt_output_callback),
        //       decompressionOutputRefCon: callback_state_ptr as *mut _,
        //   };
        //
        // Step 4: Call VTDecompressionSessionCreate.
        //
        //   let mut session_ref: VTDecompressionSessionRef = ptr::null_mut();
        //   let decoder_spec = CFDictionary::from_pairs(&[
        //       (kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder,
        //        kCFBooleanTrue),
        //   ]);
        //   let status = VTDecompressionSessionCreate(
        //       kCFAllocatorDefault,
        //       format_desc,
        //       decoder_spec.as_concrete_TypeRef(),
        //       attrs.as_concrete_TypeRef(),
        //       &callback_record,
        //       &mut session_ref,
        //   );
        //   if status != 0 {
        //       drop(unsafe { Box::from_raw(callback_state_ptr) });
        //       return Err(VtError::SessionCreateFailed { os_status: status });
        //   }
        //
        // Return the real session ref instead of null:
        //   Ok((Self { _session_ref: session_ref as *mut (), callback_state_ptr, invalidated }, receiver))

        // Stub: return an error until the Apple-host worker fills in the TODOs.
        //
        // SAFETY: We own callback_state_ptr — reconstruct the Box and drop it
        // cleanly so we don't leak heap memory.
        let _ = unsafe { Box::from_raw(callback_state_ptr) };

        Err(VtError::UnsupportedCodec {
            codec: format!(
                "{} (stub — fill in hud-l0h6t TODOs on an Apple host)",
                format.codec().name()
            ),
        })
    }

    /// Submit one AVCC-format NAL frame for asynchronous decode.
    ///
    /// `nalu` must be in **AVCC** format (4-byte big-endian length prefix before
    /// each NAL unit), not Annex B (`00 00 00 01` start codes). If you receive
    /// Annex B bytes from str0m, convert them first.
    ///
    /// `presentation_ts_ns` is the presentation timestamp in nanoseconds. The
    /// compositor uses this to schedule frame presentation per tze_hud media
    /// doctrine (arrival time ≠ presentation time).
    ///
    /// This method is **not** `async` — it submits the frame synchronously to
    /// the VideoToolbox session. The VideoToolbox callback fires asynchronously
    /// on a VT-managed thread and delivers the decoded frame to the Tokio
    /// channel.
    ///
    /// ## Errors
    ///
    /// - [`VtError::SessionInvalidated`] if the session has already been invalidated.
    /// - [`VtError::BlockBufferCreateFailed`] if `CMBlockBufferCreate` fails.
    /// - [`VtError::SampleBufferCreateFailed`] if `CMSampleBufferCreate` fails.
    /// - [`VtError::DecodeSubmitFailed`] if `VTDecompressionSessionDecodeFrame` fails.
    pub fn decode_frame(&self, nalu: &[u8], presentation_ts_ns: u64) -> Result<(), VtError> {
        if self.invalidated.load(Ordering::Acquire) {
            return Err(VtError::SessionInvalidated);
        }

        debug!(
            bytes = nalu.len(),
            pts_ns = presentation_ts_ns,
            "submitting NAL frame to VTDecompressionSession"
        );

        // TODO(hud-l0h6t): Replace the stub below with real VideoToolbox calls.
        //
        // --- Apple-host implementation sketch ---
        //
        // Step 1: Wrap NALU bytes in a CMBlockBuffer.
        //
        //   let mut block_buf: CMBlockBufferRef = ptr::null_mut();
        //   // CMBlockBufferCreateWithMemoryBlock copies the bytes.
        //   let status = CMBlockBufferCreateWithMemoryBlock(
        //       kCFAllocatorDefault,
        //       ptr::null_mut(),   // memoryBlock (null = allocate)
        //       nalu.len(),
        //       kCFAllocatorDefault,
        //       ptr::null(),       // custom block source
        //       0,                 // offset
        //       nalu.len(),
        //       0,                 // flags
        //       &mut block_buf,
        //   );
        //   if status != 0 {
        //       return Err(VtError::BlockBufferCreateFailed { os_status: status });
        //   }
        //   let status = CMBlockBufferReplaceDataBytes(
        //       nalu.as_ptr() as *const _,
        //       block_buf,
        //       0,
        //       nalu.len(),
        //   );
        //   // ... error handling ...
        //
        // Step 2: Build CMSampleBuffer timing info.
        //
        //   let pts_secs = presentation_ts_ns as f64 / 1_000_000_000.0;
        //   let timing = CMSampleTimingInfo {
        //       duration: CMTimeMakeWithSeconds(0.0, 90000),
        //       presentationTimeStamp: CMTimeMakeWithSeconds(pts_secs, 90000),
        //       decodeTimeStamp: kCMTimeInvalid,
        //   };
        //
        // Step 3: Create CMSampleBuffer.
        //
        //   let mut sample_buf: CMSampleBufferRef = ptr::null_mut();
        //   let status = CMSampleBufferCreate(
        //       kCFAllocatorDefault,
        //       block_buf,
        //       1,      // dataReady
        //       None,   // makeDataReadyCallback
        //       ptr::null_mut(),
        //       self.format_desc,   // stored in session struct
        //       1,      // numSamples
        //       1,      // numSampleTimingEntries
        //       &timing,
        //       1,      // numSampleSizeEntries
        //       &nalu.len(),
        //       &mut sample_buf,
        //   );
        //   if status != 0 {
        //       CFRelease(block_buf as *const _);
        //       return Err(VtError::SampleBufferCreateFailed { os_status: status });
        //   }
        //
        // Step 4: Decode.
        //
        //   let flags: VTDecodeFrameFlags =
        //       kVTDecodeFrame_EnableAsynchronousDecompression |
        //       kVTDecodeFrame_EnableTemporalProcessing;
        //   let mut info_flags: VTDecodeInfoFlags = 0;
        //   let status = VTDecompressionSessionDecodeFrame(
        //       self._session_ref as VTDecompressionSessionRef,
        //       sample_buf,
        //       flags,
        //       ptr::null_mut(), // sourceFrameRefCon (not used)
        //       &mut info_flags,
        //   );
        //   CFRelease(sample_buf as *const _);
        //   CFRelease(block_buf as *const _);
        //   if status != 0 {
        //       return Err(VtError::DecodeSubmitFailed { os_status: status });
        //   }

        // Stub — no real implementation until hud-l0h6t TODOs are filled.
        warn!("VtDecodeSession::decode_frame is a stub; no frame will be produced");
        Ok(())
    }
}

impl Drop for VtDecodeSession {
    fn drop(&mut self) {
        self.invalidated.store(true, Ordering::Release);

        // TODO(hud-l0h6t): Uncomment on Apple host.
        //
        //   // SAFETY: VTDecompressionSessionInvalidate is safe to call from any
        //   // thread. It blocks until all pending decode callbacks have completed,
        //   // guaranteeing that the callback will not fire after this call returns.
        //   // We may then safely reclaim the callback state.
        //   unsafe {
        //       VTDecompressionSessionInvalidate(
        //           self._session_ref as VTDecompressionSessionRef
        //       );
        //       CFRelease(self._session_ref as *const _);
        //   }

        // SAFETY: `callback_state_ptr` was created via `Box::into_raw` in
        // `VtDecodeSession::open` and is not accessed after invalidation.
        // `VTDecompressionSessionInvalidate` (called above in real impl) ensures
        // no further callbacks fire before we reclaim the Box.
        //
        // In the stub path we never actually created a valid session, so this
        // block is unreachable (open() returns Err before constructing Self).
        // The raw pointer is null in the stub to make the safety argument trivially
        // correct: we never call Box::from_raw on a null pointer.
        if !self.callback_state_ptr.is_null() {
            let _ = unsafe { Box::from_raw(self.callback_state_ptr) };
        }

        debug!("VtDecodeSession dropped and invalidated");
    }
}

// ---------------------------------------------------------------------------
// VT output callback — fires on a VideoToolbox-managed thread
// ---------------------------------------------------------------------------

/// `VTDecompressionOutputCallback` implementation.
///
/// This is a `unsafe extern "C"` function registered with VideoToolbox.
/// It fires on a VideoToolbox-internal thread per decoded frame.
///
/// ## Safety Contract (caller: VideoToolbox)
///
/// - `refcon` is a valid `*mut CallbackState` for the lifetime of the session.
///   It is invalidated only after `VTDecompressionSessionInvalidate` returns.
/// - `image_buffer` is a valid `CVImageBufferRef` (i.e. `CVPixelBufferRef`) on
///   success (`status == 0`). On error, `image_buffer` may be null.
/// - This function must be non-blocking (`try_send`, no mutex acquisition).
///
/// TODO(hud-l0h6t): Wire up the real `extern "C"` signature from objc2-video-toolbox
/// on an Apple host. The function body below is a complete behavioral sketch.
#[allow(dead_code)]
unsafe extern "C" fn vt_output_callback(
    refcon: *mut std::ffi::c_void,
    // TODO(hud-l0h6t): Replace `*mut ()` with real VT types:
    //   source_frame_refcon: *mut std::ffi::c_void,
    //   status: OSStatus,
    //   info_flags: VTDecodeInfoFlags,
    //   image_buffer: CVImageBufferRef,
    //   presentation_time_stamp: CMTime,
    //   presentation_duration: CMTime,
) {
    // TODO(hud-l0h6t): Actual callback body for Apple host:
    //
    //   if status != 0 {
    //       error!(os_status = status, "VT decode callback error");
    //       return;
    //   }
    //   if image_buffer.is_null() {
    //       warn!("VT decode callback: null image_buffer on success status");
    //       return;
    //   }
    //
    //   // SAFETY: refcon is a valid `*mut CallbackState` for the lifetime of
    //   // the session (see safety contract above).
    //   let state = &*(refcon as *mut CallbackState);
    //
    //   if state.invalidated.load(Ordering::Relaxed) {
    //       return;
    //   }
    //
    //   // Lock the CVPixelBuffer for CPU read access.
    //   // kCVPixelBufferLock_ReadOnly = 1.
    //   let cv_return = CVPixelBufferLockBaseAddress(image_buffer, 1);
    //   if cv_return != 0 {
    //       error!(cv_return, "CVPixelBufferLockBaseAddress failed");
    //       return;
    //   }
    //
    //   // Copy NV12 pixel data: Y plane then UV plane.
    //   let y_base = CVPixelBufferGetBaseAddressOfPlane(image_buffer, 0) as *const u8;
    //   let uv_base = CVPixelBufferGetBaseAddressOfPlane(image_buffer, 1) as *const u8;
    //   let y_row_bytes = CVPixelBufferGetBytesPerRowOfPlane(image_buffer, 0);
    //   let uv_row_bytes = CVPixelBufferGetBytesPerRowOfPlane(image_buffer, 1);
    //   let w = state.width as usize;
    //   let h = state.height as usize;
    //
    //   let mut pixels = Vec::with_capacity(nv12_byte_count(state.width, state.height));
    //   // Copy Y plane row by row (stride may be wider than width).
    //   for row in 0..h {
    //       let src = y_base.add(row * y_row_bytes);
    //       pixels.extend_from_slice(std::slice::from_raw_parts(src, w));
    //   }
    //   // Copy UV plane row by row.
    //   for row in 0..(h + 1) / 2 {
    //       let src = uv_base.add(row * uv_row_bytes);
    //       pixels.extend_from_slice(std::slice::from_raw_parts(src, w));
    //   }
    //
    //   CVPixelBufferUnlockBaseAddress(image_buffer, 1);
    //
    //   // Derive presentation timestamp in nanoseconds from CMTime.
    //   let pts_ns = ((presentation_time_stamp.value as f64 /
    //                   presentation_time_stamp.timescale as f64) * 1_000_000_000.0) as u64;
    //
    //   let frame = DecodedFrame::new(state.width, state.height, pts_ns, pixels);
    //
    //   // Non-blocking send — drop frame if compositor is behind.
    //   if let Err(_dropped) = state.sender.try_send(frame) {
    //       warn!("VT frame dropped: compositor channel full (capacity={})", FRAME_CHANNEL_CAPACITY);
    //   }

    // Stub: refcon is unused in stub path.
    let _ = refcon;
}
