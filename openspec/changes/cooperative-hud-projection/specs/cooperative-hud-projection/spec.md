## ADDED Requirements

### Requirement: Cooperative Attachment Contract
An already-running LLM session SHALL attach to HUD projection only by explicitly registering a cooperative projection session with a projection daemon. Attachment SHALL use a stable projection ID, provider kind, display name, optional workspace/repository hints, optional icon/profile hint, lifecycle state, and content classification. The projection contract MUST NOT claim OS-level attachment to arbitrary process stdin/stdout, terminal byte streams, or PTY state.

#### Scenario: already-running session opts in
- **WHEN** an active Codex, Claude, opencode, or similar LLM session invokes the HUD projection attach workflow with projection ID `improve-feature-xyz`
- **THEN** the projection daemon SHALL register a logical projected session for `improve-feature-xyz`
- **AND** the session SHALL be marked as cooperatively attached rather than terminal-captured

#### Scenario: arbitrary terminal capture is out of scope
- **WHEN** a projection is attached to an already-running LLM session that was not launched under a projection wrapper
- **THEN** the daemon SHALL NOT promise automatic capture of all terminal output or automatic injection into terminal stdin
- **AND** the LLM session SHALL publish output and consume input through explicit cooperative projection operations

### Requirement: Daemon-Owned Projection State
The projection daemon SHALL own durable projection state outside the LLM token context. At minimum, daemon state SHALL include active HUD connection metadata, portal lease identity, visible transcript window, retained transcript history subject to configured bounds, pending HUD input inbox, acknowledgement state, lifecycle state, unread state, privacy classification, and reconnect bookkeeping.

#### Scenario: transcript retained outside token context
- **WHEN** a projected session publishes many output updates over a long-running coding session
- **THEN** the daemon SHALL retain the configured transcript window and history metadata outside the LLM prompt context
- **AND** the LLM-facing operation result SHALL expose only bounded summaries or requested slices rather than the full retained transcript by default

#### Scenario: daemon reconnect preserves projection state
- **WHEN** the HUD connection drops and reconnects before the projection expires
- **THEN** the daemon SHALL preserve pending input, acknowledgement state, lifecycle state, and the latest coherent visible transcript window
- **AND** it SHALL republish the projected portal using the existing lease or reconnect path allowed by runtime governance

### Requirement: Low-Token LLM-Facing Operations
The projection contract SHALL expose a small provider-neutral operation set suitable for a skill, local command, or MCP server. The normative operation set SHALL include attach, publish_output, publish_status, get_pending_input, acknowledge_input, detach, and cleanup. Operation responses SHALL be bounded by default and MUST NOT return unbounded transcripts, unbounded inbox history, or raw runtime scene state.

#### Scenario: agent publishes output without transcript drain
- **WHEN** an attached LLM session calls `publish_output` with a new assistant message or status fragment
- **THEN** the daemon SHALL append or merge that update into projection state and publish a bounded portal update
- **AND** the operation response SHALL report compact success metadata rather than returning the full transcript

#### Scenario: pending input check is compact
- **WHEN** an attached LLM session calls `get_pending_input`
- **THEN** the daemon SHALL return only pending unacknowledged submissions up to configured count and byte limits
- **AND** it SHALL include compact counts for remaining pending items if more input is queued

### Requirement: HUD Input Inbox Delivery
HUD-originated text input for a projected session SHALL be stored as ordered pending inbox items in the projection daemon. Each inbox item SHALL include an opaque input ID, projection ID, submission text, submitted_at_wall_us timestamp, delivery state, optional interaction metadata, and content classification. The daemon SHALL deliver input to the LLM session only through the cooperative operation contract, and the LLM session SHALL acknowledge each item as handled, deferred, rejected, or expired.

#### Scenario: submitted HUD input becomes pending item
- **WHEN** the operator submits bounded text in an expanded projected portal
- **THEN** the daemon SHALL create an ordered pending inbox item for the owning projection
- **AND** the item SHALL remain pending until the attached LLM session acknowledges it or the item expires under policy

#### Scenario: acknowledgement updates visible state
- **WHEN** the LLM session acknowledges a pending input item as handled
- **THEN** the daemon SHALL mark that item handled
- **AND** the portal SHALL no longer present that submission as waiting for the agent

