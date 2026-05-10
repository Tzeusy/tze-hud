# cooperative-hud-projection Specification

## Purpose
Define the provider-neutral, cooperative HUD projection contract that lets already-running LLM sessions attach to a governed text-stream portal through an external projection authority, without terminal capture or runtime ownership of provider processes.

## Requirements
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

### Requirement: External Projection State Authority
Cooperative HUD projection SHALL retain projection state outside the LLM token context and outside the runtime core through an authenticated external projection authority. At minimum, retained projection state SHALL include active HUD connection metadata, advisory portal lease identity, visible transcript window, retained transcript history subject to configured bounds, pending HUD input inbox, acknowledgement state, lifecycle state, unread state, privacy classification, and reconnect bookkeeping.

For v1, projection state SHALL be memory-only and SHALL NOT persist transcript text, pending input text, owner tokens, or lease identity across daemon or host restart. Detach, cleanup, projection expiry, lease grace expiry, and owner-token expiry SHALL purge transcript text, pending input text, and owner-token verifiers for that projection. Audit records MAY outlive projection cleanup but SHALL NOT include transcript text, pending input text, or owner tokens.

#### Scenario: transcript retained outside token context
- **WHEN** a projected session publishes many output updates over a long-running coding session
- **THEN** the external projection authority SHALL retain the configured transcript window and history metadata outside the LLM prompt context
- **AND** the LLM-facing operation result SHALL expose only bounded summaries or requested slices rather than the full retained transcript by default

#### Scenario: reconnect preserves projection state
- **WHEN** the HUD connection drops and reconnects before the projection expires
- **THEN** the external projection authority SHALL preserve pending input, acknowledgement state, lifecycle state, and the latest coherent visible transcript window
- **AND** it SHALL republish the projected portal only through the existing lease or reconnect path allowed by runtime governance

#### Scenario: daemon restart purges private projection state
- **WHEN** the projection daemon or host restarts
- **THEN** prior transcript text, pending input text, owner tokens, and cached lease identity SHALL be unavailable
- **AND** the LLM session MUST attach again and receive a fresh owner token before publishing or reading input

#### Scenario: stale lease identity cannot authorize republish
- **WHEN** the runtime restarts, a resume token expires, or a cached portal lease is no longer valid
- **THEN** cached lease identity SHALL be treated as advisory state only
- **AND** the projection authority MUST perform fresh session authentication, regain authorized capabilities, and acquire a new valid lease before publishing portal mutations
- **AND** it MUST NOT use stale lease identity or request lease capabilities broader than the authenticated session grants

### Requirement: Low-Token LLM-Facing Operations
The projection contract SHALL expose a small provider-neutral operation set suitable for a skill, local command, or external projection-daemon MCP server. If exposed through MCP, projection operations SHALL be served by the external projection authority rather than the runtime v1 MCP bridge. The normative operation set SHALL include attach, publish_output, publish_status, get_pending_input, acknowledge_input, detach, and cleanup. Operation responses SHALL be bounded by default and MUST NOT return unbounded transcripts, unbounded inbox history, or raw runtime scene state.

Every operation request SHALL include `operation`, `projection_id`, `request_id`, and `client_timestamp_wall_us`. Every operation except `attach` SHALL also include the current `owner_token`. The `attach` request SHALL include `provider_kind`, `display_name`, optional `workspace_hint`, optional `repository_hint`, optional `icon_profile_hint`, optional `content_classification`, optional `hud_target`, and optional `idempotency_key`. The `publish_output` request SHALL include `output_text`, optional `output_kind`, optional `content_classification`, optional `logical_unit_id`, and optional `coalesce_key`. The `publish_status` request SHALL include `lifecycle_state` and optional bounded `status_text`. The `get_pending_input` request MAY include `max_items` and `max_bytes`. The `acknowledge_input` request SHALL include `input_id`, `ack_state`, optional bounded `ack_message`, and optional `not_before_wall_us` valid only when `ack_state = deferred`. When present, `not_before_wall_us` SHALL use wall-clock semantics, MUST be before the item's `expires_at_wall_us`, and MUST NOT be accepted for terminal acknowledgement states. The `detach` and `cleanup` requests SHALL include bounded `reason`.

Every operation response SHALL include `request_id`, `projection_id`, `accepted`, `error_code`, `server_timestamp_wall_us`, and a bounded `status_summary`. Successful `attach` responses SHALL include an `owner_token` and initial lifecycle state. Error codes SHALL be stable append-only strings. The initial error-code set SHALL include `PROJECTION_NOT_FOUND`, `PROJECTION_ALREADY_ATTACHED`, `PROJECTION_UNAUTHORIZED`, `PROJECTION_TOKEN_EXPIRED`, `PROJECTION_INVALID_ARGUMENT`, `PROJECTION_OUTPUT_TOO_LARGE`, `PROJECTION_INPUT_TOO_LARGE`, `PROJECTION_INPUT_QUEUE_FULL`, `PROJECTION_RATE_LIMITED`, `PROJECTION_STATE_CONFLICT`, `PROJECTION_HUD_UNAVAILABLE`, and `PROJECTION_INTERNAL_ERROR`.

