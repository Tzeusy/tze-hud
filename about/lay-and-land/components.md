# Component Inventory

This map documents every crate in the tze_hud workspace: what it does, what it depends on, what crosses its boundaries, and how the pieces compose into the runtime.

## Crate Inventory

### Core crates (13)

| Crate | Plane(s) | Responsibility | Key entry points |
|---|---|---|---|
| `tze_hud_scene` | None (data model) | Pure scene graph: `Scene` → `Tab[]` → `Tile[]` → `Node[]`. Types, mutations, diffs, timing, leases, validation. No GPU dependency. | `graph::SceneGraph`, `mutation::MutationBatch`, `diff::SceneDiff`, `timing::TimingHints`, `lease::capability::*` |
| `tze_hud_compositor` | None (renderer) | wgpu compositor: renders scene graph to native window or headless offscreen texture. Text rasterisation (glyphon), SVG widget rendering (resvg), GPU adapter selection. | `renderer::Compositor`, `surface::{WindowSurface, HeadlessSurface}`, `adapter::select_gpu_adapter`, `widget::WidgetRenderer`, `text::TextRasterizer` |
| `tze_hud_runtime` | MCP + gRPC | Runtime kernel: 8-stage frame pipeline orchestration, 4-thread model, budget enforcement, admission control, channels, shell/chrome, quiet hours, attention budgets, reload triggers. | `windowed::WindowedRuntime`, `headless::HeadlessRuntime`, `pipeline::FramePipeline`, `budget::BudgetEnforcer`, `channels::ChannelSet`, `mcp::start_mcp_http_server` |
| `tze_hud_protocol` | gRPC | gRPC session protocol: protobuf codegen, session server, auth, lease management, MCP bridge, subscription dispatch, widget asset store, dedup. | `proto::*` (generated), `session_server::*`, `session::*`, `auth::*`, `mcp_bridge::*` |
| `tze_hud_input` | None (pipeline) | Input pipeline: pointer events, hit-testing, pointer capture, focus tree/manager, keyboard dispatch, command model, local feedback (< 4 ms), event coalescing/batching. | `InputProcessor::process()`, `dispatch::DispatchProcessor`, `focus::FocusManager`, `capture::PointerCaptureManager`, `coalescing::FrameCoalescer` |
| `tze_hud_telemetry` | None (observability) | Structured per-frame telemetry: timing, throughput, resource metrics as machine-readable JSON. Hardware-normalised validation. | `collector::TelemetryCollector`, `record::FrameTelemetry`, `resource_monitor::ResourceMonitor`, `validation::ValidationReport` |
| `tze_hud_mcp` | MCP | MCP compatibility bridge: JSON-RPC 2.0 server exposing named tools (`create_tab`, `create_tile`, `set_content`, `dismiss`, `publish_to_zone`, `list_zones`, `list_scene`, `register_widget_asset`). | `server::McpServer`, `tools::*`, `types::{McpRequest, McpResponse}` |
| `tze_hud_a11y` | None (platform bridge) | Accessibility bridge: converts scene graph to platform a11y tree. Stubs for AT-SPI2 (Linux), UIA (Windows), NSAccessibility (macOS); no-op default. | `AccessibilityTree` trait, `NoopAccessibility`, `AccessibilityConfig` |
| `tze_hud_policy` | None (pure evaluator) | 7-level policy arbitration stack (Human Override → Safety → Privacy → Security → Attention → Resource → Content). Pure read-only evaluator; no side effects. Not wired at runtime in v1. | `stack::ArbitrationStack::evaluate()`, `mutation::evaluate_batch()`, `frame::evaluate_frame()`, `event::evaluate_event()` |
| `tze_hud_config` | None (loading) | TOML configuration: file resolution, schema validation, display profiles, agent registration, privacy/quiet-hours, zone registry, component profiles/types, hot reload (SIGHUP). | `loader::TzeHudConfig`, `resolver::resolve_config_path`, `reload::reload_config`, `profile::resolve_profile`, `policy_builder::build_effective_policy` |
| `tze_hud_resource` | None (storage) | Content-addressed resource store: BLAKE3 dedup, inline/chunked upload, validation pipeline, refcounting, GC, per-agent decoded-byte budget, cross-agent sharing, font cache, runtime widget durable store. | `upload::ResourceStore`, `dedup::DedupIndex`, `budget::BudgetRegistry`, `gc::GcRunner`, `font_cache::FontCache`, `runtime_widget_store::RuntimeWidgetStore` |
| `tze_hud_validation` | None (testing) | Visual regression (Layer 2 SSIM comparison, perceptual hash pre-screening) and developer visibility artifacts (Layer 4 index.html + manifest.json generation). | `layer2::Layer2Validator::compare()`, `layer4::ArtifactBuilder`, `ssim::compute_ssim`, `phash::compute_phash` |
| `tze_hud_widget` | None (loading) | Widget asset bundle loader: scans directories for `widget.toml` manifests, validates SVG files, resolves parameter bindings, structured error codes. | `loader::{scan_bundle_dirs, load_bundle_dir}`, `manifest::*`, `svg_ids::*`, `runtime_registration::register_runtime_widget_svg_asset` |

