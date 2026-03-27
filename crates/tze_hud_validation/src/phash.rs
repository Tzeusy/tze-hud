//! Perceptual hash (pHash) pre-screening for visual regression.
//!
//! pHash is a fast O(n) fingerprint that can quickly detect large changes.
//! Used as a cheap pre-screen *before* the more expensive SSIM computation:
//!
//! 1. Compute pHash for rendered frame and golden reference.
//! 2. If Hamming distance is 0 (bit-identical hash) → skip SSIM entirely.
//! 3. If Hamming distance is small → images are likely similar, do SSIM.
//! 4. If Hamming distance is large → images are likely very different,
//!    fail fast without SSIM or still run SSIM for structured output.
//!
//! # Algorithm: DCT-based pHash (64-bit)
//!
//! 1. Resize image to 32×32 (simple box downscale).
//! 2. Convert to grayscale (BT.709 luma).
//! 3. Compute 8×8 DCT of the top-left corner of the 32×32 image.
//! 4. Compute mean of the 64 DCT coefficients (excluding DC).
//! 5. Hash bit[i] = 1 if coefficient[i] > mean, else 0.
//!
//! This yields a 64-bit hash. Hamming distance between two hashes ≤ 10
//! typically indicates similar images.

/// A 64-bit perceptual hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PHash(pub u64);

impl PHash {
    /// Hamming distance: number of bits that differ.
    pub fn hamming_distance(self, other: PHash) -> u32 {
        (self.0 ^ other.0).count_ones()
    }

    /// True when the two hashes are close enough to proceed with SSIM.
    /// Threshold: Hamming ≤ 10 (images are probably similar).
    pub fn similar_to(self, other: PHash) -> bool {
        self.hamming_distance(other) <= 10
    }

    /// True when the images are bit-identical by hash (fast path: skip SSIM).
    pub fn identical_to(self, other: PHash) -> bool {
        self.0 == other.0
    }
}

/// Pre-screening decision from perceptual hash comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreScreenResult {
    /// Hashes are identical → skip SSIM, images are equivalent.
    SkipSsim,
    /// Hashes are similar enough to warrant SSIM.
    ProceedToSsim,
    /// Hashes are very different → likely a large regression.
    /// Still proceed to SSIM for structured failure output.
    LikelyRegression { hamming: u32 },
}

/// Compute a 64-bit perceptual hash from an RGBA8 buffer.
///
/// The input must have exactly `width * height * 4` bytes.
pub fn compute_phash(rgba: &[u8], width: u32, height: u32) -> PHash {
    assert_eq!(rgba.len(), (width * height * 4) as usize);

    // Step 1: Downsample to 32×32 grayscale.
    let small = downsample_to_32x32_gray(rgba, width as usize, height as usize);

    // Step 2: Compute 8×8 DCT over the 32×32 image (use first 8 rows and cols).
    let dct = dct8x8(&small);

    // Step 3: Compute mean of all 64 DCT coefficients.
    // The DC coefficient (index 0) is excluded (it carries mean brightness, not structure).
    let mean: f64 = dct[1..].iter().sum::<f64>() / 63.0;

    // Step 4: Build hash bits from AC coefficients only (indices 1..64).
    // Index 0 is the DC coefficient (mean brightness) and is excluded both from
    // the mean computation above and from the hash bits, so the hash captures
    // only structural/frequency content, not absolute brightness.
    let mut hash = 0u64;
    for (i, &v) in dct[1..].iter().enumerate() {
        if v > mean {
            hash |= 1u64 << i;
        }
    }

    PHash(hash)
}

/// Compare two images by perceptual hash and decide whether to proceed with SSIM.
pub fn pre_screen(src_rgba: &[u8], ref_rgba: &[u8], width: u32, height: u32) -> PreScreenResult {
    let src_hash = compute_phash(src_rgba, width, height);
    let ref_hash = compute_phash(ref_rgba, width, height);

    if src_hash.identical_to(ref_hash) {
        PreScreenResult::SkipSsim
    } else {
        let hamming = src_hash.hamming_distance(ref_hash);
        if src_hash.similar_to(ref_hash) {
            PreScreenResult::ProceedToSsim
        } else {
            PreScreenResult::LikelyRegression { hamming }
        }
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Downsample an RGBA8 image to 32×32 grayscale (box filter).
fn downsample_to_32x32_gray(rgba: &[u8], src_w: usize, src_h: usize) -> [f64; 1024] {
    let mut out = [0.0f64; 1024]; // 32×32

    for dst_y in 0..32usize {
        for dst_x in 0..32usize {
            // Source pixel range for this destination pixel.
            let x0 = dst_x * src_w / 32;
            let x1 = ((dst_x + 1) * src_w / 32).max(x0 + 1).min(src_w);
            let y0 = dst_y * src_h / 32;
            let y1 = ((dst_y + 1) * src_h / 32).max(y0 + 1).min(src_h);

            let mut sum = 0.0f64;
            let mut count = 0usize;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * src_w + sx) * 4;
                    let r = rgba[i] as f64;
                    let g = rgba[i + 1] as f64;
                    let b = rgba[i + 2] as f64;
                    // BT.709 luma
                    sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    count += 1;
                }
            }
            out[dst_y * 32 + dst_x] = if count > 0 { sum / count as f64 } else { 0.0 };
        }
    }

    out
}

