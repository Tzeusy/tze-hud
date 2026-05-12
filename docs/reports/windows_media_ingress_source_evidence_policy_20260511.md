# Windows Media Ingress Source Evidence Policy Review

Date: 2026-05-11

## Decision

Raw YouTube frame bridging is **approved for the Windows-only media ingress exemplar**, based on operator/maintainer approval recorded on 2026-05-12.

The approved bridge remains narrow: it may bridge frames from an operator-visible, official YouTube player surface into the HUD media ingress path for video ID `O0FGCxkHM-U`. The bridge must not download, rip, extract, cache, or directly host YouTube media content, and it must not route audio into the HUD runtime.

The YouTube evidence lane launches video ID `O0FGCxkHM-U` through the official embedded player URL:

```text
https://www.youtube.com/embed/O0FGCxkHM-U
```

The approval changes the lane relationship:

- Baseline HUD lane: local/synthetic video-only producer, no audio route, targets `media-pip`.
- Approved YouTube bridge lane: official-player sidecar, operator-visible player/control surface, raw video frames bridged into the HUD runtime only through the media ingress contract.
- YouTube source-evidence lane: official embedded player URL, no compositor browser node.

## Sources Checked

- YouTube Embedded Players and Player Parameters: `https://developers.google.com/youtube/player_parameters`
  - The official embed URL shape is `https://www.youtube.com/embed/VIDEO_ID`.
  - Embedded players require a minimum viewport and should preserve player controls/experience.
- YouTube API Services Developer Policies: `https://developers.google.com/youtube/terms/developer-policies`
  - YouTube audiovisual content must not be downloaded, backed up, cached, or stored without prior written approval.
- YouTube API Services Required Minimum Functionality: `https://developers.google.com/youtube/terms/required-minimum-functionality`
  - Embedded-player clients must provide identity through HTTP Referer where applicable, must not suppress referer with `noreferrer`, and must not obscure player controls with overlays.

## Boundary

Allowed in this tranche:

- Launch `O0FGCxkHM-U` through `https://www.youtube.com/embed/O0FGCxkHM-U`.
- Record sidecar launch evidence separately from HUD frame-ingress evidence.
- Use a self-owned/local synthetic source for the HUD runtime proof.
- Implement a Windows-only raw-frame bridge from the official player sidecar into the approved HUD media ingress path, provided the player remains operator-visible and the bridge is video-only.

Blocked in this tranche:

- `yt-dlp`, `youtube-dl`, direct media URL extraction, file download, cache, or offline copy.
- Browser node or WebView embedded inside the HUD compositor.
- Any YouTube audio route into the HUD runtime.

## Follow-Up Rule

Implementation must name the chosen bridge, keep the YouTube player/control model operator-visible, and record validation artifacts that prove the HUD runtime receives only video frames through `MediaIngressOpen`. If the proposed approach would bypass the official player surface, suppress controls, download/extract content, cache media, or route audio, open a separate policy-review bead before implementation.
