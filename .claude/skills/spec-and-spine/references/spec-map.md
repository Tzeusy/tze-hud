# Spec Map — Dependency Graph, File Paths, and Task Sections

## Dependency Graph

```
scene-graph (FOUNDATION)
├── runtime-kernel
├── timing-model
├── input-model
└── session-protocol
      ├── configuration
      ├── system-shell
      ├── lease-governance
      └── policy-arbitration
            └── scene-events

resource-store (ORTHOGONAL — no upstream dependencies beyond scene-graph)

validation-framework (CROSS-CUTTING — references all 11 domain specs)
```

**Implementation order:** Foundation layer first (scene-graph is the root dependency), then hot-path layer, then governance, then events/storage/validation. Within a layer, specs with fewer dependencies come first.

**Recommended sequencing:**
1. Scene Graph → Runtime Kernel → Timing Model (foundation, serial — shared scene primitives)
2. Input Model, Session Protocol (hot path, can partially parallelize after foundation)
3. Configuration, System Shell, Lease Governance (governance, after session protocol)
4. Policy Arbitration → Scene Events (arbitration feeds events)
5. Resource Store (can parallelize with governance layer)
6. Validation Framework (last — depends on all other specs for test generation)

## Full File Paths

All spec paths relative to repo root:

| Domain | Spec | RFC | Tasks section |
|--------|------|-----|---------------|
| Scene Graph | `openspec/changes/v1-mvp-standards/specs/scene-graph/spec.md` | `docs/rfcs/0001-scene-contract.md` | Section 1 (tasks 1.1–1.11) |
| Runtime Kernel | `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md` | `docs/rfcs/0002-runtime-kernel.md` | Section 3 (tasks 3.1–3.11) |
| Timing Model | `openspec/changes/v1-mvp-standards/specs/timing-model/spec.md` | `docs/rfcs/0003-timing.md` | Section 2 (tasks 2.1–2.8) |
| Input Model | `openspec/changes/v1-mvp-standards/specs/input-model/spec.md` | `docs/rfcs/0004-input.md` | Section 4 (tasks 4.1–4.9) |
| Session Protocol | `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` | `docs/rfcs/0005-session-protocol.md` | Section 5 (tasks 5.1–5.12) |
| Configuration | `openspec/changes/v1-mvp-standards/specs/configuration/spec.md` | `docs/rfcs/0006-configuration.md` | Section 6 (tasks 6.1–6.8) |
| System Shell | `openspec/changes/v1-mvp-standards/specs/system-shell/spec.md` | `docs/rfcs/0007-system-shell.md` | Section 8 (tasks 8.1–8.8) |
| Lease Governance | `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md` | `docs/rfcs/0008-lease-governance.md` | Section 7 (tasks 7.1–7.8) |
| Policy Arbitration | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md` | `docs/rfcs/0009-policy-arbitration.md` | Section 9 (tasks 9.1–9.10) |
| Scene Events | `openspec/changes/v1-mvp-standards/specs/scene-events/spec.md` | `docs/rfcs/0010-scene-events.md` | Section 10 (tasks 10.1–10.8) |
| Resource Store | `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md` | `docs/rfcs/0011-resource-store.md` | Section 11 (tasks 11.1–11.9) |
| Validation Framework | `openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md` | (cross-cutting) | Section 12 (tasks 12.1–12.10) |

**Integration tasks:** Section 13 (tasks 13.1–13.5) covers cross-subsystem convergence — not tied to a single spec.

## Cross-Subsystem Boundaries

Key integration points where multiple specs interact:

| Boundary | Specs involved | What crosses |
|----------|---------------|--------------|
| Scene mutation → lease check | Scene Graph + Lease Governance | Every mutation validates lease state before applying |
| Session handshake → capability negotiation | Session Protocol + Configuration | Session establishes capabilities from config profiles |
| Input dispatch → focus tree | Input Model + Scene Graph | Hit-testing walks the scene graph; focus is a scene property |
| Policy arbitration → lease priority | Policy Arbitration + Lease Governance | Arbitration stack uses lease priority levels |
| Event routing → subscription model | Scene Events + Session Protocol | Events dispatched through session streams |
| Timer expiry → lease lifecycle | Timing Model + Lease Governance | Clock drives TTL enforcement and heartbeat deadlines |
| Resource upload → content addressing | Resource Store + Scene Graph | Nodes reference ResourceIds for images, fonts |
| Shell controls → lease revocation | System Shell + Lease Governance | Freeze/dismiss triggers lease state transitions |
| Validation → all specs | Validation Framework + all | Test layers map to spec scenarios |

## Requirement Count by Domain

| Domain | v1-mandatory | v1-reserved | Scenarios |
|--------|-------------|-------------|-----------|
| Scene Graph | ~45 | ~8 | ~65 |
| Runtime Kernel | ~30 | ~5 | ~45 |
| Timing Model | ~25 | ~6 | ~40 |
| Input Model | ~28 | ~4 | ~42 |
| Session Protocol | ~35 | ~7 | ~55 |
| Configuration | ~22 | ~5 | ~35 |
| System Shell | ~20 | ~4 | ~32 |
| Lease Governance | ~25 | ~5 | ~38 |
| Policy Arbitration | ~22 | ~4 | ~35 |
| Scene Events | ~20 | ~5 | ~32 |
| Resource Store | ~18 | ~4 | ~28 |
| Validation Framework | ~25 | ~3 | ~40 |
| **Total** | **~315** | **~60** | **~487** |

These are approximate counts. Read individual specs for authoritative requirement lists.
