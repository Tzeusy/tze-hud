## ADDED Requirements

### Requirement: Windows Media Validation Lanes

The Windows media ingress exemplar SHALL be validated through three lanes: a deterministic synthetic media lane, a live Windows HUD media-ingress lane, and a YouTube source-evidence/bridge lane. The synthetic lane MUST be runnable without YouTube, WebRTC network peers, or live browser capture. The live Windows HUD lane MUST exercise a self-owned/local video source feeding the HUD media ingress path. The YouTube lane MUST launch video ID `O0FGCxkHM-U` through an official embed/player path, record the 2026-05-12 policy approval for Windows-only video-frame bridging, and record whether the chosen bridge actually entered the HUD through `MediaIngressOpen`.

Source: `openspec/specs/validation-framework/spec.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`, `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

**Archive carve-outs (Lane B / Lane C partially OUTSTANDING):** Lane A (synthetic) is fully satisfied and machine-verified. Lane B (live Windows) has proven authenticated admission, operator-disable, and a 10-min record-only soak, but its "self-owned/local video source rendered in the approved HUD media zone" + "first-frame time" clauses are NOT met live (`first_frame_time_ms=null`, `nonzero_frame_sample_count=0`) — the live decode → compositor path is deliberately deferred (gen-1 synthetic path, `design.md` §5); live rendered-frame proof is carve-out B. Lane C (YouTube) has only official-embed source evidence; the raw-frame bridge into `MediaIngressOpen` is NOT implemented and is owner-policy-gated (`RAW_YOUTUBE_BRIDGE_DECISION="blocked_pending_policy_approval"`; beads `hud-o33hj`/`hud-d82p7`/`hud-s0pit`) — carve-out A. Both are tracked follow-ons, not satisfied requirements (reconciliation `docs/reports/windows-media-ingress-gen1-reconciliation-20260711.md`).

#### Scenario: synthetic lane gates correctness

- **WHEN** the synthetic media validation lane runs
- **THEN** it MUST prove admission, frame presentation, placeholder-before-first-frame behavior, clipping, teardown, second-stream rejection, policy-denial behavior, lease revocation, reconnect gating, and disabled-gate responses with machine-verifiable outcomes

#### Scenario: live Windows lane proves HUD media ingress

- **WHEN** the live Windows media validation lane runs against the target Windows machine
- **THEN** it MUST show a self-owned/local video source rendered in the approved HUD media zone
- **AND** it MUST record media state, first-frame time, frame timing, dropped frames, CPU/GPU/memory, operator-disable behavior, and teardown behavior

#### Scenario: YouTube source lane records official-player evidence

- **WHEN** the YouTube source-evidence lane runs against the target Windows machine
- **THEN** it MUST launch video ID `O0FGCxkHM-U` through an official embed/player path
- **AND** it MUST record the chosen bridge path, player/control visibility, and whether bridged video frames reached the HUD through `MediaIngressOpen`
- **AND** it MUST NOT count as HUD frame-ingress proof if the player surface is bypassed, audio is routed, content is downloaded/extracted/cached, or the compositor hosts a browser surface

#### Scenario: validation artifacts are actionable

- **WHEN** validation completes
- **THEN** it MUST write a report under `docs/reports/`
- **AND** the report MUST include command lines, runtime config path, source identity, media zone identity, pass/fail status for functional gates, and record-only performance metrics
