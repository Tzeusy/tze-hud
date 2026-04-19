# RFC 0009: Policy and Arbitration

**Status:** Draft
**Issue:** rig-em4
**Date:** 2026-03-23
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0006 (Configuration), RFC 0007 (System Shell), RFC 0008 (Lease Governance)

---

## Summary

This RFC provides the single authoritative arbitration stack for all policy decisions in tze_hud. Policy is currently scattered across multiple doctrine files and RFCs with unresolved conflicts. This document consolidates everything into a formal precedence hierarchy with seven levels (0-6), defines conflict resolution rules for both cross-level and within-level disputes, specifies the policy evaluation pipeline for per-frame, per-event, and per-mutation paths, and resolves six outstanding cross-RFC contradictions.

The arbitration stack is not a design choice -- it is doctrine (architecture.md, "Policy arbitration"). This RFC gives that doctrine an implementation home with quantitative contracts and typed interfaces.

---

## Motivation

tze_hud doctrine names four policy sources -- capabilities (security.md), privacy/attention (privacy.md), zone contention (presence.md), and degradation (failure.md) -- and specifies a canonical priority order for resolving conflicts. That order is stated in architecture.md and referenced by RFC 0006 and RFC 0007, but it has no formal specification of its own.

Without a formal specification:

- Implementations must infer the arbitration order from a cross-reference chain spanning four doctrine files and six RFCs.
- Two RFCs give conflicting GPU failure responses: RFC 0002 says exit; RFC 0007 says safe mode.
- `redaction_style` is defined in both `[chrome]` and `[privacy]` in RFC 0006.
- Freeze semantics are spread across RFC 0007 and RFC 0002 with no unified override model.
- The capability vocabulary is split across RFC 0005 and RFC 0006 with naming mismatches.
- No quantitative budgets exist for policy evaluation itself.

This RFC resolves all of these.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| Canonical arbitration order | Section 1: seven-level stack with formal precedence |
| Conflict resolution rules | Section 2: cross-level and within-level resolution |
| Policy evaluation pipeline | Section 3: per-frame, per-event, per-mutation paths |
| GPU failure response | Section 4: three-tier failure arbitration (resolves RFC 0002 vs. RFC 0007) |
| `redaction_style` ownership | Section 5: privacy owns all redaction decisions |
| Freeze semantics | Section 6: Level 0 action with backpressure signals |
| Override semantics | Section 7: suppress/redirect/transform/block taxonomy |
| Capability registry | Section 8: canonical vocabulary |
| Quantitative requirements | Section 9: latency budgets for policy evaluation |
| Cross-RFC resolutions | Section 10: six contradictions resolved |

---

## 1. The Arbitration Stack

### 1.1 Formal Precedence Hierarchy

The arbitration stack is a fixed priority order with seven levels, numbered 0 (highest) to 6 (lowest). Higher levels always win. This is doctrine.

```
ARBITRATION STACK -- FORMAL PRECEDENCE

Level 0  HUMAN OVERRIDE       [HIGHEST]
  |  Dismiss, safe mode, freeze, mute.
  |  Local, instant, cannot be intercepted/delayed/vetoed.
  |
Level 1  SAFETY
  |  Safe mode (automatic), critical error recovery, GPU failure response.
  |  Degradation ladder activation at emergency thresholds.
  |
Level 2  PRIVACY
  |  Viewer context changes, content redaction, classification enforcement.
  |  Multi-viewer restriction. Redaction style ownership.
  |
Level 3  SECURITY
  |  Capability enforcement, lease validity, agent isolation.
  |  Namespace boundary enforcement. Session authentication.
  |
Level 4  ATTENTION
  |  Interruption classification, quiet hours, attention budget.
  |  Per-agent and per-zone interrupt rate limiting.
  |
Level 5  RESOURCE
  |  Budget enforcement, degradation ladder, tile shedding.
  |  Per-agent envelope limits. Frame-time guardian.
  |
Level 6  CONTENT                [LOWEST]
     Zone contention, agent priority, z-order.
     ContentionPolicy application. Cross-zone compositing.
```

### 1.2 Doctrine Citations

Each level is grounded in specific doctrine passages:

| Level | Doctrine source | Key passage |
|-------|----------------|-------------|
| 0 | security.md, "Human override" | "The human is always the ultimate authority. No agent, regardless of trust level or capability scope, can prevent the human from: dismissing any tile or overlay, revoking any lease, terminating any agent session, muting any media stream, freezing the scene, entering a 'safe mode' that disconnects all agents. These overrides are handled locally by the runtime, not routed through an agent. They cannot be intercepted, delayed, or vetoed." |
| 1 | failure.md, "Core principle" | "The runtime must always be usable, even when agents are not. No agent failure -- crash, hang, disconnect, misbehavior -- should make the screen unresponsive, blank, or stuck." |
| 2 | privacy.md, "Viewer context" | "The runtime must own this decision, not individual agents. An agent that shows a calendar with meeting details does not know who is standing in front of the screen. The runtime does." |
| 3 | security.md, "Capability scopes" | "Capabilities are granted per-session, not per-agent-type. An agent that was trusted yesterday can be restricted today. Capabilities are additive, not subtractive." |
| 4 | attention.md, "Attention Budget" | "Every screen has finite attention capacity. Interruptions are withdrawals from that budget. A screen that interrupts constantly -- even with accurate, useful information -- becomes noise." |
| 5 | failure.md, "Degradation axes" / security.md, "Resource governance" | "The runtime monitors resource consumption in real time. If an agent exceeds its budget: warning, throttle, revocation." |
| 6 | architecture.md, "Policy arbitration", step 6 | "Zone contention. Does this publish conflict with existing zone occupancy? Apply the zone's contention policy." |

### 1.3 Rust Contract

```rust
/// The result of passing a single mutation through the arbitration stack.
pub enum ArbitrationOutcome {
    /// Mutation is accepted and will be committed to the scene on the next frame.
    Commit(SceneMutation),

    /// Mutation is accepted but its rendered output will be replaced with a
    /// redaction placeholder. The mutation is committed; rendering is filtered.
    CommitRedacted {
        mutation: SceneMutation,
        redaction_reason: RedactionReason,
    },

    /// Mutation is accepted but its presentation is deferred until the queue
    /// condition clears (quiet hours end, attention budget refills, freeze ends).
    Queue {
        mutation: SceneMutation,
        queue_reason: QueueReason,
        earliest_present_us: Option<u64>,  // None = condition-dependent
    },

    /// Mutation is rejected. The agent receives a structured error.
    Reject(ArbitrationError),

    /// Mutation is shed by resource/degradation policy. No error to the agent.
    /// Zone-state effects are applied but render output is omitted.
    Shed { degradation_level: u32 },

    /// Mutation is blocked by human override (freeze). Queued for later.
    Blocked {
        mutation: SceneMutation,
        block_reason: BlockReason,
    },
}

pub enum RedactionReason {
    /// Content classification exceeds current viewer access level.
    ViewerClassInsufficient {
        required: VisibilityClassification,
        actual: ViewerClass,
    },
    /// Multi-viewer policy applied most-restrictive rule.
    MultiViewerRestriction,
}

pub enum QueueReason {
    QuietHours { window_end_us: Option<u64> },
    AttentionBudgetExhausted { per_agent: bool, per_zone: bool },
}

pub enum BlockReason {
    /// Scene is frozen by human override. Mutations queue until unfreeze.
    Freeze,
}

pub struct ArbitrationError {
    pub code: ArbitrationErrorCode,
    pub agent_id: AgentId,
    pub mutation_ref: SceneId,
    pub message: String,
    pub hint: Option<String>,
    pub level: u8,  // Which arbitration level rejected (0-6)
}

pub enum ArbitrationErrorCode {
    // Level 3: Security
    CapabilityDenied,
    CapabilityScopeInsufficient,
    NamespaceViolation,
    LeaseInvalid,
    // Level 6: Content
    ZoneEvictionDenied,
}
```