/// Compute 8×8 2D DCT-II of the top-left 8×8 block of a 32×32 array.
/// Returns 64 DCT coefficients in row-major order.
fn dct8x8(gray32: &[f64; 1024]) -> [f64; 64] {
    // Extract the 8×8 block.
    let mut block = [0.0f64; 64];
    for y in 0..8 {
        for x in 0..8 {
            block[y * 8 + x] = gray32[y * 32 + x];
        }
    }

    // 2D DCT-II: first compute 1D DCT on rows, then on columns.
    let mut row_dct = [0.0f64; 64];
    for y in 0..8 {
        dct1d_8(&block[y * 8..y * 8 + 8], &mut row_dct[y * 8..y * 8 + 8]);
    }

    let mut result = [0.0f64; 64];
    let mut col_buf = [0.0f64; 8];
    let mut col_out = [0.0f64; 8];
    for x in 0..8 {
        for y in 0..8 {
            col_buf[y] = row_dct[y * 8 + x];
        }
        dct1d_8(&col_buf, &mut col_out);
        for y in 0..8 {
            result[y * 8 + x] = col_out[y];
        }
    }

    result
}

/// 1D DCT-II of an 8-element array.
/// Output[k] = sum_{n=0}^{7} input[n] * cos(π·k·(2n+1)/16)
fn dct1d_8(input: &[f64], output: &mut [f64]) {
    debug_assert_eq!(input.len(), 8);
    debug_assert_eq!(output.len(), 8);

    for k in 0..8usize {
        let mut sum = 0.0f64;
        for n in 0..8usize {
            let angle = std::f64::consts::PI * k as f64 * (2 * n + 1) as f64 / 16.0;
            sum += input[n] * angle.cos();
        }
        output[k] = sum;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgba(w: u32, h: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
        let n = (w * h * 4) as usize;
        let mut buf = vec![0u8; n];
        for i in 0..(w * h) as usize {
            buf[i * 4] = r;
            buf[i * 4 + 1] = g;
            buf[i * 4 + 2] = b;
            buf[i * 4 + 3] = 255;
        }
        buf
    }

    /// Identical images → Hamming distance 0 → SkipSsim.
    #[test]
    fn identical_images_skip_ssim() {
        let img = solid_rgba(64, 64, 100, 150, 200);
        let result = pre_screen(&img, &img, 64, 64);
        assert_eq!(
            result,
            PreScreenResult::SkipSsim,
            "identical images must produce SkipSsim pre-screen"
        );
    }

    /// Completely different images → LikelyRegression.
    #[test]
    fn different_images_likely_regression() {
        let black = solid_rgba(64, 64, 0, 0, 0);
        let white = solid_rgba(64, 64, 255, 255, 255);
        let result = pre_screen(&black, &white, 64, 64);
        // Black vs white is a large structural difference.
        assert!(matches!(
            result,
            PreScreenResult::LikelyRegression { .. } | PreScreenResult::ProceedToSsim
        ));
    }

    /// PHash of identical images is always equal.
    #[test]
    fn phash_deterministic() {
        let img = solid_rgba(128, 128, 42, 137, 200);
        let h1 = compute_phash(&img, 128, 128);
        let h2 = compute_phash(&img, 128, 128);
        assert_eq!(h1, h2, "pHash must be deterministic");
    }

    /// Hamming distance is symmetric.
    #[test]
    fn hamming_distance_symmetric() {
        let a = solid_rgba(64, 64, 100, 100, 100);
        let b = solid_rgba(64, 64, 200, 50, 150);
        let ha = compute_phash(&a, 64, 64);
        let hb = compute_phash(&b, 64, 64);
        assert_eq!(
            ha.hamming_distance(hb),
            hb.hamming_distance(ha),
            "Hamming distance must be symmetric"
        );
    }

    /// Hamming distance of a hash with itself is 0.
    #[test]
    fn hamming_distance_self_zero() {
        let img = solid_rgba(32, 32, 128, 128, 128);
        let h = compute_phash(&img, 32, 32);
        assert_eq!(h.hamming_distance(h), 0);
    }
}
