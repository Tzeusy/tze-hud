## ADDED Requirements

### Requirement: Cooperative LLM Projection Adapter
Text stream portals SHALL support cooperative LLM-session projection as a valid non-tmux adapter family. A cooperative projection adapter SHALL publish generic output stream updates, session identity, lifecycle status, and bounded input state to the portal contract while keeping provider-specific LLM behavior outside the runtime core. Cooperative projection MUST NOT add PTY attachment, terminal-emulator semantics, provider-specific portal RPCs, or runtime ownership of LLM process lifecycles.

#### Scenario: cooperative adapter satisfies portal boundary
- **WHEN** a Codex, Claude, opencode, or similar LLM session opts into HUD projection through an external daemon
- **THEN** the resulting portal SHALL use the existing text-stream portal semantics for output, bounded input, session identity, status metadata, and lifecycle state
- **AND** the runtime-facing behavior SHALL remain independent of the provider's CLI or chat implementation details

#### Scenario: cooperative adapter does not imply process hosting
- **WHEN** a cooperative projection shows an already-running LLM session on the HUD
- **THEN** the runtime SHALL remain unaware of that LLM process lifecycle beyond generic portal session metadata
- **AND** the runtime SHALL NOT acquire authority to start, stop, inspect, or inject input into the provider process

### Requirement: Cooperative Projection Input Mapping
HUD input submitted through a cooperative projected portal SHALL remain bounded viewer input under the existing portal input contract. The portal adapter SHALL map submitted text into its cooperative inbox or equivalent semantic input mechanism, not into raw terminal keystrokes, shell commands, or provider-specific byte streams.

#### Scenario: submitted text maps to semantic inbox
- **WHEN** the operator submits text from an expanded cooperative projected portal
- **THEN** the adapter SHALL treat the submission as a transactional bounded input item for the owning projected session
- **AND** it SHALL expose that input to the active LLM session through the cooperative projection contract

#### Scenario: raw keystroke passthrough remains out of scope
- **WHEN** the operator edits text in the portal composer before submitting
- **THEN** runtime character, focus, and command events MAY support local composition behavior
- **BUT** the adapter SHALL NOT interpret ordinary editing keystrokes as terminal input to the projected LLM process

### Requirement: Cooperative Projection State Externality
Cooperative projection adapters SHALL keep retained transcript history, pending input queues, acknowledgement state, and reconnection metadata outside the scene graph and outside the runtime core. The text-stream portal surface SHALL materialize only the bounded visible transcript window, compact collapsed state, and policy-permitted status metadata.

#### Scenario: full projection state remains external
- **WHEN** a cooperative projection has retained transcript history larger than the visible portal viewport
- **THEN** the adapter or projection daemon SHALL retain that history outside the scene graph
- **AND** the portal SHALL render only the bounded visible or immediately scrollable window

#### Scenario: pending input not mirrored into scene graph
- **WHEN** multiple HUD input submissions are waiting for the LLM session
- **THEN** the portal MAY show a bounded pending count or preview permitted by policy
- **BUT** it SHALL NOT mirror the full pending input queue into scene nodes
