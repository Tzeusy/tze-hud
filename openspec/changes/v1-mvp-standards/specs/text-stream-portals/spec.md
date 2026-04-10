# Text Stream Portals Specification

Source: RFC 0013 (Text Stream Portals)
Domain: Interaction
Depends on: scene-graph, input-model, session-protocol, system-shell, lease-governance, policy-arbitration

---

## ADDED Requirements

### Requirement: Transport-Agnostic Stream Boundary
The text stream portal capability SHALL be defined in terms of generic output text streams, bounded input submission, session identity, and session status metadata. The runtime-facing contract MUST NOT depend on tmux-specific, PTY-specific, terminal-emulator-specific, or chat-provider-specific semantics. Resident portal traffic MUST continue to ride the existing primary bidirectional session stream rather than introducing a second long-lived portal stream per agent.
Source: RFC 0013 §2.1, RFC 0005 §2.5
Scope: v1-mandatory

#### Scenario: tmux adapter satisfies contract
- **WHEN** a tmux adapter publishes incremental text output and accepts bounded input submission
- **THEN** it SHALL satisfy the portal capability without the runtime needing tmux window IDs, pane IDs, or PTY lifecycle semantics

#### Scenario: resident portal does not create a second stream
- **WHEN** a resident adapter drives a text stream portal
- **THEN** all portal-related publication, control, and lease traffic SHALL remain on the existing primary session stream

### Requirement: Content-Layer Portal Surface
Text stream portals SHALL render as content-layer surfaces in the pilot phase. Portal affordances, transcript state, and portal identity MUST NOT live in the chrome layer.
Source: RFC 0013 §3.1, RFC 0007 §1.2
Scope: v1-mandatory

#### Scenario: portal renders below chrome
- **WHEN** a portal tile is visible and chrome is also visible
- **THEN** the portal tile SHALL render below chrome like any other content-layer surface

#### Scenario: portal affordance not addressable as chrome
- **WHEN** an agent queries scene topology or receives shell-state information
- **THEN** no portal affordance SHALL appear as a chrome element or shell-owned status control

### Requirement: Phase-0 Raw-Tile Pilot
The first implementation of text stream portals SHALL be expressible as a resident raw-tile pilot using existing node types and existing resident-session contracts. The phase-0 pilot MUST NOT require a terminal-emulator node, transcript-specific node, tmux-aware runtime subsystem, or portal-specific transport RPC.
Source: RFC 0013 §3.4, §4.3, §7.1
Scope: v1-mandatory

#### Scenario: pilot uses existing node types
- **WHEN** the pilot portal is rendered
- **THEN** it SHALL be constructible from existing text, solid-color, image, and hit-region primitives rather than a new dedicated node type

#### Scenario: pilot uses resident session flow
- **WHEN** the pilot portal is active
- **THEN** the owning authority SHALL be a resident authenticated actor using the existing session and lease model

### Requirement: Bounded Transcript Viewport
Phase-0 portal transcript rendering SHALL be a bounded viewport rather than an unbounded transcript store. Any transcript text materialized into scene nodes MUST remain within existing node-size and per-tile resource budgets. Full retained history, if any, SHALL live outside the scene graph.
Source: RFC 0013 §3.4, RFC 0001 §2.4
Scope: v1-mandatory

#### Scenario: visible transcript fits node budget
- **WHEN** a portal renders an expanded transcript window
- **THEN** the text content placed into scene nodes SHALL stay within the existing TextMarkdown node size limits and tile resource budgets

#### Scenario: full history not mirrored into scene nodes
- **WHEN** a portal session has transcript history larger than the currently visible window
- **THEN** the runtime SHALL represent only the bounded visible or immediately scrollable window in scene nodes rather than mirroring the full retained history

### Requirement: Low-Latency Text Interaction
Text stream portals SHALL support low-latency incremental output, bounded viewer input submission, and local-first interaction feedback. Output transcript updates SHALL behave as state-stream traffic. Viewer submit and explicit control actions SHALL behave as transactional traffic. Activity indicators and similar transient state SHALL behave as ephemeral realtime traffic.
Source: RFC 0013 §4, §5
Scope: v1-mandatory

#### Scenario: output updates stream incrementally
- **WHEN** new text output arrives in multiple increments
- **THEN** the portal SHALL present those increments as an ordered streamed interaction rather than only as a final snapshot

#### Scenario: viewer submit is transactional
- **WHEN** the viewer submits a reply or activates an interrupt-style control
- **THEN** that action SHALL be treated as transactional input rather than coalescible state-stream traffic

### Requirement: Transcript Interaction Contract
Expanded text stream portals SHALL support scrollable transcript viewing, focusable interaction affordances, and bounded reply submission under the existing local-feedback contract. Portal interaction MUST reuse runtime-owned focus, command-input, and local visual acknowledgement rules. User scroll input MUST remain authoritative over any adapter-driven attempt to reposition the transcript viewport.
Source: RFC 0013 §4.1, §4.2, §4.3, RFC 0004 §6.7
Scope: v1-mandatory

#### Scenario: transcript scroll is local-first
- **WHEN** the viewer scrolls an expanded transcript surface
- **THEN** the visible scroll offset SHALL update locally before any adapter acknowledgement is required

#### Scenario: portal control uses local feedback
- **WHEN** the viewer activates an expand, collapse, or reply affordance
- **THEN** the runtime SHALL provide local-first interaction feedback under the same latency contract as other interactive surfaces

### Requirement: Coherent Transcript Coalescing
State-stream coalescing for portal output MUST preserve a coherent retained transcript window. Intermediate render states MAY be skipped, but already-committed logical transcript units within the retained on-screen history window MUST NOT be lost merely because updates were coalesced.
Source: RFC 0013 §5
Scope: v1-mandatory

#### Scenario: coalescing preserves retained window
- **WHEN** several portal append operations are coalesced under backpressure
- **THEN** the runtime MAY render a newer complete transcript window snapshot but SHALL NOT collapse the retained window into only the latest line

### Requirement: Governance, Privacy, and Override Compliance
Text stream portals SHALL obey the same lease, privacy, redaction, dismissal, freeze, and safe-mode rules as any other governed surface. Portal identity, transcript content, and activity metadata MUST NOT bypass viewer-class filtering or shell overrides because they are text. Collapsed portal cards MUST NOT be treated as automatically safe metadata.
Source: RFC 0013 §3.3, §6.2, §6.3, §6.4
Scope: v1-mandatory

#### Scenario: portal content redacts under viewer policy
- **WHEN** portal content exceeds the current viewer's permitted classification
- **THEN** the portal SHALL be redacted under the runtime's existing privacy policy rather than exposing the transcript content

#### Scenario: portal suspends under safe mode
- **WHEN** the runtime enters safe mode while a portal is active
- **THEN** portal updates SHALL suspend under the same shell and lease rules as other content-layer surfaces

### Requirement: External Adapter Isolation
Any adapter that emits portal output, accepts viewer input, or requests portal visibility SHALL remain external to the runtime core and SHALL pass through existing authentication and capability boundaries. The runtime core MUST NOT gain implicit authority over external process or transport lifecycles as a side effect of supporting text stream portals.
Source: RFC 0013 §2.2, §2.3
Scope: v1-mandatory

#### Scenario: local adapter still authenticates
- **WHEN** a local adapter process connects to drive a text stream portal
- **THEN** it SHALL authenticate and operate under explicit capability grants rather than implicit local trust

#### Scenario: runtime does not become process host
- **WHEN** a tmux-backed portal is active
- **THEN** the runtime SHALL remain unaware of tmux-specific lifecycle management beyond the generic stream signals it receives
