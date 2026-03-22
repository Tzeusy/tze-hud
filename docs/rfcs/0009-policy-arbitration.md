# RFC 0009: Policy and Arbitration

**Status:** Draft
**Issue:** rig-em4
**Date:** 2026-03-22
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0006 (Configuration), RFC 0007 (System Shell)

---

## Summary

This RFC formalizes the tze_hud policy arbitration model: the single, authoritative specification of how the runtime resolves conflicts when multiple policy domains apply simultaneously. It defines the unified seven-step arbitration stack, the implementation contract for each step, cross-zone arbitration, and the degradation response model. It resolves two outstanding cross-RFC conflicts: the GPU failure response disagreement between RFC 0002 §7.3 and RFC 0007 §5.1, and the `redaction_style` ownership conflict between RFC 0006 `[chrome]` and RFC 0006 `[privacy]`.

The policy arbitration order is not a design choice — it is doctrine (architecture.md §"Policy arbitration"). This RFC gives that doctrine an implementation home.

---

## Motivation

The tze_hud doctrine names four policy sources — capabilities (security.md), privacy/attention (privacy.md), zone contention (presence.md), and degradation (failure.md) — and specifies a canonical priority order for resolving conflicts among them. That order is stated in architecture.md and referenced by name in RFC 0006 §5.4 and RFC 0007 §5.6, but it has no RFC of its own.

Without a formal specification:

- The arbitration order exists in doctrine but not in code contracts. Any implementation must infer the order from a cross-reference chain that spans four source documents.
- Two RFCs give conflicting answers to the GPU device loss question: RFC 0002 says terminate the process; RFC 0007 says enter safe mode. Both cannot be right.
- `redaction_style` is defined in both `[chrome]` and `[privacy]` configuration sections in RFC 0006. Implementations will disagree on which field governs.
- Cross-zone arbitration (when two agents publish to the same zone in the same frame, or when zone geometry overlaps) has no specified resolution procedure.
- The relationship between the degradation ladder (RFC 0002 §6) and the arbitration stack (architecture.md) is implicit.

This RFC resolves all of these by specifying the policy arbitration subsystem as a first-class component with defined contracts, typed inputs and outputs, and explicit conflict resolutions.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| Canonical arbitration order | §1: seven-step stack, implementation contract per step |
| Human override is unconditional | §2.1: override interrupts the stack at every step |
| GPU device loss response | §5: unified failure response; RFC 0002/0007 conflict resolved |
| `redaction_style` ownership | §3.2: canonical field location defined; RFC 0006 conflict resolved |
| Cross-zone arbitration | §4: per-zone contention policy plus cross-zone resolution procedure |
| Degradation interacts with arbitration | §6: degradation as the terminal arbitration gate |

---

## 1. The Arbitration Stack

### 1.1 Overview

Every agent action that modifies the visible scene — publishing to a zone, creating a tile, updating content, requesting a lease — passes through the arbitration stack. The stack is evaluated top-to-bottom. Each step either passes the action forward, transforms it, queues it, or rejects it. Once a step rejects an action, no lower step is evaluated.

```
┌─────────────────────────────────────────────────────────────────┐
│                    ARBITRATION STACK                            │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 1: Human Override                                   │  │
│  │  Unconditional. Freeze, dismiss, safe mode, mute.         │  │
│  │  Interrupts the stack at any point.                       │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ pass                               │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 2: Capability Gate                                  │  │
│  │  Does the agent have capability to publish here?          │  │
│  │  Reject immediately with structured error if not.         │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ pass                               │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 3: Privacy / Viewer Gate                            │  │
│  │  Does the viewer context permit this content?             │  │
│  │  Publish succeeds; rendering is redacted if not allowed.  │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ pass (possibly with redaction)     │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 4: Interruption Policy                              │  │
│  │  Is this interruption permitted right now?                │  │
│  │  Queue or pass based on quiet hours and class.            │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ pass or queue                      │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 5: Attention Budget                                 │  │
│  │  Has this agent or zone been interrupting too frequently? │  │
│  │  Coalesce or defer if budget is exhausted.                │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ pass or defer                      │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 6: Zone Contention                                  │  │
│  │  Does this publish conflict with existing zone occupancy? │  │
│  │  Apply contention policy (latest-wins, stack, merge, …).  │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ pass, replace, or stack            │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  STEP 7: Resource / Degradation Budget                    │  │
│  │  Does the runtime have capacity to render this?           │  │
│  │  Simplify, defer, or shed based on degradation level.     │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │ commit or shed                     │
│                         SCENE COMMIT                            │
└─────────────────────────────────────────────────────────────────┘
```

