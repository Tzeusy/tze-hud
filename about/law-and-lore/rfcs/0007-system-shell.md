# RFC 0007: System Shell

**Status:** Draft
**Issue:** rig-5vq.10
**Date:** 2026-03-22
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0004 (Input Model)

---

## Review History

### Operator Diagnostics Overlay — Post-V1 Design Note (rig-1co)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-1co — RFC 0007: Add operator diagnostics overlay (post-v1 design note)

#### Changes Applied

- **[PROTO → ADDED]** §7.1: Added `diagnostics_active` (field 8) to `ChromeState`. Always false in v1. Set by a local runtime toggle (keyboard shortcut or config); never exposed to agents.
- **[DEFERRED → NOTED]** §9: Added explicit design note for the operator diagnostics overlay under "Deferred (Post-V1)". Covers activation, content (per-tile state and reason, frame timing, agent session ID), chrome-layer rendering, privacy constraint (ViewerClass enum allowed; viewer identity prohibited), and agent isolation guarantee.

---

### Privacy-Safe Capture Semantics (rig-i7f)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-i7f — Define privacy-safe capture semantics for redacted tiles

#### Changes Applied

- **[FEAT → ADDED]** §3.4.1: New subsection "Capture Safety: Redaction Strategy" defining two redaction strategies — overlay redaction (v1 default) and render-skip redaction (post-v1, activated when any capture surface is registered). Specifies two leakage vectors (pre-chrome-pass capture, texture export / debug paths), the activation rule (per-frame, driven by `capture_surface_active`), and the agent isolation guarantee.
- **[PROTO → ADDED]** §7.2: Added `RedactionMode` enum (`OVERLAY`, `RENDER_SKIP`) and `redaction_mode` field (field 5) to `TileBadgeState`. Field is only meaningful when `redaction_active = true`. Always `OVERLAY` in v1.
- **[PROTO → ADDED]** §7.1: Added `capture_surface_active` (field 7) to `ChromeState`. Set by the capture surface registry (not agents) when any capture/mirror/screenshot/recording surface is active. Always false in v1.
- **[DEFERRED → NOTED]** §9: Explicit deferral entry for render-skip activation with a MUST constraint that the v1 implementation preserve the architectural separation of content rendering and chrome rendering.

---

### Freeze Advisory Model Decision (rig-9v1)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-9v1

#### Decision: Silent freeze with backpressure signal

§4.3 previously stated that agents are not informed when the scene is frozen. This is the correct default, but it creates a resource waste problem: under high-throughput conditions, agents continue generating and submitting mutations that are silently queued (up to 1000 per session) and then dropped when the queue overflows.

**Tradeoff analysis:**

- **Silent freeze** (original): Sovereignty-pure. Agents cannot detect freeze. But wasteful — agents spend compute and bandwidth generating mutations that will never be applied.
- **Advisory freeze** (rejected): Sending a "scene is frozen" advisory would leak viewer action intent. The viewer's decision to freeze the scene is viewer state, and privacy.md §"Topology visibility" / §"Agent isolation" establishes that viewer state must not be exposed to agents. An explicit `SCENE_FROZEN` notice is therefore ruled out.
- **Silent freeze with backpressure signal** (adopted): Instead of revealing that the viewer froze the scene, the runtime uses the existing mutation response path to signal general queue pressure. This does not reveal the *cause* of pressure — freeze is one possible cause, but so is a slow compositor, degradation mode, or high-throughput contention. The signal is operationally useful without being a viewer-state leak.

**Decision:** Adopt **silent freeze with backpressure signal**. The runtime sends `MUTATION_QUEUE_PRESSURE` (via `RuntimeError` in `MutationResult`) when the per-session queue reaches 80% capacity (800/1000), and `MUTATION_DROPPED` for each shed mutation. Agents that receive `MUTATION_QUEUE_PRESSURE` may voluntarily reduce submission rate. The freeze still executes unconditionally regardless of agent response.

**Doctrinal grounding:**

- This does not violate security.md §"Human override": the freeze still happens unconditionally. The advisory does not allow interception, delay, or veto.
- This does not violate privacy.md §"Agent isolation": the signal reveals queue pressure, not viewer intent. The same signal fires in any queue-pressure scenario (slow compositor, degradation, contention).
- This is consistent with architecture.md §"Policy arbitration": the `MUTATION_QUEUE_PRESSURE` signal is below priority 1 (human override). It is informational feedback on the mutation pipeline, not a governance hook.
- Parallel precedent: `DegradationNotice` (RFC 0005 §3.4) already notifies agents about runtime resource constraints without revealing viewer actions. `MUTATION_QUEUE_PRESSURE` is the same category of signal.

**Changes applied in this round:**
- **[NORMATIVE → APPLIED]** §4.3: Replaced "Agents are not informed that the scene is frozen" with the normative backpressure signal spec. Added `MUTATION_QUEUE_PRESSURE` and `MUTATION_DROPPED` behavior at 80% queue capacity and on overflow respectively.
- **[CROSS-RFC → NOTED]** §8 RFC 0005 row updated: `MUTATION_QUEUE_PRESSURE` and `MUTATION_DROPPED` error codes must be added to RFC 0005 §3.5 `RuntimeError.ErrorCode` enum and the field number reservation table. Flagged as a protocol addition requirement.

---

### Round 1 — Doctrinal Alignment Deep-Dive (rig-5vq.35)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, security.md, failure.md, privacy.md, v1.md

#### Doctrinal Alignment: 4/5
Chrome layer, human override, failure indicators, and privacy isolation all align with doctrine. The following doctrinal gaps were found and fixed:

- **[MUST-FIX → FIXED]** §2.3: Wrong cross-ref RFC 0004 §4 (IME) → RFC 0004 §8 (Event Dispatch Protocol).
- **[MUST-FIX → FIXED]** §4.1: Replace undefined `LeaseRevoked` event with correct RFC 0005 `LeaseResponse` / `lease_changes` subscription category.
- **[MUST-FIX → FIXED]** §4.2, §5.2, §5.5: Flag `SessionSuspended` / `SessionResumed` as undefined in RFC 0005; added inline notes and explicit protocol-gap entry in §8.
- **[SHOULD-FIX → FIXED]** §7.5: Clarify `SafeModePhase` enum has only NORMAL/ACTIVE (not EMERGENCY_FALLBACK) with comment explaining why the diagram's third state is outside this machine.
- **[SHOULD-FIX → FIXED]** §7.2: Clarify `budget_warning` renders as a border (§3.5), NOT a stacked badge icon.
- **[SHOULD-FIX → FIXED]** §2.5: Add Dismiss All affordance to system status indicator description to match §4.2.
- **[CONSIDER → APPLIED]** §8: Add RFC 0005 row to interaction table documenting the protocol gap.

#### Technical Robustness: 4/5
Safe mode scene-graph independence solid; ChromeState atomic read clear.

#### Cross-RFC Consistency: 3/5
RFC 0005 gap flagged and documented; §8 table now complete.

**No dimension below 3. Round 1 findings addressed. Ready for Round 2 (Technical Architecture Scrutiny).**

---

### Cross-RFC Pass — ID Type Unification (rig-6k5)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22

#### Changes Applied

- **[MUST-FIX → FIXED]** §7.3 `DismissTileEvent.tile_id`: changed `string` → `SceneId` (tile IDs are scene-object addresses per RFC 0001 §1.1).
- **[MUST-FIX → FIXED]** §7.3 `MuteToggleEvent.tile_id`: changed `optional string` → `optional SceneId`.
- **[MUST-FIX → FIXED]** §7.4 `OverrideState.muted_tile_ids`: changed `repeated string` → `repeated SceneId`.
- **[MUST-FIX → FIXED]** §7.7 `TabBarState.active_tab_id`: changed `string` → `SceneId`.
- **[MUST-FIX → FIXED]** §7.7 `TabEntry.tab_id`: changed `string` → `SceneId`.
- **[DOCUMENTED]** §7.1 `ChromeState.tile_badges`: kept as `map<string, TileBadgeState>`. Protobuf 3 map keys must be scalar types — message types (including `SceneId`) are not permitted as map keys. The string key is the UUID string encoding of the tile's `SceneId`; the comment is updated to reflect this constraint.
- Proto blocks in this RFC have no top-level `syntax = "proto3"` header; a `// import "scene.proto"` comment is added to the first proto block that references `SceneId`.

---

### Round 3 — Cross-RFC Consistency and Integration (rig-5vq.37)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, security.md, failure.md, presence.md
**Cross-RFC documents reviewed:** RFC 0002, RFC 0005, RFC 0008 (full, including §11 errata), RFC 0009 (full, including §5 conflict resolution)

#### Doctrinal Alignment: 4/5

No new doctrinal violations found. The RFC continues to faithfully implement doctrine across all reviewed soul files. Human override at position 1 (architecture.md §"Policy arbitration"), screen sovereignty, safe mode reversibility (security.md §"Human override"), and lease governance (presence.md §"Leases: presence requires governance") are all correctly expressed.

The sole remaining doctrinal gap is the RFC 0005 `SessionSuspended`/`SessionResumed` protocol gap (§8), which is correctly documented as a known gap. No doctrine point is silently dropped.

#### Technical Robustness: 4/5

No new technical gaps found in this round. Prior rounds addressed synchronization, freeze-queue overflow, safe mode / freeze interaction, and dismiss behavior on closing sessions.

#### Cross-RFC Consistency: 4/5

Two MUST-FIX items found and resolved in this round:

- **[MUST-FIX → FIXED]** §4.2: RFC 0008 §11 errata mandates that "All active leases are revoked simultaneously" be corrected to "suspended simultaneously." Safe mode is a pause, not a purge. The revoke/suspend contradiction is resolved by RFC 0008 §3.4 (DR-LG7). Applied.
- **[MUST-FIX → FIXED]** §8 (RFC 0002 row): RFC 0009 §5.2 resolves the conflict between RFC 0002 §7.3 ("trigger graceful shutdown" on GPU device loss) and RFC 0007 §5.1 ("enter safe mode" on GPU device loss). The §8 interaction table for RFC 0002 did not reference this resolution. Updated to note that RFC 0009 §5 is the authoritative two-phase GPU failure response (safe mode entry before shutdown).
- **[SHOULD-FIX → NOTED]** RFC 0009 §2.2 uses kebab-case capability names (`create-tiles`, `zone-publish`) while RFC 0006 §6.3 and RFC 0005 §7 use snake_case (`create_tiles`, `zone_publish`). RFC 0007 does not directly expose capability names in its protobuf types, so there is no fix to make here — but the naming inconsistency between RFC 0009 and RFC 0006 is a cross-RFC issue that RFC 0009 must resolve. Noted for RFC 0009's next revision.
- **[CONSIDER]** RFC 0008 §11 also notes a required addition to RFC 0005: `SessionSuspended` / `SessionResumed` must be added to `SessionMessage` oneof. RFC 0007 §8 already documents this as a "Protocol gap." No change needed; existing documentation is accurate.

