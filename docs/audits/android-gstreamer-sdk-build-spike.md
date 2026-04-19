# Android GStreamer SDK Build-System Spike

**Issued for**: `hud-4znng`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-4znng (Android GStreamer SDK build-system spike, pre-phase-3 gate)
**Discovered from**: hud-ora8.1.18 — GStreamer media pipeline audit (§2.4, §9.5, discovered follow-up 2)

---

## Verdict

**HYBRID-NATIVE-MEDIACODEC**

The GStreamer-via-NDK path is technically viable for the Android device lane (D19 phase 3), but carries meaningful build-system overhead that must be weighed against an alternative. The recommended verdict is a **hybrid approach** rather than committing fully to either pure GStreamer-via-NDK or pure native MediaCodec:

- **Primary decode path: GStreamer Android SDK** — the prebuilt SDK (available at `https://gstreamer.freedesktop.org/data/pkg/android/`) ships with the `androidmedia` plugin (backed by Android MediaCodec) baked in. Activating the `amcvideodec` element inside a GStreamer pipeline gives the same MediaCodec hardware-decode quality as a direct MediaCodec integration, while keeping the pipeline model consistent with the desktop GStreamer stack.
- **Fallback / lighter-weight alternative: `ndk::media` direct bindings** — if the GStreamer Android SDK proves too heavyweight (binary size, plugin initialization overhead, JVM bootstrap dependency), the `ndk` crate provides safe Rust bindings directly to Android's `NdkMediaCodec` C API. This yields a MediaCodec path with no GStreamer runtime.

The spike finds that the GStreamer Android SDK's build chain is substantially more complex than the desktop GStreamer path, and the complexity is front-loaded into the toolchain setup rather than the Rust code. The critical pre-phase-3 work items are:

1. Download and unpack the GStreamer Android SDK prebuilt binaries into CI.
2. Pin NDK version (r25c or later; r27 recommended for GStreamer 1.24) and Android API level (minimum 21).
3. Establish `GSTREAMER_ROOT_ANDROID` environment variable and pkg-config cross-compilation bridge.
4. Write a `build.rs` that links the Rust `.so` against the GStreamer static/gstreamer-full shared library.
5. Validate `androidmedia` plugin availability and `amcvideodec` element discovery at runtime.

If item 5 succeeds end-to-end on a Pixel (D19 procurement target), the GSTREAMER-VIA-NDK path is proven. If the build chain cannot be stabilized within a one-sprint timeframe, fall back to `ndk::media` direct bindings for the Android lane and defer full GStreamer Android SDK to post-phase-3.

---

## Scope

Phase 3 of v2-embodied-media-presence introduces the Android primary device lane (D19: 1× Pixel). The GStreamer audit (hud-ora8.1.18) rated Android support as "Experimental — high risk" and flagged the build chain as requiring a dedicated spike before the Android bead opens.

This spike evaluates:
- GStreamer Android SDK structure, download, and environment setup
- Required Android SDK + NDK + Clang version matrix
- Rust cross-compile target (`aarch64-linux-android` primary; `x86_64-linux-android` for emulator CI)
- Static vs dynamic linking trade-offs on Android
- Plugin coverage: `gst-plugins-good`, `gst-plugins-bad`, `gst-libav` availability for Android
- Hardware decode tier via the `androidmedia` plugin (MediaCodec backend)
- JNI bridge: how GStreamer integrates with Android's Surface/ANativeWindow system
- Comparison with direct Android MediaCodec (`ndk::media` crate)
- `cargo-ndk` / `cargo-mobile2` integration paths for the Rust layer
- Verdict: which approach to carry into phase 3

This spike does **not** cover audio output (cpal audit handles CoreAudio/ALSA/AAudio), WebRTC transport (str0m/webrtc-rs), or the tze_hud compositor wgpu/winit Android integration (separate concern).

---

## 1. GStreamer Android SDK

### 1.1 What It Is

The GStreamer Android SDK is a set of prebuilt static libraries, headers, and build scaffolding maintained by the GStreamer project for cross-compiling GStreamer-based applications for Android. It is built by the Cerbero build system and published alongside each GStreamer release.

**Download location**: `https://gstreamer.freedesktop.org/data/pkg/android/<version>/`

The current stable release (GStreamer 1.24.x) provides per-ABI prebuilt packages:

| File (per-ABI) | Architecture | Target triple |
|---|---|---|
| `gstreamer-1.0-android-universal-<version>.tar.xz` | All ABIs bundled | — |
| `gstreamer-1.0-android-arm64-<version>.tar.xz` | 64-bit ARM | `aarch64-linux-android` |
| `gstreamer-1.0-android-armv7-<version>.tar.xz` | 32-bit ARM | `armv7-linux-androideabi` |
| `gstreamer-1.0-android-x86_64-<version>.tar.xz` | x86-64 | `x86_64-linux-android` |
| `gstreamer-1.0-android-x86-<version>.tar.xz` | x86 | `i686-linux-android` |

