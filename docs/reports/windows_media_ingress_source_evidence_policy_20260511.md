# Windows Media Ingress Source Evidence Policy Review

Date: 2026-05-11

## Decision

Raw YouTube frame bridging is **blocked pending explicit policy approval** for the first Windows media ingress acceptance pass.

The HUD media-ingress proof uses a self-owned/local synthetic video-only source through the admitted `MediaIngressOpen` path. The YouTube evidence lane launches video ID `O0FGCxkHM-U` through the official embedded player URL:

```text
https://www.youtube.com/embed/O0FGCxkHM-U
```

The two lanes are intentionally separate:

- HUD lane: local/synthetic video-only producer, no audio route, targets `media-pip`.
- YouTube lane: external source-evidence sidecar, official embedded player URL, no compositor browser node.

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

Blocked in this tranche:

- `yt-dlp`, `youtube-dl`, direct media URL extraction, file download, cache, or offline copy.
- Browser node or WebView embedded inside the HUD compositor.
- Raw YouTube frame capture or media-track bridging into the HUD runtime.
- Any YouTube audio route into the HUD runtime.

## Follow-Up Rule

If raw YouTube frame bridging becomes necessary, open a separate policy-review bead before implementation. That review must name the proposed bridge, show why it complies with current YouTube policy and platform restrictions, and define an operator-visible player/control model that does not bypass YouTube's player surface.
