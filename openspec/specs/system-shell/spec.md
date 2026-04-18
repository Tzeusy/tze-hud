# System Shell Specification

Source: RFC 0007 (System Shell)
Domain: GOVERNANCE

## Purpose

Defines the runtime's chrome layer: the viewer-owned, agent-inaccessible set of
controls, badges, and overlays that sit unconditionally above all agent content.
Covers safe mode, freeze/mute/dismiss-all override controls, viewer class display,
privacy-safe capture architecture, disconnection and budget badges, backpressure
signals, and the v1 minimal CLI diagnostic surface. Also defines the boundary
between the chrome layer and text stream portal surfaces.

---

## Requirements

### Requirement: Chrome Layer Always On Top
The chrome layer MUST render above all agent content in every frame. The compositor MUST render three ordered layers back to front: background, content, chrome. The chrome layer MUST share the same wgpu pipeline as the content layer (not a separate window, overlay, or GPU context). There MUST be no frame-level gap in which agent content is visible without chrome.
Source: RFC 0007 §1.1
Scope: v1-mandatory

#### Scenario: Chrome above all agent tiles
- **WHEN** an agent creates a tile with the maximum z-order value
- **THEN** the chrome layer (tab bar, badges, override controls) renders on top of that tile

#### Scenario: No frame without chrome
- **WHEN** the compositor produces a frame containing agent tiles
- **THEN** the chrome render pass executes in the same frame, after all content tiles are composited

### Requirement: Agent Exclusion From Chrome
Agents MUST have no API to read, write, occlude, suppress, or receive events from the chrome layer. Chrome elements MUST NOT appear in scene topology queries. Chrome elements MUST NOT be addressable via `SceneId`.
Source: RFC 0007 §1.2
Scope: v1-mandatory

#### Scenario: Scene topology excludes chrome
- **WHEN** an agent queries the scene topology via gRPC or MCP
- **THEN** no chrome layer elements (tab bar, badges, overlay controls) appear in the result

#### Scenario: No tile can occlude chrome
- **WHEN** an agent requests a tile z-order value exceeding all other tiles
- **THEN** the tile renders below the chrome layer

### Requirement: Text Stream Portals Remain Outside Chrome
Text stream portal identity, transcript state, and interaction affordances SHALL remain outside the chrome layer unless a future shell-specific capability explicitly defines a runtime-owned portal shell surface. The current system shell MUST NOT expose agent-specific portal identities or transcript state.
Source: change text-stream-portals
Scope: v1-mandatory

#### Scenario: Portal status does not become shell status metadata
- **WHEN** one or more text stream portals are active
- **THEN** the shell status area MAY expose aggregate system health only and SHALL NOT expose portal-specific identities, transcript previews, or agent-owned controls

### Requirement: Chrome Rendering Independence
Chrome rendering MUST NOT depend on any agent state. The chrome render pass MUST read exclusively from the runtime's `ChromeState` struct, the tab list, viewer context, and cached layout geometry. If all agents crash simultaneously, chrome MUST render correctly on the next frame.
Source: RFC 0007 §1.4
Scope: v1-mandatory

#### Scenario: All agents crash
- **WHEN** all agent sessions terminate simultaneously
- **THEN** the chrome layer (tab bar, badges, override controls) renders correctly on the next frame

### Requirement: ChromeState Synchronization
`ChromeState` MUST be protected by `Arc<RwLock<ChromeState>>` (or equivalent lock-free mechanism). The control plane (network thread) MUST hold the write lock only for short-lived updates; the compositor thread MUST acquire a read lock at the start of the chrome render pass and release it before GPU submit.
Source: RFC 0007 §7.1
Scope: v1-mandatory

#### Scenario: Concurrent read and write
- **WHEN** the control plane updates a badge state while the compositor is generating chrome render commands
- **THEN** data races do not occur and the compositor reads either the pre-update or post-update snapshot atomically

### Requirement: Tab Bar Rendering
The tab bar MUST display tab names, an active tab indicator, and a tab count badge when tabs overflow the available width. The tab bar position MUST be configurable (`top`, `bottom`, `hidden`). When `hidden`, keyboard shortcuts for tab switching MUST remain active. The tab bar MUST NOT display agent-supplied metadata.
Source: RFC 0007 §2.1, §2.2
Scope: v1-mandatory

#### Scenario: Tab bar overflow
- **WHEN** there are more tabs than fit in the tab bar width
- **THEN** a `+N` indicator appears at the trailing end showing the count of off-screen tabs and the active tab is always scrolled into view

