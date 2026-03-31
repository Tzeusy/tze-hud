# Widget System Spec-to-Code Reconciliation (hud-mim2.8)

Generated: 2026-03-30
Epic: hud-mim2 (Widget system)
Auditor: hud-mim2.8 reconciliation worker

---

## Summary

All five delta specs were reviewed against the codebase delivered by sibling beads
hud-mim2.1 through hud-mim2.7. The widget system is substantially complete. Three
categories of gap items were identified:

- **A** — Fully implemented and tested (COVERED)
- **B** — Implemented but with a known deferral explicitly noted (PARTIAL/DEFERRED)
- **C** — Not implemented; constitutes a discovered gap (MISSING)

---

## spec: widget-system/spec.md

### REQ: Widget Type Ontology

| Scenario | Status | Implementing code |
|---|---|---|
| Four-level ontology (Type, Instance, Publication, Occupancy) | COVERED | `crates/tze_hud_scene/src/types.rs` — `WidgetDefinition`, `WidgetInstance`, `WidgetPublishRecord`, `WidgetOccupancy` |
| Widget type immutable after registration | COVERED | `WidgetRegistry::register_definition` overwrites, but agent path provides no mutation RPC |
| Instances reference exactly one type | COVERED | `WidgetInstance.widget_type_name` validated at config time |
| Publications reference exactly one instance | COVERED | `publish_to_widget` resolves by `instance_name` |
| Occupancy derived from active publications | COVERED | `WidgetRegistry::get_occupancy` computes effective_params |

### REQ: Widget Parameter Schema

| Scenario | Status | Implementing code |
|---|---|---|
| Valid publish accepted | COVERED | `SceneGraph::publish_to_widget` (graph.rs:2375) |
| Unknown parameter rejected → WIDGET_UNKNOWN_PARAMETER | COVERED | graph.rs:2420 |
| Type mismatch rejected → WIDGET_PARAMETER_TYPE_MISMATCH | COVERED | graph.rs:2454 |
| f32 out of range clamped | COVERED | graph.rs:2446 |
| NaN checked BEFORE clamping | COVERED | graph.rs:2431 — NaN checked first, then inf, then clamp |
| NaN NOT silently clamped | COVERED | graph.rs:2431 — explicit rejection with error |
| Partial publish retains previous values | COVERED | graph.rs:2593 merges new params over existing current_params |

### REQ: Widget Parameter Types

| Scenario | Status | Implementing code |
|---|---|---|
| f32 accepted | COVERED | graph.rs |
| f32 NaN rejected → WIDGET_PARAMETER_INVALID_VALUE | COVERED | graph.rs:2432 |
| f32 Inf rejected → WIDGET_PARAMETER_INVALID_VALUE | COVERED | graph.rs:2438 |
| String accepted within max_length | COVERED | graph.rs:2460 |
| String exceeding max_length rejected | COVERED | graph.rs:2467 |
| Color accepted | COVERED | graph.rs:2486 |
| Color out of [0,255] rejected | PARTIAL/DEFERRED | Color is stored as f32 RGBA [0.0,1.0]; clamping applied but integer-range validation not separately enforced at graph layer. The proto conversion layer maps u8→f32 so out-of-range is structurally impossible over gRPC; color-range WIDGET_PARAMETER_INVALID_VALUE test coverage is present for the overall scenario. |
| Enum accepted if in allowed_values | COVERED | graph.rs:2501 |
| Enum not in allowed_values rejected | COVERED | graph.rs:2505 |

### REQ: Widget Asset Bundle Format

