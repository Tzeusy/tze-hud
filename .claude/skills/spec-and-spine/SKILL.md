---
name: spec-and-spine
description: >
  Ground all tze_hud implementation work in the v1 MVP capability specifications (openspec/).
  The 13 capability specs are the single source of truth for feature planning and development.
  Use this skill: (1) before implementing any feature — to identify and load relevant specs,
  (2) when detecting or resolving spec-code divergence, (3) when evolving specs as features change,
  (4) when planning new work to ensure spec coverage, (5) when reconciling after implementation chunks.
  Triggers: "check the spec", "what does the spec say", "ground this in specs", "spec drift",
  "divergence", "reconcile", "does the code match the spec", "write a spec for this",
  "which spec covers", "spec-first", any implementation task touching a v1 subsystem.
---

# Spec & Spine

OpenSpec capability specifications are the backbone of tze_hud. Every feature, every implementation task, every test traces back to a normative requirement in a spec. No code without spec coverage. Stale specs are worse than no specs.

## Four-Pillar Model

| Layer | Location | Role |
|-------|----------|------|
| Doctrine | `about/heart-and-soul/` | WHY — philosophical foundations, seven non-negotiable rules |
| Design Contracts | `about/law-and-lore/rfcs/0001–0013` | HOW — wire-level protobuf schemas, state machines, field numbers, latency budgets |
| Capability Specs | `openspec/changes/v1-mvp-standards/specs/` | WHAT — normative requirements with testable scenarios, RFC traceability, v1 scope tags |
| Topology | `about/lay-and-land/` | WHERE — component boundaries, data flow, deployment topology |

Specs bridge doctrine and RFCs to implementation. Traceability chain: **spec requirement → RFC section → doctrine principle**.

## Domain Lookup

| Domain | Spec path (under `openspec/changes/v1-mvp-standards/specs/`) | Source RFC | Layer |
|--------|------|------------|-------|
| Scene Graph | `scene-graph/spec.md` | RFC 0001 | Foundation |
| Runtime Kernel | `runtime-kernel/spec.md` | RFC 0002 | Foundation |
| Timing Model | `timing-model/spec.md` | RFC 0003 | Foundation |
| Input Model | `input-model/spec.md` | RFC 0004 | Hot path |
| Session Protocol | `session-protocol/spec.md` | RFC 0005 | Hot path |
| Configuration | `configuration/spec.md` | RFC 0006 | Governance |
| System Shell | `system-shell/spec.md` | RFC 0007 | Governance |
| Lease Governance | `lease-governance/spec.md` | RFC 0008 | Governance |
| Policy Arbitration | `policy-arbitration/spec.md` | RFC 0009 | Governance |
| Scene Events | `scene-events/spec.md` | RFC 0010 | Events |
| Resource Store | `resource-store/spec.md` | RFC 0011 | Storage |
| Text Stream Portals | `text-stream-portals/spec.md` | RFC 0013 | Interaction |
| Validation Framework | `validation-framework/spec.md` | (cross-cutting) | Testing |

Task map: `openspec/changes/v1-mvp-standards/tasks.md` — 156 tasks across 13 sections.

For the full dependency graph and task-section breakdown, see [references/spec-map.md](references/spec-map.md).

## Workflow 1: Ground Implementation in Specs

Before writing code for any feature:

1. **Identify domains** — Determine which spec domain(s) the work touches. Use the domain lookup table above. Most work touches 1–3 specs.
2. **Load selectively** — Read only the relevant spec(s). Never load all 13. Use `/law-and-lore` if you need the underlying RFC details.
3. **Verify coverage** — Confirm requirements exist for the planned behavior. Check `Scope: v1-mandatory` tags.
4. **No requirement? Spec first.** — If no requirement covers the planned behavior, writing the spec is the first task. Use `/opsx:new` or `/opsx:explore` to create a delta spec.
5. **Implement against scenarios** — Each requirement's WHEN/THEN scenarios are your acceptance criteria. Implementation must satisfy them.
6. **Reconcile after** — After implementation, verify: does behavior match spec? Did drift occur? Create beads for any gaps.

