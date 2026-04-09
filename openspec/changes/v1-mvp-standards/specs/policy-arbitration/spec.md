# Policy Arbitration Specification

Source: RFC 0009 (Policy and Arbitration)
Domain: GOVERNANCE

---

## ADDED Requirements

### Requirement: V1 Authority Boundary (Implemented Runtime Path)
In v1 runtime builds, authoritative enforcement MUST remain runtime-owned and MUST be split across three surfaces: `tze_hud_runtime` (budget ladder, attention/quiet-hours state, and shell override state), `tze_hud_protocol` session handling (auth/capability/lease admission and mutation rejection surfaces), and `tze_hud_scene::policy` (scene-side policy contract). In v1, `tze_hud_policy` MUST remain a pure evaluator reference path and MUST NOT be treated as the hot-path mutation authority until that wiring is explicitly landed.
Source: about/heart-and-soul/architecture.md, crates/tze_hud_runtime/src/lib.rs, crates/tze_hud_runtime/src/budget.rs
Scope: v1-mandatory

#### Scenario: Runtime-owned enforcement remains authoritative
- **WHEN** a v1 runtime evaluates mutation admission and per-session budget state
- **THEN** decisions are produced by runtime/session/scene authorities, not by a centralized `tze_hud_policy` execution path

### Requirement: Unified Policy Hot-Path Wiring Is Deferred Beyond v1
The unified policy-wired arbitration path (single level-by-level execution through policy-owned evaluator contracts) is deferred beyond v1. In shipped v1 runtime builds, runtime/session/scene-owned enforcement remains the authority surface. Unified hot-path policy wiring MUST be treated as explicit post-v1 (v2) work.
Source: docs/reconciliations/policy_wiring_epic_prompt.md
Scope: post-v1

#### Scenario: v1 release does not claim unified hot-path wiring
- **WHEN** an operator audits v1 policy behavior
- **THEN** they MUST treat unified policy-stack execution as deferred and rely on runtime/session/scene enforcement semantics for v1 truth

### Requirement: Seven-Level Arbitration Stack
The target arbitration stack MUST use a fixed 7-level precedence (highest to lowest): Level 0 Human Override, Level 1 Safety, Level 2 Privacy, Level 3 Security, Level 4 Attention, Level 5 Resource, Level 6 Content. This ordering is doctrine and MUST NOT be modified.
Source: RFC 0009 §1.1
Scope: post-v1

#### Scenario: Stack levels are fixed
- **WHEN** the arbitration stack is initialized
- **THEN** it contains exactly 7 levels numbered 0-6 with Human Override at 0 (highest) and Content at 6 (lowest)

### Requirement: Cross-Level Conflict Resolution
When two policies at different levels conflict, the higher level MUST always win (Rule CL-1). Side effects of the losing level MUST be suppressed, not deferred (Rule CL-2). The winning level's override type MUST apply (Rule CL-3). Lower levels MUST NOT be evaluated when a higher level rejects (short-circuit).
Source: RFC 0009 §2.1
Scope: post-v1

#### Scenario: Privacy overrides content
- **WHEN** Level 2 (Privacy) says "redact tile" but Level 6 (Content) says "show tile"
- **THEN** the tile is redacted and the content contention result is irrelevant

#### Scenario: Human override overrides resource
- **WHEN** Level 0 (Human) freezes the scene but Level 5 (Resource) would shed a tile
- **THEN** the tile stays frozen and is not shed

#### Scenario: Security short-circuits lower levels
- **WHEN** Level 3 (Security) denies a capability
- **THEN** Level 5 (Resource) and Level 6 (Content) are never evaluated for that mutation

#### Scenario: Safety clears attention queue
- **WHEN** Level 1 (Safety) enters safe mode while Level 4 (Attention) has queued notifications
- **THEN** the queued notifications MUST be discarded; safe mode overrides the delivery queue

#### Scenario: Privacy redaction plus quiet hours
- **WHEN** Level 2 (Privacy) would redact a tile AND Level 4 (Attention) would queue it for quiet hours
- **THEN** the mutation IS committed with redaction (privacy transform applies) AND quiet hours evaluation still runs for presentation scheduling; the mutation is queued-with-redaction, not suppressed

#### Scenario: Human dismisses tile with valid lease
- **WHEN** the viewer dismisses a tile whose agent holds a valid ACTIVE lease
- **THEN** the lease is revoked immediately (Level 0 wins over Level 3 Security); the agent receives `LeaseResponse` with `REVOKED` and `revoke_reason = VIEWER_DISMISSED`

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
- **THEN** Level 2 (Privacy) produces the redaction flag and the chrome render pass draws the placeholder (pattern or blank, per `PrivacyConfig.redaction_style`)

