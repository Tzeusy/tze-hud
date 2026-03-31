> **HISTORICAL DOCUMENT** — This is the Gen-1 reconciliation snapshot (2026-03-22). It has been superseded by Gen-2 and Gen-3. For the current baseline, see [docs/RECONCILIATION_STATUS.md](RECONCILIATION_STATUS.md).

# Gen-1 Reconciliation: Spec-to-Code Coverage

**Issue:** rig-5vq.7
**Date:** 2026-03-22
**Scope:** heart-and-soul/ doctrine × RFCs 0001–0004 × vertical slice code
**Branch:** agent/rig-5vq.7

---

## Executive Summary

The gen-1 artifacts (four RFCs and the vertical slice codebase) achieve **strong coverage** of the
v1.md requirements in the areas of scene graph, runtime kernel, timing, and input. Of 56 discrete
v1.md requirements audited, **20 are fully covered**, **21 are partially covered**, **11 are
RFC-specified but not yet implemented** (RFC-ONLY), and **4 are absent** from both RFCs and code.

The vertical slice exercises all six declared layers but exhibits several ergonomic and
completeness gaps relative to the RFC contracts. These gaps do not break the thesis proof but would
become blockers during full implementation.

**Gap beads created:** 10 (rig-itf, rig-zo8, rig-s31, rig-hsp, rig-nfr, rig-j8e, rig-6t7, rig-ict, rig-0fi, rig-3l8).
**Gen-2 reconciliation bead:** rig-j1v — depends on all 10 gap beads.

---

## 1. Coverage Matrix

Each row maps a v1.md requirement to the RFC section that specifies it and the vertical slice
component that exercises it. Status legend:

- **FULL** — doctrine requirement is fully specified in an RFC with quantitative requirements, and
  exercised in the vertical slice.
- **RFC-ONLY** — fully specified in RFC, not yet exercised in vertical slice (expected for later
  implementation cycles).
- **PARTIAL** — RFC or vertical slice covers the concept but with gaps identified below.
- **ABSENT** — no RFC coverage and no vertical slice coverage.

### 1.1 Scene Model Requirements

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| S1 | Tabs: create, switch, delete | RFC 0001 §2.2, §3.1 | `tze_hud_scene::graph` + gRPC server | FULL |
| S2 | Tiles: create, resize, move, delete, z-order | RFC 0001 §2.3, §3.1 | `tze_hud_scene::graph` + gRPC server | FULL |
| S3 | Node type: solid color | RFC 0001 §2.4 | `tze_hud_scene::types::SolidColorNode` | RFC-ONLY |
| S4 | Node type: text/markdown | RFC 0001 §2.4 | `types::TextMarkdownNode`, VS uses gRPC | FULL |
| S5 | Node type: static image | RFC 0001 §2.4 | `types::StaticImageNode` defined in RFC, **missing from code** | PARTIAL |
| S6 | Node type: interactive hit region | RFC 0001 §2.4 | `types::HitRegionNode`, VS exercises fully | FULL |
| S7 | Sync groups: basic membership | RFC 0003 §2 | `types::Tile.sync_group` field exists; no group operations in code | PARTIAL |
| S8 | Atomic batch mutations, all-or-nothing | RFC 0001 §3, §3.2 | `mutation.rs::apply_batch` with rollback | FULL |
| S9 | Zone system: subtitle, notification, status-bar, ambient-background | RFC 0001 §2.5 | `types::ZoneDefinition` stub only — no policies, geometry, or publishing | PARTIAL |

### 1.2 Compositor Requirements

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| C1 | wgpu headless and windowed rendering | RFC 0002 §1.3, §8 | `tze_hud_compositor::surface` + `HeadlessSurface` | FULL |
| C2 | Tile composition with z-order | RFC 0002 §3.2 Stage 6 | `compositor::renderer` (stubs z-order ordering) | PARTIAL |
| C3 | Alpha blending for overlays | RFC 0002 §3.2 Stage 6, §6.2 Level 3 | `renderer.rs` (declared, alpha blend disabled in current impl) | PARTIAL |
| C4 | Background, tile borders, basic visual chrome | RFC 0002 §7, RFC 0001 §2 | VS renders background color + solid tiles | PARTIAL |
| C5 | 60fps on reference hardware | RFC 0002 §3.1 (16.6ms p99 budget) | `FrameTelemetry.frame_time_us` recorded; no assertion against 16.6ms in VS | PARTIAL |

