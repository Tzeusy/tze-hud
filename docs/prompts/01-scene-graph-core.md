# Epic 1: Scene Graph Core

> **Dependencies:** Epic 0 (test scenes, Layer 0 assertions, trait contracts)
> **Depended on by:** Epics 2, 3, 4, 5, 6, 10 (nearly everything builds on scene types)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/scene-graph/spec.md`
> **Secondary specs:** `resource-store/spec.md` (ResourceId), `validation-framework/spec.md` (Layer 0)

## Prompt

Create a `/beads-writer` epic for **scene graph core implementation** — the foundation data model that all other v1 subsystems build on. This epic implements the scene-graph spec against the test infrastructure from Epic 0.

### Context

The scene graph is the pure, renderer-independent data model at the center of tze_hud. It owns identity types, the scene hierarchy, atomic mutations, namespace isolation, zone ontology, and hit-test data. The existing crate `crates/tze_hud_scene/` already has a partial implementation in `src/graph.rs` with tiles, nodes, leases, zones, sync groups, and hit regions. Epic 0 provides test scenes and Layer 0 invariant assertions that this epic's implementation must satisfy.

### Epic structure

Create an epic with **6 implementation beads** as children:

#### 1. SceneId and ResourceId identity types (no internal dependencies)
Implement the binary identity contract per `scene-graph/spec.md` Requirement: SceneId Identity and Requirement: ResourceId Identity.
- SceneId: 16-byte little-endian UUIDv7, monotonically increasing, unique within runtime lifetime
- ResourceId: 32-byte raw BLAKE3 content hash (NOT hex-encoded), equality by byte comparison
- Both must implement `Eq`, `Hash`, `Ord`, `Clone`, `Copy`, serialization to/from protobuf `bytes`
- **Acceptance:** All Layer 0 identity invariant checks from Epic 0 pass. SceneId generation is monotonic. ResourceId deduplication works by byte equality.
- **Spec refs:** `scene-graph/spec.md` lines 23-30, `resource-store/spec.md` lines 5-16

#### 2. Scene hierarchy: Scene→Tab→Tile→Node (depends on #1)
Implement the four-level hierarchy per `scene-graph/spec.md` Requirement: Scene Graph Hierarchy.
- Scene contains ordered Tabs; Tabs contain ordered Tiles; Tiles contain a node tree
- V1 node types: SolidColorNode, TextMarkdownNode, StaticImageNode, HitRegionNode
- Tab ordering is user-visible; tile ordering within a tab is by z-order
- Namespace isolation: each agent session owns a namespace; mutations outside it are rejected
- **Acceptance:** `assert_layer0_invariants()` passes for all 25 test scenes. Namespace isolation property tests pass (proptest from Epic 0). Tab/tile/node CRUD scenarios from spec pass.
- **Spec refs:** `scene-graph/spec.md` Requirement: Scene Graph Hierarchy, Requirement: Namespace Isolation

#### 3. Atomic batch mutations (depends on #2)
Implement the transactional mutation pipeline per `scene-graph/spec.md` Requirement: Atomic Batch Mutations.
- Agent stages a batch of mutations (create tiles, set content, switch tab, etc.)
- Compositor applies entire batch in one frame — all-or-nothing
- If any mutation fails validation, entire batch is rejected with structured error
- Mutation types: CreateTile, SetTileRoot, PublishToZone, ClearZone (matching `types.proto` MutationProto)
- **Acceptance:** Partial-failure rollback test passes (Epic 0 Layer 0). Batch atomicity property tests pass. All mutation WHEN/THEN scenarios from spec pass.
- **Spec refs:** `scene-graph/spec.md` Requirement: Atomic Batch Mutations, `session-protocol/spec.md` Requirement: Mutation Transport

#### 4. Zone ontology: type, instance, publication, occupancy (depends on #2)
Implement the four-level zone model per `scene-graph/spec.md` Requirement: Zone Type Registry, Requirement: Zone Instance, Requirement: Zone Publication, Requirement: Zone Occupancy.
- Zone types: subtitle, notification, status-bar, pip, ambient-background, alert-banner (v1 set)
- Zone instances: type bound to specific tab with geometry policy, created from configuration
- Publications: single publish event with content, TTL, key, priority, classification
- Occupancy: runtime's resolved current state after contention resolution
- Contention policies: latest-wins, stack, merge-by-key, replace
- **Acceptance:** Zone test scenes from Epic 0 pass (`zone_publish_subtitle`, `zone_reject_wrong_type`, `zone_conflict_two_publishers`, etc.). Contention policy property tests pass.
- **Spec refs:** `scene-graph/spec.md` Requirement: Zone Type Registry through Requirement: Zone Occupancy Resolution

#### 5. Hit-test data model (depends on #2)
Implement hit-test region tracking per `scene-graph/spec.md` Requirement: Hit-Test Pipeline and `input-model/spec.md` Requirement: HitRegionNode.
- HitRegionNode is a leaf node type with bounds, interaction_id, accepts_focus, accepts_pointer
- Hit-test query: given (x, y) display coordinates, return the topmost hit region considering z-order
- Chrome layer always wins hit-test (checked first), then tiles in z-order, then nodes within tile
- No input processing here — just the spatial query data model
- **Acceptance:** `input_highlight` test scene passes Layer 0 invariants. Hit-test correctness for overlapping regions verified by property tests.
- **Spec refs:** `scene-graph/spec.md` Requirement: Hit-Test Pipeline, `input-model/spec.md` Requirement: HitRegionNode Primitive

#### 6. Deterministic scene snapshots (depends on #2, #4)
Implement scene serialization per `scene-graph/spec.md` Requirement: Deterministic Scene Snapshots.
- Snapshot captures full scene topology at a point in time
- Same scene state always produces byte-identical snapshot (deterministic ordering)
- Snapshots are agnostic to resource persistence (reference ResourceIds but don't embed data)
- Resources are ephemeral in v1 — snapshots don't guarantee backing data survives restart
- **Acceptance:** Snapshot determinism: serialize same scene twice, assert byte equality. Round-trip: deserialize snapshot, re-serialize, assert equality. All 25 test scenes produce valid snapshots.
- **Spec refs:** `scene-graph/spec.md` Requirement: Deterministic Scene Snapshots, `resource-store/spec.md` (v1 ephemerality)

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite the specific spec file, requirement name, and line numbers
2. **WHEN/THEN scenarios** — reference the exact spec scenarios this bead implements
3. **Acceptance criteria** — which Epic 0 tests must pass, plus any new tests added
4. **Crate/file location** — primarily `crates/tze_hud_scene/src/`
5. **Epic 0 test gates** — list the specific Layer 0 assertions and test scenes that must pass

### Dependency chain

```
#1 Identity Types ──→ #2 Scene Hierarchy ──→ #3 Atomic Batches
                                          ──→ #4 Zone Ontology ──→ #6 Snapshots
                                          ──→ #5 Hit-Test Data
```