### Requirement Structure

Every requirement in a spec follows this pattern:

```
### Requirement: <Name>
<Normative text using SHALL/MUST>
Source: RFC NNNN §X.Y
Scope: v1-mandatory | v1-reserved | post-v1

#### Scenario: <Name>
- **WHEN** <precondition>
- **THEN** <expected behavior>
```

- **v1-mandatory** — Must be implemented. Generates tasks and tests.
- **v1-reserved** — Schema defined, implementation may be minimal/stubbed.
- **post-v1** — Documented for forward compatibility. No implementation required.

## Workflow 2: Detect and Resolve Divergence

Four divergence patterns and their resolution:

### Code ahead of spec
Implementation exists but no spec requirement covers it.
1. Identify the uncovered behavior
2. Create a delta spec documenting the capability (`/opsx:new`)
3. Sync delta to main spec (`/opsx:sync`)

### Spec ahead of code
Spec requirements exist but implementation is missing.
1. Identify unimplemented v1-mandatory requirements
2. Create beads for each: `bd create --title="Implement <Requirement>" --description="Spec: <domain>/spec.md, Requirement: <Name>" -t task`
3. Link to parent epic if applicable

### Spec-code mismatch
Implementation behavior contradicts the spec.
1. Determine which is correct — consult the source RFC and doctrine
2. **Spec is wrong** → Fix the spec via delta spec, sync, archive
3. **Code is wrong** → Fix the implementation, verify against WHEN/THEN scenarios
4. Never silently accept the mismatch

### New feature request
No spec or code exists yet.
1. Start with `/opsx:explore` to investigate and clarify requirements
2. Create delta spec via `/opsx:new` → `/opsx:continue` or `/opsx:ff`
3. Only begin implementation after spec artifacts are complete (`/opsx:apply`)
4. Verify and archive after implementation (`/opsx:verify` → `/opsx:sync` → `/opsx:archive`)

## Workflow 3: Evolve Specs

Specs must stay current as features evolve. The openspec lifecycle:

```
explore → new → continue/ff → apply → verify → sync → archive
```

**Rules:**
- Delta specs describe additions, modifications, or removals within a change
- No change is archived until verified (completeness, correctness, coherence)
- After syncing delta specs to main specs, the main specs become the new source of truth
- Every requirement modification must preserve RFC traceability (`Source: RFC NNNN §X.Y`)

**When to update specs:**
- Bug fix reveals spec was ambiguous → Clarify the requirement
- Refactor changes behavior → Update affected scenarios
- New capability added → Add requirements with WHEN/THEN scenarios
- Requirement proven infeasible → Mark as deferred with rationale

## Workflow 4: Plan New Work

When planning what to build next:

1. **Check task coverage** — Read `openspec/changes/v1-mvp-standards/tasks.md` for the 156 existing tasks
2. **Check spec dependency order** — Foundation specs (scene-graph, runtime-kernel, timing) must be implemented before governance specs. See the dependency graph in [references/spec-map.md](references/spec-map.md).
3. **Evaluate against eight dimensions** — Alignment, user value, leverage, tractability, timing, dependencies, implementation risk, churn likelihood (from `about/heart-and-soul/development.md`)
4. **No spec = no plan** — If the proposed work has no spec coverage, the first task is writing the spec

## Quick Reference: Related Skills

| Need | Skill |
|------|-------|
| Underlying wire-level contracts | `/law-and-lore` (loads relevant RFCs) |
| Philosophical foundations | `/heart-and-soul` (loads relevant doctrine) |
| System topology and boundaries | `/lay-and-land` (loads component maps) |
| Create/continue spec changes | `/opsx:new`, `/opsx:continue`, `/opsx:ff` |
| Implement from spec | `/opsx:apply` |
| Verify before archiving | `/opsx:verify` |
| Sync deltas to main | `/opsx:sync` |
| Explore before committing | `/opsx:explore` |
| Reconcile spec vs code | `/reconcile-spec-to-project` |
