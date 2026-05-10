# text-stream-portals Specification

## Purpose
Defines transport-agnostic text stream portal behavior for resident raw-tile pilots, including output rendering, bounded input, session metadata, and cooperative projection integration points.

## Requirements
### Requirement: Transport-Agnostic Stream Boundary

The text stream portal capability SHALL be defined in terms of generic output text streams, bounded input submission, session identity, and session status metadata. The runtime-facing contract MUST NOT depend on tmux-specific, PTY-specific, terminal-emulator-specific, or chat-provider-specific semantics. Resident portal traffic MUST continue to ride the existing primary bidirectional session stream rather than introducing a second long-lived portal stream per agent. If portal payloads carry ordering, expiry, unread-window, or latency metadata, those fields MUST follow the existing clock-domain contract: wall-clock fields end in `_wall_us`, monotonic fields end in `_mono_us`, and arrival timestamps remain advisory rather than presentation-authoritative.

#### Scenario: tmux adapter satisfies contract

- **WHEN** a tmux adapter publishes incremental text output and accepts bounded input submission
- **THEN** it SHALL satisfy the portal capability without the runtime needing tmux window IDs, pane IDs, or PTY control semantics

#### Scenario: non-tmux adapter satisfies contract

- **WHEN** a chat-platform adapter or LLM-session adapter publishes the same generic output/input/session signals
- **THEN** it SHALL be usable through the same portal capability without changing the runtime contract

#### Scenario: resident portal does not create a second stream

- **WHEN** a resident adapter drives a text stream portal
- **THEN** all portal-related publication, control, and lease traffic SHALL remain on the existing primary session stream

#### Scenario: portal timing metadata uses typed clock domains

- **WHEN** a portal adapter supplies expiry, unread-window, scheduling, or latency metadata
- **THEN** wall-clock semantics SHALL use `_wall_us`, monotonic semantics SHALL use `_mono_us`, and those fields SHALL NOT override runtime presentation control

### Requirement: Content-Layer Portal Surface

Text stream portals SHALL render as content-layer surfaces in the pilot phase. Portal affordances and transcript state MUST NOT live in the chrome layer.

#### Scenario: portal renders below chrome

- **WHEN** a portal tile is visible and the chrome layer is also visible
- **THEN** the portal tile SHALL render below chrome like any other content-layer surface

#### Scenario: portal affordance not addressable as chrome

- **WHEN** an agent queries scene topology or receives shell-state information
- **THEN** no portal affordance SHALL appear as a chrome element or shell-owned status control

### Requirement: Phase-0 Raw-Tile Pilot

The first implementation of text stream portals SHALL be expressible as a resident raw-tile pilot using existing node types and existing resident-session contracts. The phase-0 pilot MUST NOT require a terminal-emulator node, transcript-specific node, tmux-aware runtime subsystem, or portal-specific transport RPC.

#### Scenario: pilot uses existing node types

- **WHEN** the pilot portal is rendered
- **THEN** it SHALL be constructible from existing text, solid-color, image, and hit-region primitives rather than a new dedicated node type

#### Scenario: adapter-side ANSI color with inline runs

- **WHEN** a portal adapter receives ANSI-colorized terminal output (e.g. `\e[31mERROR\e[0m: disk full`)
- **THEN** the adapter SHALL parse ANSI escape sequences and map each color run to a `TextColorRun` entry on the `TextMarkdownNode`
- **AND** the adapter SHALL strip ANSI escape sequences from the `content` string and populate `color_runs` with byte offsets into the stripped content
- **THEN** the runtime SHALL render the colored spans without a terminal-emulator node
- **AND** terminal emulation (PTY semantics, cursor positioning, scrollback) remains out of scope per Phase 0 boundary

Note: `TextMarkdownNode.color_runs` (RFC 0001 §TextMarkdownNode, scene-graph/spec.md §TextMarkdownNode inline color_runs) enables this adapter pattern. No new node type is required.

#### Scenario: pilot uses resident session flow

- **WHEN** the pilot portal is active
- **THEN** the owning authority SHALL be a resident authenticated actor using the existing session and lease model

### Requirement: Bounded Transcript Viewport

Phase-0 portal transcript rendering SHALL be a bounded viewport rather than an unbounded transcript store. Any transcript text materialized into scene nodes MUST remain within existing node-size and per-tile resource budgets. Full retained history, if any, SHALL live outside the scene graph.

#### Scenario: visible transcript fits node budget

- **WHEN** a portal renders an expanded transcript window
- **THEN** the text content placed into scene nodes SHALL stay within the existing TextMarkdown node size limits and tile resource budgets

#### Scenario: full history not mirrored into scene nodes

- **WHEN** a portal session has transcript history larger than the currently visible window
- **THEN** the runtime SHALL represent only the bounded visible or immediately scrollable window in scene nodes rather than mirroring the full retained history

### Requirement: Low-Latency Text Interaction

