## Why

The efficiency doctrine now makes idle compute, change-proportional work, constrained-hardware performance, and LLM token cost product requirements, but the canonical capability specs do not yet make those properties measurable or gateable. Without quantitative scenarios and stable calibration artifacts, implementation beads can optimize proxies while leaving the doctrine unproved.

## What Changes

- Require a quiescent scene to produce zero runtime-driven GPU submissions, surface acquisitions, or presents and no more than 120 combined main-plus-compositor runtime-driven wakeups over a 60-second interval after a five-second settling deadline; normal headless pacing is event/deadline-driven, while fixed cadence is explicit benchmark/test-only active work.
- Require layout, raster, upload, render encoding/draw, and composition work to stay inside the typed invalidation closure of an observed scene change, with structured amplification metrics that reject unchanged out-of-closure work.
- Extend the hardware-calibration vector with a gating constrained-envelope lane using a software renderer and a two-logical-CPU execution limit, while retaining the existing normalized performance ceilings.
- Add deterministic byte and token calibration for canonical `publish_to_zone`, text-stream portal turn, and `publish_to_widget` flows, with checked-in owner-approved baselines and explicit regression thresholds.
- Keep smart-glasses/VR device implementation, renderer selection, and implementation mechanics out of scope. This change defines contracts and validation gates only.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `runtime-kernel`: Add measurable idle-zero-work and work-proportional-to-change requirements for the sovereign render/event loop.
- `validation-framework`: Add the idle/change measurement protocol, constrained-envelope calibration lane, canonical LLM-flow byte/token calibration vector, and regression-gate semantics.

## Impact

- Future implementation will affect runtime event-loop pacing, render invalidation, compositor telemetry, and benchmark tooling.
- Validation artifacts gain efficiency counters, constrained-profile identity, calibration factors, canonical-flow fingerprints, byte/token measurements, and baseline comparisons.
- No protocol wire format, public API, dependency, runtime behavior, or CI workflow changes are implemented by this spec-first bead.
- Owner signoff on the complete OpenSpec delta remains the external gate before implementation beads may unblock.