### Requirement: Level 3 Security Enforcement
The security level MUST enforce capability scopes, lease validity, and namespace isolation. All required capabilities MUST pass (security is conjunctive -- a single failing check rejects the mutation). Capability checks MUST be hash-table lookups; each individual check MUST complete in < 5us under nominal load. The check MUST produce `Reject(CapabilityDenied)`, `Reject(CapabilityScopeInsufficient)`, or `Reject(NamespaceViolation)` with the missing capability named in the `hint` field.

Capability-source semantics in v1 are split and explicit:
- Session-held grants are the live authority for mutation and lease-scope checks.
- CapabilityRequest escalation eligibility is evaluated against the static config-derived per-agent allow-list.
- Dynamic rule updates and operator approval paths are post-v1 and MUST NOT be implied as active in v1 unless explicitly implemented.
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
- **THEN** it MUST complete in < 5us under nominal load

#### Scenario: Escalation policy source is distinct from held grants
- **WHEN** a capability is present in the per-agent allow-list but absent from current session grants
- **THEN** mutation and lease checks MUST still deny it until granted, and CapabilityRequest MAY grant it using the allow-list source

### Requirement: Level 4 Attention Management
The attention level MUST manage interruption flow with five classes matching the InterruptionClass enum defined in RFC 0010 §3.1: SILENT (always passes, never counted against budget), LOW (discarded during quiet hours; too stale by quiet hours exit to be useful), NORMAL (queued during quiet hours; delivered when quiet hours end), HIGH (passes through quiet hours unless `pass_through_class` raises the threshold; subject to attention budget), CRITICAL (always passes, bypasses both quiet hours and attention budget; only the runtime may emit CRITICAL). Attention budget MUST track rolling per-agent and per-zone counters over the last 60 seconds with configurable limits (defaults: 20 per agent per minute, 10 per zone per minute; 30 per zone per minute for Stack-policy zones per RFC 0010 §7). When budget is exhausted, mutations MUST be coalesced (latest-wins within agent+zone key). Attention budget check MUST complete in < 10us. Note: RFC 0009 §11.5 uses the older doctrine names (Gentle/Normal/Urgent) and older default values (6/12); RFC 0010 §3.1 is authoritative for both the enum names and the default budgets.
Source: RFC 0010 §3.1, §7, RFC 0009 §11.5
Scope: v1-mandatory

#### Scenario: Quiet hours queue NORMAL interruptions
- **WHEN** quiet hours are active and a mutation with NORMAL interruption class arrives
- **THEN** the mutation is queued until quiet hours end and then delivered in FIFO order

#### Scenario: Quiet hours discard LOW interruptions
- **WHEN** quiet hours are active and a mutation with LOW interruption class arrives
- **THEN** the mutation is discarded (not queued) because LOW content (background sync, telemetry, status refreshes) is too stale by quiet hours exit to be useful

#### Scenario: HIGH passes quiet hours
- **WHEN** quiet hours are active and a mutation with HIGH interruption class arrives
- **THEN** the mutation passes through quiet hours (unless `pass_through_class` is set higher than HIGH)

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
The content level MUST resolve zone contention using the zone's `ContentionPolicy`: LatestWins (new replaces previous), Stack (new stacks, auto-dismiss after timeout), MergeByKey (same key replaces, different keys coexist), Replace (single occupant, eviction by equal-or-higher lease priority). Same-frame contention MUST be resolved in arrival order. Cross-tab zone isolation MUST be enforced (zones are scoped to their tab). Zone contention resolution MUST complete in < 20us under nominal load.
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
Scope: post-v1

#### Scenario: Per-frame safety check
- **WHEN** a frame cycle begins
- **THEN** GPU health and scene integrity are checked first; if safe mode triggers, no further per-frame evaluation occurs

#### Scenario: Per-frame evaluation within budget
- **WHEN** a frame with typical tile count is evaluated
- **THEN** the full per-frame evaluation (Levels 1, 2, 5, 6) completes in < 200us

### Requirement: Per-Event Evaluation Pipeline
The runtime MUST evaluate the arbitration stack per-event during input drain (Stage 1) and local feedback (Stage 2). Per-event evaluation order MUST be: Level 0 (Human Override) -> Level 4 (Attention) -> Level 3 (Security). If Level 0 triggers safe mode, Levels 4 and 3 MUST stop.
Source: RFC 0009 §3.3
Scope: post-v1

