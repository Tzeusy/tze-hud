## ADDED Requirements

### Requirement: Transport-Agnostic Stream Boundary

The text stream portal capability SHALL be defined in terms of generic output text streams, bounded input submission, session identity, and session status metadata. The runtime-facing contract MUST NOT depend on tmux-specific, PTY-specific, terminal-emulator-specific, or chat-provider-specific semantics.

#### Scenario: tmux adapter satisfies contract

- **WHEN** a tmux adapter publishes incremental text output and accepts bounded input submission
- **THEN** it SHALL satisfy the portal capability without the runtime needing tmux window IDs, pane IDs, or PTY control semantics

#### Scenario: non-tmux adapter satisfies contract

- **WHEN** a chat-platform adapter or LLM-session adapter publishes the same generic output/input/session signals
- **THEN** it SHALL be usable through the same portal capability without changing the runtime contract

### Requirement: Content-Layer Portal Surface

Text stream portals SHALL render as content-layer surfaces in the pilot phase. Portal affordances and transcript state MUST NOT live in the chrome layer.

#### Scenario: portal renders below chrome

- **WHEN** a portal tile is visible and the chrome layer is also visible
- **THEN** the portal tile SHALL render below chrome like any other content-layer surface

#### Scenario: portal affordance not addressable as chrome

- **WHEN** an agent queries scene topology or receives shell-state information
- **THEN** no portal affordance SHALL appear as a chrome element or shell-owned status control

### Requirement: Phase-0 Raw-Tile Pilot

The first implementation of text stream portals SHALL be expressible as a resident raw-tile pilot using existing node types and existing resident-session contracts. The phase-0 pilot MUST NOT require a terminal-emulator node, transcript-specific node, or tmux-aware runtime subsystem.

#### Scenario: pilot uses existing node types

- **WHEN** the pilot portal is rendered
- **THEN** it SHALL be constructible from existing text, solid-color, image, and hit-region primitives rather than a new dedicated node type

#### Scenario: pilot uses resident session flow

- **WHEN** the pilot portal is active
- **THEN** the owning authority SHALL be a resident authenticated actor using the existing session and lease model

### Requirement: Low-Latency Text Interaction

Text stream portals SHALL support low-latency incremental output, bounded viewer input submission, and local-first interaction feedback. Output transcript updates SHALL behave as state-stream traffic. Viewer submit and explicit control actions SHALL behave as transactional traffic. Activity indicators and similar transient state SHALL behave as ephemeral realtime traffic.

#### Scenario: output updates stream incrementally

- **WHEN** new text output arrives in multiple increments
- **THEN** the portal SHALL present those increments as an ordered streamed interaction rather than only as a final snapshot

#### Scenario: viewer submit is transactional

- **WHEN** the viewer submits a reply or activates an interrupt-style control
- **THEN** that action SHALL be treated as transactional input rather than coalescible state-stream traffic

### Requirement: Transcript Interaction Contract

Expanded text stream portals SHALL support scrollable transcript viewing, focusable interaction affordances, and bounded reply submission under the existing local-feedback contract. Portal interaction MUST reuse runtime-owned focus, command-input, and local visual acknowledgement rules.

#### Scenario: transcript scroll is local-first

- **WHEN** the viewer scrolls an expanded transcript surface
- **THEN** the visible scroll offset SHALL update locally before any adapter acknowledgement is required

#### Scenario: portal control uses local feedback

- **WHEN** the viewer activates an expand, collapse, or reply affordance
- **THEN** the runtime SHALL provide local-first interaction feedback under the same latency contract as other interactive surfaces

### Requirement: Governance, Privacy, and Override Compliance

Text stream portals SHALL obey the same lease, privacy, redaction, dismissal, freeze, and safe-mode rules as any other governed surface. Portal content MUST NOT bypass viewer-class filtering or shell overrides because it is text.

#### Scenario: portal content redacts under viewer policy

- **WHEN** portal content exceeds the current viewer's permitted classification
- **THEN** the portal SHALL be redacted under the runtime's existing privacy policy rather than exposing the transcript content

#### Scenario: portal suspends under safe mode

- **WHEN** the runtime enters safe mode while a portal is active
- **THEN** portal updates SHALL suspend under the same shell and lease rules as other content-layer surfaces

### Requirement: External Adapter Isolation

Any adapter that emits portal output, accepts viewer input, or requests portal visibility SHALL remain external to the runtime core and SHALL pass through existing authentication and capability boundaries. The runtime core MUST NOT gain implicit authority over external process or transport lifecycles as a side effect of supporting text stream portals.

#### Scenario: local adapter still authenticates

- **WHEN** a local adapter process connects to drive a text stream portal
- **THEN** it SHALL authenticate and operate under explicit capability grants rather than implicit local trust

#### Scenario: runtime does not become process host

- **WHEN** a tmux-backed portal is active
- **THEN** the runtime SHALL remain unaware of tmux-specific lifecycle management beyond the generic stream signals it receives