### 1.3 Protocol Requirements

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| P1 | gRPC control plane with protobuf | RFC 0002 §1.1, RFC 0001 §7 | `tze_hud_protocol::server`, tonic/prost | FULL |
| P2 | Scene mutation RPCs | RFC 0001 §3.1 | `apply_mutations` RPC in VS | FULL |
| P3 | Lease management RPCs (request, renew, revoke) | RFC 0002 §4.1, RFC 0001 §3.1 | `acquire_lease`, `renew_lease`, `revoke_lease` RPCs in VS | FULL |
| P4 | Event subscription stream | RFC 0002 §2.4 | `event_tx` broadcast channel exists; no VS subscription exercised | PARTIAL |
| P5 | Telemetry stream | RFC 0002 §2.5, §3.2 Stage 8 | `tze_hud_telemetry` + VS emits JSON | FULL |
| P6 | MCP compatibility layer: create_tab, create_tile, set_content, dismiss, list_scene | RFC 0002 §1.1, §2.4 | **No MCP bridge in code at all** | ABSENT |
| P7 | MCP zone tools: publish_to_zone, list_zones | RFC 0001 §2.5, presence.md zone API | **No MCP bridge in code at all** | ABSENT |

### 1.4 Security Requirements

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| Sec1 | Agent authentication (PSK + local socket) | RFC 0002 §4.1 | `session::SessionRegistry` with PSK | FULL |
| Sec2 | Capability scopes (additive grants, revocation) | RFC 0001 §3.3, RFC 0002 §4.3 | `types::Capability` enum, `Lease.capabilities` | PARTIAL |
| Sec3 | Agent isolation (no cross-agent content access) | RFC 0001 §1.2 namespace isolation | Namespace isolation in `SceneGraph`; not enforced in server RPC handlers | PARTIAL |
| Sec4 | Resource budgets (enforced, throttle + revoke) | RFC 0002 §5 | `ResourceBudget` type defined; enforcement ladder **not implemented** | PARTIAL |

### 1.5 Interaction Requirements

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| I1 | Mouse/pointer input with hit testing | RFC 0004 §3, RFC 0001 §5 | `tze_hud_input::InputProcessor::process` | FULL |
| I2 | Touch input on supported platforms | RFC 0004 §3.2 | RFC specifies; **no touch in code** | RFC-ONLY |
| I3 | Local-first feedback (press, hover, focus) | RFC 0004 §6, RFC 0002 §3.2 Stage 2 | `InputProcessor` updates `HitRegionLocalState` | FULL |
| I4 | Input events forwarded to owning agent | RFC 0004 §6.4 | `InputResult` returned; **no gRPC event dispatch for input** | PARTIAL |
| I5 | HitRegionNode with local pressed/hovered/focused state | RFC 0001 §2.4, RFC 0004 §6.3 | `types::HitRegionLocalState` + `InputProcessor` | FULL |
| I6 | input_to_local_ack p99 < 4ms | RFC 0004 §6.2, validation.md §Layer 3 | `local_ack_us` measured in VS; no p99 assertion | PARTIAL |

### 1.6 Window Modes

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| W1 | Fullscreen mode: guaranteed on all platforms | RFC 0002 §7.1 | `WindowSurface` abstraction (implementation TBD) | RFC-ONLY |
| W2 | Overlay/HUD mode: transparent always-on-top | RFC 0002 §7.1, §7.2 | Specified in RFC; no window mode code yet | RFC-ONLY |
| W3 | Per-region input routing (click-through) | RFC 0002 §7.2 | RFC specifies WM_NCHITTEST/XShape/etc.; **not in code** | RFC-ONLY |
| W4 | Runtime configuration (not compile-time) | RFC 0002 §1.3 | `HeadlessConfig` vs `WindowSurface` trait; same binary | FULL |

