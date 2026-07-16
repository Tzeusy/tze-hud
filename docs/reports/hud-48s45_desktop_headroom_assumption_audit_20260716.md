# hud-48s45 Desktop-Headroom Assumption Audit

Issue: `hud-48s45`

Date: 2026-07-16

Scope: native Windows v1 runtime, compositor, and input path

Decision boundary: findings only; no glasses/VR target or device implementation

## Executive Summary

The Windows runtime already contains several foundations worth preserving: an
idle render gate stops unchanged scenes from reaching GPU submission, expensive
text/widget work is cached or dirty-gated, render surfaces are abstracted from
the compositor, and the input crate has a device-neutral command vocabulary.

The audit nevertheless found six architectural risks and two surfaces that need
an explicit degradation path before the same architecture can honestly claim a
glasses/VR-class envelope. The most important risks are not missing device
features. They are desktop-headroom assumptions in the current Windows path:

- both main and compositor loops continue polling while the scene is idle;
- every active frame rebuilds owned draw data for the whole visible scene;
- the resolved display profile does not drive the windowed frame cadence or
  compositor degradation state;
- frame budgets and degradation thresholds are fixed to a 60 Hz envelope;
- one compositor frame means one 2D texture view, followed by a synchronous GPU
  wait; and
- pointer-free command input is implemented as a library seam but has no
  production runtime producer/consumer path.

These findings do **not** reopen mobile, glasses, VR, or multi-device work.
`about/heart-and-soul/v1.md` keeps execution Windows-only while
`about/heart-and-soul/efficiency.md` requires the Windows architecture to stop spending desktop
headroom now. The appropriate output is current Windows efficiency/spec
reconciliation plus explicit inputs for a future device-profile OpenSpec change,
not a device implementation.

## Scope and Classification

The audit followed the concrete vectors named by the issue: idle/full-scene
render behavior, per-frame allocations, texture and memory watermarks,
single-swapchain/monoscopic assumptions, 60 Hz timing, and pointer/keyboard input
assumptions. It inspected the production call sites, not only types and tests.

Classifications mean:

- **fine** — the current abstraction survives the future envelope; preserve it.
- **needs-degradation-path** — the mechanism is sound for Windows, but a smaller
  envelope needs an explicit lower-cost mode, bound, or admission rule.
- **architectural-risk** — the current ownership or execution contract embeds an
  assumption that cannot be solved by tuning a constant alone.

Doctrine anchors:

- Windows-only execution remains authoritative:
  `about/heart-and-soul/v1.md:9-17`.
- Desktop headroom is not a budget; the future envelope includes stereo at
  90-120 Hz: `about/heart-and-soul/efficiency.md:8-26`.
- Idle cost, change-proportional work, and designed degradation are active
  compute requirements: `about/heart-and-soul/efficiency.md:33-53`.
- Device implementation remains deferred while the same scene/API model and
  negotiated budgets remain the intended direction:
  `about/heart-and-soul/mobile.md:1-5,15-38`.

## Findings Matrix

