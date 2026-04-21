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
#![cfg(target_vendor = "apple")]

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime,
    CMVideoFormatDescriptionCreateFromH264ParameterSets,
    CMVideoFormatDescriptionCreateFromHEVCParameterSets, kCMTimeInvalid,
};
use objc2_core_video::{
    CVImageBuffer, CVPixelBuffer, CVPixelBufferGetBaseAddressOfPlane,
    CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_video_toolbox::{
    VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord,
    VTDecompressionSession,
};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::error::VtError;
use crate::format::{CodecParameters, VideoFormat};
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
    session_ref: NonNull<VTDecompressionSession>,
    format_desc: NonNull<CMFormatDescription>,

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

        let format_desc = match unsafe { create_format_description(&format) } {
            Ok(desc) => desc,
            Err(err) => {
                // SAFETY: `callback_state_ptr` is uniquely owned at this point.
                let _ = unsafe { Box::from_raw(callback_state_ptr) };
                return Err(err);
            }
        };

        let callback_record = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: Some(vt_output_callback),
            decompressionOutputRefCon: callback_state_ptr.cast::<c_void>(),
        };

        let mut session_ptr: *mut VTDecompressionSession = ptr::null_mut();
        // SAFETY:
        // - `format_desc` is a valid retained CoreFoundation object.
        // - callback refcon points to a valid `CallbackState`.
        // - output pointer points to stack storage for session result.
        let status = unsafe {
            VTDecompressionSession::create(
                None,
                &format_desc,
                None,
                None,
                &callback_record,
                NonNull::from(&mut session_ptr),
            )
        };
        if status != 0 {
            // SAFETY: `callback_state_ptr` is uniquely owned at this point.
            let _ = unsafe { Box::from_raw(callback_state_ptr) };
            return Err(VtError::SessionCreateFailed { os_status: status });
        }
        let Some(session_ref) = NonNull::new(session_ptr) else {
            // SAFETY: `callback_state_ptr` is uniquely owned at this point.
            let _ = unsafe { Box::from_raw(callback_state_ptr) };
            return Err(VtError::SessionCreateFailed { os_status: -1 });
        };

        // SAFETY: `session_ref` returned from `VTDecompressionSessionCreate` has +1 retain count.
        let session_retained = unsafe { CFRetained::from_raw(session_ref) };
        Ok((
            Self {
                session_ref: CFRetained::into_raw(session_retained),
                format_desc: CFRetained::into_raw(format_desc),
                callback_state_ptr,
                invalidated,
            },
            receiver,
        ))
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

        if nalu.is_empty() {
            return Err(VtError::InvalidFormat {
                detail: "AVCC frame payload is empty".into(),
            });
        }

        let mut block_buf_ptr: *mut CMBlockBuffer = ptr::null_mut();
        // SAFETY: output pointer references stack storage; all nullable pointers are null.
        let status = unsafe {
            CMBlockBuffer::create_with_memory_block(
                None,
                ptr::null_mut(),
                nalu.len(),
                None,
                ptr::null(),
                0,
                nalu.len(),
                0,
                NonNull::from(&mut block_buf_ptr),
            )
        };
        if status != 0 {
            return Err(VtError::BlockBufferCreateFailed { os_status: status });
        }
        let Some(block_buf_nonnull) = NonNull::new(block_buf_ptr) else {
            return Err(VtError::BlockBufferCreateFailed { os_status: -1 });
        };
        // SAFETY: `CMBlockBufferCreateWithMemoryBlock` returns a retained object.
        let block_buf = unsafe { CFRetained::from_raw(block_buf_nonnull) };

        // SAFETY: `nalu` is a live slice for the duration of this call.
        let source_bytes = unsafe { NonNull::new_unchecked(nalu.as_ptr() as *mut c_void) };
        // SAFETY: Source bytes are valid and destination buffer is retained.
        let status =
            unsafe { CMBlockBuffer::replace_data_bytes(source_bytes, &block_buf, 0, nalu.len()) };
        if status != 0 {
            return Err(VtError::BlockBufferCreateFailed { os_status: status });
        }

        let pts_secs = presentation_ts_ns as f64 / 1_000_000_000.0;
        let timing = CMSampleTimingInfo {
            duration: unsafe { CMTime::with_seconds(0.0, 90_000) },
            presentationTimeStamp: unsafe { CMTime::with_seconds(pts_secs, 90_000) },
            decodeTimeStamp: unsafe { kCMTimeInvalid },
        };

        let sample_sizes = [nalu.len()];
        let mut sample_buf_ptr: *mut CMSampleBuffer = ptr::null_mut();
        // SAFETY: output pointer references stack storage and all pointed-to arrays outlive call.
        let status = unsafe {
            CMSampleBuffer::create(
                None,
                Some(&block_buf),
                true,
                None,
                ptr::null_mut(),
                Some(self.format_desc.as_ref()),
                1,
                1,
                &timing,
                1,
                sample_sizes.as_ptr(),
                NonNull::from(&mut sample_buf_ptr),
            )
        };
        if status != 0 {
            return Err(VtError::SampleBufferCreateFailed { os_status: status });
        }
        let Some(sample_buf_nonnull) = NonNull::new(sample_buf_ptr) else {
            return Err(VtError::SampleBufferCreateFailed { os_status: -1 });
        };
        // SAFETY: `CMSampleBufferCreate` returns a retained object.
        let sample_buf = unsafe { CFRetained::from_raw(sample_buf_nonnull) };

        let decode_flags = VTDecodeFrameFlags::Frame_EnableAsynchronousDecompression
            | VTDecodeFrameFlags::Frame_EnableTemporalProcessing;
        let mut info_flags = VTDecodeInfoFlags::empty();
        // SAFETY: session/sample pointers are valid retained objects.
        let status = unsafe {
            self.session_ref.as_ref().decode_frame(
                &sample_buf,
                decode_flags,
                ptr::null_mut(),
                &mut info_flags,
            )
        };
        if status != 0 {
            return Err(VtError::DecodeSubmitFailed { os_status: status });
        }

        if info_flags.contains(VTDecodeInfoFlags::FrameDropped) {
            warn!(
                pts_ns = presentation_ts_ns,
                "VT decode dropped frame synchronously"
            );
        }

        Ok(())
    }
}

