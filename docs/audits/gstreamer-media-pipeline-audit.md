# GStreamer + gstreamer-rs Media Pipeline Audit

**Issued for**: `hud-ora8.1.18`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-ora8.1 (v2-embodied-media-presence, phase 1 media pipeline)
**Context**: Procurement mandate — `openspec/changes/v2-embodied-media-presence/procurement.md`

---

## Verdict

**ADOPT-WITH-CAVEATS**

GStreamer + gstreamer-rs is the correct and already-locked-in foundation for tze_hud's media decode, timing, and synchronization (CLAUDE.md, RFC 0002 §2.8, `about/heart-and-soul/media-doctrine.md`). It has no credible Rust-native alternative for the full v2 codec + synchronization requirements, and its in-process worker model is confirmed compatible with tze_hud's security posture (E24 verdict, `docs/decisions/e24-in-process-worker-posture.md`). Five caveats must be resolved in the implementation phase (details in §8 Known Caveats):

1. **GStreamer system package availability is the dominant platform risk** — GStreamer is not bundled with the gstreamer-rs crate; it must be present as a system library. On Linux this is well-supported; on Windows and macOS it requires explicit CI/distribution work. iOS has no supported path at gstreamer-rs v0.23.
2. **Plugin license matrix requires operator acknowledgement** — the LGPL-licensed plugins cover all v2 codec requirements (H.264, VP9), but the plugin-ugly set carries patent-exposure risk in certain jurisdictions and must be explicitly permitted at package time.
3. **Hardware-decode pipeline selection requires per-platform fallback logic** — `va`, `nvcodec`, and `d3d11` hardware-decode elements are available but not guaranteed present on every machine; the pipeline must probe availability and fall back to software decode (`avdec_h264`, `vp9dec`) without surfacing errors to the runtime.
4. **Dynamic pipeline rebuilds are non-trivial** — reconnecting a GStreamer pipeline (e.g., for codec switch or stream reconnect) requires careful state machine management: `PAUSED` state, element swap, `PLAYING` re-entry. This is a known source of hangs; the implementation must test rebuild paths explicitly.
5. **End-of-stream and error propagation from GStreamer to Tokio requires an explicit bridge** — GStreamer bus messages (EOS, ERROR, WARNING) arrive on GStreamer's internal main loop, not on Tokio futures. The implementation must spawn a dedicated bridge task and correctly propagate these to the budget watchdog and degradation ladder (E25).

---

## Scope

Phase 1 of v2-embodied-media-presence introduces bounded media ingress. The signoff packet (D18) specifies:

> **Codecs**: H.264 + VP9 for v2, AV1 deferred. Glass-to-glass budget: p50 ≤150 ms / p99 ≤400 ms, decode drop ≤0.5%, lip-sync drift ≤±40 ms, TTFF ≤500 ms.

This audit evaluates GStreamer 1.x + the `gstreamer-rs` crate family (the `sdroege/gstreamer-rs` Rust bindings) as the media decode/render layer. WebRTC transport (separate from GStreamer) and audio output (separate cpal audit, hud-ora8.1.19) are out of scope here.

---

## 1. Crate Identity

### 1.1 GStreamer C Library

| Field | Value |
|---|---|
| Project | GStreamer |
| Repository | https://gitlab.freedesktop.org/gstreamer/gstreamer |
| Current stable | 1.24.x (1.24.12, 2025-12) |
| Previous stable | 1.22.x (LTS, end-of-life 2024) |
| License (core) | LGPL-2.1+ |
| License (plugins-good) | LGPL-2.0+ |
| License (plugins-bad) | LGPL-2.0+ (element-level variation) |
| License (plugins-ugly) | LGPL-2.0+ (note: codec patent exposure — see §3) |
| Homepage | https://gstreamer.freedesktop.org/ |

### 1.2 gstreamer-rs Crate Family

| Crate | Description | Current version | License |
|---|---|---|---|
| `gstreamer` | Core bindings | 0.23.4 | MIT / Apache-2.0 |
| `gstreamer-app` | AppSrc / AppSink elements | 0.23.x | MIT / Apache-2.0 |
| `gstreamer-video` | Video frame types, colorspace, timing | 0.23.x | MIT / Apache-2.0 |
| `gstreamer-audio` | Audio sink, format conversion | 0.23.x | MIT / Apache-2.0 |
| `gstreamer-pbutils` | Codec discovery, element factory introspection | 0.23.x | MIT / Apache-2.0 |
| `gstreamer-gl` | OpenGL/EGL texture sharing | 0.23.x | MIT / Apache-2.0 |

**Note on versioning**: gstreamer-rs tracks GStreamer major/minor versions (0.23.x ↔ GStreamer 1.24.x). Each minor GStreamer release requires a new major gstreamer-rs release. Pinning gstreamer-rs to `0.23` implicitly pins the GStreamer 1.24 ABI; gstreamer-rs 0.24 and 0.25 have shipped (latest 0.25.1, 2026-02-28) for GStreamer 1.26+. The 0.23 pin remains valid for GStreamer 1.24 stability; a forward-looking migration to 0.24/0.25 should be tracked as a separate maintenance task.

**Repository**: https://github.com/sdroege/gstreamer-rs
**Main maintainer**: Sebastian Dröge (sdroege), Mathieu Duponchelle (tpm2)
**Stars**: ~1,200
**MSRV**: Rust 1.80+

---

## 2. Platform Availability

GStreamer is a system library, not a bundled crate. Availability is the most operationally significant platform consideration.

### 2.1 Linux (Primary development platform)