### Media plane subsystems (2)

These subsystems activate only when the `media-ingress` capability is granted
(RFC 0008 Amendment A1). They are in-process, compositor-owned, and never
access the wgpu GPU device directly.

| Subsystem | Plane(s) | Responsibility | Key entry points |
|---|---|---|---|
| `media-worker-pool` (E24) | Media plane (in-process) | N = 2–4 compositor-owned tokio tasks; each manages one GStreamer pipeline for one active `media-ingress` stream. Capability-gated activation: `media-ingress` capability grant required before any worker spawns. Priority-preempted (lease_priority sort per RFC 0008 §2.2). Watchdog-bounded: CPU time (200ms/10s), GPU texture occupancy (256 MiB), ring-buffer occupancy (75% for 30 frames), decoder lifetime (24h). Budget-pressure contraction to 1 slot at degradation Level 2+. Worker state machine: SPAWNING → RUNNING → DRAINING → TERMINATED; FAILED is terminal. Pool manager runs on compositor thread at Stage 3. Watchdog runs as a shared tokio task on the network tokio runtime. | Pool manager (compositor thread, Stage 3 / spawn-request handling); `SessionCoordinator` tokio task (one per active worker); `DecodedFrameReady` ring buffer (4 slots per stream, drop-oldest) |
| `audio-routing` (E22) | Media plane (in-process) | cpal-based runtime-owned audio output. Decoupled from video decode pipelines. Default output device is operator-selected at first run, sticky per platform, changeable via config. Receives decoded Opus PCM from the GStreamer pipeline via a lock-free ring buffer; cpal data callback drains the ring buffer into hardware output. WASAPI (Windows), CoreAudio (macOS), ALSA / PipeWire (Linux). Sample-rate negotiation is caller responsibility; resamples to 48 kHz if device native rate differs. | `AudioRoutingSubsystem` (tokio task); cpal `Stream` (dedicated audio thread, non-Tokio); ring buffer producer (GStreamer PCM output) / consumer (cpal callback) |

**Cross-references:**
- Worker pool contract: `about/legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md` (RFC 0002 Amendment A1, issue hud-ora8.1.9)
- Audio-routing crate selection rationale: `docs/audits/cpal-audio-io-crate-audit.md` (issue hud-ora8.1.19)
- E24 in-process posture verdict: `docs/decisions/e24-in-process-worker-posture.md`
- Capability gate: `about/legends-and-lore/rfcs/reviews/0008-amendment-c13-capability-dialog.md`
- Text stream portal pilot flow: `about/lay-and-land/data-flow.md` §7
- Media-plane data-flow diagram: `about/lay-and-land/data-flow.md` §9

### App binary (1)

| Crate | Responsibility | Key entry points |
|---|---|---|
| `tze_hud_app` | Production entrypoint: CLI arg/env-var parsing, config file resolution, strict/dev security modes, `WindowedRuntime::new(config).run()`. Emits binary `tze_hud`. | `main()`, `parse_options()`, `StartupOptions`, `StartupSecurityMode` |

## Dependency Graph

```
Layer 0 — Scene Model (no external crate deps)
    tze_hud_scene

Layer 1 — Leaf subsystems (depend only on scene)
    tze_hud_telemetry        → scene
    tze_hud_a11y             → scene
    tze_hud_policy           → scene

Layer 2 — Resource and widget systems
    tze_hud_resource         → (standalone; image, ttf-parser, dashmap, blake3)
    tze_hud_widget           → scene, resource
    tze_hud_config           → scene, widget

Layer 3 — Protocol adapters
    tze_hud_input            → scene, telemetry
    tze_hud_mcp              → scene
    tze_hud_compositor       → scene, telemetry
    tze_hud_protocol         → scene, widget, telemetry, resource

Layer 4 — Validation (testing only)
    tze_hud_validation       → (standalone; image)

Layer 5 — Runtime orchestration (hub crate)
    tze_hud_runtime          → scene, compositor, protocol, input, telemetry,
                               config, widget, mcp, resource

Layer 6 — App shell
    tze_hud_app              → runtime, config

Layer M — Media plane subsystems (capability-gated; in-process; activate only when
           media-ingress capability is granted)
    media-worker-pool (E24)  → tze_hud_runtime (frame pipeline Stage 3),
                               GStreamer pipeline thread pool (black box),
                               DecodedFrameReady ring buffer → compositor
    audio-routing (E22)      → GStreamer PCM output (ring buffer producer),
                               cpal stream (audio thread, non-Tokio)
```

