## ADDED Requirements

### Requirement: Governed Media Plane Activation

V2 SHALL treat live media as a governed runtime capability rather than an ad hoc extension. Media activation MUST remain tied to explicit capability, lease, operator-policy, and budget gates.

#### Scenario: bounded ingress activation remains governed

- **WHEN** a publisher requests live media in v2
- **THEN** the runtime admits it only if capability, lease, privacy, operator, and budget gates all succeed

### Requirement: Bidirectional AV Is A Later Phase

Bidirectional AV/session semantics MUST NOT be admitted merely because bounded ingress exists. Any bidirectional media session MUST satisfy explicit audio, operator, failure, and validation contracts beyond the bounded-ingress tranche.

#### Scenario: bounded ingress does not imply two-way AV

- **WHEN** v2 bounded ingress is implemented without bidirectional AV signoff
- **THEN** the runtime continues to reject two-way AV session negotiation

### Requirement: Media Timing Is First-Class

Media publications and cues SHALL carry presentation-time semantics that remain distinct from arrival time. The runtime MUST preserve deterministic timing, expiry, and reconnect behavior for media surfaces and related overlays.

#### Scenario: timed media cue survives governance checks

- **WHEN** a timed media publication or cue is admitted
- **THEN** the runtime schedules it against its declared timing contract rather than on arrival time alone