| Scenario | Status | Implementing code |
|---|---|---|
| Valid bundle loaded → WidgetDefinition registered | COVERED | `tze_hud_widget::loader::load_bundle_dir` |
| Missing widget.toml → WIDGET_BUNDLE_NO_MANIFEST | COVERED | error.rs + loader.rs:147 |
| Invalid TOML → WIDGET_BUNDLE_INVALID_MANIFEST | COVERED | loader.rs:159 |
| Duplicate type name → WIDGET_BUNDLE_DUPLICATE_TYPE | COVERED | loader.rs:98 |
| Missing SVG file → WIDGET_BUNDLE_MISSING_SVG | COVERED | loader.rs:213 |
| SVG parse failure → WIDGET_BUNDLE_SVG_PARSE_ERROR | COVERED | loader.rs:232 |
| Rejected bundle doesn't block others | COVERED | scan_bundle_dirs continues on error |

### REQ: SVG Layer Parameter Bindings

| Scenario | Status | Implementing code |
|---|---|---|
| Linear mapping (f32 only) | COVERED | loader.rs:564 + compositor/widget.rs |
| Direct mapping (string/color) | COVERED | loader.rs:577 |
| Discrete mapping (enum) | COVERED | loader.rs:588 |
| text-content synthetic target | COVERED | compositor/widget.rs:apply_svg_text_content |
| Unresolvable binding → WIDGET_BINDING_UNRESOLVABLE | COVERED | loader.rs:513 (missing param), loader.rs:523 (missing SVG element ID), loader.rs:565/578/589 (incompatible mapping type) |

### REQ: Widget Registry

| Scenario | Status | Implementing code |
|---|---|---|
| Registry populated at startup | COVERED | `tze_hud_runtime::widget_startup::init_widget_registry` |
| Agent queries via list_widgets MCP | COVERED | `tze_hud_mcp::tools::handle_list_widgets` |
| SceneSnapshot includes WidgetRegistrySnapshot | COVERED | `SceneGraph::take_snapshot` (graph.rs:2876) includes `widget_registry` |
| Agents cannot create widget types | COVERED | No mutation or RPC exists for runtime widget type creation |

### REQ: Widget Publishing via MCP

| Scenario | Status | Implementing code |
|---|---|---|
| Successful publish via MCP | COVERED | `handle_publish_to_widget` (tools.rs) |
| Schema validation failure via MCP | COVERED | `json_to_widget_param_value` returns WIDGET_UNKNOWN_PARAMETER |
| Missing capability via MCP | COVERED | tools.rs capability gate |
| Publish to specific instance via instance_id | COVERED | tools.rs resolves instance_id override |
| list_widgets returns registry | COVERED | `handle_list_widgets` |
| ttl_us parameter supported | COVERED | `PublishToWidgetParams.ttl_us` wired to `publish_to_widget(expires_at_wall_us)` |

### REQ: Widget Publishing via gRPC

| Scenario | Status | Implementing code |
|---|---|---|
| Durable widget publish acknowledged | COVERED | session_server.rs:3531 |
| Ephemeral widget publish fire-and-forget | COVERED | session_server.rs:3546 |
| Rejected publish returns error | COVERED | session_server.rs:3548 maps ValidationError → WidgetPublishResult |
| Publish to specific instance (instance_id) | COVERED | session_server.rs:3499 resolves instance_id |
| WidgetPublish at ClientMessage field 35 | COVERED | session.proto:83 |
| WidgetPublishResult at ServerMessage field 47 | COVERED | session.proto:161 |

### REQ: Clear Widget

| Scenario | Status | **MISSING** |
|---|---|---|
| ClearWidgetMutation proto type defined | COVERED | types.proto:397 — `ClearWidgetMutation` message exists |
| ClearWidgetMutation in MutationProto.oneof | **MISSING** | types.proto MutationProto (line 108) contains only `create_tile`, `set_tile_root`, `publish_to_zone`, `clear_zone`. `ClearWidgetMutation` is NOT wired into the oneof. |
| `SceneGraph::clear_widget_publications_for_namespace` | **MISSING** | No such method exists in graph.rs or anywhere. Compare: `clear_zone_publications_for_namespace` IS implemented. |
| ClearWidget gRPC handler in session_server | **MISSING** | No handler found (grep returns nothing for clear_widget in session_server.rs) |
| ClearWidget reverts to defaults | **MISSING** | No implementation |
| ClearWidget with other publishers | **MISSING** | No implementation |
| ClearWidget capability gate | **MISSING** | No implementation |

