## Context

This bead is docs/spec/RFC reconciliation only. It resolves contract seams discovered before any implementation of persistent movable elements.

## Decisions

### 1) v1 persistence carve-out includes element identity store
The v1 boundary keeps scene graph state ephemeral, but adds a durable carve-out for an element identity store that contains user geometry overrides and element metadata. This is user-owned layout preference data, not agent scene content.

### 2) Chrome drag handles use compositor-internal interaction path
`LongPress` and `Drag` remain `V1-reserved` in RFC 0004 for the general agent gesture recognizer pipeline. Chrome drag handles are explicitly permitted to implement gesture-like behavior in a compositor-internal state machine outside that pipeline.

For drag-handle activation delay:
- Pointer/mouse: 250ms hold
- Touch: 1000ms hold

This preserves strict v1 scope while allowing usable drag-handle UX and reducing false touch activation.

### 3) Capture timing carve-out for runtime chrome
The `PointerDown` capture-acquire restriction in RFC 0004 applies to agent-requested capture semantics. Runtime-owned chrome interactions may acquire and release capture at any event phase when required for sovereign runtime UX control.

### 4) V1-compatible drag feedback only
Drag feedback uses only:
- temporary z-order boost,
- highlight border,
- immediate (non-animated) opacity state changes.

No drop shadows, no scale pulses, no eased transitions.

Clarification: long-press progress fill (0.0→1.0) is a state-derived value over elapsed hold time, not a deferred transition effect.

### 5) Mobile reset gesture avoids long-press conflict
Reset is not triggered by a second long-press gesture. Instead, a short tap on drag handle reveals a temporary reset affordance (3-second auto-dismiss), avoiding conflict with long-press drag activation.

### 6) `PublishToTileMutation` is additive, lease-gated, and fielded
`PublishToTileMutation` coexists with `SetTileRootMutation`:
- `SetTileRootMutation`: raw `tile_id` mutation path.
- `PublishToTileMutation`: element-store-addressed tile publication path that resolves stable element identity and applies runtime geometry overrides.

Mutation is lease-gated: publisher must hold an active lease for the resolved tile namespace. Proto oneof allocates the next available field number.

### 7) Runtime override application is explicit pipeline stage
RFC 0001 transaction pipeline gains a `Runtime Override Application` stage between validation and commit to apply user geometry overrides before atomic commit.

### 8) Factual identity correction
The old statement "ZoneDefinition gets an ID" is incorrect. Zone definitions already carry `SceneId`. This change instead makes identity persistent and extends persistent identity coverage to `ZoneInstance` and `WidgetInstance`.

### 9) Deletion deferred post-v1
Element-store explicit user deletion is deferred. In v1, store growth is monotonic. A future layout-management UI will provide deletion/cleanup controls.