impl Drop for VtDecodeSession {
    fn drop(&mut self) {
        self.invalidated.store(true, Ordering::Release);

        // SAFETY: `session_ref` was created by `VTDecompressionSessionCreate`.
        // Invalidate blocks until pending callbacks complete.
        unsafe {
            self.session_ref.as_ref().invalidate();
        }

        // SAFETY: `callback_state_ptr` was created via `Box::into_raw` in
        // `VtDecodeSession::open` and is not accessed after invalidation.
        // `VTDecompressionSessionInvalidate` (called above) ensures
        // no further callbacks fire before we reclaim the Box.
        if !self.callback_state_ptr.is_null() {
            let _ = unsafe { Box::from_raw(self.callback_state_ptr) };
        }

        // SAFETY: pointers came from `CFRetained::into_raw` in `open`.
        let _ = unsafe { CFRetained::from_raw(self.session_ref) };
        // SAFETY: pointer came from `CFRetained::into_raw` in `open`.
        let _ = unsafe { CFRetained::from_raw(self.format_desc) };

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
#[allow(dead_code)]
unsafe extern "C-unwind" fn vt_output_callback(
    refcon: *mut c_void,
    _source_frame_refcon: *mut c_void,
    status: i32,
    _info_flags: VTDecodeInfoFlags,
    image_buffer: *mut CVImageBuffer,
    presentation_time_stamp: CMTime,
    _presentation_duration: CMTime,
) {
    if status != 0 {
        error!(os_status = status, "VT decode callback error");
        return;
    }
    if refcon.is_null() {
        error!("VT decode callback: null refcon");
        return;
    }
    if image_buffer.is_null() {
        warn!("VT decode callback: null image_buffer on success status");
        return;
    }

    // SAFETY: `refcon` is valid for the session lifetime (contract above).
    let state = unsafe { &*(refcon as *mut CallbackState) };
    if state.invalidated.load(Ordering::Relaxed) {
        return;
    }

    // SAFETY: Callback only receives image buffers from VT decode output.
    let pixel_buffer = unsafe { &*(image_buffer as *mut CVPixelBuffer) };
    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
    // SAFETY: `pixel_buffer` is valid for callback duration.
    let cv_return = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, lock_flags) };
    if cv_return != 0 {
        error!(cv_return, "CVPixelBufferLockBaseAddress failed");
        return;
    }

    // SAFETY: Buffer is locked; plane access is valid while locked.
    let y_base = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 0) as *const u8;
    let uv_base = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 1) as *const u8;
    let y_row_bytes = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 0);
    let uv_row_bytes = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 1);

    if y_base.is_null() || uv_base.is_null() {
        // SAFETY: Balanced unlock for successful lock above.
        let _ = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags) };
        warn!("VT decode callback: missing NV12 plane base pointers");
        return;
    }

    let w = state.width as usize;
    let h = state.height as usize;
    let mut pixels = Vec::with_capacity(nv12_byte_count(state.width, state.height));

    for row in 0..h {
        // SAFETY: row bounds are derived from frame dimensions and row stride.
        let src = unsafe { y_base.add(row * y_row_bytes) };
        // SAFETY: each row copies exactly `w` visible pixels.
        let row_bytes = unsafe { std::slice::from_raw_parts(src, w) };
        pixels.extend_from_slice(row_bytes);
    }
    for row in 0..h.div_ceil(2) {
        // SAFETY: row bounds are derived from frame dimensions and row stride.
        let src = unsafe { uv_base.add(row * uv_row_bytes) };
        // SAFETY: each UV row has `w` bytes for NV12.
        let row_bytes = unsafe { std::slice::from_raw_parts(src, w) };
        pixels.extend_from_slice(row_bytes);
    }

    // SAFETY: Balanced unlock for successful lock above.
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags) };

    let pts_ns = cm_time_to_ns(presentation_time_stamp);
    let frame = DecodedFrame::new(state.width, state.height, pts_ns, pixels);

    if let Err(send_err) = state.sender.try_send(frame) {
        match send_err {
            tokio::sync::mpsc::error::TrySendError::Full(_) => {
                warn!(
                    "VT frame dropped: compositor channel full (capacity={})",
                    FRAME_CHANNEL_CAPACITY
                );
            }
            tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                warn!("VT frame dropped: compositor channel closed");
            }
        }
    }
}