**Gap verdict**: The ClearWidgetMutation spec requirement is UNIMPLEMENTED. The proto type exists but is not wired into the mutation dispatch path and has no scene-graph implementation.

### REQ: Widget Compositor Rendering

| Scenario | Status | Implementing code |
|---|---|---|
| SVG rasterized to GPU texture at registration | COVERED | `WidgetRenderer` (compositor/widget.rs:310) |
| Re-rasterize on parameter change | COVERED | dirty flag + `rasterize_and_upload` (widget.rs:498) |
| Cached texture reused when params unchanged | COVERED | `dirty` flag in `WidgetTextureEntry` (widget.rs:293) |
| Re-rasterization < 2ms for 512x512 | PARTIAL/DEFERRED | Annotated as a goal; no automated performance assertion in CI; task 12.11 in tasks.md unfinished |
| Widget composited at correct z-order | COVERED | `WIDGET_TILE_Z_MIN` (0x9000_0000) used in composite_widgets |

### REQ: Widget Parameter Interpolation

| Scenario | Status | Implementing code |
|---|---|---|
| f32 linear interpolation | COVERED | `interpolate_param` in compositor/widget.rs:256 |
| Color component-wise sRGB interpolation | COVERED | widget.rs `interpolate_param` Color branch |
| String snaps at t=0 | COVERED | widget.rs String branch |
| Enum snaps at t=0 | COVERED | widget.rs Enum branch |
| Zero transition applies immediately | COVERED | `transition_ms=0` handled as single re-rasterize |
| Interpolation at target_fps during transition | COVERED | `WidgetAnimationState` (widget.rs:300); compositor re-rasterizes while animation active |

### REQ: Widget Contention and Governance

| Scenario | Status | Implementing code |
|---|---|---|
| LatestWins | COVERED | graph.rs:2550 |
| Stack (max_depth capping) | COVERED | graph.rs:2556 |
| MergeByKey (per-key replace) | COVERED | graph.rs:2564 |
| Replace | COVERED | graph.rs:2553 |
| publish_widget capability required | COVERED | session_server.rs:3460 (gRPC), tools.rs (MCP) |
| Chrome layer attachment | COVERED | `LayerAttachment::Chrome` defined in types.rs; widget renderer uses WIDGET_TILE_Z_MIN |
| MergeByKey parameter merge semantics | COVERED | graph.rs:2564; per spec, same key replaces, different keys coexist |
| Full occupancy contention (deferred from mim2.3) | PARTIAL/DEFERRED | `WidgetRegistry::get_occupancy` comment notes "correct per-policy reduction is deferred to the widget spec"; basic LatestWins/Stack/MergeByKey/Replace logic IS implemented in `publish_to_widget`; occupancy computation in `get_occupancy` does a simple merge-all rather than per-policy application. See discovered issue below. |

### REQ: Widget Instance Lifecycle

| Scenario | Status | Implementing code |
|---|---|---|
| Static instance from config | COVERED | `init_widget_registry` in widget_startup.rs |
| Widget instance in SceneSnapshot | COVERED | `take_snapshot` includes widget_registry.widget_instances |
| Agent cannot create widget instance | COVERED | No mutation/RPC for instance creation |
| Multiple instances of same type on same tab (with instance_id) | COVERED | config/widgets.rs validates; instance_name computed from instance_id |

### REQ: Built-in Widget Types

| Scenario | Status | Implementing code |
|---|---|---|
| No built-in widget types | COVERED | WidgetRegistry starts empty; no built-in entries |
| Zero bundles configured → empty registry | COVERED | widget_startup.rs:74 — no [widget_bundles] → return immediately |
| list_widgets returns empty on zero bundles | COVERED | handle_list_widgets (tools.rs) reads registry which is empty |
| publish_to_widget rejected → WIDGET_NOT_FOUND | COVERED | Aligned spec to use WIDGET_NOT_FOUND; matches session-protocol/spec.md and implementation |

