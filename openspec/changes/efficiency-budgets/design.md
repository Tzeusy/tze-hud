## Context

The efficiency doctrine makes compute and LLM token cost product properties rather than optional optimization work. The runtime-kernel spec already requires incremental Stage 5 layout and defines the frame pipeline, while the validation-framework spec already defines hardware-normalized CPU, GPU, and upload calibration. Neither contract currently proves that a settled scene stops submitting GPU work, that a small change stays small through every render stage, that the normalized budgets hold under a deliberately constrained execution envelope, or that canonical LLM interactions remain compact.

This change is a contract-only bridge from doctrine to later implementation beads. It spans the windowed and headless event loops, compositor invalidation, telemetry, validation artifacts, CI calibration, and MCP/projection fixture surfaces. Owner approval of the quantitative delta is a post-delivery gate. No runtime code, CI workflow, baseline data, dependency, or device port is changed here.

## Goals / Non-Goals

**Goals:**

- Turn "idle screens cost nothing" into zero GPU submissions/acquisitions/presents plus a bounded CPU-wakeup requirement with an exact measurement interval.
- Define an observable invalidation closure so change-proportional work can be audited stage by stage rather than inferred from frame time.
- Add a fail-closed constrained-envelope lane that uses the existing calibration vector under a software renderer and two-logical-CPU limit.
- Define deterministic request/response byte and token measurements for zone publish, a complete cooperative portal turn, and widget publish.
- Establish owner-approved baseline compatibility and a quantitative 5-percent regression gate.

**Non-Goals:**

- Implementing event-loop pacing, damage tracking, telemetry counters, benchmark harnesses, tokenizer support, CI workflows, or baseline files.
- Selecting an exact renderer implementation, partial-present API, CPU-affinity mechanism, CI vendor, or tokenizer package.
- Claiming that WARP/llvmpipe on two logical CPUs qualifies smart-glasses or VR hardware.
- Changing protocol wire formats, public APIs, current Windows-only execution scope, or the existing frame/latency ceilings.

## Decisions

### D1: Idle is a state transition followed by a fixed observation window

**Choice:** A scene becomes contractually quiescent no later than five seconds after its final presentation-relevant event. The gate then observes a controlled 60-second interval. During that interval GPU submissions, surface acquisitions, and presents are exactly zero, while the combined runtime-driven main-plus-compositor wakeup count is at most 120. Normal headless operation is event/deadline-driven; fixed-cadence pacing is an explicit active benchmark/test mode, is recorded in artifacts, and cannot produce quiescent evidence.

**Rationale:** A five-second deadline is generous enough for bounded fades, caret state, TTL work, and queued telemetry to settle without allowing a permanent 60 Hz loop to masquerade as startup work. Sixty seconds converts the CPU ceiling into an unambiguous count and is long enough to reveal periodic pacing timers. Two wakeups per second allows bounded housekeeping while rejecting frame-rate polling by two orders of magnitude. Source attribution prevents the measurement sampler or unrelated operating-system activity from being charged to the runtime.

**Alternatives considered:**

- *Require zero CPU wakeups:* attractive in principle, but brittle across event-loop backends and bounded housekeeping; the zero-GPU invariant is the strict battery-critical line.
- *Measure process CPU percentage:* too dependent on host load and sampling resolution to diagnose why idle work occurred.
- *Use only a short unit-test interval:* faster, but periodic timers can evade it.

### D2: Proportionality is proved by an invalidation closure, not elapsed time

**Choice:** Every presentation-relevant change produces a typed invalidation closure. Layout, raster, upload, render encoding, and damaged pixels are attributed to separate per-category work-item identities, with actual-operation and closure cardinalities emitted together. Closure cardinality counts unique eligible work items; actual work counts every execution, including repeated processing of one eligible item. Encoded draw calls remain a companion metric, not a proxy for all encoding work. Out-of-closure work in any category is forbidden. Full-surface work is allowed only with a structured reason and does not count as ordinary proportional damage.

**Rationale:** Timing can remain green while a fast desktop GPU redraws the world. Closure accounting directly answers whether unchanged content was touched and remains meaningful across hardware. Including dependency reasons handles legitimate expansion through parent layout, transparent overlap, and runtime chrome without silently redefining every full redraw as "affected."

**Alternatives considered:**

- *Use draw-call count alone:* batching can reduce draw calls while still rasterizing or uploading unchanged content.
- *Require the changed node alone:* incorrect for layout dependents, overlapping transparency, and chrome.
- *Permit backend-wide full redraw as a normal pass:* would make the contract untestable on the active Windows path and erase the doctrine requirement.

### D3: The constrained lane preserves normalized ceilings

**Choice:** At least one gating lane runs WARP or llvmpipe with an enforced two-logical-CPU process limit, executes the same versioned CPU/GPU/upload calibration vector, records complete profile identity, and applies the reference lane's normalized ceilings without widening them.

**Rationale:** Reusing the existing vector isolates CPU, composition, and upload capacity while exposing assumptions that only survive desktop headroom. The lane is fail-closed when constraints or calibration are missing. Keeping normalized ceilings identical makes this a portability pressure test rather than a separate, weaker definition of acceptable behavior.

**Alternatives considered:**

- *Require a named physical low-power device:* reproducibility and runner availability are not yet established, and device implementation remains deferred.
- *Use a single slowdown scalar:* conflicts with the existing vector because CPU, GPU, and upload bottlenecks scale differently.
- *Widen budgets for software rendering:* would normalize the constraint twice and hide algorithmic regressions.

### D4: Canonical token cost measures deterministic JSON-RPC bodies

