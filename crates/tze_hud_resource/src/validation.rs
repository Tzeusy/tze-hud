//! Upload validation pipeline.
//!
//! Implements the six-step validation sequence from RFC 0011 §3.5 and
//! resource-store/spec.md lines 83-87:
//!
//! 1. `upload_resource` capability check
//! 2. BLAKE3 hash integrity — computed hash must match `expected_hash`
//! 3. Per-resource size limit (raw bytes ≤ `max_resource_bytes`)
//! 4. Agent `texture_bytes_total` budget check
//! 5. V1-supported resource type check
//! 6. Content decode check (images decode, fonts parse)
//!
//! Each step is a stand-alone function so callers can short-circuit or
//! re-use individual checks.

use crate::types::{
    DecodedMeta, ResourceError, ResourceStoreConfig, ResourceType,
    MAX_TEXTURE_DIMENSION_PX,
};

// ─── Step 1: Capability check ─────────────────────────────────────────────────

/// The string capability name required to upload resources.
pub const CAPABILITY_UPLOAD_RESOURCE: &str = "upload_resource";

/// Verify that `agent_capabilities` contains `upload_resource`.
///
/// Spec: lines 88-90, RFC 0011 §5.2.
pub fn check_capability(agent_capabilities: &[String]) -> Result<(), ResourceError> {
    if agent_capabilities
        .iter()
        .any(|c| c == CAPABILITY_UPLOAD_RESOURCE)
    {
        Ok(())
    } else {
        Err(ResourceError::CapabilityDenied)
    }
}

// ─── Step 2: BLAKE3 hash integrity ────────────────────────────────────────────

/// Compute the BLAKE3 hash of `data` and verify it matches `expected`.
///
/// Spec: lines 66-68, RFC 0011 §3.4.
///
/// BLAKE3 achieves ~3 GB/s on modern hardware; 1 MiB hashes in < 1 ms
/// (spec line 330).
pub fn check_hash(data: &[u8], expected: &[u8; 32]) -> Result<[u8; 32], ResourceError> {
    let hash = blake3::hash(data);
    let computed = *hash.as_bytes();
    if computed == *expected {
        Ok(computed)
    } else {
        let computed_hex: String = computed
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let expected_hex: String = expected
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Err(ResourceError::HashMismatch {
            computed: computed_hex,
            expected: expected_hex,
        })
    }
}

// ─── Step 3: Per-resource size limit ──────────────────────────────────────────

/// Verify that `byte_count` does not exceed `config.max_resource_bytes`.
///
/// Spec: lines 286-288, RFC 0011 §8.1.
pub fn check_raw_size(byte_count: usize, config: &ResourceStoreConfig) -> Result<(), ResourceError> {
    if byte_count > config.max_resource_bytes {
        Err(ResourceError::SizeExceeded {
            detail: format!(
                "raw upload size {byte_count} bytes exceeds limit {} bytes",
                config.max_resource_bytes
            ),
        })
    } else {
        Ok(())
    }
}

// ─── Step 4: Agent budget check ───────────────────────────────────────────────

/// Per-agent budget context for an upload.
#[derive(Clone, Debug)]
pub struct AgentBudget {
    /// Agent's `texture_bytes_total` lease limit (0 = unlimited).
    pub texture_bytes_total_limit: usize,
    /// Agent's current `texture_bytes_total` consumption.
    pub texture_bytes_total_used: usize,
}

impl AgentBudget {
    /// Return `true` if `additional_bytes` would fit within the agent's budget.
    pub fn would_fit(&self, additional_bytes: usize) -> bool {
        if self.texture_bytes_total_limit == 0 {
            return true; // unlimited
        }
        self.texture_bytes_total_used
            .checked_add(additional_bytes)
            .map(|total| total <= self.texture_bytes_total_limit)
            .unwrap_or(false)
    }
}