#### Scenario: input is not terminal keystroke passthrough
- **WHEN** the operator types text into a projected portal
- **THEN** the daemon SHALL NOT send raw keystrokes to an external terminal process
- **AND** submitted text SHALL be delivered only as a bounded semantic input item through the cooperative contract

### Requirement: Projected Portal Lifecycle
A cooperative projection SHALL realize its visible HUD presence through the existing text-stream portal surface. The projected portal SHALL support collapsed and expanded states, movable/collapsible content-layer presentation, session identity/status, unread or pending-input indicators, and detach cleanup. Portal tiles SHALL remain governed by the existing resident session, lease, input, and content-layer rules.

#### Scenario: attach creates projected portal
- **WHEN** a cooperative projection attaches with valid HUD connection details and required capabilities
- **THEN** the daemon SHALL create or reuse a text-stream portal for that projection
- **AND** the portal SHALL render as content-layer territory rather than chrome

#### Scenario: collapse preserves session affordance
- **WHEN** the operator collapses a projected portal
- **THEN** the HUD SHALL preserve a movable projected-session icon or compact card
- **AND** the compact surface SHALL reflect only policy-permitted identity, lifecycle, unread, and pending-input state

#### Scenario: detach cleans up portal
- **WHEN** the LLM session detaches from projection or the operator requests cleanup
- **THEN** the daemon SHALL release or expire the portal lease under the runtime governance contract
- **AND** stale projected-session tiles SHALL be removed from the HUD

### Requirement: Provider-Neutral Projection Identity
Projection identity SHALL be provider-neutral. The core schema SHALL support provider kind values such as `codex`, `claude`, `opencode`, and `other`, but provider-specific data SHALL live in optional metadata and MUST NOT change the behavior of input delivery, output publication, portal lifecycle, or governance.

#### Scenario: Codex and Claude use same contract
- **WHEN** one projected session declares provider kind `codex` and another declares provider kind `claude`
- **THEN** both sessions SHALL use the same attach, publish, input, acknowledgement, detach, and cleanup semantics
- **AND** provider kind MAY affect icon/profile selection without changing projection behavior

#### Scenario: unknown provider still projects
- **WHEN** an attached session declares provider kind `other`
- **THEN** the daemon SHALL allow projection if required fields and capabilities are valid
- **AND** it SHALL fall back to a generic icon/profile if no provider-specific profile exists

### Requirement: Privacy and Attention Governance
Projection identity, transcript content, status metadata, unread indicators, pending-input indicators, and provider/workspace hints SHALL be subject to the same privacy, redaction, attention, dismiss, freeze, and safe-mode rules as text-stream portals. Collapsed projected-session icons MUST NOT be treated as automatically safe metadata.

#### Scenario: collapsed projection redacts private identity
- **WHEN** viewer policy does not permit the current viewer to see the projection identity or workspace metadata
- **THEN** the collapsed icon or compact card SHALL preserve geometry but suppress provider, session name, workspace hints, transcript preview, and activity details
- **AND** it SHALL use the runtime's neutral redaction treatment

#### Scenario: unread backlog remains ambient
- **WHEN** a projected session accumulates unread output or pending HUD input without an explicit higher-priority policy
- **THEN** the projected portal SHALL remain ambient or gentle
- **AND** it SHALL NOT escalate interruption class solely because backlog grows

### Requirement: Bounded Backpressure and Expiry
Projection operations SHALL enforce configurable bounds for output payload size, retained transcript bytes, pending input count, pending input byte size, polling result size, and portal update rate. When bounds are exceeded, the daemon SHALL reject, truncate, coalesce, expire, or summarize according to explicit policy while preserving transactional HUD input semantics.

#### Scenario: oversized output is bounded
- **WHEN** an attached LLM session publishes output larger than the configured per-operation limit
- **THEN** the daemon SHALL reject or truncate the update with an explicit bounded-result status
- **AND** it SHALL NOT materialize the oversized payload into scene nodes

#### Scenario: pending input queue reaches limit
- **WHEN** the pending HUD input queue for a projection reaches its configured maximum
- **THEN** the daemon SHALL preserve already accepted transactional input items
- **AND** new submissions SHALL be rejected or deferred with a visible bounded-state indication rather than silently dropped