#### Scenario: Hidden tab bar
- **WHEN** `tab_bar_position = "hidden"`
- **THEN** the tab bar is not rendered but `Ctrl+Tab`, `Ctrl+Shift+Tab`, and `Ctrl+1`-`Ctrl+9` shortcuts still work

### Requirement: Tab Keyboard Shortcuts
Tab navigation MUST be available via keyboard shortcuts: `Ctrl+Tab` (next), `Ctrl+Shift+Tab` (previous), `Ctrl+1` through `Ctrl+8` (specific tab), `Ctrl+9` (last tab). Shortcuts MUST be handled by the input model's event dispatch protocol before tile hit-testing. Shortcut events MUST never be routed to any agent.
Source: RFC 0007 §2.3
Scope: v1-mandatory

#### Scenario: Ctrl+Tab switches to next tab
- **WHEN** the user presses `Ctrl+Tab`
- **THEN** the active tab switches to the next tab and no agent receives the keyboard event

### Requirement: Dismiss Tile Override
The dismiss tile control MUST appear as an X button in the top-right corner of a tile on hover (or on touch-hold). Activating it MUST: (1) immediately revoke the tile's lease, (2) remove the tile from the scene, (3) free the tile's resources, (4) send a lease revocation notification to the owning agent via `LeaseResponse` with `result = REVOKED` and `revoke_reason = VIEWER_DISMISSED` (RFC 0008 `RevokeReason` enum). The dismiss MUST work even if the owning agent is disconnected or closing. The agent MAY re-request a lease afterwards (dismissal is not a permanent ban).
Source: RFC 0007 §4.1
Scope: v1-mandatory

#### Scenario: Dismiss an active tile
- **WHEN** the viewer clicks the X button on an active tile
- **THEN** the tile is removed from the scene within one frame, the lease is revoked, and the agent receives `LeaseResponse` with `REVOKED` and `revoke_reason = VIEWER_DISMISSED`

#### Scenario: Dismiss tile of disconnected agent
- **WHEN** the viewer dismisses a tile whose agent is in the reconnect grace period
- **THEN** the grace period is cancelled, the tile is removed, and resources are freed

### Requirement: Safe Mode Entry Protocol
Safe mode MUST be enterable by: (1) explicit viewer action via `Ctrl+Shift+Escape` or the "Dismiss All" chrome control, (2) automatic entry on critical runtime error (GPU device loss, scene graph corruption, unrecoverable render failure). On safe mode entry: all active leases MUST be suspended (NOT revoked), all agent sessions MUST receive `SessionSuspended` with reason `safe_mode`, the safe mode overlay MUST be rendered, and all input MUST be captured by the chrome layer.
Source: RFC 0007 §4.2, §5.1, §5.2
Scope: v1-mandatory

#### Scenario: Manual safe mode entry
- **WHEN** the viewer presses `Ctrl+Shift+Escape`
- **THEN** all active leases are suspended, agents receive `SessionSuspended`, the safe mode overlay appears, and all input routes to chrome

#### Scenario: Auto safe mode on GPU loss
- **WHEN** the compositor detects `wgpu::DeviceError::Lost`
- **THEN** the runtime enters safe mode with `SafeModeEntryReason = CRITICAL_ERROR`

#### Scenario: Shell override applies to portal tiles
- **WHEN** the viewer dismisses a portal tile or enters safe mode
- **THEN** the system shell SHALL override that portal surface under the same unconditional rules as any other content-layer tile

### Requirement: Safe Mode Overlay
The safe mode overlay MUST display: a centered banner ("Safe Mode -- All agent sessions paused."), a prominent "Resume" button, the current viewer class indicator, and no agent branding or content. The overlay MUST render correctly even if the scene graph is corrupted, because it reads exclusively from `ChromeState` and references only theme colors and a font atlas.
Source: RFC 0007 §5.2, §5.4
Scope: v1-mandatory

#### Scenario: Safe mode overlay rendering
- **WHEN** safe mode is entered
- **THEN** a full-viewport overlay appears with "Safe Mode" banner, "Resume" button, viewer class icon, and no agent content

#### Scenario: Overlay with corrupted scene graph
- **WHEN** safe mode is entered due to scene graph corruption
- **THEN** the safe mode overlay still renders correctly because it does not depend on the scene graph

