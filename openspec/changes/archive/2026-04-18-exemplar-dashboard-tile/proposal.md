## Why

The v1 MVP specifications define four node types (SolidColorNode, TextMarkdownNode, StaticImageNode, HitRegionNode), atomic batch mutations, lease governance, z-order compositing, and input capture — but there is no concrete, end-to-end exemplar that proves the full raw tile API works together. Without a reference exemplar, implementors have to reverse-engineer the interaction between leases, scene mutations, input routing, and intra-tile compositing from scattered spec sections. An exemplar dashboard tile — a polished, multi-node, agent-owned tile with interactive buttons, live content updates, and lease lifecycle management — serves as both a validation target and a developer reference.

## What Changes

- Define a concrete **exemplar dashboard tile**: a 400x300 agent-owned tile at content layer depth demonstrating all four v1 node types composited in tree order (SolidColorNode background, StaticImageNode icon, TextMarkdownNode header/body, HitRegionNode buttons).
- Specify the **full gRPC lifecycle**: session establishment, lease request with TTL and `AutoRenew` policy, atomic batch creation of tile + all nodes, periodic TextMarkdownNode content updates, HitRegionNode interaction callbacks, and graceful lease release/dismiss.
- Define **input capture behavior**: HitRegionNode buttons with local pressed/hovered state (runtime-owned, < 4ms), focus semantics, event_mask configuration, and agent callback on ACTIVATE (pointer click or command input).
- Define **lease governance integration**: TTL-based lease with auto-renewal at 75%, orphan handling with disconnection badge on disconnect, lease expiry leading to tile removal.
- Specify **test integration points**: headless Layer 0 tests for tile creation + node composition, input event injection for hit region verification, lease expiry simulation, and a full user-test lifecycle scenario.

## Capabilities

### New Capabilities

- `exemplar-dashboard-tile`: End-to-end exemplar defining a polished interactive dashboard tile that exercises all four v1 node types, atomic batch mutations, lease governance (request/renew/orphan/expire), z-order compositing at content layer, input capture with local feedback, and agent callbacks. Serves as the reference implementation target for the raw tile API.

### Modified Capabilities

(none — this exemplar exercises existing capabilities without changing their requirements)

## Impact

- **Test harness**: New integration tests exercising the full gRPC session path: connect, lease, mutate, interact, disconnect.
- **Existing specs**: No spec changes. This exemplar is a consumer of scene-graph, lease-governance, input-model, and session-protocol specs.
- **Developer experience**: Provides a copy-paste reference for any agent building interactive tiles with the raw API.
- **Validation**: Proves that the four node types, atomic mutations, lease state machine, and input routing compose correctly in a single concrete scenario.
