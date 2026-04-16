# WebRTC/Media V1 Signaling Shape Decision (WM-S2a)

Date: 2026-04-09  
Issue: `hud-nn9d.7`  
Parent epic: `hud-nn9d`  
Depends on: `hud-nn9d.6` (bounded ingress capability contract)

## Decision

For the first bounded post-v1 media ingress slice, signaling SHALL use a
**session-stream envelope extension** shape:

- Extend the existing `HudSession.Session` bidirectional stream envelope.
- Allocate post-v1 media signaling payloads inside the spec-designated
  session field range (`50-99`) during `WM-S2b` (and add concrete protobuf
  `reserved` declarations as part of that schema change).
- Do **not** introduce a separate `rpc MediaSignaling(...)` in this tranche.

## Why This Is Lower Churn In This Repo

1. Topology already converges on one primary stream per agent. The architecture doctrine says session traffic is multiplexed on one stream and warns against proliferating long-lived streams (`about/heart-and-soul/architecture.md:130-134`).
2. The current protocol/runtime wiring is already centralized in `HudSession.Session` (`crates/tze_hud_protocol/proto/session.proto:22-27`), including zone publish on this stream (`crates/tze_hud_protocol/proto/session.proto:52-55`).
3. The post-v1 spec/RFC path already reserves envelope space for embodied/media signaling (`openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:712-714`, `about/legends-and-lore/rfcs/0005-session-protocol.md:1491`).
4. `ZonePublish` already compiles into the scene mutation path in one place (`crates/tze_hud_protocol/src/session_server.rs:3737-3801`), and mutation-based publish also already exists (`crates/tze_hud_protocol/proto/types.proto:328-333`). A session-stream envelope-extension decision avoids splitting this seam across two transport APIs before schema/snapshot equivalence is fixed.

Given these constraints, a separate media RPC now would add an additional control-plane seam (new service method, auth/version gating, reconnect semantics, telemetry/backpressure policy) while the bounded slice still targets one-way ingress and keeps embodied/bidirectional session semantics deferred.

## Performance Trade-Off (HOL Blocking) And Traffic Class Contract

Using the existing multiplexed session stream introduces potential
head-of-line pressure: large or complex media signaling payload handling could
delay other session messages (for example scene mutations or input events).

`WM-S2b` MUST therefore include:

1. **Per-payload traffic class documentation** for each new session-envelope
   media variant (rather than one class for the envelope as a whole), so
   delivery guarantees remain explicit when transactional and ephemeral flows
   share a transport.
2. **Message-size/processing constraints** and denial semantics that preserve
   bounded ingress behavior under congestion or malformed payloads.
3. **Operational guidance** for prioritization and fairness so existing v1
   traffic retains predictable latency in mixed workloads.

## Compatibility And Downgrade Contract

1. **V1 compatibility is unchanged.** V1 still exposes no active media signaling stream and keeps embodied/media signaling post-v1 (`openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:712-719`).
2. **Feature gating is explicit.** Agents MUST treat media signaling as unavailable unless both are true:
   - negotiated protocol version supports the `WM-S2b` media fields; and
   - session capability grant includes media-ingress capability for this slice.
3. **Downgrade behavior is deterministic.**
   - If either gate is missing, agents MUST fall back to existing non-media zone publication behavior and MUST NOT attempt media signaling payloads.
   - If policy/config keeps media disabled (default-off runtime posture), runtime denial is treated as normal downgrade, not as transport failure.
4. **Reconnect behavior follows existing session rules.** On reconnect, clients re-evaluate capability/version from `SessionResumeResult` before resuming any media-intent flow; no separate media session resumption path is introduced in this tranche.

## Deferred Boundary

A separate media RPC remains deferred and may be reconsidered only when embodied/bidirectional media scope is admitted (currently deferred) and the contract requires semantics that cannot be represented cleanly inside the session envelope without violating the "few fat streams" topology rule.

## Consequence For WM-S2b

`WM-S2b` specifies concrete protobuf field allocations, reconnect/snapshot rules,
and compatibility/failure semantics under this chosen shape in:
`docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md`.
