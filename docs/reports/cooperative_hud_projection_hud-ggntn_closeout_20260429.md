# Cooperative HUD Projection Closeout Report

Date: 2026-04-29
Epic: `hud-ggntn`
Report bead: `hud-ggntn.6`

## Source Artifacts

- OpenSpec proposal: [`openspec/changes/cooperative-hud-projection/proposal.md`](../../openspec/changes/cooperative-hud-projection/proposal.md)
- OpenSpec design: [`openspec/changes/cooperative-hud-projection/design.md`](../../openspec/changes/cooperative-hud-projection/design.md)
- OpenSpec reconciliation: [`openspec/changes/cooperative-hud-projection/reconciliation.md`](../../openspec/changes/cooperative-hud-projection/reconciliation.md)
- Cooperative projection delta: [`openspec/changes/cooperative-hud-projection/specs/cooperative-hud-projection/spec.md`](../../openspec/changes/cooperative-hud-projection/specs/cooperative-hud-projection/spec.md)
- Text-stream portals delta: [`openspec/changes/cooperative-hud-projection/specs/text-stream-portals/spec.md`](../../openspec/changes/cooperative-hud-projection/specs/text-stream-portals/spec.md)
- Task ledger: [`openspec/changes/cooperative-hud-projection/tasks.md`](../../openspec/changes/cooperative-hud-projection/tasks.md)
- Validation evidence: [`docs/evidence/cooperative-hud-projection/validation-2026-04-29.md`](../evidence/cooperative-hud-projection/validation-2026-04-29.md)

## Implementation Summary

The epic delivered the first cooperative HUD projection tranche for already-running LLM sessions. The implementation stays inside the design boundary: it is cooperative opt-in, provider-neutral, external to runtime process ownership, and built around a projection authority that owns memory-only transcript, inbox, lifecycle, lease, audit, and portal-state bookkeeping.

Primary implementation landed in `crates/tze_hud_projection`, with packaging in `.claude/skills/hud-projection/`, `.gemini/skills/hud-projection/`, and `.opencode/skills/hud-projection/`. The text-stream portal integration proof is covered by `tests/integration/text_stream_portal_adapter.rs`. No runtime PTY, tmux, terminal-capture, or provider-specific process-control surface was added.

## PR And Commit Inventory

