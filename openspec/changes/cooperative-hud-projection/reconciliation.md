# Cooperative HUD Projection Reconciliation

Date: 2026-04-28

## Scope

This reconciliation verifies `openspec/changes/cooperative-hud-projection/` against `/project-direction` and `/project-shape` expectations after the initial OpenSpec artifacts were generated.

Reviewed artifacts:

- `proposal.md`
- `design.md`
- `specs/cooperative-hud-projection/spec.md`
- `specs/text-stream-portals/spec.md`
- `tasks.md`

Shape references:

- Doctrine: `about/heart-and-soul/vision.md`, `architecture.md`, `attention.md`, `privacy.md`, `security.md`
- Design contracts: `about/legends-and-lore/rfcs/0013-text-stream-portals.md`
- Topology: `about/lay-and-land/README.md`, `data-flow.md`
- Engineering standards: `about/craft-and-care/engineering-bar.md`
- Capability specs: `openspec/specs/text-stream-portals/spec.md`, `session-protocol/spec.md`, `lease-governance/spec.md`, `input-model/spec.md`, `system-shell/spec.md`, `timing-model/spec.md`

## Project-Shape Alignment

The change is aligned with the five-pillar shape model:

- **Doctrine / WHY:** supports the presence-engine thesis by giving already-running LLM sessions a governed screen presence without collapsing the product into a terminal host, chat app, or notification stream.
- **Design Contracts / HOW:** stays within RFC 0013's text-stream portal adapter boundary: transport-agnostic output/input/session metadata, content-layer portal surface, external adapter isolation, and no PTY/process lifecycle authority in runtime core.
- **OpenSpec / WHAT:** adds a new `cooperative-hud-projection` capability for the projection authority and LLM-facing operations, plus a narrow `text-stream-portals` delta recognizing cooperative LLM projection as a concrete non-tmux adapter family.
- **Topology / WHERE:** places long-lived projection state in an external projection authority/daemon, not in runtime core, the scene graph, chrome, or LLM token context.
- **Craft and Care / QUALITY:** tasks now require stable error codes, audit fields, state-machine tests, bounded rate tests, local-ack budget checks, live `/user-test`, reconciliation, and closeout reporting.

## R1 Findings and Fixes

R1 focused on doctrine and shape placement.

Fixes applied:

- Added explicit projection operation authorization, owner binding, stable denial error codes, and audit records.
- Clarified cached lease identity as advisory only; restart or expired resume requires fresh session auth, capability grants, and a new valid lease.
- Recast daemon ownership into the observable external projection authority contract; design keeps the local daemon as first implementation.
- Added fail-closed privacy defaults and most-restrictive effective visibility.
- Added local-first pending feedback for submitted HUD input.
- Expanded validation tasks for state-machine, stable error-code, bounded-rate, and local-ack budget checks.

## R2 Findings and Fixes

R2 focused on normative completeness and testability.

Fixes applied:

- Added operation request/response envelope fields.
- Added initial stable error-code set.
- Added attach conflict semantics and structured audit record shape.
- Added owner-token generation, storage, expiry/rotation, and same-OS-user non-authority rules.
- Closed v1 persistence by making projection state memory-only across daemon/host restart and purged on detach/cleanup/expiry.
- Added deterministic v1 bounds for output, status, retained transcript, visible transcript, pending input, polling, and portal update rate.
- Added inbox state machine: `pending`, `delivered`, `deferred`, `handled`, `rejected`, `expired`.

## R3 Findings and Fixes

R3 focused on cross-spec consistency.

Fixes applied:

- Added `not_before_wall_us` to deferred acknowledgements and validated it against `expires_at_wall_us`.
- Removed owner-token proof from attach conflict handling; reattach ownership remains non-attach owner-token flow.
- Added `expires_at_wall_us` to inbox item schema and made expiration projection-authority-owned rather than an LLM acknowledgement state.
- Reworded pending input scenarios around non-terminal/waiting state to match `delivered` transitions.
- Changed tasks to require oversized-output rejection rather than per-operation truncation.
- Clarified that projection MCP packaging is external-daemon MCP, not runtime v1 MCP.

## R4 Result

R4 found no blocking or major issues. One minor cleanup ambiguity was resolved after R4:

- Owner cleanup now requires `owner_token`.
- Operator cleanup uses a separate audited operator-authority credential/path and does not expose owner token or private content.

Residual implementation watchpoints:

- Keep projection MCP tooling in the external daemon, not runtime MCP.
- Test stale lease identity and broader-than-grant lease requests fail closed.
- Live `/user-test` must cover redaction, safe mode, freeze, dismiss, orphan cleanup, and backlog non-escalation.

## Verification

`openspec validate cooperative-hud-projection --strict` passes after all reconciliation fixes.
