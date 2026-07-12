# scene-graph Delta: Node layout mode + struct-overhead budget

## MODIFIED Requirements

### Requirement: Struct Overhead Budgets
Per-tile struct overhead MUST be < 200 bytes (excluding texture data and nodes). Per-node struct overhead MUST be < 160 bytes (excluding content payloads). At maximum capacity (64 nodes per tile), total structural overhead per tile MUST be approximately 9.9 KB (tile struct + 64 node structs, content excluded).

The per-node budget was raised from 150 to 160 bytes (hud-yfj8u) when the additive `layout: NodeLayout` field — the vertical-flow layout mode governing how a node positions its children — grew `Node` from 144 to 152 bytes: one enum discriminant byte plus 8-aligned padding, because `Node` sat exactly on an 8-byte boundary with no reclaimable trailing padding. The field is additive and default (`NodeLayout::Absolute`) preserves the historical single-child-per-explicit-bounds layout, so the growth buys a real capability (runtime-resolved child stacking) at ~1.3% per-node overhead and a negligible ~0.1 KB per fully-loaded tile.

Source: RFC 0001 §8, §10, hud-yfj8u (NodeLayout field)
Scope: v1-mandatory

#### Scenario: Tile struct size
- **WHEN** `size_of::<Tile>()` plus metadata allocation is measured
- **THEN** it MUST be less than 200 bytes

#### Scenario: Node struct size
- **WHEN** `size_of::<Node>()` plus ID allocation is measured
- **THEN** it MUST be less than 160 bytes