| ID | Surface | Classification | Current Windows evidence | Future spec input |
|---|---|---|---|---|
| F1 | Unchanged-scene GPU work | **fine** | The compositor compares `scene.version` and `geometry_epoch`, includes animation/composer dirtiness, and drops the scene lock without build/encode/present when clean (`crates/tze_hud_runtime/src/windowed/mod.rs:1020-1031,1158-1189`). | Preserve as a normative invariant and add constrained-envelope measurement; no device feature required. |
| F2 | Idle CPU/wakeup behavior | **architectural-risk** | The compositor loop still wakes at `target_fps`, takes the scene lock, runs expiry/cache/animation sweeps, and sleeps for the remaining frame interval (`crates/tze_hud_runtime/src/windowed/mod.rs:1039-1056,1095-1170,1401-1408`). The winit loop uses `ControlFlow::Poll`, while `about_to_wait` performs substantial portal/input/hover work every iteration (`crates/tze_hud_runtime/src/windowed/mod.rs:572-650,2355-2359`). | Define an event/deadline-driven wake contract for the Windows runtime: mutation/input/channel wakeups plus the next real TTL/animation deadline, with measured idle wakeups/CPU. This is current efficiency work, not device work. |
| F3 | Dirty-frame scene work and transient allocation | **needs-degradation-path** | Any dirty/animated frame calls `build_windowed_frame`, which rebuilds geometry, text inputs, drag/focus/context-menu geometry, widget quads, and owned vectors for the complete visible scene (`crates/tze_hud_compositor/src/renderer/frame.rs:405-589`). `build_frame_vertices` sorts all visible tiles, creates fresh vertex/texture-command vectors, and recursively visits each root (`crates/tze_hud_compositor/src/renderer/frame.rs:94-125,185-210`). The encode path creates a fresh full vertex buffer and command encoder (`crates/tze_hud_compositor/src/renderer/mod.rs:2031-2050`). | Specify retained per-tile draw artifacts or dirty-region rebuild semantics so one-node changes are proportional to changed content. Keep a bounded full rebuild as fallback; do not prescribe an implementation before measurements. |
| F4 | Expensive-content caching and lifetime hygiene | **fine** | Markdown/truncation primes are scene-version gated before render (`crates/tze_hud_runtime/src/windowed/mod.rs:1122-1142`); widget textures are dirty-synced before acquisition (`crates/tze_hud_compositor/src/renderer/frame.rs:541-551`); image textures unused by the current scene are evicted (`crates/tze_hud_compositor/src/renderer/image_cache.rs:774-784`); widget raster caches are bounded LRU stores (`crates/tze_hud_compositor/src/widget.rs:1769-1878`). | Preserve cache correctness and eviction tests. Capacity policy belongs to F5. |
| F5 | Texture/resource/cache watermarks | **needs-degradation-path** | Per-agent defaults allow 256 MiB of textures and a 2 GiB hard maximum (`crates/tze_hud_scene/src/types.rs:587-657`; `crates/tze_hud_runtime/src/session.rs:27-59`). The resource store defaults to 64 MiB per decoded texture and 512 MiB total (`crates/tze_hud_resource/src/types.rs:165-183,210-245`). Five process-global widget pixmap caches have fixed 32-48 MiB caps totaling 208 MiB (`crates/tze_hud_compositor/src/widget.rs:1776-1778,1957-1965`). The bounds prevent unbounded growth, but they are fragmented and desktop-sized rather than one profile/pressure-governed envelope. | Future profile specs should define one aggregate CPU/GPU/cache budget, per-cache shares, pressure-triggered eviction/quality reduction, and admission failure semantics. Current Windows work may expose/account these totals without activating a device profile. |
| F6 | Display profile as an operational envelope | **architectural-risk** | `RuntimeContext` stores the resolved profile as the configuration source of truth (`crates/tze_hud_runtime/src/runtime_context.rs:108-170`), but the windowed cadence comes independently from CLI/env `opts.fps` (`app/tze_hud_app/src/main.rs:1023-1051`). The profile's truncation-input bound is consumed in both windowed and headless compositor setup (`crates/tze_hud_runtime/src/windowed/mod.rs:883-891`; `crates/tze_hud_runtime/src/headless.rs:288-291`), and config loading rejects registered-agent tile/texture/update overrides above the active profile (`crates/tze_hud_config/src/loader.rs:694-770`). Those consumers do not yet make the profile one operational runtime envelope: windowed `target_fps`, compositor degradation, cache totals, and runtime admission defaults remain independently sourced. | Reconcile configuration/runtime specs so the selected profile feeds one immutable operational envelope consumed by admission, caches, cadence, and degradation. This is also a current spec-to-code seam because RFC 0006 says profiles shape what the runtime enforces (`about/legends-and-lore/rfcs/0006-configuration.md:901-919`). |
| F7 | Refresh-rate-derived frame budgets | **architectural-risk** | Pipeline constants fix total time to 16.6 ms and input-to-next-present to 33 ms (`crates/tze_hud_runtime/src/pipeline.rs:72-99`). Degradation fixes 14/12 ms thresholds and 10/30-frame windows explicitly described at 60 fps (`crates/tze_hud_runtime/src/degradation.rs:27-48`). The controller itself has no production consumer outside its definition/re-export, so compositor level remains nominal. At 90/120 Hz, frame periods are about 11.1/8.3 ms, making the current trigger later than a missed-frame boundary. | Define budgets as functions of negotiated presentation cadence (with absolute local-input ceilings retained), express hysteresis windows in time, and specify production wiring from telemetry to compositor quality/shedding. Do not add a 90/120 Hz lane until device scope reopens. |
| F8 | Output-view model (monoscopic/single surface) | **architectural-risk** | `CompositorFrame` owns exactly one `TextureView`; `CompositorSurface` acquires one frame and reports one width/height pair (`crates/tze_hud_compositor/src/surface.rs:88-130`). `Compositor` stores scalar width/height (`crates/tze_hud_compositor/src/renderer/mod.rs:86-121`), and render passes bind one color attachment (`crates/tze_hud_compositor/src/renderer/mod.rs:2061-2079`). There is no view/eye identity or array-layer contract at the compositor boundary. | A future device-profile change must decide explicitly between local multiview/stereo composition and upstream precomposition. Either choice needs a view-set/presentation contract before code; do not infer that two compositor instances are coherent. |
| F9 | GPU submission/pipelining | **architectural-risk** | The windowed path submits one command buffer and immediately calls `device.poll(wgpu::Maintain::Wait)` before releasing the frame (`crates/tze_hud_compositor/src/renderer/frame.rs:681-699`). This serializes CPU progress behind GPU completion and leaves no declared frames-in-flight policy. | Specify bounded frames in flight, ownership/present acknowledgements, and backpressure for high-refresh envelopes. Validate that any asynchronous path preserves surface-lifetime safety and input/present telemetry. |
| F10 | Device-neutral command input model | **fine** | `CommandSource` includes keyboard, D-pad, voice, clicker, rotary, and programmatic sources; `RawCommandEvent` and `CommandProcessor` keep action semantics independent of a pointer (`crates/tze_hud_input/src/command.rs:66-100,141-233`). The current OpenSpec requires command input and pointer-free navigation (`openspec/specs/input-model/spec.md:394-434`). | Preserve the core vocabulary and focus/action semantics; no parallel wearable-only input API is needed. |
| F11 | Production pointer-free input wiring | **architectural-risk** | The production winit adapter handles `CursorMoved`, `MouseInput`, `MouseWheel`, and `KeyboardInput` directly (`crates/tze_hud_runtime/src/windowed/mod.rs:1501-1565`). `CommandProcessor` has no production runtime consumer; repository-wide uses outside its module are integration tests. Thus the device-neutral core in F10 is not a reachable production seam. | Define an input-capability/adapter boundary that normalizes OS/HID/voice inputs into command events before focus/action dispatch. This is partly current OpenSpec reconciliation; device-specific sources remain deferred. |

