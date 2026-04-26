# Data Flow Map

How data moves through the three protocol planes and the 8-stage frame pipeline.

Cross-references: `heart-and-soul/architecture.md` (protocol planes);
`legends-and-lore/rfcs/` (wire contracts); `lay-and-land/runtime-widget-asset-topology.md`
(widget asset durable store path).

---

## 1. MCP Ingress Flow

Stateless JSON-RPC 2.0 over HTTP. Each call authenticated independently (PSK).

```
Agent               tze_hud_runtime::mcp           tze_hud_mcp                tze_hud_scene
  |                       |                              |                          |
  |-- HTTP POST / ------->| run_accept_loop()            |                          |
  |   (JSON-RPC body)     |----> McpServer::dispatch() ->|                          |
  |                        |      (1) Parse JSON-RPC      |                          |
  |                        |      (2) PSK auth (bearer    |                          |
  |                        |          OR _auth param,     |                          |
  |                        |          constant-time)      |                          |
  |                        |      (3) classify_tool()     |                          |
  |                        |          Guest/Resident gate  |                          |
  |                        |      (4) invoke_tool() ----->|-- scene.lock() -------->|
  |                        |                              |   tools::handle_*()     |
  |                        |                              |   (mutate SceneGraph)   |
  |<-- HTTP 200 resp ------|<----- response body ---------|<-- result --------------|
```

**Key files:** `tze_hud_runtime/src/mcp.rs` (lifecycle), `tze_hud_mcp/src/server.rs`
(`dispatch`, `classify_tool`), `tze_hud_mcp/src/tools.rs` (all `handle_*` functions).

**Scene coherence:** MCP and gRPC share one `Arc<Mutex<SceneGraph>>`.

---

## 2. gRPC Session Flow

Protobuf over HTTP/2, bidirectional streaming. Stateful sessions with heartbeats,
leases, dedup, freeze queues, and reconnection.

```
Agent SDK            tze_hud_protocol::session_server         session::SessionRegistry
  |                        |                                        |
  |-- bidi stream -------->| HudSessionImpl::session()              |
  |-- SessionInit -------->| authenticate_session_init()            |
  |   {agent, key, caps}   | negotiate_version()                    |
  |                        |-- SessionRegistry::authenticate() ---->|
  |                        |   (AgentSession: session_id,           |
  |                        |    namespace, capabilities)            |
  |<- SessionEstablished --|   {session_id, resume_token,           |
  |                        |    heartbeat_interval_ms}              |
  |                        |                                        |
  |== Active loop =========|                                        |
  |-- ClientMessage ------>| MutationBatch -> handle_mutation_batch()|
  |   (MutationBatch,      |   (1) safe mode check                 |
  |    Heartbeat,          |   (2) freeze queue check               |
  |    LeaseRequest,       |   (3) dedup window (batch_id)          |
  |    SubscriptionChange) |   (4) sequence validation              |
  |                        |   (5) convert proto -> SceneMutation   |
  |                        |   (6) scene.apply_batch()              |
  |<- ServerMessage -------|   -> MutationResult / RuntimeError     |
```

**Lifecycle:** `Connecting -> Handshaking -> Active -> Disconnecting -> Closed -> Resuming`

**Key files:** `session_server.rs` (`HudSessionImpl`, `handle_mutation_batch`),
`session.rs` (`SharedState`, `AgentSession`), `auth.rs`, `subscriptions.rs`, `token.rs`.

---

## 3. Zone Publish Flow

`publish_to_zone` from MCP ingress through scene mutation to compositor rendering.

```
MCP caller         tze_hud_mcp::tools            tze_hud_scene              tze_hud_compositor
  |                      |                            |                           |
  |-- publish_to_zone -->| handle_publish_to_zone()   |                           |
  |   {zone_name,        |  (1) validate zone exists  |                           |
  |    content,          |  (2) parse_zone_content()  |                           |
  |    namespace}        |      (string -> StreamText, |                           |
  |                      |       or typed JSON ->      |                           |
  |                      |       Notification/StatusBar)|                          |
  |                      |  (3) grant lease (PublishZone)|                         |
  |                      |  (4) publish_to_zone_with_lease() -------------------->|
  |                      |      (contention policy:    |  stores ZonePublishRecord |
  |                      |       LatestWins/Stack/     |  in active_publishes      |
  |                      |       MergeByKey)           |                           |
  |<-- result -----------|                            |                           |
  |                      |                            |  --- next frame ---        |
  |                      |                            |  render_zone_content()     |
  |                      |                            |  -> rasterize -> GPU      |
```

