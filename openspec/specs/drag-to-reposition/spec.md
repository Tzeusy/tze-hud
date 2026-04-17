# drag-to-reposition Specification

## Purpose
TBD - synced from persistent-movable-elements change [hud-mu38].

## Requirements

### Requirement: V1-Compatible Drag Visual Feedback

The drag interaction MUST use v1-compatible visual feedback only: temporary z-order boost, a 2px highlight border, and immediate opacity state changes. The implementation MUST NOT require drop shadows, scale pulses, or animated ease transitions.
Scope: v1-mandatory

#### Scenario: Drag activation applies v1-compatible feedback

- **WHEN** an element enters active drag state
- **THEN** the compositor MUST apply temporary z-order boost and a 2px highlight border
- **AND** opacity state changes MUST apply immediately (no eased transition)

#### Scenario: Deferred visual effects are disallowed

- **WHEN** drag feedback is rendered
- **THEN** the renderer MUST NOT depend on drop-shadow, scale-pulse, or animated settle effects

#### Scenario: Long-press progress is state-derived

- **WHEN** a drag handle is held during long-press activation
- **THEN** progress MUST be represented as a state-derived value from elapsed hold duration (0.0→1.0)
- **AND** this progress signal MUST NOT require a deferred animation system

### Requirement: Reset Gesture Is Conflict-Free On Touch/Mobile Input

Reset-to-default MUST use a gesture that does not conflict with long-press drag activation. A short tap on the drag handle MUST reveal a reset affordance that auto-dismisses after 3 seconds if unused.
Scope: v1-mandatory

#### Scenario: Tap reveals reset affordance

- **WHEN** the user performs a short tap on a drag handle
- **THEN** the runtime MUST reveal a reset affordance associated with that element
- **AND** the affordance MUST auto-dismiss after 3 seconds if no action is taken

#### Scenario: Long-press remains dedicated to drag activation

- **WHEN** the user performs long-press on the drag handle
- **THEN** the interaction MUST enter drag-activation flow
- **AND** reset MUST NOT be triggered by the same long-press sequence