The stack is evaluated synchronously within the compositor thread's mutation intake stage (RFC 0002 §3.2 Stage 1). The result is a transformed `MutationBatch` — some mutations may be enriched with redaction flags, deferred to a queue, or dropped with a structured error.

### 1.2 Rust Contract

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
    /// condition clears (quiet hours end, attention budget refills).
    Queue {
        mutation: SceneMutation,
        queue_reason: QueueReason,
        earliest_present_us: Option<u64>,  // None = condition-dependent
    },
    /// Mutation is rejected. The agent receives a structured error.
    Reject(ArbitrationError),
    /// Mutation is shed by the degradation policy. No error is sent to the agent.
    /// The mutation's zone-state effects are applied (zone occupancy updates), but
    /// its render output is omitted for this frame. The agent is expected to back off
    /// on receiving a DegradationEvent; no retransmit is required — the runtime resumes
    /// rendering from last committed zone state when capacity recovers.
    /// See §6.2 for the distinction between zone-state update and render commit.
    Shed { degradation_level: u32 },
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

pub struct ArbitrationError {
    pub code: ArbitrationErrorCode,
    pub agent_id: AgentId,
    pub mutation_ref: SceneId,
    pub message: String,
    pub hint: Option<String>,
}

pub enum ArbitrationErrorCode {
    /// Step 2: agent lacks publish capability for this zone or tile.
    CapabilityDenied,
    /// Step 2: agent lacks capability to create/modify this resource type.
    CapabilityScopeInsufficient,
    /// Step 6: zone has a Replace contention policy and was occupied by an
    /// agent with higher lease priority. The publishing agent is outranked.
    ZoneEvictionDenied,
    /// Step 2: agent attempted to modify a resource outside its namespace.
    NamespaceViolation,
}
```

---

## 2. Step-by-Step Implementation

### 2.1 Step 1 — Human Override

Human override is not a gate in the normal sense. It is an asynchronous interrupt that can preempt any point in the arbitration pipeline, including mid-commit.

Override actions (dismiss tile, revoke lease, freeze scene, safe mode entry, mute media) are initiated from the main thread's input drain (RFC 0002 §2.2) and handled before any agent mutation intake for that frame. They are never queued behind agent mutations.

**Implementation contract:**
- Override commands are placed in a dedicated `OverrideCommandQueue` (bounded, capacity: 16, single-producer/single-consumer) read by the compositor thread at the top of each frame's Stage 1 intake before the `MutationBatch` channel is drained.
- If an override command arrives during Stage 1 mutation intake, it preempts: all pending mutations in the current batch are held; the override is applied; then the held mutations are re-evaluated against the new state.
- Human override cannot be deferred, coalesced, or suppressed by any other policy step.
- The override action is complete within one frame (≤ 16.6ms from input event to visual effect), as specified by RFC 0007 §4 ("Frame-bounded response").

### 2.2 Step 2 — Capability Gate

The capability gate checks whether the agent's session holds the required capability scope for the requested operation.

**Check matrix:**

| Operation | Required capability |
|-----------|-------------------|
| Publish to a zone | `zone-publish:<zone_id>` or `zone-publish:*` |
| Create a tile | `create-tiles` |
| Modify own tiles | `modify-own-tiles` |
| Request overlay z-order | `high-priority-z-order` |
| Subscribe to scene events | `subscribe-scene-events` |
| Access input events | `access-input-events` |
| Stream media | `stream-media` |
| Read full topology | `topology-read` |

Capabilities are granted per-session at handshake time (RFC 0005 §2) and may be revoked mid-session (security.md §"Capability scopes"). Revocation is immediate: the next arbitration evaluation for the affected agent fails at Step 2 for any operation that required the revoked capability.

**Outcome:** `Reject(CapabilityDenied)` or `Reject(CapabilityScopeInsufficient)`. The agent receives a structured error with the missing capability scope named in the `hint` field.

### 2.3 Step 3 — Privacy / Viewer Gate

The privacy gate compares the content's declared `VisibilityClassification` against the current `ViewerClass`. This step does not reject the mutation — it decorates it with a `CommitRedacted` outcome so the compositor applies the redaction placeholder during the chrome pass.

**Evaluation:**

```
content_classification ∈ { Public, Household, Private, Sensitive }
viewer_class           ∈ { Owner, HouseholdMember, KnownGuest, Unknown, Nobody }