**gRPC equivalent:** `MutationBatch` with `PublishToZoneMutation` goes through
`handle_mutation_batch()` -> `SceneGraph::apply_batch()` -> same `publish_to_zone_with_lease()`.

**Key files:** `tools.rs` (`handle_publish_to_zone`), `graph.rs` (`publish_to_zone_with_lease`),
`renderer.rs` (`render_zone_content`).

---

## 4. Widget Asset Flow

Two ingress paths converge at the compositor `WidgetRenderer`.

### 4a. Startup: bundle scanning

Config `[widget_bundles].paths` -> `widget_startup::init_widget_registry()` ->
`scan_bundle_dirs()` -> register `WidgetDefinition` + `WidgetInstance` in scene ->
enqueue SVGs in `scene.pending_widget_svgs` -> `process_pending_widget_svgs()` ->
`WidgetRenderer::register_svg()`.

### 4b. Runtime: MCP / gRPC registration

```
Agent              tze_hud_mcp::tools            WidgetAssetRegistry     tze_hud_scene
  |                      |                            |                      |
  |-- register_widget_asset ->                        |                      |
  |   {widget_type_id,   | handle_register_widget_asset()                   |
  |    svg_filename,     |  (1) BLAKE3 hash verify    |                      |
  |    content_hash,     |  (2) preflight dedup ----->| (hash match? skip)   |
  |    payload}          |  (3) store asset ---------->|                      |
  |                      |  (4) enqueue pending_widget_svgs ---------------->|
  |<-- accepted ---------|                            |                      |
```

gRPC path: `WidgetAssetRegister` in `session_server.rs` -> BLAKE3 verify ->
budget check (`WidgetAssetStore`) -> optional durable write (`RuntimeWidgetStore`) ->
enqueue `pending_widget_svgs`. See `runtime-widget-asset-topology.md` for full
durable store topology.

**Key files:** `widget_startup.rs`, `widget_runtime_registration.rs`, `tools.rs`
(`handle_register_widget_asset`), `session_server.rs`, `widget.rs` (`WidgetRenderer`).

---

## 5. Event Flow (Runtime to Agents)

Events flow from runtime/compositor to subscribed agents through a 4-stage pipeline.

```
Runtime                  tze_hud_runtime::event_bus          session::SessionRegistry
  |                            |                                   |
  |-- ClassifiedEvent -------->| (1) InterruptionClass             |
  |   {event_type, class,      |     (Critical/High/Normal/Low)   |
  |    source_namespace,       | (2) Blocked audit filter          |
  |    entity_id, payload}     |     (safe_mode/freeze events      |
  |                            |      never reach agents)          |
  |                            | (3) Rate limiter (1000 evt/s)     |
  |                            | (4) Subscription + self-suppression|
  |                            | (5) Coalesce under backpressure   |
  |                            |-- dispatch_to_namespace() ------->|
  |                            |   or broadcast()                  |
  |                            |      -> AgentSession.event_tx     |
  |                            |         (mpsc cap 256)            |
  |                            |      -> ServerMessage/EventBatch  |
  |                            |      -> gRPC stream -> Agent      |
```

**Subscription categories** (mandatory: `DEGRADATION_NOTICES`, `LEASE_CHANGES`;
optional: `SCENE_TOPOLOGY`, `INPUT_EVENTS`, `FOCUS_EVENTS`, `ZONE_EVENTS`,
`TELEMETRY_FRAMES`, `ATTENTION_EVENTS`, `AGENT_EVENTS`).

**Key files:** `event_bus.rs` + submodules (`coalesce`, `suppression`),
`subscriptions.rs` (both `runtime` and `protocol` crates), `session.rs`.

---

## 6. Input Flow

OS input -> local feedback (instant, no agent roundtrip) -> agent dispatch.

```
OS / winit           tze_hud_runtime::windowed      tze_hud_input              Compositor
  |                        |                            |                          |
  |-- WindowEvent -------->| Convert to RawPointerEvent |                          |
  |   (CursorMoved,       | or KeyboardEvent           |                          |
  |    MouseInput, etc.)   |                            |                          |
  |                        | Stage 1: Input Drain (<500us)                         |
  |                        |  attach timestamps, device_id, modifiers              |
  |                        |                            |                          |
  |                        |-- DispatchProcessor ------>|                          |
  |                        |                            |                          |
  |                        | Stage 2: Local Feedback (<500us)                      |
  |                        |  (a) hit_test() via ArcSwap snapshot (lock-free)      |
  |                        |  (b) update HitRegionLocalState (hover/press/focus)   |
  |                        |  (c) produce SceneLocalPatch ----------------------->|
  |                        |      (compositor applies hover tint 0.1 white,       |
  |                        |       press darken 0.85x, 2px focus ring)            |
  |                        |  (d) build typed events (Enter/Leave/Down/Up/Click)  |
  |                        |      -> per-agent EventBatch -> gRPC stream          |
```

