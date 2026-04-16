> **HISTORICAL DOCUMENT** — This is the Gen-2 reconciliation snapshot (2026-03-22). It has been superseded by Gen-3. For the current baseline, see [docs/RECONCILIATION_STATUS.md](RECONCILIATION_STATUS.md).

# Gen-2 Reconciliation: Spec-to-Code Coverage

**Issue:** rig-j1v
**Date:** 2026-03-22
**Scope:** Verification of all 10 gen-1 gap beads + updated coverage matrix
**Branch:** agent/rig-j1v
**Depends on:** rig-itf, rig-zo8, rig-s31, rig-hsp, rig-nfr, rig-j8e, rig-6t7, rig-ict, rig-0fi, rig-3l8

---

## Executive Summary

All 10 gen-1 gap beads have been implemented and merged. The codebase has
advanced significantly: 8 of the 10 gaps are **fully closed**, 2 are
**partially closed** with residual coverage holes that fall within the gen-3
hard limit.

No critical architectural gaps remain. The v1 thesis (" An LLM with only MCP
access can publish a subtitle to a zone with one tool call") is now provable
from the code. The scene graph, timing model, budget enforcement, gRPC event
subscription, and accessibility stub are all present.

Two areas of partial closure are tracked below. They are non-blocking for the
v1 implementation cycle but should inform the next reconciliation.

**`cargo check --workspace` passes with zero errors.** One unused-method
warning exists (`budget_state_entered_instant`) — not a gap.

---

## 1. Gap Closure Verification

### rig-itf: StaticImageNode — CLOSED

**Gen-1 finding:** `StaticImageNode` was defined in RFC 0001 §2.4 but absent
from the `NodeData` enum in `types.rs` and from the compositor renderer.

**Verification:**

- `crates/tze_hud_scene/src/types.rs`: `NodeData::StaticImage(StaticImageNode)`
  is present in the enum. `StaticImageNode` is fully defined with `image_data`,
  `width`, `height`, `content_hash`, `fit_mode`, and `bounds` fields. The
  `ImageFitMode` enum (`Contain`, `Cover`, `Fill`, `ScaleDown`) matches RFC
  0001 §2.4 exactly.
- `crates/tze_hud_compositor/src/renderer.rs`: `NodeData::StaticImage` is
  handled at line 239 (size calculation) and line 332 (rendering). A dedicated
  test at line 414 (`test_static_image_node_renders_placeholder_quad`) asserts
  that warm-gray placeholder pixels are produced on headless GPU. A second test
  (`test_static_image_node_composited_with_other_nodes`) exercises composition
  with a SolidColor node.

**Status: CLOSED.**

---

### rig-zo8: Zone schema — CLOSED

**Gen-1 finding:** `ZoneDefinition` in `types.rs` was a name-only stub (`id`,
`name`, `description`). All policy fields, contention logic, publish
operations, and zone-to-tile mapping were absent.

**Verification:**

- `crates/tze_hud_scene/src/types.rs`: `ZoneDefinition` now carries
  `geometry_policy: GeometryPolicy`, `accepted_media_types: Vec<ZoneMediaType>`,
  `rendering_policy: RenderingPolicy`, `contention_policy: ContentionPolicy`,
  `max_publishers: u32`, `transport_constraint: Option<TransportConstraint>`,
  and `auto_clear_ms: Option<u64>` — a full match to RFC 0001 §2.5.
- `GeometryPolicy`, `ZoneMediaType`, `RenderingPolicy`, `ContentionPolicy`
  (`LatestWins`, `Stack`, `MergeByKey`, `Replace`), and `TransportConstraint`
  are all defined as enums/structs.
- `ZoneRegistry` provides `register`, `unregister`, `get_by_name`,
  `zones_accepting`, `all_zones`, `active_for_zone`, and `snapshot`. A
  `with_defaults()` constructor pre-populates `status-bar`, `notification-area`,
  and `subtitle` zones.
- `ZoneContent`, `ZonePublishRecord`, `ZonePublishToken`, `NotificationPayload`,
  `StatusBarPayload` are defined.
- `crates/tze_hud_scene/src/graph.rs`: `publish_to_zone()` method implements
  full contention-policy dispatch (LatestWins, Stack depth limit, MergeByKey
  key replace, Replace). `SceneMutation::PublishToZone` variant in
  `mutation.rs` routes through `apply_batch`.
- 8 tests in `graph.rs` cover zone-not-found, media-type mismatch, LatestWins,
  Stack depth, and MergeByKey semantics.

**Status: CLOSED.**

---

### rig-s31: MCP bridge — CLOSED

**Gen-1 finding:** No MCP bridge existed in any crate. The `tze_hud_mcp` crate
was absent entirely. RFC 0002 §2.4 and v1.md §Protocol MCP layer require:
`create_tab`, `create_tile`, `set_content`, `dismiss`, `list_scene`,
`publish_to_zone`, `list_zones`.

**Verification:**

- `crates/tze_hud_mcp/` crate exists with `lib.rs`, `server.rs`, `tools.rs`,
  `types.rs`, `error.rs`.
- `tools.rs` provides: `handle_create_tab`, `handle_create_tile`,
  `handle_set_content`, `handle_publish_to_zone`, `handle_list_zones` — all
  five primary tools from v1.md §Protocol MCP layer (note: `dismiss` and
  `list_scene` are not in the tool list, see note below).
- `McpServer` in `server.rs` routes JSON-RPC 2.0 method names to handlers.
  `McpError` models MCP error codes correctly.
- All five tools have parameterized input structs with serde deserialization,
  output structs with serde serialization, and inline tests.

**Residual note:** `dismiss` and `list_scene` from v1.md §Protocol MCP layer
are not implemented as named tools. This is a partial implementation of the
specified tool surface. The v1 MCP thesis (subtitle publish via one tool call)
is satisfied, but `dismiss` (remove a tile's content) and `list_scene` (query
current scene state) are missing. See gen-3 gap note below.

**Status: PARTIAL (5 of 7 tools). Core path closed.**

---

### rig-hsp: Budget enforcement — CLOSED

**Gen-1 finding:** `ResourceBudget` type existed but the enforcement ladder
(Normal → Warning → Throttle → Revoke), `AgentResourceState`, `BudgetState`,
and the warning/throttle/revoke escalation were not implemented.

**Verification:**

- `crates/tze_hud_runtime/src/budget.rs`: Full three-tier enforcement ladder:
  `BudgetState::{Normal, Warning{first_exceeded}, Throttled{throttled_since}, Revoked}`.
- `WARNING_GRACE = 5s`, `THROTTLE_GRACE = 30s` match RFC 0002 §5.2.
- `BudgetEnforcer::check_mutation()` checks tile count, texture memory, update
  rate (sliding window Hz), and nodes-per-tile. Returns
  `Allow | Reject(violation) | Revoke(violation)`.
- `BudgetEnforcer::tick()` advances the ladder each frame: Normal→Warning on
  violation, Warning→Throttled after 5s grace, Throttled→Revoked after 30s.
- `BudgetEnforcer::frame_guardian_shed()` sheds lowest-priority tiles when
  Stage 3–5 exceeds 3ms.
- Critical path (hard texture OOM, repeated invariant violations) bypasses
  the ladder and goes directly to revocation.
- `BudgetTelemetrySink` trait with `NoopTelemetrySink` and
  `CollectingTelemetrySink` support test verification.
- Comprehensive tests in `budget.rs`: ladder state transitions, Hz rate
  limiting, tile count enforcement, OOM critical path, frame guardian shed.
- `AgentResourceState` in `types.rs` tracks the budget envelope with
  `BudgetViolation` variants matching RFC 0002 §5.1–5.3.

**Status: CLOSED.**

---

### rig-nfr: gRPC events — CLOSED

**Gen-1 finding:** A `broadcast::Sender<SceneEvent>` existed in `SharedState`
but nothing published events to it and no gRPC streaming RPC exposed it. Input
events were only printed to stdout.

**Verification:**

- `crates/tze_hud_protocol/src/server.rs`: `subscribe_events` async fn at
  line 400 implements a proper gRPC server-streaming RPC. It creates an mpsc
  channel and stores `tx` in the session's `event_tx` field.
- `session.rs`: `Session.event_tx: Option<mpsc::Sender<SceneEvent>>` and
  `dispatch_to_namespace()` / `broadcast_to_all()` helpers.
- Event dispatch is wired in: `acquire_lease`, `renew_lease`, `revoke_lease`,
  and `apply_mutations` all call `dispatch_to_namespace` with typed events
  (`LeaseEvent`, `MutationApplied`).
- `dispatch_input_event()` at line 428 in `server.rs` routes input events
  (pointer, keyboard) from the input pipeline to the agent owning the
  hit-tested tile via the same mpsc channel.
- Two tests in `server.rs` verify that events reach the correct per-namespace
  channel and that cross-namespace isolation is maintained.

**Status: CLOSED.**

---

### rig-j8e: SyncGroups — CLOSED

**Gen-1 finding:** `Tile.sync_group` field existed but `SyncGroup` as a scene
object, `create_sync_group`/`join_sync_group`/`leave_sync_group` mutations,
and the Stage 4 deferred-commit logic were absent.

**Verification:**

- `crates/tze_hud_scene/src/types.rs`: `SyncGroup` struct with `id`, `name`,
  `owner_namespace`, `members: BTreeSet<SceneId>`, `commit_policy`, and
  `max_deferrals`. `SyncCommitPolicy::{AllOrDefer, AvailableMembers}` enum
  matches RFC 0003 §2.2 exactly.
- `crates/tze_hud_scene/src/graph.rs`:
  - `create_sync_group()` at line 332: enforces per-namespace limit
    (`MAX_SYNC_GROUPS_PER_NAMESPACE`), returns `SyncGroupCommitDecision`.
  - `join_sync_group()` at line 389: validates tile ownership, limits
    `MAX_MEMBERS_PER_SYNC_GROUP`, prevents duplicate membership.
  - `leave_sync_group()` at line 449: removes tile from group, clears
    `Tile.sync_group` field.
  - `evaluate_sync_group_commit()` at line 468: implements RFC 0003 §2.2 —
    AllOrDefer checks readiness, defers up to `max_deferrals`, then
    force-commits. AvailableMembers always commits available subset.
  - `SyncGroupCommitDecision::{Commit{tiles}, Defer, ForceCommit{tiles}}` at
    line 790.
- Tests at lines 1228, 1279: create/join/leave lifecycle, AllOrDefer deferral
  count, cross-namespace join rejection, member limit enforcement.

**Status: CLOSED.**

---

### rig-6t7: Test scenes — PARTIAL (5 of 19 named scenes)

**Gen-1 finding:** No test scene registry existed. `validation.md` requires a
named corpus of scenes shared across all five validation layers.

**Verification:**

- `crates/tze_hud_scene/src/test_scenes.rs`: `TestSceneRegistry` exists with
  5 named scenes: `empty`, `single_tile`, `two_tiles`, `max_tiles`, `zone_test`.
- `ClockMs` injection struct allows deterministic timestamp-sensitive tests.
- `SceneSpec` metadata struct carries expected counts and capability flags.
- `InvariantViolation` and `assert_layer0_invariants()` function provide the
  Layer 0 assertion suite.
- All 5 scenes have full Layer 0 invariant tests.

**Residual gap:** `validation.md` specifies 19 named scenes in the initial
corpus (e.g., `single_tile_solid`, `three_tiles_no_overlap`,
`overlapping_tiles_zorder`, `overlay_transparency`, `tab_switch`,
`lease_expiry`, `sync_group_media`, `input_highlight`, `coalesced_dashboard`,
`three_agents_contention`, various zone-specific scenes). The current
implementation has 5 scenes covering basic structure. The 14 missing scenes
include all performance-stress, multi-agent, overlay, and zone-workflow scenes.
These are not required for the vertical slice thesis but are required for full
v1 validation completeness.

**Status: PARTIAL (infrastructure present, corpus 26% complete).**

---

### rig-ict: Injectable clock — CLOSED

**Gen-1 finding:** `SceneGraph::graph.rs` called `SystemTime::UNIX_EPOCH`
directly. `tze_hud_telemetry::record` called `Instant::now()` directly. No
injection point existed.

**Verification:**

- `crates/tze_hud_scene/src/clock.rs`: `Clock` trait with `now_millis(&self) ->
  u64`. `SystemClock` (production) and `TestClock` (manually controlled, thread-
  safe via `Arc<Mutex<u64>>`) implementations. `TestClock::advance()` and
  `TestClock::set()` for deterministic control. Full unit tests.
- `crates/tze_hud_scene/src/graph.rs`: `SceneGraph` holds `clock: Arc<dyn Clock>`
  (serde-skipped, defaults to `SystemClock`). `SceneGraph::new_with_clock()` for
  test injection. All `now_millis()` calls route through `self.clock.now_millis()`.
  Tab creation, lease grant, lease renew, lease expiry — all use injected clock.
- `test_scenes.rs`: `ClockMs` value used to derive TTLs and `present_at` offsets
  without wall-clock-sensitive assertions.
- 5 unit tests in `clock.rs` cover start value, advance, set, saturating-add
  overflow, and system clock non-zero.

**Status: CLOSED.**

---

### rig-0fi: p99 assertions — CLOSED

**Gen-1 finding:** The vertical slice printed latency values but made no
assertions against RFC budgets. Budgets were unenforceable in CI.

**Verification:**

- `examples/vertical_slice/tests/budget_assertions.rs` (555 lines): Dedicated
  test file with Layer 3 and Layer 1 assertions.
- **Layer 3 — p99 budget assertions:**
  - `test_frame_time_p99_within_budget`: 20 headless frames, asserts
    `frame_time.assert_p99_under(16_600 × 10)` (10× multiplier for
    llvmpipe/SwiftShader CI; nominal 16.6ms).
  - `test_input_to_local_ack_p99_within_budget`: 30 pointer events, asserts
    `input_to_local_ack.assert_p99_under(4_000)`.
  - `test_hit_test_p99_within_budget`: 50 pointer-move events, asserts
    `hit_test_latency.assert_p99_under(100)`.
  - `test_transaction_validation_p99_within_budget`: 50 batches, asserts
    `assert_p99_under(200 × 5)` (5× CI multiplier).
  - `test_scene_diff_p99_within_budget`: 50 diffs, asserts
    `assert_p99_under(500 × 3)` (3× CI multiplier).
- **Layer 1 — pixel assertions:**
  - `test_pixel_readback_background`: Renders 800×600 headless, asserts pixel
    buffer size = `800 × 600 × 4`. Calls `HeadlessSurface::assert_pixel_color()`
    for background region.
  - `test_pixel_readback_tile_solid_color`: Solid-color tile rendered,
    asserts pixel color in tile region within ±2 per channel (±10 for CI).
  - `test_pixel_readback_zorder_stacking`: Two tiles at z-order 1 and 2,
    asserts upper tile occludes lower tile pixels.
- `crates/tze_hud_telemetry/src/record.rs`: `LatencyBucket::assert_p99_under()`
  method with structured error on failure.
- `crates/tze_hud_compositor/src/surface.rs`:
  `HeadlessSurface::assert_pixel_color()` with tolerance parameter.

**Status: CLOSED.**

---

### rig-3l8: A11y stub — CLOSED

**Gen-1 finding:** `tze_hud_a11y` crate did not exist. RFC 0004 §5 specifies
it as a separate crate with AT-SPI2, UIA/IAccessible2, and NSAccessibility
platform bridges.

**Verification:**

- `crates/tze_hud_a11y/` crate exists with:
  - `lib.rs`: `AccessibilityTree` trait (3 methods: `update_from_scene`,
    `announce`, `focus_changed`). `AccessibilityConfig` struct with `label`,
    `role_hint`, `description`, `live`, `live_politeness` — matches RFC 0004
    §5.4. `LivePoliteness::{Polite, Assertive, Off}` enum.
  - `noop.rs`: `NoopAccessibility` — default/CI implementation, no-op.
  - `atspi.rs`: Linux AT-SPI2 stub (compiled only on `target_os = "linux"`).
  - `uia.rs`: Windows UIA/IAccessible2 stub (compiled only on
    `target_os = "windows"`).
  - `nsaccessibility.rs`: macOS NSAccessibility stub (compiled only on
    `target_os = "macos"`).
  - `WarnOnce` helper: emits one-shot tracing warn on first stub operation.
- 5 unit tests: `noop_update_from_scene_does_not_panic`,
  `noop_announce_does_not_panic`, `noop_focus_changed_does_not_panic`,
  `accessibility_config_defaults`, `noop_accepts_many_updates`.

**Status: CLOSED (structural stub matches RFC 0004 §5.8; platform
implementations remain stubs as specified for v1).**

---

## 2. Updated Coverage Matrix

Status legend unchanged from gen-1:

- **FULL** — doctrine requirement is fully specified in RFC and exercised in code.
- **RFC-ONLY** — fully specified in RFC, not yet exercised (expected for later cycles).
- **PARTIAL** — RFC or code covers the concept with identified gaps.
- **ABSENT** — no RFC coverage and no code coverage.

### 2.1 Scene Model Requirements

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| S1 | Tabs: create, switch, delete | RFC 0001 §2.2, §3.1 | `tze_hud_scene::graph` + gRPC server | FULL |
| S2 | Tiles: create, resize, move, delete, z-order | RFC 0001 §2.3, §3.1 | `tze_hud_scene::graph` + gRPC server | FULL |
| S3 | Node type: solid color | RFC 0001 §2.4 | `types::SolidColorNode`, compositor renders | FULL |
| S4 | Node type: text/markdown | RFC 0001 §2.4 | `types::TextMarkdownNode`, VS exercises | FULL |
| S5 | Node type: static image | RFC 0001 §2.4 | `types::StaticImageNode`, compositor renders | FULL |
| S6 | Node type: interactive hit region | RFC 0001 §2.4 | `types::HitRegionNode`, input processor | FULL |
| S7 | Sync groups: basic membership + AllOrDefer | RFC 0003 §2 | `types::SyncGroup`, `graph::evaluate_sync_group_commit` | FULL |
| S8 | Atomic batch mutations, all-or-nothing | RFC 0001 §3, §3.2 | `mutation.rs::apply_batch` with rollback | FULL |
| S9 | Zone system: subtitle, notification, status-bar, ambient-background | RFC 0001 §2.5 | `types::ZoneDefinition` + `ZoneRegistry` + `graph::publish_to_zone` | FULL |

### 2.2 Compositor Requirements

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| C1 | wgpu headless and windowed rendering | RFC 0002 §1.3, §8 | `tze_hud_compositor::surface` + `HeadlessSurface` | FULL |
| C2 | Tile composition with z-order | RFC 0002 §3.2 Stage 6 | `compositor::renderer` (z-order ordering present) | PARTIAL |
| C3 | Alpha blending for overlays | RFC 0002 §3.2 Stage 6, §6.2 Level 3 | `renderer.rs` (alpha blend enabled) | PARTIAL |
| C4 | Background, tile borders, basic visual chrome | RFC 0002 §7, RFC 0001 §2 | VS renders background + solid tiles | PARTIAL |
| C5 | 60fps on reference hardware | RFC 0002 §3.1 (16.6ms p99 budget) | `budget_assertions.rs::test_frame_time_p99_within_budget` (asserted, headless multiplier) | FULL |

### 2.3 Protocol Requirements

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| P1 | gRPC control plane with protobuf | RFC 0002 §1.1, RFC 0001 §7 | `tze_hud_protocol::server`, tonic/prost | FULL |
| P2 | Scene mutation RPCs | RFC 0001 §3.1 | `apply_mutations` RPC | FULL |
| P3 | Lease management RPCs (request, renew, revoke) | RFC 0002 §4.1, RFC 0001 §3.1 | `acquire_lease`, `renew_lease`, `revoke_lease` RPCs | FULL |
| P4 | Event subscription stream | RFC 0002 §2.4 | `subscribe_events` gRPC streaming RPC; per-namespace mpsc dispatch | FULL |
| P5 | Telemetry stream | RFC 0002 §2.5, §3.2 Stage 8 | `tze_hud_telemetry` + VS emits JSON | FULL |
| P6 | MCP compatibility layer: create_tab, create_tile, set_content, dismiss, list_scene | RFC 0002 §1.1, §2.4 | `tze_hud_mcp` crate; `dismiss` and `list_scene` absent | PARTIAL |
| P7 | MCP zone tools: publish_to_zone, list_zones | RFC 0001 §2.5, presence.md zone API | `handle_publish_to_zone`, `handle_list_zones` in `tze_hud_mcp` | FULL |

### 2.4 Security Requirements

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| Sec1 | Agent authentication (PSK + local socket) | RFC 0002 §4.1 | `session::SessionRegistry` with PSK | FULL |
| Sec2 | Capability scopes (additive grants, revocation) | RFC 0001 §3.3, RFC 0002 §4.3 | `types::Capability` enum, `Lease.capabilities` | PARTIAL |
| Sec3 | Agent isolation (no cross-agent content access) | RFC 0001 §1.2 namespace isolation | Namespace isolation in `SceneGraph`; gRPC dispatch isolated per-session | PARTIAL |
| Sec4 | Resource budgets (enforced, throttle + revoke) | RFC 0002 §5 | `BudgetEnforcer` with full ladder; `check_mutation` + `tick` | FULL |

### 2.5 Interaction Requirements

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| I1 | Mouse/pointer input with hit testing | RFC 0004 §3, RFC 0001 §5 | `tze_hud_input::InputProcessor::process` | FULL |
| I2 | Touch input on supported platforms | RFC 0004 §3.2 | RFC specifies; no touch in code | RFC-ONLY |
| I3 | Local-first feedback (press, hover, focus) | RFC 0004 §6, RFC 0002 §3.2 Stage 2 | `InputProcessor` updates `HitRegionLocalState` | FULL |
| I4 | Input events forwarded to owning agent | RFC 0004 §6.4 | `dispatch_input_event()` + `subscribe_events` stream | FULL |
| I5 | HitRegionNode with local pressed/hovered/focused state | RFC 0001 §2.4, RFC 0004 §6.3 | `types::HitRegionLocalState` + `InputProcessor` | FULL |
| I6 | input_to_local_ack p99 < 4ms | RFC 0004 §6.2, validation.md §Layer 3 | `budget_assertions.rs::test_input_to_local_ack_p99_within_budget` | FULL |

### 2.6 Window Modes

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| W1 | Fullscreen mode: guaranteed on all platforms | RFC 0002 §7.1 | `WindowSurface` abstraction (implementation TBD) | RFC-ONLY |
| W2 | Overlay/HUD mode: transparent always-on-top | RFC 0002 §7.1, §7.2 | Specified in RFC; no window mode code yet | RFC-ONLY |
| W3 | Per-region input routing (click-through) | RFC 0002 §7.2 | RFC specifies WM_NCHITTEST/XShape/etc.; not in code | RFC-ONLY |
| W4 | Runtime configuration (not compile-time) | RFC 0002 §1.3 | `HeadlessConfig` vs `WindowSurface` trait; same binary | FULL |

### 2.7 Failure Handling

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| F1 | Agent disconnect detection with grace period | RFC 0002 §4.2 (heartbeat timeout) | `BudgetEnforcer::tick` escalation covers budget revocation; heartbeat keepalive not wired | PARTIAL |
| F2 | Lease orphaning and cleanup | RFC 0002 §5.2 (revocation tier) | `revoke_lease` RPC removes tiles; `BudgetEnforcer` escalates to revocation | PARTIAL |
| F3 | Disconnection visual indicator | RFC 0002 §3.2 Stage 6 (chrome layer) | Chrome layer not implemented in renderer | RFC-ONLY |
| F4 | Reconnection with lease reclaim | RFC 0002 §4.1 hot-connect | Hot-connect in RFC; not in code | RFC-ONLY |

### 2.8 Validation Architecture

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| V1 | All five validation layers operational | validation.md §Five layers | Layer 0 + Layer 1 + Layer 3 present; Layer 2 (SSIM) and Layer 4 (artifacts) absent | PARTIAL |
| V2 | Test scene registry with initial corpus | validation.md §Test scene registry | `TestSceneRegistry` with 5 scenes (19 specified) | PARTIAL |
| V3 | Hardware calibration and normalized benchmarks | validation.md §Hardware-normalized | CI multipliers used; calibration vector absent | PARTIAL |
| V4 | Developer visibility artifact pipeline | validation.md §Layer 4 | No artifact generation | ABSENT |
| V5 | Property-based testing for scene graph | validation.md §Layer 0 | `cargo test` tests; no proptest/quickcheck | PARTIAL |
| V6 | DR-V1: scene separable from renderer | validation.md §DR-V1 | `tze_hud_scene` has zero GPU dependencies | FULL |
| V7 | DR-V2: headless rendering | validation.md §DR-V2 | `HeadlessSurface` + offscreen texture | FULL |
| V8 | DR-V3: structured telemetry per frame | validation.md §DR-V3 | `FrameTelemetry`, `SessionSummary`, JSON emission | FULL |
| V9 | DR-V4: deterministic test scenes | validation.md §DR-V4 | `Clock` trait + `TestClock` + `ClockMs` injection | FULL |
| V10 | DR-V5: cargo test --features headless | validation.md §DR-V5 | Headless is runtime flag; feature gate not wired up | PARTIAL |

### 2.9 Telemetry Requirements

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| T1 | Per-frame structured telemetry (timing, throughput, resources, correctness) | RFC 0002 §3.2 Stage 8 | `FrameTelemetry` has timing + resources; correctness fields still absent | PARTIAL |
| T2 | Per-session aggregates with p50/p95/p99 | RFC 0002 §3.2, validation.md §Layer 3 | `SessionSummary` + `LatencyBucket.percentile()` + `assert_p99_under` | FULL |
| T3 | JSON emission for CI consumption | validation.md §LLM development loop | `telemetry.emit_json()` in VS | FULL |

### 2.10 Platform Targets

| # | v1.md requirement | RFC section | Crate/module | Status |
|---|-------------------|-------------|-------------|--------|
| Pl1 | Linux (X11 and Wayland) | RFC 0002 §7.2, v1.md §Platform | Code is platform-independent; AT-SPI2 a11y stub present | RFC-ONLY |
| Pl2 | Windows (Win32) | RFC 0002 §7.2, v1.md §Platform | Code is platform-independent; UIA a11y stub present | RFC-ONLY |
| Pl3 | macOS (Cocoa) | RFC 0002 §7.2, v1.md §Platform | Code is platform-independent; NSAccessibility a11y stub present | RFC-ONLY |
| Pl4 | Headless CI: mesa llvmpipe / WARP / Metal | RFC 0002 §8, v1.md §Platform | `HeadlessSurface` used in VS and budget_assertions tests | PARTIAL |

---

## 3. Summary Statistics

| Category | Total | FULL | PARTIAL | RFC-ONLY | ABSENT |
|----------|-------|------|---------|----------|--------|
| Scene model | 9 | 9 | 0 | 0 | 0 |
| Compositor | 5 | 2 | 3 | 0 | 0 |
| Protocol | 7 | 6 | 1 | 0 | 0 |
| Security | 4 | 2 | 2 | 0 | 0 |
| Interaction | 6 | 5 | 0 | 1 | 0 |
| Window modes | 4 | 1 | 0 | 3 | 0 |
| Failure handling | 4 | 0 | 2 | 2 | 0 |
| Validation arch. | 10 | 5 | 4 | 0 | 1 |
| Telemetry | 3 | 2 | 1 | 0 | 0 |
| Platform targets | 4 | 0 | 1 | 3 | 0 |
| **Total** | **56** | **32 (57%)** | **13 (23%)** | **9 (16%)** | **1 (2%)** |

**Gen-1 baseline (for comparison):**

| Category | FULL | PARTIAL | RFC-ONLY | ABSENT |
|----------|------|---------|----------|--------|
| Total | 20 (36%) | 21 (38%) | 11 (20%) | 4 (7%) |

**Gen-2 delta:** FULL +12, PARTIAL −8, RFC-ONLY −2, ABSENT −3.

---

## 4. New Gaps Found (Gen-3 Candidates)

Per the gen-1 spec, gen-3 beads are the hard limit. The following are the
remaining coverage holes that should be tracked. **Both are non-critical for
the v1 thesis but would block full v1 completeness.**

### GAP-G3-1: MCP `dismiss` and `list_scene` tools absent (P6)

RFC 0002 §2.4 and v1.md §Protocol MCP layer specify `dismiss` (remove a
tile's content without deleting the tile) and `list_scene` (return current
scene state as structured JSON for LLM introspection). These are absent from
`tze_hud_mcp/src/tools.rs`.

**Impact:** Low for the subtitle-publish thesis proof; medium for full LLM
round-trip workflows where an agent needs to inspect existing scene state
before making mutations.

**Suggested:** `task`, P2.

### GAP-G3-2: Test scene corpus is 26% complete (V2)

`validation.md` §Test scene registry specifies 19 named scenes. The
`TestSceneRegistry` has 5 scenes (`empty`, `single_tile`, `two_tiles`,
`max_tiles`, `zone_test`). Missing include all overlay, multi-agent, sync-
group media, lease-expiry, input-highlight, coalesced-dashboard, stress, and
zone-workflow scenes. These are required for full Layers 0–4 validation
completeness.

**Impact:** Medium for v1 completeness; the 5 existing scenes are sufficient
for the vertical slice thesis. Missing scenes block Layer 2 (golden images)
and Layer 4 (visibility artifacts) entirely.

**Suggested:** `task`, P2.

### GAP-G3-3: Layer 2 (SSIM visual regression) and Layer 4 (artifacts) absent (V1, V4)

`validation.md` §Five validation layers requires: golden image baselines +
SSIM comparison (Layer 2) and developer visibility artifacts — `index.html`,
`summary.md`, `manifest.json` in `test_results/` (Layer 4). Neither exists.
Layer 4 is classified ABSENT — the only ABSENT item in the gen-2 matrix.

**Impact:** Medium for CI completeness. Missing golden image infrastructure
means visual regressions in the compositor cannot be detected automatically.
Missing artifacts pipeline means LLMs cannot inspect rendering output without
running `cargo test` locally.

**Suggested:** `task`, P2.

---

## 5. Final Assessment

**All 10 gen-1 gap beads are closed or partially closed:**

| Gap Bead | Title | Gen-2 Status |
|----------|-------|-------------|
| rig-itf | StaticImageNode | CLOSED |
| rig-zo8 | Zone schema | CLOSED |
| rig-s31 | MCP bridge | PARTIAL (5/7 tools) |
| rig-hsp | Budget enforcement | CLOSED |
| rig-nfr | gRPC events | CLOSED |
| rig-j8e | SyncGroups | CLOSED |
| rig-6t7 | Test scenes | PARTIAL (5/19 scenes) |
| rig-ict | Injectable clock | CLOSED |
| rig-0fi | p99 assertions | CLOSED |
| rig-3l8 | A11y stub | CLOSED |

**V1 thesis provability:**

The core v1 statement — "An LLM with only MCP access can publish a subtitle to
a zone with one tool call" — is now provable from the code:
`handle_publish_to_zone` in `tze_hud_mcp` routes through `SceneGraph::publish_to_zone`
via the zone registry, honoring the contention policy. The MCP thesis is closed.

**Critical issues: none.**

**New gen-3 gaps: 3** (non-blocking for v1 thesis).

**Handoff path: direct-merge-candidate.** This is a documentation-only commit
(the reconciliation report). No code changes were made.

---

*Report generated by Beads Worker agent on branch `agent/rig-j1v`.*
