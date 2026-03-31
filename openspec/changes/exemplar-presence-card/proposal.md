## Why

The tile API exists for genuinely custom agent layouts that no zone or widget covers — but there is no reference exemplar proving the end-to-end tile lifecycle: lease acquisition, scene mutation batches, multi-agent coexistence, periodic content updates, and orphan handling on disconnect. The presence card is the simplest non-trivial tile use case: a small fixed-position identity card per agent (avatar icon + name + status text), display-only, no input. It exercises multi-agent tile stacking (3 agents, non-overlapping vertical offsets), lease renewal, periodic scene mutations for "last active" timestamps, and the full orphan-to-staleness-badge pipeline when one agent disconnects. Without this exemplar, implementers have no canonical target for the raw tile path and testers have no fixture for multi-agent lease lifecycle validation.

## What Changes

- Define an **exemplar-presence-card specification** covering the full tile-based agent presence card: fixed-size tile (200x80), corner-anchored, with SolidColorNode background, StaticImageNode avatar (32x32), and TextMarkdownNode for agent name + last-active status
- Define **multi-agent coexistence layout**: 3 agents each create a presence card tile in the same tab, vertically stacked in the bottom-left corner with 8px gaps, non-overlapping (agent A at y=offset, agent B at y=offset+88, agent C at y=offset+176)
- Define **lease lifecycle scenarios**: lease request with AUTO_RENEW policy, periodic content mutation (update "last active" timestamp every 30s), lease renewal, and agent disconnect → ORPHANED state → disconnection badge → grace period expiry → tile removal
- Define **gRPC test sequences**: concrete MutationBatch payloads for tile creation, node insertion, and content update — exercising CreateTile, InsertNode (3 node types), and ReplaceNode (text update)
- Define a **user-test scenario**: 3 agents create presence cards, one disconnects, visual verification of staleness badge and eventual cleanup

## Capabilities

### New Capabilities
- `exemplar-presence-card`: Tile-based agent presence card exemplar defining the visual layout, multi-agent coexistence rules, lease lifecycle scenarios, gRPC test sequences, and user-test integration for raw tile path validation

### Modified Capabilities

(none — this exemplar defines test/validation artifacts for existing scene-graph and lease-governance capabilities, it does not change spec-level requirements)

## Impact

- **Test sequences**: New gRPC MutationBatch test payloads for tile creation, node insertion, and periodic content update
- **User-test workflow**: Presence-card-specific multi-agent test scenario added to validation framework
- **Implementer guidance**: Concrete tile lifecycle targets — the canonical reference for how a raw tile agent operates (lease → create tile → insert nodes → update content → renew lease → disconnect → orphan → cleanup)
- **No code changes**: This exemplar defines the target — implementation is driven by existing scene-graph and lease-governance specs
