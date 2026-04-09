# Policy Wiring Seam Contract and Ownership Matrix

Date: 2026-04-09  
Issue: `hud-iq2x.7`  
Depends on: `hud-iq2x.6` (spec-language reconciliation)

## 1. Purpose
Define the pre-implementation contract for policy wiring so `PW-03` can integrate `tze_hud_policy` without duplicating authority or moving mutable runtime state into pure evaluators.

This contract covers all three policy surfaces:

1. Direct runtime enforcement in `crates/tze_hud_runtime`
2. Pure evaluators in `crates/tze_hud_policy`
3. Scene-side policy contract in `crates/tze_hud_scene/src/policy`

## 2. Canonical vs Transitional Abstractions

| Abstraction | Location | Status | Contract |
|---|---|---|---|
| Runtime authority modules (`BudgetEnforcer`, `AttentionBudgetTracker`, `SafeModeController`, freeze queue) | `crates/tze_hud_runtime` | Canonical (v1 live) | Own and mutate enforcement state; runtime remains sovereign writer. |
| Lease/capability and contention enforcement in scene graph | `crates/tze_hud_scene` | Canonical (v1 live) | Owns lease state machine, zone occupancy, contention policy, and mutation validation pipeline. |
| `PolicyContext` and `ArbitrationOutcome` data model | `crates/tze_hud_policy/src/types.rs` | Canonical (wiring target) | Read-only snapshot in, decision out. Evaluators MUST NOT perform side effects. |
| Per-mutation/per-frame/per-event evaluators | `crates/tze_hud_policy/src/mutation.rs`, `frame.rs`, `event.rs`, `stack.rs` | Canonical (wiring target) | Runtime constructs context and executes results; evaluators MUST NOT retain mutable runtime authority state. |
| `tze_hud_scene::policy::{PolicyContext, PolicyDecision, ArbitrationAction, PolicyEvaluator}` | `crates/tze_hud_scene/src/policy/mod.rs` | Transitional | Scene-side contract/test surface only; not runtime-wired authority. Keep for seam clarity, not as an additional state owner. |

Decision: `tze_hud_policy` types are the canonical policy abstraction for wiring work; `tze_hud_scene::policy` remains transitional until an explicit consolidation/deprecation bead lands. Stateful helpers inside `tze_hud_policy` (for example `override_queue.rs`) are orchestration utilities, not policy authority owners.

## 3. Level-by-Level Input-Source Mapping (0-6)

| Level | Policy input fields | Primary input source(s) | Owner surface |
|---|---|---|---|
| 0 Human Override | `override_state.freeze_active`, `override_state.safe_mode_active`, freeze/safe-mode timing | `SafeModeController` + shell freeze manager/queue (`crates/tze_hud_runtime/src/shell/safe_mode.rs`, `shell/freeze.rs`); override queue drain contract (`tze_hud_policy/src/override_queue.rs`) | Runtime shell |
| 1 Safety | `safety_state.gpu_healthy`, `scene_graph_intact`, `frame_time_p95_us`, thresholds | Runtime GPU/thread health and frame-time guardian (`crates/tze_hud_runtime/src/lib.rs`, `budget.rs`), scene integrity/invariant checks (`crates/tze_hud_scene/src/mutation.rs`, `validation.rs`) | Runtime + scene |
| 2 Privacy | `privacy_context.effective_viewer_class`, viewer set, redaction style, content classification ceiling inputs | Shell viewer/redaction state (`crates/tze_hud_runtime/src/shell/chrome.rs`, `shell/redaction.rs`) plus zone/tile classification and zone ceiling sources (`crates/tze_hud_scene/src/types.rs`, zone publish payload) | Runtime shell + scene |
| 3 Security | `security_context.granted_capabilities`, `agent_namespace`, `lease_valid`, lease id | Session auth/capability policy + canonical capability validation (`crates/tze_hud_protocol/src/auth.rs`, `session_server.rs`), scene lease state/capability gate (`crates/tze_hud_scene/src/lease/capability.rs`, `graph.rs`) | Runtime/protocol + scene |
| 4 Attention | `attention_context` quiet-hours flags, interruption class, rolling counters, limits | Runtime event pipeline and quiet-hours path (`crates/tze_hud_runtime/src/event_bus.rs`, `quiet_hours`, `attention_budget/mod.rs`); policy ring-buffer mirror for pure eval (`crates/tze_hud_policy/src/attention_budget.rs`) | Runtime (state owner), policy (pure evaluator) |
| 5 Resource | `resource_context.degradation_level`, `budget_exceeded`, `should_shed`, `budgets_paused`, transactional flag | Runtime budget ladder and frame guardian (`crates/tze_hud_runtime/src/budget.rs`), lease/resource limits from scene (`crates/tze_hud_scene/src/mutation.rs`, lease budgets) | Runtime + scene |
| 6 Content | `content_context.zone_name`, contention policy, occupancy/priority state | Scene zone registry + contention resolution + lease priorities (`crates/tze_hud_scene/src/graph.rs`, `types.rs`) | Scene |

Rule: every `PolicyContext` field must be backed by exactly one mutable owner at write time; policy evaluators only consume snapshots.

## 4. Ownership Matrix (Runtime / Policy / Scene)

