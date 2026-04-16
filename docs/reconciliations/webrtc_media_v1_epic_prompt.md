# WebRTC and Media V1 Epic Prompt

Use `/project-direction` end-to-end for this epic. This is a spec-first direction pass, not an implementation sprint.

## Objective

Evaluate the proposal to bring WebRTC/media into v1 scope and produce a grounded work plan that answers one question clearly:

Should v1 actually absorb media/WebRTC now, and if so, in what smallest credible form that does not destabilize the compositor, protocol contracts, validation architecture, or operator surface?

## Why this epic exists

Current repo doctrine is split:

- [about/heart-and-soul/vision.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/vision.md) and [about/heart-and-soul/architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md) treat media/WebRTC as part of the long-term architecture.
- [about/heart-and-soul/v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md) explicitly defers GStreamer, video decode, live camera feeds, WebRTC, and clocked media/cues from v1.
- [README.md](/home/tze/gt/tze_hud/mayor/rig/README.md) currently advertises WebRTC/media more aggressively than the v1 doctrine.
- [openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md) reserves future space for embodied/media signaling rather than defining a current v1 contract.

If media/WebRTC is to enter v1, this repo needs a new canonical contract rather than aspiration drift.

## Scope

Focus on:

- whether WebRTC/media should be admitted into v1 at all
- smallest credible v1 media slice if admitted
- transport boundaries among gRPC, MCP, and WebRTC
- media timing model implications
- compositor and resource-budget consequences
- validation and benchmark consequences
- operator/deployment/security implications

Do not start implementation in this pass. Produce the direction report and beads decomposition only.

## Required evidence set

Read at minimum:

- [about/heart-and-soul/vision.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/vision.md)
- [about/heart-and-soul/v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md)
- [about/heart-and-soul/architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md)
- [about/heart-and-soul/mobile.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/mobile.md)
- [about/heart-and-soul/validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md)
- [about/heart-and-soul/failure.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/failure.md)
- [about/heart-and-soul/presence.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/presence.md)
- [about/legends-and-lore/rfcs/0001-scene-contract.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0001-scene-contract.md)
- [about/legends-and-lore/rfcs/0003-timing.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0003-timing.md)
- [about/legends-and-lore/rfcs/0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md)
- [openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md)
- [openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md)
- [openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md)
- [crates/tze_hud_runtime](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime)
- [crates/tze_hud_scene/src/types.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_scene/src/types.rs)
- [crates/tze_hud_protocol](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol)

## Questions the direction pass must answer

1. What exact user-visible capability justifies promoting media/WebRTC into v1?
2. What is the smallest slice that proves the thesis without exploding scope?
3. Should v1 media mean:
   - transport and signaling only
   - one receive-only video surface path
   - one bidirectional embodied session path
   - subtitle/media clock integration
   - some narrower proof
4. What new specs are required before implementation begins?
5. What validation layers must be extended so media remains testable by LLM-driven workflows?
6. What operator/security hardening is required before shipping WebRTC/media as part of a canonical runtime story?
7. Which currently-advertised claims should remain future scope even if some media enters v1?

## Output requirements

Produce the full `/project-direction` output, including:

- executive summary
- contradictions and gap analysis
- aligned next steps vs premature work
- chunked work plan with spec references
- explicit “do not do yet” section
- blunt conclusion

Then materialize a beads epic with:

- spec-writing children first where coverage is missing
- implementation children only after spec signoff
- one reconciliation bead
- one implementation report bead

## Non-negotiable constraints

- Keep the runtime local and compositor-owned.
- Do not place model logic in render or media timing loops.
- Preserve the doctrine that arrival time is not presentation time.
- Do not widen v1 in a way that destroys current validation credibility.
- Treat security, privacy, and operator ergonomics as first-class scope, not afterthoughts.

## Recommended decomposition shape

The likely chunk pattern should look roughly like:

1. Direction/spec-first decision on whether media/WebRTC belongs in v1
2. New or updated OpenSpec/RFC contract work
3. Transport/signaling seam definition
4. Runtime/compositor/resource integration plan
5. Validation and benchmark plan
6. Operator/security hardening plan
7. Documentation/report/reconciliation

## Success condition

At the end of the direction pass, a separate implementer should be able to answer:

- whether media/WebRTC is truly in v1
- what exact slice is in scope
- what specs govern it
- what can be deferred safely
- what implementation order minimizes churn