#### Scenario: agent publishes output without transcript drain
- **WHEN** an attached LLM session calls `publish_output` with a new assistant message or status fragment
- **THEN** the daemon SHALL append or merge that update into projection state and publish a bounded portal update
- **AND** the operation response SHALL report compact success metadata rather than returning the full transcript

#### Scenario: pending input check is compact
- **WHEN** an attached LLM session calls `get_pending_input`
- **THEN** the daemon SHALL return only pending unacknowledged submissions up to configured count and byte limits
- **AND** it SHALL include compact counts for remaining pending items if more input is queued

#### Scenario: attach conflict is deterministic
- **WHEN** an `attach` request uses a projection ID that is already owned by a live projection session and does not present a matching idempotency key
- **THEN** the projection authority SHALL reject the request with `PROJECTION_ALREADY_ATTACHED`
- **AND** it SHALL NOT rotate ownership, expose the existing owner token, or reveal private transcript or input state

#### Scenario: operation audit record is structured
- **WHEN** a projection operation is accepted or denied
- **THEN** the projection authority SHALL emit an audit record containing `timestamp_wall_us`, `operation`, `projection_id`, `caller_identity`, `request_id`, `accepted`, `error_code`, and `reason`
- **AND** the audit record SHALL NOT contain transcript text or HUD input text

### Requirement: Projection Operation Authorization
Every LLM-facing projection operation SHALL be authenticated, bound to the owning projection session, and authorized for the requested projection ID before it can read or mutate projection state. Projection credentials or local IPC authority MUST be unguessable or OS-protected. Cross-projection publish, status update, pending-input read, acknowledgement, detach, and cleanup attempts SHALL be denied with stable error codes and auditable records.

Successful attach SHALL issue an owner token with at least 128 bits of entropy. The projection authority SHALL store only a verifier or protected representation of the token. Every non-attach owner operation SHALL present the owner token. Owner cleanup SHALL require the owner token. Operator cleanup or override SHALL use a separate explicit operator-authority credential or local override path, SHALL NOT require the owner token, and SHALL be audited distinctly from owner cleanup. Owner tokens SHALL expire or rotate on detach, owner cleanup, operator cleanup, projection expiry, host restart, and explicit operator revocation. Same-OS-user execution alone SHALL NOT authorize cross-projection reads or mutations.

#### Scenario: cross-projection input read is denied
- **WHEN** session A calls `get_pending_input` for projection ID owned by session B
- **THEN** the projection authority SHALL deny the request with a stable authorization error code
- **AND** it SHALL NOT return pending input, transcript content, identity metadata, or lifecycle state for session B

#### Scenario: unauthorized cleanup is denied
- **WHEN** a caller without ownership or explicit operator authority calls `cleanup` for another projection
- **THEN** the projection authority SHALL deny the operation with a stable authorization error code
- **AND** it SHALL emit an audit record containing timestamp, caller identity, projection ID, operation, and denial reason

#### Scenario: operator cleanup uses separate authority
- **WHEN** an operator cleanup request presents valid operator authority for a projection whose owner token is missing or expired
- **THEN** the projection authority SHALL allow cleanup without exposing the owner token or private projection content
- **AND** it SHALL audit the operation as operator cleanup rather than owner cleanup

### Requirement: HUD Input Inbox Delivery
HUD-originated text input for a projected session SHALL be stored as ordered pending inbox items in the projection daemon. Each inbox item SHALL include an opaque input ID, projection ID, submission text, submitted_at_wall_us timestamp, expires_at_wall_us timestamp, delivery state, optional interaction metadata, and content classification. The daemon SHALL deliver input to the LLM session only through the cooperative operation contract. The LLM session SHALL acknowledge each item only as handled, deferred, or rejected; expiration is owned by the projection authority.

Inbox item states SHALL be `pending`, `delivered`, `deferred`, `handled`, `rejected`, or `expired`. New accepted submissions start as `pending`. `get_pending_input` SHALL return eligible `pending` and `deferred` items in FIFO order by submission sequence, subject to response bounds, and SHALL transition returned items to `delivered` with `delivered_at_wall_us`. `acknowledge_input` MAY transition `pending`, `delivered`, or `deferred` items to terminal `handled` or `rejected`; it MAY transition `delivered` items back to `deferred` with optional `not_before_wall_us`. Expiry SHALL use `expires_at_wall_us`; expired items transition to terminal `expired` before delivery. Repeating the same acknowledgement request for the same input ID and same terminal state SHALL be idempotent. A conflicting acknowledgement for an already terminal item SHALL return `PROJECTION_STATE_CONFLICT`.

