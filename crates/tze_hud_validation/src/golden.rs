//! Golden reference management for Layer 2 visual regression.
//!
//! # Naming convention (spec §Layer 2 - Visual Regression via SSIM)
//!
//!   `{scene_name}_{backend}.png`
//!
//! Example: `single_tile_solid_software.png`
//!
//! # Storage
//!
//! Golden images are committed to the repository under `tests/golden/`.
//! The path is resolved relative to `CARGO_MANIFEST_DIR` at test time,
//! or via an explicit path for non-test callers.
//!
//! # Regeneration
//!
//! Golden references must only be regenerated when the rendering intentionally
//! changes. The `GoldenStore::update` function writes a new golden from the
//! supplied pixel buffer. Regeneration is controlled by the caller; tests
//! should NOT auto-regenerate (that would make tests trivially pass-able).
//!
//! # Backend identifiers
//!
//! Use these canonical backend IDs:
//! - `"software"` — llvmpipe (Linux) / WARP (Windows)
//! - `"hardware"` — any hardware-backed GPU

use crate::error::ValidationError;
use image::{ImageBuffer, Rgba};
use std::path::{Path, PathBuf};

/// Manages golden reference images on disk.
pub struct GoldenStore {
    /// Root directory where golden images are stored.
    dir: PathBuf,
}

impl GoldenStore {
    /// Create a new store backed by `dir`.
    ///
    /// The directory must exist and be readable.
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    /// Canonical path for a golden reference image.
    ///
    /// Format: `{dir}/{scene_name}_{backend}.png`
    pub fn path(&self, scene_name: &str, backend: &str) -> PathBuf {
        self.dir.join(format!("{scene_name}_{backend}.png"))
    }

    /// Load a golden reference image as RGBA8 bytes.
    ///
    /// Returns `Err(ValidationError::GoldenNotFound)` if the file does not exist.
    pub fn load(&self, scene_name: &str, backend: &str) -> Result<GoldenImage, ValidationError> {
        let path = self.path(scene_name, backend);
        if !path.exists() {
            return Err(ValidationError::GoldenNotFound {
                scene: scene_name.to_string(),
                backend: backend.to_string(),
                path,
            });
        }

        let img = image::open(&path)
            .map_err(|e| ValidationError::GoldenIo {
                path: path.clone(),
                cause: e.to_string(),
            })?
            .into_rgba8();

        let width = img.width();
        let height = img.height();
        let pixels = img.into_raw();

        Ok(GoldenImage {
            pixels,
            width,
            height,
        })
    }

    /// Write a new golden reference image from RGBA8 bytes.
    ///
    /// Creates the directory if it does not exist.
    ///
    /// This function is intentionally named `update` (not `save` or `create`)
    /// to emphasise that calling it replaces a baseline. Tests should never
    /// call this during a normal run.
    pub fn update(
        &self,
        scene_name: &str,
        backend: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PathBuf, ValidationError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| ValidationError::GoldenIo {
            path: self.dir.clone(),
            cause: e.to_string(),
        })?;

        let path = self.path(scene_name, backend);
        let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(width, height, pixels.to_vec())
            .ok_or_else(|| ValidationError::GoldenIo {
            path: path.clone(),
            cause: "pixel buffer size mismatch".to_string(),
        })?;

        img.save_with_format(&path, image::ImageFormat::Png)
            .map_err(|e| ValidationError::GoldenIo {
                path: path.clone(),
                cause: e.to_string(),
            })?;

        Ok(path)
    }
}

/// A loaded golden reference image.
#[derive(Debug, Clone)]
pub struct GoldenImage {
    /// RGBA8 pixel data, row-major.
    pub pixels: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl GoldenImage {
    /// Build from raw RGBA8 bytes.
    pub fn from_rgba8(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "pixel buffer size mismatch"
        );
        Self {
            pixels,
            width,
            height,
        }
    }
}

/// Locate the `tests/golden/` directory relative to the workspace root.
///
/// During tests, `CARGO_MANIFEST_DIR` is set to the crate's root directory.
/// We walk up to find the workspace root (where `tests/golden/` lives).
///
/// Returns `None` if the directory cannot be found.
pub fn find_golden_dir() -> Option<PathBuf> {
    // First try: environment variable override (useful in CI).
    if let Ok(d) = std::env::var("TZE_HUD_GOLDEN_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }

    // Second try: walk up from CARGO_MANIFEST_DIR to find tests/golden/.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let mut dir = PathBuf::from(manifest);
    for _ in 0..5 {
        let candidate = dir.join("tests").join("golden");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }

    None
}

/// The canonical backend ID for tests running on software GPU (llvmpipe/WARP).
pub const BACKEND_SOFTWARE: &str = "software";
/// The canonical backend ID for tests running on hardware GPU.
pub const BACKEND_HARDWARE: &str = "hardware";

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Load of non-existent file returns GoldenNotFound.
    #[test]
    fn load_missing_returns_not_found() {
        let dir = std::env::temp_dir().join("tze_hud_golden_test_missing");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = GoldenStore::new(&dir);
        let err = store.load("nonexistent_scene", "software");
        assert!(matches!(err, Err(ValidationError::GoldenNotFound { .. })));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Update then load round-trips correctly.
    #[test]
    fn update_then_load_roundtrip() {
        let dir = std::env::temp_dir().join("tze_hud_golden_test_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        let store = GoldenStore::new(&dir);

        let w = 8u32;
        let h = 8u32;
        let pixels: Vec<u8> = (0..(w * h * 4)).map(|i| (i % 256) as u8).collect();

        store
            .update("test_scene", "software", &pixels, w, h)
            .unwrap();

        let loaded = store.load("test_scene", "software").unwrap();
        assert_eq!(loaded.width, w);
        assert_eq!(loaded.height, h);
        // PNG is lossless for RGBA8 — pixels must round-trip exactly.
        assert_eq!(
            loaded.pixels, pixels,
            "PNG round-trip must preserve pixels exactly"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// Path naming convention: {scene}_{backend}.png
    #[test]
    fn path_naming_convention() {
        let store = GoldenStore::new("/tmp/golden");
        let path = store.path("single_tile_solid", "software");
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "single_tile_solid_software.png",
            "golden path must follow naming convention"
        );
    }
}
