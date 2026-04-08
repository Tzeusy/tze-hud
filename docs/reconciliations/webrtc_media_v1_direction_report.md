# WebRTC/Media V1 Direction Report

## Executive Summary

`tze_hud` is building an agent-native presence runtime where the screen is sovereign and timing/composition are runtime-owned, with media as a first-class architectural concern but not a v1 shipped capability ([about/heart-and-soul/architecture.md:1](about/heart-and-soul/architecture.md:1), [about/heart-and-soul/v1.md:115](about/heart-and-soul/v1.md:115)).

Current implementation is intentionally aligned with that deferment: media/WebRTC are represented as schema and contract placeholders (zone media types, reserved protocol fields, post-v1 transport constraints), while runtime behavior enforces no live media streams in v1 ([crates/tze_hud_scene/src/types.rs:1419](crates/tze_hud_scene/src/types.rs:1419), [crates/tze_hud_protocol/src/convert.rs:330](crates/tze_hud_protocol/src/convert.rs:330), [crates/tze_hud_scene/src/lease/budget.rs:533](crates/tze_hud_scene/src/lease/budget.rs:533)).

Highest-priority next work is spec-first: formalize a minimal "media v1.5" contract without changing v1 truth claims, then sequence bounded protocol/scene/runtime deltas behind explicit deferrals and acceptance tests. Immediate implementation of live AV is premature and churn-prone under current v1 constraints.

## Project Spirit

**Core problem**: Provide deterministic, governable, low-latency on-screen presence for multiple agents without placing LLMs in the frame loop.
**Primary user**: Runtime-integrating developers building resident/guest agent experiences.
**Success looks like**: Stable scene mutation, lease arbitration, timing correctness, and predictable composition under contention.
**Trying to be**: A three-plane presence engine (MCP/gRPC/WebRTC architecture).
**Not trying to be**: A v1 live-media runtime shipping WebRTC/GStreamer decode paths.

### Doctrine vs Implementation vs Proposed V1 Contract

#### Current doctrine
- Architecture declares WebRTC as the media plane and GStreamer as core media substrate ([about/heart-and-soul/architecture.md:27](about/heart-and-soul/architecture.md:27), [about/heart-and-soul/architecture.md:215](about/heart-and-soul/architecture.md:215)). [Observed]
- v1 doctrine explicitly defers GStreamer/WebRTC/live AV and clocked media cues ([about/heart-and-soul/v1.md:115](about/heart-and-soul/v1.md:115)). [Observed]

#### Current implementation
- Runtime/spec reserve embodied/WebRTC signaling and media worker pools for post-v1 ([openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:698](openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:698), [openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:374](openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:374)). [Observed]
- Scene/protocol keep schema placeholders (`VideoSurfaceRef`, `WebRtcRequired`, reserved auth/session capabilities) but no live encode/decode path ([crates/tze_hud_scene/src/types.rs:1514](crates/tze_hud_scene/src/types.rs:1514), [crates/tze_hud_protocol/proto/session.proto:190](crates/tze_hud_protocol/proto/session.proto:190)). [Observed]
- Conversion path intentionally omits proto payload encoding for `VideoSurfaceRef` and tests describe rendering as deferred ([crates/tze_hud_protocol/src/convert.rs:330](crates/tze_hud_protocol/src/convert.rs:330), [crates/tze_hud_scene/tests/zone_ontology.rs:709](crates/tze_hud_scene/tests/zone_ontology.rs:709)). [Observed]

#### Proposed v1 contract decision (this pass)
- Keep **v1 GA contract unchanged**: no live media/WebRTC stream implementation. [Inferred]
- Introduce **spec-only preparatory tranche** that defines exact first shippable media slice and required guardrails before any runtime media threads. [Inferred]
- Candidate first slice (post-v1 only): single inbound `VideoSurfaceRef` path to a fixed zone, no bidirectional AV/session negotiation, no multi-feed mixing or audio policy in first step. [Inferred]

### Requirements