---

## 2. Conflict Resolution Rules

### 2.1 Cross-Level Conflicts

When two policies at different levels conflict, the resolution is absolute:

**Rule CL-1: Higher level always wins.** There are no exceptions. A Level 2 (Privacy) decision cannot be overridden by a Level 5 (Resource) optimization or a Level 6 (Content) contention policy.

**Rule CL-2: Side effects of the losing level are suppressed, not deferred.** When a higher level blocks an action, the lower level's side effects do not fire. The lower level is not evaluated at all (short-circuit).

**Rule CL-3: The winning level's override type applies.** See Section 7 for the override type taxonomy (suppress, redirect, transform, block).

**Cross-level conflict decision table:**

| Scenario | Winning level | Losing level | Resolution |
|----------|--------------|-------------|------------|
| Privacy says "redact tile" but Content says "show tile" | Level 2 (Privacy) | Level 6 (Content) | Tile is redacted. Content contention result is irrelevant. |
| Human says "freeze" but Resource says "shed tile" | Level 0 (Human Override) | Level 5 (Resource) | Tile stays frozen. Degradation is paused. Tile is not shed. |
| Safety enters safe mode but Attention has queued notifications | Level 1 (Safety) | Level 4 (Attention) | Queued notifications are discarded. Safe mode overrides. |
| Security denies capability but Resource would allow the budget | Level 3 (Security) | Level 5 (Resource) | Mutation rejected at Level 3. Resource check never runs. |
| Privacy redacts but Attention would have queued for quiet hours | Level 2 (Privacy) | Level 4 (Attention) | Mutation is committed with redaction. Quiet hours evaluation still runs for presentation scheduling. |
| Resource sheds tile but Content had assigned z-order | Level 5 (Resource) | Level 6 (Content) | Tile is shed. Z-order assignment is committed to scene state but not rendered. |
| Human dismisses tile but agent has valid lease | Level 0 (Human Override) | Level 3 (Security) | Lease is revoked immediately. Capability validity is irrelevant. |

### 2.2 Within-Level Conflicts

When two policies at the same level conflict, level-specific tie-breaking rules apply:

**Level 0 (Human Override):** Multiple simultaneous overrides are applied in input-event order. If the viewer triggers freeze and dismiss-all in the same frame, the input that arrived first is processed first.

**Level 1 (Safety):** When multiple safety triggers fire simultaneously (e.g., GPU failure and scene corruption detected in the same frame), the most severe response wins. Severity order: catastrophic exit > safe mode entry > GPU reconfiguration attempt.

**Level 2 (Privacy):** When multiple viewer contexts conflict (multi-viewer scenario), the most restrictive viewer class wins (privacy.md, "Multi-viewer scenarios"). Restriction order: Nobody > Unknown > KnownGuest > HouseholdMember > Owner.

**Level 3 (Security):** When multiple capability checks apply to a single mutation, all must pass. Security is conjunctive -- a single failing check rejects the mutation. There is no "most permissive wins" rule.

**Level 4 (Attention):** When both quiet hours and attention budget would queue an interruption, the longer deferral wins. If quiet hours end at 08:00 and the attention budget refills in 30 seconds, the interruption is queued until 08:00.

**Level 5 (Resource):** When per-agent budget enforcement and frame-time guardian both trigger, the frame-time guardian takes precedence (it affects all agents, not just one). Per-agent throttling applies within the frame-time guardian's decisions.

**Level 6 (Content):** When two agents publish to the same zone in the same frame, the zone's `ContentionPolicy` resolves: LatestWins uses arrival order, Stack accumulates, MergeByKey uses key identity, Replace uses lease priority (RFC 0008 Section 2.2: lower numeric priority value wins).

---

## 3. Policy Evaluation Pipeline

### 3.1 Evaluation Paths

The arbitration stack is not evaluated monolithically. Three evaluation paths exist, each with a different subset of levels and a different trigger:

```
PER-FRAME EVALUATION (every 16.6ms at 60fps)
  Level 1: Safety    -> check GPU health, frame-time guardian
  Level 2: Privacy   -> check viewer context changes, apply redaction
  Level 5: Resource  -> degradation ladder evaluation
  Level 6: Content   -> zone occupancy timeout/auto-clear

PER-EVENT EVALUATION (on input events)
  Level 0: Human Override  -> freeze, dismiss, safe mode, mute
  Level 4: Attention       -> interruption class for event-triggered content
  Level 3: Security        -> input routing capability check

PER-MUTATION EVALUATION (on each agent MutationBatch)
  Level 3: Security   -> capability gate, lease validity, namespace check
  Level 5: Resource   -> per-agent budget check, degradation shed policy
  Level 6: Content    -> zone contention resolution
```

### 3.2 Per-Frame Evaluation

The per-frame evaluation runs on the compositor thread at the start of each frame cycle, before mutation intake. It evaluates levels that depend on runtime state rather than individual agent actions.

**Order:** Safety (1) -> Privacy (2) -> Resource (5) -> Content (6)

1. **Safety check.** Query GPU device health. If `wgpu::DeviceError::Lost` or frame-time guardian emergency threshold exceeded, trigger Level 1 response (see Section 4).
2. **Privacy check.** If `ViewerClass` has changed since last frame (viewer identification pipeline produced a new result), apply redaction transitions to all affected tiles. Redaction transitions are immediate -- no animation, no delay.
3. **Resource check.** Evaluate `frame_time_p95` over the rolling 10-frame window. If threshold exceeded, transition degradation level (RFC 0002 Section 6). If recovery threshold met over 30-frame window, recover one level.
4. **Content maintenance.** Evaluate zone auto-clear timeouts. Remove expired zone publications.

**Short-circuit rule:** If Level 1 triggers safe mode entry, Levels 2/5/6 are not evaluated for that frame. Safe mode suspends all normal rendering.

### 3.3 Per-Event Evaluation

The per-event evaluation runs on the main thread during Stage 1 (Input Drain) and Stage 2 (Local Feedback) of the frame pipeline (RFC 0002 Section 3.2).

**Order:** Human Override (0) -> Attention (4) -> Security (3)

1. **Human Override check.** Override commands (dismiss, freeze, safe mode, mute) are recognized and placed in the `OverrideCommandQueue` (bounded, capacity: 16, SPSC). Override commands are delivered to the compositor thread before any `MutationBatch` intake for that frame.
2. **Attention check.** For input events that trigger agent-visible content changes (e.g., tab switch events configured in `tab_switch_on_event`), evaluate the interruption class. This check determines whether the resulting scene change is permitted under current quiet hours and attention budget.
3. **Security check.** Input events are routed only to agents with `access_input_events` capability. Input destined for agents without this capability is silently dropped.

**Short-circuit rule:** If Level 0 triggers safe mode, Level 4 and Level 3 event processing stops. All input routes to the chrome layer exclusively.

### 3.4 Per-Mutation Evaluation

The per-mutation evaluation runs on the compositor thread during Stage 3 (Mutation Intake) and Stage 4 (Scene Commit). Every mutation in every `MutationBatch` passes through this path.

**Order for zone publications (full stack):** Human Override (0, via `OverrideCommandQueue` preemption) -> Security (3) -> Privacy (2, redaction decoration) -> Attention (4, quiet hours / budget check) -> Resource (5, degradation shed) -> Content (6, zone contention)

