//! Error taxonomy for the VideoToolbox wrapper.
//!
//! Every error variant maps to a stable string code suitable for structured
//! tracing and LLM-diagnostic log parsing (per engineering-bar.md §5).

use thiserror::Error;

/// Stable string error codes. These are append-only — never rename or reuse.
///
/// Used in `tracing` spans and structured log output. Codes are stable across
/// crate versions; removing or renaming a code is a breaking change.
pub mod codes {
    pub const FORMAT_INVALID: &str = "VT_FORMAT_INVALID";
    pub const SESSION_CREATE_FAILED: &str = "VT_SESSION_CREATE_FAILED";
    pub const DECODE_SUBMIT_FAILED: &str = "VT_DECODE_SUBMIT_FAILED";
    pub const BLOCK_BUFFER_CREATE_FAILED: &str = "VT_BLOCK_BUFFER_CREATE_FAILED";
    pub const SAMPLE_BUFFER_CREATE_FAILED: &str = "VT_SAMPLE_BUFFER_CREATE_FAILED";
    pub const SESSION_INVALIDATED: &str = "VT_SESSION_INVALIDATED";
    pub const PIXEL_BUFFER_LOCK_FAILED: &str = "VT_PIXEL_BUFFER_LOCK_FAILED";
    pub const UNSUPPORTED_CODEC: &str = "VT_UNSUPPORTED_CODEC";
    pub const HARDWARE_NOT_AVAILABLE: &str = "VT_HARDWARE_NOT_AVAILABLE";
    pub const FRAME_CHANNEL_CLOSED: &str = "VT_FRAME_CHANNEL_CLOSED";
}

/// Errors produced by the VideoToolbox wrapper.
///
/// `os_status` carries the raw Apple `OSStatus` integer when the failure
/// originates from a VideoToolbox or CoreMedia API call. On non-Apple targets
/// (where this crate compiles as a stub) variants that carry `OSStatus` will
/// never be constructed.
#[derive(Debug, Error)]
pub enum VtError {
    /// The supplied SPS/PPS or codec parameters are not valid.
    ///
    /// Stable code: [`codes::FORMAT_INVALID`].
    #[error("[{code}] invalid video format: {detail}", code = codes::FORMAT_INVALID)]
    InvalidFormat { detail: String },

    /// `VTDecompressionSessionCreate` returned a non-zero `OSStatus`.
    ///
    /// Stable code: [`codes::SESSION_CREATE_FAILED`].
    #[error(
        "[{code}] VTDecompressionSessionCreate failed (OSStatus {os_status})",
        code = codes::SESSION_CREATE_FAILED
    )]
    SessionCreateFailed { os_status: i32 },

    /// `VTDecompressionSessionDecodeFrame` returned a non-zero `OSStatus`.
    ///
    /// Stable code: [`codes::DECODE_SUBMIT_FAILED`].
    #[error(
        "[{code}] VTDecompressionSessionDecodeFrame failed (OSStatus {os_status})",
        code = codes::DECODE_SUBMIT_FAILED
    )]
    DecodeSubmitFailed { os_status: i32 },

    /// `CMBlockBufferCreate` or related API returned a non-zero `OSStatus`.
    ///
    /// Stable code: [`codes::BLOCK_BUFFER_CREATE_FAILED`].
    #[error(
        "[{code}] CMBlockBuffer creation failed (OSStatus {os_status})",
        code = codes::BLOCK_BUFFER_CREATE_FAILED
    )]
    BlockBufferCreateFailed { os_status: i32 },

    /// `CMSampleBufferCreate` returned a non-zero `OSStatus`.
    ///
    /// Stable code: [`codes::SAMPLE_BUFFER_CREATE_FAILED`].
    #[error(
        "[{code}] CMSampleBuffer creation failed (OSStatus {os_status})",
        code = codes::SAMPLE_BUFFER_CREATE_FAILED
    )]
    SampleBufferCreateFailed { os_status: i32 },

    /// The session was invalidated (via `VTDecompressionSessionInvalidate`)
    /// before this operation completed.
    ///
    /// Stable code: [`codes::SESSION_INVALIDATED`].
    #[error("[{code}] session was already invalidated", code = codes::SESSION_INVALIDATED)]
    SessionInvalidated,

    /// `CVPixelBufferLockBaseAddress` returned a non-zero status.
    ///
    /// Stable code: [`codes::PIXEL_BUFFER_LOCK_FAILED`].
    #[error(
        "[{code}] CVPixelBufferLockBaseAddress failed (CVReturn {cv_return})",
        code = codes::PIXEL_BUFFER_LOCK_FAILED
    )]
    PixelBufferLockFailed { cv_return: i32 },

    /// The requested codec is not supported on this platform or device.
    ///
    /// Stable code: [`codes::UNSUPPORTED_CODEC`].
    #[error(
        "[{code}] codec not supported on this platform: {codec}",
        code = codes::UNSUPPORTED_CODEC
    )]
    UnsupportedCodec { codec: String },

    /// Hardware acceleration was requested but is not available (VP9 on iOS).
    ///
    /// Stable code: [`codes::HARDWARE_NOT_AVAILABLE`].
    #[error(
        "[{code}] hardware decode not available for codec '{codec}'; use software fallback",
        code = codes::HARDWARE_NOT_AVAILABLE
    )]
    HardwareNotAvailable { codec: String },

    /// The decoded-frame receiver channel was closed before the session could
    /// deliver frames. This indicates that the compositor task has exited.
    ///
    /// Stable code: [`codes::FRAME_CHANNEL_CLOSED`].
    #[error("[{code}] decoded frame channel closed", code = codes::FRAME_CHANNEL_CLOSED)]
    FrameChannelClosed,
}

