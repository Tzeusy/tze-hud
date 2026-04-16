# Exemplar Presence Card — Coverage Reconciliation (2026-04-16)

**Issue**: `hud-sx7q.4`
**Spec**: `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md`
**Scope**: Reconcile Presence Card planning/review artifacts to current code reality.

---

## Current State

Presence Card implementation and automated integration coverage are now materially ahead of the prior planning docs.

What is implemented and test-backed:
- Tile geometry, stacking, z-order, and Passthrough behavior.
- Node-tree construction (background/avatar/text) and resource upload semantics.
- Lease lifecycle, orphan/expiry transitions, reconnect behavior, and isolation.
- Headless/runtime gRPC integration coverage for concurrent agents and updates.

What is still missing:
- Successful authenticated execution evidence from the live resident Presence
  Card scenario run on the Windows overlay target.
- A completed manual-review closeout entry showing PASS/FAIL outcomes from that
  run.

This narrows remaining work to live validation tooling + manual-review closure, not core implementation gaps.

---

## Requirement Status Matrix

| Requirement | Status | Evidence |
|---|---|---|
| Presence Card Tile Geometry | Implemented + automated | `tests/integration/presence_card_tile.rs` |
| Multi-Agent Vertical Stacking | Implemented + automated | `tests/integration/presence_card_tile.rs`, `tests/integration/presence_card_coexistence.rs` |
| Presence Card Node Tree | Implemented + automated | `tests/integration/presence_card_tile.rs` |
| Lease Lifecycle for Presence Cards | Implemented + automated | `crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs`, `tests/integration/disconnect_orphan.rs` |
| Periodic Content Update | Implemented + automated | `tests/integration/presence_card_coexistence.rs` |
| Agent Disconnect and Orphan Handling | Implemented + automated | `tests/integration/disconnect_orphan.rs` |
| Multi-Agent Isolation During Disconnect | Implemented + automated | `tests/integration/disconnect_orphan.rs` |
| Resource Upload for Avatar Icons | Implemented + automated | `tests/integration/presence_card_tile.rs` |
| gRPC Test Sequence | Implemented + integration-tested; live resident run attempted but blocked by missing auth secret in operator env | `tests/integration/presence_card_coexistence.rs`, `docs/exemplar-presence-card-user-test.md` |
| User-Test Scenario | Implemented in `/user-test`; live visual proof currently blocked by missing `TZE_HUD_PSK` at execution time | `.claude/skills/user-test/scripts/presence_card_exemplar.py`, `docs/exemplar-presence-card-user-test.md` |

---

## Remaining Live-Proof Gaps (Exact Spec Sections)

The remaining unresolved proof is explicitly limited to these spec sections:

1. `Requirement: gRPC Test Sequence`
- `Scenario: Full single-agent lifecycle`
- `Scenario: Three-agent concurrent lifecycle`
- Remaining proof needed: successful authenticated execution evidence through
  the resident `/user-test` workflow on a live runtime target.

2. `Requirement: User-Test Scenario`
- `Scenario: User-test visual verification sequence`
- Remaining proof needed: completed manual visual run and documented pass/fail
  outcomes in the manual review checklist after environment auth unblock.

---

## 2026-04-16 Live Validation Attempt

- Attempted to run the canonical resident scenario against
  `tzehouse-windows.parrot-hen.ts.net:50051`.
- Host/port reachability checks passed for gRPC (`50051`) and MCP (`9090`).
- Scenario exited immediately with `{"error":"missing_psk","psk_env":"TZE_HUD_PSK"}`.
- Outcome: live proof remains blocked on missing `TZE_HUD_PSK` in the operator
  shell.

---

## Closure Criteria For This Epic Line

Presence Card live-proof work is complete when both are true:
- A resident `/user-test` Presence Card scenario exists and is run with captured evidence.
- `docs/exemplar-manual-review-checklist.md` marks Presence Card as done with concrete review notes and any residual risk.
