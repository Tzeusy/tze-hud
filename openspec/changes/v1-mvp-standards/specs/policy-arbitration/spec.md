# Policy Arbitration Specification

Source: RFC 0009 (Policy and Arbitration)
Domain: GOVERNANCE

---

## ADDED Requirements

### Requirement: Seven-Level Arbitration Stack
The runtime MUST implement a fixed 7-level arbitration stack with the following precedence (highest to lowest): Level 0 Human Override, Level 1 Safety, Level 2 Privacy, Level 3 Security, Level 4 Attention, Level 5 Resource, Level 6 Content. This ordering is doctrine and MUST NOT be modified.
Source: RFC 0009 §1.1
Scope: v1-mandatory

#### Scenario: Stack levels are fixed
- **WHEN** the arbitration stack is initialized
- **THEN** it contains exactly 7 levels numbered 0-6 with Human Override at 0 (highest) and Content at 6 (lowest)

### Requirement: Cross-Level Conflict Resolution
When two policies at different levels conflict, the higher level MUST always win (Rule CL-1). Side effects of the losing level MUST be suppressed, not deferred (Rule CL-2). The winning level's override type MUST apply (Rule CL-3). Lower levels MUST NOT be evaluated when a higher level rejects (short-circuit).
Source: RFC 0009 §2.1
Scope: v1-mandatory

#### Scenario: Privacy overrides content
- **WHEN** Level 2 (Privacy) says "redact tile" but Level 6 (Content) says "show tile"
- **THEN** the tile is redacted and the content contention result is irrelevant

#### Scenario: Human override overrides resource
- **WHEN** Level 0 (Human) freezes the scene but Level 5 (Resource) would shed a tile
- **THEN** the tile stays frozen and is not shed

#### Scenario: Security short-circuits lower levels
- **WHEN** Level 3 (Security) denies a capability
- **THEN** Level 5 (Resource) and Level 6 (Content) are never evaluated for that mutation

### Requirement: Level 0 Human Override
Human override actions (dismiss, safe mode, freeze, mute) MUST be local, instant, and MUST NOT be intercepted, delayed, or vetoed by any agent or policy level. Override commands MUST be placed in a dedicated `OverrideCommandQueue` (bounded, capacity 16, SPSC) read by the compositor thread before any `MutationBatch` intake. Override MUST complete within one frame (16.6ms).
Source: RFC 0009 §1.1, §11.1
Scope: v1-mandatory

#### Scenario: Override preempts mutation intake
- **WHEN** the viewer presses `Ctrl+Shift+Escape` (safe mode) while mutation intake is in progress
- **THEN** the override command is processed before any pending mutations in the current batch

#### Scenario: Override cannot be vetoed
- **WHEN** an agent has a valid lease with high priority and the viewer dismisses its tile
- **THEN** the tile is dismissed regardless of priority or capability

### Requirement: Level 1 Safety Response
The safety level MUST monitor GPU device health and scene graph integrity. On `wgpu::DeviceError::Lost`, the runtime MUST enter safe mode (Tier 2 failure response). On scene graph corruption, the runtime MUST enter safe mode. The frame-time guardian emergency threshold (`frame_time_p95 > 14ms` sustained through all degradation levels) MUST trigger Level 5 emergency degradation. When multiple safety triggers fire simultaneously, the most severe response MUST win (severity order: catastrophic exit > safe mode entry > GPU reconfiguration).
Source: RFC 0009 §4.1, §11.2
Scope: v1-mandatory

#### Scenario: GPU device lost triggers safe mode
- **WHEN** `wgpu::DeviceError::Lost` is detected
- **THEN** the runtime enters safe mode (Tier 2) with `SafeModeEntryReason = CRITICAL_ERROR`

#### Scenario: Most severe safety response wins
- **WHEN** GPU failure and scene corruption are detected in the same frame
- **THEN** the most severe response is applied