#### Scenario: submitted HUD input becomes pending item
- **WHEN** the operator submits bounded text in an expanded projected portal
- **THEN** the daemon SHALL create an ordered pending inbox item for the owning projection
- **AND** the item SHALL remain non-terminal and waiting for agent acknowledgement until the attached LLM session acknowledges it or the item expires under policy

#### Scenario: submitted HUD input gets local pending feedback
- **WHEN** the operator submits bounded text in an expanded projected portal
- **THEN** the portal SHALL provide local-first visible acknowledgement that the submission was accepted or rejected
- **AND** accepted submissions SHALL render bounded pending state without waiting for the LLM session to poll, process, or acknowledge the item

#### Scenario: acknowledgement updates visible state
- **WHEN** the LLM session acknowledges a pending, delivered, or deferred input item as handled
- **THEN** the daemon SHALL mark that item handled
- **AND** the portal SHALL no longer present that submission as waiting for the agent

#### Scenario: deferred input is redelivered after not-before time
- **WHEN** the LLM session acknowledges a delivered input item as deferred with `not_before_wall_us = T`
- **THEN** the item SHALL be hidden from `get_pending_input` results until wall-clock time T
- **AND** after T it SHALL be eligible for FIFO redelivery unless it has expired

#### Scenario: conflicting terminal acknowledgement is rejected
- **WHEN** an input item is already terminal `handled` and the session later acknowledges the same input ID as `rejected`
- **THEN** the projection authority SHALL reject the later acknowledgement with `PROJECTION_STATE_CONFLICT`
- **AND** the item SHALL remain `handled`

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
- **AND** interactive affordances that would reveal private content SHALL be disabled while redacted

#### Scenario: missing classification fails closed
- **WHEN** projection identity, output, status metadata, provider/workspace hints, or pending input omits content classification
- **THEN** the missing classification SHALL default to `private`
- **AND** effective visibility SHALL be the most restrictive of projection default, item/output classification, and portal or tile policy

#### Scenario: unread backlog remains ambient
- **WHEN** a projected session accumulates unread output or pending HUD input without an explicit higher-priority policy
- **THEN** the projected portal SHALL remain ambient or gentle
- **AND** it SHALL NOT escalate interruption class solely because backlog grows

### Requirement: Bounded Backpressure and Expiry
Projection operations SHALL enforce configurable bounds for output payload size, retained transcript bytes, pending input count, pending input byte size, polling result size, and portal update rate. When bounds are exceeded, the daemon SHALL reject, truncate, coalesce, expire, or summarize according to explicit policy while preserving transactional HUD input semantics.

Unless deployment configuration sets stricter values, v1 defaults SHALL be: `max_output_bytes_per_call = 16384`, `max_status_text_bytes = 512`, `max_retained_transcript_bytes = 262144`, `max_visible_transcript_bytes = 16384`, `max_pending_input_items = 32`, `max_pending_input_bytes_per_item = 4096`, `max_pending_input_total_bytes = 32768`, `max_poll_items = 8`, `max_poll_response_bytes = 16384`, and `max_portal_updates_per_second = 10`. Oversized output and input submissions SHALL be rejected with stable error codes rather than truncated silently. Retained transcript overflow SHALL prune oldest non-visible logical units first while preserving coherent visible-window order. Portal update-rate overflow SHALL coalesce output by `coalesce_key` or append order into the next permitted visible-window update.

#### Scenario: oversized output is bounded
- **WHEN** an attached LLM session publishes output larger than the configured per-operation limit
- **THEN** the daemon SHALL reject the update with `PROJECTION_OUTPUT_TOO_LARGE`
- **AND** it SHALL NOT materialize the oversized payload into scene nodes

#### Scenario: pending input queue reaches limit
- **WHEN** the pending HUD input queue for a projection reaches its configured maximum
- **THEN** the daemon SHALL preserve already accepted transactional input items
- **AND** new submissions SHALL be rejected with `PROJECTION_INPUT_QUEUE_FULL` and a visible bounded-state indication rather than silently dropped

#### Scenario: retained transcript overflow prunes oldest non-visible units
- **WHEN** appending a transcript unit would exceed `max_retained_transcript_bytes`
- **THEN** the projection authority SHALL prune oldest non-visible logical units until retained bytes are within budget
- **AND** it SHALL preserve the coherent visible transcript window required by the text-stream portal contract
