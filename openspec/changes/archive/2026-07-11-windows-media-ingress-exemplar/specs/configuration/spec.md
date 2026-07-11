## MODIFIED Requirements

<!--
Archive/sync note: this requirement was already synced into
openspec/specs/configuration/spec.md early, by the implementation PR that shipped
the config gate (hud-gog64.1 / PR #653). The canonical text there is the
code-accurate form (cites the exact `[media_ingress]` keys and the
`CONFIG_INVALID_MEDIA_INGRESS` reject code). This delta is therefore expressed as
MODIFIED carrying that same canonical text verbatim, so the archive sync is a
net-zero no-op on the canonical spec rather than a regression to the abstract
planning wording. See docs/reports/windows-media-ingress-gen1-reconciliation-20260711.md
(Requirement 1, SATISFIED).
-->

### Requirement: Windows Media Ingress Configuration
Windows media ingress MUST be disabled by default. The runtime MUST only enable media ingress when `[media_ingress]` explicitly sets `enabled = true`, `approved_zone = "media-pip"`, `max_active_streams = 1`, a non-empty `default_classification`, explicit `operator_disabled` state, and fixed absolute `geometry` (`x`, `y`, `width`, `height`). The approved zone MUST be content-layer, MUST accept only `VideoSurfaceRef`, MUST use `transport_constraint = WebRtcRequired`, and MUST NOT be inferred from existing default zones such as `pip` or `ambient-background`. Authenticated resident/local producers MUST receive the canonical `media_ingress` capability explicitly through `[agents.registered]`.
Source: `openspec/changes/windows-media-ingress-exemplar/specs/configuration/spec.md`
Scope: windows-media-ingress-exemplar only

#### Scenario: media ingress absent remains disabled
- **WHEN** the configuration has no `[media_ingress]` table
- **THEN** media ingress is disabled and no approved media zone is registered

#### Scenario: explicit media-pip configuration accepted
- **WHEN** `[media_ingress]` enables `media-pip` with fixed geometry, `max_active_streams = 1`, default classification, and operator-disabled state
- **THEN** the resolved config records the approved Windows media-ingress surface
- **AND** only agents explicitly granted `media_ingress` can request media admission

#### Scenario: non-approved media zone rejected
- **WHEN** `[media_ingress]` names `pip`, `ambient-background`, or any zone other than `media-pip`
- **THEN** startup fails with `CONFIG_INVALID_MEDIA_INGRESS`
