# Core Foundations

- Total estimated smart-human study time: 39 hours
- Keep this path at or below 100 hours.

## Section Overview

| Module | Why You Need It | Estimated Hours | Depends On | Progress |
|---|---|---:|---|---|
| [01 Scene, Compositor, and Runtime Sovereignty](modules/01-scene-and-compositor-prereqs.md) | Gives the foundational mental model: why the runtime owns pixels, how the scene graph relates to the compositor, and why this repo is not a browser/app shell. | 6 | None | [ ] |
| [02 Async Rust, Streaming RPC, and Backpressure](modules/02-async-rust-streaming-rpc-and-backpressure.md) | Explains the single-stream agent protocol, protobuf/codegen boundary, async Rust expectations, and bounded queue semantics. | 8 | 01 | [ ] |
| [03 Time, Clocks, and Sync-Safe Scheduling](modules/03-time-clocks-and-sync-safe-scheduling.md) | Covers wall vs monotonic time, scheduled presentation, expiry, and sync groups so timing-related code stops looking arbitrary. | 6 | 01, 02 | [ ] |
| [04 Leases, Capabilities, Privacy, and Degradation](modules/04-leases-capabilities-privacy-and-degradation.md) | Teaches the repo’s governance model: admission, revocation, redaction, attention limits, and runtime-owned failure response. | 7 | 01, 02, 03 | [ ] |
| [05 Resources, Zones, Widgets, and Publishing Surfaces](modules/05-resources-zones-widgets-and-publishing-surfaces.md) | Explains content-addressed assets plus the runtime-owned abstractions agents use to publish intent rather than raw geometry. | 5 | 01, 02, 04 | [ ] |
| [06 Validation, Telemetry, Config, and Safe Change Workflow](modules/06-validation-telemetry-config-and-safe-change-workflow.md) | Turns the architecture into day-to-day engineering practice: how to run the system, interpret tests, trust artifacts, and avoid unsafe edits. | 7 | 01-05 | [ ] |

## Stop Here If

Stop after module 4 if your immediate goal is to read the codebase, follow discussions, or review design work at a high level.

Finish all six modules before making protocol, runtime, policy, resource, or publishing-surface changes.