### 1.7 Failure Handling

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| F1 | Agent disconnect detection with grace period | RFC 0002 §4.2 (heartbeat timeout) | `heartbeat_timeout: 45s` in RFC; **not implemented in code** | RFC-ONLY |
| F2 | Lease orphaning and cleanup | RFC 0002 §5.2 (revocation tier) | `revoke_lease` RPC removes tiles; no orphan detection | PARTIAL |
| F3 | Disconnection visual indicator | RFC 0002 §3.2 Stage 6 (chrome layer) | Chrome layer **not implemented** in renderer | RFC-ONLY |
| F4 | Reconnection with lease reclaim | RFC 0002 §4.1 hot-connect | Hot-connect in RFC; **not in code** | RFC-ONLY |

### 1.8 Validation Architecture

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| V1 | All five validation layers operational | validation.md §Five layers, RFC 0001 §DR-V1 | Layer 0 tests exist; Layers 1-4 **absent** | PARTIAL |
| V2 | Test scene registry with initial corpus | validation.md §Test scene registry | **No scene registry in code** | ABSENT |
| V3 | Hardware calibration and normalized benchmarks | validation.md §Hardware-normalized performance | `LatencyBucket` percentiles exist; **no calibration vectors** | PARTIAL |
| V4 | Developer visibility artifact pipeline | validation.md §Layer 4 | **No artifact generation code** | ABSENT |
| V5 | Property-based testing for scene graph | validation.md §Layer 0 | `cargo test` tests exist; **no proptest/quickcheck** | PARTIAL |
| V6 | DR-V1: scene separable from renderer | validation.md §DR-V1 | `tze_hud_scene` crate has zero GPU dependencies | FULL |
| V7 | DR-V2: headless rendering | validation.md §DR-V2 | `HeadlessSurface` + offscreen texture | FULL |
| V8 | DR-V3: structured telemetry per frame | validation.md §DR-V3 | `FrameTelemetry`, `SessionSummary`, JSON emission | FULL |
| V9 | DR-V4: deterministic test scenes | validation.md §DR-V4 | **No injectable clock, no scene registry** | PARTIAL |
| V10 | DR-V5: cargo test --features headless | validation.md §DR-V5 | Headless is a runtime flag; feature gate not wired up | PARTIAL |

### 1.9 Telemetry Requirements

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| T1 | Per-frame structured telemetry (timing, throughput, resources, correctness) | RFC 0002 §3.2 Stage 8 | `FrameTelemetry` has timing + resources; **no correctness fields** | PARTIAL |
| T2 | Per-session aggregates with p50/p95/p99 | RFC 0002 §3.2, validation.md §Layer 3 | `SessionSummary + LatencyBucket.percentile()` | FULL |
| T3 | JSON emission for CI consumption | validation.md §LLM development loop | `telemetry.emit_json()` in VS | FULL |

### 1.10 Platform Targets

| # | v1.md requirement | RFC section | VS component | Status |
|---|-------------------|-------------|--------------|--------|
| Pl1 | Linux (X11 and Wayland) | RFC 0002 §7.2, v1.md §Platform | RFC specifies XShape + wlr-layer-shell; code is platform-independent (wgpu/winit) | RFC-ONLY |
| Pl2 | Windows (Win32) | RFC 0002 §7.2, v1.md §Platform | RFC specifies WM_NCHITTEST; code is platform-independent | RFC-ONLY |
| Pl3 | macOS (Cocoa) | RFC 0002 §7.2, v1.md §Platform | RFC specifies hitTest override; code is platform-independent | RFC-ONLY |
| Pl4 | Headless CI: mesa llvmpipe / WARP / Metal | RFC 0002 §8, v1.md §Platform | `HeadlessSurface` used in VS; CI setup not present | PARTIAL |

---

## 2. Gap Analysis

### 2.1 RFC Coverage Gaps

The following doctrine requirements appear in v1.md or heart-and-soul/ but are **underspecified** or
**absent** from the RFCs.

#### GAP-R1: StaticImageNode missing from code (S5)

RFC 0001 §2.4 specifies `StaticImageNode` with `ResourceId`, `ImageFit`, and `present_at`. The
`types.rs` code defines only `SolidColorNode`, `TextMarkdownNode`, and `HitRegionNode` in the
`NodeData` enum. `StaticImageNode` is absent from the implementation. This will block any test
scene requiring image content and the `static_image` node type promised in v1.md.