**Order for tile mutations:** Security (3) -> Resource (5) -> Content (6)

1. **Override preemption.** If override commands are pending in the `OverrideCommandQueue`, process them first. All pending mutations in the current batch are held; the override is applied; then the held mutations are re-evaluated against the new state.
2. **Security gate.** Does the agent's session hold the required capability? Is the lease valid? Is the mutation within the agent's namespace? If any check fails: `Reject(ArbitrationError)`.
3. **Privacy decoration.** Compare content classification against current viewer class. If access is denied: the mutation is committed but marked `CommitRedacted`. The agent is not informed of redaction.
4. **Attention gate.** Evaluate interruption class against quiet hours and attention budget. If below threshold during quiet hours: `Queue(QuietHours)`. If attention budget exhausted: `Queue(AttentionBudgetExhausted)`. Critical interruptions bypass both.
5. **Resource gate.** Check per-agent budget (tiles, texture bytes, update rate). Check degradation level shed policy. If budget exceeded: reject the batch. If degradation level sheds this mutation's priority class: `Shed`.
6. **Content resolution.** Apply zone's `ContentionPolicy`. For Replace zones with priority conflict: `Reject(ZoneEvictionDenied)`.

**Short-circuit rule:** If a higher level rejects or blocks, lower levels are not evaluated. A mutation rejected at Level 3 (Security) never reaches Level 5 (Resource) or Level 6 (Content).

---

## 4. Failure Arbitration

This section defines the authoritative failure response tiers. It **resolves** the conflict between RFC 0002 Section 1.4/Section 7.3 (exit on GPU loss) and RFC 0007 Section 5.1 (safe mode on GPU loss).

### 4.1 Three-Tier Failure Response

| Tier | Condition | Response | Governing RFC |
|------|-----------|----------|---------------|
| Tier 1: Recoverable GPU error | `SurfaceError::Lost` or `SurfaceError::Outdated` with successful reconfiguration | Attempt `surface.configure(device, &config)`. If success, continue. If fail, escalate to Tier 2. | RFC 0002 Section 7.3, steps 1-3 |
| Tier 2: Non-recoverable GPU, CPU intact | `wgpu::DeviceError::Lost`, adapter invalid, reconfiguration failed | Enter safe mode (RFC 0007 Section 5.1). Leases suspended (not revoked). Chrome renders in software fallback. Emit `SafeModeEntryEvent` with `reason = CRITICAL_ERROR`. Wait up to 2 seconds for overlay to render. Then escalate to Tier 3. | RFC 0007 Section 5.1, this RFC |
| Tier 3: Catastrophic | GPU unusable (safe mode overlay cannot render), or Tier 2 timeout elapsed | Flush telemetry (200ms grace). Trigger graceful shutdown (RFC 0002 Section 1.4) with non-zero exit code. | RFC 0002 Section 1.4 |

### 4.2 Tier Interaction with Arbitration Levels

Failure tiers interact with the arbitration stack at Level 1 (Safety):

- **Tier 1** does not interrupt normal arbitration. The surface reconfiguration is transparent to agents.
- **Tier 2** triggers safe mode, which suspends all leases (RFC 0008 Section 3.4) and rejects all agent mutations with `SAFE_MODE_ACTIVE`. The arbitration stack is effectively bypassed -- only Level 0 (Human Override: the viewer pressing Resume) and Level 1 (Safety: the safe mode overlay itself) are active.
- **Tier 3** terminates the process. No arbitration is possible.

### 4.3 Resolution Statement

**RFC 0002 Section 1.4 (exit) applies only at Tier 3.** The process exits only after Tier 2 safe mode has been attempted and either displayed or timed out.

**RFC 0007 Section 5.1 (safe mode) applies at Tier 2.** Safe mode is entered before shutdown to inform the viewer. It is not a permanent recovery -- it is a brief acknowledgement before exit.

**Required change to RFC 0002 Section 7.3 step 4** (already applied by prior review): "If reconfiguration fails (device truly lost): enter safe mode (RFC 0007 Section 5.1, `CRITICAL_ERROR` reason) to inform the viewer before process exit. If safe mode overlay renders within 2 seconds, display it briefly; then trigger graceful shutdown (Section 1.4) with non-zero exit code. If the overlay cannot render (GPU already unusable), skip directly to graceful shutdown."

---

## 5. Redaction Ownership

### 5.1 Principle

**The Privacy level (Level 2) owns ALL redaction decisions.** This is a single-owner rule with no exceptions.

### 5.2 Canonical Configuration Location

The `[privacy]` config section is the single source of truth for `redaction_style`:

```toml
[privacy]
# Redaction placeholder style. One of: "pattern", "agent_name", "icon", "blank".
# Default: "pattern"
redaction_style = "pattern"
```

The `ChromeConfig` Rust struct (`[chrome]` config section) MUST NOT contain a `redaction_style` field. Any reference to `redaction_style` in the `[chrome]` section is a duplication error.

### 5.3 Rendering Responsibility

The chrome layer renders the redaction visual (RFC 0007 Section 3.4), but it does not decide what to redact. The decision flow is:

1. **Privacy level decides** (Level 2): the per-frame privacy evaluation compares each tile's `VisibilityClassification` against the current `ViewerClass` and produces a redaction flag.
2. **Chrome renders** (compositor Stage 6): the chrome render pass reads the redaction flag and draws the placeholder pattern specified by `PrivacyConfig.redaction_style`.

The chrome layer is the renderer, not the arbiter. This separation ensures that redaction policy changes (viewer context transitions, multi-viewer rules) are evaluated at Level 2 and rendered by the chrome pass without the chrome layer needing to understand privacy policy.

### 5.4 Resolution Statement

**This resolves rig-zeb:** The `[chrome]` section references `[privacy]` for `redaction_style` and does not define its own. RFC 0006 Section 2.8 `ChromeConfig` TOML example must not contain `redaction_style`. This was applied in RFC 0006 Round 3 review.

---

## 6. Freeze Semantics

### 6.1 Classification

Freeze is a **Level 0 (Human Override)** action. It shares Level 0 with dismiss, safe mode, and mute. It is local, instant, and cannot be intercepted, delayed, or vetoed by any agent or lower policy level.

### 6.2 Behavior During Freeze

| Aspect | Behavior |
|--------|----------|
| Agent mutations | Queued in bounded per-session queue (1000 mutations, configurable). NOT rejected. |
| Resource budgets | Paused. The degradation ladder does not advance during freeze. The frame-time guardian does not evaluate. |
| Attention signals | Deferred. Quiet hours timers pause. Attention budget counters freeze. |
| Tile rendering | Frozen at last committed state. Badges continue to render (a frozen tile can still show a disconnection badge). |
| Input | Override controls remain active. Agent input routing is suspended. |

### 6.3 Agent Notification Model

**Agents receive a generic backpressure signal, not a freeze notification.** This is the decision documented in rig-9v1.

- Agents are NOT told about freeze specifically. The viewer's decision to freeze is viewer state and must not be exposed to agents (privacy.md, "Agent isolation").
- At 80% queue capacity (800/1000 by default): the runtime sends `MUTATION_QUEUE_PRESSURE` via `RuntimeError` in `MutationResult`. This signal fires for any queue-pressure scenario -- freeze, slow compositor, degradation, contention -- not specifically for freeze.
- On overflow: `MUTATION_DROPPED` for each shed mutation. Transactional mutations are never shed; they apply gRPC backpressure instead.
- Agents that receive `MUTATION_QUEUE_PRESSURE` may voluntarily reduce submission rate. The freeze executes unconditionally regardless.