## Interpretation

### What is already aligned

The renderer is not an unconditional 60 Hz GPU submitter anymore. F1 is a real
improvement: static scenes skip build, encode, and present. The expensive text,
image, and widget paths also demonstrate the right shape—commit-time work,
dirty flags, bounded caches, and eviction. The surface trait and command-input
types are useful portability seams even though their current cardinality and
production wiring are incomplete.

These mechanisms should not be replaced with a speculative wearable fork. They
should become the foundations of a single negotiated envelope.

### Where desktop headroom is still embedded

F2 and F3 expose a two-part idle/change-proportionality gap. The GPU is quiet
when static, but CPU loops continue to wake; once any animation or composer
caret keeps the scene active, the entire visible scene is rebuilt into fresh
owned draw data. Desktop hardware can hide both costs. A battery/thermal
envelope cannot.

F5-F7 show that budget *data* exists without one operational authority. Profile
objects, per-agent budgets, resource-store limits, raster-cache limits,
degradation levels, CLI FPS, and frame-budget constants are separate surfaces.
The future problem is not selecting smaller numbers independently. It is making
one negotiated profile govern all of them coherently and proving the production
runtime actually consumes it.

F8 and F9 are the deepest render-contract risks. A single 2D target followed by
a blocking GPU wait is coherent for the present Windows overlay. It does not
state how stereo views share scene state, timing, culling, resource lifetime, or
presentation acknowledgement. That decision must be made at the spec/RFC level
when device work is reopened.

