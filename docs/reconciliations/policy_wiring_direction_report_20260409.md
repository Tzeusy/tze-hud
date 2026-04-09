# Policy Wiring Direction Report (Gen-2)

Date: 2026-04-09
Scope: `/project-direction` reconciliation refresh for policy wiring
Issue: `hud-iq2x`
Execution brief: `docs/reconciliations/policy_wiring_epic_prompt.md`

## 1. Executive Summary

[Observed] Runtime docs and module boundaries still declare `tze_hud_policy` as an unwired pure evaluator in v1, while policy OpenSpec still carries v1-mandatory full-stack enforcement claims (`crates/tze_hud_runtime/src/lib.rs:5-26`, `crates/tze_hud_runtime/src/budget.rs:14-35`, `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:11-22`, `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:217-230`).

[Observed] The tractable path is still incremental and seam-first: (1) spec reconciliation to current truth, (2) explicit runtime-policy-scene ownership contract, (3) mutation-path pilot wiring, (4) telemetry + latency conformance, then (5) scope decision for event/frame wiring (`about/heart-and-soul/architecture.md:1-17`, `about/heart-and-soul/v1.md:11-21`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:59-73`).

[Observed] Immediate blockers are governance seams, not rendering mechanics: lease capability scope authority and capability escalation source semantics must be resolved before broader policy wiring (`crates/tze_hud_protocol/src/session_server.rs:2844-3004`, `crates/tze_hud_protocol/src/session_server.rs:3455-3527`, `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:616-636`, `about/law-and-lore/rfcs/0008-lease-governance.md:101-109`).

## 2. Contradictions and Tractable Path

### Active Contradictions

1. [Observed] **Spec-over-code claim drift:** policy spec requires seven-level v1 stack execution across frame/event/mutation, but runtime module authority notes still state policy crate is not wired (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:195-230`, `crates/tze_hud_runtime/src/lib.rs:12-26`).
2. [Observed] **Authority location drift:** policy spec implies centralized policy decisions; code ownership is split across runtime stateful enforcers, scene validation contracts, and policy pure evaluators (`crates/tze_hud_runtime/src/budget.rs:18-35`, `crates/tze_hud_runtime/src/attention_budget/mod.rs:35-55`, `crates/tze_hud_scene/src/policy/mod.rs:1-7`, `crates/tze_hud_policy/src/lib.rs:31-46`).
3. [Observed] **Lease scope seam bug surface:** lease request path forwards requested lease capabilities without explicit subset enforcement against session-granted capabilities in the request handler (`crates/tze_hud_protocol/src/session_server.rs:2946-2969`), while governance requirements require lease scope to remain bounded by session-granted authority (`about/law-and-lore/rfcs/0008-lease-governance.md:101-109`).

### Tractable Path (No Big-Bang Rewrite)

1. Reconcile policy/runtime/session spec claims to current runtime truth first.
2. Define explicit runtime-policy-scene seam contract with ownership matrix.
3. Resolve lease capability scope and capability escalation semantics as governance prerequisites.
4. Wire mutation-path policy evaluation only, preserving runtime state ownership.
5. Add policy outcome telemetry + latency gates, then decide event/frame v1 scope.

## 3. Exact Ownership Seams (Runtime vs Policy vs Scene)

| Surface | Owns | Must Not Own | Evidence |
|---|---|---|---|
| `tze_hud_runtime` | Stateful enforcement ladders, counters, safe-mode/freeze writes, pipeline orchestration | Pure policy truth as side-effect-free evaluator implementation detail | `crates/tze_hud_runtime/src/lib.rs:5-29`, `crates/tze_hud_runtime/src/budget.rs:18-35`, `crates/tze_hud_runtime/src/shell/safe_mode.rs:5-13` |
| `tze_hud_policy` | Pure snapshot evaluation (`PolicyContext -> ArbitrationOutcome`) and level semantics | Mutable runtime/session/lease/resource state writes | `crates/tze_hud_policy/src/lib.rs:23-46` |
| `tze_hud_scene::policy` | Scene-side trait contract for arbitration-level semantics and type-level ordering | Runtime ladder state, shell override writes, transport/session authority | `crates/tze_hud_scene/src/policy/mod.rs:1-7`, `crates/tze_hud_scene/src/policy/mod.rs:170-204` |
| `tze_hud_protocol::session_server` | Session handshake, capability grant/revoke transport semantics, lease request plumbing | Independent policy authority that bypasses runtime/policy contract | `crates/tze_hud_protocol/src/session_server.rs:1811-1892`, `crates/tze_hud_protocol/src/session_server.rs:3455-3527` |

### Seam Contract Requirements (Gen-2)

1. Runtime remains the sole writer of safe-mode/freeze and budget state transitions.
2. Policy crate remains pure and side-effect free.
3. Scene-side policy contract remains the schema/type contract for arbitration semantics, not a second state machine.
4. Session-server capability/lease handlers must be constrained by session-granted authority before any broader wiring.

## 4. Bead Decomposition State

### Already Materialized Under `hud-iq2x`

| Child bead | Role in plan | Status |
|---|---|---|
| `hud-iq2x.5` | Lease capability scope fix (governance seam first) | closed |
| `hud-iq2x.6` | Spec reconciliation across policy/runtime/session | closed |
| `hud-iq2x.7` | Runtime-policy-scene seam contract + ownership matrix | blocked (PR-linked) |
| `hud-iq2x.8` | Capability-escalation policy source semantics | closed |
| `hud-ew7a` | Event/frame v1 scope decision after mutation pilot telemetry | in_progress |

### Remaining Follow-On Beads Needed (Pending Instantiation)

1. Mutation-path pilot wiring via `tze_hud_policy` (depends on `.7` and `.8`).
2. Policy telemetry + latency conformance harness (depends on mutation pilot).
3. Event/frame scope decision bead (depends on telemetry outcome).

### Post-Pilot Reconciliation Instantiated

- Post-pilot reconciliation is now instantiated as `hud-p48t` (depends on mutation pilot + telemetry + `hud-ew7a` + `.6`).

### Closure Bead Instantiated

- `hud-5fb1` now tracks the final human signoff report closure artifact.
- This bead should execute only after `hud-p48t` closes.

## 5. Risks and Stop/Do-Not-Do-Yet

### Top Risks

1. Latency regressions if policy wiring is attempted before seam ownership is explicit.
2. Governance regressions if lease capability scope remains broader than session grant authority.
3. Spec credibility erosion if v1 MUST claims remain ahead of shipped behavior.

### Do Not Do Yet

| Item | Why |
|---|---|
| Big-bang rewrite that routes all frame/event/mutation paths through new policy facade at once | High churn and weak rollback boundaries |
| Event/frame wiring before mutation pilot telemetry proves budget conformance | Violates validation-first doctrine (`about/heart-and-soul/validation.md:1-20`) |
| Claims that full seven-level hot-path wiring is done in v1 | Contradicted by runtime authority docs (`crates/tze_hud_runtime/src/lib.rs:12-26`) |

---

## Conclusion

**Real direction**: Preserve runtime sovereignty and state ownership while incrementally wiring policy as a pure evaluation path with explicit runtime-policy-scene contracts.

**Work on next**: (1) finish seam contract bead `hud-iq2x.7`, (2) instantiate mutation pilot + telemetry beads, (3) carry `hud-ew7a` through go/no-go closure and then reconcile/sign off.

**Stop pretending**: v1 does not currently run full seven-level arbitration end-to-end in runtime hot paths.