**Doctrine reference:** v1.md §Scene model — "Basic node types: solid color, text/markdown, static
image, interactive hit region."

#### GAP-R2: Zone system is a stub (S9)

RFC 0001 §2.5 fully specifies the zone registry including `GeometryPolicy`, `RenderingPolicy`,
`ContentionPolicy`, `ZoneMediaType`, and `TransportConstraint`. The code in `types.rs` provides only
`ZoneDefinition { id, name, description }` — a name-only stub. All policy fields, contention
logic, publish operations, and zone-to-tile mapping are absent. v1.md requires at least subtitle,
notification, status-bar, and ambient-background zones to be functional, including MCP publishing.

**Doctrine reference:** v1.md §Scene model zones, presence.md §Zones, v1.md §Protocol MCP zone tools.

#### GAP-R3: MCP compatibility layer entirely absent (P6, P7)

RFC 0002 §2.4 mentions the MCP bridge as a responsibility of the network thread. No MCP bridge
exists in `tze_hud_protocol` or any crate. v1.md explicitly ships MCP tools: `create_tab`,
`create_tile`, `set_content`, `dismiss`, `list_scene`, `publish_to_zone`, `list_zones`. MCP is the
primary LLM-first interaction surface; its complete absence is a significant gap for the v1 thesis
("An LLM with only MCP access can publish a subtitle to a zone with one tool call").

**Doctrine reference:** v1.md §Protocol MCP layer, architecture.md §Compatibility plane.

#### GAP-R4: Resource budget enforcement ladder not implemented (Sec4)

RFC 0002 §5.1–5.3 specifies a three-tier enforcement ladder (Normal → Warning → Throttled →
Revoked) with the `AgentResourceState` and `BudgetState` types. The `ResourceBudget` type exists
in code but the enforcement logic, the `BudgetState` state machine, and the warning/throttle/revoke
escalation are not implemented. The `server.rs` RPC handlers do not check per-agent budgets before
committing mutations.

**Doctrine reference:** security.md §Resource governance, v1.md §Security resource budgets.

#### GAP-R5: Event subscription and input dispatch not wired (P4, I4)

RFC 0002 §2.4 specifies event fan-out from compositor thread to network threads. A `broadcast::Sender<SceneEvent>` exists in `SharedState` but nothing publishes scene events onto it, and no gRPC streaming RPC exposes it. RFC 0004 §6.4 specifies that input events must be dispatched to the owning agent asynchronously after local feedback. The vertical slice's `InputResult` is only printed to stdout — no gRPC dispatch happens.

**Doctrine reference:** architecture.md §Session model, v1.md §Protocol gRPC event subscription stream.

#### GAP-R6: Sync group operations not implemented (S7)

RFC 0003 §2 fully specifies `SyncGroup`, `SyncCommitPolicy` (`AllOrDefer`/`AvailableMembers`),
cross-agent sync group membership, and the deferred-commit mechanism in Stage 4. The
`Tile.sync_group` field exists but `SyncGroup` as a scene object, the `create_sync_group` /
`join_sync_group` / `leave_sync_group` mutations, and the Stage 4 deferred-commit logic are all
absent from the implementation.

**Doctrine reference:** v1.md §Scene model sync groups, RFC 0003 §2, presence.md §Scene mutations atomic.

#### GAP-R7: tze_hud_a11y crate missing (RFC 0004 §5)

RFC 0004 §5.8 specifies `tze_hud_a11y` as a dedicated crate providing the accessibility bridge
layer: AT-SPI2 on Linux, UIA/IAccessible2 on Windows, and NSAccessibility on macOS. Sections
§5.1–5.8 fully specify the a11y tree structure, role/state mapping, focus management, and platform
API bindings. The crate does not exist in the repository — not even as a stub. This is a structural
gap: the RFC names it as a separate crate boundary but no Cargo.toml entry or module scaffold has
been created.

**Doctrine reference:** RFC 0004 §5.1–5.8, DR-I7 (keyboard-only navigation), presence.md
(accessibility requirements).

### 2.2 Vertical Slice Completeness Gaps

#### GAP-VS1: Validation layers 1–4 absent (V1, V2, V4)

The vertical slice exercises the scene graph and gRPC integration but does not constitute the five
validation layers defined in validation.md. Specifically:

- **Layer 1** (headless pixel readback): The VS calls `read_pixels()` and prints the pixel
  buffer size but does not assert on any pixel value or color region.
- **Layer 2** (visual regression / SSIM): No golden images, no SSIM comparison infrastructure.
- **Layer 3** (compositor telemetry validation): `FrameTelemetry` is emitted but no p99 assertions
  are made (e.g., `assert!(frame_time_p99 < 16_600)`).
- **Layer 4** (developer visibility artifacts): No `test_results/` directory generation, no
  `index.html`, no `manifest.json`, no `summary.md`.

The test scene registry (named scenes as defined in validation.md §Test scene registry) does not
exist at all — there are no named test scenes, no `empty_scene`, `single_tile_solid`,
`three_tiles_no_overlap`, etc.

**Doctrine reference:** validation.md §Five validation layers, v1.md §Validation.

#### GAP-VS2: No injectable clock / non-deterministic timestamps (V9)

validation.md §DR-V4 requires all time sources to be injectable for deterministic tests.
`tze_hud_scene::graph` uses `SystemTime::UNIX_EPOCH` directly in `now_millis()` with no injection
point. `tze_hud_telemetry::record` uses `Instant::now()` directly. Tests that depend on
lease expiry or timestamp-sensitive behavior are non-deterministic and cannot be fully reproduced.

**Doctrine reference:** validation.md §DR-V4, RFC 0003 §1.2 (injectable clock for headless).

#### GAP-VS3: p99 budget assertions absent from telemetry (C5, I6, T1)

The vertical slice prints latency values (`local_ack_us`, `hit_test_us`, `frame_time_us`) but
makes no assertions against the RFC's specified budgets:

- `input_to_local_ack` p99 < 4,000μs — measured but not asserted
- `hit_test` p99 < 100μs — measured but not asserted
- `frame_time` p99 < 16,600μs — measured but not asserted
- `input_to_scene_commit` p99 < 50,000μs — not measured at all

Without these assertions the VS does not prove the latency thesis. A slow CI machine could pass
the VS with 50ms frame times without any failure signal.

**Doctrine reference:** validation.md §Layer 3 latency budgets, v1.md §Performance is real.

---

## 3. Gap Beads

The following beads were created as children of epic `rig-5vq`. They are listed in creation order.
All beads have been created sequentially.

| Bead ID | Title | Priority | Blocks |
|---------|-------|----------|--------|
| rig-itf | Implement StaticImageNode in tze_hud_scene and compositor | P2 | gen-2 |
| rig-zo8 | Implement full ZoneDefinition schema and zone publishing pipeline | P1 | gen-2 |
| rig-s31 | Implement MCP compatibility bridge (create_tab, create_tile, set_content, publish_to_zone, list_zones) | P1 | gen-2 |
| rig-hsp | Implement resource budget enforcement ladder (Warning/Throttle/Revoke) | P2 | gen-2 |
| rig-nfr | Wire gRPC event subscription stream and input dispatch to agents | P2 | gen-2 |
| rig-j8e | Implement SyncGroup scene object and AllOrDefer commit policy | P2 | gen-2 |
| rig-6t7 | Implement test scene registry with Layer 0 corpus (empty_scene through zone_disconnect_cleanup) | P1 | gen-2 |
| rig-ict | Add injectable clock abstraction to scene graph and telemetry | P2 | gen-2 |
| rig-0fi | Add p99 budget assertions and Layer 1 pixel assertions to vertical slice | P2 | gen-2 |
| rig-3l8 | Create tze_hud_a11y crate stub with AT-SPI2/UIA/NSAccessibility scaffolding | P2 | gen-2 |

---

## 4. Detailed RFC Audit

### RFC 0001 — Scene Contract

**Cited doctrine sections:** presence.md (tabs, tiles, zones, atomic mutations), architecture.md
(compositing model, node types, session model, resource lifecycle), security.md (agent isolation,
capability scopes), validation.md (DR-V1), v1.md (scene model).

**Coverage of cited doctrine:**