Access matrix (✓ = show content, ✗ = redact):

                 Public  Household  Private  Sensitive
Owner              ✓        ✓          ✓        ✓
HouseholdMember    ✓        ✓          ✗        ✗
KnownGuest         ✓        ✗          ✗        ✗
Unknown            ✓        ✗          ✗        ✗
Nobody             ✓        ✗          ✗        ✗
```

**Zone ceiling rule:** When an agent publishes to a zone, the effective classification is `max(agent_declared_classification, zone_default_classification)`. An agent cannot escalate visibility beyond the zone's ceiling — publishing `Public` content to a zone with a `Household` ceiling results in `Household` visibility (RFC 0001 §2.5).

**Multi-viewer rule:** When multiple viewers are present, the runtime applies the most restrictive viewer class across all present viewers (privacy.md §"Multi-viewer scenarios"). The `multi_viewer_policy` configuration field (RFC 0006 §7, `[privacy]`) governs this behavior. The owner can explicitly override the multi-viewer restriction via the privacy indicator control (RFC 0007 §6).

**Redaction rendering:** The redaction placeholder appearance — pattern, agent_name, or icon — is governed by `PrivacyConfig.redaction_style`. See §3.2 for the authoritative field location.

**Outcome:** `CommitRedacted` if access is denied, otherwise passes to Step 4.

### 2.4 Step 4 — Interruption Policy

The interruption gate checks whether the mutation's `InterruptionClass` is permitted given the current runtime conditions.

**Interruption classes** (from privacy.md):
- `Silent` — always passes through. No display disruption.
- `Gentle` — passes unless quiet hours are active.
- `Normal` — passes unless quiet hours are active.
- `Urgent` — passes through quiet hours. May be subject to attention budget (Step 5).
- `Critical` — always passes. Bypasses both quiet hours and attention budget.

**Quiet hours:** During quiet hours (configured in `[privacy.quiet_hours]`), interruptions below the `pass_through_class` threshold are queued. The runtime delivers queued mutations when quiet hours end (in FIFO order per zone). Queued mutations carry their original `InterruptionClass` — they are not reclassified on delivery.

**Outcome:** `Queue(QuietHours)` for below-threshold interruptions during quiet hours, otherwise passes to Step 5.

### 2.5 Step 5 — Attention Budget

The attention budget is a dynamic per-agent and per-zone interruption rate limit. It prevents agents from saturating the screen with frequent interruptions even when quiet hours are not active.

**Budget tracking:**
- Each agent session and each zone maintains a rolling interruption-rate counter: the number of non-silent interruptions in the last 60 seconds.
- The budget is configurable per deployment: `[privacy] max_interruptions_per_agent_per_minute` and `max_interruptions_per_zone_per_minute`.
- `Critical` interruptions are exempt from budget tracking.
- `Silent` updates are never counted (they are not interruptions).

**When budget is exhausted:**
- The mutation's interruption class is recorded but the visual effect is coalesced: the most recent queued content for this agent+zone key replaces any earlier queued content (latest-wins coalescing within the queue).
- The agent is not notified. The runtime coalesces silently; the agent observes only that its visible update rate has slowed.
- The budget refills continuously as time passes. There is no sharp reset.

**Outcome:** `Queue(AttentionBudgetExhausted)` if budget is exhausted, otherwise passes to Step 6.

### 2.6 Step 6 — Zone Contention

Zone contention resolution applies when a publish operation targets a zone that already has content from another source (another agent or a prior publish by the same agent).

This step applies the zone's `ContentionPolicy` (RFC 0001 §2.5). The contention policies are:

| Policy | Behavior | Used by |
|--------|----------|---------|
| `LatestWins` | New publish replaces previous content immediately | subtitle, ambient-background |
| `Stack` | New publish stacks on top; each entry auto-dismisses after `auto_clear_us` | notification |
| `MergeByKey` | Key-addressed; same key replaces, different keys coexist up to `max_keys` | status-bar |
| `Replace` | Single occupant; new publish evicts current occupant | pip (post-v1) |

**Cross-zone arbitration:** Zone geometry may overlap (e.g., a notification zone and a subtitle zone that both occupy the lower viewport). Overlapping zone content is composited in zone layer order (RFC 0001 §2.5 `ZoneLayerAttachment`): Background < Content < Chrome. Within the same layer, zones are composited in z-order (defined by zone configuration). The runtime does not attempt to merge or arbitrate content across zone boundaries — each zone resolves its own contention independently.

**Replace eviction rule:** For zones with `ContentionPolicy::Replace`, a new publish evicts the current occupant only if the new publish's agent session has equal or higher lease priority (numerically equal or lower priority value, following the convention where 0 = highest). If the incoming agent has lower priority than the current occupant, the publish is rejected with `ZoneEvictionDenied`. The current occupant is not notified of the attempted eviction.

**Outcome:** Content is committed, stacked, merged, or replaced per the zone's contention policy. `Reject(ZoneEvictionDenied)` for lower-priority Replace eviction attempts.

### 2.7 Step 7 — Resource / Degradation Budget

The final gate checks whether the runtime has sufficient resources to render the committed mutation at the current degradation level.

The degradation ladder is defined in RFC 0002 §6. The arbitration role at this step is to apply the degradation level's shed policy to pending mutations:

| Degradation level | Arbitration action |
|-------------------|--------------------|
| Normal (0) | All mutations commit. |
| Level 1 (Coalesce) | State-stream mutations coalesced; transactional mutations unaffected. |
| Level 2 (Reduce Texture Quality) | Texture-bearing mutations accepted; textures downscaled at render time. |
| Level 3 (Disable Transparency) | Alpha-blend mutations accepted; alpha forced to 1.0 at render time. |
| Level 4 (Shed Tiles) | New tile creation below a priority threshold is shed. Existing tiles below threshold are removed from the render pass but their leases remain valid. |
| Level 5 (Emergency) | Only the chrome layer and the single highest-priority tile are rendered. All other new tile mutations are shed. |

**Shed semantics:** A `Shed` outcome does not produce an error to the agent. The mutation is discarded. The agent should observe the `DegradationEvent` subscription notification (RFC 0002 §6.4) and back off proactively. There is no retransmit obligation — the agent is expected to re-publish when conditions improve.

**Transactional mutations are never shed.** `CreateTile`, `DeleteTile`, `LeaseRequest`, and `LeaseRelease` mutations are transactional (RFC 0005 §5.1) and are never dropped at Step 7. Only state-stream and ephemeral mutations are subject to shedding.

---

## 3. Policy Ownership: Resolved Conflicts

### 3.1 Conflict Register

This section documents cross-RFC policy ownership conflicts and their resolutions. Each entry is binding: implementations must follow the resolution, not the conflicting prior text.

### 3.2 `redaction_style` — RFC 0006 `[chrome]` vs. `[privacy]`

**Conflict:** RFC 0006 defines `redaction_style` in two configuration sections:
- `[chrome]` (RFC 0006 §2.8): `redaction_style = "pattern"` with comment "Redaction placeholder style."
- `[privacy]` (RFC 0006 §7 `[privacy]`): `redaction_style = "pattern"` with the same valid values.

Two fields for the same concept in the same config file will cause implementations to disagree on which one governs and whether they must agree.

**Resolution:** `redaction_style` is a **privacy policy field**. It belongs exclusively in `[privacy]`. The `[chrome]` entry is a duplication error introduced during review and must be removed.

**Rationale:** Redaction is a privacy mechanism (privacy.md §"Redaction behavior") — it is the rendering expression of a privacy policy decision. The chrome layer renders the redaction placeholder, but the *style* of that placeholder is a privacy configuration concern, not a chrome layout concern. Chrome configuration governs display structure (tab bar position, indicator visibility, override control display). Privacy configuration governs content visibility policy (viewer classes, quiet hours, content classification, redaction appearance).

**Required change to RFC 0006:** Remove `redaction_style` from the `[chrome]` section (§2.8). The authoritative field is:

```toml
[privacy]
# Redaction placeholder style. One of: "pattern", "agent_name", "icon", "blank".
# Default: "pattern"
redaction_style = "pattern"
```

The `ChromeConfig` Rust struct must not contain a `redaction_style` field. The `PrivacyConfig` Rust struct is the canonical owner.

**Hot-reload:** `redaction_style` is hot-reloadable as part of `[privacy]` (RFC 0006 §9 "Hot-reloadable fields"). The prior listing of `[chrome]` as hot-reloadable for "redaction style" is superseded by this resolution.

---

## 4. Cross-Zone Arbitration

### 4.1 Within-Zone Resolution

Each zone resolves its own contention independently using its `ContentionPolicy`. This is defined per-zone in the zone registry (RFC 0001 §2.5) and enforced at Step 6 of the arbitration stack.

### 4.2 Between-Zone Overlap

When zone geometries overlap, the runtime composites them in a defined order without attempting content arbitration across zone boundaries:

1. Background zones render first (behind all agent tiles).
2. Content zones render in their assigned z-order among agent tiles. Content-layer zones are pinned at a z-order above all agent-controlled z-order values.
3. Chrome zones render last, above all content. Agents publish data to chrome zones; the runtime renders it.

Within each layer, overlapping zones are composited back-to-front in ascending z-order (lower z-order value = further back). Zone z-order is set in zone configuration and is not controllable by agents at publish time.

**There is no cross-zone eviction.** An agent cannot evict another agent's content from a different zone by publishing to its own zone. Zone boundaries are hard walls, not soft priority suggestions.

### 4.3 Same-Frame Contention

When two agents publish to the same zone in the same `MutationBatch` (or in two batches queued for the same frame), the arbitration step for that zone is evaluated in the order mutations are received by the compositor thread's intake stage. The first mutation is applied; subsequent mutations are evaluated against the post-first-mutation zone state. This produces deterministic behavior under the zone's contention policy.

**Example (Stack zone):** Agent A publishes notification X; Agent B publishes notification Y in the same frame. X is evaluated first (arrives first), then Y. Both are stacked because `Stack` policy accumulates entries. Result: both X and Y are visible in the notification stack.

**Example (LatestWins zone):** Agent A publishes subtitle text X; Agent B publishes subtitle text Y in the same frame. X is applied first, then Y evicts X via LatestWins. Result: only Y is visible.

Agents have no mechanism to control intra-frame ordering. If ordering matters, agents must use separate frames or coordinate via a shared orchestrator.

### 4.4 Cross-Tab Zone Isolation

Zones are scoped to the tab in which they are defined (RFC 0001 §2.4). An agent publishing to `tab_a/subtitle` does not interact with `tab_b/subtitle`, even if both zones are the same type with the same contention policy. Cross-tab zone arbitration does not exist: zones on inactive tabs are not rendered and their contention state is inactive.

---

## 5. GPU Failure Response — RFC 0002 vs. RFC 0007 Conflict Resolution

### 5.1 Conflict Statement

RFC 0002 §7.3 and RFC 0007 §5.1 give incompatible responses to GPU device loss:

- **RFC 0002 §7.3:** "If reconfiguration fails (device truly lost): trigger graceful shutdown (§1.4) with non-zero exit code."
- **RFC 0007 §5.1:** "Automatic entry on critical runtime error: If the compositor detects a condition that would otherwise produce a blank or unresponsive screen — scene graph corruption, GPU device loss, unrecoverable render failure — it enters safe mode rather than crashing."

RFC 0002 says: process exits. RFC 0007 says: enter safe mode (process continues). These are mutually exclusive for the "device truly lost" case.

### 5.2 Resolution

**The RFC 0007 behavior is correct for partial/recoverable failures. RFC 0002 is correct for total GPU loss. The difference is GPU recovery attempt outcome.**

The unified GPU failure response is a two-phase procedure:

**Phase 1 — Recovery attempt (RFC 0002 §7.3, steps 1–3 unchanged):**
1. Compositor thread detects `SurfaceError::Lost` or `SurfaceError::Outdated`.
2. Flush telemetry with a `gpu_surface_lost` error event.
3. Attempt surface reconfiguration (`surface.configure(device, &config)`).
   - If reconfiguration succeeds: resume normally. No safe mode entry required.

**Phase 2 — Safe mode before shutdown (RFC 0007 §5.1 governs):**
If reconfiguration fails (device truly lost — `wgpu::DeviceError::Lost` or adapter becomes invalid):
1. **Enter safe mode** (RFC 0007 §5.2): suspend all agent sessions, display the safe mode overlay.
   - This replaces RFC 0002 §7.3 step 4 ("trigger graceful shutdown"). The intent of safe mode entry here is to inform the viewer that the display has failed before the process exits, rather than producing a silent blank screen or sudden disappearance.
   - If the safe mode overlay cannot render because the GPU is already unusable (i.e., `ChromeState` cannot be committed to the frame), skip to step 2 immediately.
2. **Emit `SafeModeEntryEvent`** with `reason = CRITICAL_ERROR` (RFC 0007 §7.3 `SafeModeEntryReason`).
3. **Wait up to 2 seconds** for the safe mode overlay to render (one frame budget × headroom). If the frame pipeline is not responsive within this window, skip forward.
4. **Trigger graceful shutdown** (RFC 0002 §1.4) with non-zero exit code.

**Rationale:** RFC 0007's safe mode behavior was designed precisely for this scenario: "a condition that would otherwise produce a blank or unresponsive screen." GPU device loss is the canonical example. A silent process exit leaves the viewer with no explanation. Safe mode entry before shutdown provides a one-frame acknowledgement that the runtime is terminating intentionally. The shutdown still happens — RFC 0002 is correct that a truly lost device cannot be recovered — but RFC 0007's user-facing contract is honored.

### 5.3 Required Changes

**RFC 0002 §7.3** must be updated. Replace the current step 4:

> ~~4. If reconfiguration fails (device truly lost): trigger graceful shutdown (§1.4) with non-zero exit code.~~

With:

> 4. If reconfiguration fails (device truly lost): enter safe mode (RFC 0007 §5.1, `CRITICAL_ERROR` reason) to inform the viewer before process exit. If safe mode overlay renders within 2 seconds, display it briefly; then trigger graceful shutdown (§1.4) with non-zero exit code. If the overlay cannot render (GPU already unusable), skip directly to graceful shutdown.

**RFC 0007 §5.1** requires no text change; its specification already correctly describes automatic safe mode entry on GPU device loss. The addition of the subsequent shutdown step (which §5.1 did not address) is captured above.

---

## 6. Arbitration and the Degradation Ladder

### 6.1 Relationship

The degradation ladder (RFC 0002 §6) and the arbitration stack are complementary, not competing:

- The **arbitration stack** evaluates individual mutations from agents. It answers the question: "Should this content appear, and in what form?"
- The **degradation ladder** governs the runtime's overall rendering capacity. It answers the question: "How much can the runtime render right now?"

Degradation level is an input to Step 7 of the arbitration stack. The degradation ladder transitions (RFC 0002 §6.3) happen on the compositor thread between frames and are visible to Step 7 on the next frame.

### 6.2 Degradation Does Not Bypass Arbitration

A mutation shed at Step 7 is shed, not bypassed. It has already passed Steps 1–6. This means:
- A shed mutation was capability-checked (Step 2).
- Its privacy policy was evaluated (Step 3).
- Its interruption class was evaluated (Step 4).
- Its zone contention was resolved (Step 6) — the zone state was updated as if the mutation committed, then the render was omitted.

The distinction matters for zone state correctness: the zone's occupancy is updated even for shed render mutations. A LatestWins zone where the latest publish was shed at Step 7 still has its zone state updated to reflect that publish; it simply isn't rendered this frame.

**Exception:** Tile shed at degradation Level 4 or 5 means the tile is excluded from the render pass but remains in the scene graph with its current content state. When the degradation level recovers, the tile's last committed content resumes rendering without any agent re-publish.

### 6.3 DegradationEvent and Agent Backpressure

When the runtime transitions to degradation Level 1 or higher, it emits a `DegradationEvent` to all subscribed agents (RFC 0002 §6.4). Agents that receive this event should reduce their update rate voluntarily. The arbitration stack does not require them to do so — it enforces the degradation policy independently — but voluntary backpressure reduces the amount of work that must be shed at Step 7.

Agents that ignore `DegradationEvent` and continue publishing at their normal rate will have a higher fraction of their state-stream mutations coalesced or shed. Transactional mutations from those agents are never shed (§2.7), so lease and tile management operations remain reliable even under degradation.

---

## 7. Audit and Observability

### 7.1 Arbitration Telemetry

Every Step 2 rejection and every Step 7 shed emits a telemetry record:

```json
{
  "event": "arbitration_reject",
  "step": 2,
  "code": "CAPABILITY_DENIED",
  "agent_id": "agent-abc",
  "mutation_ref": "mut-xyz",
  "capability_required": "zone-publish:notification",
  "timestamp_us": 1234567890000
}
```

```json
{
  "event": "arbitration_shed",
  "step": 7,
  "agent_id": "agent-abc",
  "mutation_ref": "mut-xyz",
  "degradation_level": 4,
  "mutation_traffic_class": "state_stream",
  "timestamp_us": 1234567890000
}
```

Steps 3–5 (redaction, queue, attention budget) emit telemetry at a lower rate — one record per session per minute when those outcomes are active — to avoid flooding the telemetry stream during sustained redaction or quiet-hour queueing.

### 7.2 Per-Agent Arbitration Summary

The `RuntimeService.GetSessionDiagnostics` RPC (post-v1) will expose per-agent arbitration statistics: reject counts by code, shed counts by level, queue depths. In v1, these are available only via telemetry.

### 7.3 Capability Grant Audit

Every capability grant and revocation is logged (security.md §"Capability scopes"):

```json
{
  "event": "capability_grant",
  "agent_id": "agent-abc",
  "capability": "zone-publish:notification",
  "granted_at_us": 1234567890000,
  "granted_by": "session_handshake"
}
```

```json
{
  "event": "capability_revoke",
  "agent_id": "agent-abc",
  "capability": "zone-publish:notification",
  "revoked_at_us": 1234567891000,
  "reason": "admin_action"
}
```

---

## 8. Protobuf Schema

The following types are new or extend existing schemas.

```protobuf
// ArbitrationError is included in MutationBatchResult (extends RFC 0005 §3.1).
message ArbitrationError {
  ArbitrationErrorCode code    = 1;
  string               agent_id       = 2;
  string               mutation_ref   = 3;  // batch_id from MutationBatch
  string               message        = 4;
  optional string      hint           = 5;
}

