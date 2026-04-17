# scene-graph Specification

## Purpose
TBD - created by archiving change text-stream-portals. Update Purpose after archive.
## Requirements
### Requirement: Text Stream Portal Phase-0 Uses Raw Tiles

The `text-stream-portals` phase-0 pilot SHALL use agent-owned content-layer raw tiles with existing V1 node types. The pilot MUST NOT require a new scene node type before the capability is proven.

#### Scenario: portal pilot tile stays below runtime-managed bands

- **WHEN** a resident portal pilot creates its surface as a raw tile
- **THEN** the tile SHALL use the normal agent-owned z-order band and remain below zone-reserved and widget-reserved runtime-managed tiles