### 6.4 Freeze Duration

- **Max freeze duration:** Configurable, default 5 minutes.
- On timeout: auto-unfreeze with `DegradationNotice` advisory to all agents indicating conditions may have changed.
- The auto-unfreeze timeout is a safety net, not a policy decision. It prevents an accidentally frozen display from remaining indefinitely unresponsive.

### 6.5 Freeze and Safe Mode Interaction

Per RFC 0007 Section 5.6:

- **Freeze active, safe mode triggered:** Safe mode wins (Level 1 overrides). The freeze state is cancelled. The freeze queue is discarded. `OverrideState.freeze_active` is set to `false`.
- **Freeze attempted during safe mode:** Ignored. Safe mode captures all input.
- **After safe mode exit:** Freeze is inactive. The viewer must re-trigger freeze if desired.

### 6.6 Resolution Statement

**This resolves rig-9v1:** Freeze is silent with backpressure signals. The `MUTATION_QUEUE_PRESSURE` signal is operationally useful without being a viewer-state leak.

---

## 7. Override Semantics by Level

### 7.1 Override Type Taxonomy

Each arbitration level uses specific override types when it prevails over a lower level:

| Override type | Definition | Effect on mutation |
|---------------|-----------|-------------------|
| **Suppress** | Action prevented entirely. | Mutation rejected with structured error. Agent is informed. |
| **Redirect** | Action rerouted to a different target. | Input routed to chrome instead of agent tile. |
| **Transform** | Action modified before commit. | Content committed but rendered with redaction placeholder. |
| **Block** | Action queued for later delivery. | Mutation stored in bounded queue; delivered when condition clears. |

### 7.2 Override Types by Level

| Level | Override types used | Examples |
|-------|-------------------|---------|
| 0: Human Override | Suppress, Redirect, Block | Dismiss = suppress (tile removed). Safe mode = redirect (all input to chrome). Freeze = block (mutations queued). Mute = suppress (media silenced). |
| 1: Safety | Suppress, Redirect | Safe mode auto-entry = redirect (all input to chrome, mutations rejected with `SAFE_MODE_ACTIVE`). GPU reconfiguration = suppress (rendering paused, mutations held). |
| 2: Privacy | Transform | Redaction = transform (mutation committed, rendering replaced with placeholder). Agent is not informed. |
| 3: Security | Suppress | Capability denied = suppress. Namespace violation = suppress. Lease invalid = suppress. All produce structured errors. |
| 4: Attention | Block | Quiet hours = block (mutations queued until window ends). Attention budget exhausted = block (mutations queued with coalescing). |
| 5: Resource | Suppress, Transform | Budget exceeded = suppress (batch rejected). Degradation shed = suppress (mutation discarded, zone state updated). Texture downscale = transform (texture accepted at reduced resolution). |
| 6: Content | Suppress | Zone eviction denied = suppress (lower-priority agent cannot evict higher-priority occupant in Replace zone). |

### 7.3 Override Composition

When multiple levels each want to apply an override, the highest level's override type takes precedence:

- If Level 0 blocks (freeze) and Level 5 would suppress (budget), the mutation is blocked (freeze wins).
- If Level 2 transforms (redact) and Level 4 would block (quiet hours), the mutation is both transformed and blocked: it is queued with a redaction flag, and when delivered, it renders with the placeholder.
- If Level 3 suppresses (capability denied), no lower level is evaluated. The mutation is rejected immediately.

---

## 8. Capability Registry

### 8.1 Canonical Vocabulary

This table is THE authority for capability identifiers. Both RFC 0005 (session handshake) and RFC 0006 (configuration) reference this list. All identifiers use `snake_case` with colon-separated sub-scopes for parameterized grants.

| Identifier | Description | Default grant | Scope | Authoritative source |
|------------|-------------|--------------|-------|---------------------|
| `create_tiles` | May request tile leases. | Not granted | Session | RFC 0005, RFC 0008 Section 3.3 |
| `modify_own_tiles` | May mutate content on tiles owned by this session. | Not granted | Session | RFC 0005 |
| `read_scene_topology` | May query the full scene topology, including other agents' lease metadata. Without this, agent sees only its own leases and public structure. | Not granted | Session | RFC 0005 Section 7.1 |
| `subscribe_scene_events` | May subscribe to the scene-event bus (system events, topology events, agent-emittable named events per RFC 0006 Section 5.5). NOT input events. | Not granted | Session | RFC 0005 Section 7.1 |
| `access_input_events` | May receive pointer, touch, keyboard, and gesture input events forwarded from the runtime (RFC 0004). NOT scene-event bus. | Not granted | Session | RFC 0005 Section 7.1 |
| `overlay_privileges` | May request tiles with overlay-level z-order positions. | Not granted | Session | RFC 0005 |
| `high_priority_z_order` | May request z-order values in the top quartile. | Not granted | Session | RFC 0005 |
| `exceed_default_budgets` | May request budget overrides at session time. Requires user prompt. | Not granted | Session | RFC 0005 |
| `read_telemetry` | May subscribe to `telemetry_frames` events (runtime performance samples). | Not granted | Session | RFC 0005 Section 7.1 |
| `stream_media` | May negotiate WebRTC media sessions. | Not granted | Session | RFC 0005 (post-v1) |
| `resident_mcp` | May use resident-level MCP tools (`create_tab`, `create_tile`, `set_content`, `dismiss`). Without this, only guest tools (`publish_to_zone`, `list_zones`, restricted `list_scene`). | Not granted | Session | RFC 0005 Section 8.3 |
| `publish_zone:<zone_name>` | May publish to the named zone instance. One grant per zone. | Not granted | Zone instance | RFC 0005 Section 7.1 |
| `publish_zone:*` | May publish to all zones. Wildcard form. | Not granted | All zones | RFC 0005 Section 7.1 |
| `emit_scene_event:<event_name>` | May fire the named scene event on the scene-event bus. The `<event_name>` must follow `<source>.<action>` naming (RFC 0006 Section 5.5). Must not use `system.` or `scene.` prefix. | Not granted | Named event | RFC 0005 |
| `lease:priority:<N>` | May request lease priority N or lower (0=Critical, 1=High, 2=Standard, 3=Low, 4=Speculative). | `lease:priority:2` | Lease | RFC 0008 Section 2.1 |

### 8.2 Naming Convention

- All identifiers: `snake_case`.
- Sub-scoped identifiers: colon-separated (`publish_zone:notification`).
- Wildcard: only `publish_zone:*` is supported. No other capability supports wildcards.
- Older uppercase forms (`CREATE_TILE`, `WRITE_SCENE`) and kebab-case forms (`create-tiles`, `zone-publish`) appearing in RFC 0001 diagrams and earlier drafts of this RFC are superseded. The `snake_case` forms in Section 8.1 are canonical.

### 8.3 Resolution Statement

**This resolves rig-vbi (capability name format inconsistency).** RFC 0005 Section 7.1 and RFC 0006 Section 6.3 both reference this table as authoritative. The `snake_case` convention is established by RFC 0005 as the wire-format standard. Earlier RFC 0009 drafts used kebab-case; those are incorrect. This table supersedes all prior capability name listings.

---

## 9. Quantitative Requirements

