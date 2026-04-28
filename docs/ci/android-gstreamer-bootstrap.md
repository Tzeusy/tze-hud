# Android GStreamer SDK CI Bootstrap

**Issued for**: `hud-685ha`
**Date**: 2026-04-19
**Author**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-685ha (Android GStreamer SDK CI bootstrap, pre-phase-3)
**Discovered from**: hud-4znng — Android GStreamer SDK build-system spike (PR #536)
**Companion audit**: `docs/audits/android-gstreamer-sdk-build-spike.md`

---

## Purpose

This document is the pre-phase-3 CI bootstrap plan for the Android GStreamer SDK
integration. It must be completed and its gate checklist validated before the
phase 3 Android implementation bead opens.

Phase 3 target device: **D19 — 1x Pixel (aarch64-linux-android)**
Phase 3 scope (from hud-4znng verdict HYBRID-NATIVE-MEDIACODEC): GStreamer Android SDK
with `androidmedia` plugin (MediaCodec backend) as primary decode path; `ndk::media`
direct bindings as fallback.

This document covers:
1. Authoritative SDK artifact (version, tarball URL)
2. NDK version + API level matrix
3. cargo-ndk 3.5+ link validation strategy
4. GitHub Actions workflow sketch for CI validation
5. Known pitfalls
6. Pre-phase-3 gate checklist

---

## 1. GStreamer Android SDK Artifact

### 1.1 Version Selection

**Recommended CI pin: GStreamer 1.24.13**

GStreamer 1.24 remains the bootstrap anchor because:
- OMX was removed in 1.24; the only hardware-decode path is `androidmedia` (MediaCodec).
  Any version prior to 1.24 would require handling OMX removal as a migration concern.
- GStreamer 1.24 aligns with NDK r25c minimum and r27 recommended (tested by the
  upstream Cerbero build system).
- gstreamer-rs 0.22.x targets GStreamer 1.24 (stable bindings; no major API breaks
  expected within the 1.24.x series).

**MR #4115 status (confirmed by hud-fts60 / PR #540)**: MR #4115 shipped in
GStreamer 1.24 (merged January 11, 2024), not 1.26 as originally speculated.
The `AMediaCodec*` C API path reduces per-frame JNI overhead (codec start/stop/
dequeue/queue operations), but does **not** eliminate the `gst_android_init` /
JVM bootstrap requirement — codec enumeration and surface-to-ANativeWindow
conversion still use JNI. All `JNI_OnLoad` ordering constraints in Sections 5.3
and 6 remain in force for GStreamer 1.24.x.

At phase 3 kickoff, verify whether `HAVE_NDKMEDIA` activates in the selected
prebuilt or source-built SDK. MR #4115 is merged in GStreamer 1.24, but PR #617
showed that the public 1.24.13 prebuilt material did not expose confirmable
`gst_amc_*_ndk` symbols or `AMediaCodec_createCodecByName` strings through the
current archive inspection gate. Treat this as unresolved until a source-built
Cerbero log or device/runtime probe confirms the NDK path.

If upgrading past GStreamer 1.24, plan for the Android.mk → CMake build system
migration (Android.mk deprecated upstream in 1.26 and removed in 1.28). Evaluate
upgrading only for a security, linker, or runtime fix that outweighs that migration.

### 1.2 Tarball URLs

**Base URL**: `https://gstreamer.freedesktop.org/data/pkg/android/<version>/`

Replace `<version>` with the pinned release (currently `1.24.13`).

| Tarball | ABI | Current availability | CI use |
|---|---|---|---|
| `gstreamer-1.0-android-universal-<version>.tar.xz` | All ABIs | Published for `1.24.13` | Bootstrap CI |
| `gstreamer-1.0-android-arm64-<version>.tar.xz` | `aarch64-linux-android` | Not published in the public `1.24.13` directory | Do not assume |
| `gstreamer-1.0-android-x86_64-<version>.tar.xz` | `x86_64-linux-android` | Not published in the public `1.24.13` directory | Do not assume |

**Current public SDK observation (2026-04-29)**: the GStreamer public Android
listing for `1.24.13` publishes only `gstreamer-1.0-android-universal-1.24.13.tar.xz`,
its signature, and its `.sha256sum`; the per-ABI `arm64` and `x86_64` archives
assumed by the original sketch are absent. The live workflow correctly downloads
the universal archive and caches it under a `universal` cache key.

**Version pinning**: Pin the exact version in CI (e.g., `1.24.13`) via an environment
variable or workflow input. Do not use `latest` — GStreamer does not publish a
stable-latest alias. Check for new patch releases at phase 3 kickoff and update the
pin if security or linker fixes are present.

**SHA-256 verification**: GStreamer publishes `.sha256sum` files alongside each
tarball at the same base URL. Always verify:

```bash
# Example for the current 1.24.13 public archive
wget https://gstreamer.freedesktop.org/data/pkg/android/1.24.13/gstreamer-1.0-android-universal-1.24.13.tar.xz
wget https://gstreamer.freedesktop.org/data/pkg/android/1.24.13/gstreamer-1.0-android-universal-1.24.13.tar.xz.sha256sum
sha256sum -c gstreamer-1.0-android-universal-1.24.13.tar.xz.sha256sum
```

### 1.3 Extraction Layout

After extraction, the SDK is pointed to via `GSTREAMER_ROOT_ANDROID`:

```
$GSTREAMER_ROOT_ANDROID/
  arm64/                          ← aarch64-linux-android
    lib/
      pkgconfig/                  ← .pc files for cross-pkg-config
      libgstreamer-full-1.0.a     ← main static archive (all plugins bundled)
      gstreamer-1.0/              ← may be absent in current public 1.24.13
    include/gstreamer-1.0/
  x86_64/                         ← x86_64-linux-android (emulator)
    lib/
      pkgconfig/
      libgstreamer-full-1.0.a
      gstreamer-1.0/              ← may be absent in current public 1.24.13
    include/gstreamer-1.0/
```

PR #617 observed the current public `1.24.13` universal archive extracting with
`arm64/lib/` static archives and `arm64/lib/pkgconfig/`, but without a separate
`arm64/lib/gstreamer-1.0/libgstandroidmedia.so`. The live workflow therefore
accepts the arm64 static archive set as the material to inspect for `androidmedia`
symbols when the separate plugin `.so` is absent.

---

## 2. NDK Version and API Level Matrix

### 2.1 NDK Version Matrix

| NDK Version | Status | Clang | Notes |
|---|---|---|---|
| r21 | Not recommended | Clang 9 | LTS but outdated; GStreamer 1.24 Cerbero targets r25+. |
| r25c | Minimum viable | Clang 14 | Stable libc++; no GCC remnants; acceptable for initial bootstrap. |
| r27 | **Recommended (pinned)** | Clang 18 | Current stable NDK (2024). Improved LLD linker; fixes static init ordering bugs in complex C++ (GLib/GObject). `ANDROID_NDK_HOME` should point here. |
| r28 | Monitor | — | Not yet released as of April 2026. Do not pin until GStreamer Cerbero validates. |

**Pin NDK r27 for phase 3.** NDK r27 is the version used by GStreamer's own CI and
cross-file examples (`android_arm64_api28.txt` in gst-build). Using a different NDK
version risks subtle linker errors or ABI mismatches in the prebuilt static archives.

**GitHub Actions**: Use `android-actions/setup-android@v3` or download the NDK
directly from `dl.google.com/android/repository/`. The NDK is pinned by revision
string (e.g., `27.2.12479018`). The exact revision can be looked up at:
`https://developer.android.com/ndk/downloads`

### 2.2 API Level Matrix

| API Level | Android Version | Status | Notes |
|---|---|---|---|
| API 21 | Android 5.0 (Lollipop) | **Minimum** | 64-bit ABI support introduced. `NdkMediaCodec.h` available. GStreamer minimum. |
| API 23 | Android 6.0 | Recommended floor | `AMediaCodec_createDecoderByType` stable. |
| API 28 | Android 9.0 (Pie) | **Recommended target** | Used in GStreamer's own cross-file. Full MediaCodec feature coverage. Pixel 3+ baseline. |
| API 34 | Android 14 | Current latest | No additional GStreamer requirements; acceptable for modern-only support. |

**Recommendation**: Compile with `--platform 28` (`-p 28` in cargo-ndk) for the initial
phase 3 build. This sets `__ANDROID_API__=28` in the NDK sysroot and enables all
MediaCodec APIs needed for the `androidmedia` plugin. The resulting `.so` will run on
any device at API 21+ (GStreamer's minimum) but may emit unavailability warnings for
APIs > 28 on older devices if they are probed.

### 2.3 Rust Target Matrix for Phase 3

| Rust triple | ABI | Use | Priority |
|---|---|---|---|
| `aarch64-linux-android` | arm64-v8a | Physical Pixel device (D19) | **Required P0** |
| `x86_64-linux-android` | x86_64 | Android emulator on x86-64 CI runner | **Required P0** |
| `armv7-linux-androideabi` | armeabi-v7a | 32-bit ARM (legacy) | Out of phase 3 scope |
| `i686-linux-android` | x86 | 32-bit x86 emulator | Out of phase 3 scope |

```bash
rustup target add aarch64-linux-android x86_64-linux-android
```

---

## 3. cargo-ndk 3.5+ Link Validation Strategy

### 3.1 Why cargo-ndk

`cargo-ndk` (https://github.com/bbqsrc/cargo-ndk) wraps `cargo build` with:
- Correct linker (`$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android28-clang`)
- Sysroot (`--sysroot $NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot`)
- ABI-aware output placement (`-o jniLibs/` → `jniLibs/arm64-v8a/`)

Version 3.5+ is required for stable GStreamer static archive linking (prior versions
had a regression in how they pass `-L` search paths to the linker when
`cargo:rustc-link-search` emits multiple paths from `build.rs`).

### 3.2 Installation

```bash
cargo install cargo-ndk --version ">=3.5" --locked
```

Verify:

```bash
cargo ndk --version
# Expected: cargo-ndk 3.5.x or later
```

### 3.3 build.rs Requirements

The Rust media crate (`tze_hud_media` or the Android-specific shim) needs a
`build.rs` that:

1. Detects the Android target triple.
2. Reads `GSTREAMER_ROOT_ANDROID` from the environment.
3. Runs cross-pkg-config to get GStreamer linker flags.
4. Emits `cargo:rustc-link-*` directives.

**Reference `build.rs` skeleton**:

```rust
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "android" {
        // Non-Android: use system GStreamer (handled by pkg-config normally)
        return;
    }

    let gst_root = env::var("GSTREAMER_ROOT_ANDROID")
        .expect("GSTREAMER_ROOT_ANDROID must be set for Android builds");

    // cargo-ndk sets CARGO_NDK_ANDROID_TARGET to the ABI name (e.g., "arm64-v8a")
    // Map ABI to GStreamer SDK subdirectory name.
    // Default to "arm64-v8a" (not "arm64") so the match below resolves correctly
    // when CARGO_NDK_ANDROID_TARGET is absent (e.g., direct cargo build, not cargo-ndk).
    let abi = env::var("CARGO_NDK_ANDROID_TARGET").unwrap_or_else(|_| "arm64-v8a".to_string());
    let gst_abi_dir = match abi.as_str() {
        "arm64-v8a" => "arm64",
        "x86_64"    => "x86_64",
        "armeabi-v7a" => "armv7",
        "x86"       => "x86",
        other => panic!("Unsupported Android ABI: {other}"),
    };

    let lib_dir = PathBuf::from(&gst_root).join(gst_abi_dir).join("lib");
    let pkgconfig_dir = lib_dir.join("pkgconfig");

    // Cross-pkg-config: use the GStreamer SDK's .pc files
    // Requires pkg-config to support cross-compilation (PKG_CONFIG_ALLOW_CROSS=1)
    let pkgconfig_path = pkgconfig_dir.display().to_string();

    // Emit link search path for the GStreamer static archives
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Link gstreamer-full (all plugins statically bundled)
    println!("cargo:rustc-link-lib=static=gstreamer-full-1.0");

    // Required Android system libraries
    println!("cargo:rustc-link-lib=dylib=android");
    println!("cargo:rustc-link-lib=dylib=log");
    println!("cargo:rustc-link-lib=dylib=z");

    // Re-run if env vars change
    println!("cargo:rerun-if-env-changed=GSTREAMER_ROOT_ANDROID");
    println!("cargo:rerun-if-env-changed=CARGO_NDK_ANDROID_TARGET");

    // Optional: run pkg-config to get additional -l flags for gst plugins
    // that are not folded into gstreamer-full (e.g., libssl, libcrypto if needed).
    // Skip for minimal bootstrap; add per-plugin as required.
    let _ = pkgconfig_path; // suppress unused warning until pkg-config call is added
}
```

**Key environment variables set by cargo-ndk** that `build.rs` can read:

| Variable | Example value | Use |
|---|---|---|
| `CARGO_NDK_ANDROID_TARGET` | `arm64-v8a` | Map to GStreamer SDK ABI dir |
| `CARGO_NDK_ANDROID_PLATFORM` | `28` | API level for sysroot |
| `ANDROID_NDK_HOME` | `/opt/ndk/27.x` | NDK root |
| `CC` | `aarch64-linux-android28-clang` | C compiler |
| `AR` | `llvm-ar` | Archiver |

### 3.4 Link Validation Test

The link validation CI step compiles a minimal Rust `.so` that imports GStreamer
symbols and verifies the linker succeeds without unresolved symbols:

```bash
# Create a minimal Cargo project that links against gstreamer-full
cat > /tmp/gst_link_probe/src/lib.rs << 'EOF'
// Minimal GStreamer symbol probe — verifies link succeeds
#[no_mangle]
pub extern "C" fn probe_gst_version() -> u32 {
    // Link probe: if this compiles, gstreamer-full is correctly linked
    extern "C" {
        fn gst_version_string() -> *const std::ffi::c_char;
    }
    unsafe { gst_version_string() as u32 }
}
EOF

cargo ndk \
  --target aarch64-linux-android \
  --platform 28 \
  build --release 2>&1 | tail -20
```

Success condition: `Finished release [optimized]` with no linker errors.
Common linker failures and their causes are listed in Section 5 (Pitfalls).

### 3.5 pkg-config Cross-Compilation Probe

Before the cargo-ndk build, verify cross-pkg-config resolves GStreamer:

```bash
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_PATH=$GSTREAMER_ROOT_ANDROID/arm64/lib/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot

pkg-config --modversion gstreamer-1.0
# Expected output: 1.24.x

pkg-config --libs gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0
# Expected: a list of -L and -l flags pointing into GSTREAMER_ROOT_ANDROID/arm64/lib/
```

If `pkg-config` returns the host system's GStreamer instead of the SDK's, the
`PKG_CONFIG_PATH` override is not taking effect. Common cause: a `PKG_CONFIG`
env var pointing to the host pkg-config binary that ignores `PKG_CONFIG_PATH`.
Fix: use `PKG_CONFIG=/usr/bin/pkg-config` explicitly or install `cross-pkg-config`.

---

## 4. GitHub Actions Workflow Sketch

The following YAML sketch is a **proposed** CI job for validating the Android
GStreamer bootstrap. It is not yet added to `.github/workflows/ci.yml`. Phase 3
kickoff is the correct time to integrate it (or add as a separate
`.github/workflows/android-bootstrap.yml`).

**Note**: As of PR #541 (hud-dr5yf) + PR #542 (hud-04b00), the workflow file
`.github/workflows/android-bootstrap.yml` exists and includes Gates 1–6 (see
Section 6 for Gate 6 HAVE_NDKMEDIA details). The sketch below reflects the original
5-gate design from hud-685ha. The live file is the authoritative source.

**Update from PR #617 / hud-5dlpd**: the live workflow should keep Gate 6 as a
warning-level audit. The public `1.24.13` archive allowed pkg-config and cargo-ndk
link probes to pass for both Android targets, but `HAVE_NDKMEDIA` could not be
confirmed from the archive symbols/strings. Hard-gating on `HAVE_NDKMEDIA` should
wait for source-built Cerbero evidence or a runtime/device probe.

```yaml
# .github/workflows/android-bootstrap.yml
# Pre-phase-3 validation: Android GStreamer SDK CI bootstrap gate
#
# This workflow validates the minimal build chain required before
# the phase 3 Android implementation bead can open:
#   - GStreamer Android SDK download + checksum
#   - NDK r27 environment setup
#   - Rust android targets installed
#   - cargo-ndk 3.5+ version verified
#   - pkg-config cross-probe succeeds (gstreamer-1.0 resolves)
#   - Rust .so links against gstreamer-full without linker errors
#
# Runs on: manual dispatch + PRs touching android/* or docs/ci/*
# Does NOT run on every push (expensive download step; ~90s per ABI).

name: Android GStreamer Bootstrap

on:
  workflow_dispatch:
    inputs:
      gst_version:
        description: "GStreamer version (e.g. 1.24.12)"
        required: true
        default: "1.24.12"
  pull_request:
    paths:
      - "docs/ci/android-gstreamer-bootstrap.md"
      - ".github/workflows/android-bootstrap.yml"
      - "crates/**/build.rs"

env:
  # Pin these at phase 3 kickoff; update via PR when bumping
  GST_VERSION: ${{ github.event.inputs.gst_version || '1.24.12' }}
  NDK_VERSION: "27.2.12479018"
  ANDROID_API_LEVEL: "28"
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  android-gstreamer-bootstrap:
    name: Android GStreamer SDK bootstrap gate
    runs-on: ubuntu-latest
    steps:
      # ── Checkout ──────────────────────────────────────────────────────────
      - uses: actions/checkout@v4

      # ── Rust toolchain + Android targets ─────────────────────────────────
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.88"
          targets: "aarch64-linux-android,x86_64-linux-android"

      - uses: Swatinem/rust-cache@v2

      # ── cargo-ndk ─────────────────────────────────────────────────────────
      - name: Install cargo-ndk (>=3.5)
        run: |
          cargo install cargo-ndk --version ">=3.5" --locked
          cargo ndk --version

      # ── Android NDK via SDK manager ───────────────────────────────────────
      - name: Install Android NDK r27
        uses: android-actions/setup-android@v3
        with:
          packages: "ndk;${{ env.NDK_VERSION }}"

      - name: Verify NDK installation
        run: |
          echo "ANDROID_NDK_HOME=$ANDROID_NDK_HOME"
          $ANDROID_NDK_HOME/ndk-build --version || \
            $ANDROID_NDK_HOME/build/ndk-build --version || \
            echo "NDK revision: $(cat $ANDROID_NDK_HOME/source.properties | grep Revision)"

      # ── GStreamer Android SDK download + verify ───────────────────────────
      - name: Cache GStreamer Android SDK
        id: cache-gst
        uses: actions/cache@v4
        with:
          path: /opt/gstreamer-android
          key: gstreamer-android-${{ env.GST_VERSION }}-arm64-x86_64

      - name: Download GStreamer Android SDK (arm64 + x86_64)
        if: steps.cache-gst.outputs.cache-hit != 'true'
        run: |
          mkdir -p /opt/gstreamer-android-dl /opt/gstreamer-android

          GST_BASE="https://gstreamer.freedesktop.org/data/pkg/android/${{ env.GST_VERSION }}"

          for ABI in arm64 x86_64; do
            TARBALL="gstreamer-1.0-android-${ABI}-${{ env.GST_VERSION }}.tar.xz"
            echo "Downloading ${TARBALL}..."
            wget -q -P /opt/gstreamer-android-dl "${GST_BASE}/${TARBALL}"
            wget -q -P /opt/gstreamer-android-dl "${GST_BASE}/${TARBALL}.sha256sum"

            echo "Verifying checksum..."
            cd /opt/gstreamer-android-dl
            sha256sum -c "${TARBALL}.sha256sum"

            echo "Extracting ${TARBALL}..."
            tar xf "${TARBALL}" -C /opt/gstreamer-android
          done

          ls /opt/gstreamer-android/

      - name: Set GSTREAMER_ROOT_ANDROID
        run: echo "GSTREAMER_ROOT_ANDROID=/opt/gstreamer-android" >> $GITHUB_ENV

      # ── Gate 1: GStreamer SDK structure present ───────────────────────────
      - name: Gate 1 — GStreamer SDK static archives present
        run: |
          COUNT=$(ls $GSTREAMER_ROOT_ANDROID/arm64/lib/*.a 2>/dev/null | wc -l)
          echo "arm64 static archives: $COUNT"
          if [ "$COUNT" -lt 50 ]; then
            echo "FAIL: Expected >50 static archives in arm64/lib/, found $COUNT"
            exit 1
          fi
          echo "PASS: GStreamer arm64 SDK has $COUNT static archives"

      # ── Gate 2: androidmedia .so present ─────────────────────────────────
      - name: Gate 2 — androidmedia plugin .so present
        run: |
          PLUGIN="$GSTREAMER_ROOT_ANDROID/arm64/lib/gstreamer-1.0/libgstandroidmedia.so"
          if [ ! -f "$PLUGIN" ]; then
            echo "FAIL: libgstandroidmedia.so not found at $PLUGIN"
            exit 1
          fi
          echo "PASS: libgstandroidmedia.so present"
          ls -lh "$PLUGIN"

      # ── Gate 3: cross-pkg-config resolves gstreamer-1.0 ──────────────────
      - name: Gate 3 — cross-pkg-config probe
        run: |
          export PKG_CONFIG_ALLOW_CROSS=1
          export PKG_CONFIG_PATH=$GSTREAMER_ROOT_ANDROID/arm64/lib/pkgconfig
          export PKG_CONFIG_SYSROOT_DIR=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot

          VERSION=$(pkg-config --modversion gstreamer-1.0)
          echo "pkg-config resolved gstreamer-1.0: $VERSION"

          # Must start with the pinned GST_VERSION major.minor
          EXPECTED_PREFIX=$(echo "${{ env.GST_VERSION }}" | cut -d. -f1-2)
          if [[ "$VERSION" != "$EXPECTED_PREFIX"* ]]; then
            echo "FAIL: Expected gstreamer-1.0 version ~$EXPECTED_PREFIX, got $VERSION"
            exit 1
          fi
          echo "PASS: gstreamer-1.0 $VERSION resolves correctly via cross-pkg-config"

      # ── Gate 4: cargo-ndk links against gstreamer-full ───────────────────
      - name: Gate 4 — cargo-ndk link probe (aarch64-linux-android)
        run: |
          # Create a minimal probe crate in a temp directory
          PROBE_DIR=$(mktemp -d)
          mkdir -p "$PROBE_DIR/src"

          cat > "$PROBE_DIR/Cargo.toml" << 'TOML'
          [package]
          name = "gst-link-probe"
          version = "0.1.0"
          edition = "2021"

          [lib]
          crate-type = ["cdylib"]
          TOML

          cat > "$PROBE_DIR/src/lib.rs" << 'RUST'
          // Minimal GStreamer link probe.
          // If this compiles for aarch64-linux-android, gstreamer-full is correctly linked.
          extern "C" {
              fn gst_version(major: *mut u32, minor: *mut u32, micro: *mut u32, nano: *mut u32);
          }

          #[no_mangle]
          pub extern "C" fn gst_probe_version() -> u32 {
              let (mut major, mut minor, mut micro, mut nano) = (0u32, 0u32, 0u32, 0u32);
              unsafe { gst_version(&mut major, &mut minor, &mut micro, &mut nano); }
              (major << 24) | (minor << 16) | (micro << 8) | nano
          }
          RUST

          cat > "$PROBE_DIR/build.rs" << 'BUILDRS'
          fn main() {
              let gst_root = std::env::var("GSTREAMER_ROOT_ANDROID")
                  .expect("GSTREAMER_ROOT_ANDROID required");
              let abi = std::env::var("CARGO_NDK_ANDROID_TARGET")
                  .unwrap_or_else(|_| "arm64-v8a".to_string());
              let gst_abi = match abi.as_str() {
                  "arm64-v8a" => "arm64",
                  "x86_64"   => "x86_64",
                  other      => panic!("Unsupported ABI: {other}"),
              };
              let lib_dir = format!("{}/{}/lib", gst_root, gst_abi);
              println!("cargo:rustc-link-search=native={}", lib_dir);
              println!("cargo:rustc-link-lib=static=gstreamer-full-1.0");
              println!("cargo:rustc-link-lib=dylib=android");
              println!("cargo:rustc-link-lib=dylib=log");
              println!("cargo:rustc-link-lib=dylib=z");
              println!("cargo:rerun-if-env-changed=GSTREAMER_ROOT_ANDROID");
          }
          BUILDRS

          echo "Running cargo ndk link probe..."
          cd "$PROBE_DIR"
          cargo ndk \
            --target aarch64-linux-android \
            --platform ${{ env.ANDROID_API_LEVEL }} \
            build --release 2>&1 | tee /tmp/cargo-ndk-probe.log

          if grep -q "^error" /tmp/cargo-ndk-probe.log; then
            echo "FAIL: cargo-ndk link probe produced errors"
            cat /tmp/cargo-ndk-probe.log
            exit 1
          fi

          if ! grep -q "Finished release" /tmp/cargo-ndk-probe.log; then
            echo "FAIL: cargo-ndk link probe did not finish successfully"
            cat /tmp/cargo-ndk-probe.log
            exit 1
          fi

          echo "PASS: cargo-ndk linked gstreamer-full successfully for aarch64-linux-android"

      # ── Gate 5: x86_64 emulator target probe ─────────────────────────────
      - name: Gate 5 — cargo-ndk link probe (x86_64-linux-android)
        run: |
          # Repeat the link probe for the emulator target
          PROBE_DIR=$(mktemp -d)
          mkdir -p "$PROBE_DIR/src"

          # (same Cargo.toml / src/lib.rs / build.rs as above)
          cat > "$PROBE_DIR/Cargo.toml" << 'TOML'
          [package]
          name = "gst-link-probe-x86"
          version = "0.1.0"
          edition = "2021"

          [lib]
          crate-type = ["cdylib"]
          TOML

          cat > "$PROBE_DIR/src/lib.rs" << 'RUST'
          extern "C" {
              fn gst_version(major: *mut u32, minor: *mut u32, micro: *mut u32, nano: *mut u32);
          }

          #[no_mangle]
          pub extern "C" fn gst_probe_version() -> u32 {
              let (mut major, mut minor, mut micro, mut nano) = (0u32, 0u32, 0u32, 0u32);
              unsafe { gst_version(&mut major, &mut minor, &mut micro, &mut nano); }
              (major << 24) | (minor << 16) | (micro << 8) | nano
          }
          RUST

          cat > "$PROBE_DIR/build.rs" << 'BUILDRS'
          fn main() {
              let gst_root = std::env::var("GSTREAMER_ROOT_ANDROID")
                  .expect("GSTREAMER_ROOT_ANDROID required");
              let abi = std::env::var("CARGO_NDK_ANDROID_TARGET")
                  .unwrap_or_else(|_| "x86_64".to_string());
              let gst_abi = match abi.as_str() {
                  "arm64-v8a" => "arm64",
                  "x86_64"   => "x86_64",
                  other      => panic!("Unsupported ABI: {other}"),
              };
              let lib_dir = format!("{}/{}/lib", gst_root, gst_abi);
              println!("cargo:rustc-link-search=native={}", lib_dir);
              println!("cargo:rustc-link-lib=static=gstreamer-full-1.0");
              println!("cargo:rustc-link-lib=dylib=android");
              println!("cargo:rustc-link-lib=dylib=log");
              println!("cargo:rustc-link-lib=dylib=z");
              println!("cargo:rerun-if-env-changed=GSTREAMER_ROOT_ANDROID");
          }
          BUILDRS

          cd "$PROBE_DIR"
          cargo ndk \
            --target x86_64-linux-android \
            --platform ${{ env.ANDROID_API_LEVEL }} \
            build --release 2>&1 | tee /tmp/cargo-ndk-probe-x86.log

          if grep -q "^error" /tmp/cargo-ndk-probe-x86.log; then
            echo "FAIL: x86_64 link probe produced errors"
            exit 1
          fi
          echo "PASS: cargo-ndk linked gstreamer-full for x86_64-linux-android"

      # ── Summary ───────────────────────────────────────────────────────────
      - name: Bootstrap gate summary
        run: |
          echo "=== Android GStreamer Bootstrap Gate Summary ==="
          echo "GStreamer version : ${{ env.GST_VERSION }}"
          echo "NDK version       : ${{ env.NDK_VERSION }}"
          echo "API level         : ${{ env.ANDROID_API_LEVEL }}"
          echo "All 5 gates passed — Android GStreamer CI bootstrap validated."
          echo "Pre-phase-3 gate is GREEN. Phase 3 Android bead may open."
```

---

## 5. Known Pitfalls

### 5.1 `CARGO_NDK_ANDROID_TARGET` vs ABI directory naming

cargo-ndk sets `CARGO_NDK_ANDROID_TARGET` to the **Android ABI name** (e.g.,
`arm64-v8a`, `x86_64`), not the Rust triple (`aarch64-linux-android`). The
GStreamer SDK uses its own directory names (`arm64`, `x86_64`). The `build.rs`
must map between these. See the reference `build.rs` in Section 3.3.

### 5.2 gstreamer-full vs individual static archives

The GStreamer Android SDK prebuilt distributes a `libgstreamer-full-1.0.a`
monolithic archive in recent versions, but older versions (pre-1.20) required
linking individual plugin archives (`libgstcoreelements.a`, `libgstapp.a`, etc.)
plus explicit `gst_plugin_register_static()` calls. For GStreamer 1.24, always
use `gstreamer-full` — it handles plugin registration via constructor functions.

If `libgstreamer-full-1.0.a` is missing from the extracted SDK, the tarball
may be corrupted or an incorrect ABI tarball was downloaded. Verify by checking
`arm64/lib/` for a file matching `libgstreamer-full*.a`.

### 5.3 JVM bootstrap ordering (androidmedia)

`libgstandroidmedia.so` requires JVM initialization before GStreamer can register
the `androidmedia` plugin. The ordering must be:

```
JNI_OnLoad  →  gst_android_init(env, context)  →  [event loop starts]  →  gst_init()
```

If `gst_android_init()` is not called before `gst_init()`, the `androidmedia`
plugin silently fails to register its JNI callbacks, and hardware decode falls
back to software without any error. The only symptom is `amcvideodec` not
appearing in `gst-inspect-1.0` on the device.

**CI impact**: This ordering constraint cannot be validated in CI without a JVM
(and therefore a real Android device or emulator). Gates 1–5 in the CI workflow
above validate the build chain only. Ordering validation happens in Phase 3
on-device tests.

### 5.4 GStreamer SDK tarball naming variation

GStreamer occasionally changes the tarball filename format between releases:
- GStreamer ≤1.22: `gstreamer-1.0-android-universal-<version>.tar.bz2`
- GStreamer 1.24+: `gstreamer-1.0-android-<abi>-<version>.tar.xz` (per-ABI)

The universal tarball moved to `.tar.xz` format (from `.tar.bz2`) in 1.24.
Pin the file extension in CI scripts and update if the format changes in 1.26.

### 5.5 pkg-config sysroot double-prepending

When both `PKG_CONFIG_SYSROOT_DIR` and a `--sysroot` clang flag are active,
some versions of `pkg-config` will double-prepend the sysroot to include paths,
producing paths like `/path/to/sysroot/path/to/sysroot/include`. If cross-compile
errors mention include files not found with an obviously doubled path, unset
`PKG_CONFIG_SYSROOT_DIR` and pass include paths manually via `CFLAGS`.

### 5.6 LLD linker and thin archives

NDK r27 uses LLD by default. LLD is strict about thin archives (`.a` files with
`--thin` flag) and may reject them if created by a different version of `llvm-ar`.
If the GStreamer SDK was built with an older Cerbero toolchain and the resulting
`.a` files are thin archives, LLD may emit `malformed archive` errors.

Workaround: check if the GStreamer SDK was built with a matching NDK version. The
SDK download page lists the NDK version used in the Cerbero build. If there is a
mismatch, use an older NDK version that matches the SDK build, or rebuild the SDK
from source with Cerbero using NDK r27.

### 5.7 `libgstandroidmedia.so` symbol resolution at runtime

`libgstandroidmedia.so` is built against the Android system JNI (loaded via
`System.loadLibrary` from Java side). It links against `libdvm.so` or `libart.so`
(the Android runtime). If the `.so` was built against an API level higher than the
device's API, or if the JVM has not been initialized when `dlopen` loads it,
the dynamic linker will report unresolved symbols.

**Symptom**: `dlopen("libgstandroidmedia.so", ...) failed: cannot locate symbol
"JNI_GetCreatedJavaVMs"`. This means `gst_android_init()` was not called in time.

### 5.8 Rust version pinned to 1.88

The main CI pins Rust toolchain to `1.88`. The Android bootstrap workflow should
use the same pin for consistency. Diverging toolchain versions between the desktop
and Android CI may produce confusing `Cargo.lock` conflicts.

### 5.9 NDK r27 revision string

The NDK download page lists NDK r27 with multiple patch revisions
(e.g., `27.2.12479018`, `27.1.12297006`). Always specify the full revision
string in the `android-actions/setup-android` step. The short name `r27` does
not resolve to a specific revision and may silently install a different patch.

---

## 6. Gate 6: HAVE_NDKMEDIA Verification

**Added in**: hud-04b00 (PR #542), discovered follow-up from hud-fts60 (PR #540).

### Why this gate exists

GStreamer 1.24 shipped MR #4115 (NDK MediaCodec path). When `HAVE_NDKMEDIA=1` is
defined at build time, `amcvideodec` uses `AMediaCodec*` C API calls for per-frame
codec operations (dequeue, queue, start, stop) instead of JNI. This reduces per-frame
JNI overhead on high-frame-rate streams.

However, `HAVE_NDKMEDIA` is only defined if the meson build system detects
`<media/NdkMediaCodec.h>` in the NDK headers at SDK build time:

```meson
if cc.check_header('media/NdkMediaCodec.h')
  androidmedia_sources += ndk_sources
  extra_cargs += [ '-DHAVE_NDKMEDIA' ]
endif
```

The prebuilt GStreamer Android SDK tarballs from freedesktop.org should activate
HAVE_NDKMEDIA automatically for NDK API level ≥21 (which exposes NdkMediaCodec.h).
But this needs explicit verification: if it is absent, all MediaCodec operations go
through JNI with no observable error, and phase 3 would miss the per-frame NDK
optimization without knowing it.

### Verification strategy

Since `libmediandk.so` is accessed via `dlopen`/`dlsym` at runtime (not direct
dynamic linkage), a `readelf -d` check for DT_NEEDED entries will not show it.
Instead, the CI gate inspects the compiled symbol content of `libgstandroidmedia.so`:

**Primary check**: `readelf -s --wide libgstandroidmedia.so | grep gst_amc_codec_ndk`

The functions defined in `ndk/gstamc-codec-ndk.c` (e.g., `gst_amc_codec_ndk_new`,
`gst_amc_codec_ndk_start`) are compiled into the `.so` as defined symbols when
`HAVE_NDKMEDIA=1`. They are absent if `HAVE_NDKMEDIA` was not set.

**Secondary check (fallback)**: `strings libgstandroidmedia.so | grep AMediaCodec_createCodecByName`

Because the NDK path uses `dlsym("AMediaCodec_createCodecByName")`, this string
literal is embedded in the binary's `.rodata` section when the NDK path is
compiled in. Its absence means the NDK path is not present.

### What to do if Gate 6 fails

If neither check passes:

1. Verify the GStreamer SDK version is ≥1.24. Older versions predate MR #4115.
2. Verify the SDK tarball layout. The current public `1.24.13` listing publishes
   only the universal archive, so do not assume per-ABI tarballs or a separate
   `libgstandroidmedia.so` are available.
3. Check if the prebuilt was built with an NDK that includes `<media/NdkMediaCodec.h>`.
   NDK r21+ provides this header for API 21+. NDK r16 and earlier do not.
4. As a workaround, rebuild the SDK from source with Cerbero using NDK r27, which
   is guaranteed to expose NdkMediaCodec.h for API 28.

PR #617 is the current observed baseline: `pkg-config` resolved `gstreamer-1.0`
to `1.24.13`, cargo-ndk linked for `aarch64-linux-android` and
`x86_64-linux-android`, and Gate 6 completed with `HAVE_NDKMEDIA_STATUS=not-confirmed`.
That result should not fail the bootstrap link gate. It should remain a visible
warning until phase 3 produces one of:

- Cerbero build logs showing `-DHAVE_NDKMEDIA` while building `androidmedia`.
- Runtime/device evidence that the NDK-backed codec path is active.
- Upstream release notes or package metadata that explicitly identify the public
  prebuilt as built with `HAVE_NDKMEDIA`.

### Important: JNI bootstrap constraint unchanged

`HAVE_NDKMEDIA` being active does **not** eliminate the `gst_android_init` /
`JNI_OnLoad` bootstrap requirement. Codec enumeration and surface-to-ANativeWindow
conversion still use JNI. See Section 5.3 and `docs/reports/gstreamer-1.26-ndk-mediacodec-audit.md`.

---

## 7. Phase-3 Gate Checklist

Pre-Phase-3 open condition: items 1-11 must pass before the phase 3 Android
implementation bead opens. Each item maps to a CI gate in the workflow above
(Section 4) or to an explicit manual verification step.

Phase-3 integration/exit condition: items 12-13 are intentionally blocked until
phase 3 implementation and on-device runtime validation exist.

| # | Gate | Verification | CI? | Status |
|---|---|---|---|---|
| 1 | GStreamer 1.24.x tarball downloaded and SHA-256 verified | CI Gate: download step exits 0 | Yes | Pending |
| 2 | GStreamer arm64 SDK has >50 static archives in `arm64/lib/` | CI Gate 1 | Yes | Pending |
| 3 | `androidmedia` material present (`libgstandroidmedia.so` when separate, otherwise arm64 static archive set) | CI Gate 2 | Yes | Pending |
| 4 | cross-pkg-config resolves `gstreamer-1.0` to version 1.24.x | CI Gate 3 | Yes | Pending |
| 5 | NDK r27 installed; `ANDROID_NDK_HOME` set correctly | CI: NDK install step | Yes | Pending |
| 6 | `cargo-ndk --version` reports 3.5+ | CI: cargo-ndk install step | Yes | Pending |
| 7 | Rust targets `aarch64-linux-android` and `x86_64-linux-android` installed | CI: rust-toolchain step | Yes | Pending |
| 8 | `cargo ndk -t aarch64-linux-android -p 28 build` links gstreamer-full without errors | CI Gate 4 | Yes | Pending |
| 9 | `cargo ndk -t x86_64-linux-android -p 28 build` links gstreamer-full without errors | CI Gate 5 | Yes | Pending |
| 10 | `HAVE_NDKMEDIA` audit executed and status recorded; confirmation requires source-build or runtime evidence | CI Gate 6 warning | Yes | Pending |
| 11 | `build.rs` ABI-to-directory mapping handles both `arm64-v8a` and `x86_64` | Review of build.rs | Manual | Pending |
| 12 | `gst_android_init()` call site exists in Android JNI entry point before `gst_init()` | Code review of JNI_OnLoad | Manual | Blocked (see `docs/reports/android-phase3-gate-validation-2026-04-21.md`) |
| 13 | On-device: `amcvideodec` element registered (`gst_element_factory_find("amcvideodec")` not null) | On-device logcat check | Device-only | Blocked (see `docs/reports/android-phase3-gate-validation-2026-04-21.md`) |

Items 1–9 are validated by the GitHub Actions workflow in Section 4. Item 10 is
observed and reported by CI, but is not a hard pre-phase-3 bootstrap gate while
the public prebuilt remains symbol-inconclusive.
Items 11–13 require phase 3 implementation work or on-device verification.

**Gate is GREEN when**: items 1–9 pass in CI, Gate 6 records either `confirmed` or
`not-confirmed` without hiding the status, item 11 passes manual review, and the
android-bootstrap workflow exits 0 on a PR that adds the actual build.rs and
Android shim crate. Do not make `HAVE_NDKMEDIA` a hard gate without source-built
Cerbero logs or runtime/device evidence.

---

## 7.1 GStreamer 1.26+ Migration Considerations

**Tracked for**: Phase 3 upgrade planning (hud-6pix7).

GStreamer 1.26 introduces a significant build system migration that will require
changes to the CI workflow and bootstrap process if phase 3 upgrades past GStreamer 1.24.x.

### Android.mk Deprecation

GStreamer 1.26 deprecates the Android.mk build system in favor of CMake. The `.mk`
files will be removed in GStreamer 1.28 (scheduled H2 2026). Current phase 3 planning
pins GStreamer 1.24.x, which still ships Android.mk, so **no action is needed until
1.24 reaches end-of-life OR phase 3 explicitly selects 1.26+ for security/performance**.

**Impact**: If/when phase 3 upgrades to 1.26+:
- GStreamer prebuilt tarballs will no longer provide Android.mk stubs
- Cerbero build configurations (if rebuilding from source) will require CMake
- CI workflow validation steps should remain functional (CMake-built artifacts are
  still `.a` and `.so`), but may require NDK CMake toolchain adjustments

### CMake-in-Gradle Replacement

The GStreamer Android documentation will pivot from `.mk` to CMake for integration
into Gradle-based Android Studio projects. For the **CI-side validator** (Section 4),
this change is largely transparent: the gates (tarball verification, pkg-config probes,
cargo-ndk link tests) do not depend on the build system, only on the resulting static
archives and `.so` files.

For **phase 3 runtime integration** (on-device JVM bootstrapping), if moving from
an existing Android project structure that relies on `.mk`, the migration path is:
1. Migrate the `AndroidManifest.xml` and Java shim to a Gradle project
2. Use the Android NDK CMake plugin (via `externalNativeBuild` in `build.gradle`)
3. Replace Android.mk-based build with a `CMakeLists.txt` that links the GStreamer
   prebuilt static archives
4. Verify `gst_android_init()` call site is preserved in the JNI layer (same as 1.24)

### FindGStreamerMobile.cmake Helper

When upgrading to 1.26+, GStreamer provides a CMake helper script for discovery
in the prebuilt tarball: `FindGStreamerMobile.cmake`. This script locates the
GStreamer `.pc` files and static archives in the extracted SDK layout.

If phase 3 uses CMake for JVM bootstrapping (instead of direct cargo-ndk), the
integration will require:
1. Copy or link `FindGStreamerMobile.cmake` into the Android project
2. Invoke `find_package(GStreamerMobile)` in the JNI CMakeLists.txt
3. Link against `GSTREAMER_MOBILE_LIBRARIES` (replaces manual `-L` and `-l` flags)

The **CI validator** (Sections 3 and 4) does not require this: cargo-ndk and pkg-config
handle discovery directly without CMake. This is a phase 3 runtime concern only.

### Timeline and Re-evaluation

- **No action needed until**: GStreamer 1.24 is deprecated OR phase 3 explicitly
  chooses 1.26+ for a significant feature/fix
- **Expected decision point**: Phase 3 kickoff (likely early Q3 2026). At that time,
  evaluate GStreamer 1.26 availability, NDK integration patterns, and Android Studio
  tooling maturity
- **Expected effort**: Medium (mostly documentation + Android project structure migration);
  CI gates remain the same

**Related beads**:
- hud-685ha — Android GStreamer SDK CI bootstrap (this bead's parent)
- hud-fts60 — GStreamer 1.26 NDK MediaCodec audit (evaluation of per-frame optimization)
- hud-dr5yf — Bootstrap CI implementation (CI workflow integration)

---

## 8. Related Documents

- `docs/audits/android-gstreamer-sdk-build-spike.md` — Full Android spike audit
  (hud-4znng, PR #536). Primary reference for architectural decisions, toolchain
  matrix, plugin coverage, and the HYBRID-NATIVE-MEDIACODEC verdict.
- `docs/reports/gstreamer-1.26-ndk-mediacodec-audit.md` — NDK MediaCodec audit
  (hud-fts60, PR #540). Confirms MR #4115 shipped in GStreamer 1.24; explains
  what HAVE_NDKMEDIA does and does not eliminate; basis for Gate 6 in this CI workflow.
- `docs/reports/android-gstreamer-ndkmedia-public-sdk-audit-20260429.md` —
  hud-5dlpd verification against the current public SDK listing and PR #617 CI
  behavior; records why Gate 6 remains an advisory assertion.
- `docs/audits/gstreamer-media-pipeline-audit.md` — GStreamer desktop pipeline
  audit. Desktop pattern reference for GStreamer pipeline model, AppSink bridge,
  GLib main loop threading.
- `docs/audits/ios-videotoolbox-alternative-audit.md` — iOS VideoToolbox audit.
  Parallel reference for the iOS lane; comparison table in the Android spike audit
  (§11) covers the key differences.
- `.github/workflows/ci.yml` — Existing CI workflow. The Android bootstrap workflow
  should follow the same structural conventions (job naming, cache usage, toolchain
  pinning).
- `openspec/changes/v2-embodied-media-presence/` — Phase 3 scope: D19 Pixel target,
  D18 latency budgets, E24/E25 codec requirements.

---

## 9. Sources

- GStreamer Android SDK download: https://gstreamer.freedesktop.org/download/
- GStreamer Android installation guide: https://gstreamer.freedesktop.org/documentation/installing/for-android-development.html
- GStreamer cross-file android_arm64_api28: https://github.com/GStreamer/gst-build/blob/master/data/cross-files/android_arm64_api28.txt
- Android NDK downloads (revision list): https://developer.android.com/ndk/downloads
- android-actions/setup-android GitHub Action: https://github.com/android-actions/setup-android
- cargo-ndk GitHub: https://github.com/bbqsrc/cargo-ndk
- cargo-ndk crates.io: https://lib.rs/crates/cargo-ndk
- `ndk` crate (rust-mobile): https://github.com/rust-mobile/ndk
- GStreamer androidmedia NDK MR #4115: https://gitlab.freedesktop.org/gstreamer/gstreamer/-/merge_requests/4115
- actions/cache v4: https://github.com/actions/cache
- Swatinem/rust-cache: https://github.com/Swatinem/rust-cache
- dtolnay/rust-toolchain: https://github.com/dtolnay/rust-toolchain
