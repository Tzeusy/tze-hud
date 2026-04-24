//! Video format descriptors for `VTDecompressionSession`.
//!
//! A `VideoFormat` bundles the codec type with the out-of-band codec parameters
//! (SPS/PPS for H.264, VPS/SPS/PPS for HEVC) needed to construct a
//! `CMVideoFormatDescription` before opening a decode session.
//!
//! On non-Apple targets, `VideoFormat` is a pure Rust data type with no platform
//! dependencies — it compiles and is usable for unit tests on Linux.

use crate::error::VtError;

/// Which video codec is in use.
///
/// Only codecs that can be wrapped in a `VTDecompressionSession` are listed.
/// VP9 is absent because `VTIsHardwareDecodeSupported(kCMVideoCodecType_VP9)`
/// returns false on iOS; see `docs/audits/ios-videotoolbox-alternative-audit.md`
/// §1.2 and §5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Codec {
    /// H.264 / AVC — hardware decode on all iOS devices (iOS 8+).
    H264,
    /// HEVC / H.265 — hardware decode on iOS 11+, A9 chip and later.
    /// Out of v2 scope; included for forward-compatibility.
    Hevc,
}

impl Codec {
    /// Returns the human-readable name for error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
        }
    }
}

/// Out-of-band codec parameters needed to open a `VTDecompressionSession`.
///
/// For H.264, these are the SPS and PPS NAL units extracted from the SDP offer
/// or from inline in-band parameter sets in the RTP stream.
///
/// For HEVC, these are the VPS, SPS, and PPS NAL units.
///
/// These bytes are passed to `CMVideoFormatDescriptionCreateFromH264ParameterSets`
/// (H.264) or `CMVideoFormatDescriptionCreateFromHEVCParameterSets` (HEVC) on
/// Apple targets.
#[derive(Debug, Clone)]
pub enum CodecParameters {
    /// H.264 parameter sets.
    H264 {
        /// Sequence Parameter Set NAL unit (without start code prefix).
        sps: Vec<u8>,
        /// Picture Parameter Set NAL unit (without start code prefix).
        pps: Vec<u8>,
    },
    /// HEVC parameter sets.
    Hevc {
        /// Video Parameter Set NAL unit.
        vps: Vec<u8>,
        /// Sequence Parameter Set NAL unit.
        sps: Vec<u8>,
        /// Picture Parameter Set NAL unit.
        pps: Vec<u8>,
    },
}

/// A video format descriptor that can be used to open a `VtDecodeSession`.
///
/// Construct via [`VideoFormat::h264_from_parameter_sets`] or
/// [`VideoFormat::hevc_from_parameter_sets`].
///
/// ## Lifetime
///
/// `VideoFormat` owns copies of all parameter set bytes. It is `Clone` and may
/// be shared across threads. On Apple targets, `VtDecodeSession::open` consumes
/// a `VideoFormat` and converts it to a `CMVideoFormatDescription` internally.
#[derive(Debug, Clone)]
pub struct VideoFormat {
    pub(crate) codec: Codec,
    // Read by the Apple-target session module; on non-Apple builds the session
    // module is cfg-gated away, so suppress the dead-code lint here.
    #[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
    pub(crate) parameters: CodecParameters,
    /// Frame width in pixels (informational; also encoded in SPS).
    pub(crate) width: u32,
    /// Frame height in pixels (informational; also encoded in SPS).
    pub(crate) height: u32,
}

impl VideoFormat {
    /// Construct an H.264 video format from raw SPS and PPS NAL unit bytes.
    ///
    /// `sps` and `pps` must not include start code prefixes (`00 00 00 01` or
    /// `00 00 01`). Strip them before calling this function.
    ///
    /// `width` and `height` are the coded picture size in pixels. These values
    /// are also encoded in the SPS — they are accepted here as a convenience
    /// for callers that parse the SDP `a=imageattr` attribute.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::InvalidFormat`] if `sps` or `pps` is empty.
    pub fn h264_from_parameter_sets(
        sps: impl Into<Vec<u8>>,
        pps: impl Into<Vec<u8>>,
        width: u32,
        height: u32,
    ) -> Result<Self, VtError> {
        let sps = sps.into();
        let pps = pps.into();

        if sps.is_empty() {
            return Err(VtError::InvalidFormat {
                detail: "SPS parameter set is empty".into(),
            });
        }
        if pps.is_empty() {
            return Err(VtError::InvalidFormat {
                detail: "PPS parameter set is empty".into(),
            });
        }
        if width == 0 || height == 0 {
            return Err(VtError::InvalidFormat {
                detail: format!("invalid coded picture size: {width}×{height}"),
            });
        }

        Ok(Self {
            codec: Codec::H264,
            parameters: CodecParameters::H264 { sps, pps },
            width,
            height,
        })
    }

