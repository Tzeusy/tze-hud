# iOS VideoToolbox Alternative Audit

**Issued for**: `hud-uzqfv`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-uzqfv (iOS media decode pre-phase-3 gate)
**Discovered from**: hud-ora8.1.18 — GStreamer media pipeline audit (§2.5, §8, §9.3)

---

## Verdict

**PRIMARY-PATH-VIDEOTOOLBOX**

VideoToolbox via `objc2-video-toolbox` (part of the `objc2` Apple framework bindings ecosystem) is the correct and recommended decode path for the iOS device lane in D19 phase 3. It provides hardware-accelerated H.264 and HEVC decode on all iOS devices, integrates via a callback-style `VTDecompressionSession` API that bridges cleanly to Tokio via channels, and is already the technique used by every production iOS media application. The `objc2-video-toolbox` crate (v0.3.2) provides auto-generated, complete, idiomatic Rust bindings generated directly from Xcode SDK headers and explicitly targets `aarch64-apple-ios`.

Two constraints must be accepted before phase 3 iOS work begins:

1. **VP9 hardware decode is unavailable on iOS via VideoToolbox.** `VTIsHardwareDecodeSupported(kCMVideoCodecType_VP9)` returns false on iOS devices; `VTDecompressionSessionCreate` returns error -12906. The VP9 path on iOS requires a software fallback — `libvpx` via the `vpx` crate or `dav1d-rs` (AV1 is better-covered on-device for newer hardware). Given that D18 targets H.264 + VP9 for v2, the iOS lane will require explicit VP9 software decode planning.
2. **The `videotoolbox-rs` crate (shiguredo) and `video-toolbox-sys` (rust-av) do not support iOS.** Both are macOS-only. The correct iOS-capable binding is `objc2-video-toolbox`.

