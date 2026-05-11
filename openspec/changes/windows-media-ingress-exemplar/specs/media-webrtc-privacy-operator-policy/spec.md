## MODIFIED Requirements

### Requirement: Explicit Enablement Policy

Media ingress MUST remain disabled by default. For the Windows media exemplar, the runtime MUST accept media ingress only when an explicit Windows media enablement state is present, machine-readable, auditable, and approved for the deployment. Missing, false, or unapproved enablement MUST deny admission before transport or decode work begins.

Source: `about/heart-and-soul/privacy.md`, `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: default-off startup remains disabled

- **WHEN** the runtime starts without explicit Windows media-ingress enablement
- **THEN** media ingress MUST remain disabled
- **AND** no media decode worker or WebRTC transport worker MUST be spawned

### Requirement: Human Operator Overrides

Human operator actions MUST take precedence over publisher intent. Media disable, safe mode, dismiss, and lease revocation MUST suppress active media ingress within one compositor frame and MUST deny new admissions until explicit re-enable or fresh admission is allowed by policy.

Source: `about/heart-and-soul/attention.md`, `about/heart-and-soul/privacy.md`, `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: operator disable stops active ingress

- **WHEN** an operator disables media ingress while the YouTube exemplar stream is active
- **THEN** presentation MUST cease within one compositor frame
- **AND** the stream MUST NOT auto-resume after re-enable

### Requirement: Media Privacy Classification and Viewer Ceiling

Every media admission MUST carry content classification metadata. Missing classification MUST fail closed as a policy denial. The runtime MUST apply the more restrictive of the media zone default, publisher classification, and current viewer/privacy ceiling before presenting the video surface. Unknown viewer context MUST fail closed or present only a neutral placeholder.

Source: `about/heart-and-soul/privacy.md`, `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: missing classification fails closed

- **WHEN** a producer requests media ingress without content classification
- **THEN** admission MUST be denied with a structured policy reason

#### Scenario: unknown viewer context suppresses media

- **WHEN** a media stream is active
- **AND** the viewer/privacy context becomes unknown
- **THEN** media presentation MUST stop or be replaced with a neutral placeholder within one compositor frame
- **AND** the stream MUST NOT resume until policy permits presentation again

### Requirement: Media Attention Governance

Media admission MUST carry an interruption class and MUST default to a quiet, non-focus-taking presentation. Quiet hours, attention budgets, and operator suppression MUST apply before presentation. Media MUST NOT expand, focus, or raise urgency unless the operator has explicitly allowed that behavior.

Source: `about/heart-and-soul/attention.md`, `about/heart-and-soul/privacy.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: quiet-hours policy suppresses attention escalation

- **WHEN** a media admission is requested during quiet hours
- **AND** the request does not carry an operator-approved exception
- **THEN** the runtime MUST keep presentation quiet or deny admission according to policy
- **AND** it MUST NOT focus, expand, or otherwise escalate the media surface

### Requirement: Media Policy Audit and Precedence

The runtime MUST emit audit/telemetry records for media admission, denial, operator disable, privacy suppression, budget teardown, and lease revocation. Admission policy MUST short-circuit before decode/transport work starts when enablement, capability, privacy, operator, stream-count, or budget gates fail.

Source: `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: denied admission produces audit without worker spawn

- **WHEN** a media request fails an admission gate
- **THEN** the runtime MUST emit a structured denial/audit reason
- **AND** it MUST NOT spawn media decode or transport workers for that request
