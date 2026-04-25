# Curriculum

This curriculum is for someone who wants to understand what `tze_hud` is doing technically before reading code deeply or making safe changes. It is not a repo walkthrough. It teaches the smallest set of underlying systems concepts that make the repository legible.

The shortest path is a single curriculum track because the active v1 surface is coherent: sovereign runtime/compositor design, streaming protocol boundaries, timing, governance, publishing abstractions, and validation discipline. Estimated total effort is about 39 hours for a smart, already-technical reader.

Mandatory before reading code:
- `01 Scene, Compositor, and Runtime Sovereignty`
- `02 Async Rust, Streaming RPC, and Backpressure`
- `03 Time, Clocks, and Sync-Safe Scheduling`
- `04 Leases, Capabilities, Privacy, and Degradation`

Best completed before first contribution work:
- `05 Resources, Zones, Widgets, and Publishing Surfaces`
- `06 Validation, Telemetry, Config, and Safe Change Workflow`

Nice to know later:
- Media-plane internals (`GStreamer`, `WebRTC`, device-profile execution) because v1 explicitly defers them, even though they shape the architecture.
- Accessibility bridge details and mobile/glasses deployment profiles.

## Overview

| Path or Section | Why You Need It | Estimated Hours | Progress |
|---|---|---:|---|
| [Core Foundations](paths/core-foundations/README.md) | Teaches the runtime/compositor model, protocol semantics, timing, policy, asset identity, publishing abstractions, and validation workflow that appear throughout the repo. | 39 | [ ] |