For tze_hud's phase 3 scope:
- **Primary CI target**: `aarch64-linux-android` (Pixel physical device, arm64-v8a).
- **Emulator CI target**: `x86_64-linux-android` (Android emulator on x86-64 CI runner).
- The universal tarball (~1.1 GB extracted) includes all four ABIs; prefer per-ABI download for CI speed.

The universal tarball was introduced for convenience alongside the `ndk-build` integration. When extracted, it places each ABI's libraries under `<GSTREAMER_ROOT_ANDROID>/<ABI>/`.

### 1.2 SDK Directory Structure

After extraction, `GSTREAMER_ROOT_ANDROID` points to the top-level directory:

```
$GSTREAMER_ROOT_ANDROID/
  arm64/                 ← aarch64-linux-android
    lib/
      pkgconfig/         ← .pc files for pkg-config cross-compilation
      libgstreamer-1.0.a ← static core (or .so for gstreamer-full)
      libgst*.a          ← per-plugin static archives
      gstreamer-1.0/
        libgstandroidmedia.so  ← androidmedia plugin (.so; JVM-dependent)
    include/
      gstreamer-1.0/
    share/gst-android/
      ndk-build/         ← legacy ndk-build Makefile fragments
      cmake/             ← CMake integration modules
```

**Critical detail**: Most GStreamer plugins are statically linked into the application library in the Android SDK distribution (via `gstreamer-full`). The `androidmedia` plugin (`libgstandroidmedia.so`) is an **exception** — it is shipped as a separate `.so` because it depends on JVM classes (`android.media.MediaCodec`) that require runtime JVM availability. This `.so` must be included in the APK's `jniLibs/` and loaded explicitly before the GStreamer pipeline is constructed.

---

## 2. Required Toolchain Versions

### 2.1 Android SDK

| Component | Required version | Notes |
|---|---|---|
| Android SDK Platform | API level 21+ (Android 5.0) | Minimum for 64-bit ABI support. GStreamer 1.24 targets API 21+. |
| Build Tools | 34.x or later | For Gradle-based build integration. |
| Android SDK Platform Tools | Latest | `adb` for device deployment. |
| CMake (SDK-bundled) | 3.22.x or later | For CMake-based GStreamer integration (alternative to ndk-build). |

**Recommended API target for phase 3**: API 28 (Android 9, Pie). This is the level used in GStreamer's own cross-file example (`android_arm64_api28.txt`); it covers the Pixel hardware target and allows the `androidmedia` MediaCodec NDK API (fully available from API 21, full feature coverage from API 28+).

### 2.2 Android NDK

| NDK version | Status | Notes |
|---|---|---|
| r18b | Legacy (GStreamer docs example for 1.16.x) | Deprecated; do not use. |
| r21 | Supported | LTS release; last NDK with full GCC remnants. |
| r25c | **Recommended minimum for GStreamer 1.24** | Stable LLVM/Clang toolchain; libc++ fully default; ships with `llvm-ar`, `llvm-strip`. |
| r27 | **Preferred** | Current stable (2024); improved LLD linker for reduced `.so` size; fixes several static init ordering issues in complex C++ code (relevant for GLib/GObject). |

**Key change: GCC removed in NDK r18+.** All Android NDK builds now use Clang (LLVM). GStreamer's build system handles this correctly for NDK r18+ via `libc++_shared` or `libc++_static`.

**Clang version with NDK r27**: Clang 18 (LLVM 18). This is the compiler that will build the JNI bridge and any C code in the Rust `build.rs`.

### 2.3 Rust Toolchain

| Component | Required | Notes |
|---|---|---|
| Rust toolchain | 1.82+ | Fixes backtrace/panic handling on `aarch64-linux-android`. Minimum for reliable cross-compilation. |
| Target: aarch64-linux-android | Yes | `rustup target add aarch64-linux-android` |
| Target: x86_64-linux-android | Yes (emulator CI) | `rustup target add x86_64-linux-android` |
| cargo-ndk | ≥3.5 | Auto-detects NDK from `ANDROID_NDK_HOME`; wraps `cargo build` with correct linker and sysroot flags. |

### 2.4 Environment Variables

A complete build environment requires:

```bash
export ANDROID_HOME=/path/to/android-sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.x.yyyyyyy
export GSTREAMER_ROOT_ANDROID=/path/to/gstreamer-1.0-android-universal-1.24.x
# pkg-config cross-compilation bridge (for cargo build.rs)
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_PATH=$GSTREAMER_ROOT_ANDROID/arm64/lib/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot
```

---

## 3. Rust Cross-Compilation Target Matrix

The primary cross-compile triple for production Android (ARM64 devices, Pixel included) is:

