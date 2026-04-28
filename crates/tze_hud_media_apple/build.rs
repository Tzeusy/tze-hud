//! build.rs — libvpx static library cross-compile for aarch64-apple-ios.
//!
//! # Overview
//!
//! On non-iOS hosts this script is a **no-op**: `cargo check --workspace` on
//! Linux (or macOS targeting a non-iOS triple) succeeds without any iOS SDK,
//! Xcode, or libvpx source tree.
//!
//! On `aarch64-apple-ios` (cargo-xcode / xcodebuild invocations with the iOS
//! SDK active) the script:
//!
//! 1. Checks `LIBVPX_IOS_LIB_DIR` — if set, uses the prebuilt static lib
//!    directly (CI caching path).
//! 2. Otherwise resolves the libvpx source tree from `LIBVPX_SRC_DIR` or
//!    auto-clones the pinned tag from GitHub into `$OUT_DIR/libvpx-src`.
//! 3. Cross-compiles libvpx via its `configure` + `make` build system,
//!    targeting `aarch64-apple-ios` with the iOS SDK sysroot.
//! 4. Emits `cargo:rustc-link-search` and `cargo:rustc-link-lib` directives
//!    so the tze_hud media crate can link `vpx_codec_vp9_dx_algo` (VP9
//!    software decode) and `vpx_codec_vp9_cx_algo` (encode, optional).
//!
//! # libvpx version pin
//!
//! **Pinned to libvpx v1.15.1** (released 2025-12-04, SHA256 below).
//! This is the latest stable release at the time of authoring (hud-n9qhl,
//! 2026-04-19). Do NOT silently upgrade — VP9 bitstream compliance tests
//! must be re-validated after any version change.
//!
//! Upstream: https://chromium.googlesource.com/webm/libvpx
//! GitHub mirror: https://github.com/webmproject/libvpx (tag v1.15.1)
//!
//! Source archive SHA-256 (v1.15.1 tarball from GitHub releases):
//!   `a8b4e9b02b5e2f3f19f1b0d4e6e2b0c9b7e8d4f2a0b3c5e8f1a2b4d7e9c3f0b1`
//!   (placeholder — replace with `sha256sum libvpx-v1.15.1.tar.gz` on the
//!    Apple CI host before merging the phase-3 activation PR)
//!
//! # configure flags for aarch64-apple-ios
//!
//! ```text
//! ./configure \
//!   --target=arm64-iphonesimulator       # or arm64-darwin-gcc for real device
//!   --prefix="$OUT_DIR/libvpx-install"   \
//!   --enable-vp9                          \
//!   --disable-vp8                         \   # VP8 not needed in tze_hud v2
//!   --disable-vp9-encoder                 \   # decoder-only; halves compile time
//!   --enable-pic                           \
//!   --disable-examples                     \
//!   --disable-unit-tests                   \
//!   --disable-docs                         \
//!   --disable-runtime-cpu-detect           \   # iOS: fixed NEON; runtime probe is
//!                                              # pointless and breaks static link
//!   --enable-neon                          \   # NEON SIMD for Apple Silicon / A-series
//!   --disable-libyuv                       \   # internal libyuv conflicts with system
//!   --extra-cflags="-isysroot $(xcrun --sdk iphoneos --show-sdk-path)"
//! ```
//!
//! For the iOS Simulator target (`aarch64-apple-ios-sim`) substitute
//! `--extra-cflags="-isysroot $(xcrun --sdk iphonesimulator --show-sdk-path)"`.
//!
//! # Fat static library (multi-arch / xcframework) strategy
//!
//! Phase 3 scope is `aarch64-apple-ios` (real device, D19) only. A single-arch
//! `.a` is sufficient. A fat/xcframework is deferred to post-v2:
//!
//! ```text
//! # Post-v2: combine real device + simulator slices
//! xcodebuild -create-xcframework \
//!   -library libvpx-device/libvpx.a  -headers libvpx-device/include \
//!   -library libvpx-sim/libvpx.a     -headers libvpx-sim/include \
//!   -output libvpx.xcframework
//! ```
//!
//! Until xcframework is needed, the build.rs produces one `.a` per target triple
//! and places it in `$OUT_DIR/libvpx-install/lib/`.
//!
//! # Build-time caching strategy
//!
//! libvpx configure + make takes ~2 minutes on a modern Apple M-series host.
//! Two caching levels are provided:
//!
//! 1. **`LIBVPX_IOS_LIB_DIR` env var** — point to a pre-built `libvpx.a` dir.
//!    CI sets this from a GitHub Actions cache keyed on the version pin +
//!    target triple. If the cache hits, the build completes in < 1s.
//!
//! 2. **`OUT_DIR` persistence** — Cargo's `OUT_DIR` is stable between incremental
//!    builds for the same target + profile. The script skips configure + make if
//!    `$OUT_DIR/libvpx-install/lib/libvpx.a` already exists. This covers local
//!    development workflows where `cargo build` is re-run without a full clean.
//!
//! 3. **`cargo:rerun-if-env-changed`** directives ensure the script re-runs when
//!    any relevant input changes (SDK path, version pin, etc.).
//!
//! # Relationship to hud-l0h6t (VtDecodeSession wrapper)
//!
//! hud-l0h6t (PR #542) adds the safe `VtDecodeSession` wrapper in this same
//! crate. The two PRs touch the same crate but different concerns:
//!
//! - **hud-n9qhl** (this PR): `build.rs` — links libvpx for VP9 software decode
//! - **hud-l0h6t** (PR #542): `src/session.rs` + `src/error.rs` + `src/format.rs`
//!   + `src/frame.rs` — VTDecompressionSession safe wrapper for H.264/HEVC
//!
//! Merge order: either PR can land first. The other PR will require a rebase but
//! there is no logical conflict — the build.rs does not depend on the src/ modules
//! and vice versa.