    /// Construct an HEVC video format from raw VPS, SPS, and PPS NAL unit bytes.
    ///
    /// Start code prefixes must be stripped before calling this function.
    ///
    /// # Errors
    ///
    /// Returns [`VtError::InvalidFormat`] if any parameter set is empty.
    pub fn hevc_from_parameter_sets(
        vps: impl Into<Vec<u8>>,
        sps: impl Into<Vec<u8>>,
        pps: impl Into<Vec<u8>>,
        width: u32,
        height: u32,
    ) -> Result<Self, VtError> {
        let vps = vps.into();
        let sps = sps.into();
        let pps = pps.into();

        if vps.is_empty() || sps.is_empty() || pps.is_empty() {
            return Err(VtError::InvalidFormat {
                detail: "HEVC parameter sets must not be empty (VPS, SPS, PPS required)".into(),
            });
        }
        if width == 0 || height == 0 {
            return Err(VtError::InvalidFormat {
                detail: format!("invalid coded picture size: {width}×{height}"),
            });
        }

        Ok(Self {
            codec: Codec::Hevc,
            parameters: CodecParameters::Hevc { vps, sps, pps },
            width,
            height,
        })
    }

    /// The codec type for this format.
    pub fn codec(&self) -> &Codec {
        &self.codec
    }

    /// Coded picture width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Coded picture height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal synthetic SPS for testing: not a real bitstream but non-empty.
    const FAKE_SPS: &[u8] = &[0x67, 0x42, 0xc0, 0x1f];
    const FAKE_PPS: &[u8] = &[0x68, 0xce, 0x38, 0x80];

    #[test]
    fn h264_format_rejects_empty_sps() {
        let err = VideoFormat::h264_from_parameter_sets(&[][..], FAKE_PPS, 1920, 1080).unwrap_err();
        assert_eq!(err.code(), crate::error::codes::FORMAT_INVALID);
        assert!(err.to_string().contains("SPS"));
    }

    #[test]
    fn h264_format_rejects_empty_pps() {
        let err = VideoFormat::h264_from_parameter_sets(FAKE_SPS, &[][..], 1920, 1080).unwrap_err();
        assert_eq!(err.code(), crate::error::codes::FORMAT_INVALID);
        assert!(err.to_string().contains("PPS"));
    }

    #[test]
    fn h264_format_rejects_zero_dimensions() {
        let err = VideoFormat::h264_from_parameter_sets(FAKE_SPS, FAKE_PPS, 0, 1080).unwrap_err();
        assert_eq!(err.code(), crate::error::codes::FORMAT_INVALID);
        // The error message must mention the invalid dimension (0).
        assert!(
            err.to_string().contains("0"),
            "expected '0' in error: {err}"
        );
    }

    #[test]
    fn h264_format_constructs_successfully() {
        let fmt = VideoFormat::h264_from_parameter_sets(FAKE_SPS, FAKE_PPS, 1920, 1080)
            .expect("valid H.264 format");
        assert_eq!(fmt.codec(), &Codec::H264);
        assert_eq!(fmt.width(), 1920);
        assert_eq!(fmt.height(), 1080);
    }

    #[test]
    fn hevc_format_rejects_empty_vps() {
        let err = VideoFormat::hevc_from_parameter_sets(&[][..], FAKE_SPS, FAKE_PPS, 1920, 1080)
            .unwrap_err();
        assert_eq!(err.code(), crate::error::codes::FORMAT_INVALID);
    }

    #[test]
    fn hevc_format_constructs_successfully() {
        let fake_vps = &[0x40u8, 0x01, 0x0c, 0x01];
        let fmt = VideoFormat::hevc_from_parameter_sets(fake_vps, FAKE_SPS, FAKE_PPS, 3840, 2160)
            .expect("valid HEVC format");
        assert_eq!(fmt.codec(), &Codec::Hevc);
        assert_eq!(fmt.width(), 3840);
        assert_eq!(fmt.height(), 2160);
    }
}