### Requirement: Safe Mode Exit
Safe mode MUST exit only by explicit viewer action: clicking/tapping "Resume", pressing `Enter`/`Space` (resume button has focus by default), or `Ctrl+Shift+Escape` (toggle). On exit: the overlay MUST be dismissed, all SUSPENDED leases MUST transition back to ACTIVE (RFC 0008 §3.3), each session MUST receive `SessionResumed` (RFC 0005 §3.7; empty message — receipt is the signal to resume mutations), each affected lease MUST receive `LeaseResume` (RFC 0008 §7.3) with `adjusted_expires_at_wall_us` and `suspension_duration_us`, TTL clocks MUST resume with elapsed suspension time excluded, staleness badges MUST clear within 1 frame, agent mutations MUST be accepted again, and the compositor MUST resume rendering from the current scene state. Agents MUST NOT re-request leases — lease identity, capability scope, and resource budget are preserved across the ACTIVE → SUSPENDED → ACTIVE cycle. Note: `SessionResumed` and `LeaseResume` are separate messages: `SessionResumed` is a session-level signal (no payload); `LeaseResume` is per-lease and carries TTL adjustment fields.
Source: RFC 0007 §5.5, RFC 0008 §3.3, RFC 0005 §3.7
Scope: v1-mandatory

#### Scenario: Resume from safe mode
- **WHEN** the viewer clicks the "Resume" button
- **THEN** the safe mode overlay is dismissed, SUSPENDED leases transition to ACTIVE, each session receives `SessionResumed` (empty), each lease receives `LeaseResume` with adjusted expiry, staleness badges clear within 1 frame, and mutations are accepted again without agents needing to re-request leases

### Requirement: Safe Mode and Freeze Interaction (Shell State Invariant)
The shell MUST enforce the following state-transition invariant: `safe_mode = true` implies `freeze_active = false`. This invariant holds regardless of the trigger that caused safe mode to activate (explicit viewer action, automatic runtime error, or any other cause). The shell is the sole writer of `OverrideState.freeze_active`; no other subsystem (including policy arbitration) may modify this field. On any safe mode activation: the freeze state MUST be cancelled, the freeze queue MUST be discarded, and `OverrideState.freeze_active` MUST be set to `false` before any other safe mode entry steps execute. If freeze is attempted during safe mode, it MUST be ignored. After safe mode exit, freeze MUST be inactive.
Source: RFC 0007 §5.6
Scope: v1-mandatory

#### Scenario: Safe mode cancels freeze (automatic trigger)
- **WHEN** the scene is frozen and a GPU failure triggers safe mode automatically
- **THEN** freeze is cancelled, the freeze queue is discarded, `OverrideState.freeze_active` is set to `false`, and safe mode overlay appears

#### Scenario: Safe mode cancels freeze (manual trigger)
- **WHEN** the scene is frozen and the viewer manually triggers safe mode via `Ctrl+Shift+Escape`
- **THEN** freeze is cancelled, the freeze queue is discarded, `OverrideState.freeze_active` is set to `false`, and safe mode overlay appears

#### Scenario: Freeze ignored during safe mode
- **WHEN** the viewer presses `Ctrl+Shift+F` while safe mode is active
- **THEN** no effect; safe mode captures all input

### Requirement: Freeze Scene
The freeze action (`Ctrl+Shift+F`) MUST freeze the current scene state. Agent mutations MUST be queued in a bounded per-session queue (default 1000). Queue overflow MUST be traffic-class-aware: transactional mutations MUST never be evicted (gRPC backpressure applied instead), state-stream mutations MUST be coalesced (latest-wins) before eviction, ephemeral mutations MUST be dropped oldest-first. Unfreeze MUST apply queued mutations in submission order.
Source: RFC 0007 §4.3
Scope: v1-mandatory

#### Scenario: Freeze queues mutations
- **WHEN** the viewer presses `Ctrl+Shift+F` and an agent submits mutations
- **THEN** mutations are queued, tile content does not update, and badges continue to render

#### Scenario: Transactional mutations never dropped
- **WHEN** the freeze queue is full and a transactional mutation (e.g., `CreateTile`) is submitted
- **THEN** the mutation is not evicted; gRPC backpressure is applied instead

#### Scenario: Unfreeze applies queued mutations
- **WHEN** the viewer unfreezes the scene
- **THEN** all queued mutations are applied in submission order in the next available frame batch

### Requirement: Freeze Backpressure Signal
Agents MUST NOT be informed that the scene is frozen. Instead, at 80% queue capacity (800/1000 default), the runtime MUST send `MUTATION_QUEUE_PRESSURE` via `RuntimeError` in `MutationResult`. On overflow for non-transactional mutations, the runtime MUST send `MUTATION_DROPPED`. These signals MUST fire for any queue-pressure scenario (not specifically freeze) to avoid leaking viewer state.
Source: RFC 0007 §4.3
Scope: v1-mandatory

#### Scenario: Queue pressure signal
- **WHEN** the per-session freeze queue reaches 80% capacity
- **THEN** the runtime sends `MUTATION_QUEUE_PRESSURE` in the `MutationResult` response