### Requirement: GPU Failure Three-Tier Response
The runtime MUST implement a three-tier failure response: Tier 1 (recoverable GPU error) attempts `surface.configure()` and continues on success; Tier 2 (non-recoverable GPU, CPU intact) enters safe mode, suspends leases, renders chrome in software fallback, waits up to 2 seconds for overlay; Tier 3 (catastrophic -- safe mode overlay cannot render) flushes telemetry (200ms grace) and triggers graceful shutdown with non-zero exit code.
Source: RFC 0009 §4.1
Scope: v1-mandatory

#### Scenario: Tier 1 recovery succeeds
- **WHEN** `SurfaceError::Lost` occurs and `surface.configure()` succeeds
- **THEN** rendering continues transparently with no agent notification

#### Scenario: Tier 2 safe mode before shutdown
- **WHEN** GPU device is lost and reconfiguration fails
- **THEN** safe mode is entered first (up to 2 seconds for overlay) before triggering graceful shutdown

#### Scenario: Tier 3 catastrophic exit
- **WHEN** safe mode overlay cannot render because the GPU is completely unusable
- **THEN** telemetry is flushed (200ms) and the process exits with a non-zero exit code

### Requirement: Level 2 Privacy Evaluation
The privacy level MUST compare each tile's `VisibilityClassification` against the current `ViewerClass` and produce a redaction flag per the access matrix: Owner sees all classifications; HouseholdMember sees public and household; KnownGuest, Unknown, and Nobody see only public. When multiple viewers are present, the most restrictive viewer class MUST apply. Privacy transitions MUST complete within 2 frames (33.2ms).
Source: RFC 0009 §11.3
Scope: v1-mandatory

#### Scenario: Private content redacted for guest
- **WHEN** a tile has `private` classification and the viewer is `known_guest`
- **THEN** the tile is committed with redaction (content rendered, placeholder drawn over it)

#### Scenario: Multi-viewer restriction
- **WHEN** an owner and a guest are both present
- **THEN** the most restrictive viewer class (guest) applies, redacting all non-public content

#### Scenario: Owner sees all content
- **WHEN** the sole viewer is the owner
- **THEN** all content including `sensitive` classification is shown without redaction

### Requirement: Privacy Zone Ceiling Rule
The effective classification of a zone publication MUST be `max(agent_declared_classification, zone_default_classification)`. An agent MUST NOT be able to escalate visibility beyond the zone's ceiling.
Source: RFC 0009 §11.3
Scope: v1-mandatory

#### Scenario: Zone ceiling enforced
- **WHEN** an agent declares `public` classification for content published to a zone with `household` default classification
- **THEN** the effective classification is `household` (the higher restriction)

### Requirement: Redaction Ownership
The Privacy level (Level 2) MUST own ALL redaction decisions. The `[privacy]` config section MUST be the single source of truth for `redaction_style`. The `ChromeConfig` (`[chrome]` section) MUST NOT contain a `redaction_style` field. The chrome layer renders the redaction visual but MUST NOT decide what to redact.
Source: RFC 0009 §5.1, §5.2, §5.3
Scope: v1-mandatory

#### Scenario: Privacy decides, chrome renders
- **WHEN** a tile needs redaction
- **THEN** Level 2 (Privacy) produces the redaction flag and the chrome render pass draws the placeholder pattern from `PrivacyConfig.redaction_style`

### Requirement: Level 3 Security Enforcement
The security level MUST enforce capability scopes, lease validity, and namespace isolation. All required capabilities MUST pass (security is conjunctive -- a single failing check rejects the mutation). Capability checks MUST be hash-table lookups completing in < 5us. The check MUST produce `Reject(CapabilityDenied)`, `Reject(CapabilityScopeInsufficient)`, or `Reject(NamespaceViolation)` with the missing capability named in the `hint` field.
Source: RFC 0009 §11.4
Scope: v1-mandatory

#### Scenario: Capability denied
- **WHEN** an agent attempts to create a tile without `create_tiles` capability
- **THEN** the mutation is rejected with `CapabilityDenied` error naming the missing capability