**Post-fix scores: Doctrinal Alignment 4, Technical Robustness 4, Cross-RFC Consistency 4. No dimension below 3. Round 3 complete.**

---

### Round 2 — Technical Architecture Scrutiny (rig-5vq.36)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, security.md, failure.md, v1.md

#### Doctrinal Alignment: 4/5

The RFC continues to align well with core doctrine. Chrome sovereignty, safe mode as last resort, human override at priority 1, and agent isolation for viewer state are all correctly specified. No new doctrinal gaps were found in this round.

#### Technical Robustness: 4/5

Four correctness gaps found and fixed in this round:

- **[MUST-FIX → FIXED]** §7.1: `ChromeState` atomicity is asserted ("read atomically") but the synchronization mechanism is not specified. The control plane (network thread) writes `ChromeState`; the compositor thread reads it. Without a defined primitive this is a data race. Added §7.1 note specifying `Arc<RwLock<ChromeState>>` as the required synchronization contract, with a note on the write side being limited to the network/control thread.
- **[MUST-FIX → FIXED]** §4.3: Freeze queue overflow drops "older mutations" — but RFC 0005 §5.1 establishes that transactional mutations are *never dropped*. Dropping a `CreateTile` or `LeaseRequest` silently violates the delivery guarantee. Fixed with traffic-class-aware overflow: transactional mutations are never dropped; state-stream mutations are coalesced (latest wins per coalesce key) before being evicted oldest-first; ephemeral mutations are dropped oldest-first under overflow.
- **[MUST-FIX → FIXED]** §5.3/§5.6 (new): The safe mode state machine had no specified behavior when `Freeze` is active at the time of safe mode entry, or when the viewer attempts to freeze while already in safe mode. Added §5.6 specifying the interaction.
- **[MUST-FIX → FIXED]** §4.1: Dismiss-tile action did not specify behavior when the target tile's agent is already in `Disconnecting` or `Closed` session state. Added a note: the tile and resources are still cleared; the `LeaseResponse` notification is best-effort (silently dropped if the session is already closed).

#### Cross-RFC Consistency: 4/5

- **[SHOULD-FIX → FIXED]** §7.4 (`ViewerPromptState.timeout_at_us`): Used `int64` but RFC 0003 §1.1 establishes `uint64` as the canonical type for monotonic timestamps in µs. Fixed to `uint64`.
- **[SHOULD-FIX → FIXED]** §7.3: `OverrideEvent.trigger` was an unvalidated `string`. Valid values are a closed set; replaced with `OverrideTrigger` enum for type safety and audit-log correctness. Similarly, `SafeModeEntryEvent.reason` was an unvalidated `string`; replaced with `SafeModeEntryReason` enum.
- **[SHOULD-FIX → FIXED]** §7.3 / §7.5: `OverrideEvent.timestamp_us` was `int64`; missed by the initial pass. Fixed to `uint64` (RFC 0003 §1.1). `SafeModeState.entered_at_us` was similarly `int64`; fixed to `uint64`. `SafeModeState.entry_reason` was `optional string` while `SafeModeEntryEvent.reason` is `SafeModeEntryReason` — fixed to use the enum for type consistency.
- **[CONSIDER]** §4.1: Dismiss swipe threshold (40% of tile width) is not configurable. On small tiles this risks accidental dismissals. Acceptable for v1 but worth making configurable in a future revision.
- **[CONSIDER]** §7.6: `SafeModeOverlayCmd.banner_text` and `resume_button_label` carry string literals rather than localization keys. Acceptable for v1 English-only deployments but creates a future I18N regression point.

**Post-fix scores: Doctrinal Alignment 4, Technical Robustness 4, Cross-RFC Consistency 4. No dimension below 3. Round 2 complete.**

---

### Round 3 — Cross-RFC Consistency (rig-5vq.13)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** security.md (human override, safe mode as pause not purge)

#### Changes Applied

**[MUST-FIX → FIXED]** §4.2 "Dismiss All / Safe Mode" step 1 previously stated "All active leases are **revoked** simultaneously." This contradicts RFC 0008 §3.4 (DR-LG7), which is the authoritative resolution of this conflict: safe mode **suspends** leases, not revokes them. Updated step 1 to "All active leases are **suspended** simultaneously" with full RFC 0008 cross-reference and rationale note. The corrected phrasing is consistent with §5.2 ("safe mode does not terminate sessions by default"), security.md ("safe mode as last resort, quickly reversible"), and the principle that revocation is for misbehavior, not emergency stops.

**Post-fix scores: Doctrinal Alignment 4, Technical Robustness 4, Cross-RFC Consistency 4. No dimension below 3. Round 3 complete.**

---

### Round 4 — Final Hardening and Quantitative Verification (rig-5vq.38)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** security.md, failure.md, v1.md, validation.md
**Cross-RFC documents reviewed:** RFC 0003, RFC 0005 (including Round 12 changelog), RFC 0008

#### Doctrinal Alignment: 5/5

No new doctrinal violations found. All prior doctrinal gaps are resolved. Human override at position 1 (architecture.md §"Policy arbitration"), screen sovereignty, safe mode reversibility (security.md §"Human override"), lease governance (presence.md), and local feedback first (failure.md) are all correctly expressed. The pulsing animation fix aligns with v1.md's animation deferral policy. No doctrine commitment is silently dropped.

#### Technical Robustness: 5/5

One internal contradiction resolved in this round:

- **[MUST-FIX → FIXED]** §5.4: Said "ChromeState is updated atomically and can be read without locks" — directly contradicted §7.1's specification of `Arc<RwLock<ChromeState>>`. The §5.4 text was stale from before Round 2 added the synchronization contract. Replaced with a correct description that references the §7.1 lock contract.

One type correction:

- **[SHOULD-FIX → FIXED]** §7.7 `TabBarState.scroll_offset_px` and `hidden_tab_count`: Used `int32` for quantities that are non-negative by definition (scroll offset is always ≥ 0 logical pixels; hidden tab count is always ≥ 0). Changed to `uint32` with explanatory comments. Using signed types for non-negative invariants forces callers to check for negative values that the model disallows.

One v1 animation policy alignment:

- **[SHOULD-FIX → FIXED]** §3.2: Disconnection badge specified a "pulsing animation (period: 2 seconds, opacity range: 60%–80%)" — directly contradicted §6.2 ("cross-fade over 300ms — the only animation permitted in v1 chrome") and the §9 deferred list ("Animated tile dismissal transitions"). Replaced with a static badge at 70% opacity with an explicit v1 deferral note for the animation.

#### Cross-RFC Consistency: 5/5

Three stale cross-reference notes resolved in this round:

- **[MUST-FIX → FIXED]** §4.2 action step 2: Still said `SessionSuspended` "must be added to RFC 0005 §2 / §3.2" — but RFC 0005 Round 12 (rig-5vq.29) added it as field 45 in §3.7. Updated to reference the authoritative location.
- **[MUST-FIX → FIXED]** §5.5 exit step 2: Said "`SessionResumed` must be added to RFC 0005 alongside `SessionSuspended` — see §8" — same staleness. Updated to reference RFC 0005 §3.7 field 46.
- **[MUST-FIX → FIXED]** §8 RFC 0005 row: Still labelled the `SessionSuspended`/`SessionResumed` issue as "Protocol gap" requiring RFC 0005 to be updated. Replaced with the resolved status referencing RFC 0005 §3.7 and Round 12.

One timestamp naming fix:

- **[SHOULD-FIX → FIXED]** §7.3 `OverrideEvent.timestamp_us`: Plain `_us` suffix is ambiguous (RFC 0003 §1.3 establishes `_mono_us` for monotonic, `_wall_us` for wall-clock; RFC 0005 Round 6 (rig-77n) applied this convention across session protocol fields). Since §8 explicitly states override events use the monotonic clock, renamed to `timestamp_mono_us`. Also updated the §8 RFC 0003 row to match.

One §8 RFC 0002 precision fix:

- **[SHOULD-FIX → FIXED]** §8 RFC 0002 row: Said "ChromeState is read atomically from the same shared state" — vague after §7.1 specified `Arc<RwLock<ChromeState>>`. Updated to name the Stage 6 pipeline position and the specific synchronization mechanism.

**Post-fix scores: Doctrinal Alignment 5, Technical Robustness 5, Cross-RFC Consistency 5. All dimensions ≥ 4. Round 4 (final) complete. RFC 0007 is shipping-ready.**

---

## Summary

This RFC defines the System Shell — the runtime-owned chrome layer that guarantees viewer sovereignty. It specifies the composition rules that place the chrome above all agent content, the layout and behavior of every chrome element (tab bar, tile badges, override controls, safe mode overlay, and privacy indicator), the internal state machines and event types governing these elements, and the protobuf types used to drive them. The chrome layer is not an agent feature — it is the human's permanent contract with the runtime.

---

## Motivation

tze_hud gives LLMs governed presence on shared physical screens. That governance only works if the human can always see what is happening and always act to stop it. A presence engine with no guaranteed human override is not a presence engine — it is a hostile takeover of the display.

Without a defined system shell:

- There is no canonical surface for override controls. Individual implementations will drift.
- The compositing relationship between chrome and agent content is unspecified; a buggy agent or renderer change could accidentally occlude controls.
- Badge semantics for disconnected or stale tiles are ad hoc, giving users inconsistent visual signals.
- Safe mode entry/exit has no defined contract, making it impossible to verify that it always works.
- Privacy indicator behavior is undefined, leaking viewer context information to agents or to observers.