| PR | Commit | Status | Scope |
| --- | --- | --- | --- |
| [#617](https://github.com/Tzeusy/tze-hud/pull/617) | [`8dc99cc`](https://github.com/Tzeusy/tze-hud/commit/8dc99cc1d566bf1ea1209ba4e3225fc8970d3020) | Merged | Defined the projection operation contract and introduced `crates/tze_hud_projection`. |
| [#618](https://github.com/Tzeusy/tze-hud/pull/618) | [`c80def5`](https://github.com/Tzeusy/tze-hud/commit/c80def50c1af76ec582715d0f5eb0a23951de4f8) | Merged | Packaged `/hud-projection` skill instructions, examples, MCP facade notes, and mirrored tool skill surfaces. |
| [#619](https://github.com/Tzeusy/tze-hud/pull/619) | [`e0dc3e7`](https://github.com/Tzeusy/tze-hud/commit/e0dc3e79355c0dadecfa17b120b64eb31d856e00) | Merged | Built projection authority daemon semantics: retained state, inbox, lifecycle, audit, reconnect, cleanup, bounds, and authorization behavior. |
| [#620](https://github.com/Tzeusy/tze-hud/pull/620) | [`40a2e27`](https://github.com/Tzeusy/tze-hud/commit/40a2e273c1fcd2e7883fc27f939d163d12fb546c) | Merged | Wired projected sessions to text-stream portal state and added provider-neutral adapter integration coverage. |
| [#621](https://github.com/Tzeusy/tze-hud/pull/621) | [`92926d2`](https://github.com/Tzeusy/tze-hud/commit/92926d217981b15acf2043c6bc5b44c5c2068432) | Merged | Supplied in the PR range, but not part of this epic; it is an Android GStreamer NDK media public SDK audit. |
| [#622](https://github.com/Tzeusy/tze-hud/pull/622) | [`4794b32`](https://github.com/Tzeusy/tze-hud/commit/4794b324fb704dc6a250cf45489ce115a564853a) | Merged | Added local/unit and headless integration validation plus the validation evidence document. |

## Cooperative HUD Projection Matrix

| Requirement | Evidence | Status |
| --- | --- | --- |
| Cooperative Attachment Contract | `crates/tze_hud_projection/src/lib.rs`; `/hud-projection` skill package; validation evidence for provider-neutral operation schema and no terminal capture. | Covered by local/headless validation |
| External Projection State Authority | `ProjectionAuthority` tests cover transcript retention, memory-only state, reconnect, stale lease denial, heartbeat, detach, cleanup, and private-state purge behavior. | Covered by local/headless validation |
| Low-Token LLM-Facing Operations | Operation envelope, attach, publish_output, publish_status, get_pending_input, acknowledge_input, detach, cleanup, stable error-code, bounds, and audit tests in `crates/tze_hud_projection/src/lib.rs`. | Covered by local/headless validation |
| Projection Operation Authorization | Cross-projection denial, owner-token checks, operator cleanup separation, token expiry, audit record bounds, and same-OS-user non-authority are exercised in `cargo test -p tze_hud_projection`. | Covered by local/headless validation |
| HUD Input Inbox Delivery | Pending, delivered, deferred, handled, rejected, expired, idempotent acknowledgement, conflict rejection, expiry, FIFO, and response-bound scenarios are covered in `crates/tze_hud_projection/src/lib.rs`. | Covered by local/headless validation |
| Projected Portal Lifecycle | `projected_portal_state`, detach cleanup behavior, portal adapter family metadata, and resident text-stream portal adapter tests cover the contract boundary without new runtime node types. | Covered by local/headless validation; live UX pending |
| Provider-Neutral Projection Identity | Provider kind behavior is tested for `codex`, `claude`, `opencode`, and fallback paths without changing operation semantics. | Covered by local/headless validation |
| Privacy and Attention Governance | Policy-permitted portal state, fail-closed private classification, bounded status, local pending feedback, and ambient backlog semantics are represented in projection authority tests and reconciliation constraints. | Covered by local/headless validation; live redaction/safe-mode pending |
| Bounded Backpressure and Expiry | Output/input size rejection, pending queue bounds, poll bounds, transcript pruning, coalesced portal updates, and lifecycle state-machine property coverage are validated by `cargo test -p tze_hud_projection`. | Covered by local/headless validation |

## Text-Stream Portals Delta Matrix

| Requirement | Evidence | Status |
| --- | --- | --- |
| Cooperative LLM Projection Adapter | `tests/integration/text_stream_portal_adapter.rs` includes `cooperative_projection_runtime_surface_is_provider_neutral_and_process_agnostic`; `ProjectedPortalAdapterFamily::CooperativeProjection` keeps runtime behavior provider-neutral. | Covered by headless integration validation |
| Cooperative Projection Input Mapping | Projection authority maps submitted portal input to bounded semantic inbox items, not raw terminal keystrokes; inbox behavior is validated in `crates/tze_hud_projection/src/lib.rs`. | Covered by local/headless validation |
| Cooperative Projection State Externality | Retained transcript history, pending input queue, acknowledgement state, reconnect metadata, and visible portal state remain in `tze_hud_projection` authority state rather than scene graph or runtime core. | Covered by local/headless validation |

## Validation Evidence

Recorded evidence: [`docs/evidence/cooperative-hud-projection/validation-2026-04-29.md`](../evidence/cooperative-hud-projection/validation-2026-04-29.md)

Commands recorded there:

```bash
cargo fmt --check
cargo test -p tze_hud_projection
cargo test -p integration --test text_stream_portal_adapter
```

The final recorded results were `cargo fmt --check` passing, `cargo test -p tze_hud_projection` passing with 34 tests, and `cargo test -p integration --test text_stream_portal_adapter` passing with 6 tests.

## Residual Risks And Deferred Seams

| Risk or seam | Status | Closeout condition |
| --- | --- | --- |
| Live Windows HUD `/user-test` for attach -> publish output -> HUD input -> poll/acknowledge -> collapse/restore -> detach cleanup | Pending | Run the visible overlay user-test and record evidence for task 4.4. |
| Live redaction, safe mode, freeze, dismiss, orphan cleanup, and backlog non-escalation | Pending live validation | Include these governance paths in the Windows pass or explicitly waive with rationale. |
| Skill/MCP facade divergence | Watch | Keep all skill and MCP packaging tied to the same projection operation schema; do not fork provider-specific semantics. |
| External daemon packaging versus runtime MCP | Watch | Projection MCP operations must remain served by the external projection authority, not runtime v1 MCP. |
| Memory-only v1 state | Accepted v1 constraint | Future durable projection state needs a separate encrypted-store contract before implementation. |
| Task checkbox drift | Administrative | `tasks.md` has unchecked implementation and validation items even where merged PRs and validation evidence show completion. Use this report plus evidence as the closure signal, then normalize tasks only if the coordinator wants the ledger edited before archive. |
| PR #621 in supplied range | Not epic evidence | It is an unrelated Android GStreamer NDK media public SDK audit and should not be used to support `hud-ggntn` requirement coverage. |

## OpenSpec Sync And Archive Readiness

The change is not ready to archive as-is. Blockers:

1. Live Windows HUD validation for task 4.4 is still pending in the recorded evidence.
2. `tasks.md` still has unchecked implementation, validation, sync, and archive items despite merged code and local/headless validation; the coordinator should decide whether to reconcile the ledger before sync/archive.
3. Accepted delta specs have not yet been synced into `openspec/specs/`.

OpenSpec sync is conditionally ready after the coordinator either completes or explicitly waives the live Windows HUD pass. Archive should wait until sync is done and the live-validation decision is recorded.