| Doctrine principle | RFC coverage | Verdict |
|-------------------|--------------|---------|
| Tabs are modes (not browser tabs) | §2.2 Tab struct with name and display_order | FULL |
| Tiles are territories (geometry, z-order, input, sync, lease, budget) | §2.3 Tile struct with all fields | FULL |
| Nodes: 4 V1 types | §2.4 all four types with field specs | FULL |
| Atomic batch mutations | §3 full pipeline with rejection semantics | FULL |
| Zone system with 4 policies | §2.5 ZoneDefinition with all policy types | FULL |
| All-or-nothing mutation | §3.2 Rejection semantics | FULL |
| Agent isolation (cross-agent content opaque) | §1.2 namespace isolation, §3.3 lease checks | FULL |
| Capability scopes (additive, granular, revocable) | §3.3 lease validation, error codes | FULL |
| DR-V1 scene separable | §1.1 no GPU types in scene objects | FULL |
| Resource lifecycle (ref-counted, deterministic) | §6 (resource lifecycle section, not fully quoted here) | FULL |
| ResourceId (content-addressed hash) | §1.1 BLAKE3 content hash | FULL |
| Structured error responses with correction_hint | §3.4 BatchRejected with context + correction_hint | FULL |

**Quantitative requirements present:** yes — snapshot < 1ms, diff < 500μs, hit-test < 100μs,
validation < 200μs/batch, commit < 50μs, full path < 300μs p99.

**Wire format specified:** yes — protobuf schema described throughout §1–§7.

**Platform targets:** pure Rust, platform-independent (correct for scene graph layer).

**Verdict: STRONG coverage. One noted discrepancy:** RFC §4.2 reconnect diff section was flagged in
the bead's close reason as contradicting v1.md's deferral of resumable state sync. This was
reportedly fixed in PR review round 2. The current RFC text in the repo should be verified against
this fix — reconciliation author could not confirm the diff section was fully removed vs. correctly
scoped to "snapshot only, no incremental diff."

### RFC 0002 — Runtime Kernel

**Cited doctrine sections:** architecture.md (screen sovereignty, compositing, resource lifecycle,
versioning), security.md (resource governance), failure.md (core principle, degradation axes),
validation.md (performance budgets, DR-V2, DR-V3), v1.md (compositor, window modes).

**Coverage of cited doctrine:**

| Doctrine principle | RFC coverage | Verdict |
|-------------------|--------------|---------|
| LLMs never sit in frame loop | §2 thread model separates agent network threads from compositor thread | FULL |
| Screen sovereignty (runtime owns pixels, timing, composition) | §3 frame pipeline, §5 budget enforcement | FULL |
| 60fps with p99 < 16.6ms | §3.1 8-stage pipeline with per-stage budgets | FULL |
| input_to_local_ack p99 < 4ms | §3.2 Stages 1+2 combined < 1ms | FULL |
| input_to_scene_commit p99 < 50ms | §4.1 admission and §3.2 Stage 4 | FULL |
| input_to_next_present p99 < 33ms | §3.2 Stages 1–7 total | FULL |
| DR-V2: headless rendering | §1.3 HeadlessSurface, offscreen texture | FULL |
| DR-V3: structured telemetry per frame | §2.5 telemetry thread, §3.2 Stage 8, TelemetryRecord | FULL |
| Degradation ladder (6 levels) | §6.2 levels 1–5 with triggers and recovery | FULL |
| Resource governance (warn/throttle/revoke) | §5.1–5.3 AgentResourceState + BudgetState | FULL |
| Human override (always usable) | §7.1 chrome layer always on top | FULL |
| Agent disconnect + grace period + reconnect | §4.2 heartbeat timeout, §4.4 hot-connect | FULL |
| Overlay mode click-through per platform | §7.2 WM_NCHITTEST, hitTest, XShape, wlr-layer-shell | FULL |

**Quantitative requirements present:** yes — all stage budgets, session limits table, per-agent
envelope table, degradation trigger/recovery thresholds.

**Wire format specified:** yes — gRPC + protobuf, bounded channel capacities, message type table.

**Platform targets:** Windows (Win32), macOS, X11, wlroots Wayland — all specified in §7.2 with
platform-specific code patterns.

**Verdict: STRONG coverage.** One noted gap: the RFC specifies `TelemetryRecord` fields including
`telemetry_overflow_count`, `shed_count`, and `degradation_level` but the code's `FrameTelemetry`
struct is missing `shed_count`, `telemetry_overflow_count`, `lease_violations`, and
`budget_overruns`. The telemetry schema in code is a subset of the RFC's specified schema.