### Dependency detail for `tze_hud_runtime` (the hub)

```
tze_hud_runtime
├── tze_hud_scene
├── tze_hud_compositor
├── tze_hud_protocol
├── tze_hud_input
├── tze_hud_telemetry
├── tze_hud_config
├── tze_hud_widget
├── tze_hud_mcp [features = ["http"]]
├── tze_hud_resource
├── tokio, tonic, wgpu, winit, arc-swap, uuid
└── [platform] libc (Linux), windows (Windows)
```

## Boundary Contracts

### MCP ingress boundary

- **Entry**: `tze_hud_mcp::server::McpServer` (JSON-RPC 2.0)
- **HTTP listener**: `tze_hud_runtime::mcp::start_mcp_http_server` (feature-gated `http`)
- **What crosses in**: JSON-RPC tool calls (`create_tab`, `create_tile`, `set_content`, `dismiss`, `publish_to_zone`, `list_zones`, `list_scene`, `register_widget_asset`)
- **What crosses out**: `McpResponse` with `result` or `error` (structured error codes: `WIDGET_ASSET_*`, `CAPABILITY_MISSING`, etc.)
- **What does NOT cross**: Raw scene graph references, GPU handles, binary payloads. All content is JSON-serialised.
- **Capability gate**: `CallerContext` carries agent namespace and lease; every tool call checks required capabilities before execution.
- **Port**: default 9090 (`--mcp-port`, `TZE_HUD_MCP_PORT`)

### gRPC session boundary

- **Entry**: `tze_hud_protocol::session_server` (tonic)
- **Wire format**: Protobuf over HTTP/2, bidirectional streaming (`HudSession`)
- **What crosses in**: `SessionInit`, `MutationBatch`, `SubscriptionRequest`, `ResourceUploadStart/Chunk/Complete`, `WidgetAssetRegister`, `ZonePublish`
- **What crosses out**: `SessionEstablished`, `MutationResult`, `SceneEvent`, `TelemetryFrame`, `WidgetAssetRegisterResult`
- **Auth**: PSK-based (`tze_hud_protocol::auth`, `tze_hud_protocol::token`). Constant-time comparison (`subtle`).
- **Capability enforcement**: per-session capability set negotiated at `SessionInit`; checked on every mutation.
- **Port**: default 50051 (`--grpc-port`, `TZE_HUD_GRPC_PORT`)
- **Proto packages**: `tze_hud.protocol.v1` (types, events), `tze_hud.protocol.v1.session` (session stream)

### Runtime core boundary

- **Entry**: `tze_hud_runtime::{WindowedRuntime, HeadlessRuntime}`
- **Channel topology** (`channels.rs`): bounded channels only, no dynamic spawning. Key channels:
  - `InputEvent` — ring buffer, `INPUT_EVENT_CAPACITY`
  - `SceneEventTransactional` — backpressure channel, `SCENE_EVENT_TRANSACTIONAL_CAPACITY`
  - `SceneEventStateStream` — coalesce-key channel, `SCENE_EVENT_STATE_STREAM_CAPACITY`
  - `SceneEventEphemeral` — ring buffer, `SCENE_EVENT_EPHEMERAL_CAPACITY`
  - `SceneLocalPatch` — ring buffer, `SCENE_LOCAL_PATCH_CAPACITY`
  - `FrameReadySignal` — oneshot-style notify between compositor and main thread
  - `TelemetryRecord` — ring buffer, `TELEMETRY_RECORD_CAPACITY`
- **Thread model**: 4 fixed groups (main, compositor, network/tokio, telemetry)
- **What does NOT cross**: wgpu Device/Queue (exclusively owned by compositor thread), winit EventLoop (exclusively owned by main thread)

### Compositor boundary

- **Entry**: `tze_hud_compositor::renderer::Compositor`
- **What crosses in**: `SceneGraph` snapshot (read), `ChromeDrawCmd[]`, `SceneLocalPatch`
- **What crosses out**: `CompositorFrame` (rendered to surface), `FrameTelemetry` (timing metrics)
- **GPU surface**: `WindowSurface` (windowed) or `HeadlessSurface` (CI/test)
- **Adapter selection**: `select_gpu_adapter()` enforces platform backends (Vulkan on Linux, D3D12+Vulkan on Windows, Metal on macOS). Falls back to `Backends::all()` for headless.
- **What does NOT cross**: Scene mutations (compositor reads committed state, never writes)

### Resource store boundary

