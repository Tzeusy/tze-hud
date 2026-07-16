## ADDED Requirements

### Requirement: Quiescent Runtime Has Zero Runtime-Driven GPU Work
The runtime MUST enter a quiescent state no later than 5 seconds after the last scene mutation, input-local-state change, animation deadline, media-frame arrival, TTL expiry, resize, or other presentation-relevant event has been processed. While the scene remains unchanged and no animation, composer caret, media clock, pending mutation, readback, or operator-requested capture requires presentation, the runtime SHALL issue zero runtime-driven GPU queue submissions, acquire zero surface frames, and call present zero times. In a controlled 60-second quiescent interval with no external input or network traffic, the combined runtime-driven main-plus-compositor event-loop wakeup count MUST be no greater than 120. Benchmark sampling and operating-system wakeups not requested by the runtime MUST be identified separately and MUST NOT be counted as runtime-driven wakeups. Normal headless operation MUST be event/deadline-driven. A fixed-cadence headless scheduler MUST require explicit benchmark/test activation, MUST identify its pacing mode and requested cadence, and MUST NOT be classified as quiescent or used as evidence for this requirement.
Source: about/heart-and-soul/efficiency.md section "Compute budget"; RFC 0002 sections 1.2, 3.2, and 8.4
Scope: v1-mandatory

#### Scenario: Static scene performs no GPU work
- **WHEN** a windowed overlay or headless scene has processed its final change, has no active animations or timed presentation work, and remains quiescent for a 60-second measurement interval beginning after the 5-second settling deadline
- **THEN** the runtime MUST report exactly 0 GPU queue submissions, 0 surface acquisitions, and 0 presents during the measurement interval

#### Scenario: Static scene has bounded runtime wakeups
- **WHEN** the controlled quiescent-scene measurement runs for 60 seconds with external input and network traffic disabled
- **THEN** the combined runtime-driven main-plus-compositor event-loop wakeup count MUST be no greater than 120, and the artifact MUST report the combined count, per-loop counts, interval duration, wakeup sources, and excluded sampler or operating-system wakeups

#### Scenario: Fixed-cadence headless mode is active work
- **WHEN** a benchmark or test explicitly activates fixed-cadence headless pacing, including the default 60fps benchmark target
- **THEN** the runtime MUST record the pacing mode and requested cadence, MUST classify timer-driven frames as active benchmark/test work, and MUST NOT report the run as quiescent-idle evidence

#### Scenario: Presentation-relevant change exits quiescence
- **WHEN** a scene mutation, input-local-state change, animation deadline, media-frame arrival, TTL expiry, resize, readback, or operator-requested capture occurs while the runtime is quiescent
- **THEN** the runtime MUST leave quiescence, process the eligible work, and MUST NOT classify the resulting submission, present, or wakeup as idle work

### Requirement: Render Work Is Proportional to the Invalidation Closure
For every committed presentation-relevant change, the runtime MUST compute an invalidation closure containing the changed node or runtime-owned surface state plus only the layout dependents, visual dependents, overlapping compositing contributors, and chrome regions whose output can be affected by that change. Layout resolution, text or image rasterization, texture upload, render encoding, and composition damage SHALL be attributed to separate typed per-category closure work-item identities. A closure cardinality MUST count unique eligible work-item identities, while its corresponding actual-work count MUST count every operation, including repeated processing of the same eligible identity; encoded draw calls MUST be reported separately and MUST NOT stand in for non-draw encoding work. Unchanged nodes and tiles outside the closure MUST incur zero layout resolutions, rasterizations, texture uploads, or render-encoding operations. The damaged pixel region MUST be contained within the union of the closure's affected output bounds, except when the runtime records a structured full-surface invalidation reason such as surface creation, resize, device recovery, or an explicitly unsupported partial-present backend. The runtime MUST emit per-change per-category closure cardinalities, actual-work counts, damaged-pixel area, viewport area, and structured amplification ratios so full-scene fallback cannot be mistaken for proportional work.
Source: about/heart-and-soul/efficiency.md section "Compute budget"; RFC 0002 section 3.2 Stages 5-7
Scope: v1-mandatory

#### Scenario: One-node content change excludes unrelated tiles
- **WHEN** one text node changes in a 50-tile scene and the change has no layout, overlap, chrome, or resource dependency on the other 49 tiles
- **THEN** the invalidation closure MUST contain only the changed node and its affected tile region, all 49 unrelated tiles MUST report zero layout resolutions, rasterizations, texture uploads, and render-encoding operations, and the damaged pixel area MUST NOT exceed the affected tile region

#### Scenario: Dependency expansion remains explicit and bounded
- **WHEN** a change affects parent layout, overlapping transparent content, or runtime chrome and therefore expands beyond the directly changed node
- **THEN** every additional node, tile, region, or render-plan item that receives work MUST be named by a dependency reason in the invalidation closure, and actual layout, raster, upload, render-encoding, and damage counts MUST NOT exceed their corresponding typed closure counts or area

#### Scenario: Full-surface invalidation is diagnostic
- **WHEN** the runtime damages the full surface because of surface creation, resize, device recovery, or a backend that explicitly lacks the required partial-present capability
- **THEN** the efficiency telemetry MUST record the full-surface invalidation reason and backend capability, and MUST NOT report the event as satisfying the ordinary change-proportional damage path