#### Scenario: Namespace violation
- **WHEN** an agent attempts to modify a tile in another agent's namespace
- **THEN** the mutation is rejected with `NamespaceViolation`

#### Scenario: Capability check latency
- **WHEN** a single capability check is performed
- **THEN** it completes in < 5us

### Requirement: Level 4 Attention Management
The attention level MUST manage interruption flow with five classes matching the InterruptionClass enum defined in scene-events: SILENT (always passes, not counted), LOW (blocked during quiet hours, counted), NORMAL (blocked during quiet hours, counted), HIGH (passes through quiet hours, subject to budget), CRITICAL (always passes, bypasses both quiet hours and budget). Attention budget MUST track rolling per-agent and per-zone counters over the last 60 seconds with configurable limits (defaults: 20 per agent per minute, 10 per zone per minute; 30 per zone per minute for Stack-policy zones). When budget is exhausted, mutations MUST be coalesced (latest-wins within agent+zone key). Attention budget check MUST complete in < 10us.
Source: RFC 0009 §11.5
Scope: v1-mandatory

#### Scenario: Quiet hours block NORMAL interruptions
- **WHEN** quiet hours are active and a mutation with NORMAL interruption class arrives
- **THEN** the mutation is queued until quiet hours end

#### Scenario: HIGH passes quiet hours
- **WHEN** quiet hours are active and a mutation with HIGH interruption class arrives
- **THEN** the mutation passes through quiet hours

#### Scenario: CRITICAL bypasses everything
- **WHEN** a CRITICAL interruption arrives during quiet hours with an exhausted attention budget
- **THEN** the mutation passes through both quiet hours and attention budget

#### Scenario: Attention budget exhausted
- **WHEN** an agent exceeds 20 interruptions per minute
- **THEN** subsequent mutations are coalesced (latest-wins) until the budget refills

### Requirement: Level 5 Resource Enforcement
The resource level MUST enforce per-agent budgets and the degradation ladder. Over-budget batches MUST be rejected atomically. Degradation shedding MUST NOT produce an error to the agent. Zone state MUST be updated even for shed mutations (scene state correct, render omitted). Transactional mutations (`CreateTile`, `DeleteTile`, `LeaseRequest`, `LeaseRelease`) MUST never be shed.
Source: RFC 0009 §11.6
Scope: v1-mandatory

#### Scenario: Shed mutation updates zone state
- **WHEN** a mutation is shed at Level 5 due to degradation
- **THEN** the zone state is updated to reflect the publish but the render output is omitted

#### Scenario: Transactional mutations never shed
- **WHEN** degradation is at Level 5 and a `CreateTile` mutation arrives
- **THEN** the mutation is NOT shed; it passes through to scene commit

### Requirement: Level 6 Content Resolution
The content level MUST resolve zone contention using the zone's `ContentionPolicy`: LatestWins (new replaces previous), Stack (new stacks, auto-dismiss after timeout), MergeByKey (same key replaces, different keys coexist), Replace (single occupant, eviction by equal-or-higher lease priority). Same-frame contention MUST be resolved in arrival order. Cross-tab zone isolation MUST be enforced (zones are scoped to their tab). Zone contention resolution MUST complete in < 20us.
Source: RFC 0009 §11.7
Scope: v1-mandatory

#### Scenario: LatestWins contention
- **WHEN** two agents publish to a LatestWins zone in the same frame
- **THEN** the second publish (by arrival order) replaces the first

#### Scenario: Cross-tab zone isolation
- **WHEN** an agent publishes to `tab_a/subtitle`
- **THEN** it does not interact with `tab_b/subtitle`

### Requirement: Per-Frame Evaluation Pipeline
The runtime MUST evaluate the arbitration stack per-frame at the start of each frame cycle, before mutation intake. Per-frame evaluation order MUST be: Level 1 (Safety) -> Level 2 (Privacy) -> Level 5 (Resource) -> Level 6 (Content). If Level 1 triggers safe mode, Levels 2/5/6 MUST NOT be evaluated for that frame. Total per-frame evaluation MUST complete in < 200us.
Source: RFC 0009 §3.2
Scope: v1-mandatory

