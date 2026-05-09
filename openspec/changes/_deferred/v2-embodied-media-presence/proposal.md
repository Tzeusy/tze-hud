> **DEFERRED INDEFINITELY (2026-05-09).** This change is parked. The project has refocused on a performant single-device Rust HUD runtime for Windows before any multi-device, mobile, glasses, embodied-presence, or media-plane work. All 44 v2 beads (`hud-ora8.*`) were closed; the doctrine files this change relies on (`about/heart-and-soul/v2.md`, `mobile.md`, `media-doctrine.md`) are marked deferred. Active work tracked under epic `hud-9wljr` and openspec change `windows-first-performant-runtime`. **Do not pick up tasks from this change.** Multi-device scope returns only after the single-Windows runtime is done.
>
> Original proposal follows.

---

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
- `identity-and-roles`: role definitions (owner/admin/member/guest), user directory schema, and role-to-capability binding. Added post-signoff per C12/signoff-packet.md spec→decision mapping.
- `identity-portability`: device-reboot-persistent embodied identity with user-initiated cryptographic export/import for device migration. No cloud identity anchor. Added post-signoff per B9/signoff-packet.md spec→decision mapping.

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