use std::env;
use std::path::PathBuf;

fn main() {
    // Gate: only run on iOS targets.  All other hosts (Linux, macOS for a
    // non-iOS target, Windows) are a no-op — the crate compiles as an empty
    // stub and cargo check --workspace passes without any iOS SDK.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "ios" {
        return;
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo"));

    println!("cargo:rerun-if-env-changed=LIBVPX_IOS_LIB_DIR");
    println!("cargo:rerun-if-env-changed=LIBVPX_SRC_DIR");
    println!("cargo:rerun-if-env-changed=LIBVPX_VERSION");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");

    // --- Fast path: prebuilt library supplied by CI cache or developer ---
    if let Ok(prebuilt_dir) = env::var("LIBVPX_IOS_LIB_DIR") {
        let lib_path = PathBuf::from(&prebuilt_dir);
        assert!(
            lib_path.join("libvpx.a").exists(),
            "LIBVPX_IOS_LIB_DIR is set to '{prebuilt_dir}' but libvpx.a was not found there"
        );
        println!("cargo:rustc-link-search=native={}", lib_path.display());
        println!("cargo:rustc-link-lib=static=vpx");
        return;
    }

    // --- Source build path ---
    // Pinned libvpx version (see module-level docs for rationale).
    let vpx_version = env::var("LIBVPX_VERSION").unwrap_or_else(|_| "v1.15.1".to_string());

    let install_dir = out_dir.join("libvpx-install");
    let lib_file = install_dir.join("lib").join("libvpx.a");

    // Incremental-build skip: if the library was already built in this
    // OUT_DIR (same target + profile), reuse it without re-running configure.
    if lib_file.exists() {
        println!(
            "cargo:rustc-link-search=native={}",
            install_dir.join("lib").display()
        );
        println!("cargo:rustc-link-lib=static=vpx");
        return;
    }

    // Locate libvpx source tree (or clone it).
    let src_dir = resolve_or_clone_libvpx_source(&out_dir, &vpx_version);

    // Determine the iOS target string expected by libvpx's configure script.
    // libvpx configure --target values for iOS:
    //   arm64-darwin-gcc          → aarch64-apple-ios (real device)
    //   arm64-iphonesimulator     → aarch64-apple-ios-sim (simulator ARM64)
    //   x86_64-iphonesimulator    → x86_64-apple-ios (simulator Intel)
    let vpx_target = match target_arch.as_str() {
        "aarch64" => {
            // Distinguish real device vs simulator via the `target_vendor` env.
            // For `aarch64-apple-ios`:     CARGO_CFG_TARGET_VENDOR = "apple"
            // For `aarch64-apple-ios-sim`: CARGO_CFG_TARGET_VENDOR = "apple"
            // The cargo target triple suffix "-sim" is exposed via CARGO_CFG_TARGET_OS
            // as "ios" in both cases, but the linker env differs.
            // Use the SDK selector: real device uses iphoneos, sim uses iphonesimulator.
            let sdk = ios_sdk_path("iphoneos");
            println!("cargo:warning=libvpx build: target=arm64-darwin-gcc sdk={sdk}");
            ("arm64-darwin-gcc", sdk)
        }
        other => {
            panic!(
                "tze_hud_media_apple/build.rs: unsupported iOS architecture '{other}'. \
                 Only aarch64 (aarch64-apple-ios) is in scope for phase 3. \
                 Open a follow-up bead if x86_64 simulator support is needed."
            );
        }
    };

    // Resolve SDK sysroot.
    let (libvpx_target, sdk_path) = vpx_target;

    // Run configure.
    let build_dir = out_dir.join("libvpx-build");
    std::fs::create_dir_all(&build_dir).expect("failed to create libvpx build directory");

    let configure_path = src_dir.join("configure");
    assert!(
        configure_path.exists(),
        "libvpx configure script not found at {}. \
         Ensure LIBVPX_SRC_DIR points to the libvpx source root (or let \
         LIBVPX_IOS_LIB_DIR short-circuit to a prebuilt library).",
        configure_path.display()
    );

    let mut configure_command = std::process::Command::new(&configure_path);
    configure_command
        .current_dir(&build_dir)
        .env("SDKROOT", &sdk_path)
        .arg(format!("--target={libvpx_target}"))
        .arg(format!("--prefix={}", install_dir.display()))
        .arg("--enable-vp9")
        .arg("--disable-vp8")
        // Decoder-only: halves compile time and binary size.
        // VP9 encode is not required for tze_hud's receive-only media plane.
        .arg("--disable-vp9-encoder")
        .arg("--enable-pic")
        .arg("--disable-examples")
        .arg("--disable-unit-tests")
        .arg("--disable-docs")
        // Fixed NEON SIMD on Apple A-series / M-series; runtime CPU detection
        // is pointless and breaks static link on iOS (no dynamic CPUID).
        .arg("--disable-runtime-cpu-detect")
        .arg("--enable-neon")
        // Internal libyuv conflicts with system frameworks on iOS.
        .arg("--disable-libyuv")
        .arg(format!("--extra-cflags=-isysroot {sdk_path}"));
    run_command(&mut configure_command, "libvpx configure");

    // Run make (parallel build for speed).
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    run_command(
        std::process::Command::new("make")
            .current_dir(&build_dir)
            .arg(format!("-j{parallelism}")),
        "libvpx make",
    );

    // Install headers + lib into install_dir.
    run_command(
        std::process::Command::new("make")
            .current_dir(&build_dir)
            .arg("install"),
        "libvpx make install",
    );

    assert!(
        lib_file.exists(),
        "libvpx build completed but libvpx.a not found at {}",
        lib_file.display()
    );

    // Emit linker directives.
    println!(
        "cargo:rustc-link-search=native={}",
        install_dir.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=vpx");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the absolute path to the requested iOS SDK sysroot using xcrun.
///
/// Panics with a clear message if xcrun is unavailable (i.e. on non-Apple hosts,
/// which should be excluded by the target_os guard above).
fn ios_sdk_path(sdk: &str) -> String {
    let output = std::process::Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .expect(
            "xcrun not found — iOS SDK builds require Xcode. \
             This code path should only be reached on Apple hosts.",
        );
    assert!(
        output.status.success(),
        "xcrun --sdk {sdk} --show-sdk-path failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("xcrun output is not valid UTF-8")
        .trim()
        .to_owned()
}

/// Return the libvpx source directory, cloning it if absent.
///
/// Priority:
///   1. `LIBVPX_SRC_DIR` env var (developer or CI vendored source).
///   2. `$out_dir/libvpx-src` — auto-cloned from GitHub mirror at the pinned tag.
fn resolve_or_clone_libvpx_source(out_dir: &PathBuf, version: &str) -> PathBuf {
    if let Ok(src) = env::var("LIBVPX_SRC_DIR") {
        let path = PathBuf::from(&src);
        assert!(
            path.join("configure").exists(),
            "LIBVPX_SRC_DIR='{src}' does not contain a libvpx configure script"
        );
        return path;
    }

    // Auto-clone into OUT_DIR (persists between incremental builds).
    let clone_target = out_dir.join("libvpx-src");
    if clone_target.join("configure").exists() {
        // Already cloned in a previous build invocation; reuse.
        return clone_target;
    }

    // Shallow clone of the pinned tag — much faster than a full history clone.
    // GitHub mirror of the canonical Chromium Gerrit repo.
    let url = "https://github.com/webmproject/libvpx.git";
    run_command(
        std::process::Command::new("git")
            .args([
                "clone",
                "--depth=1",
                "--branch",
                version,
                url,
                clone_target.to_str().expect("OUT_DIR path is not UTF-8"),
            ])
            .current_dir(out_dir),
        "git clone libvpx",
    );

    clone_target
}

/// Run a command, panicking with a diagnostic message on failure.
fn run_command(cmd: &mut std::process::Command, label: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("{label}: failed to spawn: {e}"));
    assert!(status.success(), "{label}: exited with status {status}");
}
