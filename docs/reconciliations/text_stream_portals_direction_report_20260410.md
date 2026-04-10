# Text Stream Portals Direction Report

Date: 2026-04-10
Scope: `/project-direction` package for transport-agnostic text stream portals
Status: RFC and OpenSpec change generated; stopped before bead creation

## Executive Summary

[Observed] The project is already pointed at a missing interaction class: low-latency, governed text interaction that is richer than a chat box but lighter than full media presence. Doctrine explicitly says the CLI, chat transcript, and generated webpage are incomplete forms, and `presence.md` already allows future transcript-oriented surfaces while preserving raw tiles as the custom-layout escape hatch.

[Observed] The current repository does not yet have product-level spec coverage for this idea. The scene contract still exposes only four v1 node types, the system shell forbids agent-specific chrome UI, and the input scroll primitives appear to exist only as library seams rather than a fully wired runtime surface. That means the work is aligned, but it is not implementation-ready.

[Inferred] The correct next move is spec-first: define a transport-agnostic portal contract whose boundary is input/output text streams and session metadata, not tmux. Treat tmux as the first adapter candidate, not the product capability. Stop before beads until that contract is accepted.

## Project Spirit

**Core problem**: give the runtime a governed way to host low-latency streaming text interactions without turning the product into a terminal host or chat app shell.
**Primary user**: internal developers and operators shaping resident-agent interaction surfaces and adapter boundaries.
**Success looks like**: the project can support a portal where streamed text arrives incrementally, the viewer can reply with bounded low-latency interaction, and the entire flow remains under leases, privacy, safe mode, and shell override rules.
**Trying to be**: an agent-native presence engine that can express text interaction as one more governed surface class.
**Not trying to be**: a terminal emulator, a tmux-specific HUD feature, a chrome tray of agent apps, or a generic chat framework.

### Requirements

| # | Requirement | Class | Evidence | Status |
|---|------------|-------|---------|--------|
| 1 | Text interaction must remain subordinate to presence rather than replacing it | Hard | `about/heart-and-soul/vision.md` | Partial |
| 2 | Portal surfaces must remain content-layer territory, not chrome-owned UI | Hard | `openspec/changes/v1-mvp-standards/specs/system-shell/spec.md` | Unmet for proposed `(i)` chrome affordance |
| 3 | Runtime boundary must be transport-agnostic text streams, not tmux semantics | Hard | user direction + new RFC 0013 | Newly specified |
| 4 | Pilot must avoid terminal-emulator scope creep | Hard | `about/heart-and-soul/vision.md`, RFC 0013 | Newly specified |
| 5 | Portal interactions must obey focus, local feedback, scroll, dismiss, redaction, and safe mode | Hard | v1 input/system-shell specs, RFC 0013 | Partial |
| 6 | New capability needs explicit OpenSpec coverage before implementation | Hard | `spec-and-spine` doctrine | Met by this change set |

### Contradictions

[Observed] The desired expandable `(i)` icon conflicts with the current shell contract if interpreted as chrome. System-shell requirements forbid agents from reading, writing, or depending on chrome state, and the status area must not expose agent identities.

[Observed] The repository already contains scroll and command-input building blocks, but I found no runtime/compositor/app references wiring the scroll registry into a shipped transcript surface. The project therefore has enabling seams, not a complete portal path.

## Current State

| Dimension | Status | Summary | Key Evidence |
|-----------|--------|---------|-------------|
| Spec adherence | Weak | No current capability spec covers text stream portals | `openspec/changes/` scan |
| Core workflows | Missing | No end-to-end portal workflow exists in repo docs or specs | repo search |
| Test confidence | Missing | No portal-specific tests exist because no portal capability exists | repo search |
| Observability | Adequate | Existing validation doctrine is reusable, but no portal artifacts exist yet | `about/heart-and-soul/validation.md` |
| Delivery readiness | Weak | No accepted contract yet; chrome/content-layer boundary unresolved before this report | doctrine + system-shell spec |
| Architectural fitness | Adequate | The architecture supports a raw-tile pilot, but not a chrome-hosted or terminal-emulator version | doctrine + scene/input/session contracts |

