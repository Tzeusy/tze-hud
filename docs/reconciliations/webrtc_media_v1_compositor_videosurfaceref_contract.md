# WebRTC/Media V1 Compositor Contract for VideoSurfaceRef Rendering (WM-S3c)

Date: 2026-04-09
Issue: `hud-nn9d.12`
Parent epic: `hud-nn9d`
Depends on: `hud-nn9d.6` (WM-S1), `hud-nn9d.9` (WM-S2c), `hud-nn9d.10` (WM-S3)

## Purpose

Define the compositor-side contract for rendering `VideoSurfaceRef` in the
bounded post-v1 media ingress slice.

This contract is normative for:

1. texture ownership and lifecycle boundaries,
2. present-time and expiry semantics at render time,
3. degradation and fallback render states,
4. non-audio rendering behavior.

This contract does not replace signaling/schema definitions (WM-S2a/WM-S2b),
zone identity/transport constraints (WM-S2c), activation gates/budgets (WM-S3),
or privacy/operator policy (WM-S3b).

## Inputs and Existing Constraints

1. `ZoneContent` includes `VideoSurfaceRef` in scene/types contracts as a
   post-v1 media payload (`openspec/changes/v1-mvp-standards/specs/scene-graph/spec.md:233`,
   `docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md:88`).
2. WM-S2b defines publication timing fields (`present_at_wall_us`,
   `expires_at_wall_us`) and snapshot-first resume semantics for media
   publication state (`docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md:102`,
   `:146`).
3. WM-S2c constrains media ingress to a fixed approved zone class with
   `transport_constraint = WebRtcRequired` and fixed `layer_attachment`
   (`openspec/specs/media-webrtc-bounded-ingress/spec.md:141`).
4. WM-S3 defines bounded-stream admission, degradation coupling, and level-4/5
   forced teardown within one compositor frame (`docs/reconciliations/webrtc_media_v1_runtime_activation_gate_budgets.md:76`,
   `:95`).
5. Runtime kernel contract keeps GPU `Device` + `Queue` ownership on compositor
   thread, with main thread performing `surface.present()` only
   (`openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:55`,
   `:147`).

## Contract Decision: Runtime-Owned Surface Realization

`VideoSurfaceRef` is a declarative render reference, not a client-owned GPU
handle. The runtime compositor owns all GPU texture realization and submission.

## Texture Ownership and Lifecycle Contract

1. `surface_id` in `VideoSurfaceRef` identifies logical media surface identity
   within runtime/session scope; it is not a borrowed GPU pointer.
2. The compositor thread exclusively owns allocation, update, and destruction of
   textures backing active `VideoSurfaceRef` content.
3. Publishers MUST NOT provide or assume direct GPU resource ownership,
   synchronization primitives, or memory-mapped texture access.
4. On ingress close, lease revocation, policy disable, or forced teardown, the
   runtime MUST release compositor-owned media texture resources deterministically
   and remove them from active render lists.
5. Resume epoch mismatch (or missing epoch confirmation) MUST prevent reuse of
   stale pre-disconnect transport texture state; runtime MUST treat that state as
   non-authoritative until re-open/reconciliation succeeds.

## Present-Time and Expiry Render Semantics

1. Compositor MUST honor `present_at_wall_us` no-earlier-than semantics for
   `VideoSurfaceRef` publication visibility.
2. Compositor MUST honor `expires_at_wall_us` cut-off semantics and stop
   rendering media content at or after expiry.
3. If publication becomes eligible to present (`present_at_wall_us` reached) but
   no decoded frame is available yet, compositor MUST render a deterministic
   media fallback state in the approved zone instead of presenting stale or
   undefined pixels.
4. When first valid frame becomes available, compositor MUST switch from fallback
   to active media rendering on the next eligible frame.
5. Presentation and expiry enforcement remain active during degradation and MUST
   not be bypassed by media decode/render load.

## Degradation and Fallback State Contract

The compositor MUST expose deterministic media render states for the bounded
slice:

1. `AwaitingFirstFrame`: publication accepted and present-eligible, but no
   decodable frame available yet.
2. `ActiveFrame`: decodable frame stream is available and being rendered in-zone.
3. `DegradedRender`: media remains admitted under degradation levels 2-3; runtime
   MAY apply bounded quality reduction (e.g., downscale or frame drop) while
   keeping zone confinement and timing invariants.
4. `FallbackPlaceholder`: media content replaced by deterministic placeholder due
   to source stall/transport loss/policy transition.