F10/F11 show the same pattern in input: the right semantic abstraction exists,
but production still enters through mouse/keyboard-shaped callbacks. Tests of
`CommandProcessor` prove the pure seam, not that a pointer-free device can reach
it.

## Spec Work Routing

No device-specific implementation or new device lane is authorized by this
report. The findings route as follows:

| Timing | Findings | Appropriate next artifact |
|---|---|---|
| Current Windows efficiency reconciliation | F2, F3 | Delta requirements for idle wakeups/CPU and change-proportional render work, with Windows/headless measurement scenarios. |
| Current configuration/runtime reconciliation | F5, F6, F7, F11 | Audit/delta requirements proving the resolved profile and degradation/input abstractions have production consumers. Device-specific scenarios remain excluded. |
| Future device-profile reopening only | F5, F7, F8, F9, F11 | A device-profile OpenSpec/RFC amendment defining constrained budgets, cadence-derived timing, local multiview vs upstream composition, frames in flight, and input-capability adapters. |
| Preserve as invariants | F1, F4, F10 | Regression requirements/tests; no architecture replacement. |

The future change should use the existing `configuration`, `runtime-kernel`,
`input-model`, `resource-store`, and `validation-framework` capabilities. It
should not revive the parked v2 change wholesale.

Tracker routing is deliberately deduplicated:

- `hud-le1e0` already owns the efficiency-budget delta for bounded idle CPU
  wakeups and change-proportional/damage-scoped work; `hud-hnigs` owns its idle
  GPU/wakeup telemetry and CI enforcement. F2/F3 should feed those beads rather
  than create a parallel requirements change. `hud-0jfqd` is the existing RFC
  0002 timing-reconciliation decision that gates the quiescence contract.
- F5/F6/F7/F11 identify a distinct current configuration/runtime/input
  reconciliation seam: prove that the resolved profile, degradation controller,
  and command-input abstraction have the required production consumers. This
  should be tracked separately from the idle-work delta.
- F8/F9 and the device-envelope portions of F5/F7/F11 remain future spec inputs
  only. Do not create device implementation work until the owner reopens that
  lane.

## Reproduction Pointers

Read-only commands used to verify production consumers and absence claims at
`origin/main` commit `d33ad12f`:

```bash
rg -n -U 'runtime_context\s*\.profile|\.profile\.(target_fps|min_fps|max_tiles|max_texture_mb|max_agents|max_agent_update_hz|max_truncation_input_bytes)' \
  crates/tze_hud_runtime app/tze_hud_app --glob '*.rs'
rg -n 'DegradationController' --glob '*.rs' \
  --glob '!crates/tze_hud_runtime/src/degradation.rs' .
rg -n 'CommandProcessor|RawCommandEvent' \
  --glob '*.rs' --glob '!crates/tze_hud_input/src/command.rs' .
rg -n 'ControlFlow::Poll|frame_interval|Maintain::Wait|build_windowed_frame|Vec::new' \
  crates/tze_hud_runtime crates/tze_hud_compositor --glob '*.rs'
```

The first search returns the windowed and headless truncation-size production
consumers plus `runtime_context.rs` tests, but no windowed cadence,
compositor-degradation, cache-total, or admission-default consumer. Config-load
validation separately consumes profile ceilings for registered-agent overrides.
The second search returns only the public re-export. The third returns the public
re-export and integration tests, not a production runtime producer for command
input.