#### Scenario: Override processed before attention
- **WHEN** a `Ctrl+Shift+Escape` input and a tab-switch event arrive in the same frame
- **THEN** the override (safe mode) is processed first and the tab-switch event is discarded

### Requirement: Per-Mutation Evaluation Pipeline
Every mutation in every `MutationBatch` MUST pass through the per-mutation evaluation. For tile mutations, the order MUST be: Security (3) -> Resource (5) -> Content (6). For zone publications, the full order MUST be: Override preemption (0) -> Security (3) -> Privacy (2, redaction decoration) -> Attention (4) -> Resource (5) -> Content (6). Short-circuit: if a higher level rejects, lower levels MUST NOT be evaluated. Per-mutation policy check MUST complete in < 50us.
Source: RFC 0009 §3.4
Scope: post-v1

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
Freeze MUST be classified as a Level 0 (Human Override) action. During freeze: agent mutations MUST be queued (not rejected), resource budgets MUST be paused, the degradation ladder MUST NOT advance, attention signals MUST be deferred, tile rendering MUST be frozen at last committed state, override controls MUST remain active, agent input routing MUST be suspended. Max freeze duration MUST be configurable; auto-unfreeze MUST occur on timeout.
Source: RFC 0009 §6.1, §6.2
Scope: v1-mandatory

#### Scenario: Degradation paused during freeze
- **WHEN** the scene is frozen and frame-time would normally trigger degradation
- **THEN** the degradation ladder does not advance

#### Scenario: Auto-unfreeze on timeout
- **WHEN** the scene has been frozen for longer than the configured max freeze duration
- **THEN** the scene auto-unfreezes with a `DegradationNotice` advisory to all agents

### Requirement: Capability Registry Canonical Names
The capability registry MUST use `snake_case` naming convention. All identifiers MUST match the table in RFC 0009 §8.1 AS AMENDED by RFC 0005 Round 14 (rig-b2s): the three capability names revised in that round are `read_scene_topology` (not `read_scene`), `access_input_events` (not `receive_input`), and `publish_zone:<zone>` (not `zone_publish:<zone>`). Sub-scoped identifiers MUST use colon separation (`publish_zone:notification`). Only `publish_zone:*` is supported as a wildcard form. Older uppercase forms (`CREATE_TILE`), kebab-case forms (`create-tiles`), and the pre-Round-14 names (`read_scene`, `receive_input`, `zone_publish`) MUST be treated as superseded and MUST NOT be used. The canonical source of truth for all capability names is RFC 0006 §6.3 (which references RFC 0005 as the wire-format authority).
Source: RFC 0009 §8.1, §8.2, RFC 0005 Round 14, RFC 0006 §6.3
Scope: v1-mandatory

#### Scenario: Snake case enforced
- **WHEN** the runtime evaluates a capability check
- **THEN** it uses `snake_case` canonical names (e.g., `create_tiles`, not `CREATE_TILE` or `create-tiles`)

#### Scenario: Pre-Round-14 names rejected
- **WHEN** a capability grant uses `read_scene`, `receive_input`, or `zone_publish:subtitle`
- **THEN** startup fails with `CONFIG_UNKNOWN_CAPABILITY` with a hint naming the canonical replacement (`read_scene_topology`, `access_input_events`, `publish_zone:subtitle`)

### Requirement: Policy Evaluation Latency Budgets
The runtime MUST meet the following latency budgets: full per-frame evaluation < 200us, per-mutation policy check < 50us, human override response < 1 frame (16.6ms), privacy transition < 2 frames (33.2ms). Individual sub-checks MUST each consume no more than 20us; their measured mean latencies MUST be ordered by cost as follows: capability lookup fastest (< 5us), then attention check (< 10us), then zone contention resolution (< 20us).
Source: RFC 0009 §9.1
Scope: post-v1

#### Scenario: Per-frame evaluation within budget
- **WHEN** the runtime operates under normal load
- **THEN** the full per-frame evaluation completes in < 200us

#### Scenario: Per-mutation evaluation within budget
- **WHEN** the runtime processes any mutation
- **THEN** the per-mutation policy check completes in < 50us

#### Scenario: Human override response within one frame
- **WHEN** a human override command is issued
- **THEN** the override response completes within 1 frame (16.6ms)

