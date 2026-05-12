# Windows Media Ingress Exemplar Design

## 1. Scope Decision

The reactivation is deliberately narrower than the previously deferred V2 media program. This change admits one Windows-only, video-only inbound media stream into one approved runtime-owned media zone.

This is a deliberate narrow exception to the current media deferral, not a general media-plane revival. Delivery waits on the Windows-first performance prerequisite unless maintainers explicitly approve this one-stream exception.

In scope:

- native Windows HUD runtime only
- one active inbound stream at a time
- video frames only
- runtime-owned `VideoSurfaceRef` surface
- approved configured media zone
- explicit enablement/capability/privacy/operator/budget admission
- synthetic source validation
- live Windows HUD exemplar using a self-owned/local video source
- YouTube video ID `O0FGCxkHM-U` shown through an official embed/player as operator-visible source evidence

Out of scope:

- audio routing or playback
- bidirectional AV calls
- recording
- cloud relay/SFU
- multi-feed layouts
- mobile, glasses, or cross-device media
- embodied presence
- browser surface nodes
- YouTube download/ripping/extraction

## 2. Architecture

The runtime remains sovereign. Agents or producers request media ingress; they do not own compositor resources. The media path is:

1. An authenticated resident/local producer session with `media_ingress` capability issues `MediaIngressOpen` for the approved media zone.
2. Admission checks explicit media enablement, operator override state, privacy classification, capabilities, stream count, codec/support, schedule, and resource budget.
3. The runtime assigns a `surface_id`, publishes `ZoneContent::VideoSurfaceRef(surface_id)` into the approved zone, and starts the decode/transport path.
4. The decode path produces `VideoFrame` values.
5. The compositor uploads the latest frame to a runtime-owned `wgpu::Texture` and renders it clipped to the zone geometry.
6. Operator disable, safe mode, lease revoke, expiry, or budget breach tears down presentation within one compositor frame.

The first implementation uses synthetic frames and a self-owned/local video source to keep the media path independently verifiable. A YouTube sidecar may load the official embeddable/player path for operator-visible evidence. As of 2026-05-12, operator/maintainer policy approval allows a Windows-only bridge from that official player sidecar into the HUD media ingress path, provided the bridge is video-only, keeps the player/control model operator-visible, and does not download, rip, extract, cache, or directly host YouTube media content.

## 3. Zone Contract

The approved zone is `media-pip` for this change. The zone contract:

- `accepted_media_types` includes only `VideoSurfaceRef` for the media path.
- `transport_constraint = WebRtcRequired` or the nearest existing equivalent.
- `contention_policy = Replace`.
- `max_publishers = 1`.
- `layer_attachment = Content`.
- Geometry is configured by runtime config, not by publisher payload.
- Widgets and existing zones are not repurposed for live media.
- Existing default zones such as `pip` and `ambient-background` MUST NOT accept `VideoSurfaceRef` unless they are explicitly configured as the approved media zone by a later change.

## 4. YouTube Exemplar

The exemplar URL is `https://www.youtube.com/watch?v=O0FGCxkHM-U`. The implementation should use video ID `O0FGCxkHM-U` with the supported embedded player path, for example a WebView2 or browser-hosted producer using `https://www.youtube.com/embed/O0FGCxkHM-U` and the YouTube IFrame/player API where control is needed.

The YouTube player is not part of the compositor. It may be an example application, test script, or local sidecar that proves the official-player source can be launched and controlled on the Windows machine. It must not:

- download the video file,
- use `yt-dlp`/download extractors,
- bypass the YouTube player surface,
- route audio into the HUD runtime in this tranche,
- feed raw frames or media tracks into the HUD except through the approved Windows-only video-frame bridge.

The baseline HUD frame-ingress proof uses a self-owned/local test source. The approved YouTube bridge is a follow-on validation lane and must record the exact bridge path, player/control model, and evidence that only video frames enter the HUD through `MediaIngressOpen`.

## 5. Validation Strategy

Validation runs in two lanes:

- Lane A: deterministic synthetic frames in headless tests, no live network or YouTube dependency.
- Lane B: live Windows user-test with a self-owned/local video source feeding the HUD media ingress path on the target Windows machine.
- Lane C: operator-visible YouTube source evidence that launches video ID `O0FGCxkHM-U` through the official embed/player on the target Windows machine.

Lane A gates implementation correctness. Lane B provides release evidence for HUD media ingress. Lane C provides YouTube exemplar evidence. With the 2026-05-12 approval, a Windows-only Lane C bridge may also count as HUD frame-ingress proof if it uses the official player sidecar, enters the HUD only through `MediaIngressOpen`, remains video-only, and preserves the operator-visible player/control model.

## 6. Risks

- YouTube embed/player capture may be constrained by DRM, browser, platform capture policy, or YouTube terms. The approved bridge must fail closed and preserve the local/synthetic HUD lane as the independent fallback proof.
- GStreamer Windows bootstrap may require MSVC, plugin, or PATH work that is not currently captured by CI.
- CPU readback from a WebView/player can be too expensive. The first acceptance bar is correctness; the later Windows soak decides whether optimization is required.
- Audio must remain rejected despite YouTube being an audiovisual source.
