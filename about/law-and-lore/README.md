# Law and Lore — Design Contracts

Authoritative design contracts for tze_hud: wire-level protobuf schemas, state machines, field allocations, latency budgets, and integration contracts.

## Structure

| Directory | Content |
|-----------|---------|
| `rfcs/` | 11 numbered design contract RFCs (0001–0011) |
| `reviews/` | Review rounds per RFC |
| `prompts/` | Epic development prompts |
| `reconciliations/` | Spec-to-code reconciliation reports |

## RFC Index

| RFC | Title | Domain |
|-----|-------|--------|
| 0001 | Scene Contract | Scene graph, mutations, node types, identity |
| 0002 | Runtime Kernel | Thread model, frame pipeline, budgets, degradation |
| 0003 | Timing | Clock domains, sync groups, frame deadlines |
| 0004 | Input | Focus, pointer capture, gestures, keyboard |
| 0005 | Session Protocol | Wire protocol, session lifecycle, message envelope |
| 0006 | Configuration | TOML config, display profiles, capabilities |
| 0007 | System Shell | Chrome layer, safe mode, freeze, mute, override |
| 0008 | Lease Governance | Lease state machine, priority, revocation |
| 0009 | Policy Arbitration | 7-level arbitration stack |
| 0010 | Scene Events | Event taxonomy, interruptions, quiet hours |
| 0011 | Resource Store | Content-addressed storage, uploads, GC |
| 0012 | Component Shape Language | Design tokens, component profiles, visual extensibility |