GStreamer via a custom iOS build remains theoretically possible but is practically blocked: as of April 2026, GStreamer iOS + Rust integration is explicitly "not yet possible" per the GStreamer community (GStreamer Discourse #3760), due to unresolved framework/static library compatibility with Xcode/Apple Clang's automatic linking. This path is not recommended for phase 3 without a dedicated build-system spike.

AVFoundation (higher-level Apple AV APIs) is not recommended as the primary decode layer — it adds abstraction overhead and reduces frame-delivery timing control, which conflicts with tze_hud's "arrival time ≠ presentation time" doctrine.

---

## Scope

Phase 3 of v2-embodied-media-presence introduces real primary device coverage including one iPhone (D19). This audit evaluates the media decode alternatives for iOS, where GStreamer (the primary decode stack for desktop platforms) has no Rust-binding support.

The signoff packet D18 specifies: **H.264 + VP9 for v2, AV1 deferred.** The iOS lane must cover both codecs. This audit does not cover:
- Audio decode (covered by `cpal` audit for iOS; CoreAudio backend is stable)
- WebRTC transport (transport is handled by `webrtc-rs` / `str0m` upstream of the decode layer)
- macOS — VideoToolbox on macOS is already wrapped by GStreamer's `vtdec` element; no separate integration is needed

---

## 1. VideoToolbox Framework Overview

### 1.1 What It Is

VideoToolbox is Apple's low-level hardware video codec framework, available on iOS, macOS, tvOS, and visionOS. It exposes direct access to hardware encode/decode engines without going through higher-level AV abstraction layers. Both AVFoundation and AVKit are backed by VideoToolbox internally.

**Key API primitives:**

| API | Description |
|---|---|
| `VTDecompressionSession` | Session object for video decode. Create once, feed frames, receive via callback. |
| `VTDecompressionSessionDecodeFrame` | Submit a `CMSampleBuffer` for decode. Asynchronous by default. |
| `VTDecompressionOutputCallback` | C-style callback invoked per decoded frame with a `CVImageBuffer`. |
| `VTIsHardwareDecodeSupported` | Query whether hardware decode is available for a given codec type. |
| `CMVideoFormatDescriptionCreate` | Construct format description from SPS/PPS NAL units (H.264) or VPS/SPS/PPS (HEVC). |
| `CMSampleBufferCreateReady` | Wrap an RTP-reassembled NAL unit block as a sample buffer for submission. |
| `kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder` | Hint to request hardware decode path. |

### 1.2 Codec Support on iOS

| Codec | iOS availability | Hardware decode | Notes |
|---|---|---|---|
| H.264 (AVC) | iOS 8+ | Yes — all devices | Baseline/Main/High profiles. VT-native format description creation via `CMVideoFormatDescriptionCreateFromH264ParameterSets`. |
| HEVC (H.265) | iOS 11+ | Yes — A9 chip and later | Most relevant for high-quality streams post-v2. |
| VP9 | iOS (all) | **Not supported** | `VTIsHardwareDecodeSupported(kCMVideoCodecType_VP9)` → false; `VTDecompressionSessionCreate` → error -12906. YouTube uses a private entitlement (`com.apple.developer.coremedia.allow-alternate-video-decoder-selection`) not available to third parties. VP9 requires software decode on iOS. |
| AV1 | iOS 16+ (A17/M2+) | Partial — newer devices only | Hardware AV1 decode exists on recent Apple Silicon (iPhone 15 Pro, M2 iPad). API coverage incomplete in VideoToolbox: format description creation is not as ergonomic as for H.264/HEVC. Not a v2 requirement (deferred per D18). |
| ProRes | iOS 15.1+ (M1 devices) | Yes | Post-v2; not relevant for streaming decode. |

**Summary for v2 iOS lane**: H.264 is fully hardware-accelerated on all iOS devices. VP9 requires software fallback. HEVC and AV1 are available on newer devices but are out of v2 scope.

### 1.3 Async vs Sync API

`VTDecompressionSessionDecodeFrame` is **asynchronous by default**: frames are submitted with a callback and decoded out-of-order at the hardware's pace. A `kVTDecodeFrame_EnableAsynchronousDecompression` flag controls this; setting `kVTDecodeFrame_1xRealTimePlayback` hints the session to pace decode.

The callback (`VTDecompressionOutputCallback`) fires on a VideoToolbox-managed thread, not on any Tokio thread — exactly the same integration pattern as GStreamer's AppSink callback firing on GStreamer's internal thread. The bridge pattern from the GStreamer audit (§6.4) applies directly: use `try_send` on a `tokio::sync::mpsc` channel from the callback, consume in a Tokio task.

### 1.4 RTP → VideoToolbox Integration Pattern

WebRTC RTP depacketization produces NALU-reassembled H.264 Annex B or AVCC format byte streams. Feeding these to VideoToolbox requires:

1. Parse SPS/PPS from the RTP stream (from the codec parameters in the SDP offer, or from inline NAL units).
2. Call `CMVideoFormatDescriptionCreateFromH264ParameterSets` to build a `CMVideoFormatDescription`.
3. For each frame: wrap the AVCC byte stream in a `CMBlockBuffer`, then in a `CMSampleBuffer`.
4. Call `VTDecompressionSessionDecodeFrame` with the sample buffer.
5. Receive decoded `CVImageBuffer` (typically as `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange` / NV12) in the callback.
6. Convert or upload the `CVPixelBuffer` to wgpu texture.

This integration has well-established precedent in iOS WebRTC applications. The `str0m` WebRTC library (sans-I/O, the webrtc-rs companion for RTP depacketization) delivers reassembled frames as `Sample` payloads — these map directly to steps 2–4 above.

---

## 2. Rust Binding Landscape

### 2.1 `objc2-video-toolbox` (Recommended)

| Field | Value |
|---|---|
| Crate | `objc2-video-toolbox` |
| Current version | 0.3.2 |
| License | Zlib OR Apache-2.0 OR MIT |
| crates.io | https://crates.io/crates/objc2-video-toolbox |
| docs.rs | https://docs.rs/objc2-video-toolbox/ |
| Repository | https://github.com/madsmtm/objc2 (generated sub-crate) |
| Generated from | Xcode 16.4 SDK headers (auto-generated via `bindgen`-style tooling) |
| iOS targets | `aarch64-apple-ios`, `aarch64-apple-ios-macabi` |
| macOS targets | `aarch64-apple-darwin`, `x86_64-apple-darwin` |

**Coverage:** Full VideoToolbox API surface including `VTDecompressionSession`, `VTCompressionSession`, `VTPixelTransferSession`, `VTPixelRotationSession`, frame processing components, and all profile/level constants for H.264 and HEVC. VP9 codec type constant (`kCMVideoCodecType_VP9`) is present through the CoreMedia types available in companion `objc2-core-media` and `objc2-core-video` crates.

**Maintenance:** Part of the `objc2` project — actively maintained by madsmtm, one of the most complete and up-to-date Apple framework binding efforts in the Rust ecosystem. 4,487+ commits on main, 12 open PRs, framework bindings regenerated against each Xcode release. 100% of the public API is documented per docs.rs.

**Integration model with `objc2`:** The `objc2` project separates Objective-C runtime machinery (`objc2` crate) from framework bindings (per-framework crates like `objc2-video-toolbox`, `objc2-core-media`, `objc2-core-video`, `objc2-foundation`). A VideoToolbox decode integration requires all of these companion crates.

**Caveats:**
- Auto-generated bindings expose a C-style unsafe API at the bottom layer. Safe idiomatic wrappers for the `VTDecompressionSession` lifecycle must be written by the tze_hud implementation (not provided by the crate itself).
- No examples are provided in the crate documentation. Implementation patterns must be derived from Apple's documentation and existing iOS VideoToolbox sample code (Swift/ObjC).
- The `VTDecompressionOutputCallback` is a C function pointer, requiring `unsafe` code and careful lifetime management when bridging to a Rust closure.

### 2.2 `videotoolbox-rs` (shiguredo) — macOS Only, Not Suitable

| Field | Value |
|---|---|
| Crate | `videotoolbox-rs` (shiguredo/video-toolbox-rs) |
| Current version | 2026.1.1 |
| License | Apache-2.0 |
| Platform | **macOS ARM64 only** — no iOS |
| Repository | https://github.com/shiguredo/video-toolbox-rs |

**Assessment:** This crate is macOS-only. It provides a higher-level, type-safe API for VideoToolbox with H.264/H.265/VP9/AV1 decode, `supported_codecs()` detection, and dynamic resolution change support. It is well-maintained (35 commits, CI active, April 2026 release) and serves as a good reference implementation for the kind of ergonomic wrapper tze_hud would build over `objc2-video-toolbox`. **Not usable for iOS.**

### 2.3 `videotoolbox-rs` (doom-fish, v0.1.0) — Abandoned

| Field | Value |
|---|---|
| Crate | `videotoolbox-rs` (doom-fish/core-frameworks) |
| Current version | 0.1.0 (December 2024) |
| Notes | Docs.rs build failed; no iOS mention |

**Assessment:** Abandoned and unbuildable. Ignore.

### 2.4 `video-toolbox-sys` (rust-av) — Unmaintained

| Field | Value |
|---|---|
| Crate | `video-toolbox-sys` |
| Repository | https://github.com/rust-av/video-toolbox-sys (fork of AstroHQ/video-toolbox-sys) |
| Stars | 2 |
| Releases | None published |
| License | MIT |

**Assessment:** Explicitly supports iOS + macOS (descriptions mention both), but is unmaintained (2 stars, no releases, minimal commits). Do not use.

### 2.5 Crate Comparison Summary

| Crate | iOS | Maintenance | Coverage | Recommendation |
|---|---|---|---|---|
| `objc2-video-toolbox` 0.3.2 | Yes | Active (madsmtm) | Full VT API (auto-gen) | **Primary path** |
| `videotoolbox-rs` (shiguredo) 2026.1.1 | No (macOS only) | Active | High-level, safe | Reference only |
| `video-toolbox-sys` (rust-av) | Yes (per docs) | Unmaintained | Unknown | Do not use |
| `videotoolbox-rs` (doom-fish) 0.1.0 | Unknown | Abandoned | Incomplete | Do not use |

---

## 3. Comparison: GStreamer iOS Fork

GStreamer has an official iOS SDK built via the Cerbero build system. The `gstreamer-rs` 0.23.x crate has no iOS support, but theoretically one could link against a custom-built GStreamer iOS xcframework.

**Current status (April 2026):** GStreamer iOS + Rust integration is explicitly "not yet possible" per the GStreamer Discourse community thread (December 2024). The root blocker: "framework/static libraries are not usable with Xcode/Apple Clang's automatic linking." A proof-of-concept is in development but has not shipped; the Cerbero build system supports iOS xcframework output (iOS ARM64 + iOS Simulator ARM64 + iOS Simulator x86_64) but the Rust binding layer has not been adapted.

Additionally, Rust support in Cerbero is enabled by default for `native-linux`, `cross-macos-universal`, and `native-win64` but **not** for iOS targets; more targets are listed as "will be enabled in the future."

**Effort estimate:** Bringing GStreamer to iOS with Rust bindings would require:
1. A custom Cerbero build producing an iOS xcframework with all required plugins
2. Manual linking configuration for the Rust build system (no `pkg-config` on iOS)
3. Resolving the Xcode/Apple Clang static linking incompatibility (open issue, no ETA)
4. Maintaining the custom build chain across GStreamer releases

This is a multi-week spike with no guaranteed outcome. It would block phase 3 indefinitely while the GStreamer community resolves the upstream issue. **Not recommended.**

**Verdict: GSTREAMER-IOS-FORK is not viable for phase 3.** The custom build path is blocked by an open upstream issue. Do not pursue this.

---

## 4. Comparison: AVFoundation Higher-Level APIs

Apple's `AVFoundation` (`AVAsset`, `AVPlayerItem`, `AVAssetReader`, `AVVideoComposition`) provides a higher-level video decode and playback layer built on top of VideoToolbox.

**Rust bindings:** Available via `objc2-av-foundation`, part of the same `objc2` project.

**Why not AVFoundation as the primary decode layer:**

1. **Timing control is insufficient.** `AVPlayerItem` drives its own clock; hooking tze_hud's compositor presentation clock into AVFoundation's internal timeline is architecturally invasive. The "arrival time ≠ presentation time" doctrine requires the runtime to own presentation timing — AVFoundation assumes it does.
2. **Frame delivery is playback-oriented, not decode-oriented.** `AVAssetReader` is designed for file-based decode, not for live RTP streams. `AVPlayer` does support HLS/RTSP but not raw RTP payloads from WebRTC.
3. **Jitter buffer and RTP reassembly are not AVFoundation concerns.** The WebRTC transport layer (str0m/webrtc-rs) handles RTP; feeding reassembled NAL frames directly into VideoToolbox (`VTDecompressionSession`) is the correct integration point.
4. **VideoToolbox is already accessible** via `objc2-video-toolbox` at the same abstraction cost as using `AVFoundation`, but with more control.

**When AVFoundation is appropriate:** Loading and decoding local video files for the iOS device's own pre-recorded content. Not appropriate for the live WebRTC media stream decode path in tze_hud v2.

**Verdict:** AVFoundation is not the right decode layer for tze_hud's iOS media plane. Use VideoToolbox directly.

---

## 5. VP9 on iOS: Software Fallback Strategy

Since VideoToolbox provides no VP9 hardware decode on iOS (confirmed via Apple Developer Forums: `VTIsHardwareDecodeSupported(kCMVideoCodecType_VP9)` → false; error -12906 on session create), the v2 iOS lane requires a software decode path for VP9.

### 5.1 Options

| Option | Crate | Status | Notes |
|---|---|---|---|
| libvpx software decode | `vpx` / `vpx-encode` | Active; bindgen-based | libvpx is the reference VP9 implementation. The `vpx-encode` crate provides VP8/VP9 encode; a companion decode path via `vpx_codec_vp9_cx_algo` is available at the C API level. Requires linking `libvpx` as a static lib on iOS (no system `libvpx` on iOS). |
| dav1d (for AV1 post-v2) | `dav1d-rs` | Active | AV1 only; relevant post-v2 when AV1 is admitted. |
| OpenH264 (H.264 fallback) | `openh264-rs` | Active | H.264 only; not needed since VT covers H.264 in hardware. |

### 5.2 Recommended VP9 Strategy for Phase 3

Build `libvpx` as a static library for `aarch64-apple-ios` via a build script in the iOS media crate, using the upstream libvpx source. This is standard practice in the iOS WebRTC ecosystem — Google's own WebRTC iOS SDK bundles libvpx this way.

For the phase 3 iOS device lane scope, VP9 software decode via libvpx is the only viable path. CPU usage will be higher than H.264 VT hardware decode (~3–10% per stream at 720p30 on modern Apple Silicon) but well within budget for one concurrent media stream.

**Phase 3 recommendation**: Support H.264 as the primary codec via VideoToolbox hardware decode. Add VP9 software decode via libvpx as the secondary path, clearly marked as `#[cfg(target_os = "ios")]`-gated in the decode backend selector.

---

## 6. Mac Shared Codebase Potential

VideoToolbox is available on macOS as well as iOS, and GStreamer's `vtdec` element already wraps it. For the phase 3 implementation, the VideoToolbox Rust integration can be written once as a conditionally-compiled decode backend that serves both iOS and macOS.

**Recommended structure:**

```
crates/tze_hud_media/
  src/
    decode/
      mod.rs             -- backend selector (cfg-gated)
      gstreamer.rs       -- Linux/Windows/macOS via GStreamer
      videotoolbox.rs    -- iOS (and macOS fallback/secondary)
      libvpx.rs          -- iOS VP9 software decode
```

On macOS, GStreamer's `vtdec` element is still preferred because it integrates naturally into the GStreamer pipeline model (jitter buffer, RTP depayloading, pipeline state machine) that desktop platforms use. The `videotoolbox.rs` decode backend is the iOS-primary path.

For macOS, the VideoToolbox backend can serve as an emergency fallback if GStreamer is not installed (developer machines without GStreamer). This is optional and should not be the macOS primary path — the GStreamer pipeline model's timing infrastructure is more correct for the full v2 pipeline.

---

## 7. Integration Architecture for Phase 3

This section is non-normative design guidance. The authoritative iOS implementation contract will be authored when the phase 3 iOS bead opens.

### 7.1 Recommended Cargo Dependencies

```toml
# In the iOS media decode crate (cfg-gated to apple targets)
[target.'cfg(target_os = "ios")'.dependencies]
objc2 = "0.5"
objc2-video-toolbox = "0.3"
objc2-core-media = "0.3"
objc2-core-video = "0.3"
objc2-foundation = "0.3"
# VP9 software decode: libvpx static link (build.rs required)
# No crates.io entry for libvpx decode alone; use the C API via bindgen or
# wrap the vpx_codec C API directly in the iOS media crate
```

### 7.2 VTDecompressionSession Lifecycle

```rust
// Non-normative sketch — not final implementation
// Create session
// 1. Parse SPS/PPS from SDP/RTP stream
// 2. CMVideoFormatDescriptionCreateFromH264ParameterSets(sps, pps, ...)
// 3. Create VTDecompressionSession with output callback + pixel format attributes
//    (kCVPixelFormatType_420YpCbCr8BiPlanarFullRange / NV12)
// 4. Set kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder

// Per-frame decode
// 1. Wrap NAL unit bytes in CMBlockBuffer
// 2. CMSampleBufferCreateReady(block_buf, format_desc, timing, ...)
// 3. VTDecompressionSessionDecodeFrame(session, sample_buf, flags, ...)

// Callback (fires on VT thread — NOT Tokio)
// let (frame_tx, frame_rx) = tokio::sync::mpsc::channel::<DecodedFrameReady>(4);
// VTDecompressionOutputCallback:
//   if status == noErr {
//       let _ = frame_tx.try_send(DecodedFrameReady { cv_image_buf, pts });
//   }

// Tokio compositor task drains frame_rx and uploads to wgpu texture
```

### 7.3 Tokio Bridge

The `VTDecompressionOutputCallback` fires on VideoToolbox's internal thread. Use the same bridge pattern documented in the GStreamer audit (§6.4):

- Non-blocking `try_send` in the callback (real-time thread must not block)
- `tokio::sync::mpsc` with capacity ≤ 4 (matches the ring buffer model in RFC 0002 §2.8)
- Compositor Tokio task drains the channel and uploads CVImageBuffer pixels to wgpu via `queue.write_texture()`

For the pixel format upload step: `CVPixelBuffer` in NV12 format can be accessed via `CVPixelBufferLockBaseAddress` / `CVPixelBufferGetBaseAddress`; the resulting pointer is a CPU-side buffer suitable for `wgpu::Queue::write_texture`. DMA-BUF zero-copy is not applicable on iOS (that is a Linux Vulkan path). Metal texture sharing via `CVMetalTextureCache` is a post-v2 optimization that eliminates the CPU round-trip.

---

## 8. Known Caveats

### 8.1 VP9 — No Hardware Decode on iOS

This is the primary functional gap relative to the desktop GStreamer pipeline. The iOS lane will have:
- H.264: hardware decode (VideoToolbox `VTDecompressionSession`) — **full speed**
- VP9: software decode (libvpx static lib) — **higher CPU cost, no HW acceleration**

For the phase 3 real-device target (1× iPhone per D19), this is acceptable. If the primary codec for phase 3 testing is H.264 (the WebRTC mandate per RFC 7742), VP9 is secondary. The iOS libvpx path must be validated for the D18 glass-to-glass latency budget at 720p30.

### 8.2 No Prebuilt `libvpx` for iOS on crates.io

There is no standard crate providing a prebuilt static `libvpx` for iOS. The implementation will need a `build.rs` that either:
- Downloads and compiles libvpx from source (requires host tools: cmake, nasm/yasm in the iOS cross build)
- Vendors a prebuilt xcframework for libvpx (simpler, brittle to upstream libvpx releases)

This is a build-system complexity item — real work is required before phase 3 CI boots.

### 8.3 `objc2-video-toolbox` is Bindings, Not a Safe Wrapper

The `objc2-video-toolbox` crate exposes the raw VideoToolbox API with Rust type mapping but does not provide safe session lifecycle management. tze_hud must implement:
- A safe `VtDecodeSession` wrapper that manages `VTDecompressionSession` create/destroy
- Callback bridging (C function pointer → Tokio channel) with `unsafe` FFI code
- Error mapping from `OSStatus` codes to tze_hud's structured error taxonomy

This is approximately 300–500 lines of focused FFI integration code, comparable in scope to the GStreamer AppSink bridge.

### 8.4 iOS Simulator Target

`aarch64-apple-ios-sim` and `x86_64-apple-ios` (simulator) also support VideoToolbox (software decode path only in simulators, no hardware acceleration). The CI strategy should include simulator-based VT session creation tests even when real hardware decode is unavailable. This matches D20's "simulators supplementary only" policy.

### 8.5 Session State Machine for Pipeline Rebuild

Unlike GStreamer's pipeline state machine (well-documented, library-managed), `VTDecompressionSession` is a more primitive object: create, decode, invalidate. There is no "pause" or "seek" concept. For stream reconnect (the codec switch / reconnect scenario from the GStreamer audit §8.2), the iOS implementation must: invalidate the existing session → parse new SPS/PPS → create a new session. This is simpler than GStreamer's pipeline rebuild but requires the same discipline around draining the Tokio channel before session teardown.

---

## 9. Summary

| Criterion | Assessment |
|---|---|
| Codec — H.264 hardware decode | Full; `VTDecompressionSession`; all iOS devices from iOS 8+ |
| Codec — HEVC hardware decode | Full; iOS 11+, A9+; out of v2 scope |
| Codec — VP9 | **Not available in HW on iOS**; software decode via libvpx required |
| Codec — AV1 | Partial (iOS 16+, newer silicon only); not a v2 requirement |
| Primary Rust binding | `objc2-video-toolbox` 0.3.2 — full API, iOS-capable, actively maintained |
| macOS shared path | VideoToolbox available on macOS; usable as GStreamer fallback/complement |
| GStreamer iOS fork | Blocked by open upstream issue; not viable for phase 3 |
| AVFoundation | Not suitable as primary decode layer; wrong abstraction level |
| Async API | Callback-based; bridges to Tokio via `try_send` on mpsc channel |
| RTP integration | RTP depacketization (str0m) → NALU assembly → CMSampleBuffer → VT session |
| Build complexity | `objc2-video-toolbox` requires safe wrapper ~300–500 lines; libvpx requires build.rs |
| Phase 3 readiness | Not yet started; two pre-work items required before phase 3 opens (§8.2, §8.3) |

**Verdict: PRIMARY-PATH-VIDEOTOOLBOX.** VideoToolbox via `objc2-video-toolbox` is the correct decode path for the iOS device lane. H.264 hardware decode is fully covered. VP9 requires a libvpx software fallback with a custom build.rs. GStreamer-iOS-fork is blocked upstream and should not be pursued. The implementation has two concrete pre-work items before the phase 3 iOS bead opens: (1) write the safe `VtDecodeSession` wrapper over `objc2-video-toolbox`, and (2) build a libvpx static library build.rs for `aarch64-apple-ios`.

---

## Discovered Follow-Ups

1. **libvpx build.rs for iOS cross-compile** (pre-phase-3-iOS): A `build.rs` or xcframework build script is needed to produce a static `libvpx` for `aarch64-apple-ios`. Estimate: 1–2 days. Must be resolved before the phase 3 iOS VP9 test matrix can run.

2. **Safe VtDecodeSession wrapper** (phase-3-iOS bead pre-work): ~300–500 lines of `unsafe` FFI code wrapping `VTDecompressionSession` lifecycle, callback bridging, and `OSStatus` error mapping. This is a standalone implementation bead before the iOS media pipeline wires up to the rest of the stack.

3. **CVMetalTextureCache zero-copy upload** (post-v2 iOS optimization): On iOS with Metal, `CVMetalTextureCache` allows zero-copy GPU upload of `CVPixelBuffer` decoded frames without the CPU-side `wgpu::Queue::write_texture` roundtrip. Defer to post-v2 but file as a performance bead.

4. **GStreamer iOS upstream tracking** (monitor): The upstream GStreamer + Rust + iOS blocker (static library linking incompatibility) is under active development. Track the GitLab issue and re-evaluate after phase 3 ships — if resolved by phase 4, GStreamer iOS could unify the iOS and desktop decode paths.

---

## Sources

- Apple VideoToolbox documentation: https://developer.apple.com/documentation/videotoolbox
- VTDecompressionSession Apple docs: https://developer.apple.com/documentation/VideoToolbox/VTDecompressionSession
- `kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder`: https://developer.apple.com/documentation/videotoolbox/kvtvideodecoderspecification_enablehardwareacceleratedvideodecoder
- iOS/iPadOS VP9 Codec support — Apple Developer Forums: https://developer.apple.com/forums/thread/664770
- VideoToolbox AV1 decoding on Apple — Apple Developer Forums: https://developer.apple.com/forums/thread/722933
- `objc2-video-toolbox` crates.io: https://crates.io/crates/objc2-video-toolbox
- `objc2-video-toolbox` docs.rs: https://docs.rs/objc2-video-toolbox/
- `objc2` GitHub repository: https://github.com/madsmtm/objc2
- `videotoolbox-rs` (shiguredo) GitHub: https://github.com/shiguredo/video-toolbox-rs
- `videotoolbox-rs` crates.io (doom-fish): https://crates.io/crates/videotoolbox-rs
- `video-toolbox-sys` (rust-av) GitHub: https://github.com/rust-av/video-toolbox-sys
- GStreamer iOS + Rust — GStreamer Discourse #3760: https://discourse.gstreamer.org/t/gstreamer-app-development-for-ios-with-rust/3760
- GStreamer iOS SDK installation: https://gstreamer.freedesktop.org/documentation/installing/for-ios-development.html
- GStreamer Cerbero build system: https://github.com/GStreamer/cerbero
- Video Toolbox and Hardware Acceleration (objc.io): https://www.objc.io/issues/23-video/videotoolbox/
- Accelerating H264 decoding on iOS with VideoToolbox (Medium): https://medium.com/liveop-x-team/accelerating-h264-decoding-on-ios-with-ffmpeg-and-videotoolbox-1f000cb6c549
- `vpx-encode` crate — Rust libvpx interface: https://github.com/astraw/vpx-encode
- `str0m` — sans-I/O WebRTC Rust: https://github.com/algesten/str0m
- GStreamer gstreamer-rs GitHub: https://github.com/sdroege/gstreamer-rs
- GStreamer media pipeline audit (companion): `docs/audits/gstreamer-media-pipeline-audit.md`
- v2 signoff packet (D18, D19, B11): `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 media doctrine: `about/heart-and-soul/media-doctrine.md`
- RFC 0002 §2.8 Media Worker Boundary: `about/legends-and-lore/rfcs/0002-runtime-kernel.md`