5. `Teardown`: media render path removed following close/disable/revoke/forced
   degradation teardown.

Rules:

1. At degradation levels 0-1, `ActiveFrame` rendering is allowed if admission
   gates remain satisfied.
2. At degradation levels 2-3, runtime MAY transition `ActiveFrame ->
   DegradedRender` but MUST keep rendering bounded to the approved zone and
   layer attachment contract.
3. At degradation levels 4-5, runtime MUST transition to `Teardown` within one
   compositor frame and deny new media admissions until WM-S3 recovery criteria
   pass.
4. If frame production stalls after previously active rendering, runtime MAY
   briefly hold the last valid frame for continuity but MUST transition to
   `FallbackPlaceholder` before stale content is presented indefinitely.

## Zone Confinement and Layer Attachment Contract

1. `VideoSurfaceRef` rendering MUST occur only in the WM-S2c-approved media zone
   identity/class and MUST be clipped to that zone geometry.
2. Compositor MUST attach the rendered media surface only to the configured
   zone `layer_attachment` for that zone instance (Background, Content, or
   Chrome), with no publisher override.
3. Media rendering MUST NOT bypass reserved zone z-order rules or draw outside
   the runtime-owned zone layer pathway.

## Non-Audio Render Contract

1. The compositor media path for this slice is strictly video-only.
2. Runtime MUST NOT decode, mix, route, or play audio for `VideoSurfaceRef`
   publications.
3. Any transport/input state that attempts to introduce audio-bearing media into
   this bounded slice MUST be rejected or torn down with deterministic policy
   denial behavior; audio presence MUST NOT silently widen scope.
4. Audio handling remains explicitly deferred to post-bounded-slice follow-on
   scope.

## Required Telemetry/State Outputs

Runtime MUST emit machine-usable state transitions for media render lifecycle:

1. state transitions among `AwaitingFirstFrame`, `ActiveFrame`,
   `DegradedRender`, `FallbackPlaceholder`, and `Teardown`,
2. reason-coded transitions (`present-not-yet`, `no-frame-available`,
   `degradation-level-change`, `lease-revoked`, `operator-disabled`,
   `budget-denied`, `audio-policy-violation`, `transport-ended`),
3. frame-visibility timing markers sufficient to verify no-early-present and
   timely teardown behavior in validation.

## Contract Validation Scenarios (Normative)

1. **Runtime-owned texture boundary**
- **WHEN** a `VideoSurfaceRef` publication is accepted
- **THEN** runtime compositor owns texture allocation/update/lifecycle and no
  client-provided GPU handle is consumed directly

2. **No early media present**
- **WHEN** `present_at_wall_us` is in the future
- **THEN** compositor renders no active media frame before that wall-clock point

3. **Fallback before first decoded frame**
- **WHEN** present time is reached but no decodable frame exists yet
- **THEN** compositor renders deterministic fallback state in approved zone until
  first valid frame arrives

4. **Expiry clears media visibility**
- **WHEN** `expires_at_wall_us` is reached for active `VideoSurfaceRef`
- **THEN** compositor stops media rendering at/after expiry and transitions out
  of `ActiveFrame`

5. **Level-3 degradation keeps bounded render with quality reduction**
- **WHEN** degradation level is 3 with an admitted media stream
- **THEN** runtime MAY reduce media quality but MUST keep zone/layer/timing
  invariants intact

6. **Level-4 degradation forces teardown**
- **WHEN** degradation level advances to 4 with active media rendering
- **THEN** compositor transitions to `Teardown` within one compositor frame and
  no active media frame remains visible

7. **Audio path is denied**
- **WHEN** audio-bearing media is attempted in `VideoSurfaceRef` render path
- **THEN** runtime rejects/tears down the media path with deterministic policy
  denial and no audio playback occurs

8. **Zone/layer confinement is enforced**
- **WHEN** a publication tries to render outside approved media zone identity or
  layer attachment
- **THEN** runtime rejects rendering and preserves existing approved-zone state

## Acceptance Traceability (`hud-nn9d.12`)

1. Texture ownership is explicit:
- fulfilled by runtime-owned surface realization and lifecycle rules.
2. Present-time semantics are explicit:
- fulfilled by no-early-present, expiry cut-off, and fallback-on-missing-frame
  rules.
3. Degradation/fallback states are explicit:
- fulfilled by named state model and level-coupled transition rules.
4. Non-audio behavior is explicit:
- fulfilled by strict video-only render contract and audio denial semantics.
