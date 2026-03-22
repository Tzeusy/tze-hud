---
name: law-and-lore
description: >
  Load tze_hud RFC design contracts to contextualize implementation work. The docs/rfcs/
  directory contains 11 RFCs that define the wire-level contracts, data models, state machines,
  protobuf schemas, and quantitative budgets for the tze_hud presence engine. Consult relevant
  RFCs before implementing features, writing protobuf definitions, designing state machines,
  choosing field numbers, setting performance budgets, or resolving cross-subsystem integration
  questions. Use this skill proactively when the task touches: scene graph, runtime kernel,
  timing/clocks, input handling, session protocol, configuration, system shell/chrome, leases,
  policy arbitration, events, or resource storage. Selectively load ONLY the RFCs relevant to
  your current task — do not load all 11 at once.
---

# tze_hud Law and Lore — Design Contracts

The `docs/rfcs/` directory contains the authoritative design contracts for tze_hud. These are not aspirational docs — they are the wire-level specifications that code must conform to: protobuf schemas, state machines, field allocations, latency budgets, and integration contracts.

**Consult relevant RFCs before:**
- Implementing any subsystem or feature
- Writing or modifying protobuf definitions
- Allocating field numbers in session envelope messages
- Setting or validating performance budgets
- Resolving how two subsystems interact
- Writing tests that assert contract behavior

**Do NOT load all RFCs at once.** Each RFC is large (2000–6000 lines). Select by task domain.

## RFC index — select by relevance

### Foundation (load first when starting any subsystem work)

| RFC | File | Read when... | Key content |
|-----|------|-------------|-------------|
| **0001** | `docs/rfcs/0001-scene-contract.md` | Touching scene graph, mutations, node types, identity model | SceneId/ResourceId types, mutation pipeline, zone registry, tile/tab/node hierarchy, protobuf schema |
| **0002** | `docs/rfcs/0002-runtime-kernel.md` | Touching process architecture, frame pipeline, budgets, degradation | Thread model, 8-stage frame pipeline, admission control, budget enforcement, degradation ladder, window management |

### Timing, Input, Protocol (the hot path)

| RFC | File | Read when... | Key content |
|-----|------|-------------|-------------|
| **0003** | `docs/rfcs/0003-timing.md` | Touching clocks, sync groups, presentation scheduling, frame deadlines | Clock domains (`_wall_us`/`_mono_us`), sync groups, frame deadline semantics, relative scheduling |
| **0004** | `docs/rfcs/0004-input.md` | Touching focus, pointer capture, gestures, keyboard, accessibility | Focus tree, pointer capture protocol, gesture pipeline, IME, local feedback contract, event dispatch |
| **0005** | `docs/rfcs/0005-session-protocol.md` | Touching wire protocol, session lifecycle, message envelope, reconnection | Session state machine, multiplexed oneof envelope (47+ fields), traffic classes, version negotiation, MCP bridge, subscription management |

### Governance and policy

| RFC | File | Read when... | Key content |
|-----|------|-------------|-------------|
| **0006** | `docs/rfcs/0006-configuration.md` | Touching TOML config, display profiles, capability registry | Display profiles (full-display/headless/mobile), budget defaults, capability vocabulary, validation rules |
| **0007** | `docs/rfcs/0007-system-shell.md` | Touching chrome layer, safe mode, freeze, mute, override, badges | Chrome semantics, safe mode protocol, privacy-safe capture, backpressure signals, audit events |
| **0008** | `docs/rfcs/0008-lease-governance.md` | Touching leases, priority, suspension, revocation, resource budgets | Lease state machine, priority levels 0–4, suspension vs revocation, orphan handling, grace periods, zone interaction |
| **0009** | `docs/rfcs/0009-policy-arbitration.md` | Touching conflict resolution, arbitration stack, GPU failure handling | 7-level arbitration stack (human override → safety → privacy → security → attention → resource → content) |

### Events and storage

| RFC | File | Read when... | Key content |
|-----|------|-------------|-------------|
| **0010** | `docs/rfcs/0010-scene-events.md` | Touching event taxonomy, interruptions, quiet hours, subscriptions | Event taxonomy (input/scene/system), interruption classes, quiet hours, event bus, `tab_switch_on_event` contract |
| **0011** | `docs/rfcs/0011-resource-store.md` | Touching content-addressed storage, uploads, GC, font lifecycle | Upload protocol, BLAKE3 content addressing, reference counting, cross-agent sharing, size limits |

## How to load

Read RFCs directly from `docs/rfcs/`:

```
Read docs/rfcs/0001-scene-contract.md    # scene graph data model
Read docs/rfcs/0005-session-protocol.md  # wire protocol and envelope
Read docs/rfcs/0008-lease-governance.md  # lease state machine
```

For cross-RFC integration questions (field number conflicts, cross-subsystem contracts, resolved inconsistencies, quantitative budgets), load the reference map:

```
Read .claude/skills/law-and-lore/references/cross-rfc-map.md
```

## Key contracts from the RFCs

These contracts are load-bearing. Violating them breaks cross-subsystem integration:

1. **One stream per agent.** Session protocol (0005) multiplexes all message types over a single bidirectional gRPC stream — do not proliferate streams.
2. **Leases govern all screen territory.** Agents cannot mutate the scene without a valid lease (0008). Leases have TTL, priority, capability scope, and revocation semantics.
3. **7-level arbitration stack.** Policy conflicts resolve top-down: human override > safety > privacy > security > attention > resource > content (0009).
4. **Safe mode suspends, it does not revoke.** Safe mode freezes agent leases; only explicit revocation terminates them (0007, 0008).
5. **Clock domains are typed.** Wall-clock fields end `_wall_us`, monotonic fields end `_mono_us`. Never mix them (0003, 0005).
6. **Field numbers are allocated.** Session envelope fields 1–47 are assigned; 50–99 are reserved for post-v1. Check 0005 §9 before adding fields.
7. **Local feedback is non-negotiable.** Input acknowledgement must happen locally within 4ms p99 — never wait for a remote roundtrip (0004).

## V1 scope boundaries from RFCs

- **Input v1:** Focus model, pointer capture, local feedback, pointer/keyboard events. Gesture pipeline and IME may ship minimal. A11y structure ships, platform bridge defers.
- **Config v1:** `full-display` and `headless` profiles active. `mobile` profile schema-reserved but fails at startup if configured.
- **Session v1:** Full snapshot reconnection only. Delta burst replay defers to post-v1.
- **Resource store v1:** Content-addressed upload/download, reference counting, GC. Font lifecycle and cross-agent sharing included. Persistence model defers.
