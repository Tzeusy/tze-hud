# Text Stream Portals Validation Evidence (hud-t98e.4)

Date: 2026-04-16
Branch: `agent/hud-t98e.4`

## Automated Integration Evidence

Command:

```bash
cargo test -p integration --test text_stream_portal_surface --test text_stream_portal_coalescing --test text_stream_portal_governance
```

Result summary:

- `text_stream_portal_surface`: 6 passed, 0 failed
- `text_stream_portal_coalescing`: 3 passed, 0 failed
- `text_stream_portal_governance`: 9 passed, 0 failed

Verification rerun (focused acceptance targets):

```bash
cargo test -p integration --test text_stream_portal_coalescing --test text_stream_portal_governance -- --nocapture
```

Observed output:

- `text_stream_portal_coalescing`: 3 passed, 0 failed
- `text_stream_portal_governance`: 9 passed, 0 failed

## Coverage Delivered By This Bead

- Coalescing: rapid append + bounded-tail + intermediate-frame skip semantics
- Governance: lease expiry/revocation/orphan lifecycle, redaction, safe-mode suspend/resume, freeze backpressure genericity
- Interaction: expand/collapse visibility, local reply acknowledgement, scroll-authority persistence
- Ambient attention: backlog remains ambient and coalesces under budget pressure
- Shell isolation: shell status snapshot omits portal identity/transcript, shell dismiss removes tile

## User-Test Path Status

The acceptance path references a tmux adapter-driven `/user-test` flow. In this branch snapshot, adapter/protocol portal surfaces are not present.

Evidence commands:

```bash
rg --files examples | rg 'tmux_portal_adapter'
rg -n 'PortalOpen|PortalAppend|PortalInput|PortalControl|PortalEvent|PortalOpenResult' crates/tze_hud_protocol/proto/session.proto
```

Observed result:

- No `examples/tmux_portal_adapter.rs` match in `examples/`
- No portal protobuf message matches in `crates/tze_hud_protocol/proto/session.proto`

Interpretation:

- Automated validation coverage for portal semantics is now present and passing.
- Live tmux-adapter `/user-test` execution remains blocked in this branch state until adapter/proto portal surfaces are available.