#### Scenario: Per-frame safety check
- **WHEN** a frame cycle begins
- **THEN** GPU health and scene integrity are checked first; if safe mode triggers, no further per-frame evaluation occurs

#### Scenario: Per-frame evaluation within budget
- **WHEN** a frame with typical tile count is evaluated
- **THEN** the full per-frame evaluation (Levels 1, 2, 5, 6) completes in < 200us

### Requirement: Per-Event Evaluation Pipeline
The runtime MUST evaluate the arbitration stack per-event during input drain (Stage 1) and local feedback (Stage 2). Per-event evaluation order MUST be: Level 0 (Human Override) -> Level 4 (Attention) -> Level 3 (Security). If Level 0 triggers safe mode, Levels 4 and 3 MUST stop.
Source: RFC 0009 §3.3
Scope: v1-mandatory

#### Scenario: Override processed before attention
- **WHEN** a `Ctrl+Shift+Escape` input and a tab-switch event arrive in the same frame
- **THEN** the override (safe mode) is processed first and the tab-switch event is discarded

### Requirement: Per-Mutation Evaluation Pipeline
Every mutation in every `MutationBatch` MUST pass through the per-mutation evaluation. For tile mutations, the order MUST be: Security (3) -> Resource (5) -> Content (6). For zone publications, the full order MUST be: Override preemption (0) -> Security (3) -> Privacy (2, redaction decoration) -> Attention (4) -> Resource (5) -> Content (6). Short-circuit: if a higher level rejects, lower levels MUST NOT be evaluated. Per-mutation policy check MUST complete in < 50us.
Source: RFC 0009 §3.4
Scope: v1-mandatory

#### Scenario: Zone publish full stack
- **WHEN** a zone publish mutation arrives
- **THEN** it passes through override preemption, security gate, privacy decoration, attention gate, resource gate, and content resolution in order

#### Scenario: Per-mutation latency
- **WHEN** a frame contains 64 mutations (max batch)
- **THEN** each per-mutation policy check completes within 50us (total ~3.2ms for the batch)

### Requirement: ArbitrationOutcome Types
The arbitration stack MUST produce one of six outcomes for each mutation: Commit (accepted, committed to scene), CommitRedacted (committed but rendered with redaction placeholder), Queue (deferred until condition clears), Reject (rejected with structured error), Shed (shed by resource/degradation policy, no error), Blocked (queued by human override freeze).
Source: RFC 0009 §1.3
Scope: v1-mandatory

#### Scenario: CommitRedacted outcome
- **WHEN** Privacy (Level 2) determines a tile should be redacted
- **THEN** the mutation is committed (scene state updated) but rendering uses a redaction placeholder

#### Scenario: Blocked by freeze
- **WHEN** the scene is frozen and an agent submits a mutation
- **THEN** the outcome is Blocked with `block_reason = Freeze` and the mutation is queued

### Requirement: Override Type Taxonomy
Each arbitration level MUST use specific override types: Suppress (action prevented, agent informed), Redirect (action rerouted, e.g., input to chrome), Transform (action modified, e.g., redaction), Block (action queued for later). Level 0 uses Suppress/Redirect/Block, Level 1 uses Suppress/Redirect, Level 2 uses Transform, Level 3 uses Suppress, Level 4 uses Block, Level 5 uses Suppress/Transform, Level 6 uses Suppress.
Source: RFC 0009 §7.1, §7.2
Scope: v1-mandatory

#### Scenario: Privacy transforms but does not suppress
- **WHEN** Level 2 (Privacy) redacts a tile
- **THEN** the mutation is committed (not rejected) but rendered with a placeholder (Transform override type)

#### Scenario: Level 3 suppresses
- **WHEN** Level 3 (Security) denies a capability
- **THEN** the mutation is rejected with a structured error (Suppress override type)

