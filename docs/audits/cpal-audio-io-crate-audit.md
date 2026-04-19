# cpal Audio I/O Crate Audit

**Issued for**: `hud-ora8.1.19`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-ora8.1 (v2-embodied-media-presence, E22 audio stack)
**Context**: Procurement mandate — `openspec/changes/v2-embodied-media-presence/procurement.md`

---

## Verdict

**ADOPT-WITH-CAVEATS**

cpal is the right foundation for tze_hud's runtime-owned audio routing (E22). It is the canonical Rust audio I/O crate, actively maintained, covers all required desktop platforms, and its callback-driven model integrates cleanly with a Tokio-based runtime. Three caveats must be resolved in the implementation phase:

1. **WASAPI default-device-change tracking requires an explicit workaround** — streams opened against `default_output_device()` on Windows do not automatically follow OS-level device switches. The audio-routing subsystem must register an `IMMNotificationClient` listener and rebuild the stream on `OnDefaultDeviceChanged`, which is the operator-selected sticky device requirement in E22.
2. **Sample-rate and format negotiation is the caller's responsibility** — cpal rejects unsupported configurations rather than adapting. The subsystem must query `supported_output_configs()`, select the configuration closest to Opus's target (48 kHz / stereo), and insert a resampling step when the device native rate differs.
3. **PipeWire backend conflict detection is a live issue** — a known bug (issue #1170) means cpal may select the wrong backend on Linux when both PipeWire and PulseAudio are running. The implementation should explicitly enumerate the desired host (`Host::pipewire()` or `Host::alsa()`) rather than relying on auto-selection.

---

## Scope

E22 from the v2 signoff packet specifies:

> **Audio stack**: Opus codec, stereo channels, runtime-owned routing. Default output device: operator-selected at first run, sticky, changeable via config. Spatial audio is phase 4 refinement.

This audit evaluates `cpal` (the `RustAudio/cpal` crate) as the audio I/O layer for that stack. The codec itself (Opus, via `audiopus` or `opus-rs`) is out of scope. Mixing, DSP, and spatial audio are phase 4 and out of scope.

---

## 1. Crate Identity

| Field | Value |
|---|---|
| Crate | `cpal` |
| Repository | https://github.com/RustAudio/cpal |
| Current version | 0.17.3 |
| Release date | 2026-02-18 |
| MSRV | Rust 1.78 (bumped in 0.17.2) |
| License | Apache-2.0 |
| Stars | ~3,700 |
| Forks | ~496 |
| Total releases | 85 |
| crates.io | https://crates.io/crates/cpal |
| docs.rs | https://docs.rs/cpal |

---

## 2. Platform Backend Coverage

### 2.1 Desktop Platforms (Required for v2 phase 1–3)

| Platform | Default Backend | Optional Backends | Coverage Verdict |
|---|---|---|---|
| **Windows** | WASAPI | ASIO, JACK | Full. WASAPI covers shared-mode stereo output. ASIO is available for exclusive-mode sub-10ms use if needed post-v2. |
| **macOS** | CoreAudio | JACK | Full. CoreAudio is mature and well-exercised in the RustAudio ecosystem. Loopback recording added in 0.17.0 (macOS ≥14.6). |
| **Linux** | ALSA | JACK, PipeWire, PulseAudio | Full with caveats — see §4. PipeWire backend is feature-flag opt-in (`pipewire` feature). ALSA requires `libasound2-dev` at build time even when PipeWire/PulseAudio is the runtime. |
| **BSD** | ALSA | JACK, PipeWire, PulseAudio | Same as Linux. |

### 2.2 Mobile / Embedded Platforms (Required for v2 phase 3)

| Platform | Backend | Notes |
|---|---|---|
| **Android** | AAudio (via `ndk::audio`) | Migrated from `oboe` to direct NDK bindings in 0.16.0. Requires Android API 26+. AAudio is Google's recommended low-latency Android audio API. On Android <8.1, AAudio falls back to OpenSL ES inside the NDK layer. |
| **iOS** | CoreAudio | Same backend as macOS. Stable. |

### 2.3 Non-Native Targets (Out of scope for v2)

| Platform | Backend |
|---|---|
| WebAssembly | Web Audio API (default) or Audio Worklet (feature flag, nightly Rust) |
| tvOS | CoreAudio (Tier 3, experimental) |

### 2.4 Notable Gaps

- **No Windows WASAPI exclusive mode** without ASIO. Shared-mode WASAPI latency is typically 30–50 ms. The E22 requirement (Opus, stereo, runtime-owned routing) does not specify a hard latency target; the v2 glass-to-glass budget (p50 ≤150 ms, p99 ≤400 ms, per D18) is for video, not audio-only. Shared-mode WASAPI is adequate for v2.
- **No PipeWire by default on Linux**. The `pipewire` feature flag must be explicitly enabled. This is correct behavior — tze_hud should select the backend at runtime based on the host environment rather than relying on a compile-time default.
- **No JACK by default on any platform**. Opt-in via the `jack` feature flag. JACK is appropriate for pro-audio routing setups but not a v2 requirement.

---

## 3. API Stability

### 3.1 Version and Semver Posture

cpal is pre-1.0 (`0.x.y`). The crate has been under active development for approximately 8+ years. The pace of breaking changes has been moderate:

| Period | Breaking versions | Key changes |
|---|---|---|
| Dec 2024 | 0.16.0 | AAudio backend migration from `oboe` to `ndk::audio`; `Stream::clone()` removed from CoreAudio |
| Dec 2024–Feb 2025 | 0.17.0–0.17.3 | `SampleRate` changed from struct to `u32` type alias; `DeviceBusy` variant added then reverted (0.17.2 yanked, 0.17.3 reverts it) |
| Unreleased (master) | — | Error enums marked `#[non_exhaustive]`; `StreamInstant` API redesigned; `DeviceTrait::build_*_stream()` takes `StreamConfig` by value |

### 3.2 Observed Stability Pattern

Breaking changes arrive roughly once per three months at minor version boundaries (`0.N.0`). Patch releases fix regressions quickly (0.17.2 yanked within days; 0.17.3 followed the same day with the revert). This is acceptable for a pre-1.0 crate in active development.

The `#[non_exhaustive]` annotation on error enums (landed in unreleased master) is a stabilization signal — it means error handling code will not need to change when new variants are added.

### 3.3 API Design Stability

The core API surfaces have been stable in shape for several releases:

- `Host → Device → Stream` three-tier model is unchanged.
- Callback-driven `build_output_stream()` / `build_input_stream()` pattern is stable.
- `BufferSize::Fixed(u32)` and `BufferSize::Default` have been available since at least 0.14.x.
- `supported_output_configs()` / `supported_input_configs()` for capability discovery is stable.

---

## 4. Latency Characteristics

cpal does not publish formal latency benchmarks. The following figures are derived from the audio engineering community's documented behavior for the underlying backends:

### 4.1 Achievable Latency by Backend

| Backend | Typical Shared-Mode Latency | Minimum Buffer (frames) | Notes |
|---|---|---|---|
| WASAPI (Windows) | 30–50 ms shared mode | ~480 frames @ 48kHz | Exclusive mode (requires ASIO feature) can achieve 3–10 ms |
| CoreAudio (macOS) | 10–20 ms default | ~256 frames @ 44.1kHz | `IOBufferFrameSize` is configurable; 128 frames (~3 ms) is common in DAW setups |
| ALSA (Linux) | 5–20 ms | 64–256 frames | Varies heavily by hardware; ALSA period size is set via `BufferSize::Fixed` |
| PipeWire (Linux) | ~21 ms default | 1024 frames @ 48kHz (default quantum) | Quantum is configurable down to 32 frames; PipeWire's compositor adds one quantum of latency |
| AAudio (Android) | 10–40 ms | API 27+ with exclusive mode: ~3–5 ms | Low-latency mode requires explicit `PerformanceMode::LowLatency` — the NDK backend passes this through |

### 4.2 Buffer Size Control

cpal exposes `BufferSize` in `StreamConfig`:

```rust
StreamConfig {
    channels: 2,
    sample_rate: SampleRate(48_000),
    buffer_size: BufferSize::Fixed(256), // or BufferSize::Default
}
```

`BufferSize::Default` defers to the OS/driver default. `BufferSize::Fixed(n)` requests a specific frame count; the backend may round up to the nearest hardware-supported size. On ALSA, `BufferSize::Default` can produce anything from a PipeWire quantum (1024 frames) to `u32::MAX` on misconfigured hardware — always use `BufferSize::Fixed` in production.

### 4.3 Callback vs Poll

cpal is strictly **callback-driven**, not poll-based. The runtime spawns a dedicated high-priority audio thread that calls the data callback at the required cadence. The callback receives a mutable output buffer to fill. This is the correct model for tze_hud: the audio-routing subsystem enqueues decoded Opus frames into a ring buffer; the cpal callback drains it.

### 4.4 Real-Time Priority

When the `audio_thread_priority` feature is enabled, cpal elevates the audio thread to real-time priority via `rtkit` on Linux or equivalent mechanisms on other platforms. This is the correct posture for the tze_hud media worker. Note: on Linux, `rtkit` requires either appropriate `limits.conf` entries or user capabilities.

---

## 5. Maintenance Health

| Metric | Observation |
|---|---|
| Last release | 0.17.3 — 2026-02-18 (2 months ago) |
| Release cadence | 3–5 releases per year, accelerating in late 2024–2025 |
| Active issues | Several longstanding platform-specific bugs (oldest: 2019–2020); recent issues filed April 2026 |
| Contributor activity | Regular PRs from multiple contributors; not a single-maintainer project |
| Organisation | Under the `RustAudio` GitHub org, which also houses `rodio`, `cpal`, and related crates |
| Dependents | Used directly by `rodio` (the primary high-level Rust audio library) and many game-audio crates including `kira` |
| Breaking-change handling | 0.17.2 yanked within days when it introduced an inadvertent SemVer break; patch reverted promptly |

**Assessment**: Healthy. The crate is the de facto standard for Rust audio I/O. Its position as the backend for `rodio` and `kira` means regression pressure from dependents is high, which is a positive maintenance signal.

---

## 6. Known Gotchas

### 6.1 WASAPI Default-Device-Change Tracking (Critical for E22)

**Issue #740**: When a stream is created against `default_output_device()` and the OS default device is changed, the stream does not follow. The audio-routing subsystem must:

1. Register an `IMMNotificationClient` via the Windows crate's "implement" feature.
2. Listen for `OnDefaultDeviceChanged`.
3. Rebuild the cpal stream against the new device.

This is a known gap; PR #1027 exists but has not merged as of the audit date. The implementation must handle this explicitly. This is the same mechanism needed for E22's "sticky, changeable via config" requirement.

### 6.2 Sample-Rate Negotiation

**Issue #593**: `build_output_stream` with a sample rate that does not match the device's shared-mode default returns `StreamConfigNotSupported` on Windows WASAPI. The subsystem must:

1. Call `supported_output_configs()` to enumerate supported ranges.
2. Select a config within the supported range closest to 48 kHz.
3. If 48 kHz is not in range, insert a resampler (e.g., `rubato`) between the Opus decoder and the cpal callback.

Opus decodes to 48 kHz natively. Most modern hardware supports 48 kHz in shared mode, so resampling should be the uncommon path.

### 6.3 Channel Layout Assumptions

cpal works in terms of channel count only — it has no knowledge of channel semantics (left/right, surround positions). For stereo output (E22: "stereo channels"), this is not a limitation. Spatial audio (E25 degradation ladder: "spatial audio → framerate") is phase 4 and will require a DSP layer above cpal regardless.

### 6.4 Format Conversion Overhead

cpal does not perform format conversion. If the device native format is `F32` but the output pipeline produces `I16` (Opus PCM output), the subsystem must perform the conversion in the data callback. This is a single-pass multiply, not a latency concern, but it must be explicit.

### 6.5 PipeWire / PulseAudio Backend Conflict

**Issue #1170** (filed 2026-04-18): When both PipeWire and PulseAudio are running, cpal may select the wrong backend. On modern Linux systems (Ubuntu 22.04+, Fedora 38+), PipeWire runs as a PulseAudio compatibility layer. The host should be selected explicitly:

```rust
// Prefer PipeWire on Linux; fall back to ALSA
#[cfg(target_os = "linux")]
let host = cpal::host_from_id(cpal::HostId::PipeWire)
    .unwrap_or_else(|_| cpal::default_host());
```

### 6.6 ALSA: Startup Noise on Device Enumeration

**Issue #384** (longstanding): ALSA produces unwanted stderr output when enumerating devices with `devices()`. This can be suppressed by using `libasound2-dev` debug flags or redirecting stderr during enumeration. Low severity for tze_hud since device enumeration happens once at startup.

### 6.7 32-bit Linux Platform Crash

**Issue #1134** (filed 2026-03-26): Segfault on 32-bit platforms with 64-bit `time_t`. tze_hud's target platforms are all 64-bit per procurement.md hardware specs; this is not a concern.

---

## 7. Alternatives Assessment

### 7.1 `rodio`

**Verdict**: Not appropriate as the I/O layer.

`rodio` is a high-level audio playback library that wraps cpal. It adds format decoding (via Symphonia), mixing, and effect primitives. For tze_hud, these abstractions sit in the wrong layer — the audio-routing subsystem must control timing, format, and routing directly. Use cpal directly.

### 7.2 `kira`

**Verdict**: Not appropriate as the I/O layer.

`kira` is a game-audio manager that provides sound instances, tweening, and effect chains on top of cpal. Designed for declarative "play sound effect" usage. The tze_hud media plane needs PCM-level control for Opus decode → output pipeline. kira adds abstractions that obscure the timing model. Use cpal directly.

### 7.3 `oboe` (Rust bindings for Google's Oboe C++ library)

**Verdict**: Not appropriate.

The `oboe` crate wraps Google's Oboe C++ library, which itself wraps AAudio/OpenSL ES. cpal 0.16.0+ directly uses the NDK's AAudio bindings (`ndk::audio`) for Android, removing the need for the Oboe C++ intermediary. The `oboe` Rust crate's last meaningful release predates cpal's AAudio migration and is effectively superseded for Android use within cpal itself.

### 7.4 Raw OS Bindings

**Verdict**: Not appropriate for v2.

Direct use of `windows` crate WASAPI APIs, `alsa` crate, or `coreaudio-sys` would give maximum control at the cost of significant per-platform implementation work, a custom device-enumeration layer, and ongoing maintenance of platform-specific audio thread management. cpal already encapsulates this correctly and is the established community abstraction. Raw bindings would be appropriate only if cpal's callback model introduced a hard architectural constraint (e.g., callback cannot be async-aware) — which it does not, as the ring-buffer pattern is well-established.

### 7.5 GStreamer Audio Sink (via `gstreamer-audio` crate)

**Verdict**: Viable alternative for consideration, but not necessary.

GStreamer is already locked in for media ingest, decode, timing, and synchronization (CLAUDE.md). GStreamer has native audio output sinks (`autoaudiosink`, `wasapi2sink`, `alsa2sink`, `pipewiresink`) that handle device selection, format negotiation, and resampling automatically. If the audio-routing subsystem needs to output audio that is already flowing through a GStreamer pipeline (e.g., Opus decode → PCM → output), using GStreamer's audio sink directly may be simpler than demuxing PCM into a cpal stream.

**However**: E22 specifies "runtime-owned routing," implying the runtime controls the output device selection and can re-route independently of stream lifecycle. GStreamer's sink management is pipeline-centric and less amenable to runtime device-switch semantics. cpal gives the subsystem full control.

**Decision**: Use cpal as the audio I/O layer. Use GStreamer for upstream decode (the Opus decode step). Hand off PCM at the cpal boundary via a ring buffer. This is the clean interface: GStreamer owns decode timing, cpal owns output hardware.

---

## 8. Integration Guidance for E22 Audio-Routing Subsystem

This section is non-normative design guidance, not a final spec. The actual subsystem design bead (post-audit) owns the authoritative decisions.

### 8.1 Recommended Feature Flags

```toml
[dependencies]
cpal = { version = "0.17", features = ["audio_thread_priority"] }

# Linux: enable PipeWire explicitly for modern distros
# Windows: ASIO is out of scope for v2; add when spatial audio is tackled
```

### 8.2 Device Selection Pattern (E22: operator-selected, sticky)

```rust
// Pseudo-code for the audio-routing subsystem
fn select_output_device(config: &AudioConfig) -> cpal::Device {
    let host = platform_default_host(); // see §6.5 for Linux nuance
    match &config.output_device_id {
        Some(id) => host.device_by_id(id)    // stable device IDs added in 0.17.0
                        .unwrap_or_else(|_| host.default_output_device().unwrap()),
        None => host.default_output_device().unwrap(),
    }
}
```

Stable device IDs (`device_by_id`, added in 0.17.0) are the mechanism for operator-selected sticky devices. The audio config stores the device ID; at first run, the operator is prompted and the selected ID persisted.

### 8.3 Stream Configuration Pattern

```rust
// Enumerate supported configs, find closest to 48kHz stereo
let mut configs = device.supported_output_configs()?;
let config = configs
    .find(|c| c.channels() == 2 &&
              c.min_sample_rate() <= SampleRate(48_000) &&
              c.max_sample_rate() >= SampleRate(48_000))
    .map(|c| c.with_sample_rate(SampleRate(48_000)))
    .or_else(|| {
        // fallback: use max supported rate, insert resampler
        device.default_output_config().ok()
    })
    .expect("no usable output config");
```

### 8.4 Ring-Buffer Pattern (Tokio integration)

The cpal data callback runs on a dedicated non-Tokio thread. Use a lock-free ring buffer (e.g., `ringbuf`) to bridge:

- **Producer side**: Tokio task receives decoded Opus PCM, writes to ring buffer.
- **Consumer side**: cpal callback reads from ring buffer, writes to output buffer.
- **Underrun handling**: If the ring buffer is empty (producer stalled), fill with silence. Do not block the callback.

This pattern preserves cpal's real-time callback contract without requiring async-aware callbacks.

---

## 9. Summary

| Criterion | Assessment |
|---|---|
| Platform coverage — desktop | Full (WASAPI, CoreAudio, ALSA, PipeWire) |
| Platform coverage — mobile | Full (AAudio/Android, CoreAudio/iOS) |
| API stability | Pre-1.0, moderate breaking-change cadence; core stream API stable |
| Latency — shared mode | 10–50 ms by platform; adequate for v2 glass-to-glass budget |
| Latency — low-latency mode | ASIO/exclusive: 3–10 ms (post-v2) |
| Maintenance health | Active; RustAudio org; depended on by rodio, kira |
| WASAPI device-change tracking | Gap — requires explicit `IMMNotificationClient` (known issue #740) |
| Sample-rate negotiation | Caller responsibility; resampler needed if device ≠ 48 kHz |
| PipeWire conflict | Known bug #1170; explicit host selection required |
| Format conversion | Caller responsibility; no automatic conversion |

**Verdict: ADOPT-WITH-CAVEATS.** The three caveats (WASAPI device-change, sample-rate negotiation, PipeWire host selection) are implementable and well-understood. None require alternative crate selection.

---

## Sources

- cpal GitHub repository: https://github.com/RustAudio/cpal
- cpal crates.io: https://crates.io/crates/cpal
- cpal docs.rs: https://docs.rs/cpal/latest/cpal/
- cpal releases (0.15.3–0.17.3): https://github.com/RustAudio/cpal/releases
- cpal CHANGELOG (v0.17.0): https://github.com/RustAudio/cpal/blob/v0.17.0/CHANGELOG.md
- Issue #740 (WASAPI device-change): https://github.com/RustAudio/cpal/issues/740
- Issue #593 (WASAPI sample rate): https://github.com/RustAudio/cpal/issues/593
- Issue #628 (ALSA duplex API): https://github.com/RustAudio/cpal/issues/628
- Issue #1170 (PipeWire/PulseAudio conflict): https://github.com/RustAudio/cpal/issues/1170
- DeepWiki: RustAudio/cpal architecture: https://deepwiki.com/RustAudio/cpal
- DeepWiki: Device enumeration: https://deepwiki.com/RustAudio/cpal/5.4-device-enumeration
- DeepWiki: Building and using cpal: https://deepwiki.com/RustAudio/cpal/4-building-and-using-cpal
- v2-embodied-media-presence signoff packet E22: `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 procurement list: `openspec/changes/v2-embodied-media-presence/procurement.md`