| Distribution | Package manager | Status |
|---|---|---|
| Ubuntu 22.04 LTS | `apt` | `libgstreamer1.0-dev`, `gstreamer1.0-plugins-*`. GStreamer 1.20 ships; 1.24 available via backports or PPA. |
| Ubuntu 24.04 LTS | `apt` | GStreamer 1.24 ships natively. Preferred. |
| Fedora 40+ | `dnf` | GStreamer 1.24 ships. |
| Arch Linux | `pacman` | Rolling release, always current. |
| NixOS | `nix` | Excellent GStreamer packaging with feature control. |
| Alpine | `apk` | GStreamer 1.22+. |

**Assessment**: Fully supported. CI requires `libgstreamer1.0-dev` + `libgstreamer-plugins-base1.0-dev` + per-plugin-set dev packages. The GPU runner box (D18 procurement) must have these installed.

**Required build-time packages for CI**:
```
libgstreamer1.0-dev
libgstreamer-plugins-base1.0-dev
libgstreamer-plugins-bad1.0-dev
gstreamer1.0-plugins-good
gstreamer1.0-plugins-bad
gstreamer1.0-libav          # H.264 software decode via libav/FFmpeg
gstreamer1.0-vaapi          # Hardware decode (VA-API, Intel/AMD)
```

### 2.2 macOS

| Method | GStreamer version | Notes |
|---|---|---|
| Homebrew | 1.24.x | `brew install gstreamer` installs core + most plugins. Reliable for development. |
| GStreamer.framework | 1.24.x | Official binary SDK from gstreamer.freedesktop.org. Required for app distribution. ~180 MB installed. |

**Assessment**: Supported for development and distribution. The official .framework bundle is the correct choice for any installable binary (phase 3 macOS primary device lane per D19). gstreamer-rs exposes `PKG_CONFIG_PATH` guidance for finding the framework.

**PKG_CONFIG_PATH setup for CI/build**:
```bash
export PKG_CONFIG_PATH=/Library/Frameworks/GStreamer.framework/Versions/1.0/lib/pkgconfig
```

### 2.3 Windows

| Method | GStreamer version | Notes |
|---|---|---|
| MSVC binaries (official) | 1.24.x | Official MSVC runtime and development installers from gstreamer.freedesktop.org. Required for MSVC-based Rust builds. |
| MSYS2 / MinGW-w64 | 1.24.x | Available via `pacman` in MSYS2. Simpler for development, not suitable for distribution. |
| vcpkg | 1.22.x | Lags behind; not recommended. |

**Assessment**: Supported but requires explicit installer management in CI. The official binary SDK must be downloaded and installed as a CI step; it is not available via `winget` or `choco` for the development headers. Windows CI is more complex to bootstrap than Linux.

**Distribution concern**: tze_hud must either (a) bundle the GStreamer runtime DLLs with the installer, or (b) require users to install GStreamer separately. Option (a) is standard but adds ~40–80 MB to the installer depending on the plugin subset selected. This is an explicit gap to plan before the Windows distribution lane (phase 3 per D19).

**Note**: The Windows primary device lane per D19 is "already in use per CLAUDE.md" — this makes GStreamer Windows setup a near-term action item, not a future concern.

### 2.4 Android (Phase 3 mobile)

| Method | Status | Notes |
|---|---|---|
| `gstreamer-rs` Android support | **Experimental** | Android cross-compilation via NDK is possible but requires manual GStreamer Android SDK download + environment setup. No first-class crate support. |
| GStreamer Android SDK | 1.24.x | Official GStreamer prebuilt `.aar` for Android. Requires custom CMake/Gradle integration. |