/// Verify that `decoded_bytes` fits within the agent's texture budget and the
/// runtime-wide texture ceiling.
///
/// Spec: lines 96-98, 300-303, RFC 0011 §8.3, §11.2.
pub fn check_budget(
    decoded_bytes: usize,
    agent: &AgentBudget,
    runtime_total_texture_bytes_used: usize,
    config: &ResourceStoreConfig,
) -> Result<(), ResourceError> {
    // Runtime-wide ceiling.
    if runtime_total_texture_bytes_used
        .checked_add(decoded_bytes)
        .map(|total| total > config.max_total_texture_bytes)
        .unwrap_or(true)
    {
        return Err(ResourceError::BudgetExceeded {
            detail: format!(
                "runtime-wide texture memory {runtime_total_texture_bytes_used} + {decoded_bytes} \
                 would exceed limit {}",
                config.max_total_texture_bytes
            ),
        });
    }

    // Per-agent limit.
    if !agent.would_fit(decoded_bytes) {
        return Err(ResourceError::BudgetExceeded {
            detail: format!(
                "agent texture budget {}/{} would be exceeded by {decoded_bytes} decoded bytes",
                agent.texture_bytes_total_used, agent.texture_bytes_total_limit
            ),
        });
    }

    Ok(())
}

// ─── Step 5: V1 type check ────────────────────────────────────────────────────

/// Verify that `resource_type` is in the v1-supported set.
///
/// Spec: lines 41-42, RFC 0011 §2.2.
pub fn check_resource_type(resource_type: ResourceType) -> Result<(), ResourceError> {
    if resource_type.is_v1_supported() {
        Ok(())
    } else {
        Err(ResourceError::UnsupportedType(resource_type.to_string()))
    }
}

// ─── Step 6: Decode validation ────────────────────────────────────────────────

/// Attempt to decode `data` as the given `resource_type`, producing
/// `DecodedMeta`.
///
/// For images: full decode to RGBA8 — validates pixel count and dimension
/// limits.  For fonts: parse the font file (no full render).
///
/// # Decompression bomb defense
///
/// If a PNG/JPEG decodes to a texture exceeding `config.max_decoded_texture_bytes`
/// or to a dimension > `MAX_TEXTURE_DIMENSION_PX`, the decode is aborted with
/// `ResourceError::SizeExceeded` (spec lines 290-292).
///
/// Spec: lines 93-94, RFC 0011 §3.5, step 6.
pub fn decode_and_validate(
    data: &[u8],
    resource_type: ResourceType,
    config: &ResourceStoreConfig,
) -> Result<DecodedMeta, ResourceError> {
    match resource_type {
        ResourceType::ImageRgba8 => validate_raw_rgba8(data, config),
        ResourceType::ImagePng => decode_image(data, resource_type, config),
        ResourceType::ImageJpeg => decode_image(data, resource_type, config),
        ResourceType::FontTtf | ResourceType::FontOtf => validate_font(data),
    }
}

fn validate_raw_rgba8(
    data: &[u8],
    config: &ResourceStoreConfig,
) -> Result<DecodedMeta, ResourceError> {
    // For raw RGBA8, the decoded size IS the input size.
    // Require the byte count to be a multiple of 4.
    if data.len() % 4 != 0 {
        return Err(ResourceError::DecodeError(
            "IMAGE_RGBA8 byte count must be a multiple of 4".into(),
        ));
    }

    let pixel_count = data.len() / 4;
    // Minimum degenerate check — cannot determine w/h from raw bytes alone;
    // the protocol message carries width/height separately.  Here we just
    // check the decoded size cap.
    if data.len() > config.max_decoded_texture_bytes {
        return Err(ResourceError::SizeExceeded {
            detail: format!(
                "raw RGBA8 decoded size {} bytes exceeds limit {}",
                data.len(),
                config.max_decoded_texture_bytes
            ),
        });
    }

    // Infer dimensions from pixel count assuming square for validation purposes.
    // Callers should pass explicit dimensions through the upload protocol fields;
    // this fallback just provides a rough dimension estimate.
    let side = (pixel_count as f64).sqrt() as u32;
    Ok(DecodedMeta {
        decoded_bytes: data.len(),
        width_px: side,
        height_px: side,
    })
}

