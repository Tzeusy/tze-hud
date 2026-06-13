## Why

The archived cooperative HUD projection work gives an already-running LLM session a bounded, daemon-owned way to project into a text-stream portal. The next missing v1 contract is the external authority above that surface: a provider-neutral launcher/attacher that can manage multiple LLM sessions, authenticate them to the local Windows HUD runtime, and route each session to existing governed HUD surfaces without moving session lifecycle or rendering responsibility into the compositor.

This is needed now because the Windows-first v1 target explicitly requires multiple agents to coexist on screen, while current projection code does not yet describe or implement launch/attach orchestration across zones, widgets, and leased portals.

## What Changes

- Add an external agent projection authority contract for provider-neutral launched and attached LLM sessions.
- Define Windows HUD target authentication metadata as authority-held configuration, with no secrets in audit records, docs, or scene state.
- Define governed presence routing from each managed session to one existing v1 surface class: zone publish, widget publish, or leased text-stream/raw-tile portal.
- Define multi-session lifecycle behavior for concurrent sessions, disconnect/reconnect bookkeeping, cleanup, revocation, and expiry.
- Keep runtime/compositor ownership unchanged: no LLM in the frame loop, no new compositor chrome, no PTY/terminal capture, and no provider-specific process authority inside runtime core.
- Add a vertical-slice demo shape with three concurrent provider-neutral sessions publishing distinct governed elements.

## Capabilities

### New Capabilities

- `external-agent-projection-authority`: External launcher/attacher and governed routing authority for multiple provider-neutral LLM sessions targeting the local Windows HUD runtime.

### Modified Capabilities

- `cooperative-hud-projection`: Extend the projection contract to allow an external authority to supervise multiple launched or attached sessions and route their presence to zones, widgets, or leased portals while preserving the existing cooperative operation and governance rules.

## Impact

- Affected code:
  - `crates/tze_hud_projection/src/lib.rs`
  - `crates/tze_hud_projection/src/bin/projection_authority.rs` if CLI exposure is needed for the vertical slice
  - `crates/tze_hud_projection/tests/`
- Affected specs:
  - new `external-agent-projection-authority` capability
  - delta to `cooperative-hud-projection`
- Affected validation:
  - focused cargo tests for the authority and three-session demo
  - live Windows `/user-test` evidence or a bead-recorded blocker

Runtime core, compositor node types, and provider CLIs should not need changes for the first vertical slice.