#### Scenario: Mutation dropped signal
- **WHEN** the queue is full and a state-stream mutation is evicted
- **THEN** the runtime sends `MUTATION_DROPPED` for the evicted mutation

### Requirement: Disconnection Badge
When an agent's lease enters the orphaned state (agent disconnected, within grace period), a disconnection badge MUST appear on all affected tiles within one frame. The badge MUST be a dim plug/link-break icon at 70% opacity (static in v1, no animation). The tile's content MUST render at reduced opacity (70%). The badge MUST clear immediately when the agent reconnects and reclaims the lease.
Source: RFC 0007 §3.2
Scope: v1-mandatory

#### Scenario: Disconnection badge appears
- **WHEN** an agent disconnects and its lease enters the grace period
- **THEN** a dim link-break icon appears on all affected tiles within one frame

#### Scenario: Disconnection badge clears on reconnect
- **WHEN** the agent reconnects and reclaims its lease
- **THEN** the disconnection badge is removed immediately

### Requirement: Redaction Placeholder
When a tile's content is redacted due to viewer context, the tile's content area MUST be filled with a neutral placeholder (configurable via `[privacy].redaction_style`; allowed values: `pattern`, `blank`). No agent name, content hint, or icon MUST be shown. Interactive affordances (hit regions) MUST be disabled while redacted. The agent MUST NOT be notified that its tile is redacted. When the viewer context changes to permit the content, the placeholder MUST be removed and tile content composited normally.
Source: RFC 0007 §3.4
Scope: v1-mandatory

#### Scenario: Tile redacted for guest viewer
- **WHEN** a tile with `private` classification is displayed to a viewer with `unknown` class
- **THEN** the tile's content is replaced with a neutral placeholder (pattern or blank, per `[privacy].redaction_style`), hit regions are disabled, and the agent is not notified

#### Scenario: Redaction removed on viewer change
- **WHEN** the viewer context changes from `unknown` to `owner`
- **THEN** the redaction placeholder is removed and the tile's actual content is displayed

### Requirement: Budget Warning Badge
When an agent's resource consumption reaches 80% of its session budget, a budget warning badge MUST appear as a subtle amber border highlight (2px, 70% opacity) on the affected tiles. The badge MUST be removed when consumption drops below 80%.
Source: RFC 0007 §3.5
Scope: v1-mandatory

#### Scenario: Budget warning badge appears
- **WHEN** an agent's texture memory usage reaches 80% of its budget
- **THEN** an amber border highlight appears on all tiles under that agent's lease

### Requirement: Override Control Guarantees
All override controls MUST satisfy: (1) local execution (no network roundtrip, no agent callback), (2) frame-bounded response (visual effect within one frame, <= 16.6ms), (3) unconditional (no agent capability or policy can prevent execution), (4) no agent veto (agents receive notification but cannot undo the override).
Source: RFC 0007 §4.5
Scope: v1-mandatory

#### Scenario: Override is frame-bounded
- **WHEN** the viewer triggers a dismiss action
- **THEN** the tile visually disappears within one frame (16.6ms) of the input event

#### Scenario: Override is unconditional
- **WHEN** the viewer triggers safe mode while an agent holds a valid lease with high priority
- **THEN** safe mode activates regardless of the agent's priority or capabilities

### Requirement: Viewer Class Indicator
The current viewer class MUST be displayed as an icon in the system status area. Icons: Owner (filled circle), Household member (partial circle), Known guest (outline circle), Unknown (question mark), Nobody (dim circle). No text by default; tooltip/long-press reveals the full label. Viewer class transitions MUST use a cross-fade over 300ms (the only animation permitted in v1 chrome).
Source: RFC 0007 §6.1, §6.2
Scope: v1-mandatory

#### Scenario: Viewer class icon displayed
- **WHEN** the viewer class is `owner`
- **THEN** a filled circle icon appears in the system status area

#### Scenario: Viewer class transition animation
- **WHEN** the viewer class changes from `owner` to `unknown`
- **THEN** the icon transitions via cross-fade over 300ms

### Requirement: Agent Isolation for Viewer State
No viewer state MUST be available to agents through any API surface. gRPC scene events MUST NOT include viewer class. MCP tools MUST NOT return viewer class. Tile redaction MUST be silent to agents. The `list_scene` MCP tool and scene topology gRPC responses MUST omit viewer context.
Source: RFC 0007 §6.4
Scope: v1-mandatory

#### Scenario: Agent cannot detect viewer class
- **WHEN** an agent queries the scene topology
- **THEN** the response contains no viewer class information