Text stream portals SHALL support low-latency incremental output, bounded viewer input submission, and local-first interaction feedback. Output transcript updates SHALL behave as state-stream traffic. Viewer submit and explicit control actions SHALL behave as transactional traffic. Activity indicators and similar transient state SHALL behave as ephemeral realtime traffic.

#### Scenario: output updates stream incrementally

- **WHEN** new text output arrives in multiple increments
- **THEN** the portal SHALL present those increments as an ordered streamed interaction rather than only as a final snapshot

#### Scenario: viewer submit is transactional

- **WHEN** the viewer submits a reply or activates an interrupt-style control
- **THEN** that action SHALL be treated as transactional input rather than coalescible state-stream traffic

### Requirement: Transcript Interaction Contract

Expanded text stream portals SHALL support scrollable transcript viewing, focusable interaction affordances, and bounded reply submission under the existing local-feedback contract. Portal interaction MUST reuse runtime-owned focus, command-input, and local visual acknowledgement rules. User scroll input MUST remain authoritative over any adapter-driven attempt to reposition the transcript viewport.

#### Scenario: transcript scroll is local-first

- **WHEN** the viewer scrolls an expanded transcript surface
- **THEN** the visible scroll offset SHALL update locally before any adapter acknowledgement is required

#### Scenario: portal control uses local feedback

- **WHEN** the viewer activates an expand, collapse, or reply affordance
- **THEN** the runtime SHALL provide local-first interaction feedback under the same latency contract as other interactive surfaces

### Requirement: Coherent Transcript Coalescing

State-stream coalescing for portal output MUST preserve a coherent retained transcript window. Intermediate render states MAY be skipped, but already-committed logical transcript units within the retained on-screen history window MUST NOT be lost merely because updates were coalesced.

#### Scenario: coalescing preserves retained window

- **WHEN** several portal append operations are coalesced under backpressure
- **THEN** the runtime MAY render a newer complete transcript window snapshot but SHALL NOT collapse the retained window into only the latest line

### Requirement: Governance, Privacy, and Override Compliance

Text stream portals SHALL obey the same lease, privacy, redaction, dismissal, freeze, and safe-mode rules as any other governed surface. Portal identity, transcript content, and activity metadata MUST NOT bypass viewer-class filtering or shell overrides because they are text. Collapsed portal cards MUST NOT be treated as automatically safe metadata.

#### Scenario: portal content redacts under viewer policy

- **WHEN** portal content exceeds the current viewer's permitted classification
- **THEN** the portal SHALL be redacted under the runtime's existing privacy policy rather than exposing the transcript content

#### Scenario: portal suspends under safe mode

- **WHEN** the runtime enters safe mode while a portal is active
- **THEN** portal updates SHALL suspend under the same shell and lease rules as other content-layer surfaces

#### Scenario: collapsed portal preserves geometry while redacted

- **WHEN** the current viewer is not permitted to see a portal's identity or transcript state
- **THEN** the portal SHALL preserve its geometry, suppress transcript previews and activity details, and replace visible content with the runtime's neutral redaction treatment

#### Scenario: disconnected portal follows orphan path

- **WHEN** the owning resident portal session disconnects unexpectedly
- **THEN** the lease SHALL transition through the normal orphan lifecycle, the visible portal SHALL freeze at its last coherent state or runtime placeholder policy, and grace expiry SHALL remove the governed surface under the existing lease rules

#### Scenario: freeze does not disclose viewer intent

- **WHEN** a portal is active while the runtime freezes visible scene mutation
- **THEN** adapters SHALL observe only the existing generic queue-pressure or dropped-mutation semantics rather than a portal-specific freeze signal

### Requirement: Ambient Portal Attention Defaults

Text stream portal activity indicators, unread state, and transcript churn SHALL default to ambient or gentle presentation. Portal activity MUST NOT self-escalate interruption class merely because a stream is active, a backlog is growing, or new transcript units continue to arrive.

#### Scenario: unread backlog does not auto-upgrade urgency

- **WHEN** a portal accumulates unread transcript updates without explicit higher-priority content policy
- **THEN** the runtime SHALL keep the portal's activity presentation ambient or gentle rather than upgrading it to a stronger interruption class solely because of backlog growth

#### Scenario: typing indicator remains ambient

- **WHEN** a live portal exposes typing or activity indicators
- **THEN** those indicators SHALL remain subordinate to the runtime's existing attention model and SHALL NOT behave like repeated notifications

### Requirement: External Adapter Isolation

Any adapter that emits portal output, accepts viewer input, or requests portal visibility SHALL remain external to the runtime core and SHALL pass through existing authentication and capability boundaries. The runtime core MUST NOT gain implicit authority over external process or transport lifecycles as a side effect of supporting text stream portals.

#### Scenario: local adapter still authenticates

- **WHEN** a local adapter process connects to drive a text stream portal
- **THEN** it SHALL authenticate and operate under explicit capability grants rather than implicit local trust

#### Scenario: runtime does not become process host

- **WHEN** a tmux-backed portal is active
- **THEN** the runtime SHALL remain unaware of tmux-specific lifecycle management beyond the generic stream signals it receives

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