**Choice:** The canonical vector measures request, response, and total UTF-8 bytes and tokens for `publish_to_zone`, `portal_projection_attach` + publish + bounded long-poll + acknowledgement, and `publish_to_widget`. Fixtures pin framing, content, IDs, clocks, operation order, and deterministic responses. Transport headers and credentials are excluded; dynamic secret-bearing fields are replaced with fixed canonical sentinels before measurement. Tokenizer identity and vocabulary fingerprint are part of the result.

**Rationale:** JSON-RPC bodies are the stable content an LLM-facing client emits and receives. Separating request and response exposes regressions in either direction. The full portal turn captures setup and the append/coalesce/long-poll interaction pattern rather than benchmarking only the smallest call. Pinning fixture and tokenizer identity makes token counts reproducible and safe to compare.

**Alternatives considered:**

- *Count bytes only:* misses tokenizer-sensitive schema verbosity and does not measure the currency named by doctrine.
- *Include HTTP headers and bearer tokens:* adds transport/client noise and risks secret capture without improving API-shape measurement.
- *Measure live arbitrary conversations:* realistic but non-deterministic and unsuitable as a regression authority.

### D5: Compatible baselines fail above five percent and surface every increase

**Choice:** Each flow has a checked-in owner-approved baseline containing every per-operation and per-flow request, response, and total byte/token value emitted by the calibration artifact. Any compatible value increase above 5 percent fails. Increases up to 5 percent may pass only with a structured warning. Fixture, schema, or tokenizer drift yields `baseline_incompatible`; a newly versioned baseline cannot become authoritative without owner approval.

**Rationale:** Deterministic fixtures need no statistical noise allowance, but a small reviewable band avoids turning additive protocol evolution into an automatic emergency. Comparing each direction as well as totals prevents a response regression from being hidden by a request reduction. Fail-closed compatibility and approval stop baseline regeneration from laundering a regression.

**Alternatives considered:**

- *Absolute global token ceilings immediately:* no approved measurements exist yet, so choosing numbers would be speculative.
- *Any one-token increase fails:* maximally strict but makes deliberate schema growth unnecessarily disruptive.
- *Twenty-percent threshold:* too loose for a deterministic product metric and can compound rapidly.

### D6: The initial v1 token authority is the revised canonical-client packet

**Choice:** The initial comparison authority is the owner-approved revised
canonical-client packet from `hud-ht1k7`: `tiktoken-rs` `0.12.0`,
`o200k_base`, vocabulary SHA-256
`446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`,
fixture fingerprint
`blake3:86774ba0b39a5d1e812a9705fe0221d3071425d3b73a2ad07aada041530c1601`,
and the exact zone, portal, and widget values in the validation-framework
delta. The initial baseline records `approval.status=owner_approved` and
`approval.decision_reference=hud-ht1k7`. Zone and widget use MCP-standard
`tools/call`; portal measurements retain the production client's operation
discriminators.

**Rationale:** An independent audit found that the superseded candidate used
legacy bare-method envelopes for zone/widget and omitted the portal operation
discriminators. Pinning the corrected framing, identity, fingerprints, and
provenance prevents a future baseline edit from silently comparing a different
client contract or laundering token growth.

**Alternatives considered:**

- *Retain the earlier candidate:* rejected because it measured a non-canonical
  client shape and its approval did not transfer to the corrected packet.
- *Keep the exact values only in the checker JSON:* rejected because the
  normative OpenSpec contract must state the approved product budget, not
  merely point at an implementation artifact.

## Risks / Trade-offs

- **[Risk] The five-second settling allowance hides a slow animation or timer leak** → The artifact reports the actual quiescence transition time, and later implementation can tighten the deadline with evidence without weakening zero work during observation.
- **[Risk] Invalidation closures are inflated until every redraw appears valid** → Closure members require typed dependency reasons, and the canonical one-node/50-tile scenario has no legitimate unrelated dependencies.
- **[Risk] Software-renderer calibration normalizes away inefficient application work** → Calibration workloads are versioned and stability-checked; closure counters provide a hardware-independent companion gate.
- **[Risk] Token counts differ across model families** → The regression authority pins one tokenizer and vocabulary fingerprint while byte counts remain tokenizer-independent; other tokenizers can be reported as non-authoritative dimensions later.
- **[Risk] A 5-percent relative threshold rounds ambiguously for small counts** → The implementation must compare exact integer counts using `measured * 100 > baseline * 105`, avoiding floating-point and rounding ambiguity.
- **[Risk] The constrained lane is mistaken for device qualification** → Artifacts and the normative spec label it a low-power proxy and explicitly exclude glasses/VR qualification.

## Migration Plan

1. Obtain explicit owner signoff on this proposal, design, and quantitative delta before unblocking implementation beads.
2. Add telemetry schema/counters and deterministic fixtures without enabling gates; emit candidate artifacts for review.
3. Implement the idle and invalidation-closure scenarios, then enforce their zero/bounded counters.
4. Add the constrained-envelope runner using the existing calibration vector and prove its constraint identity before making it gating.
5. Add the pinned tokenizer and canonical-flow harness, collect candidate baselines, and obtain owner approval for the initial checked-in authority.
6. Enable byte/token regression gating and publish structured trends. Rollback consists of disabling a newly introduced CI gate while retaining artifacts for diagnosis; normative budgets and approved baselines require a new reviewed spec delta to change.

## Open Questions

- Resolved 2026-07-17 by owner decision `hud-ht1k7`: the first authority is `tiktoken-rs` `0.12.0` with `o200k_base` and vocabulary SHA-256 `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`.
- Which existing CI lane can enforce two logical CPUs most reliably on the selected WARP or llvmpipe runner? The artifact must record and prove the chosen mechanism.
