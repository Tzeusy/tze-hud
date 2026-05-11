## Why

The user-visible goal is to make the Windows HUD capable of showing a live video source as a governed HUD element, with a concrete exemplar shaped around YouTube video ID `O0FGCxkHM-U`. This is aligned with tze_hud's long-term media-plane architecture, but the current active doctrine explicitly defers all WebRTC/GStreamer/media behavior and requires a fresh proposal before any reactivation.

This change reopens only the smallest credible slice: one Windows-only, video-only inbound media stream into a runtime-owned HUD media zone. It does not revive the parked multi-device, mobile, glasses, bidirectional AV, audio, recording, cloud relay, agent-to-agent media, or embodied-presence program.

Implementation MUST NOT begin until either `windows-first-performant-runtime` satisfies its release/performance bar or this change records an explicit maintainer-approved exception for this one-stream Windows-only exemplar.

## What Changes

- Reactivate bounded media ingress for the native Windows runtime only.
- Add a canonical `media_ingress` capability and explicit Windows media configuration for a single approved media zone.
- Modify the existing deferred media contracts so their bounded one-way visual ingress requirements become active under this change's explicit Windows-only enablement gate.
- Modify scene/protocol/runtime/validation requirements enough to make `VideoSurfaceRef` render real decoded frames rather than only a placeholder.
- Treat the YouTube exemplar as an official-player/operator-visible source demonstration unless a documented policy review approves a raw-frame bridge. Machine-verifiable HUD frame-ingress acceptance uses a self-owned, local, or synthetic video source.
- Require any YouTube path to use the supported IFrame/embed player surface; the runtime must not download/rip/extract YouTube content and must not become a browser shell.
- Keep media disabled by default unless explicit config, capability, privacy, operator, and budget gates pass.
- Keep audio rejected in this tranche, even if the external source contains an audio track.

## Capabilities

### Modified Capabilities

- `configuration`: Add default-off Windows media ingress enablement, approved media zone identity, stream limit, operator disable state, and `media_ingress` capability configuration.
- `media-webrtc-bounded-ingress`: Reactivate the bounded ingress envelope for this Windows-only slice.
- `media-webrtc-privacy-operator-policy`: Reactivate explicit enablement, privacy, operator disable, and audit requirements for this Windows-only slice.
- `scene-graph`: Promote `VideoSurfaceRef` from unsupported/deferred to supported only inside the approved media zone when media ingress is enabled.
- `session-protocol`: Make existing `MediaIngress*` and `VideoSurfaceRef` schema active for the Windows media ingress slice.
- `runtime-kernel`: Allow media worker/decode pipeline activation only behind the explicit Windows media gate.
- `validation-framework`: Add synthetic and live Windows media validation lanes.

## Impact

- Affected code:
  - `crates/tze_hud_protocol/proto/session.proto`
  - `crates/tze_hud_protocol/proto/types.proto`
  - `crates/tze_hud_protocol/src/session_server.rs`
  - `crates/tze_hud_scene/src/types.rs`
  - `crates/tze_hud_scene/src/graph.rs`
  - `crates/tze_hud_runtime/src/media_ingress.rs`
  - `crates/tze_hud_runtime/src/media_admission.rs`
  - `crates/tze_hud_runtime/src/gst_decode_pipeline.rs`
  - `crates/tze_hud_compositor/src/video_surface.rs`
  - `crates/tze_hud_compositor/src/renderer.rs`
  - `app/tze_hud_app/config/*.toml`
  - `.claude/skills/user-test/scripts/` or `examples/` for the Windows media exemplar
  - `about/heart-and-soul/v1.md` and `about/heart-and-soul/media-doctrine.md` narrow-exception notes after acceptance
- Affected systems:
  - Windows runtime config and deployment
  - gRPC resident session protocol
  - compositor texture upload/render path
  - privacy/operator override policy
  - headless and live Windows validation
- New dependency risk:
  - Windows GStreamer/WebRTC/WebView2 producer bootstrap must be documented and validated.
  - YouTube playback must use an official embed/player path; no download or extraction tooling is admitted by this change. Bridging YouTube player frames into the HUD is blocked until a policy review records that the chosen approach complies with YouTube terms.