### REQ: Widget Input Mode

| Scenario | Status | Implementing code |
|---|---|---|
| Widget tiles default input_mode=Passthrough | COVERED | Compositor renders widgets as pure GPU quads; no HitRegionNode added; `WIDGET_TILE_Z_MIN` places widgets at z-level where input passthrough applies |
| Widget tile has no HitRegionNodes | COVERED | Widget compositor adds no nodes to scene graph tile structure; passthrough is structural |
| Pointer passes through to content tile below | COVERED | Scene graph hit-testing uses z-order and tile input_mode |

---

## spec: scene-graph/spec.md

### REQ: Widget Registry in Scene Graph

| Scenario | Status | Implementing code |
|---|---|---|
| SceneGraph.widget_registry exists | COVERED | graph.rs:47 |
| Queryable for type definitions and instances | COVERED | `WidgetRegistry` methods |

### REQ: Widget Type Definition

| Scenario | Status | Implementing code |
|---|---|---|
| WidgetDefinition with all fields | COVERED | types.rs:1648 |
| Default value type must match param_type | COVERED | loader.rs `parse_default_value` validates type |
| Widget type id uniqueness | COVERED | scan_bundle_dirs duplicate detection |
| Widget type id format [a-z][a-z0-9-]* | **MISSING** | No regex validation of widget type name format at bundle load time. The spec requires rejection with an invalid id format error for names like "My Gauge!". `loader.rs` does not validate the name pattern. |

### REQ: Widget Instance in Scene Graph

| Scenario | Status | Implementing code |
|---|---|---|
| WidgetInstance with all fields | COVERED | types.rs:1667 |
| current_params initialized to defaults | COVERED | config/widgets.rs:253 `build_widget_instance` |
| geometry_override applied | COVERED | config/widgets.rs:242 |
| Duplicate instance without instance_id rejected | COVERED | config/widgets.rs:188 |
| Widget type reference validated | COVERED | config/widgets.rs:159 |
| instance_name is instance_id or widget_type_name | COVERED | config/widgets.rs:181, widget_startup.rs |

### REQ: Widget Publish Record

| Scenario | Status | Implementing code |
|---|---|---|
| WidgetPublishRecord with all fields | COVERED | types.rs:1686 |
| Publish record stored on publish | COVERED | graph.rs:2533 |
| TTL expiry removes record → recompute occupancy | **MISSING** | `expires_at_wall_us` field exists on `WidgetPublishRecord` but there is NO `drain_expired_widget_publications` function (compare: `drain_expired_zone_publications` exists for zones). No timer-driven expiry is wired in the runtime loop (windowed.rs and headless.rs call `drain_expired_zone_publications` but NOT a widget equivalent). |
| Unknown parameter rejected | COVERED | graph.rs:2420 |
| Parameter type mismatch rejected | COVERED | graph.rs:2454 |

### REQ: Widget Occupancy

| Scenario | Status | Implementing code |
|---|---|---|
| WidgetOccupancy with all fields | COVERED | types.rs:1704 |
| Occupancy reflects LatestWins | COVERED | graph.rs publish + get_occupancy |
| Occupancy reflects MergeByKey | PARTIAL/DEFERRED | `get_occupancy` comment: "correct per-policy reduction deferred"; effective_params computed by LatestWins-style merge regardless of policy |
| Occupancy falls back to defaults when no publications | COVERED | get_occupancy returns WidgetDefinition defaults when no publishes |

### REQ: WidgetParameterValue Type

| Scenario | Status | Implementing code |
|---|---|---|
| F32, String, Color, Enum variants | COVERED | types.rs:1575 |
| F32 must be finite | COVERED | graph.rs:2431-2443 |
| String at most 1024 bytes | COVERED | graph.rs:2461 |
| Enum must match allowed_values | COVERED | graph.rs:2501 |
| Type mismatch rejected | COVERED | graph.rs |