### Requirement: Override Composition
When multiple levels each want to apply an override, the highest level's type MUST take precedence. If Level 0 blocks (freeze) and Level 5 would suppress (budget), the mutation MUST be blocked. If Level 2 transforms (redact) and Level 4 would block (quiet hours), the mutation MUST be both transformed and blocked (queued with redaction flag, rendered with placeholder on delivery).
Source: RFC 0009 §7.3
Scope: v1-mandatory

#### Scenario: Block wins over suppress
- **WHEN** the scene is frozen (Level 0 block) and the agent is over budget (Level 5 suppress)
- **THEN** the mutation is blocked (freeze wins)

#### Scenario: Transform and block composed
- **WHEN** Level 2 redacts and Level 4 queues for quiet hours
- **THEN** the mutation is queued with a redaction flag and rendered with placeholder when delivered

### Requirement: Freeze Semantics at Level 0
Freeze MUST be classified as a Level 0 (Human Override) action. During freeze: agent mutations MUST be queued (not rejected), resource budgets MUST be paused, the degradation ladder MUST NOT advance, attention signals MUST be deferred, tile rendering MUST be frozen at last committed state, override controls MUST remain active, agent input routing MUST be suspended. Max freeze duration: configurable (default 5 minutes), auto-unfreeze on timeout.
Source: RFC 0009 §6.1, §6.2
Scope: v1-mandatory

#### Scenario: Degradation paused during freeze
- **WHEN** the scene is frozen and frame-time would normally trigger degradation
- **THEN** the degradation ladder does not advance

#### Scenario: Auto-unfreeze on timeout
- **WHEN** the scene has been frozen for longer than the max freeze duration (default 5 minutes)
- **THEN** the scene auto-unfreezes with a `DegradationNotice` advisory to all agents

### Requirement: Capability Registry Canonical Names
The capability registry MUST use `snake_case` naming convention. All identifiers MUST match the table in RFC 0009 §8.1. Sub-scoped identifiers MUST use colon separation (`publish_zone:notification`). Only `publish_zone:*` is supported as a wildcard form. Older uppercase forms (`CREATE_TILE`) and kebab-case forms (`create-tiles`) MUST be treated as superseded and MUST NOT be used.
Source: RFC 0009 §8.1, §8.2
Scope: v1-mandatory

#### Scenario: Snake case enforced
- **WHEN** the runtime evaluates a capability check
- **THEN** it uses `snake_case` canonical names (e.g., `create_tiles`, not `CREATE_TILE` or `create-tiles`)

### Requirement: Policy Evaluation Latency Budgets
The runtime MUST meet the following latency budgets: full per-frame evaluation < 200us, per-mutation policy check < 50us, human override response < 1 frame (16.6ms), privacy transition < 2 frames (33.2ms), single capability check < 5us, attention budget check < 10us, zone contention resolution < 20us.
Source: RFC 0009 §9.1
Scope: v1-mandatory

#### Scenario: All latency budgets met
- **WHEN** the runtime operates under normal load
- **THEN** all policy evaluation latency budgets are met as specified

### Requirement: Policy Telemetry
Policy evaluation latency MUST be tracked in the per-frame `TelemetryRecord` via a `PolicyTelemetry` struct containing: `per_frame_eval_us`, `per_mutation_eval_us_p99`, `mutations_rejected`, `mutations_redacted`, `mutations_queued`, `mutations_shed`, and `override_commands_processed`.
Source: RFC 0009 §9.2
Scope: v1-mandatory

#### Scenario: Telemetry captures rejection count
- **WHEN** 3 mutations are rejected at Level 3 in a single frame
- **THEN** the frame's `PolicyTelemetry.mutations_rejected` is 3

### Requirement: Arbitration Telemetry Events
Every Level 3 rejection and every Level 5 shed MUST emit a telemetry record with the arbitration level, error code, agent ID, mutation reference, and timestamp. Levels 2 (redaction) and 4 (attention budget) MUST emit telemetry at a lower rate (one record per session per minute when those outcomes are active).
Source: RFC 0009 §13.1
Scope: v1-mandatory