impl VtError {
    /// Returns the stable string code for this error, suitable for structured
    /// log output and LLM-diagnostic parsing.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidFormat { .. } => codes::FORMAT_INVALID,
            Self::SessionCreateFailed { .. } => codes::SESSION_CREATE_FAILED,
            Self::DecodeSubmitFailed { .. } => codes::DECODE_SUBMIT_FAILED,
            Self::BlockBufferCreateFailed { .. } => codes::BLOCK_BUFFER_CREATE_FAILED,
            Self::SampleBufferCreateFailed { .. } => codes::SAMPLE_BUFFER_CREATE_FAILED,
            Self::SessionInvalidated => codes::SESSION_INVALIDATED,
            Self::PixelBufferLockFailed { .. } => codes::PIXEL_BUFFER_LOCK_FAILED,
            Self::UnsupportedCodec { .. } => codes::UNSUPPORTED_CODEC,
            Self::HardwareNotAvailable { .. } => codes::HARDWARE_NOT_AVAILABLE,
            Self::FrameChannelClosed => codes::FRAME_CHANNEL_CLOSED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_prefix() {
        // Every code must start with "VT_" — this is the namespace convention.
        let errors: &[VtError] = &[
            VtError::InvalidFormat {
                detail: "test".into(),
            },
            VtError::SessionCreateFailed { os_status: -12906 },
            VtError::DecodeSubmitFailed { os_status: -12902 },
            VtError::BlockBufferCreateFailed { os_status: -12771 },
            VtError::SampleBufferCreateFailed { os_status: -12770 },
            VtError::SessionInvalidated,
            VtError::PixelBufferLockFailed { cv_return: -6661 },
            VtError::UnsupportedCodec {
                codec: "vp9".into(),
            },
            VtError::HardwareNotAvailable {
                codec: "vp9".into(),
            },
            VtError::FrameChannelClosed,
        ];
        for e in errors {
            assert!(
                e.code().starts_with("VT_"),
                "error code must be VT_-prefixed, got: {}",
                e.code()
            );
        }
    }

    #[test]
    fn error_display_contains_code() {
        let e = VtError::SessionCreateFailed { os_status: -12906 };
        let msg = e.to_string();
        assert!(
            msg.contains("VT_SESSION_CREATE_FAILED"),
            "Display output must embed the stable code: {msg}"
        );
        assert!(
            msg.contains("-12906"),
            "Display output must carry the OSStatus: {msg}"
        );
    }
}