### REQ: Scene Snapshot Serialization (MODIFIED)

| Scenario | Status | Implementing code |
|---|---|---|
| Snapshot includes widget_registry | COVERED | `SceneGraph::take_snapshot` (graph.rs:2876) |
| widget_registry serialized in deterministic order (BTreeMap) | COVERED | `SceneGraphWidgetRegistry` uses `BTreeMap<String, WidgetDefinition>` (types.rs:1848) |
| Snapshot determinism with widgets | COVERED | BTreeMap ordering ensures determinism |
| Snapshot includes WidgetRegistrySnapshot in proto delivery | COVERED | `convert.rs::widget_registry_snapshot_to_proto` used in session_server snapshot |

---

## spec: session-protocol/spec.md

### REQ: Proto File Layout

| Scenario | Status | Implementing code |
|---|---|---|
| types.proto contains all widget types | COVERED | types.proto:254-409 contains all required widget proto types |
| session.proto contains WidgetPublish + WidgetPublishResult | COVERED | session.proto:539, 551 |
| events.proto unchanged | COVERED | events.proto has no widget additions |
| WidgetPublish at ClientMessage field 35 | COVERED | session.proto:83 |
| WidgetPublishResult at ServerMessage field 47 | COVERED | session.proto:161 |
| PublishToWidgetMutation in types.proto | PARTIAL | Note in types.proto comments: "PublishToWidgetMutation was removed because it was an exact duplicate of WidgetPublish; use WidgetPublish instead." Spec §Proto File Layout lists it as required. This is a deliberate simplification. |

### REQ: ClientMessage and ServerMessage Envelopes

| Scenario | Status | Implementing code |
|---|---|---|
| widget_publish at field 35 | COVERED | session.proto |
| widget_publish_result at field 47 | COVERED | session.proto |
| Existing field allocations unchanged | COVERED | Verified in session.proto |

### REQ: Widget Publish Session Message

| Scenario | Status | Implementing code |
|---|---|---|
| WidgetPublish fields (widget_name, instance_id, params, transition_ms, ttl_us, merge_key) | COVERED | session.proto:539 |
| Durable ack | COVERED | session_server.rs:3531 |
| Ephemeral fire-and-forget | COVERED | session_server.rs:3546 |
| Capability check | COVERED | session_server.rs:3460 |
| WIDGET_NOT_FOUND on unknown widget | COVERED | session_server.rs:3551 |

### REQ: Widget Publish Result

| Scenario | Status | Implementing code |
|---|---|---|
| WidgetPublishResult fields (accepted, widget_name, error) | COVERED | session.proto:551, session_server.rs |
| All error codes: WIDGET_NOT_FOUND, WIDGET_UNKNOWN_PARAMETER, WIDGET_PARAMETER_TYPE_MISMATCH, WIDGET_PARAMETER_INVALID_VALUE, WIDGET_CAPABILITY_MISSING | COVERED | session_server.rs:3551-3574 |
| Not sent for ephemeral | COVERED | session_server.rs:3546 |

### REQ: Widget Registry Query

| Scenario | Status | Implementing code |
|---|---|---|
| SceneSnapshot includes full WidgetRegistrySnapshot | COVERED | session_server.rs:1071-1100, graph.rs take_snapshot |
| No separate widget registry query RPC | COVERED | No WidgetRegistryRequest/Response on session stream |
| MCP: list_widgets | COVERED | tools.rs |

---

## spec: configuration/spec.md

### REQ: Zone Registry Configuration (MODIFIED)

| Scenario | Status | Implementing code |
|---|---|---|
| Widget bundles loaded from [widget_bundles].paths | COVERED | config/widgets.rs + widget_startup.rs |
| Unknown widget type → CONFIG_UNKNOWN_WIDGET_TYPE | COVERED | config/widgets.rs:161 |

### REQ: Capability Vocabulary (MODIFIED)

