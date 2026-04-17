## Why

tze_hud already rejects the idea that the CLI, chat transcript, or generated webpage is the final form of agent interaction. But the project also rejects collapsing into a generic chat app or terminal host. What is missing is a governed middle ground: a low-latency text interaction surface that can stream output over time, accept bounded viewer input, and remain subject to the runtime's sovereignty, leases, redaction, safe mode, and override controls.

The key architectural mistake would be to define this feature around tmux. Tmux is one useful adapter, not the capability. The capability is a **text stream portal** whose runtime boundary is transport-agnostic input/output text streams plus session metadata. That boundary is broad enough to support tmux, human chat platforms, and LLM interactions without hardcoding any of them into the runtime.

## What Changes

- Define a new **text-stream-portals capability** describing governed portal surfaces for low-latency streaming text interaction
- Define a **transport-agnostic adapter boundary** for portal output streams, input submission, and session metadata
- Define a **phase-0 raw-tile pilot contract**: resident gRPC session, content-layer tile, existing node types, no terminal-emulator semantics, no chrome-hosted portal UI
- Generate **RFC 0013** to capture the cross-subsystem contract across scene, input, session, shell, and governance boundaries
- Generate a **direction report** documenting alignment, misalignment, blockers, and the explicit stop-before-beads decision

## Capabilities

### New Capabilities

- `text-stream-portals`: governed, low-latency streaming text interaction surfaces with transport-agnostic adapters and a raw-tile pilot path

### Modified Capabilities

- `scene-graph`: clarify that the phase-0 pilot is a raw-tile capability built from existing node types and content-layer constraints
- `input-model`: extend capability-level expectations for scrollable transcript interaction, focus, and reply submission
- `system-shell`: clarify that portal affordances remain outside chrome and must obey override, redaction, and shell isolation rules

## Impact

- Affects doctrine and design-contract documentation across `about/heart-and-soul/` and `about/legends-and-lore/`
- Adds a new OpenSpec capability spec and implementation task breakdown
- Does not yet implement runtime code or allocate beads
- Creates a reviewable contract for later implementation planning