enum ArbitrationErrorCode {
  ARBITRATION_ERROR_CODE_UNSPECIFIED    = 0;
  CAPABILITY_DENIED                     = 1;
  CAPABILITY_SCOPE_INSUFFICIENT         = 2;
  ZONE_EVICTION_DENIED                  = 3;
  NAMESPACE_VIOLATION                   = 4;
}

// DegradationShedEvent is emitted per-mutation when shed at Step 7.
// Distinct from DegradationEvent (RFC 0002 §6.4) which is a level-change event.
message DegradationShedEvent {
  string mutation_ref        = 1;
  uint32 degradation_level   = 2;
  string traffic_class       = 3;  // "state_stream" | "ephemeral" (transactional mutations are never shed; see §2.7)
  uint64 timestamp_us        = 4;
}

// AttentionBudgetEvent: emitted when an agent's attention budget is exhausted.
// Agents may use this to reduce their interrupt rate proactively.
message AttentionBudgetEvent {
  string  agent_id          = 1;
  string  zone_id           = 2;  // empty if per-agent budget (not zone-specific)
  uint32  interruptions_per_minute_actual  = 3;
  uint32  interruptions_per_minute_budget  = 4;
  uint64  timestamp_us      = 5;
}
```

---

## 9. Cross-RFC Interaction Table

| RFC | Interaction |
|-----|-------------|
| RFC 0001 (Scene Contract) | Zone `ContentionPolicy` types are the input to Step 6. Zone `ZoneLayerAttachment` governs cross-zone compositing order (§4.2). `VisibilityClassification` is the privacy gate input (Step 3). |
| RFC 0002 (Runtime Kernel) | Degradation ladder level is the Step 7 input. RFC 0002 §7.3 GPU failure response is superseded by §5.2 of this RFC. Arbitration runs within RFC 0002 Stage 1 (mutation intake). |
| RFC 0005 (Session Protocol) | Capability grants and revocations (Step 2 inputs) are established at session handshake (RFC 0005 §2). `ArbitrationError` is added to `MutationBatchResult`. Transactional message guarantees (RFC 0005 §5.1) take precedence over Step 7 shed policy. |
| RFC 0006 (Configuration) | `[privacy].redaction_style` is the authoritative configuration field (§3.2). `[privacy].quiet_hours` and `[privacy].max_interruptions_*` configure Steps 4 and 5. The `[chrome].redaction_style` field is removed. |
| RFC 0007 (System Shell) | Human override (Step 1) is implemented by RFC 0007 §4. Safe mode entry (RFC 0007 §5.1) is the GPU failure response for partial failure before shutdown (§5.2). Redaction placeholder rendering is the chrome pass described in RFC 0007 §3.4; `redaction_style` in `PrivacyConfig` governs its appearance. |

---

## 10. Open Questions

1. **Attention budget defaults.** The doctrine states that attention budget is a real constraint (attention.md) but does not quantify it. This RFC defers the specific default values for `max_interruptions_per_agent_per_minute` and `max_interruptions_per_zone_per_minute` to the Configuration RFC revision. A reasonable starting point: 6 per agent per minute (one per 10 seconds), 12 per zone per minute (allowing multiple agents to publish to the same zone at moderate rates).

2. **Step 7 shed notification.** Currently agents are not notified when a mutation is shed. A `DegradationShedEvent` subscription category could let agents observe their shed rate and back off. This is deferred to post-v1 to keep the subscription surface minimal.

3. **Attention budget persistence across quiet hours.** If an agent exhausted its attention budget at 21:55 and quiet hours begin at 22:00, do queued mutations that were attention-budget-deferred also get queued under the quiet-hours queue, or do they expire? The current spec says queues are independent. This interaction should be tested explicitly in the `policy_arbitration_collision` test scene (validation.md §"Test scene registry").

4. **Cross-tab zone state.** §4.4 states that zones on inactive tabs have inactive contention state. The behavior when a tab becomes active and a zone on that tab has pending content from before it was last visible is unspecified. This is deferred to the Scene Contract RFC revision.
