//! build.rs — Android GStreamer SDK linker bridge for tze_hud_media_android.
//!
//! On non-Android hosts this script is a no-op: `cargo check --workspace` on
//! Linux passes without `GSTREAMER_ROOT_ANDROID` being set.
//!
//! On Android targets (cargo-ndk invocation) it:
//!   1. Reads `GSTREAMER_ROOT_ANDROID` — path to the extracted GStreamer SDK.
//!   2. Maps `CARGO_NDK_ANDROID_TARGET` (Android ABI name) → GStreamer SDK
//!      subdirectory name (e.g. "arm64-v8a" → "arm64").
//!   3. Emits `cargo:rustc-link-*` directives for gstreamer-full and the
//!      required Android system libraries.
//!
//! Reference: docs/ci/android-gstreamer-bootstrap.md §3.3 (hud-685ha, PR #539)
//!
//! ABI mapping (CARGO_NDK_ANDROID_TARGET → GStreamer SDK directory):
//!   arm64-v8a   → arm64    (aarch64-linux-android — D19 Pixel, P0)
//!   x86_64      → x86_64   (x86_64-linux-android  — emulator CI, P0)
//!   armeabi-v7a → armv7    (out of phase 3 scope)
//!   x86         → x86      (out of phase 3 scope)
//!
//! cargo-ndk environment variables read by this script:
//!   CARGO_NDK_ANDROID_TARGET   Android ABI name (not the Rust triple)
//!   CARGO_NDK_ANDROID_PLATFORM Android API level (28 for phase 3)
//!   ANDROID_NDK_HOME           NDK r27 root

use std::env;
use std::path::PathBuf;

fn main() {
    // Gate on target OS — this script is a no-op on non-Android hosts.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }

    let gst_root = env::var("GSTREAMER_ROOT_ANDROID")
        .expect("GSTREAMER_ROOT_ANDROID must be set for Android builds");

    // cargo-ndk sets CARGO_NDK_ANDROID_TARGET to the Android ABI name.
    // Default to arm64-v8a (the P0 target, D19 Pixel) when building directly
    // with cargo without cargo-ndk (e.g. for IDE inspection).
    let abi = env::var("CARGO_NDK_ANDROID_TARGET").unwrap_or_else(|_| "arm64-v8a".to_string());

    // Map Android ABI → GStreamer SDK subdirectory.
    // Note: cargo-ndk uses "arm64-v8a", NOT "arm64" and NOT the Rust triple.
    // The fallback in hud-685ha PR #539 was corrected to "arm64-v8a" (not "arm64").
    let gst_abi_dir = match abi.as_str() {
        "arm64-v8a" => "arm64",
        "x86_64" => "x86_64",
        "armeabi-v7a" => "armv7",
        "x86" => "x86",
        other => panic!("Unsupported Android ABI: {other}"),
    };

    let lib_dir = PathBuf::from(&gst_root).join(gst_abi_dir).join("lib");
    let pkgconfig_dir = lib_dir.join("pkgconfig");

    // Emit link search path for GStreamer static archives.
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Link gstreamer-full — monolithic static archive (all plugins bundled).
    // GStreamer 1.24 always ships libgstreamer-full-1.0.a; see spike doc §5.2.
    println!("cargo:rustc-link-lib=static=gstreamer-full-1.0");

    // Required Android system libraries (always dynamic).
    println!("cargo:rustc-link-lib=dylib=android");
    println!("cargo:rustc-link-lib=dylib=log");
    println!("cargo:rustc-link-lib=dylib=z");

    // Re-run when environment changes — prevents stale cached builds.
    println!("cargo:rerun-if-env-changed=GSTREAMER_ROOT_ANDROID");
    println!("cargo:rerun-if-env-changed=CARGO_NDK_ANDROID_TARGET");
    println!("cargo:rerun-if-env-changed=CARGO_NDK_ANDROID_PLATFORM");

    // pkgconfig_dir is reserved for future per-plugin pkg-config queries
    // (e.g. libssl, libcrypto for encrypted streams).  Not used in the
    // minimal bootstrap; suppress the unused-variable warning until needed.
    let _ = pkgconfig_dir;
}
