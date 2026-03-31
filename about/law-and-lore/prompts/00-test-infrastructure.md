# Epic 0: Test-Driven Validation Infrastructure

> **Dependencies:** None — starts first. All other epics depend on this.
> **Depended on by:** Epics 1–12 (all consume test scenes, trait contracts, and assertion infrastructure)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md`
> **Secondary specs:** All 12 subsystem specs (scenarios become test cases)

## Prompt

> **Before starting:** Read `docs/prompts/PREAMBLE.md` for authority rules, doctrine guardrails, and v1 scope tagging requirements that apply to every bead.

Create a `/beads-writer` epic for **test-driven validation infrastructure** — the prerequisite layer that must exist before implementation workers can begin building v1 subsystems against `openspec/changes/v1-mvp-standards`.

### Context

The v1-mvp-standards spec set contains 378 normative requirements across 12 subsystem specs, with 623 WHEN/THEN scenarios that map directly to test cases. The project doctrine (`heart-and-soul/validation.md`, `validation-framework/spec.md`) treats validation as architecture, not garnish. The existing codebase already has:

- `TestSceneRegistry` with 5 named scenes (spec requires 25) in `crates/tze_hud_scene/src/test_scenes.rs`
- `assert_layer0_invariants()` with 16 structural checks
- `HeadlessRuntime` with pixel readback in `crates/tze_hud_runtime/src/headless.rs`
- `LatencyBucket` with `assert_p99_under()` in `crates/tze_hud_telemetry/src/record.rs`
- `Clock` trait with `TestClock` for deterministic time injection in `crates/tze_hud_scene/src/clock.rs`
- `proptest` already in dev-dependencies for `tze_hud_scene`
- 3 pixel readback tests and 5 budget assertion tests in `examples/vertical_slice/tests/budget_assertions.rs`
- Proto definitions (`types.proto`, `events.proto`, `session.proto`) under `crates/tze_hud_protocol/proto/`

### Epic structure

Create an epic with **5 category beads** as children, forming a partial dependency chain. Each category bead should be an independently assignable unit of work. The categories are:

#### 1. Test scene registry expansion (no dependencies — can start immediately)
Expand from 5 → 25 named test scenes as pure data structures in `test_scenes.rs`. Every scene is specified by name in `validation-framework/spec.md` line 160: `empty_scene`, `single_tile_solid`, `three_tiles_no_overlap`, `overlapping_tiles_zorder`, `overlay_transparency`, `tab_switch`, `lease_expiry`, `mobile_degraded`, `sync_group_media`, `input_highlight`, `coalesced_dashboard`, `max_tiles_stress`, `three_agents_contention`, `overlay_passthrough_regions`, `disconnect_reclaim_multiagent`, `privacy_redaction_mode`, `chatty_dashboard_touch`, `zone_publish_subtitle`, `zone_reject_wrong_type`, `zone_conflict_two_publishers`, `zone_orchestrate_then_publish`, `zone_geometry_adapts_profile`, `zone_disconnect_cleanup`, `policy_matrix_basic`, `policy_arbitration_collision`. Each scene definition must specify: scene graph structure, synthetic content, Layer 0 invariants that must hold, and expected properties for later pixel/budget validation. No GPU or rendering needed — pure data.

#### 2. Trait contract test harnesses (no dependencies — can start immediately)
Define Rust traits that encode spec requirements, with test suites written against the trait that any correct implementation must satisfy. Key traits to define with companion test modules:
- `LeaseStateMachine` — state transitions per `lease-governance/spec.md` (REQUESTED→ACTIVE→EXPIRED/ORPHANED/RECLAIMED/REVOKED/SUSPENDED, TTL pause during suspension, grace periods)
- `PolicyEvaluator` — pure function over typed PolicyContext per `policy-arbitration/spec.md` (7-level stack, short-circuit rules, budget checks)
- `EventRouter` — dispatch pipeline per `scene-events/spec.md` (naming convention, interruption classification, quiet hours, subscription filtering)
- `ResourceStore` — upload/dedup/GC per `resource-store/spec.md` (BLAKE3 content addressing, refcounting, ephemeral v1 lifecycle)
- `ConfigLoader` — parse→normalize→validate→freeze per `configuration/spec.md` (profile resolution, capability vocabulary validation, includes rejection)
Each trait's test module should encode the spec's WHEN/THEN scenarios as `#[test]` functions that call trait methods. Tests compile but fail until a correct implementation is provided.

