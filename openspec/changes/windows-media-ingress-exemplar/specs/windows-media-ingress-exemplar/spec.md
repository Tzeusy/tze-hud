## ADDED Requirements

### Requirement: Windows Media Ingress Exemplar Scope

The runtime SHALL support a Windows-only media ingress exemplar that presents exactly one inbound video-only stream in a runtime-owned HUD media zone. The exemplar SHALL target the native Windows runtime and SHALL NOT admit mobile, glasses, multi-device, embodied-presence, audio routing, recording, cloud relay, or bidirectional AV semantics.

Source: `about/heart-and-soul/v1.md`, `about/heart-and-soul/architecture.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: Windows-only media exemplar is admitted

- **WHEN** the Windows runtime starts with explicit media-ingress enablement and a configured approved media zone
- **THEN** one inbound video-only stream MAY be admitted through the media ingress contract
- **AND** the resulting presentation MUST render in the approved content-layer media zone

#### Scenario: non-Windows deployment lanes remain inactive

- **WHEN** Linux, macOS, mobile, or glasses deployment work attempts to use this exemplar as authority
- **THEN** the work MUST be rejected or deferred unless a separate OpenSpec change reopens that platform lane

### Requirement: YouTube Source Evidence Boundary

The YouTube exemplar SHALL use a local sidecar/source-evidence runner that relies on a supported YouTube embed/player path for video ID `O0FGCxkHM-U`. The HUD runtime SHALL NOT receive raw YouTube frames or media tracks unless a documented policy review approves the chosen bridge. The runtime SHALL NOT download, rip, extract, or directly host YouTube media content. The compositor SHALL NOT become a browser surface host for this exemplar.

Source: `about/heart-and-soul/architecture.md`, YouTube IFrame Player API, YouTube API Services Developer Policies
Scope: Active only for source-evidence and policy-gated bridge decisions in the accepted Windows-only media slice.

#### Scenario: exemplar uses supported player source

- **WHEN** the exemplar is launched for `https://www.youtube.com/watch?v=O0FGCxkHM-U`
- **THEN** the producer MUST use the video ID `O0FGCxkHM-U` through a supported embed/player source path
- **AND** the HUD runtime MUST NOT see raw YouTube frames or media tracks unless the policy review allows that bridge

#### Scenario: download or browser-shell path is rejected

- **WHEN** an implementation proposes `yt-dlp`, direct media URL extraction, file download, or arbitrary browser-node rendering inside the compositor
- **THEN** reviewers MUST reject that implementation as outside this change

#### Scenario: HUD proof uses self-owned or local source

- **WHEN** validation requires machine-verifiable HUD frame-ingress proof
- **THEN** the producer MUST use a self-owned, local, or synthetic video source unless a policy-approved YouTube bridge exists
- **AND** the report MUST distinguish HUD media-ingress proof from YouTube source-evidence proof

### Requirement: Exemplar Demonstrates Operator Control

The Windows media exemplar SHALL demonstrate that operator controls override media presentation. Dismiss, media disable, lease revocation, and safe mode SHALL remove or suppress media presentation within one compositor frame and SHALL NOT auto-resume the prior stream after re-enable.

Source: `about/heart-and-soul/attention.md`, `about/heart-and-soul/privacy.md`, `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: operator disables active media

- **WHEN** the media exemplar is actively rendering in the media zone
- **AND** the operator disables media or enters safe mode
- **THEN** media presentation MUST stop within one compositor frame
- **AND** the stream MUST require a fresh admission flow before presenting again
