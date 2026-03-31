# Epic 4: Lease Governance

> **Dependencies:** Epic 0 (trait contracts for LeaseStateMachine), Epic 1 (scene graph, namespace)
> **Depended on by:** Epics 5, 6, 8, 11 (input needs lease checks, session manages leases, policy references leases, shell suspends leases)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md`
> **Secondary specs:** `scene-graph/spec.md` (namespace), `session-protocol/spec.md` (lease messages)

## Prompt

> **Before starting:** Read `docs/prompts/PREAMBLE.md` for authority rules, doctrine guardrails, and v1 scope tagging requirements that apply to every bead.

Create a `/beads-writer` epic for **lease governance** — the state machine that governs every agent's presence on screen. No agent can mutate the scene without a valid lease; leases have TTL, priority, capability scope, and revocation semantics.

### Context

Lease governance is the spec reviewers' highest-rated subsystem (9.2/10). It defines the lease state machine, suspension/resume during safe mode, TTL pause, orphan handling, and resource budget enforcement. Epic 0 provides the `LeaseStateMachine` trait contract with test harnesses encoding the spec's WHEN/THEN scenarios. The existing `crates/tze_hud_scene/src/graph.rs` has partial lease tracking that should be extracted into a dedicated module.

### Epic structure

Create an epic with **5 implementation beads**:

#### 1. Lease state machine (depends on Epic 0 trait contract, Epic 1 identity types)
Implement the core state machine per `lease-governance/spec.md` Requirement: Lease State Machine.
- States: REQUESTED, ACTIVE, SUSPENDED, ORPHANED, REVOKED, EXPIRED, DENIED, RELEASED
- Valid transitions enumerated in spec; all others rejected
- Each transition produces an auditable event
- **Acceptance:** All `LeaseStateMachine` trait contract tests from Epic 0 pass. Every valid transition verified. Every invalid transition rejected with structured error. `lease_expiry` test scene passes.
- **Spec refs:** `lease-governance/spec.md` Requirement: Lease State Machine, lines 15-45

#### 2. TTL, renewal, and expiration (depends on #1)
Implement lease lifecycle timing per `lease-governance/spec.md` Requirement: Lease TTL and Renewal.
- Auto-renewal policies: MANUAL, AUTO_RENEW, ONE_SHOT
- TTL pause during suspension (safe mode): elapsed suspension time excluded from TTL
- Expiration triggers EXPIRED transition and resource cleanup
- `adjusted_expires_at_wall_us` on resume
- **Acceptance:** TTL countdown verified with `TestClock`. Suspension pauses TTL correctly. Auto-renewal fires before expiry. ONE_SHOT leases expire without renewal.
- **Spec refs:** `lease-governance/spec.md` Requirement: Lease TTL and Renewal, Requirement: TTL Pause During Suspension

#### 3. Disconnect grace and orphan handling (depends on #1, #2)
Implement graceful disconnect behavior per `lease-governance/spec.md` Requirement: Disconnect Grace Period.
- On ungraceful disconnect: leases enter ORPHANED state with grace period (default 15s)
- During grace: tiles remain visible with staleness badge, mutations rejected
- If agent reconnects within grace: leases restored to ACTIVE
- If grace expires: leases transition to RECLAIMED, resources freed
- **Acceptance:** `disconnect_reclaim_multiagent` test scene passes. Grace period timing verified. Reconnection within grace restores lease. Expiry after grace frees resources.
- **Spec refs:** `lease-governance/spec.md` Requirement: Disconnect Grace Period, Requirement: Orphan Handling

#### 4. Priority and capability scope (depends on #1)
Implement lease priority and capability enforcement per `lease-governance/spec.md` Requirement: Lease Priority, Requirement: Capability Scope.
- Static priority 0-255 (0 = highest); guides degradation shedding order
- Capability scope: `create_tiles`, `modify_own_tiles`, `manage_tabs`, `publish_zone:*`, `read_scene_topology`, `resident_mcp` (per canonical vocabulary from `configuration/spec.md`)
- Mutations outside capability scope rejected with structured error
- **Acceptance:** Priority-based shedding order matches spec. Capability enforcement rejects unauthorized mutations. `three_agents_contention` test scene passes.
- **Spec refs:** `lease-governance/spec.md` Requirement: Lease Priority, Requirement: Capability Scope, `configuration/spec.md` Requirement: Capability Vocabulary

#### 5. Resource budget enforcement per lease (depends on #1, #4)
Implement per-agent resource governance per `lease-governance/spec.md` Requirement: Resource Budget Enforcement.
- Three-tier ladder: Warning (limit exceeded) → Throttle (unresolved 5s, reduce update rate 50%) → Revocation (sustained 30s)
- Per-agent budgets: max_tiles, max_texture_bytes, max_update_rate_hz, max_nodes_per_tile
- Shared resources double-counted per agent (anti-collusion)
- **Note:** Guest MCP zone publishing does NOT require a lease — guests publish to zones without lease acquisition per `session-protocol/spec.md` Requirement: MCP Bridge Guest Tools
- **Acceptance:** Three-tier ladder transitions verified with time injection. Budget violations produce correct warnings/throttles/revocations. Budget tests from `budget.rs` pass. Guest zone publishing verified lease-free.
- **Spec refs:** `lease-governance/spec.md` Requirement: Resource Budget Enforcement, `runtime-kernel/spec.md` Requirement: Per-Agent Resource Budgets

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `lease-governance/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference the exact spec scenarios
3. **Acceptance criteria** — which Epic 0 trait contract tests must pass
4. **Crate/file location** — extract from `graph.rs` into dedicated `crates/tze_hud_scene/src/lease.rs` or similar
5. **State machine diagram** — reference the spec's state transition table

### Dependency chain

```
Epic 0 + Epic 1 ──→ #1 State Machine ──→ #2 TTL/Renewal ──→ #3 Disconnect Grace
                                      ──→ #4 Priority/Caps ──→ #5 Budget Enforcement
```