Rollback: only on explicit agent rejection (100ms reverse). Silence/latency
does not trigger rollback.

**Key files:** `windowed.rs`, `dispatch.rs` (`DispatchProcessor`), `hit_test.rs`,
`local_feedback.rs`, `events.rs`, `event_queue.rs`, `coalescing.rs`, `batching.rs`,
`pipeline.rs` (`HitTestSnapshot`), `channels.rs`.

---

## 7. Text Stream Portal Pilot Flow

Phase-0 text stream portals are resident raw-tile compositions over the primary
`HudSession` stream. They are not a new runtime transport, not a terminal
emulator, and not a chrome-layer shell feature.

```
Operator / Windows input
  │
  │ pointer, wheel, keyboard, clipboard
  ▼
tze_hud_runtime::windowed
  │
  │ (1) pointer hit-test + focus transition through tze_hud_input
  │ (2) Windows pointer capture/release for header drag continuity
  │ (3) keyboard dispatch to the focused composer tile
  │ (4) Ctrl+V clipboard read -> RawCharacterEvent
  ▼
tze_hud_protocol::session_server
  │
  │ input/focus/scroll events over the session event channel
  │ existing MutationBatch responses share this channel, so short
  │ typing bursts need channel headroom
  ▼
resident portal adapter / user-test script
  │
  │ owns portal state:
  │ - output transcript viewport and scroll offset
  │ - composer text buffer, cursor, placeholder, caret blink
  │ - header drag position
  │ - submit/clear behavior
  │
  │ sends scene updates as existing MutationBatch messages:
  │ - frame tile root + decorative nodes
  │ - transparent capture tiles for header, composer, transcript
  │ - TextMarkdownNode updates for transcript/composer
  │ - SolidColorNode caret updates
  ▼
tze_hud_scene -> tze_hud_compositor
  │
  │ render existing node types only:
  │ SolidColor, TextMarkdown, HitRegion
  ▼
HUD overlay
```

Important current seams:

- Composer text uses `TextMarkdownNode.color_runs` as a literal full-span
  marker so markdown characters remain visible and the protocol conversion can
  select monospace rendering until `font_family` exists in the proto.
- Normal printable input should be sourced from runtime character events.
  The current Windows path does not emit a separate character event for Space,
  so the portal adapter synthesizes only Space from key-down fallback.
- Portal drag is content-layer tile movement, not OS window movement. Mouse
  wheel should affect the transcript/composer capture tile under the cursor or
  do nothing; it should not move the overlay window.
- The scroll contract is a subset of the portal flow: the OUTPUT transcript
  pane owns the scrollable viewport, and the old standalone scroll exemplar is
  covered by the portal's `scroll` phase.

**Key files:** `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`,
`.claude/skills/user-test/scripts/hud_grpc_client.py`,
`crates/tze_hud_runtime/src/windowed.rs`,
`crates/tze_hud_protocol/src/session_server.rs`,
`crates/tze_hud_protocol/src/convert.rs`.

**Cross-references:** `about/legends-and-lore/rfcs/0013-text-stream-portals.md`,
`openspec/specs/text-stream-portals/spec.md`,
`docs/text-stream-refinement.md`.

---

## 8. Frame Pipeline Summary

8-stage pipeline (RFC 0002 section 3.2), four thread groups, strict budgets:

```
Stage  Name               Thread      p99 Budget   Channel
-----  -----------------  ----------  ----------   -----------------
  1    Input Drain        Main        < 500us      Ring buffer
  2    Local Feedback     Main        < 500us      ArcSwap snapshot
  3    Mutation Intake    Compositor  < 1ms        Backpressure
  4    Scene Commit       Compositor  < 1ms        (internal)
  5    Layout Resolve     Compositor  < 1ms        (internal)
  6    Render Encode      Compositor  < 4ms        (internal)
  7    GPU Submit+Present Comp+Main   < 8ms        FrameReadySignal
  8    Telemetry Emit     Telemetry   < 200us      Ring buffer
```

Total 1-7: < 16.6ms. Input-to-local-ack: < 4ms. Input-to-present: < 33ms.