```
aarch64-linux-android
```

Additional triples required for comprehensive coverage:

| Triple | Use | Android ABI name |
|---|---|---|
| `aarch64-linux-android` | All modern 64-bit ARM Android (Pixel, Samsung Galaxy, etc.) | arm64-v8a |
| `armv7-linux-androideabi` | 32-bit ARM Android (legacy; SDK minimum) | armeabi-v7a |
| `x86_64-linux-android` | Android emulator (x86-64 CI runner) | x86_64 |
| `i686-linux-android` | 32-bit x86 emulator (rarely needed) | x86 |

**Phase 3 minimum**: `aarch64-linux-android` (physical device) + `x86_64-linux-android` (CI emulator). Drop `armv7` and `i686` from the initial scope; add later only if 32-bit device support becomes a requirement.

**cargo-ndk invocation**:

```bash
cargo ndk \
  --target aarch64-linux-android \
  --platform 28 \
  -o android-build/app/src/main/jniLibs \
  build --release
```

The `-o` flag places the compiled `.so` into the Android Studio project's `jniLibs/` directory at the correct ABI path (`arm64-v8a/`).

---

## 4. Static vs Dynamic Linking on Android

### 4.1 Default: Statically Linked via gstreamer-full

The GStreamer Android SDK's prebuilt distribution uses `gstreamer-full` — a single combined static library that includes GStreamer core + most plugins statically linked. The application links against `gstreamer-full` (the `.a` or `.so` depending on build mode), which produces one large `.so` for the APK's JNI layer.

**Advantages of static linking via gstreamer-full**:
- Single `.so` file in `jniLibs/` (excluding `androidmedia`), simplifying APK packaging.
- Link-time elimination of unused plugins reduces final binary size.
- No dynamic plugin loading path needed; `gst_init()` registers plugins at compile time via macro-generated registration stubs.
- Avoids Android's historical limit of 64 simultaneously loaded `.so` files (pre-Android 8 devices).

**Disadvantages of static linking**:
- Large `.so` size: a gstreamer-full link with a moderate plugin selection (core + base + good + bad plugins for H.264 + VP9) typically produces a `.so` of 8–20 MB (compressed in APK). For a presence engine used on phones, this is acceptable.
- LGPL-2.1 static linking note: GStreamer core is LGPL; statically linking LGPL code into a closed-source binary is legally contentious. tze_hud is open-source; this is not a restriction. For a closed-source distribution, the standard resolution is to use the gstreamer-full shared library path (`.so`) so users can relink — the Android `.so` model satisfies this even for static plugin bundling.
- Longer link times in CI compared to a pure Rust build.

### 4.2 Dynamic Plugin Loading (.so plugins)

The `androidmedia` plugin (`libgstandroidmedia.so`) is an **explicit exception** to the static-linking model. Because it instantiates JVM objects (`android.media.MediaCodec`) via JNI at runtime, it cannot be a static archive — it must be a `.so` loaded at runtime. The implication:

- `libgstandroidmedia.so` must be included in `jniLibs/arm64-v8a/` alongside the main application `.so`.
- The JVM must be initialized (i.e., the code must be running inside an Android Activity) before `gst_init()` registers this plugin.
- A `GstPluginLoader` or explicit `gst_plugin_load_file()` call is required to register `androidmedia` before the first pipeline uses `amcvideodec`.

**Key note from the OMX removal (GStreamer 1.24)**: Prior to GStreamer 1.24, the `gst-omx` plugin was an alternative Android hardware-decode path via the OpenMAX AL API. GStreamer 1.24 removed `gst-omx` entirely. As of GStreamer 1.24+, the **only** hardware-decode path on Android is `androidmedia` / `amcvideodec` (the MediaCodec JNI backend). There is no pure-NDK hardware decode fallback within GStreamer 1.24.