#### Scenario: Agent cannot detect redaction
- **WHEN** an agent's tile is being redacted
- **THEN** the agent receives no notification and cannot distinguish redacted from non-redacted state

### Requirement: Shell Audit Events
Every human override action and viewer-context change MUST produce a `ShellAuditEvent` emitted to the telemetry thread. Audit events MUST include: `timestamp_mono_us`, `trigger` (keyboard shortcut, pointer gesture, or auto), and affected tile/session IDs where applicable. Audit events MUST never be sent to agents. The audit event set MUST include: `tile_dismissed`, `all_dismissed`, `safe_mode_entered`, `safe_mode_exited`, `freeze_activated`, `freeze_deactivated`, `viewer_class_changed`, `viewer_prompt_shown`, `viewer_prompt_resolved`.
Source: RFC 0007 §7.8
Scope: v1-mandatory

#### Scenario: Dismiss produces audit event
- **WHEN** the viewer dismisses a tile
- **THEN** a `ShellAuditEvent` with `tile_dismissed` payload is emitted to the telemetry thread, including the tile's SceneId and the trigger type

#### Scenario: Audit events never reach agents
- **WHEN** any shell audit event is emitted
- **THEN** it is routed to the telemetry thread only and never appears in any agent-facing gRPC or MCP response

### Requirement: Audit Privacy Constraint
Shell audit events MUST NOT contain: viewer name/username/account identifier, face recognition confidence scores or biometric features, authentication method or credential details, device identifiers, or geolocation data. `AuditViewerClassChanged` MUST carry only `old_class` and `new_class` enum values.
Source: RFC 0007 §7.8.4
Scope: v1-mandatory

#### Scenario: Viewer class change audit contains only class values
- **WHEN** the viewer class changes from `owner` to `household_member`
- **THEN** the `AuditViewerClassChanged` event carries `old_class = OWNER` and `new_class = HOUSEHOLD_MEMBER` with no viewer identity details

### Requirement: System Status Indicator
The tab bar's trailing end MUST contain a system status indicator showing: a health dot (green/amber/red for all connected / some degraded / all disconnected or safe mode), active agent count, current viewer class icon, and a "Dismiss All" affordance accessible via keyboard focus traversal. The indicator MUST NOT expose agent identities or names.
Source: RFC 0007 §2.5
Scope: v1-mandatory

#### Scenario: Status indicator shows agent count
- **WHEN** three agents are connected
- **THEN** the system status indicator shows a green dot and "3 agents" with no agent names

### Requirement: Mute Control Surface
The chrome layer MUST reserve a mute control surface for per-tile and global audio muting. In v1, mute controls MUST be rendered as disabled/greyed (media is deferred). They MUST accept input but take no action (log a noop). Global mute shortcut: `Ctrl+Shift+M`.
Source: RFC 0007 §4.4
Scope: v1-reserved

#### Scenario: Mute control noop in v1
- **WHEN** the viewer presses `Ctrl+Shift+M` in v1
- **THEN** a noop is logged and no media action occurs

### Requirement: V1 Diagnostic Surface
The v1 minimal diagnostic surface MUST provide: scene graph dump, active lease listing, resource utilization per-agent, zone registry state, and telemetry snapshot. These MUST be available via CLI (not GUI). The operator diagnostics overlay (GUI) is deferred to post-v1.
Source: RFC 0007 §9, design.md
Scope: v1-mandatory

#### Scenario: CLI scene graph dump
- **WHEN** an operator requests a scene graph dump via CLI
- **THEN** the full scene graph state is output including all tiles, leases, and zone occupancy

### Requirement: Capture-Safe Redaction Architecture
The compositor MUST keep content rendering and chrome rendering as separable passes. V1 MUST ship overlay-only redaction (`capture_surface_active` always false). Implementations MUST NOT assume overlay-only is permanent. Render-skip redaction (post-v1) MUST be architecturally preserved by maintaining the separation of content and chrome render passes.
Source: RFC 0007 §3.4.1
Scope: v1-mandatory

#### Scenario: Separable render passes
- **WHEN** the compositor renders a frame
- **THEN** content rendering and chrome rendering are executed as separate passes, preserving the architectural separation required for future render-skip redaction

### Requirement: Full TypeScript Inspector
A full TypeScript inspector and admin panel GUI for diagnostics MUST NOT be required for v1. V1 diagnostic access SHALL be CLI-only. The GUI inspector is deferred to post-v1.
Source: RFC 0007 §9
Scope: post-v1

#### Scenario: Deferred inspector
- **WHEN** v1 is deployed
- **THEN** no TypeScript inspector or admin panel GUI is required
