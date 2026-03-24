# Epic 8: Policy Arbitration

> **Dependencies:** Epic 0 (PolicyEvaluator trait contract), Epic 4 (lease priority), Epic 7 (capabilities, quiet hours)
> **Depended on by:** Epic 9 (events filtered by policy), Epic 11 (shell invokes policy for overrides)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
> **Secondary specs:** `lease-governance/spec.md` (priority), `configuration/spec.md` (capabilities, quiet hours)

## Prompt

Create a `/beads-writer` epic for **policy arbitration** — the fixed 7-level precedence stack that resolves conflicts between human overrides, safety, privacy, security, attention, resources, and content.

### Context

Policy arbitration turns the platform's "civic law" into a real contract. The 7-level stack is fixed and exception-free (freeze/safe-mode interaction is owned by system-shell, not policy). Policy evaluation must be a pure function over a typed PolicyContext — no side effects. Epic 0 provides the `PolicyEvaluator` trait contract. The spec defines separate per-mutation, per-event, and per-frame evaluation pipelines.

### Epic structure

Create an epic with **4 implementation beads**:

#### 1. Seven-level arbitration stack (depends on Epic 0 PolicyEvaluator contract)
Implement the fixed stack per `policy-arbitration/spec.md` Requirement: Arbitration Stack.
- Level 0: Human Override, Level 1: Safety, Level 2: Privacy, Level 3: Security, Level 4: Attention, Level 5: Resource, Level 6: Content
- Ordering is doctrine — MUST NOT be modified
- Higher-numbered levels cannot override lower-numbered levels
- Short-circuit: if a higher level decides, lower levels are not evaluated
- **Acceptance:** PolicyEvaluator trait tests from Epic 0 pass. Stack ordering enforced. Short-circuit behavior verified. `policy_matrix_basic` and `policy_arbitration_collision` test scenes pass.
- **Spec refs:** `policy-arbitration/spec.md` Requirement: Arbitration Stack, lines 10-17

#### 2. Per-mutation evaluation pipeline (depends on #1, Epic 4 capabilities)
Implement per-mutation policy checks per `policy-arbitration/spec.md` Requirement: Per-Mutation Evaluation.
- Every mutation checked against: capability scope, privacy/viewer context, resource budget
- Per-mutation latency < 50µs
- Batch of 64 mutations: total policy check < 3.2ms
- Rejected mutations produce structured error with level, code, and correction hint
- **Acceptance:** Per-mutation latency budget met. Capability violations rejected with correct error. Privacy redaction applied when viewer context requires it.
- **Spec refs:** `policy-arbitration/spec.md` Requirement: Per-Mutation Evaluation, Requirement: Capability Registry Canonical Names

#### 3. Per-event and per-frame evaluation (depends on #1)
Implement event and frame evaluation per `policy-arbitration/spec.md` Requirement: Per-Event Evaluation, Requirement: Per-Frame Evaluation.
- Per-event: Level 0 → Level 4 → Level 3 during input drain and local feedback
- Per-frame: Level 1 → Level 2 → Level 5 → Level 6 at frame start before mutation intake
- If Level 1 triggers safe mode during per-frame: Levels 2/5/6 not evaluated
- If Level 0 triggers safe mode during per-event: Levels 4/3 stop
- **Acceptance:** Per-frame evaluation order verified. Safe mode short-circuit verified. Per-event ordering verified. Full-frame evaluation < 200µs.
- **Spec refs:** `policy-arbitration/spec.md` Requirement: Per-Event Evaluation, Requirement: Per-Frame Evaluation

#### 4. Attention budget and quiet hours (depends on #1, Epic 7 config)
Implement attention management per `policy-arbitration/spec.md` Requirement: Attention Budget.
- Interruption classes: CRITICAL/HIGH/NORMAL/LOW/SILENT
- Quiet hours: CRITICAL and HIGH pass through; NORMAL and LOW queued until quiet hours end
- Rate limiting: per-agent and per-zone rolling 60-second windows
- SILENT never interrupts, always passes quiet hours
- **Acceptance:** Interruption filtering matches class. Quiet hours queue NORMAL/LOW correctly. Rate limiting enforces per-agent/per-zone counters. Budget tests pass.
- **Spec refs:** `policy-arbitration/spec.md` Requirement: Attention Budget, `scene-events/spec.md` Requirement: Interruption Classification

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `policy-arbitration/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference exact spec scenarios
3. **Acceptance criteria** — which Epic 0 PolicyEvaluator tests must pass
4. **Crate/file location** — new policy module, ideally as pure functions
5. **Purity constraint** — policy evaluation must be side-effect-free; state transitions belong to shell/lease modules

### Dependency chain

```
Epics 0+4+7 ──→ #1 Arbitration Stack ──→ #2 Per-Mutation
                                      ──→ #3 Per-Event/Frame
                                      ──→ #4 Attention/Quiet Hours
```