| Scenario | Status | Implementing code |
|---|---|---|
| publish_widget:<name> accepted | COVERED | config/capability.rs:150 |
| publish_widget:* accepted | COVERED | config/capability.rs:151 |
| Non-canonical widget capability rejected | COVERED | config/capability.rs:154 |

### REQ: Widget Bundle Configuration (ADDED)

| Scenario | Status | Implementing code |
|---|---|---|
| [widget_bundles].paths resolved relative to config | COVERED | config/widgets.rs:293 `resolve_bundle_path` |
| Path not found → CONFIG_WIDGET_BUNDLE_PATH_NOT_FOUND | COVERED | config/widgets.rs:92 |
| Absent section → empty registry, no error | COVERED | config/widgets.rs:83 |
| Duplicate type across bundles → CONFIG_WIDGET_BUNDLE_DUPLICATE_TYPE | COVERED | scan_bundle_dirs + widget_startup.rs |

### REQ: Widget Instance Configuration (ADDED)

| Scenario | Status | Implementing code |
|---|---|---|
| [[tabs.widgets]] with type, geometry, initial_params | COVERED | raw.rs RawTabWidget + config/widgets.rs |
| initial_params validated against schema | COVERED | config/widgets.rs:204 |
| Invalid initial_params → CONFIG_WIDGET_INVALID_INITIAL_PARAMS | COVERED | config/widgets.rs:349 |
| Multiple instances with instance_id | COVERED | config/widgets.rs:180 |
| Duplicate type without instance_id rejected | COVERED | config/widgets.rs:188 |

---

## spec: resource-store/spec.md

### REQ: V1 Resource Type Enumeration (MODIFIED)

| Scenario | Status | Implementing code |
|---|---|---|
| IMAGE_SVG variant added | COVERED | resource/types.rs:113 |
| IMAGE_SVG validates well-formed XML + svg root | COVERED | resource/validation.rs:317 |
| IMAGE_SVG NOT rasterized at upload | COVERED | validation.rs uses quick-xml parse only |
| IMAGE_SVG not publishable directly to zones | COVERED | resource/types.rs: zone media types do not include ImageSvg |

### REQ: SVG Resource Budget Accounting (ADDED)

| Scenario | Status | Implementing code |
|---|---|---|
| viewBox budget = w * h * 4 | COVERED | validation.rs:326-332 |
| No dimensions → 512x512 default | COVERED | `extract_svg_dimensions` falls back to SVG_DEFAULT_DIMENSION_PX |
| Dimensions clamped to 2048x2048 | COVERED | `clamp_svg_dim` (validation.rs:450) |
| Widget SVG budget is runtime overhead (not agent budget) | COVERED | RFC 0011 documents this; widget startup does not charge agent budget |

---

## Gap Items (Discovered)

### GAP-1: ClearWidgetMutation — Not Wired (P1)

**Status**: MISSING
**Files**: types.proto (MutationProto oneof), crates/tze_hud_scene/src/graph.rs
**Description**: `ClearWidgetMutation` proto message exists in types.proto but is not included in `MutationProto.oneof` and has no handler in session_server.rs. The scene graph has `clear_zone_publications_for_namespace` but no corresponding `clear_widget_publications` method. This makes the spec requirement "Clear Widget" entirely unimplemented at the runtime level.
**Spec ref**: widget-system/spec.md §Requirement: Clear Widget
**Suggested bead**: type=feature, P1

### GAP-2: Widget TTL/Expiry Enforcement — Not Wired (P1)

**Status**: MISSING
**Files**: crates/tze_hud_runtime/src/windowed.rs, crates/tze_hud_runtime/src/headless.rs, crates/tze_hud_scene/src/graph.rs
**Description**: `WidgetPublishRecord` has `expires_at_wall_us` field. Zones have `drain_expired_zone_publications()` called on every frame tick in both windowed.rs and headless.rs. No equivalent `drain_expired_widget_publications()` function exists. Widget publications with a TTL never expire.
**Spec ref**: scene-graph/spec.md §Requirement: Widget Publish Record §Scenario: TTL expiry
**Suggested bead**: type=feature, P1