Stage 3 integrates `BudgetEnforcer`: admission gate, delta accounting,
enforcement ladder (Normal -> Warning -> Throttled -> Revoked), post-revocation cleanup.

**Key files:** `pipeline.rs`, `channels.rs`, `budget.rs`, `threads.rs`, `renderer.rs`.

---

---

## 9. Media Plane Data Flow

Capability-gated (`media-ingress`). Activates only when an agent holds a valid
`media-ingress` capability grant (RFC 0008 Amendment A1, RFC 0009 Amendment A1).
All layers below operate on the **trusted** side of the gRPC/MCP wire boundary.
Cross-agent isolation is enforced by `session_id` tagging on `DecodedFrameReady`
messages — the compositor thread refuses to blit a frame tagged with session A's
`session_id` into session B's tile.

### 9a. Video ingress: WebRTC → GStreamer → compositor surface

```
                    ┌─────────────────────────────────────────┐
                    │     TRUST BOUNDARY: gRPC/MCP wire        │
                    │   Agent lives outside this boundary.     │
                    │   Everything below is compositor-internal.│
                    └──────────────────┬──────────────────────┘
                                       │
Agent (remote)                         │ MediaIngressOpen (gRPC, RFC 0005 A1)
  │                                    │ → activation gate check:
  │── MediaIngressOpen ───────────────>│   (1) capability: media-ingress
  │   {stream_url, session_id, ...}    │   (2) budget headroom (pool slot,
  │                                    │       per-session stream cap,
  │                                    │       texture memory headroom)
  │                                    │   (3) role authority (owner/admin)
  │                                    │
  │                     ┌──────────────┴──────────────────────────────┐
  │                     │  Pool Manager (compositor thread, Stage 3)   │
  │                     │  Claims pool slot; spawns SessionCoordinator │
  │                     └──────────────┬──────────────────────────────┘
  │                                    │
  │                     ┌──────────────▼──────────────────────────────┐
  │                     │  SessionCoordinator (tokio task,             │
  │                     │  network tokio runtime)                      │
  │                     │                                              │
  │                     │  WebRTC source (webrtc-rs)                   │
  │                     │    └─ RTP packet stream                      │
  │                     │         │                                    │
  │                     │         ▼  tokio bridge                      │
  │                     │    GStreamer appsrc element                  │
  │                     │    (gstreamer-app::AppSrc)                   │
  │                     │         │ push_buffer()                      │
  │                     │         ▼                                    │
  │                     │    GStreamer decode pipeline                  │
  │                     │    (GStreamer-managed thread pool;            │
  │                     │     NOT compositor-controlled)               │
  │                     │                                              │
  │                     │    H.264 or VP9 decode element               │
  │                     │    (hardware: va/nvcodec/d3d11;              │
  │                     │     fallback: avdec_h264/vp9dec)             │
  │                     │         │ decoded YUV/RGB frames             │
  │                     │         ▼                                    │
  │                     │    GStreamer appsink element                 │
  │                     │    (gstreamer-app::AppSink)                  │
  │                     │         │ new_sample callback                │
  │                     │         ▼                                    │
  │                     │    DecodedFrameReady ring buffer             │
  │                     │    (4 slots per stream, drop-oldest,        │
  │                     │     tagged with session_id)                  │
  │                     └──────────────┬──────────────────────────────┘
  │                                    │
  │                     ┌──────────────▼──────────────────────────────┐
  │                     │  Watchdog (tokio task, shared across pool)   │
  │                     │  Polls per-worker thresholds every ~1s:      │
  │                     │    - CPU time: 200ms / 10s window            │
  │                     │    - GPU texture occupancy: 256 MiB          │
  │                     │    - Ring-buffer occupancy: ≥75% for 30 fr. │
  │                     │    - Decoder lifetime: 24h                   │
  │                     │  Threshold crossed → DRAINING transition     │
  │                     └──────────────┬──────────────────────────────┘
  │                                    │
  │                     ┌──────────────▼──────────────────────────────┐
  │                     │  Compositor Thread (Stage 3 + Stage 6)       │
  │                     │                                              │
  │                     │  Stage 3: Drains DecodedFrameReady;          │
  │                     │    validates session_id tag (cross-agent     │
  │                     │    isolation enforcement)                    │
  │                     │    Uploads CPU buffer to GPU texture via     │
  │                     │    device.create_texture + queue.write_texture│
  │                     │    (sole wgpu Device/Queue owner — §2.8)     │
  │                     │                                              │
  │                     │  Stage 6: Blits GPU texture into tile        │
  │                     │    compositing region; renders to surface    │
  │                     └─────────────────────────────────────────────┘
```