fn decode_image(
    data: &[u8],
    resource_type: ResourceType,
    config: &ResourceStoreConfig,
) -> Result<DecodedMeta, ResourceError> {
    use image::ImageFormat;

    let format = match resource_type {
        ResourceType::ImagePng => ImageFormat::Png,
        ResourceType::ImageJpeg => ImageFormat::Jpeg,
        _ => unreachable!("only PNG and JPEG are routed here"),
    };

    // Decode to RGBA8.  The `image` crate will return an error for corrupt data.
    let img = image::load_from_memory_with_format(data, format).map_err(|e| {
        ResourceError::DecodeError(format!("{resource_type}: {e}"))
    })?;

    let width_px = img.width();
    let height_px = img.height();

    // Dimension check (decompression bomb defense, spec lines 290-292).
    if width_px > MAX_TEXTURE_DIMENSION_PX || height_px > MAX_TEXTURE_DIMENSION_PX {
        return Err(ResourceError::SizeExceeded {
            detail: format!(
                "{resource_type} dimensions {width_px}x{height_px} exceed \
                 maximum {MAX_TEXTURE_DIMENSION_PX}"
            ),
        });
    }

    // Decoded in-memory size as RGBA8 (4 bytes per pixel).
    let decoded_bytes = (width_px as usize)
        .checked_mul(height_px as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| ResourceError::SizeExceeded {
            detail: format!(
                "{resource_type} decoded size overflows usize at {width_px}x{height_px}"
            ),
        })?;

    if decoded_bytes > config.max_decoded_texture_bytes {
        return Err(ResourceError::SizeExceeded {
            detail: format!(
                "{resource_type} decoded size {decoded_bytes} bytes exceeds limit {}",
                config.max_decoded_texture_bytes
            ),
        });
    }

    Ok(DecodedMeta {
        decoded_bytes,
        width_px,
        height_px,
    })
}

fn validate_font(data: &[u8]) -> Result<DecodedMeta, ResourceError> {
    // Use ttf-parser to validate the font file.  We do not fully render glyphs
    // here; we just verify the font parses successfully.
    ttf_parser::Face::parse(data, 0)
        .map_err(|e| ResourceError::DecodeError(format!("font parse error: {e:?}")))?;

    Ok(DecodedMeta {
        decoded_bytes: data.len(),
        width_px: 0,
        height_px: 0,
    })
}

// ─── Convenience: run all six validation steps ────────────────────────────────

/// Run all six validation steps for a completed upload and return `DecodedMeta`.
///
/// Callers should run the hash check *before* calling this function when they
/// want to validate the hash inline (e.g., after chunked upload completes).
/// This function does *not* re-hash; pass `expected_hash` for the check.
///
/// For the dedup-hit fast path (step 2 reveals the resource is already known),
/// callers skip steps 3-6 entirely — no re-validation needed.
pub fn validate_upload(
    data: &[u8],
    expected_hash: &[u8; 32],
    resource_type: ResourceType,
    agent_capabilities: &[String],
    agent_budget: &AgentBudget,
    runtime_total_texture_bytes_used: usize,
    config: &ResourceStoreConfig,
) -> Result<DecodedMeta, ResourceError> {
    // 1. Capability gate.
    check_capability(agent_capabilities)?;

    // 2. Hash integrity.
    check_hash(data, expected_hash)?;

    // 3. Raw size limit.
    check_raw_size(data.len(), config)?;

    // 5. Type check (before decode to short-circuit unsupported types early).
    check_resource_type(resource_type)?;

    // 6. Decode validation (also validates decoded size limits).
    let meta = decode_and_validate(data, resource_type, config)?;

    // 4. Budget check (after decode so we know the true decoded size).
    check_budget(
        meta.decoded_bytes,
        agent_budget,
        runtime_total_texture_bytes_used,
        config,
    )?;

    Ok(meta)
}

