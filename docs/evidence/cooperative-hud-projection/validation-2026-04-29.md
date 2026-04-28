# Cooperative HUD Projection Validation Evidence

Date: 2026-04-29
Bead: `hud-ggntn.5`
Worker branch: `agent/hud-ggntn.5`

## Scope

This evidence covers local/unit and headless integration validation for `openspec/changes/cooperative-hud-projection/tasks.md` items 4.1, 4.2, 4.3, 4.5, 4.6, and 4.7.

Live Windows HUD validation for task 4.4 was not executed in this worker. It remains pending for a `/user-test` pass on the visible Windows overlay because this worker only ran local/headless gates.

## Coverage Matrix

| Requirement area | Evidence |
| --- | --- |
| Operation schema, bounds, idempotency, authorization, audit fields, stable error codes | `cargo test -p tze_hud_projection`; existing and extended tests in `crates/tze_hud_projection/src/lib.rs` cover required request fields, stable error-code strings, attach conflict/idempotency, oversized output/input, cross-projection denial, owner/operator cleanup, bounded audit records, and token expiry. |
| Daemon lifecycle: transcript retention, pending input acknowledgement, reconnect, stale lease, heartbeat, detach, cleanup | `cargo test -p tze_hud_projection`; added coverage for monotonic heartbeat recording requiring a live HUD connection, reconnect preserving transcript and delivered inbox state while dropping advisory leases, stale/overbroad lease denial, detach removing projected portal state, and cleanup authority separation. |
| Property/state-machine coverage | `cargo test -p tze_hud_projection`; added property test `lifecycle_state_machine_never_reuses_stale_connection_or_lease` plus existing FIFO/bounded pending-input property coverage. |
| No PTY/tmux/terminal capture/provider-specific runtime authority | `cargo test -p integration --test text_stream_portal_adapter`; added `cooperative_projection_runtime_surface_is_provider_neutral_and_process_agnostic`, and existing tests prove both tmux and non-tmux adapters use the same generic bridge while cooperative projection state exposes only `CooperativeProjection` + `ResidentSessionLease` authority. |
| Bounded polling/output rate and local-first input feedback | `cargo test -p tze_hud_projection`; existing tests cover bounded polling responses, input queue full behavior, local accepted/rejected portal input feedback, transcript pruning, and portal update-rate coalescing. |
| Live attach -> output -> HUD input -> poll/ack -> collapse/restore -> detach cleanup | Pending live Windows `/user-test`; not run in this worker. Local/headless coverage exercises the same authority/inbox/portal-state contracts without a visible overlay. |

## Commands Run

```bash
cargo fmt --check
```

Result: passed.

```bash
cargo test -p tze_hud_projection
```

Result: passed. Final run: 34 tests passed, 0 failed, 0 ignored; doc-tests passed with 0 tests.

```bash
cargo test -p integration --test text_stream_portal_adapter
```

Result: passed. Final run: 6 tests passed, 0 failed, 0 ignored.

## Residual Risks

Live Windows HUD validation remains pending. The local and headless integration tests validate operation semantics, authority boundaries, lifecycle state, and portal adapter constraints, but they do not prove visible overlay behavior for collapse/restore, drag/reposition, or human-visible cleanup on Windows.