- **Entry**: `tze_hud_resource::upload::ResourceStore`
- **What crosses in**: Raw resource bytes (textures, fonts, SVG), `UploadStartRequest` with BLAKE3 `expected_hash`
- **What crosses out**: `ResourceId` (32-byte BLAKE3 digest), `ResourceStored` confirmation, `BudgetViolation` errors
- **Dedup**: content-addressed by BLAKE3 hash; `DedupIndex` returns existing resource if hash matches
- **Budget accounting**: `BudgetRegistry` charges decoded bytes per agent; double-counted across agents for shared resources
- **GC**: `GcRunner` evicts resources after configurable grace period (default 60 s), 5 ms per-cycle budget
- **Durability**: v1 ephemeral (in-memory only; lost on restart). `RuntimeWidgetStore` has separate on-disk durable store for widget SVG assets.

## Examples and Test Binaries

### Examples (4)

| Binary | Description |
|---|---|
| `vertical_slice` | v1 canonical conformance reference. Demonstrates the complete agent lifecycle: session init, capability negotiation, lease, tab/tile creation, zone publishing, input loop, telemetry, safe mode, graceful shutdown. Runs headless or windowed; supports production config and dev mode. |
| `dashboard_tile_agent` | Raw tile API exemplar. Proves the gRPC session protocol composes correctly for a polished interactive dashboard tile. Covers session establishment, lease, resource upload, atomic tile creation batch, periodic content updates, and input callbacks (Refresh/Dismiss buttons). |
| `benchmark` | Layer-3 performance validation. Runs hardware-normalised calibration (CPU scene mutations, GPU fill/composition, texture upload throughput), validates timing against normalised budgets, emits structured JSON (`--emit telemetry.json`). |
| `render_artifacts` | Layer-4 developer visibility artifact generator. Produces per-run HTML gallery (`index.html`), machine-readable manifest (`manifest.json`), per-scene directories, and LLM-readable summary. |

### Integration test suite (1 crate, 10 test binaries)

| Test binary | Coverage |
|---|---|
| `multi_agent` | Multi-agent session coexistence |
| `presence_card_tile` | Avatar upload, tile geometry, 3-node flat tree, batch submission |
| `presence_card_coexistence` | 30-second content update loop, human-friendly time formatting, 3-agent concurrent operation |
| `disconnect_orphan` | Disconnect detection, orphan handling, reconnect-within-grace (AC1-AC6), deterministic `TestClock` |
| `dashboard_tile_creation` | Atomic tile creation batch with 48x48 PNG resource upload, partial failure atomicity |
| `dashboard_tile_input` | HitRegionNode input capture, local feedback, focus cycling (Tab/Shift+Tab), keyboard activation |
| `dashboard_tile_lifecycle` | End-to-end connect-lease-upload-create-update-Refresh-Dismiss lifecycle, disconnect-during-lifecycle |
| `trace_regression` | Record/replay trace determinism |
| `soak` | Soak and leak test suite (10 s default, configurable to 6 h via `TZE_HUD_SOAK_SECS`) |
| `v1_thesis` | Capstone test: aggregates Layers 0-4, multi-agent, zone publish, headless render, performance budgets into structured v1 proof report |
| `subtitle_streaming` | Subtitle streaming breakpoint reveal, MCP/gRPC parity, zone metadata |

## Cross-references

- **Protocol plane definitions**: `about/heart-and-soul/architecture.md` (MCP / gRPC / WebRTC three-plane model)
- **Wire contracts**: `about/legends-and-lore/rfcs/` (13 RFCs):
  - RFC 0001 (scene), 0002 (runtime kernel), 0003 (timing), 0004 (input), 0005 (session protocol), 0006 (configuration), 0007 (system shell), 0008 (lease governance), 0009 (policy arbitration), 0010 (scene events), 0011 (resource store), 0013 (text stream portals)
  - RFC 0002 Amendment A1 (media worker lifecycle): `about/legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md`
- **Capability specs**: `openspec/specs/` (exemplar-notification, media-webrtc specs)
- **v1 scope**: `about/heart-and-soul/v1.md`
- **Validation framework**: `about/heart-and-soul/validation.md`
- **Runtime widget asset topology**: `about/lay-and-land/runtime-widget-asset-topology.md`
- **Operator checklists**: `about/lay-and-land/operations/`
- **Text stream portal pilot flow**: `about/lay-and-land/data-flow.md` §7
- **Media plane data-flow**: `about/lay-and-land/data-flow.md` §9
- **E24 in-process posture verdict**: `docs/decisions/e24-in-process-worker-posture.md`
- **Audio-routing crate audit**: `docs/audits/cpal-audio-io-crate-audit.md`
- **GStreamer pipeline audit**: `docs/audits/gstreamer-media-pipeline-audit.md`