/// Test helpers shared across crate-internal test modules.
///
/// Gated on `#[cfg(test)]` so they are not compiled into library consumers.
#[cfg(test)]
pub mod test_helpers {
    /// Return a minimal valid 1×1 red RGBA PNG (70 bytes).
    ///
    /// Bytes verified with Python `struct`/`zlib` and PIL that CRCs are correct
    /// and the image decodes to a 1×1 RGBA pixel.
    pub fn minimal_png_1x1() -> Vec<u8> {
        // 70-byte valid 1×1 red (ff0000ff) RGBA8 PNG.
        // Byte sequence verified with Python PIL: Image.open(BytesIO(data))
        // returns a 1×1 RGBA image.
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
            0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
            0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41,
            0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
            0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x56, 0xc7,
            0x2f, 0x0d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
            0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> ResourceStoreConfig {
        ResourceStoreConfig::default()
    }

    fn caps_with_upload() -> Vec<String> {
        vec![CAPABILITY_UPLOAD_RESOURCE.to_string()]
    }

    fn caps_empty() -> Vec<String> {
        vec![]
    }

    fn unlimited_budget() -> AgentBudget {
        AgentBudget {
            texture_bytes_total_limit: 0,
            texture_bytes_total_used: 0,
        }
    }

    // ─── Step 1: capability ───────────────────────────────────────────────────

    #[test]
    fn capability_check_pass() {
        assert!(check_capability(&caps_with_upload()).is_ok());
    }

    #[test]
    fn capability_check_denied_for_empty_caps() {
        // Acceptance: guest agent rejected with RESOURCE_CAPABILITY_DENIED
        // (spec lines 88-90).
        let err = check_capability(&caps_empty()).unwrap_err();
        assert_eq!(err, ResourceError::CapabilityDenied);
        assert_eq!(err.wire_code(), "RESOURCE_CAPABILITY_DENIED");
    }

    // ─── Step 2: hash integrity ───────────────────────────────────────────────

    #[test]
    fn hash_check_pass() {
        let data = b"valid data";
        let hash = *blake3::hash(data).as_bytes();
        assert!(check_hash(data, &hash).is_ok());
    }

    #[test]
    fn hash_check_mismatch() {
        // Acceptance: RESOURCE_HASH_MISMATCH when computed ≠ expected
        // (spec lines 66-68).
        let data = b"some bytes";
        let wrong_hash = [0xffu8; 32];
        let err = check_hash(data, &wrong_hash).unwrap_err();
        assert!(matches!(err, ResourceError::HashMismatch { .. }));
        assert_eq!(err.wire_code(), "RESOURCE_HASH_MISMATCH");
    }

    // ─── Step 3: raw size ─────────────────────────────────────────────────────

    #[test]
    fn raw_size_pass() {
        let config = default_config();
        assert!(check_raw_size(1024, &config).is_ok());
    }

    #[test]
    fn raw_size_exceeded() {
        // Acceptance: 20 MiB rejected (spec lines 286-288).
        let config = default_config();
        let twenty_mib = 20 * 1024 * 1024;
        let err = check_raw_size(twenty_mib, &config).unwrap_err();
        assert!(matches!(err, ResourceError::SizeExceeded { .. }));
        assert_eq!(err.wire_code(), "RESOURCE_SIZE_EXCEEDED");
    }

    // ─── Step 4: budget ───────────────────────────────────────────────────────

    #[test]
    fn budget_check_unlimited_always_passes() {
        let config = default_config();
        let budget = unlimited_budget();
        // Even a large decoded size passes if limit=0 (unlimited).
        assert!(check_budget(100_000_000, &budget, 0, &config).is_ok());
    }

    #[test]
    fn budget_check_agent_exceeded() {
        // Acceptance: RESOURCE_BUDGET_EXCEEDED when agent limit is full
        // (spec lines 96-98).
        let config = default_config();
        let budget = AgentBudget {
            texture_bytes_total_limit: 10 * 1024 * 1024,
            texture_bytes_total_used: 10 * 1024 * 1024, // fully consumed
        };
        let err = check_budget(1024, &budget, 0, &config).unwrap_err();
        assert!(matches!(err, ResourceError::BudgetExceeded { .. }));
        assert_eq!(err.wire_code(), "RESOURCE_BUDGET_EXCEEDED");
    }

    #[test]
    fn budget_check_runtime_exceeded() {
        // Acceptance: runtime-wide limit rejected (spec lines 300-303).
        let config = default_config();
        let budget = unlimited_budget();
        // Simulate runtime at 512 MiB ceiling.
        let runtime_used = 512 * 1024 * 1024;
        let err = check_budget(1024, &budget, runtime_used, &config).unwrap_err();
        assert!(matches!(err, ResourceError::BudgetExceeded { .. }));
    }

    // ─── Step 5: type check ───────────────────────────────────────────────────

    #[test]
    fn type_check_png_accepted() {
        // Acceptance: IMAGE_PNG accepted (spec lines 37-38).
        assert!(check_resource_type(ResourceType::ImagePng).is_ok());
    }

    // Note: all v1 variants exist in the enum; there is no way to construct an
    // unknown type without changing the enum, so the unsupported-type path is
    // tested via the wire-code unit test in types.rs.

    // ─── Step 6: decode validation ────────────────────────────────────────────

    #[test]
    fn decode_rgba8_valid() {
        // 2×2 image = 16 bytes of RGBA8.
        let data = vec![0u8; 16];
        let config = default_config();
        let meta = decode_and_validate(&data, ResourceType::ImageRgba8, &config).unwrap();
        assert_eq!(meta.decoded_bytes, 16);
    }

    #[test]
    fn decode_rgba8_non_multiple_of_4_rejected() {
        let data = vec![0u8; 15]; // not divisible by 4
        let config = default_config();
        let err = decode_and_validate(&data, ResourceType::ImageRgba8, &config).unwrap_err();
        assert!(matches!(err, ResourceError::DecodeError(_)));
    }

    #[test]
    fn decode_png_corrupt_rejected() {
        // Acceptance: corrupt PNG → RESOURCE_DECODE_ERROR (spec lines 93-94).
        let corrupt_png = b"this is not a valid png";
        let config = default_config();
        let err =
            decode_and_validate(corrupt_png, ResourceType::ImagePng, &config).unwrap_err();
        assert!(matches!(err, ResourceError::DecodeError(_)));
        assert_eq!(err.wire_code(), "RESOURCE_DECODE_ERROR");
    }

    #[test]
    fn decode_jpeg_corrupt_rejected() {
        let corrupt_jpeg = b"not a jpeg at all";
        let config = default_config();
        let err =
            decode_and_validate(corrupt_jpeg, ResourceType::ImageJpeg, &config).unwrap_err();
        assert!(matches!(err, ResourceError::DecodeError(_)));
    }

    #[test]
    fn decode_font_corrupt_rejected() {
        let corrupt_font = b"not a valid font file";
        let config = default_config();
        let err =
            decode_and_validate(corrupt_font, ResourceType::FontTtf, &config).unwrap_err();
        assert!(matches!(err, ResourceError::DecodeError(_)));
        assert_eq!(err.wire_code(), "RESOURCE_DECODE_ERROR");
    }

    #[test]
    fn decode_rgba8_size_exceeded() {
        // Acceptance: decompression bomb defense (spec lines 290-292).
        let config = ResourceStoreConfig {
            max_decoded_texture_bytes: 8, // very small for test
            ..Default::default()
        };
        let data = vec![0u8; 16]; // 16 > 8 limit
        let err =
            decode_and_validate(&data, ResourceType::ImageRgba8, &config).unwrap_err();
        assert!(matches!(err, ResourceError::SizeExceeded { .. }));
    }

    use super::test_helpers::minimal_png_1x1;

    #[test]
    fn decode_png_1x1_succeeds() {
        let config = default_config();
        let data = minimal_png_1x1();
        let meta = decode_and_validate(&data, ResourceType::ImagePng, &config).unwrap();
        assert_eq!(meta.width_px, 1);
        assert_eq!(meta.height_px, 1);
        assert_eq!(meta.decoded_bytes, 4); // 1×1×4 bytes RGBA8
    }

    #[test]
    fn agent_budget_unlimited_fits_any_size() {
        let budget = AgentBudget {
            texture_bytes_total_limit: 0,
            texture_bytes_total_used: 0,
        };
        assert!(budget.would_fit(usize::MAX));
    }

    #[test]
    fn agent_budget_limited_fits() {
        let budget = AgentBudget {
            texture_bytes_total_limit: 1000,
            texture_bytes_total_used: 500,
        };
        assert!(budget.would_fit(500));
        assert!(!budget.would_fit(501));
    }
}
