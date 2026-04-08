# Policy Wiring Direction Report

Date: 2026-04-08
Scope: `/project-direction` pass for policy wiring
Issue: `hud-iq2x.1`

## 1. Executive Summary
`[Observed]` The project spirit is a sovereign runtime that enforces lease/capability governance with predictable performance, not a passive renderer (`about/heart-and-soul/v1.md:11-21`, `about/heart-and-soul/README.md:3-6`).

`[Observed]` Policy-related OpenSpec requirements currently overstate implemented runtime behavior: specs require a fully wired 7-level arbitration pipeline for frame/event/mutation flows, but runtime docs and authority boundaries explicitly state `tze_hud_policy` is not wired in v1 and enforcement originates in runtime/scene modules (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:10-230`, `crates/tze_hud_runtime/src/lib.rs:12-26`, `crates/tze_hud_runtime/src/budget.rs:23-35`).

`[Inferred]` Highest-priority next work is a two-track alignment program: (1) spec reconciliation to split "v1 implemented authority" vs "policy-stack reference authority", and (2) seam-by-seam runtime wiring of pure policy evaluators where they do not duplicate state ownership, starting with the mutation path.

## 2. Project Spirit And Requirements

### Project Spirit
**Core problem**: Give LLMs governed, real-time, multi-agent on-screen presence with strict runtime sovereignty and latency budgets.
**Primary user**: Resident/guest agents and runtime operators validating governance/perf in production-like flows.
**Success looks like**: 60fps budgets hold while lease/capability controls and zone publishing remain enforced under contention and failure (`about/heart-and-soul/v1.md:11-21`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:155-170`).
**Trying to be**: A sovereign policy-governed runtime kernel, not an ad hoc app layer (`openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:10-13`).
**Not trying to be**: Full post-v1 dynamic policy orchestration in v1 (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:368-374`).

### Requirements
| # | Requirement | Class | Evidence | Status |
|---|---|---|---|---|
| 1 | Runtime sovereignty over leases/capabilities | Hard | `about/heart-and-soul/v1.md:11`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:10-13` | Partial |
| 2 | Seven-level arbitration stack governs runtime decisions | Hard | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:10-12`, `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:217-230` | Unmet |
| 3 | Mutation/event/frame paths meet strict latency budgets | Hard | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:195-227`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:155-170` | Partial |
| 4 | Mid-session capability escalation must be policy-validated | Hard | `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:615-636` | Partial |
| 5 | Dynamic policy rules deferred beyond v1 | Non-goal (v1) | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:368-374` | Met |
| 6 | Single-source authority boundary between runtime state owners and policy evaluators | Soft | `crates/tze_hud_policy/src/lib.rs:29-46`, `crates/tze_hud_runtime/src/budget.rs:14-35` | Partial |
| 7 | Use external prompt brief `docs/reconciliations/policy_wiring_epic_prompt.md` | Unknown | Referenced by bead, file missing in tree | Unmet |

### Contradictions
- `[Observed]` Spec says full per-mutation policy stack MUST run (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:217-224`), but runtime declares policy crate is not wired and no `PolicyContext`/`ArbitrationOutcome` flow exists (`crates/tze_hud_runtime/src/lib.rs:22-26`).
- `[Observed]` Spec implies centralized stack ownership; code splits authority across runtime budget/safe-mode/attention and scene capability gates (`crates/tze_hud_runtime/src/budget.rs:14-35`, `crates/tze_hud_runtime/src/attention_budget/mod.rs:35-55`, `crates/tze_hud_scene/src/graph.rs:571-601`).
- `[Observed]` The issue references a prompt file that does not exist on this branch nor `origin/main`; direction had to be reconstructed from the bead text and project specs.

## 3. Current State

| Dimension | Status | Summary | Key Evidence |
|-----------|--------|---------|-------------|
| Spec adherence | Weak | Policy specs outrun runtime wiring reality | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:10-230`, `crates/tze_hud_runtime/src/lib.rs:12-26` |
| Core workflows | Adequate | Session mutation/zone publish/capability paths are implemented with deterministic handlers | `crates/tze_hud_protocol/src/session_server.rs:1812-1862`, `crates/tze_hud_protocol/src/session_server.rs:1944-2100`, `crates/tze_hud_protocol/src/session_server.rs:3440-3512`, `crates/tze_hud_protocol/src/session_server.rs:3647-3788` |
| Test confidence | Adequate | Deep local tests exist in policy + protocol, but CI has known unstable jobs | `crates/tze_hud_policy/src/tests.rs`, `.github/workflows/ci.yml:99-184`, `AGENTS.md:226` |
| Observability | Adequate | Structured telemetry and budget signals exist, but no policy-wire telemetry path | `crates/tze_hud_runtime/src/budget.rs:196-224`, `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:653-661` |
| Delivery readiness | Adequate | CI gates are broad; protobuf deps and known unstable tests reduce confidence | `.github/workflows/ci.yml:51-55`, `.github/workflows/ci.yml:99-184`, `AGENTS.md:226` |
| Architectural fitness | Adequate | Clear authority seams exist; they are good for incremental wiring if spec is reconciled first | `crates/tze_hud_runtime/src/lib.rs:5-29`, `crates/tze_hud_policy/src/lib.rs:31-46`, `crates/tze_hud_runtime/src/budget.rs:14-35` |

### Expanded Assessment
- **Spec adherence**: `[Observed]` `tze_hud_policy` is implemented as pure evaluators/tests, but not in runtime hot paths. This is direct drift against current v1 policy spec language.
- **Core workflows**: `[Observed]` Runtime handles safe mode, freeze queueing, capability requests, and zone publish acks. These are robust workflow hooks for integration, but they bypass the formal arbitration stack abstraction.
- **Test confidence**: `[Observed]` Policy module has broad scenario tests; protocol has integration tests for capability request and safe mode behaviors. `[Observed]` known pre-existing CI instability remains for certain jobs.
- **Observability**: `[Observed]` Budget telemetry exists, but no per-level policy decision telemetry from runtime-wired `ArbitrationOutcome` because that path is not active.
- **Delivery readiness**: `[Observed]` CI is extensive and explicit. `[Inferred]` readiness for policy-wiring changes is medium because stable infrastructure exists but baseline failures can mask regressions.
- **Architectural fitness**: `[Inferred]` The current split (stateful runtime owners + pure policy evaluators) is a workable architecture if codified as explicit seams and enforced by specs/tests.

## 4. Alignment Review

### Aligned Next Steps
1. **Spec reconciliation for v1 policy authority seams**
- Alignment: Core
- User value: High
- Leverage: High
- Tractability: Ready
- Timing: Now
- Risk: Low
- Churn: Low

2. **Mutation-path pilot wiring to `tze_hud_policy::mutation` (read-only evaluation, runtime executes outcomes)**
- Alignment: Core
- User value: High
- Leverage: High
- Tractability: Needs architecture notes first
- Timing: Soon
- Risk: Medium
- Churn: Medium

3. **Policy outcome telemetry + budget conformance benchmarks in wired path**
- Alignment: Supporting
- User value: Medium
- Leverage: Medium
- Tractability: Ready after pilot
- Timing: Soon
- Risk: Medium
- Churn: Medium

### Misaligned Directions
- **Big-bang runtime rewrite to funnel all authorities immediately through one new policy facade**.
`[Inferred]` This is high churn and risks breaking currently stable runtime ownership boundaries.

### Premature Work
- **Dynamic runtime policy-rule editing for v1**.
`[Observed]` Explicitly deferred post-v1 (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:368-374`).

### Deferred
- **Per-event and per-frame full stack wiring parity** after mutation path proves latency and correctness.

### Rejected
- **Continue asserting “full policy stack is wired in v1” without implementation evidence**.
This should be rejected immediately as spec credibility debt.

## 5. Gap Analysis

### Blockers
| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| Spec/runtime contradiction on policy wiring | Teams cannot tell which authority model is true | Maintainers, reviewers | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:217-230` vs `crates/tze_hud_runtime/src/lib.rs:12-26` | Reconcile specs first, then wire incrementally | M |
| Missing canonical epic prompt file | Reproducibility of direction pass is reduced | Future workers | missing `docs/reconciliations/policy_wiring_epic_prompt.md` | Restore file or update bead references | S |

### Important Enhancements
| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No runtime emissions for arbitration-level decisions | Hard to verify policy behavior and budgets | Operators, CI | no runtime `ArbitrationOutcome` flow (`crates/tze_hud_runtime/src/lib.rs:24-26`) | Add policy decision telemetry in pilot wiring | M |
| Capability request policy source is simplistic in v1 | Authorization policy semantics are underspecified | Security reviewers | `crates/tze_hud_protocol/src/session_server.rs:3455-3461` | Formalize policy source and tests | M |

### Strategic Gaps
| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No seam contract doc mapping runtime-owned state to policy contexts | Future wiring risks duplicate logic | Core runtime engineers | split authority comments in runtime/policy crates | Add seam contract doc + invariants | M |
| Latency budget proof for wired policy path absent | Could regress frame budgets silently | Performance owners | strict limits in spec (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:195-227`) | Add benchmark harness for wired path | L |

## 6. Work Plan

### Immediate alignment work

#### Chunk 1: Reconcile policy authority spec language
**Objective**: Align policy-arbitration/runtime/session specs to match current v1 enforcement reality and planned wiring seam.
**Spec reference**: `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`, `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
**Dependencies**: none
**Why ordered here**: removes ambiguity before code churn
**Scope**: M
**Parallelizable**: Yes — with documentation-only verification in parallel
**Serialize with**: Chunk 2+

**Acceptance criteria**:
- [ ] Spec sections explicitly distinguish “implemented runtime authority today” from “policy evaluator integration target.”
- [ ] Any MUST claims about currently unwired stack paths are either downgraded or backed by implementation tasks.
- [ ] Contradictions table in this report is closed or reduced to explicit follow-ups.

**Notes**: Keep doctrine unchanged unless contradiction requires doctrine edit.

##### Reconciliation: Chunk 1
- [ ] Spec text matches current runtime behavior and declared integration path.
- [ ] No contradictory MUST statements remain for v1 wiring status.

#### Chunk 2: Define policy wiring seam contract
**Objective**: Specify exact boundary: who builds `PolicyContext`, who executes outcomes, who owns mutable state.
**Spec reference**: `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
**Dependencies**: Chunk 1
**Why ordered here**: prevents duplicated or circular authority.
**Scope**: M
**Parallelizable**: No — core contract decision
**Serialize with**: Chunk 3-5

**Acceptance criteria**:
- [ ] Contract maps Level 0-6 inputs to concrete runtime data sources.
- [ ] Ownership matrix is explicit for budget/attention/safe-mode/resource counters.
- [ ] Failure/latency invariants are testable and tied to CI gates.

**Notes**: Base on existing boundaries in `crates/tze_hud_runtime/src/lib.rs`, `crates/tze_hud_runtime/src/budget.rs`, `crates/tze_hud_policy/src/lib.rs`.

##### Reconciliation: Chunk 2
- [ ] Contract document is consistent with crate-level authority comments.
- [ ] No ownership overlap across runtime and policy crates.

### Near-term delivery work

#### Chunk 3: Mutation-path pilot wiring
**Objective**: Route zone/tile mutation evaluation through `tze_hud_policy` mutation evaluator while preserving runtime state ownership.
**Spec reference**: `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:217-230`
**Dependencies**: Chunk 2
**Why ordered here**: highest-impact, easiest to isolate path.
**Scope**: L
**Parallelizable**: Partial — tests can be prepared in parallel
**Serialize with**: Chunk 4

**Acceptance criteria**:
- [ ] Runtime constructs `PolicyContext` snapshots for mutation evaluation.
- [ ] Runtime executes outcomes without moving mutable counters/state into policy crate.
- [ ] Existing safe-mode/freeze/capability semantics remain unchanged or explicitly updated in spec.

**Notes**: Avoid rewiring per-event/per-frame in same chunk.

##### Reconciliation: Chunk 3
- [ ] Behavior matches updated spec scenarios for mutation path.
- [ ] No frame-budget regressions beyond agreed thresholds.

#### Chunk 4: Telemetry and conformance harness for wired path
**Objective**: Emit per-level policy decision traces and benchmark p99/p95 against spec limits.
**Spec reference**: `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:195-227`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:155-170`
**Dependencies**: Chunk 3
**Why ordered here**: prove non-regression and observability immediately.
**Scope**: M
**Parallelizable**: Yes — telemetry + benchmark implementation can split
**Serialize with**: Chunk 5

**Acceptance criteria**:
- [ ] CI-visible outputs show policy decision distributions and latencies.
- [ ] Benchmarks assert per-mutation and frame-level budgets.
- [ ] Failures produce actionable diagnostics.

**Notes**: Integrate with existing telemetry pipeline and CI jobs.

##### Reconciliation: Chunk 4
- [ ] Telemetry schema/documentation matches emitted runtime fields.
- [ ] Budget assertions are deterministic in CI.

### Strategic future work

#### Chunk 5: Extend wiring to per-event and per-frame stacks
**Objective**: Complete remaining stack integration for event/frame paths.
**Spec reference**: `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:194-210`
**Dependencies**: Chunks 1-4
**Why ordered here**: only after mutation-path success and observability.
**Scope**: L
**Parallelizable**: No — shared runtime paths
**Serialize with**: all previous chunks

**Acceptance criteria**:
- [ ] Event/frame evaluation order and short-circuit semantics conform to spec.
- [ ] Safe-mode and override handling maintain one-frame guarantees.
- [ ] CI and integration tests cover these flows end-to-end.

**Notes**: Reassess post-v1 deferrals before committing full scope.

### Block Reconciliation: Policy Wiring Program
- [ ] All chunk acceptance criteria met.
- [ ] Updated specs accurately describe implementation.
- [ ] No unresolved authority contradictions remain.
- [ ] Follow-up debt tracked as beads, not TODO comments.

## 7. Do Not Do Yet

| Item | Reason | Revisit when |
|------|--------|-------------|
| Full dynamic policy-rule hot reload in v1 | Explicit post-v1 deferral and high churn | After core wiring and v1 stability achieved |
| Global policy facade rewrite across runtime/protocol/scene in one PR | Excessive blast radius and regression risk | After seam contract + mutation pilot are proven |
| Claiming policy-stack conformance without measured latency/telemetry proof | Violates evidence-first governance | After Chunk 4 harness lands |

## 8. Appendix

### A. Repository Map
- Language: Rust workspace
- Key modules:
  - Runtime orchestration: `crates/tze_hud_runtime`
  - Session transport handling: `crates/tze_hud_protocol/src/session_server.rs`
  - Scene validation/enforcement: `crates/tze_hud_scene/src/mutation.rs`, `graph.rs`
  - Policy evaluator: `crates/tze_hud_policy`
  - Doctrine/spec: `about/heart-and-soul`, `openspec/changes/v1-mvp-standards/specs`

### B. Critical Workflows
1. **MutationBatch intake**: ClientMessage -> session server safe-mode/freeze/timing checks -> scene `apply_batch` lease/budget/invariant checks -> MutationResult (`crates/tze_hud_protocol/src/session_server.rs:1813-2100`, `crates/tze_hud_scene/src/mutation.rs:320-520`).
2. **Zone publish**: Client ZonePublish -> conversion -> scene publish batch path -> durable/ephemeral ack semantics (`crates/tze_hud_protocol/src/session_server.rs:3647-3788`, `crates/tze_hud_scene/src/graph.rs:2866-3096`).
3. **Capability escalation**: CapabilityRequest -> policy_capabilities evaluation -> CapabilityNotice or RuntimeError(PERMISSION_DENIED) (`crates/tze_hud_protocol/src/session_server.rs:3440-3512`).
4. **Budget/safe-mode governance**: runtime budget ladder + safe-mode shared flag gates mutation acceptance (`crates/tze_hud_runtime/src/budget.rs:1-35`, `crates/tze_hud_protocol/src/session_server.rs:1950-1977`).

### C. Spec Inventory
| Spec | Coverage status |
|------|-----------------|
| `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md` | Partially implemented; key wiring drift |
| `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md` | Largely implemented with known policy-authority ambiguity |
| `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` | Largely implemented for transport semantics; policy-source semantics partial |
| `about/heart-and-soul/v1.md` | Directionally aligned with sovereign runtime doctrine |

### D. Evidence Index
- `about/heart-and-soul/README.md:3-6`
- `about/heart-and-soul/v1.md:11-21`
- `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:10-12`
- `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:194-230`
- `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:368-374`
- `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:10-13`
- `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:155-170`
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:615-636`
- `crates/tze_hud_runtime/src/lib.rs:12-26`
- `crates/tze_hud_runtime/src/budget.rs:14-35`
- `crates/tze_hud_runtime/src/attention_budget/mod.rs:35-55`
- `crates/tze_hud_protocol/src/session_server.rs:1812-1862`
- `crates/tze_hud_protocol/src/session_server.rs:1944-2100`
- `crates/tze_hud_protocol/src/session_server.rs:3440-3512`
- `crates/tze_hud_protocol/src/session_server.rs:3647-3788`
- `crates/tze_hud_scene/src/mutation.rs:320-520`
- `crates/tze_hud_scene/src/graph.rs:571-602`
- `.github/workflows/ci.yml:99-184`
- `AGENTS.md:226`

---

## Conclusion

**Real direction**: Build a seam-explicit governance model where runtime remains state owner and `tze_hud_policy` becomes a progressively wired pure decision engine, with specs updated first to reflect current reality.

**Work on next**: (1) reconcile policy/runtime/session specs, (2) define formal seam contract, (3) implement mutation-path pilot wiring plus telemetry/latency conformance harness.

**Stop pretending**: The current v1 runtime already executes the full seven-level arbitration stack end-to-end in hot paths.