### 9.1 Policy Evaluation Latency Budgets

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Full per-frame evaluation (Levels 1, 2, 5, 6) | < 200us | Must fit within Stage 3 mutation intake budget (RFC 0002: p99 < 1ms) with headroom for mutation processing. |
| Per-mutation policy check (Levels 3, 5, 6) | < 50us | A frame with 64 mutations (max batch) at 50us each = 3.2ms; fits within the combined Stage 3+4 budget of 2ms. |
| Human override response (Level 0) | < 1 frame (16.6ms) | Override visual effect must appear within one frame of the input event (RFC 0007 Section 4.5: "Frame-bounded response"). |
| Privacy transition (viewer context change, Level 2) | < 2 frames (33.2ms) | Viewer identification pipeline produces result; redaction transitions apply on the next frame; visual update on the frame after. Two-frame budget allows the identification result to arrive mid-frame. |
| Capability check (Level 3, single capability) | < 5us | Capability checks are hash-table lookups against the session's granted set. Must not dominate per-mutation budget. |
| Attention budget check (Level 4) | < 10us | Rolling counter comparison against configured thresholds. |
| Zone contention resolution (Level 6) | < 20us | ContentionPolicy application for a single zone. LatestWins and MergeByKey are O(1); Stack is O(log n) for insertion. |

### 9.2 Telemetry Integration

Policy evaluation latency is tracked in the per-frame `TelemetryRecord`:

```rust
pub struct PolicyTelemetry {
    pub per_frame_eval_us: u32,          // Total per-frame evaluation time
    pub per_mutation_eval_us_p99: u32,   // p99 per-mutation policy check (this frame)
    pub mutations_rejected: u32,          // Count of Level 3 rejections this frame
    pub mutations_redacted: u32,          // Count of Level 2 redactions this frame
    pub mutations_queued: u32,            // Count of Level 4 queued this frame
    pub mutations_shed: u32,              // Count of Level 5 shed this frame
    pub override_commands_processed: u32, // Count of Level 0 overrides this frame
}
```

---

## 10. Cross-RFC Resolutions

This section lists every cross-RFC contradiction resolved by this RFC. Each entry is binding: implementations must follow the resolution.

### 10.1 rig-nev: Arbitration Order Canonicalization

**Conflict:** Architecture.md defines a 7-step policy evaluation order. RFC 0002 defines a 5-level degradation ladder. RFC 0007 defines override semantics. No single document specifies how these interact or which takes precedence when they disagree.

**Resolution:** This RFC (Sections 1-3) provides the unified 7-level arbitration stack with formal precedence, conflict resolution rules, and three evaluation pipelines. Architecture.md's 7-step order is faithfully mapped to Levels 0-6 with the addition of the Safety level (Level 1) which was implicit in the doctrine but not explicitly named. The degradation ladder (RFC 0002 Section 6) is a mechanism within Level 5 (Resource). Override controls (RFC 0007 Section 4) are mechanisms within Level 0 (Human Override).

### 10.2 rig-81i: GPU Failure Response Contradiction

**Conflict:** RFC 0002 Section 7.3 step 4 says "trigger graceful shutdown" on GPU device loss. RFC 0007 Section 5.1 says "enter safe mode" on GPU device loss. Both cannot be correct for the same failure condition.

**Resolution:** Section 4 of this RFC defines a three-tier failure response. Tier 2 (non-recoverable GPU, CPU intact) enters safe mode (RFC 0007 is correct) for up to 2 seconds, then triggers graceful shutdown (RFC 0002 is correct). Both RFCs are correct for different phases of the failure response. RFC 0002 Section 7.3 has been updated to reference this resolution.

### 10.3 rig-tew: Degradation Ladder vs. Arbitration Stack

**Conflict:** RFC 0002 Section 6 defines a degradation ladder that sheds tiles by priority, but the arbitration stack in architecture.md places degradation at step 7 (after zone contention). This creates ambiguity about whether shed decisions respect zone contention results.

**Resolution:** Section 3.4 of this RFC specifies that degradation shedding (Level 5) operates after Content resolution (Level 6) in the per-mutation evaluation path. A shed mutation has already had its zone contention resolved -- the zone state is updated, but the render output is omitted. Tile shedding at the per-frame level (frame-time guardian) operates on already-committed scene state and sheds lowest-priority tiles from the render pass without modifying zone state.

### 10.4 rig-zeb: Redaction Style Dual Ownership

**Conflict:** RFC 0006 defined `redaction_style` in both `[chrome]` (Section 2.8) and `[privacy]` (Section 7). Two config fields for the same concept causes implementation disagreement.

**Resolution:** Section 5 of this RFC assigns exclusive ownership to `[privacy]`. The `[chrome]` entry was a duplication error. RFC 0006 Round 3 review removed `redaction_style` from `ChromeConfig`. The `PrivacyConfig` Rust struct is the canonical owner.

### 10.5 rig-9v1: Freeze Agent Notification

**Conflict:** RFC 0007 Section 4.3 originally stated "agents are not informed that the scene is frozen." This is sovereignty-pure but wasteful -- agents generate mutations that will be queued and eventually dropped. The question was whether to send an advisory (which leaks viewer state) or a backpressure signal (which does not).

**Resolution:** Section 6 of this RFC specifies silent freeze with backpressure signal. The runtime sends `MUTATION_QUEUE_PRESSURE` at 80% queue capacity and `MUTATION_DROPPED` on overflow. These signals do not reveal freeze specifically -- they fire for any queue-pressure scenario. RFC 0007 Section 4.3 has been updated with this model.

### 10.6 rig-vbi: Capability Name Format Inconsistency

**Conflict:** RFC 0005 and RFC 0006 Section 6.3 use `snake_case` capability names (`create_tiles`, `publish_zone`). Earlier drafts of RFC 0009 used kebab-case (`create-tiles`, `zone-publish`). RFC 0001 diagrams use uppercase (`CREATE_TILE`). Three naming conventions for the same identifiers.

**Resolution:** Section 8 of this RFC establishes the canonical capability registry. All identifiers use `snake_case` as defined by RFC 0005 (wire-format authority). Kebab-case and uppercase forms are superseded and must not be used in new code or documentation.

---

## 11. Step-by-Step Implementation Detail

### 11.1 Level 0 -- Human Override

Human override is not a gate in the normal sense. It is an asynchronous interrupt that can preempt any point in the arbitration pipeline, including mid-commit.

Override actions (dismiss tile, revoke lease, freeze scene, safe mode entry, mute media) are initiated from the main thread's input drain (RFC 0002 Section 2.2) and handled before any agent mutation intake for that frame. They are never queued behind agent mutations.

**Implementation contract:**

- Override commands are placed in a dedicated `OverrideCommandQueue` (bounded, capacity: 16, SPSC) read by the compositor thread at the top of each frame's Stage 3 intake before the `MutationBatch` channel is drained.
- If an override command arrives during Stage 3 mutation intake, it preempts: all pending mutations in the current batch are held; the override is applied; then the held mutations are re-evaluated against the new state.
- Human override cannot be deferred, coalesced, or suppressed by any other policy level.
- The override action is complete within one frame (16.6ms), as specified by RFC 0007 Section 4.5.

### 11.2 Level 1 -- Safety

The safety level monitors runtime health and triggers protective responses when the system is at risk of becoming unresponsive.

**Triggers:**

| Trigger | Response | Threshold |
|---------|----------|-----------|
| GPU device lost (unrecoverable) | Tier 2 failure response (Section 4) | `wgpu::DeviceError::Lost` |
| Scene graph corruption | Safe mode entry | Invariant check failure in Stage 4 |
| Frame-time guardian emergency | Degradation Level 5 (emergency) | `frame_time_p95 > 14ms` sustained through all degradation levels |

**Interaction with Level 0:** If the viewer has frozen the scene and a safety trigger fires, safety wins. Freeze is cancelled and safe mode is entered (RFC 0007 Section 5.6).