| # | Requirement | Class | Evidence | Status |
|---|---|---|---|---|
| 1 | v1 ships without GStreamer/WebRTC live media | Hard | `v1.md` defer list | Met |
| 2 | Media/WebRTC future path must remain architecturally prepared | Hard | architecture three-plane + media sections | Met |
| 3 | Session protocol reserves embodied/media signaling slots for later | Hard | session-protocol spec post-v1 requirement | Met |
| 4 | Runtime must not spawn media pool in v1 | Hard | runtime-kernel deferred media pool requirement | Met |
| 5 | Zone/content schema may include media placeholders without rendering | Soft | scene types + zone ontology test | Met |
| 6 | Move media into active scope only with spec-first signoff | Hard | project-direction rule + current missing media slice spec | Unmet |
| 7 | Maintain low-churn evolution path from v1 static content to media | Soft | architecture extensibility doctrine | Partial |

### Contradictions

- Doctrine tension: architecture says "media is one reason project exists" while v1 doctrine forbids media runtime implementation. This is not a bug, but it is a communication hazard without explicit phased contract language ([about/heart-and-soul/architecture.md:215](about/heart-and-soul/architecture.md:215), [about/heart-and-soul/v1.md:115](about/heart-and-soul/v1.md:115)). [Observed]
- Issue brief references `docs/reconciliations/webrtc_media_v1_epic_prompt.md`, but file is absent in this branch and `origin/main`; this report used issue text + cited evidence set as source brief. [Observed]

## Current State

| Dimension | Status | Summary | Key Evidence |
|-----------|--------|---------|-------------|
| Spec adherence | Adequate | v1 deferrals are explicit and implemented | v1/spec/runtime references above |
| Core workflows | Adequate | Zone publishing works for v1 content classes; media remains schema-only | `types.rs`, `zone_ontology.rs` |
| Test confidence | Adequate | Explicit tests cover deferred media semantics and v1 budgets | `zone_ontology.rs`, `lease/budget.rs` |
| Observability | Adequate | Existing runtime telemetry and structured protocol paths are present | session/runtime specs |
| Delivery readiness | Adequate | CI and broad test corpus exist; some unrelated known instability remains | `AGENTS.md` notes |
| Architectural fitness | Strong | Architecture can absorb post-v1 media without collapse; deferrals preserved | architecture + protocol/runtime reserved interfaces |

### Dimension Notes

- **Spec adherence**: Implemented behavior intentionally matches deferred media scope (no contradiction requiring emergency correction). [Observed]
- **Core workflows**: Current published zone/media abstractions support non-media v1 interactions while keeping surface-level compatibility for future additions. [Observed]
- **Test confidence**: Tests codify key boundaries (e.g., `max_concurrent_streams == 0`, schema-accepted-yet-not-rendered video references). [Observed]
- **Observability**: No media-specific telemetry yet, but existing frame/session telemetry scaffolding can host it when scope opens. [Inferred]
- **Delivery readiness**: Platform is capable of incremental additions; introducing live media without contract-first milestones would destabilize delivery. [Inferred]
- **Architectural fitness**: Three-plane model and reserved fields reduce migration risk if future work is serialized and bounded. [Observed]

## Alignment Review

### Aligned next steps
- Write a dedicated media/WebRTC capability spec describing smallest credible post-v1 slice, explicit non-goals, and acceptance tests.
- Define session-protocol deltas for first media signaling path with field-level constraints and backward compatibility.
- Define runtime-kernel activation gate for media worker pool with strict v1-off defaults and measurable perf/safety budgets.

### Misaligned directions
- "Add full WebRTC + multi-feed AV + embodied presence in v1" is misaligned with published v1 contract and would invalidate current doctrine commitments.

### Premature work
- Integrating actual GStreamer pipelines now is premature until scope, clocks, budgets, and degradation behavior are fully spec’d and signed off.

### Deferred
- Audio routing/mixing policy, bidirectional call semantics, adaptive bitrate orchestration, and multi-stream arbitration should remain deferred until single-stream ingest path stabilizes.