**Assessment**: Technically possible but non-trivial. MediaCodec (Android's native HW decode) is available as a GStreamer element (`androidmedia`) in the Android SDK, which is the preferred decode path. The Rust + GStreamer + Android NDK build chain requires documented setup steps before the Android device lane can be enabled (phase 3). This is the second most significant platform gap.

### 2.5 iOS (Phase 3 mobile)

| Method | Status | Notes |
|---|---|---|
| `gstreamer-rs` iOS support | **Not supported** | No gstreamer-rs iOS support in the 0.23.x crate. The GStreamer C SDK has an iOS SDK but Rust cross-compilation to iOS with GStreamer linking is unsupported and untested by the gstreamer-rs project. |

**Assessment**: This is a hard gap for v2 phase 3. The iOS primary device lane per D19 cannot use GStreamer for media decode. See §8 (Alternatives) for the iOS path: VideoToolbox via a dedicated Rust crate or FFI.

**Discovered follow-up**: iOS media decode requires an alternative library or direct VideoToolbox FFI. This is outside the scope of this audit but must be tracked before phase 3 iOS CI begins.

### 2.6 Platform Summary

| Platform | GStreamer Availability | Distribution Path | v2 Phase | Risk |
|---|---|---|---|---|
| Linux | Full | System packages | Phase 1 | Low |
| macOS | Full | .framework bundle | Phase 1 / Phase 3 | Low |
| Windows | Full | Official MSVC installer | Phase 1 / Phase 3 | Medium (CI bootstrap complexity) |
| Android | Partial (experimental) | GStreamer Android SDK | Phase 3 | High |
| iOS | Not supported by gstreamer-rs | N/A — needs alternative | Phase 3 | Blocker — needs alternative |

---

## 3. License Posture

### 3.1 GStreamer Core and gstreamer-rs

**GStreamer core** (`libgstreamer`, `gst-plugins-base`) is LGPL-2.1+. The Rust bindings (`gstreamer-rs`) are MIT/Apache-2.0.

LGPL-2.1 embedding implications for a Rust binary:
- Static linking to LGPL libraries in a closed-source application is legally contentious. The LGPL requires that users be able to relink against a modified version of the library.
- The standard resolution: **dynamically link** GStreamer (`.so`/`.dylib`/`.dll`), which is the default behavior when GStreamer is installed as a system package or via the official SDK.
- For tze_hud: since the application is open-source (check repo license), LGPL embedding is not a restriction. Even if closed-source, dynamic linking is the natural deployment model (system package or bundled DLLs), satisfying the LGPL relink requirement.

**Conclusion**: LGPL-2.1 poses no practical restriction for tze_hud's deployment model.

### 3.2 Plugin License Matrix

| Plugin Set | License | Required for v2 | Notes |
|---|---|---|---|
| `gst-plugins-base` | LGPL-2.0+ | Yes | Foundations: `videoscale`, `videoconvert`, `audioresample`, `appsink`/`appsrc` |
| `gst-plugins-good` | LGPL-2.0+ | Yes | `vp9dec` (VP9 software decode), RTP demux, MatroskaDemux |
| `gst-plugins-bad` | LGPL-2.0+ (element-level) | Yes | `h264parse`, `nvcodec` (NVDEC hardware), `d3d11videosink`/`d3d11h264dec` (Windows D3D11), `openh264dec` |
| `gst-plugins-ugly` | LGPL-2.0+ | **No for v2** | `x264enc` (H.264 SW encode) — only needed for phase 4f voice/video emit. H.264 decode (`avdec_h264`) is in `gst-libav`, not ugly. |
| `gst-libav` | LGPL-2.1+ (FFmpeg) | Yes | `avdec_h264` (H.264 software decode via FFmpeg) |
| `gst-vaapi` | LGPL-2.1+ | Optional | VA-API hardware decode on Linux (Intel, AMD) |

**Key point on plugins-ugly**: The "ugly" designation is historically about software patent exposure (MPEG-LA, H.264 in some jurisdictions), not GPL. For H.264 **decode** (not encode), `avdec_h264` (in `gst-libav`) is the correct element and it does not carry the same patent concern as the encoding-focused `x264enc` in plugins-ugly. V2 phase 1 (bounded ingress) is **decode-only** — plugins-ugly is not required.

**H.264 patent note**: H.264 patents remain valid in most jurisdictions through at least 2027. MPEG-LA licenses require royalty payments for H.264 encode/decode in commercial products above certain thresholds. For a local-first presence engine (not a codec vendor or large-scale distributor), this is typically covered by the runtime environment's existing FFmpeg/libav licenses. Track this before any commercial distribution.

**Conclusion**: V2 phase 1 (H.264 decode + VP9 decode) requires only LGPL-licensed plugin sets (`gst-plugins-good`, `gst-libav`). No plugins-ugly required until phase 4 encode. LGPL embedding is compatible with tze_hud's deployment model.

---

## 4. Codec Coverage vs. v2 Requirements

The signoff packet D18 specifies: **H.264 + VP9 for v2, AV1 deferred.**

### 4.1 H.264 (AVC) — Required for v2

| Decode path | GStreamer element | Plugin set | HW acceleration | Notes |
|---|---|---|---|---|
| Software | `avdec_h264` | `gst-libav` | No | Universally available; FFmpeg-backed. Baseline/Main/High profiles. |
| Software (OpenH264) | `openh264dec` | `gst-plugins-bad` | No | Cisco's open-source H.264; patent-safer alternative for encode; decode quality lags FFmpeg. |
| VA-API (Linux HW) | `vah264dec` | `gst-plugins-bad` (vaapi) | Intel/AMD | Requires VA-API support; available on most modern Intel/AMD iGPUs. |
| NVDEC (NVIDIA HW) | `nvh264dec` | `gst-plugins-bad` (nvcodec) | NVIDIA | CUDA-free; uses NVCUVID. Available on NVIDIA GPUs with NVENC/NVDEC. |
| D3D11 (Windows HW) | `d3d11h264dec` | `gst-plugins-bad` (d3d11) | Windows D3D11 | MediaFoundation-backed hardware decode. Available on most Windows devices. |
| VideoToolbox (macOS HW) | `vtdec` | `gst-plugins-bad` (applemedia) | macOS/iOS | Apple VideoToolbox; available on all Apple Silicon + Intel Mac. |
| MediaCodec (Android HW) | `androidmediacodecdec` | `gstreamer-androidmedia` | Android NDK | Available via the Android GStreamer SDK. |

**Assessment**: H.264 decode is fully covered across all desktop platforms. Hardware paths exist for all major GPU vendors (Linux: VA-API + NVDEC, Windows: D3D11, macOS: VideoToolbox). The primary software fallback (`avdec_h264`) is universally available.

### 4.2 VP9 — Required for v2

| Decode path | GStreamer element | Plugin set | HW acceleration | Notes |
|---|---|---|---|---|
| Software | `vp9dec` | `gst-plugins-good` | No | libvpx-backed; fully open, royalty-free. |
| VA-API (Linux HW) | `vavp9dec` | `gst-plugins-bad` (vaapi) | Intel/AMD | Available on Intel Gen9+ and AMD RDNA with VA-API 1.16+. |
| NVDEC (NVIDIA HW) | `nvvp9dec` | `gst-plugins-bad` (nvcodec) | NVIDIA | Available on NVIDIA Pascal+ (GTX 10xx and newer). |
| D3D11 (Windows HW) | `d3d11vp9dec` | `gst-plugins-bad` (d3d11) | Windows D3D11 | Available on Windows 10/11 with compatible GPU driver. |

**Assessment**: VP9 software decode is universally available via `vp9dec` (royalty-free, no patent concerns). Hardware paths exist for Linux + Windows. macOS VideoToolbox does **not** support VP9 hardware decode — software path required on macOS.

### 4.3 AV1 — Deferred per D18

Noting for completeness: GStreamer 1.24 includes `dav1ddec` (dav1d-backed AV1 software decode, in `gst-plugins-bad`), plus hardware paths via `vav1dec` (VA-API Linux), `nvav1dec` (NVDEC), and `d3d11av1dec` (Windows D3D11). When AV1 is eventually admitted (post-v2), the GStreamer plugin support will be ready.

### 4.4 Opus (Audio — E22)

GStreamer's `opusdec` element (in `gst-plugins-bad`) handles Opus decode. The cpal audit (hud-ora8.1.19) recommends using GStreamer for upstream decode (Opus → PCM) and handing off PCM to cpal. The `opusdec` element + an `appsink` capturing PCM buffers is the natural bridge. No additional concern here.

### 4.5 Codec Coverage Summary

| Codec | v2 Required | GStreamer SW decode | GStreamer HW decode (Linux) | macOS HW | Windows HW |
|---|---|---|---|---|---|
| H.264 | Phase 1 | avdec_h264 | vah264dec / nvh264dec | vtdec | d3d11h264dec |
| VP9 | Phase 1 | vp9dec | vavp9dec / nvvp9dec | SW only | d3d11vp9dec |
| AV1 | Deferred | dav1ddec | vav1dec / nvav1dec | SW only | d3d11av1dec |
| Opus (audio) | Phase 1 | opusdec | — | — | — |

---

## 5. Latency Analysis

### 5.1 Pipeline Overhead

A minimal GStreamer decode pipeline for an RTP stream:

```
udpsrc → rtpjitterbuffer → rtpvp9depay → vp9dec → appsink
              or
udpsrc → rtpjitterbuffer → rtph264depay → h264parse → avdec_h264 → appsink
```

GStreamer pipeline components add latency:

| Component | Typical added latency | Notes |
|---|---|---|
| `rtpjitterbuffer` | 200 ms default (configurable) | Jitter buffer is the dominant latency source for RTP. Must be set to `latency=50ms` or lower for glass-to-glass ≤150 ms p50. |
| Element initialization (TTFF) | 50–150 ms | First-frame initialization, pipeline negotiation, decoder warm-up. One-time cost. |
| Decode frame latency (H.264 SW, 1080p) | 5–15 ms | Variable by resolution and CPU. Hardware decode: 2–5 ms. |
| `appsink` polling | < 1 ms | `emit-signals=true` or `try_pull_sample(Duration::ZERO)` pattern adds negligible overhead. |
| `DecodedFrameReady` channel | < 1 ms | Ring buffer write + compositor drain (RFC 0002 §2.8 pattern). |
| Compositor texture upload | 1–3 ms | `queue.write_texture()` on CPU-side RGBA data. DMA-BUF zero-copy path on Linux saves this step. |

**Glass-to-glass budget analysis** (D18: p50 ≤150 ms, p99 ≤400 ms):

With `rtpjitterbuffer latency=50ms`, typical decode ~10 ms (SW), compositor texture upload ~2 ms, and WebRTC transport ~20–80 ms (LAN), total p50 is approximately 80–120 ms — within budget. The jitter buffer must be tuned; the default 200 ms would push p50 over budget. This is a configuration caveat, not a structural limitation.

For the TTFF target (≤500 ms per D18): pipeline construction + first-frame decode is typically 200–400 ms including decoder initialization. This is achievable but must be validated on the GPU runner box.

### 5.2 In-Process Worker Model vs. Tokio (E24 Compatibility)

RFC 0002 §2.8 states explicitly:
> GStreamer has its own internal thread pool (managed by its scheduler and element graph). WebRTC ICE/DTLS threads are managed by the WebRTC library. These cannot be collapsed into the Tokio runtime.

The E24 verdict (`docs/decisions/e24-in-process-worker-posture.md`) confirms:
> The Tokio tasks do not replace or own the GStreamer internal thread pool; they orchestrate pipeline lifecycle and enforce budgets on top of it.

**Integration model**:
- GStreamer's `GMainLoop` or GStreamer's internal bus thread handles pipeline state changes and bus messages (EOS, ERROR). This is a native thread — it MUST NOT be a Tokio task.
- A dedicated `std::thread` (not a Tokio task) runs the GStreamer main loop per pipeline or per session.
- Tokio tasks (`budget watchdog`, `session controller`) interact with GStreamer pipelines via the gstreamer-rs API (state changes: `set_state(State::Playing)`), which is thread-safe.
- `AppSink::set_callbacks()` or polling via `try_pull_sample()` from a Tokio task provides the decoded frame handoff into the `DecodedFrameReady` ring buffer.

**Concern**: mixing `async`-context polling of GStreamer's blocking `pull_sample()` is a footgun. The correct pattern is `try_pull_sample(ClockTime::ZERO)` (non-blocking) inside a Tokio task with a `tokio::time::interval` wake, or using `AppSink`'s emit-signals model with a `crossbeam` channel bridge. The `new-sample` signal fires on GStreamer's internal thread; the Tokio task receives via a channel.

### 5.3 GPU Surface Handoff

RFC 0002 §2.8 establishes that decoded frames arrive as CPU-side buffers (or DMA-BUF on Linux) into the `DecodedFrameReady` ring buffer. The compositor thread uploads to wgpu. GStreamer's `appsink` provides `gst::Sample` containing a `gst::Buffer` with video frame data. The `gst_video::VideoFrame::map()` API provides a safe, typed view of the raw pixel data.

The DMA-BUF zero-copy path (Linux/Vulkan only) is post-v1; the phase 1 implementation uses CPU-side upload. This is correct for v2 phase 1 scope.

---

## 6. In-Process Safety

### 6.1 Thread Model Compatibility with RFC 0002 Amendment 1 §worker-pool

RFC 0002 §2.8 pre-declares the worker pool:

> The media worker pool is managed entirely by GStreamer's internal scheduler. From the compositor's perspective, the media pool is a black box that delivers decoded frames.

This is fully compatible with GStreamer's threading model. The implementation does NOT need to — and should NOT try to — manage GStreamer's internal threads. The compositor creates a pipeline, calls `set_state(Playing)`, and reads from `AppSink`. GStreamer manages its own thread pool for the decode graph.

The E24 "Shared worker pool (N=2–4) with priority-based preemption" applies to the Tokio-layer budget watchdog and session controller, not to the GStreamer decode threads. The watchdog monitors pipeline health and enforces budget limits by calling `set_state(Paused)` or `pipeline.send_event(Event::Eos)` to shed load — it does not preempt GStreamer's internal threads directly.

### 6.2 GLib Initialization

GStreamer requires GLib's type system to be initialized. In a multi-threaded Rust binary:
- `gst::init()` must be called exactly once before any other GStreamer API.
- `gst::init()` is safe to call from any thread but must be called before spawning threads that use GStreamer.
- The recommended pattern for tze_hud: call `gst::init()?` during compositor startup, before the media worker pool is created. gstreamer-rs makes this idiomatic.

### 6.3 Memory Safety

gstreamer-rs provides safe Rust wrappers around the GStreamer C API. The wrappers use `Arc<>`-based reference counting mirroring GStreamer's GLib refcount model. Key safety properties:
- `gst::Pipeline`, `gst::Element`, `gst::Pad` are reference-counted and `Send + Sync`.
- `gst::Buffer` provides safe memory-mapped access via `gst::Buffer::map()` — the map guard enforces borrow rules.
- `gst_video::VideoFrame` maps the buffer with format-aware typing.

**Known CVE surface**: GStreamer's C plugins (especially `avdec_*` via FFmpeg) have historically had CVEs (buffer overflows in codec parsers). The E24 verdict acknowledges this explicitly: "codec-level memory-safety risk is zero. It is not." The mitigation is defense-in-depth: bounded ring buffers, budget watchdog with decoder restart, E25 degradation ladder. A future subprocess-sandbox for codec isolation is a post-v2 hardening item.

### 6.4 GStreamer Bus and Tokio

GStreamer's message bus (errors, EOS, state changes) fires callbacks on GStreamer's internal bus thread, not on Tokio. Correct bridging pattern:

```rust
// Non-normative sketch
let (bus_tx, bus_rx) = tokio::sync::mpsc::channel(32);
let bus = pipeline.bus().unwrap();
bus.add_watch(move |_, msg| {
    let _ = bus_tx.try_send(msg.to_owned());
    Continue(true)
});
// Tokio task:
while let Some(msg) = bus_rx.recv().await {
    match msg.view() {
        MessageView::Eos(..) => { /* trigger EOS handling in session controller */ }
        MessageView::Error(e) => { /* escalate to budget watchdog */ }
        _ => {}
    }
}
```

The `add_watch` callback must be `'static + Send` — this is enforced by gstreamer-rs. The `try_send` (non-blocking) pattern prevents the GStreamer bus thread from blocking if the Tokio consumer is behind.

---

## 7. Maintenance Health

| Metric | Observation |
|---|---|
| GStreamer last release | 1.24.12 — December 2024 |
| GStreamer release cadence | Quarterly minor releases; long-term-supported 1.22 LTS cycle until 2024 |
| gstreamer-rs last release | 0.23.4 — 2025 (tracks GStreamer 1.24) |
| gstreamer-rs maintainer | Sebastian Dröge (sdroege); active daily committer since 2017 |
| gstreamer-rs organization | Maintained under the `gstreamer-rs` GitHub org |
| Stars (gstreamer-rs) | ~1,200 |
| Upstream GStreamer | Freedesktop.org GitLab; multiple full-time maintainers at Collabora, Igalia, Centricular |
| Dependents (gstreamer-rs) | Used in production by commercial multimedia projects (e.g., Servo, multiple embedded media systems) |
| CVE history | GStreamer has a regular CVE cadence in codec plugins; the project responds quickly with patch releases. Subscribe to gstreamer-security@lists.freedesktop.org for notifications. |

**Assessment**: Very healthy. GStreamer upstream is a mature, corporate-backed open-source project. gstreamer-rs has a single highly active lead maintainer with Freedesktop backing. The crate tracks GStreamer releases closely and is the only credible production-quality Rust binding for GStreamer.

**Version pinning recommendation**: Pin to `gstreamer = "0.23"` in `Cargo.toml`. When GStreamer 1.26 ships (anticipated mid-2025), migrate to gstreamer-rs 0.24 as a separate tracked bead. The API changes between gstreamer-rs minor versions are breaking in the Rust sense (new major version) but typically mechanical.

---

## 8. Known Caveats

**Note**: The Verdict's top-5 caveats (§ Verdict above) emphasize platform availability and licensing (covered in §2 and §3). This section details the five implementation-phase caveats that demand active discipline in the phase 1 work. Together they form the complete caveat inventory.

### 8.1 Hardware-Decode Pipeline Selection

Hardware-decode elements (`vah264dec`, `nvh264dec`, `d3d11h264dec`, `vtdec`) are not guaranteed present on every machine. Pipeline construction will fail with `MISSING_PLUGIN` if the element is not available.

**Mitigation**: Use a probe-and-fallback pattern:

```rust
// Non-normative sketch
fn build_h264_decode_bin() -> gst::Element {
    // Try hardware decode first
    for hw_element in &["vah264dec", "nvh264dec", "d3d11h264dec", "vtdec"] {
        if gst::ElementFactory::find(hw_element).is_some() {
            if let Ok(elem) = gst::ElementFactory::make(hw_element).build() {
                return elem;
            }
        }
    }
    // Fallback to software decode (universally available via gst-libav)
    gst::ElementFactory::make("avdec_h264").build().expect("avdec_h264 must be installed")
}
```

`gst_pbutils::Discoverer` and `gst_pbutils::ElementFactory::find()` provide safe plugin presence checks.

### 8.2 Dynamic Pipeline Rebuilds

For stream reconnect or codec switch (e.g., publisher changes resolution), the pipeline must be rebuilt. GStreamer's `Pipeline::set_state(State::Null)` → element reconfiguration → `set_state(State::Playing)` is the correct pattern, but:

- Calling `set_state(State::Null)` while the pipeline is `Playing` is safe but blocks until all elements complete their `Null` transition.
- Element state changes are asynchronous; `get_state()` with a timeout must be polled to confirm completion.
- Failing to drain `AppSink` before pipeline state change can cause a deadlock if AppSink's buffer queue is full.

**Mitigation**: The implementation must use `set_state()` + `state()` polling with explicit timeouts (500 ms max per E24 budget), and drain `AppSink` before pipeline teardown. The E25 degradation ladder's "freeze+no-input → tear down media" step must exercise this path in validation.

### 8.3 End-of-Stream Handling

GStreamer sends an EOS bus message when a source reaches end of stream. For live RTP streams (v2 phase 1), EOS typically indicates stream termination or server disconnect, not natural end-of-file. The implementation must:
- Treat live-stream EOS as a session degradation event (not a normal termination).
- Route the EOS bus message to the session controller via the Tokio bridge (§6.4).
- Apply the E25 degradation ladder: media surface shows last frame with disconnection badge (per B11 in the signoff packet).

### 8.4 Error Propagation

GStreamer ERROR bus messages contain `glib::Error` with domain and code. Not all errors map cleanly to tze_hud's structured error taxonomy. The implementation must:
- Map at minimum: `GST_STREAM_ERROR_DECODE` → decode failure, `GST_RESOURCE_ERROR_READ` → network/source failure.
- Log the full `glib::Error` detail string to the audit log (C17).
- Any unmapped GStreamer error escalates to the budget watchdog as a decode failure event.

### 8.5 GStreamer Version Compatibility on CI and Platform Availability

The GPU runner box (D18, phase 1 gate) must run Ubuntu 24.04 or later with GStreamer 1.24 to access all required elements. A CI job running Ubuntu 22.04 with GStreamer 1.20 will fail to find `vah264dec` and `vavp9dec` (added in 1.22), and some `d3d11` elements are 1.24-only. The CI image version must be pinned and documented.

**Platform availability integration note**: While platform availability (§2) and licensing (§3) are resolved before implementation begins, the GStreamer installation and version on each CI target remains a runtime dependency. The implementation must verify at initialization (§10.2 `init_media_subsystem()`) that required elements are present; failure cascades to the E25 degradation ladder.

---

## 9. Alternative Paths

The following alternatives were evaluated. GStreamer remains the correct choice for v2; these notes are for phase-3 exceptions and future reference.

### 9.1 Pure Rust: `rav1e` (AV1 encode) / `dav1d` (AV1 decode)

**rav1e** (Rust AV1 encoder) and **dav1d** bindings (`dav1d-rs`) are pure-Rust-friendly alternatives for AV1, but:
- AV1 is deferred to post-v2 (D18).
- Even when AV1 is admitted, the v2 architecture uses GStreamer's `dav1ddec` element (which wraps dav1d in C) rather than the Rust `dav1d-rs` crate, to maintain the consistent pipeline model.

**Verdict**: Not applicable for v2.

### 9.2 Pure Rust: `openh264-rs` / H.264

The `openh264-rs` crate provides Rust bindings to Cisco's OpenH264. It covers Baseline profile H.264 only, with lower decode quality than FFmpeg's `avdec_h264`. OpenH264 is Cisco-provided with a royalty-free patent license for decode (but not encode).

**Verdict**: Inferior to `avdec_h264` for production decode. Useful as an emergency fallback if GStreamer is unavailable on an unexpected platform, but not the primary path. For v2, use GStreamer.

### 9.3 Platform-Native: Apple VideoToolbox

VideoToolbox (macOS/iOS) provides hardware-accelerated H.264/HEVC/VP9 decode via `VTDecompressionSession`. On macOS, GStreamer's `vtdec` element already wraps VideoToolbox — no separate integration needed. On iOS, where GStreamer has no gstreamer-rs support, VideoToolbox FFI is the recommended media decode path for phase 3.

**Verdict**: Already covered by GStreamer on macOS. On iOS, a lightweight `VideoToolbox` FFI wrapper or the `video-toolbox` crate is the correct alternative for the iOS device lane (phase 3). File as a discovered follow-up.

### 9.4 Platform-Native: Windows Media Foundation

Windows Media Foundation (MF) provides H.264/VP9/HEVC hardware decode on Windows. GStreamer's `d3d11h264dec` already wraps MF for hardware paths — no separate integration needed.

**Verdict**: Covered by GStreamer on Windows.

### 9.5 Platform-Native: Android MediaCodec

Android MediaCodec provides hardware H.264/VP9/AV1 decode on Android NDK. GStreamer's `androidmediacodecdec` wraps MediaCodec in the Android GStreamer SDK. Direct `ndk::media` crate bindings are an alternative for lighter-weight integration without the full GStreamer SDK.

**Verdict**: GStreamer Android SDK is the correct v2 path if Android decode quality is needed. If the Android GStreamer SDK proves too heavyweight for phase 3, evaluate `ndk::media` direct bindings as a lightweight alternative. File as a discovered follow-up.

### 9.6 FFmpeg via `ffmpeg-next` crate

The `ffmpeg-next` crate provides Rust bindings to FFmpeg's `libavcodec`/`libavformat`/`libavutil`. It would give direct codec access without GStreamer's pipeline overhead. However:
- GStreamer's `gst-libav` already wraps FFmpeg's codecs; using both would duplicate dependency and version management.
- FFmpeg's Rust bindings are less ergonomic than gstreamer-rs and lack the pipeline timing infrastructure (timestamps, synchronization buses) that GStreamer provides.
- GStreamer's pipeline model is architecturally aligned with tze_hud's timing doctrine (arrival time ≠ presentation time).

**Verdict**: Not appropriate for tze_hud. GStreamer already incorporates FFmpeg codecs where needed.

---

## 10. Integration Guidance for Phase 1 Implementation

This section is non-normative design guidance. The authoritative implementation contract will be authored in RFC 0014 (Media Plane Wire Protocol).

### 10.1 Recommended Cargo Dependencies

```toml
[dependencies]
gstreamer = "0.23"
gstreamer-app = "0.23"
gstreamer-video = "0.23"
gstreamer-audio = "0.23"       # for Opus decode path
gstreamer-pbutils = "0.23"     # for plugin discovery

[build-dependencies]
# None — GStreamer headers found via pkg-config at build time
```

No compile-time feature flags are needed for the core dependency; plugin availability is probed at runtime (§8.1).

### 10.2 Initialization Pattern

```rust
// In compositor startup (called once, before media worker pool creation)
pub fn init_media_subsystem() -> anyhow::Result<()> {
    gst::init()?;
    // Verify required elements are present
    for required in &["appsrc", "appsink", "rtpjitterbuffer", "avdec_h264", "vp9dec"] {
        if gst::ElementFactory::find(required).is_none() {
            anyhow::bail!("Required GStreamer element '{}' not found. Install gstreamer1.0-plugins-good and gstreamer1.0-libav.", required);
        }
    }
    Ok(())
}
```

### 10.3 Pipeline Construction Pattern

```rust
// Non-normative sketch for an H.264 RTP ingress pipeline
fn build_h264_rtp_pipeline(rtp_port: u16) -> anyhow::Result<gst::Pipeline> {
    let pipeline = gst::Pipeline::new();
    let src = gst::ElementFactory::make("udpsrc")
        .property("port", rtp_port as i32)
        .build()?;
    let jitterbuf = gst::ElementFactory::make("rtpjitterbuffer")
        .property("latency", 50u32)  // 50 ms jitter buffer (tune for D18 budget)
        .build()?;
    let depay = gst::ElementFactory::make("rtph264depay").build()?;
    let parse = gst::ElementFactory::make("h264parse").build()?;
    let decode = build_h264_decode_bin(); // probe HW, fallback to avdec_h264
    let sink = gst::ElementFactory::make("appsink")
        .property("emit-signals", true)
        .property("max-buffers", 4u32)  // ring buffer capacity per RFC 0002 §2.8
        .property("drop", true)          // drop-oldest
        .build()?;

    pipeline.add_many([&src, &jitterbuf, &depay, &parse, &decode, &sink])?;
    gst::Element::link_many([&src, &jitterbuf, &depay, &parse, &decode, &sink])?;
    Ok(pipeline)
}
```

### 10.4 AppSink Callback Pattern (Tokio Bridge)

```rust
// Non-normative sketch
let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<DecodedFrameReady>(4);
let app_sink = pipeline
    .by_name("appsink0")
    .and_downcast::<gst_app::AppSink>()
    .unwrap();
app_sink.set_callbacks(
    gst_app::AppSinkCallbacks::builder()
        .new_sample(move |sink| {
            if let Ok(sample) = sink.pull_sample() {
                let buf = sample.buffer().unwrap();
                let video_info = /* from caps */;
                let frame = gst_video::VideoFrame::map(/* ... */);
                let _ = frame_tx.try_send(DecodedFrameReady { /* ... */ });
            }
            Ok(FlowSuccess::Ok)
        })
        .build(),
);

// In Tokio compositor task:
while let Some(frame) = frame_rx.recv().await {
    // Upload to wgpu texture on compositor thread
}
```

---

## 11. Summary

| Criterion | Assessment |
|---|---|
| Platform — Linux | Full; system packages; low risk |
| Platform — macOS | Full; .framework bundle; low risk |
| Platform — Windows | Full with CI complexity; MSVC installer; medium risk |
| Platform — Android | Partial (experimental); GStreamer Android SDK; high risk |
| Platform — iOS | **Not supported** by gstreamer-rs; VideoToolbox alternative required |
| License (LGPL-2.1) | Compatible with dynamic-link deployment model |
| Codec H.264 | Full: SW (avdec_h264) + HW per platform |
| Codec VP9 | Full: SW (vp9dec) + HW (excl. macOS HW path) |
| Codec AV1 | Deferred per D18; GStreamer ready when admitted |
| Opus decode | opusdec; integrates with cpal PCM handoff |
| Latency (jitter buffer) | Must configure `latency=50ms`; default 200ms exceeds D18 budget |
| In-process safety (E24) | Compatible; GStreamer manages own thread pool; Tokio layer orchestrates |
| Tokio runtime integration | Requires GLib main loop bridge; `AppSink`-to-Tokio channel pattern documented |
| GPU surface handoff | CPU-side ring buffer per RFC 0002 §2.8; DMA-BUF zero-copy is post-v2 |
| Hardware-decode probe | Probe-and-fallback required; no single hardware path is universal |
| Dynamic pipeline rebuild | Non-trivial; state machine discipline required; must be exercised in validation |
| EOS / error propagation | Requires explicit GStreamer bus → Tokio bridge |
| Maintenance health | Very healthy; Freedesktop-backed; active corporate maintainers |
| gstreamer-rs version | 0.23.x (GStreamer 1.24.x); next minor break at GStreamer 1.26 |

**Verdict: ADOPT-WITH-CAVEATS.** GStreamer + gstreamer-rs is the correct and already-committed media decode stack for tze_hud v2. The five caveats (iOS gap, jitter buffer tuning, hardware-decode probe-and-fallback, dynamic pipeline rebuild discipline, GStreamer bus → Tokio bridge) are all implementable and well-understood. None require an alternative crate selection for the desktop-first phase 1 scope. iOS requires a separate VideoToolbox path before phase 3 mobile begins.

---

## Discovered Follow-Ups

1. **iOS media decode alternative** (pre-phase-3): gstreamer-rs has no iOS support. Before the iOS device lane (D19 phase 3) opens, a VideoToolbox Rust integration strategy must be selected and audited. Candidate: `video-toolbox` crate or direct `ndk-sys`-style FFI. File as a phase-3 gate before the iOS bead opens.

2. **Android GStreamer SDK evaluation** (pre-phase-3): The Android GStreamer SDK build chain (cross-compilation, NDK integration, `.aar` bundling) needs a dedicated build-system spike before the Android device lane begins. A lighter alternative (`ndk::media` direct bindings) should be considered if the SDK proves too heavyweight.

3. **GStreamer Windows CI bootstrap** (pre-phase-1, Windows real-decode): The Windows CI lane requires the GStreamer MSVC development SDK to be installed as a CI step. The GPU runner box procurement (D18) must confirm its OS and GStreamer version at setup time.

4. **GStreamer CVE monitoring subscription**: Subscribe `tze_hud` maintainers to `gstreamer-security@lists.freedesktop.org` (or equivalent GitHub Security Advisory watch on the `gstreamer/gstreamer` mirror) before any media pipeline ships to users.

---

## Security Advisory Subscription & Response

Before any media pipeline ships to users, tze_hud maintainers must subscribe to GStreamer security advisories and establish a response policy for CVEs affecting the media ingest path.

### Subscription Sources

1. **GStreamer Security Mailing List**
   - Address: `gstreamer-security@lists.freedesktop.org`
   - Scope: Official security advisories for GStreamer core and plugins
   - Subscribe at: https://lists.freedesktop.org/mailman/listinfo/gstreamer-security (or contact the list admin)

2. **RustSec Advisory Database**
   - Crate: `cargo-audit`
   - Scope: Security advisories for `gstreamer-rs` and transitive Rust dependencies (e.g., FFmpeg bindings via gst-libav)
   - Database: https://github.com/rustsec/advisory-db
   - Usage: `cargo audit` in CI/locally to detect known vulnerabilities

3. **GitHub Security Advisories**
   - Repositories to watch:
     - `freedesktop/gstreamer` (upstream C library)
     - `gstreamer/gstreamer` mirror on GitHub
     - `sdroege/gstreamer-rs` (Rust bindings)
   - Enable GitHub security alerts on these repos

4. **NVD CVE Feed**
   - Keyword: "gstreamer"
   - Source: https://nvd.nist.gov/vuln/search
   - Covers: Upstream GStreamer C library CVEs and codec-related vulnerabilities

### Response Policy

| Severity | SLA | Response |
|---|---|---|
| **Critical** (RCE, remote unauthenticated code execution, authentication bypass) | Within 7 days | Patch or deploy mitigation (e.g., lease revocation disabling media plane) |
| **High** (Denial of Service, information leak, privilege escalation) | Within 30 days | Patch and validate; escalate if blockers prevent timely patch |
| **Medium/Low** | Next scheduled gstreamer-rs version bump | Roll into standard dependency update cycle; no expedited response required |

**Critical CVE example**: A buffer overflow in `avdec_h264` parsing that crashes the media worker or leaks heap data would trigger a 7-day SLA. Response options:
- Publish a patched gstreamer-rs / GStreamer version and update Cargo.lock.
- If upstream patch is not available: temporarily revoke media plane lease (via E25 degradation ladder), marking surfaces as "media unavailable" until patch is published.

**High CVE example**: A denial-of-service vector that causes the media worker to hang on malformed input would trigger a 30-day SLA. Patch and validate on the test media pipeline before promoting to production.

### Owner & Escalation

- **Owner**: Media plane operator (to be assigned at phase 1 closeout). Responsible for monitoring, triaging, and coordinating patches.
- **Escalation path**: Any CVE blocking the SLA above escalates to the project lead and is tracked in the project's CVE backlog.
- **Audit trail**: All responses logged in the media pipeline audit log (C17) with CVE ID, severity, response action, and patch date.

### Cross-Reference

- **Codec-level memory safety hardening**: See `hud-lezjj` (defense-in-depth sandboxing and subprocess isolation for codec workers), tracked as post-v2 work.
- **Media plane degradation policy**: See E25 in the v2 signoff packet for the full graceful degradation ladder.

---

## Sources

- GStreamer homepage: https://gstreamer.freedesktop.org/
- GStreamer 1.24 release notes: https://gstreamer.freedesktop.org/releases/1.24/
- gstreamer-rs GitHub: https://github.com/sdroege/gstreamer-rs
- gstreamer-rs 0.23 changelog: https://github.com/sdroege/gstreamer-rs/blob/main/CHANGELOG.md
- gstreamer-rs crates.io: https://crates.io/crates/gstreamer
- GStreamer plugin documentation: https://gstreamer.freedesktop.org/documentation/plugins_doc.html
- GStreamer on Windows (official binary SDK): https://gstreamer.freedesktop.org/download/#windows
- GStreamer on macOS (official .framework): https://gstreamer.freedesktop.org/download/#macos
- GStreamer on Android SDK: https://gstreamer.freedesktop.org/documentation/installing/for-android-development.html
- RFC 0002 §2.8 Media Worker Boundary: `about/legends-and-lore/rfcs/0002-runtime-kernel.md`
- E24 in-process worker posture verdict: `docs/decisions/e24-in-process-worker-posture.md`
- v2 media doctrine: `about/heart-and-soul/media-doctrine.md`
- v2 signoff packet (D18, E22, E24, E25): `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 procurement list: `openspec/changes/v2-embodied-media-presence/procurement.md`
- cpal audio I/O audit (companion): `docs/audits/cpal-audio-io-crate-audit.md`
- MPEG-LA H.264 patent licensing: https://www.mpegla.com/programs/avc-h-264/
- OpenH264 Cisco patent grant: https://blogs.cisco.com/collaboration/cisco-provides-free-h-264-codec-licensing