### RFC 0003 — Timing Model

**Cited doctrine sections:** architecture.md (time is first-class, arrival ≠ presentation, message
classes), presence.md (sync-group membership), validation.md (split latency budgets, DR-V4).

**Coverage of cited doctrine:**

| Doctrine principle | RFC coverage | Verdict |
|-------------------|--------------|---------|
| Arrival time ≠ presentation time | §3.1 invariant, §5.3 present_at semantics | FULL |
| Four clock domains | §1.1 display/monotonic/network/media | FULL |
| Sync groups (atomic multi-tile commit) | §2 full spec with AllOrDefer/AvailableMembers | FULL |
| present_at / expires_at / sync_group in API | §3.2 timestamp fields table | FULL |
| DR-V4: injectable clocks | §1.2 "injectable clock source for all timing paths" | FULL (in RFC, absent in code) |
| Split latency budgets (3 metrics) | Referenced from validation.md §Layer 3 | FULL |
| Clock drift detection and correction | §4 full drift rules with tolerance tables | FULL |
| Frame deadline model | §5.1 mutation intake cutoff, §5.2 late arrival policy | FULL |
| Expiry policy | §5.4 expiry heap, O(expired_items) | FULL |
| Media timing (GStreamer) | §6 explicitly post-v1 deferred | FULL (deferred) |

**Quantitative requirements present:** yes — drift tolerance (100ms/1s), max pending queue (256),
presentation accuracy (±1 frame), skew correction formula.

**Wire format specified:** yes — all timestamp fields defined as uint64 UTC microseconds.

**Platform targets:** not applicable (timing model is platform-independent).

**Verdict: FULL doctrine coverage.** No gaps in RFC itself. Implementation gap (injectable clock)
captured in GAP-VS2 / rig-ict.

### RFC 0004 — Input Model

**Cited doctrine sections:** presence.md (interaction model, focus, input routing, gesture
arbitration, IME/a11y), architecture.md (overlay click-through), v1.md (hit_region node,
local-first feedback).

**Coverage of cited doctrine:**