### 11.3 Level 2 -- Privacy

The privacy level compares content classification against viewer context and produces redaction decisions.

**Access matrix:**

```
                 Public  Household  Private  Sensitive
Owner              SHOW     SHOW      SHOW      SHOW
HouseholdMember    SHOW     SHOW    REDACT    REDACT
KnownGuest         SHOW   REDACT    REDACT    REDACT
Unknown            SHOW   REDACT    REDACT    REDACT
Nobody             SHOW   REDACT    REDACT    REDACT
```

**Zone ceiling rule:** The effective classification of a zone publication is `max(agent_declared_classification, zone_default_classification)`. An agent cannot escalate visibility beyond the zone's ceiling.

**Multi-viewer rule:** When multiple viewers are present, the most restrictive viewer class applies (privacy.md, "Multi-viewer scenarios"). The owner can override this via the privacy indicator control (RFC 0007 Section 6).

### 11.4 Level 3 -- Security

The security level enforces capability scopes, lease validity, and namespace isolation.

**Check matrix (per Section 8.1 capability registry):**

| Operation | Required capability |
|-----------|-------------------|
| Publish to a zone | `publish_zone:<zone_name>` or `publish_zone:*` |
| Create a tile | `create_tiles` |
| Modify own tiles | `modify_own_tiles` |
| Request overlay z-order | `overlay_privileges` |
| Request high z-order | `high_priority_z_order` |
| Subscribe to scene events | `subscribe_scene_events` |
| Access input events | `access_input_events` |
| Stream media | `stream_media` |
| Read full topology | `read_scene_topology` |
| Read telemetry | `read_telemetry` |
| Use resident MCP tools | `resident_mcp` |
| Request lease priority N | `lease:priority:<N>` |
| Emit scene event | `emit_scene_event:<event_name>` |

Capabilities are granted per-session at handshake (RFC 0005 Section 1.3) and may be revoked mid-session. Revocation is immediate: the next arbitration evaluation fails at Level 3.

**Outcome:** `Reject(CapabilityDenied)`, `Reject(CapabilityScopeInsufficient)`, or `Reject(NamespaceViolation)` with the missing capability named in the `hint` field.

### 11.5 Level 4 -- Attention

The attention level manages interruption flow to prevent notification fatigue and respect quiet hours.

**Interruption classes** (RFC 0010 §3.2):
- `SILENT` -- always passes. No display disruption. Not counted against budget.
- `LOW` -- batched/deferred. Counted against budget. Blocked during quiet hours.
- `NORMAL` -- standard agent activity. Counted against budget. Blocked during quiet hours if configured.
- `HIGH` -- passes through quiet hours. Subject to attention budget.
- `CRITICAL` -- always passes. Bypasses both quiet hours and attention budget.

**Quiet hours:** During quiet hours, `LOW` and `NORMAL` interruptions are queued. `SILENT` continues (invisible by definition). `HIGH` and `CRITICAL` pass through. Queued mutations are delivered when quiet hours end, in FIFO order per zone.

**Attention budget:** Rolling per-agent and per-zone counters track non-silent interruptions over the last 60 seconds. Configurable limits: `max_interruptions_per_agent_per_minute` (default: 20) and `max_interruptions_per_zone_per_minute` (default: 10). When exhausted, mutations are coalesced (latest-wins within agent+zone key). Budget refills continuously. `CRITICAL` interruptions are exempt.

### 11.6 Level 5 -- Resource

The resource level enforces per-agent budgets and applies the degradation ladder.

**Per-agent budget enforcement** (RFC 0002 Section 5): three-tier ladder (Warning -> Throttle -> Revocation). Budget checks happen in Stage 3 before scene modification. Over-budget batches are rejected atomically.

**Degradation ladder** (RFC 0002 Section 6): five levels (Coalesce -> Reduce Texture Quality -> Disable Transparency -> Shed Tiles -> Emergency). Degradation level is an input to this arbitration step.

**Shed semantics:** A `Shed` outcome does not produce an error. Zone state is updated but rendering is omitted. Transactional mutations (`CreateTile`, `DeleteTile`, `LeaseRequest`, `LeaseRelease`) are never shed.

### 11.7 Level 6 -- Content

The content level resolves zone contention and agent priority.

**Contention policies** (RFC 0001 Section 2.5):

| Policy | Behavior | Used by |
|--------|----------|---------|
| `LatestWins` | New publish replaces previous content | subtitle, ambient-background |
| `Stack` | New publish stacks; each entry auto-dismisses after `auto_clear_us` | notification |
| `MergeByKey` | Same key replaces; different keys coexist up to `max_keys` | status-bar |
| `Replace` | Single occupant; eviction by equal-or-higher lease priority | pip (post-v1) |

**Cross-zone arbitration:** Zone geometry overlaps are resolved by compositing order (Background < Content < Chrome). Within a layer, zones are composited in ascending z-order. There is no cross-zone eviction.

**Same-frame contention:** When two agents publish to the same zone in the same frame, mutations are evaluated in arrival order. The first is applied; subsequent mutations are evaluated against the post-first-mutation zone state.

**Cross-tab zone isolation:** Zones are scoped to the tab in which they are defined (RFC 0001 Section 2.4). An agent publishing to `tab_a/subtitle` does not interact with `tab_b/subtitle`. Cross-tab zone arbitration does not exist.

---

## 12. Arbitration and the Degradation Ladder

### 12.1 Relationship

The degradation ladder (RFC 0002 Section 6) and the arbitration stack are complementary, not competing:

- The **arbitration stack** evaluates individual mutations from agents. It answers: "Should this content appear, and in what form?"
- The **degradation ladder** governs the runtime's overall rendering capacity. It answers: "How much can the runtime render right now?"

Degradation level is an input to Level 5 of the arbitration stack. The degradation ladder transitions (RFC 0002 Section 6.3) happen on the compositor thread between frames and are visible to Level 5 on the next frame.

### 12.2 Degradation Does Not Bypass Arbitration

A mutation shed at Level 5 is shed, not bypassed. It has already passed Levels 3 and higher. This means:
- A shed mutation was capability-checked (Level 3).
- Its privacy policy was evaluated (Level 2).
- Its interruption class was evaluated (Level 4).
- Its zone contention was resolved (Level 6) -- the zone state was updated as if the mutation committed, then the render was omitted.

The distinction matters for zone state correctness: the zone's occupancy is updated even for shed render mutations. A LatestWins zone where the latest publish was shed at Level 5 still has its zone state updated to reflect that publish; it simply is not rendered this frame.

**Exception:** Tile shed at degradation Level 4 or 5 means the tile is excluded from the render pass but remains in the scene graph with its current content state. When the degradation level recovers, the tile's last committed content resumes rendering without any agent re-publish.

### 12.3 DegradationEvent and Agent Backpressure

When the runtime transitions to degradation Level 1 or higher, it emits a `DegradationEvent` to all subscribed agents (RFC 0002 Section 6.4). Agents that receive this event should reduce their update rate voluntarily. The arbitration stack does not require them to do so -- it enforces the degradation policy independently -- but voluntary backpressure reduces the amount of work that must be shed at Level 5.

---

## 13. Audit and Observability

### 13.1 Arbitration Telemetry

Every Level 3 rejection and every Level 5 shed emits a telemetry record:

```json
{
  "event": "arbitration_reject",
  "level": 3,
  "code": "CAPABILITY_DENIED",
  "agent_id": "agent-abc",
  "mutation_ref": "mut-xyz",
  "capability_required": "publish_zone:notification",
  "timestamp_us": 1234567890000
}
```