### Rejected
- Any plan that re-labels v1 as media-capable without matching implementation and tests is rejected as spec drift and trust erosion.

## Gap Analysis

### Blockers
| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No dedicated media/WebRTC capability spec in active spec set | Cannot safely decompose implementation backlog | Runtime + protocol maintainers | No `openspec/specs/*media*` capability in this branch | Write spec first | M |
| Missing explicit phased contract wording linking architecture media vision to v1 deferment | Creates contradictory planning interpretations | Maintainers/reviewers | Architecture vs v1 docs tension | Update doctrine/spec cross-links | S |

### Important Enhancements
| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No first-slice performance/latency budget for media ingress | High risk of unbounded implementation | Runtime/compositor owners | Deferred media worker boundary exists but no activation budgets | Add numeric budgets in spec | M |
| No explicit threat/privacy model for media ingress in v1-adjacent docs | Safety posture unclear before enabling AV | Security/privacy reviewers | Media deferred; no active contract | Add privacy/security section to media spec | M |

### Strategic Gaps
| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| End-to-end rehearsal harness for media timing | Needed before production-grade AV claims | Validation framework maintainers | Validation framework currently not media-first | Add post-v1 validation scenes | L |
| Media-focused observability schema | Necessary for incident/debug readiness | Ops/dev tooling | No active media plane runtime in v1 | Add when first media slice lands | M |

## Work Plan

### Immediate alignment work

### Chunk 1: Create media/WebRTC capability spec

**Objective**: Define the smallest credible post-v1 media slice as a normative spec.
**Spec reference**: Spec work required
**Dependencies**: None
**Why ordered here**: All implementation work is blocked on contract clarity.
**Scope**: M
**Parallelizable**: No — this sets canonical contract language.
**Serialize with**: All later chunks

**Acceptance criteria**:
- [ ] New capability spec exists under `openspec/` with clear MUST/SHALL language.
- [ ] Spec states explicit non-goals and deferred items.
- [ ] Spec includes measurable acceptance scenarios.

**Notes**: Include timing model, transport boundaries, and lease budget interactions.

### Reconciliation: Create media/WebRTC capability spec

Check:
- [ ] Implemented intent matches cited doctrine/spec boundaries.
- [ ] Acceptance criteria are testable and non-ambiguous.
- [ ] No hidden scope inflation in the spec text.
- [ ] Follow-up backlog implied by the spec is explicit.
- [ ] Existing v1 docs remain accurate.

### Chunk 2: Session-protocol media signaling delta (spec-only)

**Objective**: Specify first signaling messages/fields for post-v1 media path while preserving v1 compatibility.
**Spec reference**: `session-protocol` + new media spec
**Dependencies**: Chunk 1
**Why ordered here**: Protocol contract must precede runtime wiring.
**Scope**: M
**Parallelizable**: No — depends on canonical media scope.
**Serialize with**: Chunk 3

**Acceptance criteria**:
- [ ] Reserved-field usage and compatibility behavior are explicit.
- [ ] Failure/error semantics are specified.
- [ ] Security/auth assumptions are stated.

**Notes**: Keep embodied presence still deferred unless separately approved.

### Reconciliation: Session-protocol media signaling delta

Check:
- [ ] Delta aligns with existing reserved-field strategy.
- [ ] No contradiction with v1 deferred clauses.
- [ ] Scenarios cover downgrade/fallback behavior.
- [ ] Review confirms backward compatibility.
- [ ] Open questions are captured, not hidden.

### Chunk 3: Runtime-kernel media activation gate (spec + scaffold plan)

**Objective**: Define activation criteria and safeguards before any media worker threads/pipelines are enabled.
**Spec reference**: `runtime-kernel` + new media spec
**Dependencies**: Chunk 1, Chunk 2
**Why ordered here**: Prevents ad-hoc runtime enablement.
**Scope**: M
**Parallelizable**: No — shared runtime boundaries.
**Serialize with**: All implementation chunks