#### Scenario: Privacy transition within two frames
- **WHEN** viewer class changes require privacy re-evaluation
- **THEN** all privacy transitions complete within 2 frames (33.2ms)

### Requirement: Policy Telemetry
When mutation-path policy wiring is explicitly enabled, policy evaluation latency MUST be tracked in the per-frame `TelemetryRecord` via a `PolicyTelemetry` struct containing: `per_frame_eval_us`, `per_mutation_eval_us_p99`, `mutations_rejected`, `mutations_redacted`, `mutations_queued`, `mutations_shed`, and `override_commands_processed`.
Note: This remains target-state telemetry for the unified policy wiring seam. In current v1 runtime builds, authoritative hot-path enforcement is still runtime/session/scene-owned (see `Requirement: V1 Authority Boundary (Implemented Runtime Path)` above), and `tze_hud_policy` telemetry is not yet wired into per-frame runtime telemetry output.
Source: RFC 0009 §9.2
Scope: v1-reserved

#### Scenario: Telemetry captures rejection count
- **WHEN** mutation-path policy wiring is explicitly enabled and 3 mutations are rejected at Level 3 in a single frame
- **THEN** the frame's `PolicyTelemetry.mutations_rejected` is 3

### Requirement: Arbitration Telemetry Events
When mutation-path policy wiring is explicitly enabled, every Level 3 rejection and every Level 5 shed MUST emit a telemetry record with the arbitration level, error code, agent ID, mutation reference, and timestamp. Levels 2 (redaction) and 4 (attention budget) MUST emit telemetry at a lower rate (one record per session per minute when those outcomes are active).
Source: RFC 0009 §13.1
Scope: v1-reserved

#### Scenario: Rejection telemetry emitted
- **WHEN** mutation-path policy wiring is explicitly enabled and a mutation is rejected at Level 3 for capability denial
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

### Cross-Reference: Freeze and Safe Mode State Transitions
Freeze/safe-mode state transitions are governed exclusively by the system shell. See system-shell spec: "Requirement: Safe Mode and Freeze Interaction (Shell State Invariant)." Policy arbitration and evaluation MUST NOT write `OverrideState` or perform freeze/safe-mode override state transitions.

### Closeout Note: v1 Scope Shrink
Unified level-stack hot-path wiring is explicitly deferred to post-v1 (v2). Any future promotion of these deferred requirements MUST include fresh reconciliation evidence proving code/spec/doctrine alignment for that release train.

### Spec-First Handoff Notes
- `hud-iq2x.7`: define the runtime-policy-scene ownership matrix and the executable seam for PolicyContext/ArbitrationOutcome integration before re-promoting `v1-reserved` stack requirements to `v1-mandatory`.
- `hud-iq2x.8`: define capability-escalation source semantics (session grant set vs lease scope vs mid-session requests) before claiming Level 3 Security stack parity with this spec.
- `hud-1jd5`: runtime hot-path arbitration telemetry emission remains deferred until mutation-path policy wiring is explicitly enabled; pure evaluator events in `tze_hud_policy` are not by themselves runtime telemetry delivery evidence.

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
Scope: post-v1

#### Scenario: V1 diagnostics via telemetry only
- **WHEN** an operator needs per-agent arbitration statistics in v1
- **THEN** they must use telemetry records (no dedicated RPC)

---

## Appendix: Implementation Notes

This appendix collects tunable defaults and implementation guidance that inform but do not constitute normative requirements. Implementations MAY differ from these notes provided all normative requirements above are satisfied.

### Sub-Lookup Implementation Guidance

Individual policy sub-checks should be implemented using O(1) data structures to meet the normative latency bounds (capability < 5us, attention < 10us, zone contention < 20us):

- **Capability check** (Level 3): single hash-table lookup. Execute first since it is the fastest and most likely to reject early.
- **Attention budget check** (Level 4): rolling counter read over an in-memory circular buffer.
- **Zone contention resolution** (Level 6): policy dispatch via a small match/switch on the ContentionPolicy variant.

All three sub-checks together must comfortably fit within the 50us per-mutation budget; 35us of headroom should be reserved for remaining pipeline overhead.

### Freeze Duration Default

The max freeze duration is a configurable UX tuning parameter. Suggested default: 5 minutes. This value reflects a balance between protecting viewers from runaway freezes and allowing legitimate long-freeze workflows (e.g., presentation mode). Deployments with tighter UX requirements (kiosk, public display) should reduce this to 30–60 seconds. The auto-unfreeze mechanism is normative; the specific default timeout is not.