This RFC resolves all of these by treating the chrome layer as a first-class, independently specified subsystem with hard contracts for composition, correctness, and agent isolation.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| Viewer sovereignty — human can always stop agents | Override controls §4, Safe mode §5 |
| Disconnection/staleness never leaves stale tiles silent | Tile badges §3 |
| Privacy — viewer context never exposed to agents | Privacy/viewer UX §6 |
| Chrome always rendered above agent content | Chrome layer composition §1 |
| Override controls are local and instantaneous | Override controls §4 |
| Runtime always usable even if scene graph corrupted | Safe mode §5 |
| Human interventions auditable (security.md: "every capability grant and revocation is logged") | Shell audit events §7.8 |

---

## 1. Chrome Layer Composition

### 1.1 Layer Stack Position

The compositor renders three ordered layers, back to front (specified in architecture.md §Compositing model):

1. **Background layer** — ambient content or transparent passthrough. Runtime-owned.
2. **Content layer** — agent tiles, z-ordered within the layer.
3. **Chrome layer** — runtime-owned system shell UI. Always on top, always rendered last.

The chrome layer renders on the same wgpu pipeline as the content layer. It is not a separate window, a separate OS overlay, or a separate GPU context. It is a set of render passes that execute after all content tiles have been composited, drawing chrome elements into the final framebuffer.

This architecture guarantee is load-bearing: because chrome shares the pipeline, it is always produced in the same frame as the content beneath it. There is no frame-level gap in which agent content is visible without chrome.

### 1.2 Agent Exclusion

Agents have no API to:
- Read the chrome layer's current state.
- Write to the chrome layer.
- Occlude the chrome layer (no tile z-order value can exceed chrome).
- Suppress chrome rendering.
- Receive events from chrome UI elements.

The chrome layer does not appear in scene topology queries. Its elements are not `SceneId`-addressed objects in the agent-visible scene graph.

### 1.3 Chrome Elements

The chrome layer contains the following elements. All are defined in this RFC:

| Element | Section | Default visibility |
|---------|---------|-------------------|
| Tab bar | §2 | Visible |
| System status indicator | §2.4 | Visible |
| Tile badges | §3 | Conditional (per tile state) |
| Focus ring | RFC 0004 §5.6 | Conditional (on active input focus) |
| Override controls | §4 | Visible (opacity varies by context) |
| Safe mode overlay | §5 | Hidden; shown only in safe mode |
| Privacy indicator | §6 | Visible |

### 1.4 Rendering Independence

Chrome rendering must not depend on any agent state. The chrome render pass reads from:
- The runtime's own `ChromeState` struct (defined in §7).
- The compositor's tab list, viewer context, and override event queue.
- Cached layout geometry computed from display profile.

If all agents crash simultaneously, chrome renders correctly on the next frame. If the scene graph is corrupted (see §5, Safe Mode), chrome renders its safe mode overlay without touching the scene graph at all.

### 1.5 Display Profile Adaptation

Chrome layout adapts to the active display profile. Profile-specific overrides are expressed as a `ChromeLayout` configuration:

```rust
pub struct ChromeLayout {
    /// Tab bar position and height in logical pixels.
    pub tab_bar: TabBarLayout,
    /// Reserved margin for override controls (top-right corner by default).
    pub override_controls_margin: Rect,
    /// Badge anchor relative to tile corner (default: top-right).
    pub badge_anchor: BadgeAnchor,
    /// Minimum readable font size for chrome text.
    pub min_font_size_px: f32,
}
```

Profiles (desktop, compact, etc.) supply `ChromeLayout` values. The chrome renderer reads these at layout time; no profile-conditional logic lives in the render passes.

---

## 2. Tab Bar

### 2.1 Position and Visibility

The tab bar is positioned at the top of the viewport by default. Position is configurable per display profile:

```toml
[chrome.tab_bar]
position = "top"   # "top" | "bottom" | "hidden"
```

When `position = "hidden"`, the tab bar is not rendered. Keyboard shortcuts for tab switching remain active. This mode is intended for dedicated single-tab displays (e.g., a smart mirror locked to one tab).

The tab bar height is determined by `ChromeLayout.tab_bar.height_px`. On displays smaller than a threshold (configurable, default 600px logical height), the tab bar compresses to a minimal height showing only the active tab indicator and overflow count.

### 2.2 Tab Bar Content

Each rendered tab displays:
- **Tab name** — the human-readable string set in the tab's configuration. Truncated with ellipsis if it exceeds the allocated width.
- **Active indicator** — a visual marker (underline or highlight, style defined by the active theme) on the currently active tab.
- **Tab count badge** — when there are more tabs than can be displayed without overflow, a `+N` label appears at the end of the visible tab list.

The tab bar does not display agent-supplied metadata (tile counts, agent names, custom icons). These are agent concerns and do not belong in the chrome.

### 2.3 Keyboard Shortcuts

All tab navigation is available without mouse/touch:

| Action | Default shortcut |
|--------|-----------------|
| Next tab | `Ctrl+Tab` |
| Previous tab | `Ctrl+Shift+Tab` |
| Switch to tab N | `Ctrl+1` through `Ctrl+8` |
| Switch to last tab | `Ctrl+9` (always the last tab, regardless of count — matches browser convention) |

Shortcuts are configurable in `config.toml`. They are handled by the input model's event dispatch protocol (RFC 0004 §8), which evaluates chrome shortcuts before tile hit-testing. Shortcut events are never routed to any agent.

### 2.4 Overflow Handling

When the number of tabs exceeds what fits in the tab bar width:

- Tabs scroll horizontally. The active tab is always scrolled into view.
- A `+N` indicator appears at the trailing end showing the count of off-screen tabs.
- Scrolling the tab bar is a chrome gesture (horizontal scroll/swipe on the tab bar area) and is handled locally without agent involvement.
- For displays with more than 10 tabs, scrolling is supplemented by a tab picker (invoked by long-press or `Ctrl+\``). The tab picker is a chrome-rendered overlay listing all tabs; it is not an agent surface.

### 2.5 System Status Indicator

The tab bar's trailing end (opposite the overflow indicator) contains a system status indicator. It shows:
- A green/amber/red dot indicating overall session health (all agents connected / some degraded / all disconnected or safe mode).
- Active agent count (e.g., `3 agents`).
- Current viewer class icon (see §6.1).
- A "Dismiss All" affordance (e.g., a subtle button or long-press target) that triggers the override action defined in §4.2. This affordance is accessible via keyboard focus traversal through the chrome layer.

The system status indicator does not expose agent identities or names.

---

## 3. Tile Badges and Indicators

Tile badges are chrome-layer overlays anchored to individual content-layer tiles. They communicate runtime state to the viewer without agent involvement. Badges are always rendered above tile content.

### 3.1 Badge Anatomy

All badges share a common visual language:
- **Position:** Top-right corner of the tile, inset by 4 logical pixels. This position is invariant — all badges appear in the same location.
- **Size:** 20×20 logical pixels for icon-only badges; auto-width for text badges with a fixed icon prefix.
- **Layering:** Multiple badges stack vertically downward from the top-right corner, each 4 logical pixels apart.
- **Priority order** (top to bottom when stacked): disconnection, staleness, budget warning. Redaction is not a stacking badge — it is a full content replacement (see §3.4).

### 3.2 Disconnection Badge

Shown when the agent that owns the tile's lease has disconnected but the grace period has not yet expired (see failure.md §Agent crashes).

Appearance:
- A dim plug/link-break icon at reduced opacity (70%). In v1, this badge is static — the pulsing animation described in earlier drafts is deferred post-v1 (§9.2 deferred list: "Animated tile dismissal transitions"). The badge is visually distinct from active state at a glance; animation adds polish but is not required for correctness.
- The tile's content is rendered at reduced opacity (70%) to signal frozen state without being alarming.

Behavior:
- Badge appears immediately on disconnection detection.
- Badge clears immediately when the agent reconnects and reclaims the lease.
- If the grace period expires without reconnection, the tile is cleared and the badge is removed with it.

The disconnection badge is intentionally subtle. It signals "this is stale" without demanding immediate viewer attention. It is not a modal, not a full-tile overlay, and not a notification.

### 3.3 Staleness Indicator

Shown when a tile's content has not been updated beyond a configurable threshold, and the tile's content type is expected to be live.

```toml
[chrome.badges]
staleness_threshold_secs = 30   # default: 30 seconds
```

Appearance:
- A clock icon in the top-right badge position.
- No animation — the staleness indicator is static, unlike the disconnection badge.

Behavior:
- Staleness tracking is per-tile, based on the last `content_updated_at` timestamp in the tile's scene state.
- Only tiles with `stale_content_alert = true` in their lease metadata show this badge. Agents declare this intent at lease-request time. A static image tile should not show a staleness badge.
- The staleness indicator does not trigger a reconnection attempt or affect the lease.

### 3.4 Redaction Placeholder

Shown when a tile's content is redacted due to viewer context (see privacy.md §Redaction behavior). This is not strictly a "badge" — it replaces the tile's content entirely — but it is chrome-rendered and belongs to this section.

Appearance:
- The tile geometry is preserved. Its bounds remain in place in the layout.
- The tile's content area is filled with a neutral pattern (configurable; default: a subtle crosshatch in the theme's muted foreground color).
- No agent name, no content hint, no icon. The placeholder conveys "something is here but not for you" without revealing what.
- Interactive affordances (hit regions) on the tile are disabled while redacted. The tile is visible but inert.

Behavior:
- Redaction is applied by the compositor during the chrome pass, after the content layer is composited. The tile's content is already rendered into a texture; the chrome pass draws the placeholder pattern over it.
- The agent is not notified that its tile is being redacted. Redaction is invisible to the publishing agent.
- When the viewer context changes to one that permits the tile's classification, the placeholder is removed and the tile's content texture is composited normally.

#### 3.4.1 Capture Safety: Redaction Strategy

The default redaction approach (overlay redaction) draws the placeholder pattern over an already-rendered tile texture. This is safe for live local display — the final composited framebuffer is the only consumer, and the placeholder is always on top.

However, once any capture surface is active (screen capture, OS screenshot, mirror output, remote view, recording), the pre-redaction tile texture continues to exist in GPU memory with the actual agent content. Two leakage vectors exist:

1. **Pre-chrome-pass capture:** A mirror or remote view that reads the framebuffer before the chrome pass executes sees unredacted tile textures.
2. **Texture export / debug paths:** A debug tool, OS-level screen grabber, or tile-texture export mechanism that reads individual tile textures — rather than the final composited output — sees unredacted content.

To close these vectors, the compositor supports two redaction strategies, selected per-tile at render time:

**Strategy 1 — Overlay redaction (v1 default):**
The tile's content is rendered into its texture normally. The chrome pass draws the placeholder pattern over it. The tile texture in GPU memory contains actual agent content. This is the v1 default because v1 defers all capture, mirror, and remote viewing surfaces; there is no second consumer of tile textures and no pre-chrome-pass capture path.

**Strategy 2 — Render-skip redaction (required when any capture surface is active):**
The compositor skips rendering the tile's content entirely — the tile's texture slot is not populated with agent content. Instead, the placeholder pattern is rendered directly into the tile's bounds during the content layer pass (before the chrome pass). The tile texture never contains agent content. Because the placeholder is already in the framebuffer before any capture surface reads it, all capture consumers see only the placeholder regardless of when they sample the framebuffer.

**Activation rule:** Render-skip redaction is activated automatically for all currently-redacted tiles whenever a capture surface is registered with the compositor. The compositor evaluates per-tile redaction strategy at the start of each frame, not at redaction-state-change time. The strategy transitions atomically with the per-frame `ChromeState` snapshot: a tile transitions from overlay to render-skip on the first frame after `capture_surface_active` becomes true, and back to overlay on the first frame after it becomes false.

**V1 note:** Render-skip redaction is architecturally specified here to prevent implementation decisions that would foreclose it (e.g., merging content rendering and chrome rendering into a single monolithic pass). V1 ships overlay-only — `capture_surface_active` is always false in v1 because capture, mirroring, and remote viewing are all deferred post-v1. Implementations MUST NOT assume overlay-only is permanent; the compositor architecture MUST keep content rendering and chrome rendering as separable passes.

**Agent isolation:** Neither strategy changes the agent-facing contract. Agents do not receive notification of which strategy is active. The strategy selection is invisible to agents — it is a compositor-internal detail governed by the presence of capture surfaces, which are a runtime (not agent) concern.

### 3.5 Budget Warning Badge

Shown when an agent's session is approaching its resource budget limit (see security.md §Resource governance). This is a viewer-facing signal, not an agent error.

Appearance:
- A subtle amber border highlight on the affected tile (2px, 70% opacity).
- Not a badge icon — the border highlight is the indicator.

Behavior:
- Shown when the agent's resource consumption reaches 80% of its session budget.
- Removed when consumption drops below 80%.
- The agent receives a separate warning event through the gRPC session stream; the badge is an additional viewer-visible signal.

---

## 4. Override Controls

Override controls are the viewer's direct intervention surface. They are always local, always instantaneous, and never routed through any agent.

### 4.1 Dismiss Tile

**Mechanism:** An X button appears in the top-right corner of a tile on hover (or on touch-hold for touch displays). On desktop displays without hover, it appears when focus is on the tile via keyboard navigation.

**Action:** Clicking/tapping/activating the dismiss button:
1. Immediately revokes the tile's lease.
2. Removes the tile from the scene.
3. Frees the tile's resources.
4. Sends a lease revocation notification to the owning agent via the `LeaseResponse` message (RFC 0005 §3.2, `lease_changes` subscription category) with reason `viewer_dismissed`.

The agent may re-request a lease. The runtime does not permanently block an agent that was dismissed — viewer dismissal is a momentary choice, not a permanent ban. Permanent capability revocation is a separate administrative action.

**Behavior when the owning agent is already disconnected or closing:**
If `DismissTileEvent` arrives for a tile whose agent session is in `Disconnecting`, `Closed`, or orphaned state (lease already in grace period), the runtime still:
1. Cancels the grace period immediately (the tile is not waiting for reconnection).
2. Clears the tile from the scene and frees its resources.
3. Skips the `LeaseResponse` notification — the session is not active to receive it. The notification is silently dropped (best-effort delivery applies to notifications for already-closed sessions; attempting to write to a closed gRPC stream is an error, not a protocol violation).

This ensures dismiss always has the effect the viewer expects, regardless of the agent's current session state.

**Swipe gesture:** On touch displays, a left-to-right or right-to-left swipe across a tile activates the dismiss action directly (no button required). The swipe threshold is 40% of tile width.

### 4.2 Dismiss All / Safe Mode

**Primary shortcut:** `Ctrl+Shift+Escape`

**Secondary:** A "Dismiss All" control in the system status area (accessible via keyboard focus traversal through the chrome layer).

**Action:**
1. All active leases are suspended simultaneously. Sessions are not terminated; leases transition to `SUSPENDED` state (RFC 0008 §3.3) and are restored to `ACTIVE` on safe mode exit without requiring agents to re-request leases. (Correction per RFC 0008 §11 errata: the prior text incorrectly stated "revoked." Safe mode is a pause, not a purge — see RFC 0008 §3.4 for the canonical resolution of this revoke/suspend contradiction.)
2. All agent sessions receive `SessionSuspended` with reason `safe_mode` (sessions are suspended, not terminated — see §5.2 for rationale). `SessionSuspended` is defined in RFC 0005 §3.7 (field 45 of `SessionMessage` oneof, added in RFC 0005 Round 12).
3. The runtime enters safe mode (see §5).

This is the "emergency stop" for the entire display. It is not reversible by agents — they cannot reinstate their sessions in response to this event. The viewer must explicitly exit safe mode.

**Authoritative behavior:** RFC 0008 §3.4 (DR-LG7) is the authoritative specification of this action. "Dismiss All" **suspends** leases, not revokes them. The distinction is critical: a viewer who accidentally triggers safe mode can resume without agents needing to reconnect and re-establish their leases. Revocation is the correct response to *budget violations* and *malicious behavior*, not to the human pressing the emergency stop.

### 4.3 Freeze Scene

**Shortcut:** `Ctrl+Shift+F`

**Description:** Freezes the current scene state. Agent mutations are queued but not applied. The display shows the frozen scene. Incoming mutations accumulate in a bounded queue.

**Behavior:**
- While frozen, tile content does not update. Badges continue to render (a frozen tile can still show a disconnection badge from before freeze).
- The freeze indicator appears in the system status area (a pause icon).
- Queue limit: 1000 mutations per session (configurable). Overflow behavior is **traffic-class-aware** — not a blanket "drop oldest":
  - **Transactional mutations** (RFC 0005 §5.1: "never dropped") are **never evicted** from the freeze queue. If the transactional portion of the queue is full, the runtime applies backpressure to that agent's session via gRPC flow control (same mechanism as the `MutationBatch` channel — see RFC 0002 §2.6). The agent's mutations accumulate in the gRPC send buffer until either the freeze ends or the agent's buffer fills and it observes backpressure.
  - **State-stream mutations** (coalescing allowed): when the per-session queue is full, state-stream mutations from the same agent are coalesced (latest wins per coalesce key) before older entries are evicted. This matches normal backpressure behavior (RFC 0005 §2.4).
  - **Ephemeral realtime mutations**: dropped without delay when the queue is full, identical to normal backpressure behavior.
- Unfreeze is triggered by the same shortcut (`Ctrl+Shift+F`) or via the freeze indicator in the status bar.
- On unfreeze, queued mutations are applied in submission order in the next available frame batch.

**Freeze agent notification model (decided in rig-9v1): silent freeze with backpressure signal.**