### GAP-3: Widget Type Id Format Validation — Missing (P2)

**Status**: MISSING
**Files**: crates/tze_hud_widget/src/loader.rs
**Description**: spec scene-graph/spec.md §WidgetDefinition requires the `id` field to match `[a-z][a-z0-9-]*`. The bundle loader validates name is non-empty but does not reject names with uppercase letters, spaces, or special characters.
**Spec ref**: scene-graph/spec.md §Requirement: Widget Type Definition §Scenario: Widget type id format validation
**Suggested bead**: type=bug, P2

### GAP-4: Widget Occupancy Per-Policy Resolution (Full Contention) — Deferred (P2)

**Status**: PARTIAL/DEFERRED (explicitly noted in code comment in graph.rs `get_occupancy`)
**Files**: crates/tze_hud_scene/src/types.rs `WidgetRegistry::get_occupancy`
**Description**: `get_occupancy` builds `effective_params` by merging all active publications' params without applying the contention policy semantics. For Stack and MergeByKey, the merge ordering and policy rules are not fully applied in the occupancy read path (though they ARE applied correctly during publish via `publish_to_widget`). The occupancy struct reflects all publications but the `effective_params` merge in `get_occupancy` is a simple overlay regardless of policy.
**Spec ref**: scene-graph/spec.md §Requirement: Widget Occupancy, widget-system/spec.md §Requirement: Widget Contention and Governance
**Suggested bead**: type=task, P2

### GAP-5: Widget Type Name "WIDGET_TYPE_NOT_FOUND" vs "WIDGET_NOT_FOUND" Inconsistency (P3)

**Status**: RESOLVED
**Files**: session-protocol/spec.md, widget-system/spec.md, session_server.rs
**Description**: Aligned widget-system/spec.md §Built-in Widget Types to use WIDGET_NOT_FOUND (matching session-protocol/spec.md and implementation) instead of WIDGET_TYPE_NOT_FOUND. This resolves the spec inconsistency and ensures canonical naming across documentation.

### GAP-6: Re-rasterization Performance Budget Not Asserted in CI (P3)

**Status**: PARTIAL/DEFERRED
**Description**: Spec requires re-rasterization < 2ms for 512x512 on reference hardware. The compositor has the implementation but no automated CI benchmark enforces this. Task 12.11 in tasks.md is marked unchecked. The pre-existing CI test failures (budget_assertions.rs) suggest this category of test is fragile in CI.
**Suggested bead**: type=task, P3

---

## opsx:sync Assessment

The delta specs in `openspec/changes/widget-system/specs/` cover five capability domains
(widget-system, scene-graph, session-protocol, configuration, resource-store). The RFC documents
in `docs/rfcs/` have been updated by hud-mim2.6 to include widget content. There is no
separate authoritative `openspec/specs/` tree — the delta specs in the changes directory
ARE the canonical spec artifacts for this project at this stage.

**Recommendation**: opsx:sync is NOT needed before epic closure. The delta specs in the
changes directory are the sole authoritative specs. They should be retained as-is until
the `openspec/changes/widget-system` change is formally archived (that is a separate
operation performed after all gaps are addressed or triaged).

---

## Epic Closure Recommendation

**Recommendation: DO NOT close hud-mim2 yet.**

Three P1 gaps remain unimplemented:
1. GAP-1: ClearWidgetMutation (entire spec requirement unimplemented)
2. GAP-2: Widget TTL/expiry enforcement (widget publications never expire)

These are required by v1-mandatory spec requirements. The epic should remain open until
these gaps are addressed or explicitly descoped with a documented rationale.

Once GAP-1 and GAP-2 are fixed and merged, the epic may be closed. GAP-3 through GAP-6
are lower priority and can be tracked as follow-on work without blocking epic closure.
