# Exemplar Presence Card — Coverage Reconciliation (2026-04-10)

**Issue**: `hud-sx7q.1`
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
- A repo-native **live resident** Presence Card scenario in `/user-test`.
- A completed manual-review closeout entry showing Windows/live proof results.

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
| gRPC Test Sequence | Implemented + integration-tested; live resident proof pending | `tests/integration/presence_card_coexistence.rs` |
| User-Test Scenario | Documented; live resident proof pending | `docs/exemplar-presence-card-user-test.md` |

---

## Remaining Live-Proof Gaps (Exact Spec Sections)

The remaining unresolved proof is explicitly limited to these spec sections:

1. `Requirement: gRPC Test Sequence`
- `Scenario: Full single-agent lifecycle`
- `Scenario: Three-agent concurrent lifecycle`
- Remaining proof needed: execution evidence through the resident `/user-test` workflow on a live runtime target.

2. `Requirement: User-Test Scenario`
- `Scenario: User-test visual verification sequence`
- Remaining proof needed: completed manual visual run and documented pass/fail outcomes in the manual review checklist.

---

## Closure Criteria For This Epic Line

Presence Card live-proof work is complete when both are true:
- A resident `/user-test` Presence Card scenario exists and is run with captured evidence.
- `docs/exemplar-manual-review-checklist.md` marks Presence Card as done with concrete review notes and any residual risk.
