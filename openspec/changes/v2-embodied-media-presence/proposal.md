## Why

V1 intentionally proves governed on-screen presence with bounded surface area. It explicitly defers live media, WebRTC, embodied presence, exercised mobile/glasses profiles, and broader orchestration. That boundary is correct for v1, but it leaves the core thesis only partially realized: the doctrine says tze_hud is a presence engine that can eventually hold space, stream media, synchronize cues, and react across real devices.

The repo now has enough v1 and post-v1 contract work to define a credible v2 program:

1. a bounded media ingress contract exists,
2. the runtime already has the governance primitives that media and embodied presence must obey,
3. the project doctrine already names media, interaction, synchronization, and revocation as first-class concepts.

V2 should therefore be the release where the system becomes a true multi-plane presence engine rather than a v1-safe subset of one.

## What Changes

This change defines a v2 program centered on four capability expansions:

1. **Media plane activation.** Admit governed WebRTC/media as a real runtime capability, starting from bounded ingress and extending to bidirectional AV only behind explicit validation and operator policy.
2. **Embodied presence.** Add a third presence level beyond guest and resident, with stronger transport/session semantics and human-visible governance.
3. **Device-profile execution.** Exercise mobile and glasses device profiles as real deployment targets rather than schema-only placeholders.
4. **End-to-end validation and operations.** Extend validation, observability, and failure handling so media/device/embodied behavior is measurable and operable.

## Capabilities

### New capabilities

- `media-plane`: WebRTC signaling, media ingress/egress, stream lifecycle, media clocks, and operator-governed activation.
- `embodied-presence`: embodied-capability negotiation, richer session/device semantics, and coordinated media/presence behavior.
- `device-profile-execution`: exercised mobile/glasses runtime profiles with explicit upstream-composition and degradation contracts.

### Modified capabilities

- `session-protocol`: adds media signaling and richer presence negotiation.
- `runtime-kernel`: adds media worker lifecycle, activation gates, and decode/render degradation behavior.
- `validation-framework`: adds media/device/operator validation lanes.
- `configuration`: adds device/media enablement and deployment-shape controls.

## Impact

- Adds a new v2 OpenSpec program rather than mutating v1 promises.
- Converts the existing bounded-ingress contract into the first phase of a broader v2 media program.
- Creates a staged path from bounded ingress to embodied/bidirectional media without collapsing the project's governance model.
- Forces explicit planning for device profiles, privacy/operator controls, validation lanes, and failure handling before broad media claims are made.
