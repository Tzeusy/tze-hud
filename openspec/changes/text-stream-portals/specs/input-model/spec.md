## ADDED Requirements

### Requirement: Text Stream Portal Interaction Reuses Local-First Input

Expanded text stream portals SHALL reuse the existing local-first input model for focus, activation, command input, and scroll. Portal affordances MUST NOT require a slower special-case input path.

#### Scenario: portal reply affordance gets local feedback

- **WHEN** the viewer activates a portal reply affordance
- **THEN** the runtime SHALL provide local visual acknowledgement before any adapter roundtrip completes

#### Scenario: transcript scroll remains local-first

- **WHEN** the viewer scrolls an expanded transcript portal
- **THEN** the transcript viewport SHALL respond through the runtime-owned scroll path rather than adapter-latency-driven repaint