fn cm_time_to_ns(ts: CMTime) -> u64 {
    if ts.timescale <= 0 || ts.value < 0 {
        return 0;
    }
    let value = ts.value as u128;
    let timescale = ts.timescale as u128;
    let nanos = value.saturating_mul(1_000_000_000u128) / timescale;
    nanos.min(u64::MAX as u128) as u64
}

unsafe fn create_format_description(
    format: &VideoFormat,
) -> Result<CFRetained<CMFormatDescription>, VtError> {
    match &format.parameters {
        CodecParameters::H264 { sps, pps } => {
            let mut param_set_pointers = [
                NonNull::new(sps.as_ptr() as *mut u8).ok_or_else(|| VtError::InvalidFormat {
                    detail: "empty H264 SPS parameter set".into(),
                })?,
                NonNull::new(pps.as_ptr() as *mut u8).ok_or_else(|| VtError::InvalidFormat {
                    detail: "empty H264 PPS parameter set".into(),
                })?,
            ];
            let mut param_set_sizes = [sps.len(), pps.len()];
            let mut desc_ptr: *const CMFormatDescription = ptr::null();

            // SAFETY: pointers reference stack arrays with matching lengths.
            let status = unsafe {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    None,
                    param_set_pointers.len(),
                    NonNull::new(param_set_pointers.as_mut_ptr())
                        .expect("array pointer is non-null"),
                    NonNull::new(param_set_sizes.as_mut_ptr()).expect("array pointer is non-null"),
                    4,
                    NonNull::from(&mut desc_ptr),
                )
            };
            if status != 0 {
                return Err(VtError::InvalidFormat {
                    detail: format!(
                        "CMVideoFormatDescriptionCreateFromH264ParameterSets returned OSStatus {status}"
                    ),
                });
            }

            let Some(desc_nonnull) = NonNull::new(desc_ptr as *mut CMFormatDescription) else {
                return Err(VtError::InvalidFormat {
                    detail: "CMVideoFormatDescriptionCreateFromH264ParameterSets returned null description"
                        .into(),
                });
            };
            // SAFETY: CoreMedia create function returns retained description (+1).
            Ok(unsafe { CFRetained::from_raw(desc_nonnull) })
        }
        CodecParameters::Hevc { vps, sps, pps } => {
            let mut param_set_pointers = [
                NonNull::new(vps.as_ptr() as *mut u8).ok_or_else(|| VtError::InvalidFormat {
                    detail: "empty HEVC VPS parameter set".into(),
                })?,
                NonNull::new(sps.as_ptr() as *mut u8).ok_or_else(|| VtError::InvalidFormat {
                    detail: "empty HEVC SPS parameter set".into(),
                })?,
                NonNull::new(pps.as_ptr() as *mut u8).ok_or_else(|| VtError::InvalidFormat {
                    detail: "empty HEVC PPS parameter set".into(),
                })?,
            ];
            let mut param_set_sizes = [vps.len(), sps.len(), pps.len()];
            let mut desc_ptr: *const CMFormatDescription = ptr::null();

            // SAFETY: pointers reference stack arrays with matching lengths.
            let status = unsafe {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    None,
                    param_set_pointers.len(),
                    NonNull::new(param_set_pointers.as_mut_ptr())
                        .expect("array pointer is non-null"),
                    NonNull::new(param_set_sizes.as_mut_ptr()).expect("array pointer is non-null"),
                    4,
                    None,
                    NonNull::from(&mut desc_ptr),
                )
            };
            if status != 0 {
                return Err(VtError::InvalidFormat {
                    detail: format!(
                        "CMVideoFormatDescriptionCreateFromHEVCParameterSets returned OSStatus {status}"
                    ),
                });
            }

            let Some(desc_nonnull) = NonNull::new(desc_ptr as *mut CMFormatDescription) else {
                return Err(VtError::InvalidFormat {
                    detail: "CMVideoFormatDescriptionCreateFromHEVCParameterSets returned null description"
                        .into(),
                });
            };
            // SAFETY: CoreMedia create function returns retained description (+1).
            Ok(unsafe { CFRetained::from_raw(desc_nonnull) })
        }
    }
}