### 9b. Audio ingress: GStreamer Opus decode → cpal output

```
  SessionCoordinator (tokio task)
  │
  │  GStreamer audio decode pipeline
  │  (Opus RTP → rtpopusdepay → opusdec → audioconvert → audioresample)
  │        │ decoded PCM (48 kHz, stereo, F32 or I16)
  │        ▼
  │  Lock-free ring buffer (ringbuf)
  │  Producer side: tokio task writes PCM frames
  │        │
  │        │ ← Audio-Routing Subsystem (E22) ─────────────────────┐
  │        │                                                        │
  │        ▼                                                        │
  │  cpal data callback (dedicated non-Tokio audio thread)         │
  │  (real-time priority via rtkit / platform equivalent)          │
  │    - Drains ring buffer into cpal output buffer                │
  │    - Underrun: fills with silence (never blocks callback)      │
  │    - Format conversion (F32 ↔ I16) if device native ≠ F32     │
  │        │                                                        │
  │        ▼                                                        │
  │  cpal Stream → hardware output                                  │
  │  (WASAPI on Windows, CoreAudio on macOS,                       │
  │   ALSA/PipeWire on Linux)                                      │
  │                                                                 │
  │  Operator-selected sticky device (stored in config):           │
  │    - Device ID persisted via cpal stable device IDs (v0.17.0+) │
  │    - On Windows: IMMNotificationClient watches for             │
  │      OnDefaultDeviceChanged; stream rebuilt on device switch   │
  └────────────────────────────────────────────────────────────────┘
```

### 9c. Trust boundaries and isolation summary

| Boundary | Where | Enforcement |
|---|---|---|
| Agent ↔ runtime | gRPC/MCP wire | PSK auth, capability check, lease gate |
| Cross-agent media isolation | `DecodedFrameReady` message | `session_id` tag; compositor refuses cross-agent blits |
| GPU device ownership | Compositor thread | Only compositor thread holds wgpu `Device`/`Queue`; no media worker may call GPU APIs directly (RFC 0002 §2.8) |
| GStreamer thread pool | Black box managed by GStreamer | Session coordinator interacts only via `AppSrc`/`AppSink`/bus APIs; pipeline internals are not compositor-visible |
| cpal audio thread | Non-Tokio, real-time priority | Ring buffer decouples Tokio PCM producer from real-time callback; callback never blocks |

**Key files (forthcoming, owned by RFC 0014 and downstream implementation beads):**
`tze_hud_runtime/src/media/` (pool manager, session coordinator, watchdog),
`tze_hud_runtime/src/audio/` (audio-routing subsystem, cpal integration).

**Cross-references:**
- Worker pool lifecycle contract: `legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md`
- E24 in-process posture: `docs/decisions/e24-in-process-worker-posture.md`
- Audio-routing crate selection: `docs/audits/cpal-audio-io-crate-audit.md`
- GStreamer pipeline details: `docs/audits/gstreamer-media-pipeline-audit.md`
- Media-plane component entries: `lay-and-land/components.md` §"Media plane subsystems"
- Capability gate: `legends-and-lore/rfcs/reviews/0008-amendment-c13-capability-dialog.md`

---

## Cross-references

| Topic | Document |
|-------|----------|
| Protocol planes | `heart-and-soul/architecture.md` |
| Scene graph contract | `legends-and-lore/rfcs/0001-scene-contract.md` |
| Runtime kernel | `legends-and-lore/rfcs/0002-runtime-kernel.md` |
| Timing | `legends-and-lore/rfcs/0003-timing.md` |
| Input pipeline | `legends-and-lore/rfcs/0004-input.md` |
| Session protocol | `legends-and-lore/rfcs/0005-session-protocol.md` |
| System shell | `legends-and-lore/rfcs/0007-system-shell.md` |
| Lease governance | `legends-and-lore/rfcs/0008-lease-governance.md` |
| Scene events | `legends-and-lore/rfcs/0010-scene-events.md` |
| Resource store | `legends-and-lore/rfcs/0011-resource-store.md` |
| Widget asset topology | `lay-and-land/runtime-widget-asset-topology.md` |
| Security doctrine | `heart-and-soul/security.md` |
| Text stream portal pilot flow | §7 (this document) |
| Media plane data flow | §9 (this document) |
| Media worker lifecycle | `legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md` |
| Audio-routing crate audit | `docs/audits/cpal-audio-io-crate-audit.md` |
| GStreamer pipeline audit | `docs/audits/gstreamer-media-pipeline-audit.md` |
