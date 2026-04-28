# Cooperative Projection Resident gRPC Adapter Evidence

Date: 2026-04-29
Bead: `hud-ggntn.9`
Worker branch: `agent/hud-ggntn.9`

## Scope

This evidence covers GAP-2 from PR #624 / `hud-ggntn.7`: a daemon-side resident gRPC adapter that materializes cooperative projection state into the existing `HudSession` raw-tile text-stream portal path.

It does not add the LLM-facing daemon CLI/MCP/control surface from GAP-1, and it does not claim live Windows overlay governance coverage from GAP-3.

## Coverage Matrix

| Requirement area | Evidence |
| --- | --- |
| Attach creates or reuses a content-layer portal | `cooperative_projection_resident_grpc_adapter_drives_projected_portal_lifecycle` creates a resident tile on first materialization, records the returned tile ID, then reuses the same tile through `PublishToTile`. |
| HUD composer submission maps to semantic inbox | The same test calls `ResidentGrpcPortalAdapter::submit_composer_text`, verifies accepted local feedback within the input-feedback budget, then polls the item through `ProjectionAuthority::handle_get_pending_input`. |
| Collapse/restore | The test collapses via `ProjectionAuthority::collapse_projected_portal`, renders compact geometry, then expands via `expand_projected_portal` and verifies expanded composer affordance returns. |
| Drag/reposition or movable compact affordance | The adapter updates compact geometry through `move_compact_to`; the test renders collapsed state through resident gRPC and verifies the tile moved to the requested coordinates. |
| Detach/cleanup lease release | The test sends `LeaseRelease` through the same resident stream and verifies the scene has no stale projected tiles afterward. |
| Resident-path budget evidence | Adapter-local payload build is asserted against `RESIDENT_PORTAL_UPDATE_BUILD_BUDGET_US = 16_600`; composer feedback is asserted against `RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US = 4_000`. |
| No PTY/tmux/process lifecycle authority | The adapter only emits `HudSession` `MutationBatch` and `LeaseRelease` messages, and the test asserts serialized projection state does not expose PTY, tmux, terminal, stdin/stdout, spawn, kill, or process lifecycle authority. |

## Commands Run

```bash
cargo fmt --check
```

Result: passed.

```bash
cargo clippy -p tze_hud_projection --features resident-grpc -- -D warnings
```

Result: passed.

```bash
cargo clippy -p integration --test text_stream_portal_adapter --features headless -- -D warnings
```

Result: passed.

```bash
cargo test -p tze_hud_projection --features resident-grpc --lib
```

Result: passed. Final run: 34 passed, 0 failed.

```bash
cargo test -p integration --test text_stream_portal_adapter --features headless
```

Result: passed. Final run: 7 passed, 0 failed.
