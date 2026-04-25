# Scene, Compositor, and Runtime Sovereignty

- Estimated smart-human study time: 6 hours
- Keep every module at or below 10 hours.

## Why This Module Matters

If you miss the repo’s core stance that the runtime is sovereign, almost every crate will look upside down. `tze_hud` is not letting agents “paint UI.” It is giving them governed presence inside a compositor that owns pixels, timing, input, and safety. That makes scene graph, tiles, zones, widgets, and shell chrome runtime concepts first and agent concepts second.

## Learning Goals

- Explain why the runtime/compositor boundary is the project’s primary architectural invariant.
- Distinguish scene graph, tile, zone, widget, node, layer, and chrome responsibilities.
- Understand why headless and windowed modes share one compositor model.

## Subsection: Screen Sovereignty and Scene Modeling

### Why This Matters Here

This repo assumes you already accept that “runtime owns the screen” is not branding language; it is a hard system boundary. Once you do, the rest of the architecture stops feeling arbitrary: scene is pure data, compositor owns GPU state, shell chrome is always runtime-owned, and agent APIs mostly publish intent rather than direct render commands.

### Technical Deep Dive

In compositor-style systems, the process that owns the display surface also has to own the timing and correctness boundaries. If outside actors can directly paint pixels, block the frame loop, or bypass input routing, you cannot provide deterministic frame budgets, safe fallback behavior, or consistent override controls.

The transferable concept here is separation between:
- semantic intent: what an external actor wants shown
- scene state: the structured model the runtime commits atomically
- composition/presentation: the renderer’s local, deterministic work

`tze_hud` expresses that separation with a pure scene graph (`Scene -> Tab -> Tile -> Node`) and a compositor/runtime that realizes it. Tiles are leased regions with geometry and z-order. Nodes are content inside tiles. Zones and widgets are runtime-owned managed publishing surfaces built on top of tile mechanics. The chrome layer is reserved for runtime controls and indicators.

The reason headless mode matters is the same reason pure scene state matters: if composition logic depends on a real window or interactive operator, the system becomes harder to test and therefore harder for both humans and LLMs to change safely. A sovereign runtime must be able to render the same scene model to an offscreen surface for CI.

### Where It Appears In The Repo

- `README.md`
- `about/heart-and-soul/architecture.md`
- `about/heart-and-soul/presence.md`
- `about/lay-and-land/components.md`
- `openspec/specs/runtime-kernel/spec.md`
- `openspec/specs/scene-graph/spec.md`

### Sample Q&A

- Q: Why does the repo insist that LLMs must never sit in the frame loop?
  A: Because deterministic composition, budget enforcement, and human override depend on a local runtime controlling presentation even when agents are slow, noisy, or disconnected.
- Q: Why are zones and widgets not just convenience UI widgets?
  A: They are managed publishing abstractions that keep geometry, rendering policy, contention, and runtime ownership local instead of pushing layout/render logic into agents.

### Progress

- [ ] Exposed: I can define the key terms in this subsection
- [ ] Working: I can explain the core idea in my own words
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can explain why a direct-rendering shortcut would violate the repo’s architecture

### Mastery Check

Target level: `working`

You should be able to explain why `tze_hud` is a sovereign runtime rather than a UI toolkit, and map tiles/zones/widgets/chrome to that model without notes.

## Module Mastery Gate

- [ ] I can summarize the core concepts in this module
- [ ] I can answer the hardest subsection Q&A without notes
- [ ] I can point to where these ideas appear in the repository
- [ ] I can explain the difference between pure scene state and runtime-owned composition

## What This Module Unlocks Next

It gives you the base mental model needed for understanding protocol handlers, queueing, timing, and policy as runtime concerns rather than “app logic.”

### Where It Appears In The Repo

- `openspec/specs/resource-store/spec.md`
- `about/heart-and-soul/presence.md`
- `about/heart-and-soul/v1.md`
- `openspec/specs/widget-system/spec.md`
- `crates/tze_hud_mcp/src/tools.rs`
- `tests/integration/presence_card_tile.rs`

### Sample Q&A

- Q: Why does the repo use BLAKE3-based `ResourceId` instead of path-based asset identity?
  A: Because identity follows immutable content, enabling deduplication and stable references regardless of upload path or caller.
