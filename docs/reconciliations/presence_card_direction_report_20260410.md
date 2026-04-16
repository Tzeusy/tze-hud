# Presence Card Direction Report

Date: 2026-04-10
Scope: `/project-direction` package for the exemplar Presence Card
Status: Planned, locally materialized, and filed as epic `hud-sx7q`

## Executive summary

[Observed] The Presence Card is not an optional flourish. It is one of the cleanest v1 proofs of the core thesis: an LLM can hold raw tile territory over time, coexist with other agents, survive lease transitions, and fail gracefully without collapsing the rest of the surface. That aligns directly with the project doctrine in [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md#L3), [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L11), and [failure.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/failure.md#L15).

[Observed] The codebase is farther along than the existing planning docs admit. The repo already contains tile-builder integration coverage, concurrent gRPC coexistence coverage, and disconnect/orphan coverage for Presence Card behavior in [presence_card_tile.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_tile.rs#L1), [presence_card_coexistence.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_coexistence.rs#L1), and [disconnect_orphan.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/disconnect_orphan.rs#L1). But the live Windows validation path is still missing: the current `/user-test` skill only defines MCP zone/widget flows in [.claude/skills/user-test/SKILL.md](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/SKILL.md#L8), the deployed production config only exposes widget instances in [production.toml](/home/tze/gt/tze_hud/mayor/rig/app/tze_hud_app/config/production.toml#L61), and the existing gRPC helper does not yet expose Presence Card-specific resident-flow conveniences in [hud_grpc_client.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/hud_grpc_client.py#L52).

[Inferred] The highest-priority next work is therefore not more speculative scene-graph implementation. It is to reconcile stale planning docs to current implementation reality, extend the resident gRPC user-test tooling to cover the full Presence Card flow, add a first-class live `/user-test` scenario, and then use that scenario to close the manual checklist item. Anything broader would be planning theater, and anything narrower would keep the exemplar trapped in headless tests without a real operator-visible proof.

## Project Spirit

**Core problem**: Prove that a resident agent can hold custom tile territory on a real display with leases, resource governance, graceful failure handling, and multi-agent coexistence.
**Primary user**: Internal developers and agent operators validating raw tile exemplars and v1 presence claims.
**Success looks like**: Three resident agents can create stacked Presence Card tiles, update “Last active” content over time, survive one-agent disconnect/orphan transitions, and complete a reproducible live Windows manual review using repo-native tooling.
**Trying to be**: The canonical raw-tile exemplar proving v1 tile/lease/failure behavior, complementary to zone/widget exemplars.
**Not trying to be**: A new widget type, a runtime-owned zone, a design-system-only exercise, or a generic orchestrator framework.

### Requirements

| # | Requirement | Class | Evidence | Status |
|---|------------|-------|---------|--------|
| 1 | Presence Card must exercise raw tile territory rather than zone/widget abstraction | Hard | [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md#L145), [spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L10) | Met in tests, unmet in live tooling |
| 2 | V1 must prove tile holding, lease model, and 3-agent coexistence | Hard | [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L11) | Partial |
| 3 | Scene mutations must remain atomic; Presence Card creation/update cannot expose intermediate garbage | Hard | [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md#L26), [spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L17) | Met in tests |
| 4 | Static-image avatar uploads must be content-addressed and capability-gated | Hard | [0001-scene-contract.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0001-scene-contract.md#L47), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0011-resource-store.md#L45) | Met in integration tests, unmet in live harness |
| 5 | Disconnects must orphan leases, preserve frozen content, show disconnection affordance, and permit grace-period reclaim | Hard | [failure.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/failure.md#L15), [0008-lease-governance.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0008-lease-governance.md#L1), [spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L103) | Partial |
| 6 | Validation must be machine-readable and culminate in human-visible artifacts for medium/high complexity work | Hard | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md#L3), [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md#L141) | Partial |
| 7 | Presence Card live review should use resident gRPC/session flow, not be faked through widgets or guest MCP | Hard | [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md#L143), [.claude/skills/user-test/scripts/hud_grpc_client.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/hud_grpc_client.py#L5) | Unmet |
| 8 | Manual review must end in checklist closure, not a one-off ad hoc run | Soft | [docs/exemplar-manual-review-checklist.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-manual-review-checklist.md#L204), [docs/exemplar-presence-card-user-test.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md#L1) | Unmet |
| 9 | Presence Card should not require new runtime product surfaces before live proof exists | Non-goal | [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md#L145), [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L25) | N/A |
| 10 | Generic raw-tile orchestration framework must land before one exemplar works live | Non-goal | [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L152) | N/A |

### Contradictions

[Observed] The current coverage note understates implementation reality. It still says `hud-apoe.3` and `hud-apoe.4` are blocked and that periodic updates, coexistence, and disconnect flows are only partial in [docs/exemplar-presence-card-coverage.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-coverage.md#L12), but the repo now contains those integration surfaces in [tests/integration/presence_card_coexistence.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_coexistence.rs#L1) and [tests/integration/disconnect_orphan.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/disconnect_orphan.rs#L1). This is doc-to-code drift, not missing implementation.

[Observed] The manual user-test document describes a seven-step live scenario in [docs/exemplar-presence-card-user-test.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md#L11), but the actual `/user-test` skill only documents zone/widget flows in [.claude/skills/user-test/SKILL.md](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/SKILL.md#L8). The script inventory also has no Presence Card scenario entry, so the documented manual flow is not executable through the current skill surface.

## Current State

| Dimension | Status | Summary | Key Evidence |
|-----------|--------|---------|-------------|
| Spec adherence | Adequate | The exemplar spec is clear and most core behaviors already have code-level coverage, but the live user-test path is absent | [spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L10) |
| Core workflows | Adequate | Headless tile creation, content update, coexistence, and disconnect flows exist; Windows manual flow does not | [presence_card_tile.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_tile.rs#L1) |
| Test confidence | Strong | Presence Card has scene-layer and integration coverage across geometry, resources, updates, coexistence, and orphan handling | [lease_lifecycle_presence_card.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs#L1) |
| Observability | Adequate | The doctrine for artifacts exists, but the Presence Card live scenario has no current machine-readable run artifact | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md#L141) |
| Delivery readiness | Weak | The deployed Windows path is widget-only from the operator skill/config perspective, and no live Presence Card scenario is wired | [production.toml](/home/tze/gt/tze_hud/mayor/rig/app/tze_hud_app/config/production.toml#L61), [.claude/skills/user-test/SKILL.md](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/SKILL.md#L109) |
| Architectural fitness | Strong | The architecture already supports raw tiles, static images, leases, disconnect badges, and resident gRPC sessions; the gap is tooling/integration, not core design | [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md#L11), [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md#L1) |

[Observed] The biggest strength is architectural fitness. Presence Card does not need a new scene model, a new protocol plane, or a new runtime surface. The existing tile, lease, static-image, and gRPC machinery already expresses the exemplar.

[Observed] The biggest gap is delivery reality. The repo can prove Presence Card headlessly, but it cannot currently prove it on the real Windows overlay through the same operator-facing `/user-test` workflow used for the widget exemplars.

## Alignment Review

### Aligned next steps

1. [Observed] Reconcile Presence Card planning artifacts to the current implementation so work starts from honest gaps rather than stale blocked-bead assumptions.
2. [Inferred] Extend the resident gRPC user-test helper to cover the exemplar’s actual wire needs: avatar upload, `StaticImageNode`, `UpdateTileOpacity`, `UpdateTileInputMode`, concurrent sessions, and controlled disconnect/reconnect.
3. [Inferred] Add a first-class Presence Card live scenario under `/user-test` that drives three resident agents through the spec’s visible lifecycle.
4. [Inferred] Use that scenario to produce the missing live Windows validation evidence and close the manual checklist entry.

### Misaligned directions

1. [Observed] Recasting Presence Card as a widget or zone would dodge the point of the exemplar, which is to prove the raw tile path.
2. [Inferred] Adding a built-in runtime-owned Presence Card surface before the raw-tile scenario is validated live would be architecture avoidance.
3. [Inferred] Creating a generic tile orchestration framework before one repo-native live scenario works would be overreach and likely churn.

### Premature work

1. [Inferred] Visual polish beyond the current spec, such as themed cards or richer avatar treatments, is premature until the live proof path exists.
2. [Inferred] Cross-exemplar raw-tile abstractions are premature until the Presence Card live scenario has demonstrated what helper seams are actually reusable.

### Deferred

1. [Inferred] A reusable raw-tile resident harness shared across multiple exemplars is aligned, but it should follow the Presence Card scenario rather than precede it.
2. [Inferred] Native screenshot/video artifact capture for overlay manual reviews is useful, but the repo first needs a single trustworthy live scenario to exercise.

### Rejected

1. [Observed] A new built-in production widget/config surface for Presence Card is rejected for this tranche. The exemplar already has a spec and tests for the raw tile path; replacing it with a widget would invalidate the purpose of the work.

## Gap Analysis

### Blockers

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No `/user-test` Presence Card scenario exists | Manual review cannot be executed through repo-native tooling | Operators, reviewers | [.claude/skills/user-test/SKILL.md](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/SKILL.md#L8), [docs/exemplar-presence-card-user-test.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md#L11) | Implement resident gRPC scenario + skill integration | M |
| `hud_grpc_client.py` lacks exemplar-specific resident helpers | The current helper cannot express the full manual flow without bespoke ad hoc code | Tooling | [hud_grpc_client.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/hud_grpc_client.py#L52) | Extend helper with resource upload + richer mutation helpers | M |
| Planning/coverage docs are stale versus the code | Beads risk targeting already-completed work or missing the real gap | Planners, implementers | [docs/exemplar-presence-card-coverage.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-coverage.md#L12), [presence_card_coexistence.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_coexistence.rs#L1) | Reconcile docs before implementation | S |
| No live Windows validation evidence or checklist closure exists | The exemplar cannot honestly be called complete | Reviewers, operators | [docs/exemplar-manual-review-checklist.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-manual-review-checklist.md#L204) | Run scenario live and update review artifacts | M |

### Important Enhancements

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| Live scenario lacks structured operator/run artifact output | Validation doctrine prefers machine-readable evidence over anecdote | Reviewers, future agents | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md#L141) | Emit concise run transcript/artifact from Presence Card scenario | S |
| Presence Card coverage and manual docs are disconnected from `/user-test` skill | Docs cannot directly drive execution | Operators | [docs/exemplar-presence-card-user-test.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md#L1), [.claude/skills/user-test/SKILL.md](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/SKILL.md#L8) | Cross-link scenario from skill and review docs | S |

### Strategic Gaps

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No reusable resident raw-tile validation harness abstraction | Future tile exemplars will likely repeat setup/disconnect orchestration work | Tooling | [hud_grpc_client.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/hud_grpc_client.py#L102) | Defer until Presence Card scenario reveals stable seams | M |
| Native overlay artifact capture remains manual | Human-visible proof is hard to preserve across sessions | Reviewers | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md#L141) | Defer until after first live scenario works end-to-end | M |

## Work Plan

### Immediate alignment work

### Chunk 1: Reconcile Presence Card coverage and review artifacts

**Objective**: Update Presence Card planning docs so they reflect the current codebase and isolate the real missing work.
**Spec reference**: `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md` — all requirements; especially `Requirement: gRPC Test Sequence` and `Requirement: User-Test Scenario`
**Dependencies**: None
**Why ordered here**: Current docs still describe blocked work that already exists in tests; planning from stale assumptions would create churn.
**Scope**: S
**Parallelizable**: No — this defines the current truth the rest of the epic depends on.
**Serialize with**: Chunks 2-4

**Acceptance criteria**:
- [ ] Coverage and review docs accurately distinguish implemented headless/integration behavior from missing live tooling
- [ ] Remaining gaps are narrowed to live resident user-test integration and manual validation closure
- [ ] Checklist/update documents cite the exact spec sections still awaiting live proof

**Notes**: This is a doc/code reconciliation pass, not new product work.

### Chunk 2: Extend gRPC user-test helper for Presence Card resident flows

**Objective**: Make the existing resident helper capable of driving the full Presence Card wire contract without one-off script hacks.
**Spec reference**: `Requirement: Presence Card Node Tree`, `Requirement: Resource Upload for Avatar Icons`, `Requirement: gRPC Test Sequence`
**Dependencies**: Chunk 1
**Why ordered here**: The live scenario cannot be written cleanly until the helper can express uploads and full tile mutations.
**Scope**: M
**Parallelizable**: No — the scenario script depends on these helper capabilities.
**Serialize with**: Chunks 3-4

**Acceptance criteria**:
- [ ] Helper supports avatar upload and `StaticImageNode` construction for Presence Card flows
- [ ] Helper supports `UpdateTileOpacity` and `UpdateTileInputMode` in the same scenario surface as tile creation
- [ ] Helper exposes enough session/disconnect primitives to drive the orphan/grace sequence without ad hoc duplicated protocol code

**Notes**: Keep the helper narrow to resident raw-tile scenarios; do not turn it into a speculative generic framework.

### Chunk 3: Add Presence Card live scenario and `/user-test` integration

**Objective**: Create a repo-native live Windows scenario that launches three resident agents and walks the manual user-test lifecycle.
**Spec reference**: `Requirement: Multi-Agent Vertical Stacking`, `Requirement: Periodic Content Update`, `Requirement: Agent Disconnect and Orphan Handling`, `Requirement: User-Test Scenario`
**Dependencies**: Chunk 2
**Why ordered here**: This is the missing delivery surface that converts headless coverage into operator-visible proof.
**Scope**: M
**Parallelizable**: No — it depends on the helper contract and writes the operator-facing skill/docs surface.
**Serialize with**: Chunk 4

**Acceptance criteria**:
- [ ] A Presence Card scenario script exists under `.claude/skills/user-test/scripts/`
- [ ] The `/user-test` skill documents how to run the Presence Card scenario on Windows
- [ ] The scenario can drive three agents through create → update → disconnect/orphan → cleanup with structured step output

**Notes**: The live scenario should use resident gRPC/session flow, not widget or MCP publish shortcuts.

### Chunk 4: Execute Presence Card live validation and close manual review

**Objective**: Produce the missing live evidence and update the checklist/coverage artifacts based on the actual Windows run.
**Spec reference**: `Requirement: User-Test Scenario`; `docs/exemplar-manual-review-checklist.md` item 7
**Dependencies**: Chunk 3
**Why ordered here**: Manual review closure is the product proof this exemplar still lacks.
**Scope**: M
**Parallelizable**: No — it is the terminal validation gate for this tranche.
**Serialize with**: None

**Acceptance criteria**:
- [ ] The scenario is run against the Windows overlay target
- [ ] Manual review outcomes are recorded in the Presence Card user-test/coverage artifacts
- [ ] Checklist item 7 is updated with actual visual outcomes or an explicit remaining blocker

**Notes**: If the live run uncovers runtime defects, create follow-up beads rather than burying them in docs.

### Block Reconciliation: Presence Card Live Proof

Check:
- [ ] All chunks' acceptance criteria are met
- [ ] The live scenario matches the exemplar spec rather than a simplified approximation
- [ ] No drift remains between coverage docs, user-test docs, and actual scripts
- [ ] Checklist state reflects real execution evidence
- [ ] Follow-up gaps are captured in beads, not TODO comments

## Bead Graph

Epic:
- `hud-sx7q` — Presence Card live user-test flow

Implementation children:
- `hud-sx7q.1` — Reconcile Presence Card coverage and review artifacts
- `hud-sx7q.2` — Extend gRPC user-test helper for Presence Card resident flows
- `hud-sx7q.3` — Add Presence Card live scenario and `/user-test` integration
- `hud-sx7q.4` — Execute Presence Card live validation and close manual review

Terminal children:
- `hud-sx7q.5` — Reconcile spec-to-code (gen-1) for Presence Card live user-test flow
- `hud-sx7q.6` — Generate epic report for: Presence Card live user-test flow

## Do Not Do Yet

| Item | Reason | Revisit when |
|------|--------|-------------|
| Rebuild Presence Card as a widget | Breaks the purpose of the raw-tile exemplar | Never for this exemplar |
| Add a generic multi-exemplar raw-tile orchestration framework | Premature before one live scenario exists | After Presence Card live scenario stabilizes |
| Expand scope into new styling/themes/avatar polish | Cosmetic work before live proof | After checklist item 7 is closed |
| Add new runtime-owned config surfaces for Presence Card | Avoids rather than validates the resident tile path | Only if doctrine changes |

## Appendix

### A. Repository Map
- `openspec/changes/exemplar-presence-card/` — authoritative exemplar spec, design, and tasks
- `tests/integration/` — Presence Card tile, coexistence, and disconnect integration tests
- `.claude/skills/user-test/` — current live Windows operator workflow
- `app/tze_hud_app/config/production.toml` — deployed Windows widget surface, useful as contrast for what Presence Card is not

### B. Critical Workflows
1. Resident agent authenticates over gRPC session stream and receives namespace/capabilities
2. Agent requests lease, uploads avatar resource, creates Presence Card tile, sets node tree
3. Three agents coexist with non-overlapping tile geometry and independent namespaces
4. One agent disconnects, lease becomes orphaned, tile freezes and badged state appears, grace period expires or lease is reclaimed
5. Operator runs a Windows live scenario and records checklist results

### C. Spec Inventory
- `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md` — primary Presence Card contract
- `openspec/changes/exemplar-presence-card/tasks.md` — task decomposition, partially stale versus code
- `docs/exemplar-presence-card-user-test.md` — manual scenario definition
- `docs/exemplar-manual-review-checklist.md` — visual signoff destination

### D. Evidence Index
- [presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md)
- [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md)
- [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md)
- [failure.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/failure.md)
- [0001-scene-contract.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0001-scene-contract.md)
- [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md)
- [0008-lease-governance.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0008-lease-governance.md)
- [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0011-resource-store.md)
- [presence_card_tile.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_tile.rs)
- [presence_card_coexistence.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/presence_card_coexistence.rs)
- [disconnect_orphan.rs](/home/tze/gt/tze_hud/mayor/rig/tests/integration/disconnect_orphan.rs)
- [hud_grpc_client.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/hud_grpc_client.py)
- [user-test skill](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/SKILL.md)
- [Presence Card coverage doc](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-coverage.md)
- [Presence Card manual user-test doc](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md)

---

## Conclusion

**Real direction**: Presence Card is a raw-tile proof-of-presence exemplar whose remaining work is live resident validation tooling and review closure, not foundational scene/runtime invention.

**Work on next**: reconcile stale Presence Card docs, extend the resident gRPC helper for the exemplar flow, add the live `/user-test` scenario, then run Windows manual validation and close checklist item 7.

**Stop pretending**: that Presence Card is already complete end-to-end. It has strong headless and integration coverage, but it does not yet have a repo-native live Windows proof path.