```json
{
  "event": "arbitration_shed",
  "level": 5,
  "agent_id": "agent-abc",
  "mutation_ref": "mut-xyz",
  "degradation_level": 4,
  "mutation_traffic_class": "state_stream",
  "timestamp_us": 1234567890000
}
```

Levels 2 (redaction) and 4 (attention budget) emit telemetry at a lower rate -- one record per session per minute when those outcomes are active -- to avoid flooding the telemetry stream during sustained redaction or quiet-hour queueing.

### 13.2 Per-Agent Arbitration Summary

The `RuntimeService.GetSessionDiagnostics` RPC (post-v1) will expose per-agent arbitration statistics: reject counts by code, shed counts by level, queue depths. In v1, these are available only via telemetry.

### 13.3 Capability Grant Audit

Every capability grant and revocation is logged (security.md, "Capability scopes"):

```json
{
  "event": "capability_grant",
  "agent_id": "agent-abc",
  "capability": "publish_zone:notification",
  "granted_at_us": 1234567890000,
  "granted_by": "session_handshake"
}
```

```json
{
  "event": "capability_revoke",
  "agent_id": "agent-abc",
  "capability": "publish_zone:notification",
  "revoked_at_us": 1234567891000,
  "reason": "admin_action"
}
```

---

## 14. Protobuf Schema

### 14.1 New Types

```protobuf
// ArbitrationError is included in MutationBatchResult (extends RFC 0005 Section 3.1).
message ArbitrationError {
  ArbitrationErrorCode code          = 1;
  string               agent_id     = 2;
  string               mutation_ref = 3;  // batch_id from MutationBatch
  string               message      = 4;
  optional string      hint         = 5;
  uint32               level        = 6;  // Arbitration level that rejected (0-6)
}

enum ArbitrationErrorCode {
  ARBITRATION_ERROR_CODE_UNSPECIFIED = 0;
  CAPABILITY_DENIED                 = 1;
  CAPABILITY_SCOPE_INSUFFICIENT     = 2;
  ZONE_EVICTION_DENIED              = 3;
  NAMESPACE_VIOLATION               = 4;
  LEASE_INVALID                     = 5;
}

// DegradationShedEvent is emitted per-mutation when shed at Level 5.
// Distinct from DegradationEvent (RFC 0002 Section 6.4) which is a level-change event.
message DegradationShedEvent {
  string mutation_ref      = 1;
  uint32 degradation_level = 2;
  string traffic_class     = 3;  // "state_stream" | "ephemeral"
  uint64 timestamp_us      = 4;
}

// AttentionBudgetEvent: emitted when an agent's attention budget is exhausted.
message AttentionBudgetEvent {
  string agent_id                           = 1;
  string zone_id                            = 2;  // empty if per-agent (not zone-specific)
  uint32 interruptions_per_minute_actual    = 3;
  uint32 interruptions_per_minute_budget    = 4;
  uint64 timestamp_us                       = 5;
}

// PolicyTelemetry: embedded in TelemetryRecord (RFC 0002 Section 3.2, Stage 8).
message PolicyTelemetry {
  uint32 per_frame_eval_us            = 1;
  uint32 per_mutation_eval_us_p99     = 2;
  uint32 mutations_rejected           = 3;
  uint32 mutations_redacted           = 4;
  uint32 mutations_queued             = 5;
  uint32 mutations_shed               = 6;
  uint32 override_commands_processed  = 7;
}
```

---

## 15. Cross-RFC Interaction Table

| RFC | Interaction |
|-----|-------------|
| RFC 0001 (Scene Contract) | Zone `ContentionPolicy` types are the input to Level 6. Zone `ZoneLayerAttachment` governs cross-zone compositing order (Section 11.7). `VisibilityClassification` is the Level 2 input. |
| RFC 0002 (Runtime Kernel) | Degradation ladder level is the Level 5 input. RFC 0002 Section 7.3 GPU failure response is resolved by Section 4 of this RFC. Arbitration runs within RFC 0002 Stage 3 (mutation intake). Frame-time guardian (RFC 0002 Section 5.2) is a Level 5 mechanism. |
| RFC 0005 (Session Protocol) | Capability grants and revocations (Level 3 inputs) are established at session handshake. `ArbitrationError` is added to `MutationBatchResult`. Transactional message guarantees (RFC 0005 Section 5.1) take precedence over Level 5 shed policy. Canonical capability names defined in Section 8.1 of this RFC. `MUTATION_QUEUE_PRESSURE` and `MUTATION_DROPPED` error codes added to `RuntimeError.ErrorCode`. |
| RFC 0006 (Configuration) | `[privacy].redaction_style` is the authoritative configuration field (Section 5). `[privacy].quiet_hours` and `[privacy].max_interruptions_*` configure Level 4. The `[chrome].redaction_style` field is removed. Capability identifiers in Section 6.3 of RFC 0006 reference Section 8.1 of this RFC. |
| RFC 0007 (System Shell) | Human override (Level 0) is implemented by RFC 0007 Section 4. Safe mode (RFC 0007 Section 5) implements Level 1 safety response. Freeze semantics (RFC 0007 Section 4.3) are governed by Section 6 of this RFC. Redaction placeholder rendering is the chrome pass described in RFC 0007 Section 3.4; `redaction_style` in `PrivacyConfig` governs its appearance. |
| RFC 0008 (Lease Governance) | Lease validity is a Level 3 security check. `lease:priority:<N>` capability defined in Section 8.1. Safe mode suspends leases (RFC 0008 Section 3.4, DR-LG7); this is a Level 1 safety action. Tile shedding at Level 5 leaves leases ACTIVE (rendering-only suppression). |
| RFC 0010 (Scene Events) | Scene-event bus capabilities (`subscribe_scene_events`, `emit_scene_event:<name>`) are Level 3 security checks defined in Section 8.1. |
| RFC 0011 (Resource Store) | Resource reference counting and lifecycle are Level 5 resource governance inputs. |

---

## 16. Open Questions

1. **Attention budget defaults.** The doctrine states attention budget is a real constraint but does not quantify it. Default values proposed in Section 11.5 (6 per agent per minute, 12 per zone per minute) are starting points subject to tuning.

2. **Level 5 shed notification.** Currently agents are not notified when a mutation is shed. A `DegradationShedEvent` subscription category could let agents observe their shed rate. Deferred to post-v1.

3. **Attention budget persistence across quiet hours.** If an agent exhausted its budget at 21:55 and quiet hours begin at 22:00, queues are independent. The attention budget queue holds budget-deferred mutations; the quiet hours queue holds time-deferred mutations. Both are delivered independently when their conditions clear.

4. **Cross-tab zone state.** Section 11.7 states zones on inactive tabs have inactive contention state. Behavior when a tab becomes active with pending zone content is deferred to the Scene Contract RFC revision.

5. **Per-frame privacy evaluation cost.** With many tiles (64+) and frequent viewer context changes, the per-frame privacy check could approach the 200us budget. An incremental dirty-flag approach (only re-evaluate tiles whose classification changed or when viewer class changes) would reduce this to O(changed_tiles) instead of O(all_tiles).

---

## Amendment 1 (2026-04-19): C12 Role-Based Operators

**Source:** v2 embodied-media-presence signoff packet, decision C12 (F29 mandated: RFC 0009 amendment required before implementation beads on this topic).
**Bead:** hud-ora8.1.12
**Cross-reference:** `openspec/changes/v2-embodied-media-presence/specs/identity-and-roles/` (capability spec hud-ora8.2.5, to be authored) owns role definitions, user-directory schema, and role-to-capability binding. This amendment establishes only the policy-arbitration impact of those roles.

---