### Why it matters

[Observed] The architecture is not the blocker. Boundary clarity is the blocker. Raw tiles, resident sessions, focus, local feedback, and lease governance already exist. What is missing is the contract that says what a portal is and is not.

[Observed] The biggest risk is building the wrong thing efficiently: either a tmux-specific bridge that hardens the wrong abstraction, or a chrome-layer affordance that violates shell doctrine. That is exactly why planning must stop at RFC/spec generation before beads.

## Alignment Review

### Aligned Next Steps

1. [Inferred] Define `text-stream-portals` as a transport-agnostic capability with external adapters and a content-layer pilot.
2. [Inferred] Use a phase-0 raw-tile pilot as the proving ground.
3. [Inferred] Keep tmux as the first adapter candidate only.

### Misaligned Directions

1. [Observed] A full terminal-emulator feature is misaligned. It would move PTY/rendering semantics into the runtime and erode scope boundaries.
2. [Observed] A chrome-resident agent portal tray is misaligned under the current shell contract.

### Premature Work

1. [Observed] Bead generation is premature before the portal contract is accepted.
2. [Observed] Any dedicated node-type implementation is premature before a raw-tile pilot demonstrates stable recurring needs.

### Deferred

1. [Inferred] A first-class transcript node or runtime-managed portal surface is deferred pending pilot evidence.
2. [Inferred] Rich editing semantics, terminal parity, or byte-stream passthrough are deferred indefinitely unless separately justified.

### Rejected

1. [Observed] Tmux-aware runtime logic is rejected as the core boundary.
2. [Observed] Portal affordances in chrome are rejected for this change.

## Gap Analysis

### Blockers

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No transport-agnostic contract | Without it, any implementation hardens the wrong abstraction | planners, implementers | repo/spec scan | RFC + OpenSpec first | M |
| Chrome/content-layer ambiguity | Misplacing the portal in chrome would violate shell doctrine | implementers | system-shell spec | resolve in RFC/spec | S |
| No capability spec | No implementation can be judged correct yet | implementers, reviewers | openspec scan | add `text-stream-portals` spec | S |

### Important Enhancements

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No pilot validation plan | A raw-tile proof needs explicit acceptance criteria | reviewers | current absence | include in spec/tasks | S |
| No adapter contract examples | Adapters will drift if left implicit | implementers | current absence | define tmux/chat/LLM examples in RFC | S |

### Strategic Gaps

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No future promotion criteria | Raw-tile pilots often calcify without a clear promotion rule | architecture team | current absence | define in RFC 0013 | S |

## Work Plan

### Immediate Alignment Work

1. Update doctrine to name low-latency text stream portals as an intended use case without collapsing the product into chat.
2. Add RFC 0013 to define the adapter boundary, surface model, governance, and phased contract.
3. Create `text-stream-portals` OpenSpec artifacts.

### Near-Term Delivery Work

1. Review and accept the Phase-0 pilot shape: resident raw tile, external adapter, content-layer only.
2. Decide whether reply input is bounded submission only or a richer in-surface editing model.
3. Only after signoff: decompose into beads.

### Strategic Future Work

1. Reassess whether repeated portal patterns justify a first-class runtime portal surface or node type.

## Do Not Do Yet

| Item | Reason | Revisit when |
|------|--------|-------------|
| Generate beads | Contract not yet accepted | after RFC/spec signoff |
| Build terminal-emulator semantics | Solves the wrong problem first | only if separate doctrine and RFC support it |
| Put agent portal UI in chrome | violates system-shell contract | only if shell doctrine changes |

---

## Conclusion

**Real direction**: transport-agnostic text stream portals are aligned as a governed presence surface, provided they stay content-layer, adapter-driven, and explicitly non-terminal.

**Work on next**: accept the doctrine updates, review RFC 0013, and review the `text-stream-portals` OpenSpec change.

**Stop pretending**: the project does not yet have a valid implementation contract for this feature, and tmux is not the right product boundary.
