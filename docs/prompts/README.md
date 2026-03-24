# v1-mvp-standards Implementation Epics

Each file is a self-contained prompt for creating a `/beads-writer` epic. The epic numbering reflects the dependency order.

## Epic Dependency Graph

```
Epic 0: Test Infrastructure ─────────────────────────────────────────────┐
    │                                                                    │
    ├──→ Epic 1: Scene Graph Core ───────────────────────────────────────┤
    │        │                                                           │
    │        ├──→ Epic 2: Runtime Kernel + Compositor ───────────────────┤
    │        │        │                                                  │
    │        ├──→ Epic 3: Timing Model ──────────────────────────────────┤
    │        │        │                                                  │
    │        ├──→ Epic 4: Lease Governance ──────────────────────────────┤
    │        │        │                                                  │
    │        │        ├──→ Epic 5: Input Model ──────────────────────────┤
    │        │        │     (also depends on Epics 2, 3)                 │
    │        │        │                                                  │
    │        │        ├──→ Epic 6: Session Protocol ─────────────────────┤
    │        │        │     (also depends on Epics 1, 3, 4)              │
    │        │        │        │                                         │
    │        │        │        ├──→ Epic 7: Configuration ───────────────┤
    │        │        │        │     (also depends on Epic 4)            │
    │        │        │        │        │                                │
    │        │        │        │        ├──→ Epic 8: Policy Arbitration ─┤
    │        │        │        │        │     (also depends on Epic 4)   │
    │        │        │        │        │                                │
    │        │        │        │        ├──→ Epic 9: Scene Events ───────┤
    │        │        │        │        │     (also depends on Epic 8)   │
    │        │        │        │        │                                │
    │        │        ├──→ Epic 10: Resource Store ──────────────────────┤
    │        │        │     (also depends on Epics 1, 6)                 │
    │        │        │                                                  │
    │        │        ├──→ Epic 11: System Shell ────────────────────────┤
    │        │              (also depends on Epics 2, 4, 8)              │
    │        │                                                           │
    └────────┴──────────────────→ Epic 12: Integration + Convergence ────┘
                                  (depends on ALL epics 1–11)
```

## Parallelization Strategy

| Phase | Epics | Can run in parallel |
|-------|-------|-------------------|
| **Phase 0** | Epic 0 (test infrastructure) | Must complete first |
| **Phase 1** | Epics 1, 2*, 3*, 4* | 1 starts; 2/3/4 start as soon as Epic 1 identity types land |
| **Phase 2** | Epics 5, 6, 7 | Start as their dependencies from Phase 1 complete |
| **Phase 3** | Epics 8, 9, 10, 11 | Start as their dependencies from Phase 2 complete |
| **Phase 4** | Epic 12 | After all subsystems substantially complete |

*Epics 2, 3, 4 can begin their first beads once Epic 1's identity types (#1.1) land, not after all of Epic 1 completes.

## Spec-to-Epic Mapping

| Spec | Epic | Sub-beads |
|------|------|-----------|
| `validation-framework/spec.md` | 0, 12 | 0: test infrastructure; 12: Layers 2-4 |
| `scene-graph/spec.md` | 1 | 6 beads |
| `runtime-kernel/spec.md` | 2 | 5 beads |
| `timing-model/spec.md` | 3 | 4 beads |
| `lease-governance/spec.md` | 4 | 5 beads |
| `input-model/spec.md` | 5 | 5 beads |
| `session-protocol/spec.md` | 6 | 7 beads |
| `configuration/spec.md` | 7 | 4 beads |
| `policy-arbitration/spec.md` | 8 | 4 beads |
| `scene-events/spec.md` | 9 | 4 beads |
| `resource-store/spec.md` | 10 | 4 beads |
| `system-shell/spec.md` | 11 | 5 beads |
| Cross-subsystem integration | 12 | 6 beads |

**Total: 13 epics, 64 sub-beads**

## How to Use

Each prompt file is designed to be given to an Opus instance with `/beads-writer` capability. The instance should:

1. Read the prompt
2. Read the referenced spec files to extract exact line numbers and WHEN/THEN scenarios
3. Create the epic and sub-beads with full spec traceability
4. Wire dependency chains between sub-beads

Sub-beads should be granular enough for a single worker session (hours, not days).