Freeze does not disconnect agents — their sessions remain active. Agents are **not** informed that the scene is frozen (the viewer's freeze action is viewer state and must not be exposed to agents per privacy.md §"Agent isolation"). An explicit `SCENE_FROZEN` advisory is therefore absent by design.

Instead, the runtime uses the existing mutation response path to signal general queue pressure, without revealing the cause:

1. **At 80% queue capacity** (800/1000 by default): the runtime sends a `RuntimeError` with `error_code = "MUTATION_QUEUE_PRESSURE"` in the `MutationResult` response for the triggering batch. This indicates that the per-session mutation queue is under pressure and that the agent should consider reducing submission rate. The signal fires for any queue-pressure scenario (freeze, slow compositor, degradation, high-throughput contention) — not specifically for freeze. Agents that honor this signal will voluntarily reduce submission rate; agents that ignore it continue normally.
2. **On overflow** (queue full, mutation shed): the runtime sends a `RuntimeError` with `error_code = "MUTATION_DROPPED"` in the `MutationResult` for each shed mutation. Transactional mutations are never shed (see above); only state-stream and ephemeral mutations are eligible for overflow eviction.

**Rationale for this model:**
- Does not reveal viewer intent: the signal is triggered by queue pressure state, not by the freeze action. Any slow-consumption scenario (not just freeze) can produce the same signal.
- Consistent with `DegradationNotice` (RFC 0005 §3.4): the runtime already notifies agents about resource constraints without revealing viewer actions.
- Does not allow agent veto: the freeze executes unconditionally. Agents that reduce rate cooperate passively; agents that ignore the signal are correct — the runtime queues or sheds their mutations per the overflow rules above.
- Compatible with architecture.md §"Policy arbitration": `MUTATION_QUEUE_PRESSURE` is informational feedback below priority 1. It does not intercept, delay, or veto the freeze action.

**Note:** `MUTATION_QUEUE_PRESSURE` and `MUTATION_DROPPED` must be added to RFC 0005 §3.5 `RuntimeError.ErrorCode` enum. See §8 RFC 0005 row.

### 4.4 Mute (Reserved Surface — Defined, Not V1)

The chrome layer reserves a mute control surface for per-tile and global audio muting. V1 defers media integration (GStreamer, WebRTC) so mute controls are not functional in v1.

The control surface is defined here to prevent incompatible implementations:
- **Per-tile mute:** Speaker icon badge, toggleable, appears on tiles with active media leases.
- **Global mute:** `Ctrl+Shift+M`. Mutes all active media streams.

These controls are rendered as disabled/greyed in v1 if media is not active. They accept input but take no action (log a noop).

### 4.5 Override Control Guarantees

All override controls satisfy:
- **Local execution.** Control actions execute entirely within the runtime process. No network roundtrip, no agent callback, no IPC.
- **Frame-bounded response.** The control's visual effect (tile disappears, safe mode overlay appears, freeze indicator shows) is reflected within one frame (≤ 16.6ms) of the input event.
- **Unconditional.** No agent capability, no scene state, no policy evaluation can prevent an override control from executing. The priority model in architecture.md §Policy arbitration places human override at position 1, above all other rules.
- **No agent veto.** Agents receive notification of what happened (via session events) but cannot respond in a way that undoes the override.

---

## 5. Safe Mode

Safe mode is the runtime's highest-protection state. It guarantees the viewer can always recover control of the display even under severe failure conditions.

### 5.1 Entry Conditions

Safe mode is entered by:
1. **Explicit viewer action:** `Ctrl+Shift+Escape` (§4.2), or activating the "Dismiss All" chrome control.
2. **Automatic entry on critical runtime error:** If the compositor detects a condition that would otherwise produce a blank or unresponsive screen — scene graph corruption, GPU device loss, unrecoverable render failure — it enters safe mode rather than crashing.

Automatic entry logs the triggering condition to the runtime's structured error log.

### 5.2 Safe Mode Behavior

On safe mode entry:
1. **Session suspension.** All agent gRPC sessions receive `SessionSuspended` with reason `safe_mode`. Sessions are not terminated — their network connections are maintained, but all mutations are rejected with `SAFE_MODE_ACTIVE` until safe mode exits. `SessionSuspended` is defined in RFC 0005 §3.7 (field 45 of `SessionMessage` oneof).
2. **Scene replacement.** Agent tiles are replaced with neutral placeholders. The placeholder appearance matches the redaction placeholder (§3.4) — a subtle neutral pattern — but covers the full tile bounds with a "Session Paused" label in the center.
3. **Safe mode overlay.** A full-viewport overlay is rendered with:
   - A centered banner: "Safe Mode — All agent sessions paused."
   - A prominent "Resume" button.
   - Current viewer class indicator.
   - No agent branding, no agent content.
4. **All input is captured.** In safe mode, all input events are consumed by the chrome layer. No input reaches agent tiles or is forwarded to agents.

Safe mode does not terminate sessions by default. This is intentional: a viewer who accidentally entered safe mode should be able to resume without agents needing to reconnect and re-establish their leases.

### 5.3 Safe Mode State Machine

```
┌──────────────────────────────────────────────────────┐
│                    NORMAL                            │
│  (agents active, scene renders normally)             │
└──────────────────────────────────────────────────────┘
         │                            ▲
         │  Entry trigger             │  Exit trigger
         │  (shortcut or auto)        │  (viewer action)
         ▼                            │
┌──────────────────────────────────────────────────────┐
│                   SAFE MODE                          │
│  Sessions suspended, tiles replaced, overlay shown   │
│  No agent mutations accepted (SAFE_MODE_ACTIVE err)  │
│  All input captured by chrome                        │
└──────────────────────────────────────────────────────┘
         │
         │  Critical error on entry itself
         │  (overlay cannot render due to GPU loss)
         ▼
┌──────────────────────────────────────────────────────┐
│               EMERGENCY FALLBACK                     │
│  OS-level blank screen or OS notification            │
│  Runtime has failed unrecoverably                    │
│  (this state is never reached by design)             │
└──────────────────────────────────────────────────────┘
```

### 5.4 Scene Graph Independence

Safe mode must render its overlay correctly even if the scene graph is in an invalid state. This is why the chrome layer's render pass reads exclusively from `ChromeState` (§7.1) rather than the scene graph. `ChromeState` is protected by `Arc<RwLock<ChromeState>>` (§7.1 synchronization contract); the compositor acquires a read lock at the start of the chrome render pass and holds it only for command generation, not during GPU submit. Even if the scene graph's backing store is corrupted, the chrome pass can still complete — the safe mode overlay is derived entirely from `ChromeState` and references only theme colors and a font atlas.

The safe mode overlay is specified as a fixed set of render commands (see §7.3 `SafeModeOverlayCmd`) that reference only theme colors and a font atlas — no scene graph entities.

### 5.5 Exit from Safe Mode

Safe mode exits only by explicit viewer action:
- Clicking/tapping the "Resume" button on the safe mode overlay.
- Keyboard: `Enter` or `Space` while the Resume button has focus (it has focus by default on safe mode entry).
- Shortcut: `Ctrl+Shift+Escape` (same as entry — toggle behavior).

On exit:
1. The safe mode overlay is dismissed.
2. Sessions transition from suspended to active. Agents receive `SessionResumed`. `SessionResumed` is defined in RFC 0005 §3.7 (field 46 of `SessionMessage` oneof, added in RFC 0005 Round 12).
3. Agent mutations are accepted again.
4. The compositor resumes applying pending scene mutations from the queue (if any were queued during suspension).
5. The scene renders with current tile state (which may differ from pre-safe-mode state if agents continued submitting mutations during suspension — those mutations were queued, not discarded).

### 5.6 Safe Mode and Freeze Interaction

The safe mode and freeze states (§4.3) may be simultaneously requested. The following rules govern their interaction:

**Freeze active → Safe mode entry triggered:**
1. Safe mode entry takes priority unconditionally (human override is highest priority per architecture.md §Policy arbitration).
2. The freeze state is **cancelled** on safe mode entry. The freeze queue is discarded — pending mutations that were queued during freeze are dropped (all classes, including transactional). Agents are not notified about the dropped mutations; they will see their sessions suspended by `SessionSuspended` and must reconcile state when they resume.
3. The freeze indicator is removed from the system status area as part of the safe mode overlay rendering (the overlay replaces all normal chrome elements).
4. `OverrideState.freeze_active` is set to `false` as part of the safe mode entry transition.

**Freeze shortcut triggered while already in safe mode:**
- The freeze shortcut (`Ctrl+Shift+F`) is ignored while safe mode is active. The safe mode overlay captures all input (§5.2, point 4), and the freeze action is not applicable to a suspended session set. The shortcut produces no effect and is not logged as an override event.

**Safe mode exit with prior freeze:**
- After safe mode exit, the freeze state is inactive (it was cancelled on entry). The viewer must explicitly re-trigger freeze if desired. The freeze queue is empty; agents start fresh with normal mutation delivery.

This interaction is reflected in the `OverrideState` type (§7.4): `freeze_active` will always be `false` when `SafeModeState.phase == SAFE_MODE_PHASE_ACTIVE`.

---

## 6. Privacy / Viewer State UX

The privacy indicator and viewer state UX give the viewer a persistent, non-intrusive signal of the current viewer context. They are chrome-rendered and never expose state to agents.

### 6.1 Viewer Class Indicator

The current viewer class is displayed as an icon in the system status area (§2.5). The icon is small and unobtrusive — it communicates state at a glance without dominating the interface.

| Viewer class | Icon | Description |
|---|---|---|
| Owner | Filled circle | Full access. Private and sensitive content visible. |
| Household member | Partial circle | Shared content visible. Private content redacted. |
| Known guest | Outline circle | Guest-appropriate content only. |
| Unknown/unauthenticated | Question mark | Ambient content only. All private content redacted. |
| Nobody (no presence) | Dim circle | Screen in passive mode. |

The icon is accompanied by no text by default. A tooltip (hover) or long-press (touch) reveals the full label.

### 6.2 Viewer Context Transitions

When the viewer context changes:
1. The viewer class icon transitions smoothly (cross-fade over 300ms — the only animation permitted in v1 chrome).
2. Tiles whose visibility classification changes (newly visible or newly redacted) update on the next frame after the context change is applied.
3. If private content transitions from visible to redacted (e.g., viewer switches from Owner to Guest), the transition is immediate — the placeholder pattern appears in the same frame the context change takes effect.
4. If content transitions from redacted to visible, the same frame-immediacy applies.
5. A `VIEWER_CLASS_CHANGED` audit event is emitted to the telemetry thread (§7.8) carrying `old_class` and `new_class`. The event contains only the resulting viewer class values — it does not carry viewer identity details (name, biometric scores, authentication method). See §7.8 privacy constraint.

The agent is never notified of viewer context changes. Its mutations continue to be accepted or rejected based on capability scopes, not viewer context.

### 6.3 "Who's Watching?" Prompt

When the viewer detection mechanism is uncertain about the current viewer (e.g., face recognition returns a confidence score below threshold), the runtime may present an optional identity confirmation prompt:

- The prompt appears in the chrome layer as a compact bottom-bar overlay (not full-screen).
- Content: viewer class icon + "Is this you?" with one or two selectable identities and a "Guest" fallback.
- Input is captured by the chrome layer. The prompt does not block tile rendering — tiles continue to display redacted content appropriate to the lowest-confidence viewer class until the prompt is resolved.
- Prompt timeout: 30 seconds (configurable). On timeout, the runtime defaults to the lowest-confidence classification.
- The prompt is optional and disabled by default. It is enabled via:
  ```toml
  [privacy]
  viewer_identification_prompt = true
  ```

### 6.4 Agent Isolation for Viewer State

No viewer state is available to agents through any API surface:
- gRPC scene events do not include viewer class.
- MCP tools do not return viewer class.
- Tile redaction is silent — agents do not receive a notification that their tile is redacted.
- The `list_scene` MCP tool and scene topology gRPC responses omit viewer context.

An agent that publishes private content will have that content redacted in the chrome pass. The agent cannot detect this. This is by design: an agent that knows "a guest is watching" could use that information in ways that violate the viewer's privacy.

---

## 7. Protobuf / Internal Types

The types in this section define the internal state and render commands for the chrome layer. These types are not part of the agent-facing gRPC API. They are used by the compositor internally and are not exposed on any agent-accessible RPC.

### 7.1 ChromeState

`ChromeState` is the single source of truth for all chrome rendering decisions. It is maintained by the control plane and read atomically by the compositor thread.

**Synchronization contract.** `ChromeState` crosses a thread boundary: the control plane (running on the network/Tokio thread) writes it; the compositor thread reads it each frame. In Rust, this is expressed as `Arc<RwLock<ChromeState>>`:

- **Writers (control plane):** Acquire a write lock, update the relevant fields (e.g., update a badge, transition safe mode phase), then release. Write operations on `ChromeState` are infrequent (driven by agent events, not by the frame loop) and should be short-lived — never hold the write lock across I/O or agent callbacks.
- **Readers (compositor thread):** Acquire a read lock at the start of the chrome render pass (Stage 6, RFC 0002 §3.1). Hold it only for the duration of command generation from `ChromeState` — do not hold it during GPU submit. Multiple compositor frames may read concurrently only if a future design introduces parallel render passes; for v1 the compositor thread is the sole reader.
- **Write priority.** If write latency becomes a bottleneck (unlikely — writes are sparse events), consider `arc-swap` crate's `ArcSwap<ChromeState>` for zero-contention reads: the control plane atomically swaps the pointer to a new `ChromeState` value without blocking readers. The compositor thread always reads the latest fully-committed snapshot.

The compositor must never read `ChromeState` fields directly from a raw pointer or non-synchronized reference. The `Arc<RwLock<>>` (or `ArcSwap`) wrapper is the mandatory access path. This is a correctness invariant, not a performance suggestion — violation is a data race.

```protobuf
// import "scene.proto";  // SceneId (tze_hud.scene.v1) — RFC 0001 §7.1
//
// Note: SceneId appears in several messages below. Protobuf 3 map keys must be
// scalar types, so ChromeState.tile_badges uses string keys (UUID string encoding
// of the tile SceneId). All other tile/tab ID fields use SceneId directly.

// Internal — not agent-accessible.
message ChromeState {
  // Tab bar state.
  TabBarState tab_bar = 1;

  // Viewer context for privacy indicator.
  ViewerClass viewer_class = 2;

  // Per-tile badge state, keyed by tile SceneId encoded as UUID string.
  // Note: protobuf 3 map keys cannot be message types, so SceneId is encoded
  // as a UUID string here. All other tile ID references in this RFC use SceneId.
  map<string, TileBadgeState> tile_badges = 3;

  // Active override state.
  OverrideState override_state = 4;

  // Safe mode state.
  SafeModeState safe_mode = 5;

  // Viewer identification prompt (if active).
  optional ViewerPromptState viewer_prompt = 6;

  // True when any capture surface is active (screen capture, OS screenshot, mirror,
  // recording, remote view). When true, the compositor switches all currently-redacted
  // tiles to REDACTION_MODE_RENDER_SKIP (§3.4.1). Always false in v1 — capture surfaces
  // are deferred post-v1. This field is set by the capture surface registry, not by agents.
  bool capture_surface_active = 7;

  // True when the operator diagnostics overlay is active (§9 — post-v1).
  // When true, the chrome layer renders per-tile diagnostic panels showing frame timing,
  // agent status, and resource usage. Always false in v1. Set by a local runtime toggle
  // (keyboard shortcut or config); never exposed to agents.
  bool diagnostics_active = 8;
}
```

### 7.2 Badge State Types

```protobuf
// Internal — not agent-accessible.
message TileBadgeState {
  bool disconnection_badge = 1;       // Agent lease is orphaned.
  bool staleness_badge = 2;           // Content not updated beyond threshold.
  bool redaction_active = 3;          // Content replaced by placeholder.
  bool budget_warning = 4;            // Agent approaching resource limit.
  // Stack order for badge icons: disconnection > staleness (top-right corner, §3.1).
  // budget_warning renders as an amber border highlight (§3.5), NOT as a stacked badge icon;
  //   it does not occupy the top-right badge position.
  // redaction_active drives a full content replacement, not a badge icon (§3.4).

  // Redaction strategy for this tile (§3.4.1). Only meaningful when redaction_active = true.
  // REDACTION_MODE_OVERLAY is always used in v1 (capture surfaces are deferred post-v1).
  // The compositor selects REDACTION_MODE_RENDER_SKIP automatically when
  // ChromeState.capture_surface_active is true. Agents never observe this field.
  RedactionMode redaction_mode = 5;
}

// Redaction rendering strategy (§3.4.1).
// V1 always uses OVERLAY; RENDER_SKIP is reserved for post-v1 capture surface support.
enum RedactionMode {
  REDACTION_MODE_UNSPECIFIED = 0;
  REDACTION_MODE_OVERLAY = 1;      // Tile content rendered; placeholder drawn on top by chrome pass.
  REDACTION_MODE_RENDER_SKIP = 2;  // Tile content not rendered; placeholder fills tile bounds in content pass.
}

enum ViewerClass {
  VIEWER_CLASS_UNSPECIFIED = 0;
  VIEWER_CLASS_OWNER = 1;
  VIEWER_CLASS_HOUSEHOLD = 2;
  VIEWER_CLASS_GUEST = 3;
  VIEWER_CLASS_UNKNOWN = 4;
  VIEWER_CLASS_NOBODY = 5;
}
```

### 7.3 Override Event Types

Override events are emitted by the input/chrome layer when a viewer override is activated. They are consumed by the control plane (which applies the state change) and logged to the audit stream. The audit stream is defined in §7.8 (Shell Audit Events). Each `OverrideEvent` is the source of one `ShellAuditEvent` — the control plane translates the override into the corresponding audit event type and emits it to the telemetry thread before applying the state change.

```protobuf
// Internal — not agent-accessible.
message OverrideEvent {
  oneof event {
    DismissTileEvent dismiss_tile = 1;
    DismissAllEvent dismiss_all = 2;
    FreezeToggleEvent freeze_toggle = 3;
    MuteToggleEvent mute_toggle = 4;
    SafeModeEntryEvent safe_mode_entry = 5;
    SafeModeExitEvent safe_mode_exit = 6;
  }
  uint64 timestamp_mono_us = 10;  // Monotonic clock (RFC 0003 §1.3 `_mono_us` convention; µs since arbitrary epoch).
  OverrideTrigger trigger = 11;
}

enum OverrideTrigger {
  OVERRIDE_TRIGGER_UNSPECIFIED = 0;
  KEYBOARD_SHORTCUT = 1;      // User pressed a keyboard shortcut (Ctrl+Shift+Escape, etc.)
  POINTER_GESTURE = 2;        // User clicked/tapped/swiped a chrome control
  AUTO_CRITICAL_ERROR = 3;    // Runtime entered safe mode automatically on critical failure
}

message DismissTileEvent {
  SceneId tile_id = 1;
}

message DismissAllEvent {}

message FreezeToggleEvent {
  bool freeze_active = 1;  // true = entering freeze, false = exiting freeze.
}

message MuteToggleEvent {
  optional SceneId tile_id = 1;  // absent = global mute.
  bool muted = 2;
}

message SafeModeEntryEvent {
  SafeModeEntryReason reason = 1;
  optional string error_detail = 2;  // populated for CRITICAL_ERROR trigger.
}

enum SafeModeEntryReason {
  SAFE_MODE_ENTRY_REASON_UNSPECIFIED = 0;
  VIEWER_ACTION = 1;      // User triggered safe mode via shortcut or chrome control
  CRITICAL_ERROR = 2;     // Runtime auto-entered safe mode on unrecoverable error
}

message SafeModeExitEvent {}
```

### 7.4 Override State and Viewer Prompt State Types

`OverrideState` captures the current active override conditions (freeze, mute). It is a snapshot read by the chrome render pass each frame.

```protobuf
// Internal — not agent-accessible.
message OverrideState {
  bool freeze_active = 1;          // Scene is frozen (Ctrl+Shift+F active).
  bool global_mute_active = 2;     // Global audio mute active.
  repeated SceneId muted_tile_ids = 3;  // Per-tile muted tile IDs.
}
```

`ViewerPromptState` tracks whether the "Who's Watching?" identification prompt is currently displayed.

```protobuf
// Internal — not agent-accessible.
message ViewerPromptState {
  repeated ViewerIdentityChoice choices = 1;  // Selectable identities.
  uint64 timeout_at_us = 2;  // Monotonic timestamp (RFC 0003 §1.1 uint64 µs); prompt auto-dismisses at this time.
}

message ViewerIdentityChoice {
  string label = 1;         // Human-readable identity label (e.g., "Alice", "Guest").
  ViewerClass viewer_class = 2;  // Viewer class that will be applied if selected.
}
```

### 7.5 Safe Mode State Machine Type

```protobuf
// Internal — not agent-accessible.
message SafeModeState {
  SafeModePhase phase = 1;
  optional SafeModeEntryReason entry_reason = 2;  // Typed to match SafeModeEntryEvent.reason (§7.3).
  optional uint64 entered_at_us = 3;               // Monotonic timestamp (RFC 0003 §1.1 uint64 µs).
}

enum SafeModePhase {
  SAFE_MODE_PHASE_NORMAL = 0;
  SAFE_MODE_PHASE_ACTIVE = 1;
  // EMERGENCY_FALLBACK is shown in §5.3 state diagram as the terminal state when
  // safe mode itself cannot render (GPU loss). It is not represented here because
  // the runtime cannot maintain `ChromeState` in that condition — it degrades to
  // an OS-level signal (blank screen or OS notification) outside this state machine.
  // The protobuf state machine therefore has only NORMAL and ACTIVE.
}
```

### 7.6 Chrome Render Commands

The compositor's chrome render pass is driven by a sequence of `ChromeRenderCmd` values derived from `ChromeState` at frame time. These are not persisted — they are computed each frame.

```protobuf
// Internal — not agent-accessible.
message ChromeRenderCmd {
  oneof cmd {
    TabBarRenderCmd tab_bar = 1;
    BadgeRenderCmd badge = 2;
    RedactionPlaceholderCmd redaction = 3;
    BudgetWarningBorderCmd budget_warning = 4;
    OverrideControlsRenderCmd override_controls = 5;
    SafeModeOverlayCmd safe_mode_overlay = 6;
    PrivacyIndicatorRenderCmd privacy_indicator = 7;
    ViewerPromptRenderCmd viewer_prompt = 8;
    FreezeIndicatorRenderCmd freeze_indicator = 9;
  }
}

// Overlay for safe mode — references only theme colors and font atlas.
// No scene graph entities.
message SafeModeOverlayCmd {
  string banner_text = 1;       // "Safe Mode — All agent sessions paused."
  string resume_button_label = 2; // "Resume"
  bool resume_button_focused = 3; // true by default on safe mode entry.
  ViewerClass viewer_class = 4;  // For viewer class icon in overlay.
}
```

### 7.7 Tab Bar Internal State

```protobuf
// Internal — not agent-accessible.
message TabBarState {
  TabBarPosition position = 1;
  repeated TabEntry tabs = 2;
  SceneId active_tab_id = 3;
  uint32 scroll_offset_px = 4;   // Horizontal scroll offset for overflow (non-negative logical pixels).
  bool overflow_active = 5;
  uint32 hidden_tab_count = 6;   // Count of off-screen tabs (non-negative).
}

enum TabBarPosition {
  TAB_BAR_POSITION_TOP = 0;
  TAB_BAR_POSITION_BOTTOM = 1;
  TAB_BAR_POSITION_HIDDEN = 2;
}

message TabEntry {
  SceneId tab_id = 1;
  string  name   = 2;
  // Active tab is identified by TabBarState.active_tab_id; no redundant is_active field.
}
```

### 7.8 Shell Audit Events

Shell audit events record every human override action and viewer-context change for operator review, compliance analysis, and post-hoc debugging. This section defines the event envelope, the typed event set, the emission path, and the privacy constraints that govern the audit log.

#### 7.8.1 Purpose and Scope

The audit log answers the questions:
- "What override actions did the viewer take, and when?"
- "Why did an agent get disconnected?"
- "When did the viewer context change, and from what to what?"

The log is not a surveillance tool. It must contain only the information an operator legitimately needs to diagnose runtime behavior. Viewer identity details are explicitly excluded (see §7.8.4 Privacy Constraint).

#### 7.8.2 Audit Event Envelope

Every shell audit event is wrapped in a `ShellAuditEvent` envelope:

```protobuf
// Internal — not agent-accessible. Never sent to any agent.
// Emitted to the telemetry thread (RFC 0002 §2.5) as an AuditRecord.
message ShellAuditEvent {
  // Monotonic timestamp at which the event was generated, in microseconds
  // (RFC 0003 §1.3 `_mono_us` convention). Set by the control plane immediately
  // before emitting the event — before the state change is applied.
  uint64 timestamp_mono_us = 1;

  // How the override was triggered (keyboard shortcut, pointer gesture, or
  // automatic runtime action). Uses the same OverrideTrigger enum as §7.3
  // to keep the audit log aligned with the override event model.
  // OVERRIDE_TRIGGER_UNSPECIFIED is used for events that have no direct
  // OverrideEvent source (e.g., VIEWER_CLASS_CHANGED driven by the
  // auto-detection pipeline rather than an explicit viewer action).
  OverrideTrigger trigger = 2;

  // IDs of tiles directly affected by this event. Absent when the event is
  // not tile-scoped (e.g., SAFE_MODE_ENTERED, VIEWER_CLASS_CHANGED).
  // Uses SceneId (RFC 0001 §1.1) — the same type used in OverrideEvent.
  repeated SceneId affected_tile_ids = 3;

  // IDs of sessions directly affected. Absent when the event is not
  // session-scoped. Session IDs are the same string identifiers used in
  // RFC 0005. Not present for VIEWER_CLASS_CHANGED or prompt events.
  repeated string affected_session_ids = 4;

  // The specific audit event payload.
  oneof event {
    AuditTileDismissed      tile_dismissed       = 10;
    AuditAllDismissed       all_dismissed        = 11;
    AuditSafeModeEntered    safe_mode_entered    = 12;
    AuditSafeModeExited     safe_mode_exited     = 13;
    AuditFreezeActivated    freeze_activated     = 14;
    AuditFreezeDeactivated  freeze_deactivated   = 15;
    AuditViewerClassChanged viewer_class_changed = 16;
    AuditViewerPromptShown  viewer_prompt_shown  = 17;
    AuditViewerPromptResolved viewer_prompt_resolved = 18;
  }
}
```

#### 7.8.3 Event Payload Types

Each payload type carries only the minimum fields needed to make the event useful for diagnostics. No viewer identity details are included in any payload (see §7.8.4).

```protobuf
// A single tile was dismissed by the viewer.
// affected_tile_ids in the envelope carries the dismissed tile's SceneId.
message AuditTileDismissed {}

// All tiles were dismissed (Dismiss All action, §4.2).
// This is emitted once for the Dismiss All action; it does not expand into
// individual AuditTileDismissed events. affected_tile_ids carries the full
// set of tile SceneIds that were dismissed.
message AuditAllDismissed {
  uint32 tile_count = 1;  // Number of tiles dismissed. Matches len(affected_tile_ids).
}

// Safe mode was entered (§5.2).
message AuditSafeModeEntered {
  SafeModeEntryReason reason = 1;  // VIEWER_ACTION or CRITICAL_ERROR (§7.3).
  // error_detail is included only for CRITICAL_ERROR — never for VIEWER_ACTION.
  // This mirrors SafeModeEntryEvent.error_detail (§7.3) to aid debugging.
  // It must not contain viewer identity information.
  optional string error_detail = 2;
  uint32 sessions_suspended = 3;  // Count of sessions suspended on entry.
}

// Safe mode was exited (§5.5).
message AuditSafeModeExited {
  // Duration in microseconds from safe mode entry to exit.
  // Computed as: exit timestamp_mono_us − SafeModeState.entered_at_us.
  uint64 duration_us = 1;
  uint32 sessions_resumed = 2;  // Count of sessions resumed on exit.
}

// Scene freeze was activated (Ctrl+Shift+F pressed with freeze_active=false).
message AuditFreezeActivated {}

// Scene freeze was deactivated (Ctrl+Shift+F pressed with freeze_active=true).
message AuditFreezeDeactivated {
  // Duration in microseconds the freeze was active.
  uint64 duration_us = 1;
}

// The viewer class changed (§6.2). Contains only the class transition —
// never viewer identity, name, face recognition confidence, or authentication
// details. See §7.8.4.
message AuditViewerClassChanged {
  ViewerClass old_class = 1;
  ViewerClass new_class = 2;
}

// The "Who's Watching?" identification prompt was shown (§6.3).
message AuditViewerPromptShown {
  uint32 choice_count = 1;  // Number of identity choices presented.
  uint64 timeout_at_us = 2; // Monotonic timestamp when the prompt will auto-dismiss.
}

// The "Who's Watching?" prompt was resolved (selected or timed out).
message AuditViewerPromptResolved {
  AuditPromptResolution resolution = 1;
  // viewer_class_after: the class applied after resolution.
  // This is the only viewer-state field permitted — it is the same
  // class value already visible via VIEWER_CLASS_CHANGED.
  ViewerClass viewer_class_after = 2;
}

enum AuditPromptResolution {
  AUDIT_PROMPT_RESOLUTION_UNSPECIFIED = 0;
  VIEWER_SELECTED = 1;   // Viewer tapped a named identity.
  GUEST_SELECTED  = 2;   // Viewer tapped the "Guest" fallback.
  TIMED_OUT       = 3;   // Prompt expired; runtime applied lowest-confidence class.
  DISMISSED       = 4;   // Viewer explicitly dismissed the prompt without selecting.
}
```

#### 7.8.4 Privacy Constraint

The audit log must not become a viewer surveillance record. The following data must **never** appear in any `ShellAuditEvent` field:

- Viewer name, username, or account identifier.
- Face recognition confidence score or biometric features.
- Authentication method, credential type, or PIN attempt details.
- Device identifiers associated with the viewer.
- Geolocation or physical presence information.

The `AuditViewerClassChanged` event carries only `old_class` and `new_class` (both `ViewerClass` enum values). The `AuditViewerPromptResolved` event carries only the resulting `ViewerClass` and a resolution enum. These are the only viewer-state fields permitted in audit events, and they are the same class values already visible in `ChromeState.viewer_class` — they add no new identity information.

This constraint is enforced at the call site: the control plane must build `ShellAuditEvent` payloads exclusively from `OverrideEvent` fields and `ViewerClass` transitions, never from raw viewer-detection pipeline data.

#### 7.8.5 Emission Path

Shell audit events travel a dedicated path to the telemetry thread. They are never sent to agents.

```
Input/chrome layer
  → OverrideEvent (§7.3) or viewer-context-change notification
    → Control plane constructs ShellAuditEvent
      → Emits to telemetry thread as AuditRecord
        → Telemetry thread serializes to JSON and writes to sink
```

**Channel:** Audit events are sent via the existing `TelemetryRecord` channel (RFC 0002 §2.5, capacity 256, ring-buffer semantics — oldest dropped under overflow). They are wrapped in a new `AuditRecord` variant of `TelemetryRecord`:

```rust
// Added to TelemetryRecord as an additional field.
// Audit events are low-frequency (bounded by human interaction rate) and
// will not materially affect the telemetry channel's overflow budget.
pub struct AuditRecord {
    pub event: ShellAuditEvent,
}
```

The `TelemetryRecord` channel already carries frame metrics and timing records. Audit events ride the same channel because:
1. The telemetry thread is the correct consumer — it owns the structured-output sink.
2. A separate channel would require the control plane to hold an additional channel reference with no benefit.
3. Audit events are human-rate (< 1/sec in normal operation) and add negligible channel pressure.

**Ordering guarantee:** The control plane emits the `ShellAuditEvent` *before* applying the state change (e.g., before updating `ChromeState`, before dispatching `SessionSuspended` to agents). This ensures the audit log reflects the viewer's intent even if the state change fails. If the telemetry channel is full and the event is dropped, the `telemetry_overflow_count` counter in `TelemetryRecord` is incremented — operators can detect dropped audit events via this counter.

**Consumption:** The telemetry thread serializes `AuditRecord` events to the configured sink (stdout / file / remote endpoint) as structured JSON objects, distinct from per-frame metric records. Operators can filter by record type:

```json
{
  "record_type": "shell_audit",
  "timestamp_mono_us": 1234567890,
  "trigger": "KEYBOARD_SHORTCUT",
  "event_type": "safe_mode_entered",
  "payload": {
    "reason": "VIEWER_ACTION",
    "sessions_suspended": 3
  }
}
```

**Agents never receive audit events.** There is no gRPC RPC, no MCP tool, and no `EventNotification` variant that exposes audit events to agents. The `EventNotification` channel (RFC 0002 §2.6) carries scene and session events for agents — audit events are routed exclusively to the telemetry thread, not to the `EventNotification` channel.

#### 7.8.6 Source Mapping

Each `ShellAuditEvent` type is derived from a specific source:

| Audit Event | Source |
|---|---|
| `TILE_DISMISSED` | `DismissTileEvent` in `OverrideEvent` (§7.3) |
| `ALL_DISMISSED` | `DismissAllEvent` in `OverrideEvent` (§7.3) |
| `SAFE_MODE_ENTERED` | `SafeModeEntryEvent` in `OverrideEvent` (§7.3) |
| `SAFE_MODE_EXITED` | `SafeModeExitEvent` in `OverrideEvent` (§7.3) |
| `FREEZE_ACTIVATED` | `FreezeToggleEvent` (freeze_active=true) in `OverrideEvent` (§7.3) |
| `FREEZE_DEACTIVATED` | `FreezeToggleEvent` (freeze_active=false) in `OverrideEvent` (§7.3) |
| `VIEWER_CLASS_CHANGED` | Viewer-context-change notification from the viewer-detection pipeline (§6.2); not an `OverrideEvent`. Trigger is `OVERRIDE_TRIGGER_UNSPECIFIED` when driven by auto-detection, `POINTER_GESTURE` when driven by "Who's Watching?" prompt selection. |
| `VIEWER_PROMPT_SHOWN` | "Who's Watching?" prompt displayed by the chrome layer (§6.3) |
| `VIEWER_PROMPT_RESOLVED` | "Who's Watching?" prompt dismissed or timed out (§6.3) |

`MuteToggleEvent` (§7.3) does not have a corresponding audit event in v1 — mute is a media-layer action whose full audit semantics depend on media integration (deferred post-v1). If mute is implemented in a future revision, a `MUTE_TOGGLED` audit event type should be added following the same pattern.

---

## 8. Interaction with Other RFCs

| RFC | Relationship |
|-----|-------------|
| RFC 0001 (Scene Contract) | Chrome renders above the scene graph. `SceneId` is used to key `TileBadgeState`. Chrome elements are not `SceneId`-addressable. |
| RFC 0002 (Runtime Kernel) | Chrome render pass executes as Stage 6 in the compositor thread's per-frame pipeline (after content tile compositing). `ChromeState` is read under `Arc<RwLock<ChromeState>>` read lock at the start of the chrome render pass (see §7.1 synchronization contract). **Audit event path:** Shell audit events (§7.8) are emitted by the control plane to the telemetry thread via the existing `TelemetryRecord` channel (RFC 0002 §2.5, ring-buffer capacity 256). The `AuditRecord` wrapper rides the same channel as frame metrics; audit events are human-rate and add negligible channel pressure. **GPU failure path:** RFC 0002 §7.3 and this RFC §5.1 previously conflicted on GPU device loss response (RFC 0002 said graceful shutdown; RFC 0007 said safe mode entry). RFC 0009 §5 resolves this: Phase 1 attempts surface reconfiguration (RFC 0002 §7.3 steps 1–3); Phase 2 enters safe mode before shutdown if reconfiguration fails. RFC 0002 §7.3 step 4 must be updated per RFC 0009 §5.3. |
| RFC 0003 (Timing Model) | Override events carry `timestamp_mono_us` using the monotonic clock (RFC 0003 §1.3 `_mono_us` convention). Override execution is frame-bounded — effects appear within one frame of the event. Shell audit events (§7.8) also carry `timestamp_mono_us` using the same convention. |
| RFC 0004 (Input Model) | Chrome elements are the highest-priority hit-test layer (RFC 0001 §5.2 traversal order: chrome always wins). Chrome shortcuts are evaluated before tile hit-testing by RFC 0004 §8 (Event Dispatch Protocol). In safe mode, the input model routes all events to the chrome layer exclusively. |
| RFC 0005 (Session Protocol) | `SessionSuspended` (field 45) and `SessionResumed` (field 46) are defined in RFC 0005 §3.7 (added in Round 12, rig-5vq.29). The safe mode protocol gap documented in earlier rounds is resolved. Lease suspension on safe mode entry and lease revocation on tile dismiss both use the existing `LeaseResponse` / `lease_changes` subscription category (RFC 0005 §3.2, §7.1). **Freeze backpressure (rig-9v1):** Two new error codes must be added to RFC 0005 §3.5 `RuntimeError.ErrorCode` enum: `MUTATION_QUEUE_PRESSURE` (sent at 80% queue capacity per session during freeze or any high-pressure scenario) and `MUTATION_DROPPED` (sent per shed non-transactional mutation on overflow). Both are carried in `MutationResult.error` on the existing `mutation_result = 30` field — no new `SessionMessage` payload fields are required. These codes must be added to the `ErrorCode` enum and documented in the §3.2 server→client table. |
| RFC 0008 (Lease Governance) | Authoritative specification for lease lifecycle during safe mode. Safe mode entry suspends all `ACTIVE` leases (not revokes them — see §4.2 and RFC 0008 §3.4). Tile dismiss (§4.1) transitions a lease to `REVOKED`. TTL clock pauses during suspension (RFC 0008 §3.6). The revoke/suspend distinction is load-bearing: suspended leases survive safe mode exit intact. |
| RFC 0009 (Policy Arbitration) | Human override (§4, §5) implements Step 1 of RFC 0009's seven-step arbitration stack — unconditional, preempts all other policy steps. RFC 0009 §5 resolves the GPU failure path conflict with RFC 0002 (see RFC 0002 row above). RFC 0009 §2.1 specifies the `OverrideCommandQueue` contract for how override actions preempt in-flight mutation intake. |

---

## 9. V1 Scope

### In V1

- Tab bar (top/bottom/hidden position, overflow scroll, keyboard shortcuts).
- System status indicator (session health, agent count, viewer class icon).
- Disconnection badge.
- Staleness badge.
- Redaction placeholder.
- Budget warning border.
- Dismiss tile (X button, swipe gesture on touch).
- Dismiss all / safe mode entry.
- Freeze scene.
- Mute control surface (defined, rendered as disabled, non-functional without media).
- Safe mode overlay with Resume control.
- Privacy indicator (viewer class icon).
- Viewer context transition (immediate, 300ms cross-fade for icon).
- Optional "Who's Watching?" prompt (disabled by default).
- All `ChromeState`, badge, override event, and render command protobuf types.
- Shell audit events (§7.8): `ShellAuditEvent` envelope, all nine event payload types, emission via `TelemetryRecord` channel, structured JSON output by telemetry thread.

### Deferred (Post-V1)

- Animated tile dismissal transitions (slide-out).
- Per-tile mute functionality (depends on media integration, RFC post-v1); mute audit event (`MUTE_TOGGLED`) deferred alongside mute functionality.
- Full tab picker UI (keyboard invoked list of all tabs).
- Granular viewer authentication UI flows (biometric, PIN).
- Remote chrome state inspection via admin tooling.
- Theme customization API (chrome renders with a fixed default theme in v1).
- Remote audit log streaming endpoint (operators can consume the audit log via file/stdout in v1; a live streaming API is post-v1).
- Render-skip redaction activation (§3.4.1): `capture_surface_active` is always false in v1. The capture surface registry and the `REDACTION_MODE_RENDER_SKIP` path are not implemented in v1. The architectural separation of content rendering and chrome rendering required to support render-skip MUST be preserved by the v1 implementation.
- **Operator diagnostics overlay** (`diagnostics_active` — §7.1 field 8): A chrome-layer overlay for operator and developer use that surfaces per-tile diagnostic information. Design intent recorded here to prevent incompatible implementation decisions (e.g., building diagnostics as an agent-facing API instead of a chrome overlay).

  **Activation.** A keyboard shortcut (e.g., `Ctrl+Shift+D`) or a configurable runtime toggle sets `ChromeState.diagnostics_active`. Toggling is a local display-node action; it does not generate any agent-visible event or mutation.

  **Content (per-tile diagnostic panel).** When active, each visible tile renders an overlay panel showing:
  - Current tile state: `redacted`, `shed`, `stale`, `budget-warned`, `frozen`, or `disconnected`.
  - Reason for that state (e.g., "viewer class: Guest, tile classification: Private" for redaction; "degradation level 4, tile priority was lowest" for shed; "last update 45 s ago, threshold 30 s" for stale; "agent at 85% of texture memory budget" for budget-warned).
  - Relevant thresholds (staleness threshold, budget percentage, degradation level).
  - Agent session ID for the tile's lease holder (non-sensitive — session ID only, not viewer context).
  - Last frame timestamp and frame timing statistics (frame duration, missed deadlines).

  **Rendering.** Chrome-layer overlay — rendered in the same chrome pass as all other chrome elements (Stage 6, RFC 0002 §3.1). The diagnostics panel sits above tile content and other badge icons. Same composition guarantees and z-order rules apply.

  **Privacy constraint.** The overlay MUST NOT expose viewer identity, viewer-detection pipeline data, or any information prohibited by §7.8.4 (Privacy Constraints on Audit Events). It MAY show the viewer's current `ViewerClass` enum value (already visible in `ChromeState.viewer_class`) to explain a redaction decision, but MUST NOT show viewer name, face-recognition score, biometric data, or geolocation. Example allowed display: "viewer class: Guest, tile classification: Private". Example prohibited display: "viewer: John Smith".

  **Agent isolation.** The diagnostics overlay is entirely chrome-rendered. It is never forwarded through any agent-facing RPC, never included in scene topology visible to agents, and never reachable via MCP or gRPC agent APIs. `ChromeState.diagnostics_active` is an internal runtime field (same access class as all `ChromeState` fields — see §7.1).

---

## 10. Open Questions

1. **Tab picker in V1?** The tab bar overflow count (`+N`) is insufficient for displays with many tabs. The tab picker (§2.4) may be required for v1 usability on large deployments. Decision deferred to implementation.

2. **Budget warning threshold.** 80% of session budget is the proposed threshold for the budget warning badge (§3.5). This may be too sensitive or too permissive for typical workloads. Requires empirical tuning once resource budget enforcement (RFC 0002) is implemented and measurable.

3. **Viewer prompt design.** The "Who's Watching?" prompt (§6.3) is disabled by default and lightly specified. Its interaction design (number of selectable identities, timeout behavior, animation) will need more definition before implementation. This is acceptable to leave for the implementation RFC.

4. **Safe mode keyboard shortcut conflict.** `Ctrl+Shift+Escape` is also used by some OS task managers. Alternative shortcut should be evaluated on each platform. The shortcut is configurable, so this is a configuration guidance question rather than a design question.