**Acceptance criteria**:
- [ ] Media worker pool enablement conditions are explicit.
- [ ] Budgets and degradation interactions are quantified.
- [ ] Validation requirements are listed before implementation approval.

**Notes**: Preserve v1 default behavior (`media disabled`) until explicit milestone.

### Reconciliation: Runtime-kernel media activation gate

Check:
- [ ] Activation criteria are measurable.
- [ ] Budget and degradation coupling is specified.
- [ ] v1 behavior remains unchanged by default.
- [ ] Required validation scenes are identified.
- [ ] No undocumented assumptions remain.

### Block Reconciliation: Media/WebRTC direction block

Check:
- [ ] All chunk criteria met.
- [ ] End-to-end direction is coherent and low-churn.
- [ ] Doctrine/spec/implementation boundaries are explicit.
- [ ] Deferred work is clearly marked.
- [ ] Backlog conversion can proceed without reinterpretation.

### Near-term delivery work
- Convert each chunk above into beads with strict dependencies (spec-first, then implementation).
- Generate human review report bead before any runtime media coding beads.

### Strategic future work
- After first-slice signoff: implement one-way ingest path + validation scene + telemetry.
- Defer bidirectional AV/session complexity until first-slice metrics are stable.

## Do Not Do Yet

| Item | Reason | Revisit when |
|------|--------|-------------|
| Full embodied presence/WebRTC in v1 | Contradicts v1 doctrine and deferred protocol/runtime requirements | After post-v1 media slice spec + signoff |
| Multi-feed compositor mixing and adaptive bitrate suite | High complexity/churn without base media ingress proven | After single-stream path passes validation budgets |
| Audio policy/routing engine | Premature without stable video ingest/control contract | After signaling and runtime gate specs land |

## Appendix

### A. Repository Map
- Core: Rust workspace (`crates/`), protocol (`tze_hud_protocol`), runtime/compositor/scene crates.
- Doctrine: `about/heart-and-soul/`.
- Design contracts/specs: `openspec/changes/v1-mvp-standards/specs/`.
- Validation/tests: `tests/`, crate-local tests.

### B. Critical Workflows
1. Agent session establish -> lease grant -> scene/zone publish.
2. Zone publish validation by media type + contention policy.
3. Reconnect flow using full `SceneSnapshot` in v1 (no incremental media replay).

### C. Spec Inventory (media-relevant subset)
- `about/heart-and-soul/v1.md` (v1 boundaries and explicit deferrals).
- `about/heart-and-soul/architecture.md` (three-plane architecture and media doctrine).
- `openspec/.../session-protocol/spec.md` (media signaling deferred clauses).
- `openspec/.../runtime-kernel/spec.md` (media worker pool deferred clauses).
- `openspec/.../validation-framework/spec.md` (validation harness expectations).

### D. Evidence Index
- `about/heart-and-soul/v1.md:115`
- `about/heart-and-soul/architecture.md:27`
- `about/heart-and-soul/architecture.md:215`
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:698`
- `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:374`
- `crates/tze_hud_scene/src/types.rs:1419`
- `crates/tze_hud_scene/src/types.rs:1514`
- `crates/tze_hud_protocol/src/convert.rs:330`
- `crates/tze_hud_scene/src/lease/budget.rs:533`
- `crates/tze_hud_scene/tests/zone_ontology.rs:709`
- `crates/tze_hud_protocol/proto/session.proto:190`

---

## Conclusion

**Real direction**: Keep v1 as a deterministic non-live-media presence runtime while preparing a spec-first, tightly bounded post-v1 media/WebRTC entry path.

**Work on next**: (1) write a dedicated media/WebRTC capability spec, (2) define session-protocol media signaling delta, (3) define runtime media activation gate with quantified budgets and validation requirements.

**Stop pretending**: v1 can or should ship live WebRTC/GStreamer media now; it cannot without violating current doctrine and causing churn.