- Q: When should an agent use a zone or widget instead of a raw tile?
  A: When the runtime already offers a managed surface or parameterized visual that matches the use case, because that preserves governance and keeps layout/render logic out of the agent.

### Progress

- [ ] Exposed: I can define `ResourceId`, zone, widget, and occupancy
- [ ] Working: I can explain the difference between the three publishing abstraction levels
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can explain why asset registration and publish are separate stages

### Mastery Check

Target level: `working`

You should be able to explain why resource identity is content-based and why the repo keeps most agents on zones/widgets instead of raw tile math.

## Module Mastery Gate

- [ ] I can summarize the asset identity model
- [ ] I can explain raw tiles vs zones vs widgets
- [ ] I can point to the MCP tool layer and at least one spec covering publishing
- [ ] I can explain why runtime-owned publishing reduces agent complexity

## What This Module Unlocks Next

It sets up the final module, where config, validation, telemetry, and safe contribution workflow turn these abstractions into day-to-day engineering practice.

*** Add File: /home/tze/gt/tze_hud/mayor/rig/curriculum/paths/core-foundations/modules/06-validation-telemetry-config-and-safe-change-workflow.md
# Validation, Telemetry, Config, and Safe Change Workflow

- Estimated smart-human study time: 7 hours
- Keep every module at or below 10 hours.

## Why This Module Matters

This repo is built to be changed through structured evidence, not intuition. Headless rendering, telemetry, calibrated performance checks, artifact generation, fail-closed startup, and canonical runtime configuration are all part of the engineering model. If you skip them, you will misunderstand both the tests and the app entrypoint.

## Learning Goals

- Explain the five validation layers and what each catches.
- Understand why logs, telemetry, and artifacts are primary debugging surfaces.
- Understand the canonical runtime app/config/deployment contract well enough to run or validate the system safely.

## Subsection: Evidence-Driven Runtime Engineering

### Why This Matters Here

`tze_hud` is explicitly designed for LLM-assisted development. That means the system must expose structured truth about rendering, timing, and behavior instead of relying on someone eyeballing a screen. The same design principle shows up in startup behavior: the runtime should fail closed, expose configuration errors clearly, and make listener/auth state explicit.

### Technical Deep Dive

The general idea is observability as product architecture. In systems that render natively rather than through a browser DOM, you need other ways to make correctness visible: pure logic tests, headless pixel readback, perceptual comparison, per-frame telemetry, and generated artifacts.

`tze_hud` adds a second concept on top of that: calibrated performance. Raw times from different machines are not directly comparable, so the repo normalizes performance by hardware factors. That changes what “passing performance” means.

The runtime config/deployment side follows the same philosophy. A canonical binary, a loader schema, explicit CLI/env precedence, and strict startup rules all reduce ambiguity. This is part of safe engineering because a contributor should be able to tell whether a failure is architectural, config-related, or simply an operator mistake.

### Where It Appears In The Repo

- `about/heart-and-soul/validation.md`
- `openspec/specs/validation-framework/spec.md`
- `app/tze_hud_app/src/main.rs`
- `app/tze_hud_app/tests/production_boot.rs`
- `about/lay-and-land/operations/DEPLOYMENT.md`
- `tests/integration/v1_thesis.rs`

### Sample Q&A

- Q: Why are developer artifacts and structured telemetry part of the core architecture here instead of optional tooling?
  A: Because the repo is designed to be validated by LLMs and humans through machine-readable evidence, not by manual visual inspection alone.
- Q: Why does the canonical app startup reject missing config or insecure default PSKs?
  A: Because fail-closed startup is part of runtime sovereignty; ambiguous or insecure runtime state should not silently limp into operation.

### Progress

- [ ] Exposed: I can define the five validation layers and fail-closed startup
- [ ] Working: I can explain why telemetry and artifacts are central in this repo
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can describe which validation layer I would use first for a given class of change

### Mastery Check

Target level: `working`

You should be able to explain how this repo turns runtime behavior into structured evidence and why the canonical runtime app/config path matters operationally.

## Module Mastery Gate

- [ ] I can summarize the five validation layers
- [ ] I can explain hardware-normalized performance at a high level
- [ ] I can point to the canonical app entrypoint and production boot tests
- [ ] I can describe a safe first-change workflow grounded in tests and artifacts

## What This Module Unlocks Next

After this module, you should be ready to read the repo with purpose, choose safer first tasks, and avoid the most common category errors when proposing or landing changes.
