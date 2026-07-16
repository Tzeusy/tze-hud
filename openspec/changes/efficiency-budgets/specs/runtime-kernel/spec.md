## ADDED Requirements

### Requirement: Quiescent Runtime Has Zero Render-Driven GPU Work
The runtime MUST enter a quiescent state no later than 5 seconds after the last scene mutation, input-local-state change, animation deadline, media-frame arrival, TTL expiry, resize, or other presentation-relevant event has been processed. While the scene remains unchanged and no animation, composer caret, media clock, pending mutation, readback, or operator-requested capture requires presentation, the runtime SHALL issue zero render-driven GPU queue submissions, acquire zero surface frames, and call present zero times. In a controlled 60-second quiescent interval with no external input or network traffic, runtime-driven main/compositor event-loop wakeups MUST average no more than 2 per second. Benchmark sampling and operating-system wakeups not requested by the runtime MUST be identified separately and MUST NOT be counted as runtime-driven wakeups.
Source: about/heart-and-soul/efficiency.md section "Compute budget"; RFC 0002 sections 1.2, 3.2, and 8.4
Scope: v1-mandatory

#### Scenario: Static scene performs no GPU work
- **WHEN** a windowed overlay or headless scene has processed its final change, has no active animations or timed presentation work, and remains quiescent for a 60-second measurement interval beginning after the 5-second settling deadline
- **THEN** the runtime MUST report exactly 0 GPU queue submissions, 0 surface acquisitions, and 0 presents during the measurement interval

#### Scenario: Static scene has bounded runtime wakeups
- **WHEN** the controlled quiescent-scene measurement runs for 60 seconds with external input and network traffic disabled
- **THEN** the runtime-driven main/compositor event-loop wakeup count MUST be no greater than 120, and the artifact MUST report the count, interval duration, wakeup sources, and excluded sampler or operating-system wakeups

#### Scenario: Presentation-relevant change exits quiescence
- **WHEN** a scene mutation, input-local-state change, animation deadline, media-frame arrival, TTL expiry, resize, readback, or operator-requested capture occurs while the runtime is quiescent
- **THEN** the runtime MUST leave quiescence, process the eligible work, and MUST NOT classify the resulting submission, present, or wakeup as idle work

### Requirement: Render Work Is Proportional to the Invalidation Closure
For every committed presentation-relevant change, the runtime MUST compute an invalidation closure containing the changed node or runtime-owned surface state plus only the layout dependents, visual dependents, overlapping compositing contributors, and chrome regions whose output can be affected by that change. Layout resolution, text or image rasterization, texture upload, render encoding, and composition damage SHALL be attributed to this closure. Unchanged nodes and tiles outside the closure MUST incur zero layout resolutions, rasterizations, or texture uploads. The damaged pixel region MUST be contained within the union of the closure's affected output bounds, except when the runtime records a structured full-surface invalidation reason such as surface creation, resize, device recovery, or an explicitly unsupported partial-present backend. The runtime MUST emit per-change closure cardinalities, actual-work counts, damaged-pixel area, viewport area, and structured amplification ratios so full-scene fallback cannot be mistaken for proportional work.
Source: about/heart-and-soul/efficiency.md section "Compute budget"; RFC 0002 section 3.2 Stages 5-7
Scope: v1-mandatory

#### Scenario: One-node content change excludes unrelated tiles
- **WHEN** one text node changes in a 50-tile scene and the change has no layout, overlap, chrome, or resource dependency on the other 49 tiles
- **THEN** the invalidation closure MUST contain only the changed node and its affected tile region, all 49 unrelated tiles MUST report zero layout resolutions, rasterizations, and texture uploads, and the damaged pixel area MUST NOT exceed the affected tile region

#### Scenario: Dependency expansion remains explicit and bounded
- **WHEN** a change affects parent layout, overlapping transparent content, or runtime chrome and therefore expands beyond the directly changed node
- **THEN** every additional node, tile, or region that receives work MUST be named by a dependency reason in the invalidation closure, and actual layout, raster, upload, and damage counts MUST NOT exceed their corresponding closure counts or area

#### Scenario: Full-surface invalidation is diagnostic
- **WHEN** the runtime damages the full surface because of surface creation, resize, device recovery, or a backend that explicitly lacks the required partial-present capability
- **THEN** the efficiency telemetry MUST record the full-surface invalidation reason and backend capability, and MUST NOT report the event as satisfying the ordinary change-proportional damage path
