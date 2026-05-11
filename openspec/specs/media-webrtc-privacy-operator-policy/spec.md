# Specification: Media/WebRTC Privacy, Operator Controls, and Enablement Policy

> **DEFERRED INDEFINITELY (2026-05-09).** Pairs with `media-webrtc-bounded-ingress` and is deferred for the same reason: the project has refocused on a performant single-device Rust HUD runtime for Windows. The accepted `openspec/changes/windows-media-ingress-exemplar/` change is the only active exception and is limited to default-off Windows-only, one-stream, video-only `media-pip` ingress with explicit operator disable state. Active source of truth: `openspec/changes/windows-first-performant-runtime/`, `openspec/changes/windows-media-ingress-exemplar/` for the exception, and epic `hud-9wljr`.
>
> Original spec follows.

## Purpose

Define the viewer/privacy constraints, human/operator overrides, observability
requirements, and explicit enablement policy for bounded media ingress on a
household-facing display.

This specification is normative for governance and admission policy only. It
does not define signaling shape, schema/snapshot semantics, compositor render
behavior, or validation harness details.

It is the downstream `WM-S3b` contract referenced by
`openspec/specs/media-webrtc-bounded-ingress/spec.md`.

---

## Requirements

### Requirement: Viewer Privacy Ceiling
Media ingress on a household-facing display MUST be treated as visible to
nearby viewers and therefore MUST be governed by explicit viewer/privacy
policy before admission and before presentation. Every ingress publication MUST
carry content classification metadata. The runtime MUST deny admission, or keep
ingress disabled, when the current viewer context is unknown, unavailable, or
does not satisfy the declared privacy ceiling for the target surface.
Scope: post-v1-contract-tranche

#### Scenario: Unknown viewer context fails closed
- **WHEN** a media ingress request arrives and the runtime cannot resolve a
  valid viewer context for the display
- **THEN** the request MUST be denied and no media MUST be presented

#### Scenario: Viewer ceiling is not satisfied
- **WHEN** a media ingress publication carries content classification that
  exceeds the active viewer/privacy ceiling for the target surface
- **THEN** the runtime MUST reject the publication with a structured policy
  denial

---

### Requirement: Human Operator Overrides
Human operator actions MUST take precedence over publisher intent. An operator
disable MUST immediately suppress active media ingress and MUST deny new
admissions until an explicit operator re-enable occurs. Re-enable MUST NOT
silently restore a previously pending or active media stream; any desired media
ingress MUST be re-admitted after the override is lifted.
Scope: post-v1-contract-tranche

#### Scenario: Operator disable stops active ingress
- **WHEN** an operator disables media ingress while a stream is active
- **THEN** presentation MUST cease within one compositor frame and the stream
  MUST be torn down or held inactive

#### Scenario: Operator re-enable does not auto-resume
- **WHEN** an operator re-enables media ingress after a disable
- **THEN** no prior stream MUST be resumed implicitly
- **AND** the publisher MUST issue a fresh admission flow if ingress is still
  desired

---

### Requirement: Explicit Enablement Policy
Media ingress MUST remain disabled by default. The runtime MUST accept media
ingress only when an explicit enablement state is present and approved for the
deployment. The enablement state MUST be machine-readable, auditable, and
checked as part of admission. If the enablement state is missing, false, or
otherwise not approved, the runtime MUST treat media ingress as disabled.
Scope: post-v1-contract-tranche

#### Scenario: Default-off startup remains disabled
- **WHEN** the runtime starts without an explicit media-ingress enablement
  decision
- **THEN** media ingress MUST remain disabled and no admission MUST occur

#### Scenario: Missing enablement approval blocks admission
- **WHEN** media ingress is requested but the deployment has not explicitly
  approved the enablement state
- **THEN** the runtime MUST reject the request as disabled policy, not as a
  transport failure

---

### Requirement: Observability and Auditability
The runtime MUST emit structured observability signals for media-ingress
admission decisions, policy denials, operator enable/disable actions, and
teardown events. Each signal MUST include the affected surface or zone, the
decision outcome, and a machine-readable reason code. The runtime MUST NOT
emit raw media frames, audio, or viewer biometric data in these signals.
Scope: post-v1-contract-tranche

#### Scenario: Admission denial is auditable without payload leakage
- **WHEN** a media ingress admission is denied by privacy or enablement policy
- **THEN** the runtime MUST record a structured event with the denial reason
- **AND** the event MUST NOT contain raw media content or viewer biometric data

#### Scenario: Operator toggle is visible to telemetry
- **WHEN** an operator disables or re-enables media ingress
- **THEN** the runtime MUST emit an operator-action event with the new state and
  affected surface or zone identifier

---

### Requirement: Admission Precedence
Media ingress admission MUST be evaluated in this order: explicit enablement
state, operator override state, viewer/privacy ceiling, then the remaining
bounded-ingress admission checks defined by the dependent specs. If any earlier
check fails, the runtime MUST deny the request deterministically and MUST NOT
attempt later checks that would imply the request is already admissible.
Scope: post-v1-contract-tranche

#### Scenario: Disabled policy short-circuits admission
- **WHEN** the explicit enablement state is disabled
- **THEN** the runtime MUST deny admission before evaluating viewer/privacy
  or transport-specific checks

#### Scenario: Operator disable short-circuits admission
- **WHEN** an operator disable is active
- **THEN** the runtime MUST deny admission before evaluating viewer/privacy
  or transport-specific checks
