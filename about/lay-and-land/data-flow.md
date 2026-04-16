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

## 7. Frame Pipeline Summary

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