#### Scenario: Rejection telemetry emitted
- **WHEN** a mutation is rejected at Level 3 for capability denial
- **THEN** a telemetry record is emitted with `event = "arbitration_reject"`, `level = 3`, `code = "CAPABILITY_DENIED"`, and the agent/mutation identifiers

### Requirement: Capability Grant Audit
Every capability grant and revocation MUST be logged with the agent ID, capability name, timestamp, and granting source (e.g., `session_handshake`, `admin_action`). Capabilities MUST be granted per-session and MAY be revoked mid-session (revocation is immediate).
Source: RFC 0009 §13.3
Scope: v1-mandatory

#### Scenario: Capability grant logged
- **WHEN** an agent is granted `publish_zone:notification` during session handshake
- **THEN** a structured log entry is emitted with the agent ID, capability name, timestamp, and `granted_by = "session_handshake"`

### Requirement: Within-Level Conflict Resolution
Within-level conflicts MUST follow level-specific tie-breaking rules: Level 0 processes multiple overrides in input-event order; Level 1 uses severity order (most severe wins); Level 2 uses most-restrictive viewer class; Level 3 requires all checks to pass (conjunctive); Level 4 uses longer deferral; Level 5 gives frame-time guardian precedence over per-agent throttling; Level 6 applies the zone's ContentionPolicy.
Source: RFC 0009 §2.2
Scope: v1-mandatory

#### Scenario: Level 2 most-restrictive wins
- **WHEN** two viewers with different access levels are present (owner and guest)
- **THEN** the most restrictive level (guest) applies

#### Scenario: Level 3 conjunctive check
- **WHEN** a mutation requires both `create_tiles` and `publish_zone:subtitle` capabilities
- **THEN** both must pass; failure of either rejects the mutation

### Requirement: Degradation Does Not Bypass Arbitration
A mutation shed at Level 5 MUST have already passed all higher levels (3, 2, etc.). Zone state MUST be updated for shed mutations (scene state correct, render omitted). The zone's occupancy is updated even for shed render mutations.
Source: RFC 0009 §12.2
Scope: v1-mandatory

#### Scenario: Shed mutation was capability-checked
- **WHEN** a mutation is shed at Level 5
- **THEN** it has already passed Level 3 (capability), Level 2 (privacy), and Level 4 (attention) checks

### Requirement: Freeze and Safe Mode Interaction
Per RFC 0007 §5.6: if freeze is active when safe mode triggers, safe mode MUST win (Level 1 overrides Level 0 freeze). Freeze MUST be cancelled, freeze queue discarded, `OverrideState.freeze_active` set to `false`. Freeze attempted during safe mode MUST be ignored. After safe mode exit, freeze MUST be inactive.
Source: RFC 0009 §6.5
Scope: v1-mandatory

#### Scenario: Safety overrides freeze
- **WHEN** the scene is frozen and a GPU failure triggers safe mode
- **THEN** freeze is cancelled, safe mode overlay appears, and freeze queue is discarded

### Requirement: Dynamic Policy Rules
Dynamic policy rules that can be loaded or modified at runtime are deferred to post-v1. V1 policy rules are static (derived from configuration at load time).
Source: RFC 0009 §16
Scope: post-v1

#### Scenario: Deferred dynamic policy
- **WHEN** v1 is deployed
- **THEN** all policy rules are static and defined at configuration load time

### Requirement: Per-Agent Arbitration Diagnostics
The `RuntimeService.GetSessionDiagnostics` RPC exposing per-agent arbitration statistics (reject counts by code, shed counts by level, queue depths) is deferred to post-v1. In v1, these statistics MUST be available only via telemetry.
Source: RFC 0009 §13.2
Scope: v1-reserved

#### Scenario: V1 diagnostics via telemetry only
- **WHEN** an operator needs per-agent arbitration statistics in v1
- **THEN** they must use telemetry records (no dedicated RPC)