**Note on NDK-based MediaCodec variant**: There is a GStreamer merge request (MR #4115) adding an NDK-based MediaCodec code path (using `AMediaCodec*` C APIs instead of JNI Java reflection). As of April 2026, the status of this MR in the stable 1.24.x release branch is unclear — it may be present in GStreamer 1.26+. If available, the NDK-based path eliminates the JVM bootstrap requirement and is preferred. This should be explicitly probed during the phase 3 build validation.

### 4.3 Recommended Linking Strategy for Phase 3

1. Use the GStreamer Android SDK's gstreamer-full static archive for all plugins except `androidmedia`.
2. Include `libgstandroidmedia.so` separately in `jniLibs/`.
3. Call `gst_plugin_load_file()` early in the Android `JNI_OnLoad` callback (before `gst_init()`) to register the `androidmedia` plugin from the `.so` path.
4. Probe `GstElementFactory::find("amcvideodec")` at runtime to confirm MediaCodec availability; fall back to software decode (`avdec_h264`, `vp9dec`) if not found.

---

## 5. Plugin Availability for Android

The GStreamer Android SDK prebuilt distribution includes the following plugin sets relevant to tze_hud's v2 codec requirements:

### 5.1 Plugin Coverage Table

| Plugin set | Availability in Android SDK | Status | Notes |
|---|---|---|---|
| `gst-plugins-base` | Yes — statically linked | Full | `appsrc`, `appsink`, `videoconvert`, `audioresample`, `typefind`, etc. |
| `gst-plugins-good` | Yes — statically linked | Partial | `vp9dec` (libvpx), `rtpjitterbuffer`, `rtph264depay`, `rtpvp9depay`, `matroskademux`. Available. |
| `gst-plugins-bad` | Yes — statically linked | Partial | `h264parse`, `opusdec`. `androidmedia` via separate `.so`. |
| `gst-libav` | Yes — statically linked | Partial | `avdec_h264` (H.264 software decode). Included. |
| `gst-plugins-ugly` | Not needed for v2 | — | H.264 decode does not require ugly; `avdec_h264` is in `gst-libav`. |

### 5.2 H.264 Decode on Android

| Decode path | GStreamer element | Available in Android SDK | Notes |
|---|---|---|---|
| Hardware (MediaCodec) | `amcvideodec` | Yes (via `androidmedia` `.so`) | Preferred path; maps to device's `MediaCodec` hardware H.264 decoder. |
| Software (FFmpeg) | `avdec_h264` | Yes (static) | Fallback if `androidmedia` unavailable or device lacks HW decoder. |
| OMX | Removed in 1.24 | **No** | Do not use; plugin removed from GStreamer 1.24. |

### 5.3 VP9 Decode on Android

| Decode path | GStreamer element | Available in Android SDK | Notes |
|---|---|---|---|
| Hardware (MediaCodec VP9) | `amcvideodec` | Yes (via `androidmedia` `.so`) | MediaCodec supports VP9 hardware decode on Android 5+ devices; availability probe required at runtime. |
| Software (libvpx) | `vp9dec` | Yes (static, from `gst-plugins-good`) | Universal fallback. `libvpx` is bundled in the Android SDK distribution. |

VP9 hardware decode via MediaCodec is available from Android 5.0 (API 21) on devices with the VP9 hardware decoder. Availability is device-specific (Samsung, Pixel, etc.) and must be probed via `amcvideodec`'s `android.media.MediaCodec.createDecoderByType("video/x-vnd.on2.vp9")`. Unlike iOS (where VP9 hardware decode is universally unavailable), Android VP9 hardware decode is widely available on modern Pixel devices.

---

## 6. Hardware Decode Performance Tier: MediaCodec via `androidmedia`

### 6.1 MediaCodec as GStreamer's Android Hardware-Decode Backend

The `androidmedia` plugin in `gst-plugins-bad` wraps Android's `android.media.MediaCodec` API via JNI. The element `amcvideodec` dynamically selects the system's hardware codec for the given MIME type (e.g., `video/avc` for H.264, `video/x-vnd.on2.vp9` for VP9).

**Performance characteristics** (Android MediaCodec hardware decode):
- H.264 1080p30: hardware decode latency ~2–5 ms per frame (all modern Pixel/Samsung devices)
- VP9 720p30: hardware decode latency ~3–8 ms per frame (device-dependent; Pixel hardware VP9 decoder)
- TTFF (first frame time): 50–200 ms including `MediaCodec.configure()` + `start()` overhead

**MediaCodec glass-to-glass budget analysis** (D18 budgets: p50 ≤150 ms, p99 ≤400 ms):
With `rtpjitterbuffer latency=50ms` + MediaCodec HW decode ~5 ms + Tokio/wgpu compositor upload ~3 ms + WebRTC transport ~20–80 ms (LAN), total p50 ≈ 80–140 ms — within budget. The jitter buffer latency tuning from the GStreamer audit applies here identically.

### 6.2 JNI Bootstrap Requirement

The `androidmedia` plugin uses JNI to call `android.media.MediaCodec` Java methods. This requires:
1. The JVM to be initialized (Android `Activity` or `Service` context active).
2. A valid `JNIEnv*` available when `gst_init()` runs.

GStreamer provides a helper `gst_android_init(JNIEnv *env, jobject context)` (exposed as `gst_android_jni_utils.h`) that must be called from `JNI_OnLoad` or the Activity's `onCreate`. In the Rust + JNI integration, this translates to calling this C function via `jni-sys` FFI in the `JNI_OnLoad` callback.

**tze_hud-specific concern**: tze_hud uses `winit` + `android-activity` for the main event loop on Android. The `android-activity` crate calls `JNI_OnLoad` through its native glue. tze_hud's Android entry point must call `gst_android_init()` before any GStreamer pipeline construction. The ordering constraint:

```
JNI_OnLoad → gst_android_init() → [Android event loop starts] → gst_init() → pipeline construction
```

If `gst_android_init()` is called after `gst_init()`, the `androidmedia` plugin will fail to register its JNI callbacks and hardware decode will silently fall through to software.

---

## 7. JNI Bridge: GStreamer ↔ Android Surface System

### 7.1 Video Output Surface Integration

For live media rendering, GStreamer needs to output decoded frames to the compositor's wgpu texture (CPU-side buffer path, matching the desktop model per RFC 0002 §2.8). The `appsink` element is the correct integration point — it delivers decoded frames as `gst::Sample` objects containing a `gst::Buffer` with raw pixel data. The `gst_video::VideoFrame::map()` API provides a safe typed view.

On Android, GStreamer can also render directly to an `ANativeWindow` (Android native surface) via the `glimagesink` element with surface input provided via `VideoOverlay::set_window_handle()`. For tze_hud, this is **not** the desired path — the compositor owns the wgpu texture and must perform the upload. Use `appsink` → CPU buffer → `wgpu::Queue::write_texture()`, matching the desktop path.

**Pixel format note**: `amcvideodec` (MediaCodec-backed) typically outputs frames in `NV12` (YUV 4:2:0 semi-planar) or `NV21` format. The compositor must convert or upload with a wgpu format conversion pass. On Android, `CVPixelBuffer`-style zero-copy paths do not exist — a CPU-side copy is required for the v2 phase 3 scope. Post-v2: an EGL `SurfaceTexture` path could enable zero-copy GPU upload, but this requires GStreamer `glimagesink` integration with the wgpu GL/Vulkan context, which is architecturally complex.

### 7.2 ANativeWindow and SurfaceTexture

GStreamer requires an `ANativeWindow` backing the `Surface` passed to a video sink. `SurfaceTexture` objects cannot be used directly (GStreamer's Android sink does not implement `SurfaceTexture` rendering). For tze_hud's `appsink`-based model, this is irrelevant — `appsink` pulls CPU buffers and bypasses the `ANativeWindow` entirely.

### 7.3 GLib Main Loop on Android

GStreamer requires a GLib main loop thread (for the GStreamer bus and pipeline state changes). On Android, this must be a dedicated `std::thread` (same as the desktop pattern per the GStreamer audit §5.2). The Android event loop (managed by `android-activity`) must not be blocked by GLib; the GLib main loop runs concurrently on its own thread. No special Android accommodation is needed beyond ensuring `gst_android_init()` is called before the GLib main loop starts.

---

## 8. Direct Alternative: `ndk::media` (Native MediaCodec Bindings)

### 8.1 Overview

The `ndk` crate (https://github.com/rust-mobile/ndk) provides safe Rust bindings to the Android NDK, including `ndk::media::media_codec::MediaCodec` — direct Rust access to `AMediaCodec*` from `<media/NdkMediaCodec.h>`.

| Field | Value |
|---|---|
| Crate | `ndk` |
| Current version | 0.9.x (2024) |
| Repository | https://github.com/rust-mobile/ndk |
| License | MIT OR Apache-2.0 |
| Feature flag | `media` feature enables `ndk::media` |
| Requires | NDK API 21+; `libmediandk.so` (system library) |

### 8.2 `ndk::media` Advantages Over GStreamer Android SDK

| Factor | GStreamer-via-NDK | `ndk::media` direct |
|---|---|---|
| Binary size overhead | ~8–20 MB `.so` (gstreamer-full) | ~0 MB (links against system `libmediandk.so`) |
| Build complexity | High — GStreamer SDK download + env setup + build.rs | Low — `cargo-ndk` + NDK toolchain only |
| JVM dependency for HW decode | Required (`gst_android_init`) | **None** — NdkMediaCodec is pure C NDK API |
| Plugin ecosystem | Full (RTP, jitter buffer, VP9 software, etc.) | Manual (must implement jitter buffer, RTP depacketization, etc.) |
| Tokio integration | Established pattern (AppSink channel bridge) | Manual (callback-based AMediaCodec async output) |
| GStreamer pipeline model | Consistent with desktop stack | Not applicable — no pipeline model |
| Phase 3 feasibility | Viable but complex build chain | Viable; similar effort to iOS VideoToolbox integration |

### 8.3 When to Choose `ndk::media` Over GStreamer

`ndk::media` is the better choice if:
- Binary size is a hard constraint (e.g., the Android APK must be under 50 MB total)
- The GStreamer build chain cannot be stabilized within the phase 3 spike sprint
- The Android target is a thin client that only needs MediaCodec decode with no jitter buffering, VP9 software fallback, or Opus decode (i.e., H.264 only from a pre-packetized stream)

`ndk::media` is approximately the same scope of implementation work as the iOS VideoToolbox path (300–600 lines of FFI integration code to wrap `AMediaCodec*` lifecycle, output buffer callbacks, and Tokio bridge).

### 8.4 Codec Coverage Comparison

| Codec | GStreamer via androidmedia | `ndk::media` NdkMediaCodec | Notes |
|---|---|---|---|
| H.264 HW decode | `amcvideodec` | `AMediaCodec` (MIME `video/avc`) | Both paths; same hardware backend |
| VP9 HW decode | `amcvideodec` (if device supports) | `AMediaCodec` (MIME `video/x-vnd.on2.vp9`) | Both paths; device-dependent |
| VP9 SW decode | `vp9dec` (libvpx in SDK) | Not available natively — requires bundled libvpx | GStreamer wins here |
| Opus audio | `opusdec` in GStreamer | Manual (no NDK Opus support) | GStreamer wins here |
| RTP jitter buffering | `rtpjitterbuffer` in GStreamer | Manual implementation required | GStreamer wins here |

**Conclusion**: For v2 phase 3 scope (H.264 + VP9 + Opus, RTP ingress from WebRTC), GStreamer provides more complete coverage with `gst-plugins-good` (libvpx VP9 SW fallback, RTP handling) and `opusdec`. The `ndk::media` path requires reimplementing RTP depacketization and VP9 software decode, which erases the build complexity advantage.

---

## 9. cargo-ndk and cargo-mobile2 Integration

### 9.1 cargo-ndk (Preferred for CI and Build)

`cargo-ndk` (https://github.com/bbqsrc/cargo-ndk) wraps `cargo build` with the correct Android NDK linker, sysroot, and target flags. Invocation:

```bash
cargo ndk \
  --target aarch64-linux-android \
  --platform 28 \
  -o ./android-build/app/src/main/jniLibs \
  build --release
```

`cargo-ndk` auto-detects NDK from `ANDROID_NDK_HOME`. It handles:
- Setting `CC`/`CXX`/`AR`/`STRIP` to the NDK Clang toolchain
- Adding `--sysroot` to link paths
- Placing `.so` output in the correct ABI subdirectory (`arm64-v8a/`)

**Integration with GStreamer build.rs**: `cargo-ndk` sets the NDK-specific environment variables before calling `cargo build`. A `build.rs` in the Android media crate uses these + `GSTREAMER_ROOT_ANDROID` to:
1. Run `pkg-config --libs gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0` (cross-pkg-config pointing at `$GSTREAMER_ROOT_ANDROID/arm64/lib/pkgconfig`).
2. Emit `cargo:rustc-link-lib=static=gstreamer-full-1.0` (or the individual GStreamer static libs).
3. Emit `cargo:rustc-link-lib=dylib=android` and `cargo:rustc-link-lib=dylib=log` (system Android libs).

### 9.2 cargo-mobile2 (App Scaffolding)

`cargo-mobile2` (https://github.com/tauri-apps/cargo-mobile2) generates Android Studio project boilerplate for Rust mobile apps. It handles the Gradle build file, `jniLibs/` placement, manifest, and Activity entry point. It is useful for bootstrapping the Android app shell but is not strictly required if tze_hud already has an Android project structure via `android-activity` + winit.

For tze_hud's phase 3, `cargo-ndk` is sufficient for the Rust build step. `cargo-mobile2` is optional scaffolding that can reduce boilerplate for the initial Android project setup.

### 9.3 android-activity + winit on Android

tze_hud uses `winit` for its cross-platform windowing layer. `winit`'s Android backend uses `android-activity` crate (NativeActivity or GameActivity backend). The `android-activity` crate provides the `JNI_OnLoad` entry point and the event loop. GStreamer's `gst_android_init()` must be called within `JNI_OnLoad` or early in the app lifecycle — `android-activity` exposes this hook.

For wgpu surface creation on Android, `winit` provides `Window::raw_window_handle()` returning `AndroidNdkWindowHandle` (wrapping `ANativeWindow*`). `wgpu::Instance::create_surface()` accepts this handle and creates a Vulkan surface. This path is independent of GStreamer, which uses `appsink` (not a window sink) for the tze_hud model.

---

## 10. Phase 3 Build Validation Checklist

Before the Android implementation bead opens, these steps must be verified end-to-end:

| Step | Validation command / check | Pass condition |
|---|---|---|
| 1. GStreamer SDK downloaded | `ls $GSTREAMER_ROOT_ANDROID/arm64/lib/*.a \| wc -l` | > 50 static archives present |
| 2. NDK r27 installed | `$ANDROID_NDK_HOME/ndk-build --version` | Shows NDK revision r27.x |
| 3. Rust targets added | `rustup target list --installed \| grep android` | `aarch64-linux-android` present |
| 4. cargo-ndk installed | `cargo ndk --version` | ≥3.5 |
| 5. Cross pkg-config resolves | `PKG_CONFIG_PATH=$GSTREAMER_ROOT_ANDROID/arm64/lib/pkgconfig pkg-config --modversion gstreamer-1.0` | Prints `1.24.x` |
| 6. Rust `.so` compiles | `cargo ndk -t aarch64-linux-android -p 28 build --release 2>&1 \| tail -5` | `Finished release` with no linker errors |
| 7. `androidmedia` `.so` present | `ls $GSTREAMER_ROOT_ANDROID/arm64/lib/gstreamer-1.0/libgstandroidmedia.so` | File exists |
| 8. APK deploys to device | `adb install -r tze_hud.apk && adb logcat -s GStreamer` | No `gst_init` errors in logcat |
| 9. `amcvideodec` element found | `adb shell am start ... && adb logcat -s tze_hud` | Logs show `amcvideodec: found` |
| 10. H.264 decode latency | Device decode test with reference stream (D18 fixed library) | p50 ≤150 ms glass-to-glass |

Steps 1–6 can be validated in CI without a device. Steps 7–10 require a connected Pixel device (phase 3 procurement item from D19).

---

## 11. Comparison with iOS VideoToolbox Path

The iOS VideoToolbox audit (hud-uzqfv) established `PRIMARY-PATH-VIDEOTOOLBOX` with `objc2-video-toolbox`. The analogous Android comparison:

| Factor | Android (GStreamer-via-NDK / ndk::media) | iOS (VideoToolbox via objc2-video-toolbox) |
|---|---|---|
| H.264 HW decode | MediaCodec via `amcvideodec` (GStreamer) or `AMediaCodec` (ndk) | VTDecompressionSession (always available iOS 8+) |
| VP9 HW decode | Device-dependent via MediaCodec | **Not available** on iOS |
| VP9 SW decode | `vp9dec` (GStreamer) | libvpx (manual build.rs) |
| Rust binding maturity | `ndk` crate (stable, 0.9.x) OR GStreamer + gstreamer-rs | `objc2-video-toolbox` (auto-gen bindings) |
| Build complexity | Higher (GStreamer Android SDK setup) or comparable to iOS (ndk::media) | Medium (objc2 + build.rs for libvpx) |
| Safe wrapper needed | Yes (~300–500 lines for ndk::media path; less for GStreamer appsink) | Yes (~300–500 lines for VTDecompressionSession) |
| JVM dependency | Required for GStreamer `androidmedia`; none for `ndk::media` | Not applicable |
| GStreamer consistency | Full (GStreamer pipeline model) or partial (ndk::media) | None (VideoToolbox is the alternative to GStreamer) |

**Key advantage of Android over iOS**: VP9 hardware decode is available on Android (device-dependent) but unavailable on iOS. The Android lane avoids the libvpx-only VP9 fallback for modern Pixel devices.

---

## 12. Discovered Follow-Ups

1. **GStreamer Android SDK CI bootstrap** (pre-phase-3): A CI job must download the per-ABI GStreamer Android SDK prebuilt tarball, set `GSTREAMER_ROOT_ANDROID`, and verify that `cargo-ndk` can link against `gstreamer-full`. Estimated effort: 1–2 days. Must be done before the phase 3 Android bead opens.

2. **NDK-based MediaCodec path in GStreamer 1.26** (investigate at phase 3 kickoff): GStreamer MR #4115 adds an NDK C-API-based MediaCodec code path (`AMediaCodec*`) that eliminates the JVM bootstrap requirement. If this ships in GStreamer 1.26 (current stable as of early 2025), upgrading gstreamer-rs to 0.24/0.25 (GStreamer 1.26) for the Android target removes the `gst_android_init()` ordering constraint. Check at phase 3 kickoff.

3. **VP9 HW decode device probe matrix** (phase-3 Android bead): The Pixel (phase 3 target device) hardware VP9 decoder availability must be confirmed at first boot. `amcvideodec`'s capability query for `video/x-vnd.on2.vp9` should be logged to the audit trail (C17) and included in the phase 3 real-decode CI matrix (D18).

4. **EGL SurfaceTexture zero-copy path** (post-v2 optimization): On Android with Vulkan, EGL `SurfaceTexture` fed from `amcvideodec`'s output buffer can enable zero-copy GPU upload, eliminating the CPU `wgpu::Queue::write_texture()` round-trip. This is a post-v2 optimization (same priority as DMA-BUF zero-copy on Linux). File as a performance bead at phase 3 closeout.

5. **`cargo-ndk` + GStreamer `build.rs` integration guide** (phase-3 Android bead): A documented `build.rs` recipe that correctly calls cross-pkg-config, links against gstreamer-full, and emits the necessary `cargo:rustc-link-*` directives is required before any Rust code in `tze_hud_media` can `use gstreamer::*` in an Android build. This is implementation pre-work, not a separate bead — capture in the Android bead's description.

---

## 13. Summary

| Criterion | Assessment |
|---|---|
| GStreamer Android SDK availability | Official prebuilt binaries at freedesktop.org for each GStreamer release |
| Primary ABI | `aarch64-linux-android` (arm64-v8a); `x86_64-linux-android` for CI emulator |
| NDK required | r27 recommended; r25c minimum |
| Android API level | 21 minimum; 28 target (API 28 cross-file used in GStreamer's own examples) |
| Clang version | LLVM/Clang 18 (NDK r27); libc++ default since NDK r17 |
| Static linking | gstreamer-full static archive (core + most plugins); `androidmedia` is separate `.so` |
| H.264 HW decode | `amcvideodec` via `androidmedia` plugin; JVM init required |
| H.264 SW fallback | `avdec_h264` (gst-libav) statically bundled |
| VP9 HW decode | `amcvideodec` (device-dependent; widely available on Pixel) |
| VP9 SW fallback | `vp9dec` (libvpx) statically bundled in Android SDK |
| OMX plugin | Removed in GStreamer 1.24; do not reference |
| JNI bootstrap | `gst_android_init()` must be called in `JNI_OnLoad` before `gst_init()` |
| Surface/render model | `appsink` → CPU buffer → `wgpu::Queue::write_texture()` (consistent with desktop) |
| cargo-ndk | v3.5+; wraps `cargo build` with NDK linker; handles ABI placement |
| Alternative: `ndk::media` | Lower build complexity; no VP9 SW fallback or RTP plumbing; suitable if binary size is critical |
| Phase 3 readiness | Build chain setup required (CI + build.rs); feasible within one sprint |

**Verdict: HYBRID-NATIVE-MEDIACODEC** — Use the GStreamer Android SDK with the `androidmedia` plugin (which itself wraps MediaCodec) as the primary decode path for the Android lane. The build chain complexity is real but tractable. If the CI bootstrap cannot be stabilized in one sprint, fall back to `ndk::media` direct bindings for the initial phase 3 Android bead and defer full GStreamer Android SDK integration to a follow-up bead. The two paths are architecturally compatible and can be gated via `#[cfg(feature = "gstreamer-android")]`.

---

## Sources

- GStreamer Android installation guide: https://gstreamer.freedesktop.org/documentation/installing/for-android-development.html
- GStreamer Android tutorials: https://gstreamer.freedesktop.org/documentation/tutorials/android/index.html
- GStreamer Android Tutorial 3 (Video): https://gstreamer.freedesktop.org/documentation/tutorials/android/video.html
- GStreamer download page: https://gstreamer.freedesktop.org/download/
- GStreamer cross-file for Android ARM64 API28: https://github.com/GStreamer/gst-build/blob/master/data/cross-files/android_arm64_api28.txt
- OMX removal from GStreamer 1.24 (Discourse): https://discourse.gstreamer.org/t/omx-removal-from-gstreamer-1-24/1672
- GStreamer androidmedia NDK MR #4115: https://gitlab.freedesktop.org/gstreamer/gstreamer/-/merge_requests/4115
- GStreamer static linking README: https://github.com/GStreamer/gst-plugins-base/blob/master/README.static-linking
- GStreamer dynamic .so library discussion: https://discourse.gstreamer.org/t/has-anyone-made-dynamic-libs-build-of-gstreamer-for-android/606
- GStreamer gstreamer-full (Collabora blog): https://www.collabora.com/news-and-blog/news-and-events/generate-mininal-gstreamer-build-tailored-to-your-needs.html
- servo/libgstreamer_android_gen: https://github.com/servo/libgstreamer_android_gen
- `cargo-ndk` GitHub: https://github.com/bbqsrc/cargo-ndk
- `cargo-ndk` crates.io: https://lib.rs/crates/cargo-ndk
- `cargo-mobile2` GitHub: https://github.com/tauri-apps/cargo-mobile2
- `ndk` crate (rust-mobile): https://github.com/rust-mobile/ndk
- `android-ndk-sys` docs.rs: https://docs.rs/android-ndk-sys
- Rust on Android — cross-compilation guide: https://greptime.com/blogs/2025-04-14-rust-in-android-edge-based-practice
- Android NDK MediaCodec C API: https://developer.android.com/ndk/reference/group/media
- GStreamer media pipeline audit (companion): `docs/audits/gstreamer-media-pipeline-audit.md`
- iOS VideoToolbox alternative audit (companion): `docs/audits/ios-videotoolbox-alternative-audit.md`
- v2 signoff packet (D18, D19, E24, E25): `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 procurement list (Android Pixel target): `openspec/changes/v2-embodied-media-presence/procurement.md`
- RFC 0002 §2.8 Media Worker Boundary: `about/legends-and-lore/rfcs/0002-runtime-kernel.md`
- E24 in-process worker posture verdict: `docs/decisions/e24-in-process-worker-posture.md`
