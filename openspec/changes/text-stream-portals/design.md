## Context

tze_hud already has the core ingredients for a narrow portal pilot:

- resident gRPC sessions with authenticated namespaces,
- raw tile ownership under leases,
- existing text/image/hit-region node types,
- local-first focus and command-input semantics,
- shell-level override, privacy, and safe-mode behavior.

What it does not have is a coherent contract for **stream-backed text interaction surfaces**. The current doctrine permits transcript-oriented surfaces, but the current spec stack does not define:

- whether a portal is a chrome surface or content-layer surface,
- whether the product boundary is tmux, PTY, chat provider, or generic text streams,
- how scroll, reply submission, and local feedback fit together,
- when a raw-tile pilot should be promoted to a dedicated surface class.

This design resolves those questions before any implementation work starts.

## Goals / Non-Goals

**Goals:**
- Define the feature around transport-agnostic text input/output streams rather than a tmux-specific bridge
- Keep the runtime tmux-agnostic and terminal-agnostic
- Establish a phase-0 pilot that can be built with resident raw tiles and external adapters
- Preserve shell sovereignty by keeping portal UI out of chrome
- Make the contract broad enough for human chat transports and LLM interactions alike

**Non-Goals:**
- Building a terminal emulator
- Giving the runtime PTY, shell, or process-lifecycle responsibilities
- Adding agent-specific chrome widgets or trays
- Defining full editing, IME, or copy-mode semantics in this first contract
- Creating beads or implementation ownership in this change

## Decisions

### 1. Product boundary: generic text streams, not tmux

**Decision:** The runtime contract is expressed as session metadata plus output-stream append/update events and bounded input submission. Tmux is only an adapter candidate.

**Rationale:** This keeps the feature reusable across tmux, human chat systems, and LLM sessions. It also prevents the runtime from inheriting terminal/process semantics accidentally.

**Alternative considered:** Make tmux the core contract. Rejected because it hardens the wrong abstraction and narrows the capability unnecessarily.

### 2. Surface attachment: content layer, not chrome

**Decision:** Portal surfaces live in content-layer territory in the pilot phase.

**Rationale:** Current shell contracts require chrome rendering independence from agent state and forbid agent-specific chrome semantics. The proposal's expandable `(i)` affordance therefore has to be implemented as a content-layer portal card or tile-local affordance, not a chrome control.

**Alternative considered:** Runtime-owned chrome tray for sessions. Rejected because it violates the current shell contract and status-indicator constraints.

### 3. Phase-0 implementation shape: raw-tile pilot

**Decision:** The first implementation proof is a resident raw-tile surface assembled from existing node types.

**Rationale:** This uses already-governed primitives, keeps implementation risk down, and forces the team to prove the interaction class before promoting it to a dedicated surface.

**Alternative considered:** Define a new transcript or portal node immediately. Rejected as premature without repeated evidence.

### 4. Interaction shape: bounded reply submission, local-first controls

**Decision:** The initial contract guarantees expand/collapse, reply submission, scroll, and optional control actions such as interrupt/cancel, all under local-first feedback rules.

**Rationale:** These are the minimum viable interactions that make a portal useful without committing to full in-surface text-editing semantics.

**Alternative considered:** Full terminal-like interactive editing. Rejected as oversized for the first contract.

### 5. Promotion rule: separate pilot from durable runtime surface

**Decision:** The RFC explicitly requires evidence before creating a dedicated runtime portal surface or node type.

**Rationale:** This minimizes churn and avoids baking one pilot implementation path into the long-term architecture prematurely.

**Alternative considered:** Treat the pilot as the final runtime shape. Rejected because the right long-term shape is still unknown.

## Risks / Trade-offs

- **[Risk] Raw-tile pilots may feel cumbersome** → Mitigation: treat that friction as evidence. If it recurs across adapters, promote with a later RFC.
- **[Risk] “bounded reply submission” may be too weak for real workflows** → Mitigation: keep it as an explicit open question rather than silently sliding into terminal semantics.
- **[Risk] Adapter boundaries may under-specify ordering/backpressure** → Mitigation: RFC 0013 and the new capability spec define traffic-class expectations clearly.
- **[Trade-off] Content-layer affordances may be less always-visible than chrome** → This is acceptable because chrome sovereignty is the higher-order constraint.
- **[Trade-off] One capability covers human-chat and LLM use cases** → This broadens the contract, but it is the right abstraction if the boundary is truly text streams rather than adapter brand names.

## Migration Plan

No runtime migration is part of this change. The artifact set is planning-only:

1. update doctrine,
2. add RFC 0013,
3. add OpenSpec capability and tasks,
4. review for signoff,
5. stop before bead creation.

## Open Questions

1. Should the first pilot support only single-line or bounded message submission, or a richer draft-edit model?
2. Should transcript history/windowing be adapter-owned in the pilot, runtime-owned, or split?
3. Does the pilot need a runtime-owned unread/activity indicator, or can that remain tile-local state?
4. At what point does repeated adapter demand justify a first-class portal node or runtime-managed portal abstraction?