### A1.1 Role Taxonomy

v2 introduces four operator roles. Roles are assigned to human principals who configure and operate the runtime installation, not to agents. Agents operate under capability grants; roles govern the operator's authority to issue and modify those grants.

| Role | Semantic | Scope |
|------|----------|-------|
| `owner` | Full authority over the installation. May grant any capability to any agent, override any policy, revoke any session, and configure the runtime unrestricted. Exactly one owner per installation in v2 (multi-owner federation is post-v2). | Installation |
| `admin` | Full operational authority short of federation and ownership transfer. May grant or revoke any non-owner-reserved capability, configure display policies, revoke sessions, and manage the user directory. Cannot promote another principal to `owner`. | Installation |
| `member` | Trusted household member. May interact with the display (touch, gesture, voice). May configure personal attention and privacy preferences within operator-defined bounds. Cannot grant or revoke agent capabilities or manage other users. | User-level |
| `guest` | Transient visitor. Interaction permitted if the operator enables guest interaction in config. No capability-management authority. Session scope is ephemeral; no persistent preference storage. | User-level |

**Identity boundary:** Role assignments are principal records in the operator identity store (not to be confused with agent session capability grants). An agent's capability grants are separate from the role of the human operator who configured them. An agent session does not "inherit" an operator's role.

---

### A1.2 Data Model

The following fields are added to the operator identity record. They are stored in the runtime's operator configuration and exposed for audit. All fields are federation-aware in structure but federation enforcement is **not implemented in v2** (v2 ships no federation sub-epic; federation cross-operator policy merge is deferred per signoff A5 / C12 note).

```rust
/// Operator principal record — stored in runtime identity store.
pub struct OperatorPrincipal {
    /// Stable identifier for this principal. Local UUID in v2;
    /// federation-aware DID reserved for post-v2 federation.
    pub id: PrincipalId,

    /// Human-readable display name.
    pub display_name: String,

    /// Role assigned to this principal.
    pub role: OperatorRole,

    /// Federation origin — set to `Local` in v2.
    /// Reserved for cross-operator federation (post-v2 only).
    pub origin: PrincipalOrigin,

    /// Device bindings for biometric/hardware authentication (post-v2).
    pub devices: Vec<DeviceBinding>,
}

pub enum OperatorRole {
    Owner,
    Admin,
    Member,
    Guest,
}

pub enum PrincipalOrigin {
    /// v2: all principals are local.
    Local,
    /// Post-v2: federated principal from another installation.
    /// Fields present in enum variant but federation enforcement is NOT active in v2.
    Federated { federation_id: String, remote_installation_id: String },
}
```

**v2 enforcement boundary:** `PrincipalOrigin::Federated` is defined in the data model to allow future federation without wire-breaking changes. In v2, the runtime rejects any principal record with `origin = Federated` at load time with a configuration error. Federation-aware fields in data structures ship in v2; federation *logic* does not.

---

### A1.3 Policy Arbitration Impact

Role-based operator authority intersects the arbitration stack at **Level 3 (Security)** and **Level 0 (Human Override)**. No new arbitration levels are added. Roles do not introduce a new stack level; they extend the principal-authority model within existing levels.

#### Level 3 — Capability Grant and Revocation Authority

Capability grants and revocations (Section 11.4) require an authorizing operator action. The table below governs which roles may perform which capability-management operations:

| Operation | Owner | Admin | Member | Guest |
|-----------|-------|-------|--------|-------|
| Grant any capability to an agent session | Yes | Yes | No | No |
| Revoke a capability from an agent session | Yes | Yes | No | No |
| Grant `overlay_privileges` or `high_priority_z_order` | Yes | Yes | No | No |
| Grant `exceed_default_budgets` (requires user prompt, RFC 0009 §8.1) | Yes | Yes | No | No |
| Grant `stream_media`, `resident_mcp` | Yes | Yes | No | No |
| Manage other principals' role assignments | Yes | Admin → Member/Guest only (targets must not be Owner or Admin) | No | No |
| Promote a principal to `owner` | Yes | No | No | No |

**Conjunctive check:** A capability grant is accepted at Level 3 only if (a) the requested capability is within the set the agent's profile may receive (not restricted by presence level or session type), AND (b) the authorizing operator principal holds a role with grant authority for that capability. Both checks must pass; neither alone is sufficient.

**Revocation:** Any operator with grant authority for a capability may also revoke it. Revocation is immediate: the next arbitration evaluation for the affected agent fails at Level 3 with `CapabilityDenied`.

#### Level 0 — Human Override Authority

Human override actions (dismiss, freeze, safe mode, mute, lease revocation) are performed at **Level 0** and are **not gated by operator role in v2**. Any person present at the screen who can activate the override controls (RFC 0007 Section 4) may execute a Level 0 action. The rationale is unchanged from the original doctrine (security.md, "Human override"): the human in front of the screen is always the ultimate authority and must never be locked out by a policy system.

**Operator role and override controls (v2 scope):** In v2, the override control surface (RFC 0007 Section 4) is always accessible to the present viewer regardless of role. Role-based restriction of the override UI is a post-v2 governance refinement and is explicitly out of v2 scope.

#### Level 3 — Lease Operations

Lease grant and revocation (RFC 0008) require operator authority for non-default priority leases. The role-to-lease-authority mapping follows the capability table above: `owner` and `admin` may grant `lease:priority:1` (High) leases to agent sessions. `lease:priority:0` (Critical) remains runtime-internal per RFC 0008 §2.1 and is not operator-grantable. `member` and `guest` principals have no lease-management authority.

For standard priority (`lease:priority:2`) and below, lease grants are governed purely by the agent's session capability grants; no additional operator role check is applied.

---

### A1.4 Audit Integration

Role-based operations are recorded in the capability grant audit log (Section 13.3). Two new event types are added:

```json
{
  "event": "role_grant",
  "principal_id": "principal-abc",
  "role": "admin",
  "granted_by": "principal-xyz",
  "granted_at_us": 1234567890000
}
```

```json
{
  "event": "role_revoke",
  "principal_id": "principal-abc",
  "role": "admin",
  "revoked_by": "principal-xyz",
  "revoked_at_us": 1234567891000,
  "reason": "admin_action"
}
```

Role changes are subject to the same retention and local append-only log policy as capability grant/revoke events (signoff C17: 90-day default, operator-configurable, daily rotation, schema versioned).

---

### A1.5 Cross-RFC Interaction Addendum

This amendment extends the cross-RFC interaction table (Section 15):

| RFC | Interaction |
|-----|-------------|
| RFC 0005 (Session Protocol) | Session handshake establishes capability grants; authorizing-principal role is validated at that point. `SessionInit` does not carry a role field — roles are a runtime identity store concern, not a per-session wire field. |
| RFC 0008 (Lease Governance) | Lease priority grants above Standard require operator role authority per A1.3. The `lease:priority:<N>` capability (Section 8.1) is the existing mechanism; this amendment specifies which roles may authorize its grant. |
| `openspec/changes/v2-embodied-media-presence/specs/identity-and-roles/` | The `identity-and-roles` capability spec (hud-ora8.2.5) is the authoritative source for role definitions, user-directory schema, role-to-capability binding tables, and the principal identity wire format. This amendment is the policy-arbitration surface of that spec; the full role model lives there. |

**v2 non-enforcement note:** Federation-aware `PrincipalOrigin::Federated` fields are defined but the runtime rejects federated principals at load time. Any federation policy enforcement (cross-operator role merge, federated capability delegation) is deferred to a post-v2 phase and requires a separate RFC amendment at that time.

*End of Amendment 1.*