| Doctrine principle | RFC coverage | Verdict |
|-------------------|--------------|---------|
| Local feedback first (< 4ms) | §6.1–6.2, DR-I1 | FULL |
| Runtime arbitrates (agents don't race) | §3.5 gesture arbiter | FULL |
| Screen is sovereign (chrome wins hit-test) | §1.1 focus tree, §3.5 | FULL |
| LLMs never sit in frame loop | §6.4 remote semantics are async | FULL |
| Focus model: per-tile, not per-agent | §1.1–1.4 focus tree + cycling | FULL |
| Input routing and bubbling | §3.1, presence.md cross-reference | FULL |
| Gesture arbitration | §3.3–3.6 recognizer pipeline + arbiter | FULL |
| IME/text input | §4 full IME lifecycle + platform APIs | FULL |
| Accessibility (AT-SPI, UIA, NSAccessibility) | §5.1–5.8 full a11y tree | FULL |
| Pointer capture | §2 capture model | FULL |
| input_to_local_ack p99 < 4ms | DR-I1 | FULL |
| Hit-test < 100μs for 50 tiles | DR-I2, RFC 0001 §5.1 | FULL |
| Keyboard-only navigation | §5.7, DR-I7 | FULL |

**Quantitative requirements present:** yes — DR-I1 through DR-I8 table, gesture recognizer < 50μs
per update, IME < 1 frame, a11y tree sync < 100ms.

**Wire format specified:** yes — all protobuf messages for focus, capture, IME, and a11y.

**Platform targets:** Windows (UIA/IAccessible2), macOS (NSAccessibility), Linux (AT-SPI2), IME
platform table in §4.6.

**One RFC gap:** RFC 0004 §5.8 specifies `tze_hud_a11y` as a separate crate but this crate does
not exist in the repository. The a11y bridge is a missing crate not yet created even as a stub.

**Verdict: STRONG coverage with one structural gap** (missing `tze_hud_a11y` crate stub).

---

## 5. Vertical Slice Layer-by-Layer Audit

The vertical slice (`examples/vertical_slice/src/main.rs`) claims to demonstrate 6 layers.

| Layer | Claimed | What actually happens | Verdict |
|-------|---------|----------------------|---------|
| 1. Headless scene graph | Scene created at 800×600 | `HeadlessRuntime::new()` creates scene + GPU surface | PASS |
| 2. Native window + compositor | "wgpu" rendering | `render_frame()` calls GPU pipeline; pixel readback works | PASS (minimal) |
| 3. Resident gRPC agent | Client connects + authenticates | tonic client authenticates with PSK | PASS |
| 4. Lease acquisition | Acquire + renew + revoke | Three RPCs exercised and asserted | PASS |
| 5. Interactive hit-region | Hover + press + release | `InputProcessor` exercises local feedback | PASS |
| 6. Telemetry + artifacts | JSON emitted | `emit_json()` produces session summary | PASS (partial — no artifacts) |

**All 6 layers connected end-to-end:** yes — data flows from scene graph through gRPC mutation
through compositor render through input processor through telemetry in one execution.

**API ergonomics observations:**

1. The VS must create a tab directly via `runtime.shared_state()` after connecting the gRPC client
   because there is no "create tab" RPC. This is an ergonomic gap — agents should be able to
   create tabs via the same mutation pipeline.

2. The `tab_id: String::new()` in `CreateTileMutation` indicates the server silently uses the
   active tab when no tab ID is provided. This implicit behavior is not in RFC 0001 and is an
   undocumented API assumption.

3. The vertical slice tests only a single agent. The v1.md success criterion requires 3 concurrent
   agents. A multi-agent test is absent.

---

## 6. Summary Statistics

| Category | Total requirements | FULL | PARTIAL | RFC-ONLY | ABSENT |
|----------|--------------------|------|---------|----------|--------|
| Scene model | 9 | 5 | 3 | 1 | 0 |
| Compositor | 5 | 1 | 4 | 0 | 0 |
| Protocol | 7 | 4 | 1 | 0 | 2 |
| Security | 4 | 1 | 3 | 0 | 0 |
| Interaction | 6 | 3 | 2 | 1 | 0 |
| Window modes | 4 | 1 | 0 | 3 | 0 |
| Failure handling | 4 | 0 | 1 | 3 | 0 |
| Validation arch. | 10 | 3 | 5 | 0 | 2 |
| Telemetry | 3 | 2 | 1 | 0 | 0 |
| Platform targets | 4 | 0 | 1 | 3 | 0 |
| **Total** | **56** | **20 (36%)** | **21 (38%)** | **11 (20%)** | **4 (7%)** |

Notes:
- RFC-ONLY items are not gaps — they are correctly specified but implementation is expected in a
  later cycle (windowed rendering, platform click-through, multi-platform CI).
- ABSENT items (MCP layer, test scene registry, visibility artifact pipeline) represent missing
  RFC coverage that needs new implementation work.
- PARTIAL items span a wide range: some require small additions (add assertions to VS), others
  require significant new code (budget enforcement ladder, sync group operations).

---

## 7. Conclusion and Next Steps

The gen-1 RFCs are high-quality specifications. RFCs 0001–0004 collectively provide thorough,
quantitative, doctrine-grounded coverage of the core v1 architecture. The major structural
decisions — scene graph model, frame pipeline, timing contract, input model — are well-specified
with correct performance budgets, wire formats, and platform targets.

The gen-1 code (vertical slice + crates) proves that the architecture is implementable and that the
six declared layers connect end-to-end. However, it is substantially incomplete relative to the
full v1.md feature set.

The 10 gap beads created (rig-itf, rig-zo8, rig-s31, rig-hsp, rig-nfr, rig-j8e, rig-6t7, rig-ict, rig-0fi, rig-3l8) cover the highest-priority missing work:
zone publishing (critical for the v1 MCP thesis), MCP bridge (critical for the v1 LLM story),
test scene registry (required for the validation thesis), and several protocol/security
completeness items.

A **gen-2 reconciliation bead** (rig-j1v) has been created that depends on all 10 gap beads and
will re-run this audit workflow after those gaps are closed.

---

*Report generated by Beads Worker agent on branch `agent/rig-5vq.7`.*
