# Resources, Zones, Widgets, and Publishing Surfaces

- Estimated smart-human study time: 5 hours
- Keep every module at or below 10 hours.

## Why This Module Matters

Much of `tze_hud` exists to keep agents from doing low-level layout and rendering work. To understand how the repo delivers that, you need both the content-addressed asset model and the runtime-owned publishing abstractions layered on top of it.

## Learning Goals

- Explain why uploaded resources use immutable content identity.
- Distinguish raw tiles, zones, and widgets as three different abstraction levels.
- Understand the two-stage asset registration then publish model.

## Subsection: Asset Identity and Managed Publishing

### Why This Matters Here

Without this model, the repo would look like a random mix of upload code, MCP tools, and SVG handling. In reality, those pieces support a consistent design goal: agents declare intent or typed values, and the runtime owns the visual realization.

### Technical Deep Dive

Content-addressed storage solves two problems at once: deduplication and identity. If two uploads have the same bytes, they should share a `ResourceId`; otherwise caches, references, and storage accounting become harder to reason about.

On top of that, the repo offers three display abstractions:
- raw tiles for custom, lease-driven composition
- zones for runtime-owned semantic publishing surfaces
- widgets for runtime-owned parameterized visuals

The key concept is increasing abstraction while preserving runtime authority. Zones hide geometry and rendering policy. Widgets hide visual-template logic while still allowing expressive parameter updates. Both rely on asset registration that is separate from publish-time traffic so the hot path stays small and typed.

### Where It Appears In The Repo

- `openspec/specs/resource-store/spec.md`
- `about/heart-and-soul/presence.md`
- `about/heart-and-soul/v1.md`
- `openspec/specs/widget-system/spec.md`
- `crates/tze_hud_mcp/src/tools.rs`
- `tests/integration/presence_card_tile.rs`

### Sample Q&A

- Q: Why does the repo use BLAKE3-based `ResourceId` instead of path-based asset identity?
  A: Because identity follows immutable content, enabling deduplication and stable references regardless of upload path or caller.
- Q: When should an agent use a zone or widget instead of a raw tile?
  A: When the runtime already offers a managed surface or parameterized visual that matches the use case, because that preserves governance and keeps layout/render logic out of the agent.

### Progress

- [ ] Exposed: I can define `ResourceId`, zone, widget, and occupancy
- [ ] Working: I can explain the difference between the three publishing abstraction levels
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can explain why asset registration and publish are separate stages

### Mastery Check

Target level: `working`

You should be able to explain why resource identity is content-based and why the repo keeps most agents on zones/widgets instead of raw tile math.

## Module Mastery Gate

- [ ] I can summarize the asset identity model
- [ ] I can explain raw tiles vs zones vs widgets
- [ ] I can point to the MCP tool layer and at least one spec covering publishing
- [ ] I can explain why runtime-owned publishing reduces agent complexity

## What This Module Unlocks Next

It sets up the final module, where config, validation, telemetry, and safe contribution workflow turn these abstractions into day-to-day engineering practice.