#### 3. Layer 0 scene graph assertion expansion (depends on #1 for scene fixtures)
Expand `assert_layer0_invariants()` from 16 checks to comprehensive coverage of all scene-graph, lease-governance, and zone-related requirements. This is pure logic — no GPU, no rendering, must run in < 2 seconds total. Key areas from the specs:
- Atomic batch semantics: partial batch failure rolls back entire batch (`scene-graph/spec.md`)
- Namespace isolation: agents cannot mutate outside their namespace (`scene-graph/spec.md`)
- Zone publication rules: type validation, contention policies, media-type checks (`scene-graph/spec.md`)
- Lease state machine: all valid/invalid transitions (`lease-governance/spec.md`)
- Hit-test pipeline correctness: chrome-first, z-order, node-level (`input-model/spec.md`)
- Property-based testing via proptest: generate random scenes/mutations, assert invariants hold for 10,000+ configurations per `validation-framework/spec.md`

#### 4. Protocol conformance test suite (depends on #2 for trait contracts)
Test the wire protocol contract against the proto definitions and session-protocol spec:
- Message round-trip: every message type in `types.proto`, `events.proto`, `session.proto` serializes and deserializes correctly
- Session state machine: encode legal transitions (Connecting→Handshaking→Active, Active→Disconnecting→Closed, etc.) and verify illegal transitions are rejected, per `session-protocol/spec.md`
- Capability gating: guest tools (`publish_to_zone`, `list_zones`, `list_scene`) succeed without `resident_mcp`; resident tools (`create_tab`, `create_tile`, `set_content`, `dismiss`) fail with structured `CAPABILITY_REQUIRED` error, per `session-protocol/spec.md` lines 487-510
- Heartbeat protocol: verify timeout detection at `heartbeat_missed_threshold` × interval, per `session-protocol/spec.md`
- Clock domain validation: verify all timestamp fields use correct `_wall_us`/`_mono_us` suffixes, per `timing-model/spec.md`
- Subscription filtering: verify category-to-capability mapping, per `session-protocol/spec.md` lines 445-452

#### 5. Layer 1 pixel readback test definitions (depends on #1 for scene fixtures)
Define expected pixel assertions for all 25 test scenes that have visual output. Pattern: render scene → read pixels → assert region colors within tolerance. Extend the 3 existing pixel tests to cover:
- All solid-color and z-order scenes from the registry
- Alpha blending tolerance (±2/channel on software GPU, ±1 for solid fills) per `validation-framework/spec.md`
- Zone rendering: subtitle text present, notification stack visible, status-bar entries rendered
- Redaction: privacy_redaction_mode scene shows neutral pattern, no agent content visible
- Chrome layer: always rendered on top, never occluded by agent tiles
These tests define expected values now; they will start passing as compositor features land.

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite the specific spec file, requirement name, and line numbers that define the acceptance criteria (e.g., "per `lease-governance/spec.md` Requirement: Lease State Machine, lines 15-45")
2. **WHEN/THEN scenarios** — quote or reference the exact spec scenarios that the tests encode
3. **Acceptance criteria** — concrete, measurable: number of test functions, invariant checks, scene definitions, or proto messages covered
4. **Crate/file location** — where the tests should live in the workspace
5. **What this does NOT include** — explicitly state that these beads produce test infrastructure, not implementation code. Implementation beads come later and must make these tests pass.

### Dependency chain

```
#1 Scene Registry ──→ #3 Layer 0 Assertions
                  ──→ #5 Layer 1 Pixel Definitions
#2 Trait Contracts ──→ #4 Protocol Conformance
```

Beads #1 and #2 have no dependencies and can start immediately in parallel. #3 and #5 depend on #1 (scene fixtures). #4 depends on #2 (trait contracts).