| Mutable state family | Runtime (`tze_hud_runtime`) | Policy (`tze_hud_policy`) | Scene (`tze_hud_scene` and `tze_hud_scene::policy`) |
|---|---|---|---|
| Safe mode / freeze lifecycle | Sole writer (`SafeModeController`, freeze manager); enforces shell invariant | Read-only via `OverrideState`; MUST NOT transition shell state | Scene sees resulting lease states; transitional policy trait may model checks only |
| Budget ladder and resource counters | Sole writer (`BudgetEnforcer`) for warning/throttle/revoke, guardian | Pure read of `ResourceContext`; no counters owned | Scene enforces per-batch resource validation and lease-scoped limits |
| Attention rolling state and quiet-hours queues | Sole writer (`AttentionBudgetTracker`, quiet-hours queues) | Pure read using `AttentionContext`/ring-buffer snapshots | Scene carries interruption metadata; no attention counter ownership |
| Capability grants/revocations and session auth scope | Writer via protocol/runtime context and capability policy | Pure capability checks on snapshot sets | Scene enforces lease capability gates at mutation/publish time |
| Lease state machine (ACTIVE/SUSPENDED/...) | Triggers transitions through protocol/shell actions | Reads lease validity only | Sole owner of lease records and transitions in scene graph |
| Zone contention occupancy + lease-priority ordering | Consumes outcomes for rendering/degradation behavior | Pure decision path for Level 6 logic | Sole owner of zone registry occupancy and contention semantics |
| Privacy redaction render behavior | Shell owns render-path redaction application and viewer privacy boundary | Produces redaction decisions (`CommitRedacted`/redacted queue) from snapshots | Scene stores classifications/payload context; transitional `scene::policy` remains contract-only |

## 5. `PolicyContext` Construction Contract

The runtime wiring path MUST satisfy all rules below.

1. Construction boundary:
   `PolicyContext` is built on runtime hot paths (per-frame, per-event, per-mutation) after ingesting current authoritative runtime/scene state and before policy evaluation.
2. Snapshot-only input:
   the builder copies/scalars/snapshot views into `PolicyContext`; it does not hand mutable owners to the evaluator.
3. Level consistency:
   each level's fields must come from the level owner mapped in Section 3. No synthetic duplicate owner is allowed.
4. Purity boundary:
   `tze_hud_policy` evaluation functions remain side-effect free. They cannot write freeze flags, budget ladders, queues, lease records, or zone occupancy.
5. Threading discipline:
   context assembly and outcome execution stay in runtime-owned orchestration lanes; policy crate is compute-only.
6. Transitional compatibility:
   if scene-side `policy` traits are used for tests/documentation, they are adapters over canonical `tze_hud_policy` semantics, not parallel authority.

Reference shape (non-normative):

```rust
let ctx = PolicyContext {
    override_state: /* shell snapshot */,
    safety_state: /* runtime + scene integrity snapshot */,
    privacy_context: /* viewer + classification snapshot */,
    security_context: /* capabilities + lease validity snapshot */,
    attention_context: /* quiet-hours + rolling budget snapshot */,
    resource_context: /* degradation + budget snapshot */,
    content_context: /* zone contention snapshot */,
};
```

## 6. `ArbitrationOutcome` Execution Contract

`ArbitrationOutcome` is a decision type. Runtime/scene code executes it.

| Outcome | Execution contract | Owner executing side effects |
|---|---|---|
| `Commit` | Apply mutation through existing atomic scene pipeline (`SceneGraph::apply_batch`) and return success ack/event path. | Runtime/protocol + scene |
| `CommitRedacted` | Commit mutation/state exactly as `Commit`; additionally mark/redraw through shell redaction path so presentation is filtered, not state-skipped. | Runtime shell + scene |
| `Queue { ... }` | Enqueue for deferred delivery (quiet-hours/freeze semantics) using existing runtime queue mechanisms; preserve atomic ordering/coalescing guarantees. | Runtime |
| `Reject(error)` | Do not mutate scene state; emit structured error response/telemetry with level+code and correlation identifiers. | Runtime/protocol |
| `Shed { ... }` | Preserve authoritative state where spec requires (for example occupancy updates), but omit render output according to degradation policy. No agent-facing hard error. | Runtime + scene |
| `Blocked { block_reason: BlockReason::Freeze }` | Treat as Level 0 freeze queueing contract (not an authorization denial). Delivery resumes on unfreeze/timeout path. | Runtime shell |

Execution invariants:

1. Policy crate never performs side effects.
2. Runtime does not reinterpret outcomes into contradictory semantics (for example, `Blocked` must not be emitted as `Reject`).
3. Scene graph remains the authoritative state transition engine for committed mutations.

## 7. Seam Guards for Follow-On Beads

Before `PW-03` wiring lands, code review must reject changes that:

1. Introduce new mutable policy state into `tze_hud_policy`.
2. Duplicate runtime budget/attention/freeze state machines inside evaluators.
3. Treat `tze_hud_scene::policy` as a second live authority instead of a transitional contract surface.
4. Skip explicit `PolicyContext` field provenance or `ArbitrationOutcome` execution mapping.

This document is the required seam contract for policy wiring implementation beads.
