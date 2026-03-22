# RFC 0004 — Input Model

**Status:** Draft
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0003 (Lease Model)
**Authored:** 2026-03-22

---

## Review Changelog

| Round | Date | Reviewer | Focus | Changes |
|-------|------|----------|-------|---------|
| 1 | 2026-03-22 | rig-5vq.23 | Doctrinal alignment deep-dive | DR table: added DR-I3/I4 (input_to_scene_commit, input_to_next_present) from validation.md §3; added DR-I11 (headless testability). §6.1a: new headless testability section. §7.1: fixed `interaction_id` comment (now consistent with RFC 0001 §2.4 "forwarded in events"). §7.3/§9.1: added `interaction_id` field to PointerDownEvent, PointerUpEvent, ClickEvent, DoubleClickEvent. §9.1: removed `HitRegionConfig` (replaced with canonical `HitRegionNode` reference to RFC 0001 §9). §11.2: scroll deferral reframed as requiring pre-implementation resolution (local-first scroll is a doctrine commitment). RFC 0001 §2.4 and §9: unified `HitRegionNode` to include all input-model fields with cross-reference to RFC 0004. |

---

## Summary

This RFC defines the interaction model for tze_hud: how OS-level input events travel from hardware into the runtime, are routed to the correct agent, and produce local visual feedback before any agent roundtrip. It covers the focus model, pointer capture, gesture arbitration, IME composition, accessibility hooks, the local feedback contract, hit-region node primitives, event dispatch protocol, and the protobuf schema for all input messages.

The governing principles are:

- **Local feedback first.** The runtime updates press state, focus rings, and hover highlights in the same frame the event arrives, without waiting for the agent. Remote semantics follow.
- **Runtime arbitrates.** Agents do not race for input. The runtime decides which tile and which node receives each event. Agents do not negotiate directly with each other.
- **Screen is sovereign.** The chrome layer always wins hit-testing. System gestures pass through regardless of agent activity.
- **LLMs must never sit in the frame loop.** Input drives local state immediately; agent callbacks are asynchronous.

---

## Motivation

Without a defined interaction model, every agent implements ad-hoc input handling, local feedback is inconsistent, gesture conflicts are unresolved races, and accessibility is never added because "someone will do it later." The Input RFC makes all of this explicit and testable before any interactive code is written.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-I1 | input_to_local_ack p99 < 4ms | validation.md §3, v1.md §V1 must prove |
| DR-I2 | Hit-test latency < 100μs for 50 tiles | RFC 0001 §5.1 |
| DR-I3 | input_to_scene_commit p99 < 50ms (local agents) | validation.md §3, v1.md §V1 must prove |
| DR-I4 | input_to_next_present p99 < 33ms | validation.md §3, v1.md §V1 must prove |
| DR-I5 | Event dispatch to agent < 2ms from hit-test | this RFC |
| DR-I6 | Gesture recognition < 1ms from final touch event | this RFC |
| DR-I7 | IME composition window update < 1 frame (16.6ms) | this RFC |
| DR-I8 | Accessibility tree sync < 100ms after scene change | this RFC |
| DR-I9 | Keyboard-only navigation for all interactions | presence.md |
| DR-I10 | Platform a11y API support (UIAutomation, NSAccessibility, AT-SPI) | presence.md |
| DR-I11 | All input behavior testable headlessly (no display server required) | validation.md DR-V2, DR-V5 |

---

## 1. Focus Model

### 1.1 Focus Tree

Focus is a property of the scene graph, not of individual agents. At any moment, at most one **focus owner** exists per tab: either the chrome layer (a runtime UI element), or a specific tile, or a specific `HitRegionNode` within a tile.

```
FocusState {
    tab_id: SceneId,
    owner: FocusOwner,
}

enum FocusOwner {
    None,
    Chrome { element: ChromeElement },
    Tile { tile_id: SceneId },
    Node { tile_id: SceneId, node_id: SceneId },
}
```

**Focus resolution rule:** When a tile has focus and it contains a `HitRegionNode` with `accepts_focus: true`, the node is the fine-grained focus owner. Keyboard events target the node first, then bubble to the tile if the node does not consume them. When a tile has focus but no focused node, keyboard events target the tile directly.

**Focus persistence across tabs:** Each tab maintains independent focus state. Switching tabs suspends the current tab's focus and restores the previous tab's focus when returning. The suspended focus is preserved in memory but does not generate events.

```
                     Tab A (active)          Tab B (suspended)
                     ┌──────────────┐        ┌──────────────┐
                     │ FocusOwner:  │        │ FocusOwner:  │
                     │ Node(T2,N1)  │        │ Tile(T5)     │  ← preserved,
                     └──────────────┘        └──────────────┘    no events
```

### 1.2 Focus Acquisition

**Click-to-focus.** When a pointer event produces a `NodeHit` or `TileHit` result and the hit target accepts focus (tile has `input_mode != Passthrough`, node has `accepts_focus: true`), the runtime transfers focus to that target before forwarding the pointer event to the agent.

**Programmatic focus request.** An agent may request focus for a node it owns:

```protobuf
message FocusRequest {
  string session_id  = 1;
  string tile_id     = 2;
  string node_id     = 3;  // empty = tile-level focus
  bool   steal       = 4;  // if false, request is denied if another agent holds focus
}

message FocusResponse {
  enum Result {
    GRANTED    = 0;
    DENIED     = 1;   // steal=false and focus held by another agent
    INVALID    = 2;   // tile/node does not exist or not owned by this agent
  }
  Result result    = 1;
  string reason    = 2;
}
```

An agent cannot forcibly steal focus from another agent unless `steal: true` is set in the request. The runtime grants steal requests at its discretion; it may deny if the current focus owner has an active interaction in progress (e.g., mid-gesture).

**Focus transfer on tile destruction.** If a focused tile or node is destroyed, focus falls back to the previously focused element on the same tab, or to `None` if no prior focus exists.

**Focus isolation from other agents.** An agent cannot observe or query the focus state of tiles it does not own. The only focus event an agent receives is a `FocusGained`/`FocusLost` event for its own tiles/nodes.

### 1.3 Focus Cycling (Tab Key Traversal)

Tab key advances focus forward through the focusable elements on the current tab; Shift+Tab advances backward.

**Traversal order** follows tile z-order (lowest z first) and within each tile, tree order of `HitRegionNode` elements (depth-first, left-to-right sibling order). Tiles with `input_mode == Passthrough` are excluded from traversal.

**Cycle boundary.** After the last focusable element, focus wraps to the first. The chrome layer tab bar is excluded from the Tab key cycle (chrome focus is accessed via platform-standard keyboard shortcuts such as F6 or Ctrl+Tab).

**Chrome focus vs content focus.** Chrome focus (focus inside runtime UI) is logically separate from content focus (focus inside agent tiles). Switching between them uses platform-specific shortcuts. An agent cannot receive keyboard events when chrome focus is active.

```
Focus cycle within a tab:

Chrome layer   ──────────────────────────────── (not in Tab cycle)
                              ▲
                     F6 / platform shortcut
                              ▼
Content layer:
  Tile(z=1) → Node(z=1,N1) → Node(z=1,N2)
       ↓
  Tile(z=3) → Node(z=3,N1)
       ↓
  Tile(z=8) → [no HitRegion nodes] → Tile-level focus
       ↓
  (wrap to start)
```

### 1.4 Focus Events

The runtime dispatches these events to the owning agent when focus changes:

```protobuf
message FocusGainedEvent {
  string tile_id  = 1;
  string node_id  = 2;   // empty = tile-level focus
  FocusSource source = 3;
}

message FocusLostEvent {
  string tile_id  = 1;
  string node_id  = 2;
  FocusLostReason reason = 3;
}

enum FocusSource {
  CLICK      = 0;
  TAB_KEY    = 1;
  PROGRAMMATIC = 2;
}

enum FocusLostReason {
  CLICK_ELSEWHERE  = 0;
  TAB_KEY          = 1;
  PROGRAMMATIC     = 2;
  TILE_DESTROYED   = 3;
  TAB_SWITCHED     = 4;
  LEASE_REVOKED    = 5;
}
```

---

## 2. Capture Model

### 2.1 Pointer Capture Semantics

**Pointer capture** allows a node to receive all pointer events until it releases capture, even if the pointer leaves the node or tile bounds. This is the standard model for drag-and-drop, custom sliders, and touch-tracking interactions.

Only one node can hold pointer capture at a time, globally across the entire scene (not per-tab). Capture is associated with a specific pointer device (identified by `device_id`).

### 2.2 Capture Lifetime

1. **Acquire.** A node acquires capture in response to a `PointerDownEvent`. Capture cannot be acquired on `PointerMove` or `PointerUp`. Capture is acquired via the capture-request RPC (see §2.3) or automatically if the node sets `auto_capture: true` in its `HitRegionNode` definition.

2. **Active.** While capture is active, all pointer events from the captured device are routed to the capturing node, bypassing normal hit-testing. The pointer may leave the node's bounds and the tile's bounds without releasing capture.

3. **Release.** Capture is released on:
   - Explicit `CaptureReleaseRequest` from the owning agent.
   - `PointerUpEvent` for the captured device (automatic release, configurable per node via `release_on_up: bool`).
   - Capture theft by the runtime (see §2.4).

### 2.3 Capture Request/Release Protocol

```protobuf
message CaptureRequest {
  string session_id  = 1;
  string tile_id     = 2;
  string node_id     = 3;
  string device_id   = 4;
}

message CaptureResponse {
  enum Result {
    GRANTED  = 0;
    DENIED   = 1;   // another node holds capture for this device
    INVALID  = 2;   // node does not exist or not owned by agent
  }
  Result result = 1;
  string reason = 2;
}

message CaptureReleaseRequest {
  string session_id = 1;
  string tile_id    = 2;
  string node_id    = 3;
  string device_id  = 4;
}

message CaptureReleasedEvent {
  string tile_id    = 1;
  string node_id    = 2;
  string device_id  = 3;
  CaptureReleaseReason reason = 4;
}

enum CaptureReleaseReason {
  AGENT_RELEASED    = 0;
  POINTER_UP        = 1;
  RUNTIME_REVOKED   = 2;
  LEASE_REVOKED     = 3;
}
```

### 2.4 Capture Theft

The runtime may revoke capture unconditionally for system events:

- Alt+Tab (or equivalent window-switch shortcut)
- System notification requiring full screen (lock screen, emergency alert)
- Agent lease revocation
- Tab switch initiated by user

When capture is stolen, the runtime sends a `PointerCancelEvent` to the capturing node followed by a `CaptureReleasedEvent` with `reason: RUNTIME_REVOKED`. The agent must treat `PointerCancelEvent` as terminal — the interaction is over.

---

## 3. Gesture Model

### 3.1 Overview

Gestures are recognized from raw touch and pointer events by the runtime's gesture pipeline. Agents do not implement gesture recognition; they receive named gesture events. The runtime arbitrates all conflicts.

### 3.2 Supported Gestures (V1)

| Gesture | Touch | Pointer | Description |
|---------|-------|---------|-------------|
| `Tap` | 1-finger brief contact | Click (left button) | Brief touch or click |
| `DoubleTap` | 1-finger two taps | Double click | Two taps within 300ms |
| `LongPress` | 1-finger hold ≥ 500ms | Right mouse button press | Extended hold |
| `Drag` | 1-finger move | Left button + move | Single-finger translation |
| `Pinch` | 2-finger spread/squeeze | Scroll wheel (zoom axis) | Scale gesture |
| `Swipe` | 1-finger quick flick | Not supported | Directional fast swipe |
| `ContextMenu` | Long press or 2-finger tap | Right click | Context menu request |

### 3.3 Gesture Recognizer Pipeline

Raw events pass through a pipeline of candidate recognizers running in parallel. Each recognizer tracks a state machine over the event stream. When a recognizer reaches a terminal state (recognized or failed), it signals the arbiter.

```
OS events (touch/pointer)
         │
         ▼
  ┌─────────────────────────────────────────────────┐
  │              Event Preprocessor                 │
  │  • Attach timestamps                            │
  │  • Assign device_id                             │
  │  • Filter OS-level gestures (system swipe etc.) │
  └────────────────────┬────────────────────────────┘
                       │
                       ▼  (fan-out to all recognizers)
  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐
  │    Tap    │  │  LongPress│  │   Drag    │  │   Pinch   │
  │Recognizer │  │Recognizer │  │Recognizer │  │Recognizer │
  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘
        │              │              │              │
        └──────────────┴──────────────┴──────────────┘
                                │
                                ▼
                        ┌───────────────┐
                        │   Arbiter     │
                        │ (picks winner)│
                        └───────┬───────┘
                                │
                      ┌─────────┴─────────┐
                      │                   │
                 Winner event         Cancel events
                 → owning agent       → losers
```

### 3.4 Recognizer State Machines

Each recognizer tracks state. Example: the Tap recognizer.

```
Tap recognizer:

IDLE ──pointer_down──► POSSIBLE ──pointer_up (< 150ms, < 10px moved)──► RECOGNIZED
                           │
                           ├── pointer_up (> 150ms) ──► FAILED
                           └── pointer_moved (> 10px) ──► FAILED
```

**Budget:** Each recognizer update must complete in < 50μs. Total gesture recognition from the final event to winner selection: < 1ms.

### 3.5 Gesture Conflict Resolution

When multiple recognizers signal RECOGNIZED for the same event sequence:

**Priority by specificity (descending):**

1. `Pinch` (multi-touch, highest specificity)
2. `LongPress`
3. `Drag`
4. `DoubleTap`
5. `Tap`
6. `ContextMenu`

Higher-specificity gestures win. If two gestures have equal priority (e.g., a touch sequence that qualifies as both `Tap` and the beginning of `LongPress`), the `LongPress` recognizer delays its recognition until the minimum hold duration expires or the `Tap` recognizer's window closes.

**Cross-tile gesture arbitration.** When a gesture spans multiple tiles (e.g., a drag that starts in tile A and crosses into tile B):

- The tile where the gesture **starts** owns it.
- The owning tile's agent receives all events for the gesture, including pointer coordinates that extend outside its tile bounds.
- Tile B does not receive any events for that gesture.

The arbiter tracks the `capture_tile_id` from the first `PointerDownEvent` and binds the gesture to that tile.

### 3.6 Gesture Cancellation

When the arbiter selects a winner:

1. The winner's recognizer enters ACTIVE state; the runtime dispatches `GestureBeganEvent` to the owning agent.
2. All other recognizers for the same event sequence receive `GestureCancelledEvent` internally and return to IDLE.
3. The agents of tiles involved in the losing recognizers receive `PointerCancelEvent` (if they had received any pointer events).

### 3.7 Platform Gesture Integration

OS-level gestures (e.g., macOS three-finger swipe for Mission Control, Windows task view gesture, Wayland compositor gestures) are consumed by the OS before reaching winit. The runtime does not intercept or suppress them. Agents should design interactions that do not conflict with common system gestures.

---

## 4. IME (Input Method Editor)

### 4.1 Requirement

CJK text input, emoji keyboards, voice dictation, and physical keyboard layouts all route through the OS Input Method Editor. The runtime must cooperate with the platform IME subsystem rather than implement its own text input.

### 4.2 IME Lifecycle

IME composition is a two-phase process:

1. **Composition phase.** The user types characters via the IME. The composed characters are provisional — not yet committed. The IME may show a candidate window with alternatives. The runtime renders the composition text in-place with a visual underline to indicate provisional state.

2. **Commit phase.** The user confirms a candidate (or presses Enter). The composed text is committed as a final `character` event sequence. The runtime removes the composition underline and forwards the final characters to the agent.

```
IME Event Sequence:

  ImeCompositionStarted { position: Point2D }
        │
        ├── ImeCompositionUpdated { text: "ni", cursor: 2, highlighted: 0..2 }
        ├── ImeCompositionUpdated { text: "nǐ", cursor: 3, highlighted: 0..3 }  (candidate selected)
        │
  ImeCompositionCommitted { text: "你" }   ← final character delivered
```

### 4.3 IME Composition Window Positioning

The IME candidate window is displayed by the OS IME subsystem, not by the runtime. The runtime provides the **text insertion point** to the OS IME subsystem so it can position its candidate window near the cursor.

The insertion point is derived from:
1. The currently focused `HitRegionNode`'s bounds (screen-space).
2. The cursor offset within the node, if the agent has declared it via `SetImePosition`.

```protobuf
message SetImePositionRequest {
  string session_id   = 1;
  string tile_id      = 2;
  string node_id      = 3;
  float  cursor_x     = 4;   // display-space X coordinate
  float  cursor_y     = 5;   // display-space Y coordinate
  float  line_height  = 6;   // IME candidate window hint
}
```

The runtime translates this to the OS-native IME position API:
- **Windows:** `ImmSetCompositionWindow`, `ITfContextView::GetTextExt`
- **macOS:** `firstRectForCharacterRange` in NSTextInputClient
- **Linux:** `preedit_string` / `commit_string` via IBus or Fcitx XIM/Wayland protocols

### 4.4 IME Composition Events

The runtime forwards all IME events to the focused node's owning agent:

```protobuf
message ImeCompositionStartedEvent {
  string tile_id  = 1;
  string node_id  = 2;
}

message ImeCompositionUpdatedEvent {
  string tile_id    = 1;
  string node_id    = 2;
  string text       = 3;   // current composition string (provisional)
  uint32 cursor_pos = 4;   // byte offset of cursor within text
  uint32 sel_start  = 5;   // highlighted range start (for candidate selection)
  uint32 sel_end    = 6;   // highlighted range end
}

message ImeCompositionCommittedEvent {
  string tile_id  = 1;
  string node_id  = 2;
  string text     = 3;   // final committed text
}

message ImeCompositionCancelledEvent {
  string tile_id  = 1;
  string node_id  = 2;
}
```

**Update latency target:** IME composition window must update within one frame (< 16.6ms) of the user's input.

### 4.5 IME Candidate List Rendering

The IME candidate list (the popup showing input alternatives) is **rendered by the OS IME subsystem**, not by tze_hud. The runtime does not implement its own candidate list. This is intentional: OS IME subsystems have deep knowledge of locale, input methods, and accessibility that would be prohibitive to replicate.

In overlay (HUD) mode, the OS IME candidate window renders above the tze_hud overlay window (OS IME windows are always topmost). No special z-order handling is needed.

### 4.6 Input Method Support

| Method | Platform | Notes |
|--------|----------|-------|
| CJK (Pinyin, Cangjie, etc.) | Windows, macOS, Linux | Via OS IME |
| Emoji keyboard | Windows, macOS | OS emoji picker |
| Voice input | macOS | Dictation mode via IME protocol |
| Dead keys / compose | Linux, Windows | Handled by winit/OS |
| Right-to-left text | All | Agent responsibility; runtime forwards events |

---

## 5. Accessibility

### 5.1 Commitment

Accessibility is a first-class requirement, not an afterthought. The runtime exposes a live accessibility tree derived from the scene graph, bridged to the platform's native accessibility API. Screen readers, switch access, and keyboard-only navigation must all work without any agent involvement.

### 5.2 Accessibility Tree Structure

The runtime maintains an accessibility tree that mirrors the scene graph. The tree is updated within 100ms of any scene change.

```
A11yTree
└── Root (represents the tze_hud window/runtime)
    ├── TabBar (chrome)
    │   ├── TabButton("Morning", selected=true)
    │   └── TabButton("Work", selected=false)
    └── ContentArea (the active tab)
        ├── Tile(id="T1", label="Weather", role=Region)
        │   └── HitRegion(id="N1", label="Temperature", role=Button, pressed=false)
        └── Tile(id="T2", label="News Feed", role=Feed)
            ├── HitRegion(id="N2", label="Headline 1", role=Article)
            └── HitRegion(id="N3", label="Read more", role=Link)
```

### 5.3 Scene Graph to A11y Tree Mapping

| Scene element | A11y role | Required properties |
|---------------|-----------|---------------------|
| Tab (current) | `tab`, selected | tab name (from scene) |
| Tab (other) | `tab`, not selected | tab name |
| Tile | `region` | tile label (from `Tile.accessibility_label` field) |
| `SolidColorNode` | Not exposed | Decorative, excluded |
| `TextMarkdownNode` | `staticText` | text content |
| `StaticImageNode` | `image` | alt text from `accessibility_label` |
| `HitRegionNode` | `button` (default) | `accessibility_label`, state |

Agents declare accessibility metadata on nodes and tiles. The runtime does not infer accessibility semantics from content — it bridges what agents declare.

### 5.4 Accessibility Metadata (Agent-Declared)

Tiles and nodes carry accessibility metadata:

```protobuf
message AccessibilityConfig {
  string label        = 1;   // Human-readable label (required for interactive elements)
  string role_hint    = 2;   // Override default role mapping: "button", "link", "menuitem", etc.
  string description  = 3;   // Longer description for screen reader detail mode
  bool   live         = 4;   // true = announce content changes (aria-live equivalent)

  enum LivePoliteness {
    POLITE     = 0;  // Announce after current speech finishes
    ASSERTIVE  = 1;  // Interrupt current speech
    OFF        = 2;  // No announcement (default)
  }
  LivePoliteness live_politeness = 5;
}
```

### 5.5 Screen Reader Announcements

When a tile or node with `live: true` changes content, the runtime queues an announcement to the platform a11y API:

- **Polite:** Appended to the announcement queue; announced after current speech.
- **Assertive:** Interrupts current speech and announces immediately.

Announcements are rate-limited: at most one assertive announcement per 500ms to prevent screen reader flooding.

### 5.6 Focus Indication

Focus indication is dual-channel:

1. **Visual.** The runtime renders a focus ring on the currently focused `HitRegionNode` or tile boundary. The focus ring is rendered in the chrome layer (above all agent content) to guarantee visibility. Style: 2px solid ring, color configurable (defaults to system accent color with 3:1 contrast ratio minimum against the tile's background color).

2. **Semantic.** The a11y tree marks the focused element with the `focused` state. Screen readers announce focus changes.

Both channels update within Stage 2 (Local Feedback) of the frame pipeline — the focus ring appears in the same frame as the event that causes focus transfer.

### 5.7 Keyboard-Only Navigation

All interactions achievable with pointer input must also be achievable with keyboard input alone:

| Pointer action | Keyboard equivalent |
|----------------|---------------------|
| Click tile | Tab to focus, Enter or Space |
| Context menu | Application key, or Shift+F10 |
| Drag | Keyboard move mode: focus + arrow keys (agent implements) |
| Scroll | Arrow keys, Page Up/Down when tile has focus |
| Tab close | Focus tab, Delete key |

The runtime provides: Tab cycling, Enter/Space activation, Escape to cancel, arrow key routing to focused nodes. Complex interactions (drag, resize) are the agent's responsibility — the runtime provides focus and key events; the agent implements the keyboard mode.

### 5.8 Platform A11y API Integration

| Platform | API | Implementation |
|----------|-----|----------------|
| Windows | UI Automation (UIA) | `IAccessible2` or `IRawElementProviderSimple` |
| macOS | NSAccessibility | `NSAccessibilityElement` protocol |
| Linux X11/Wayland | AT-SPI2 | `at-spi2-core` via D-Bus |

The a11y bridge is a separate Rust module (crate: `tze_hud_a11y`) that subscribes to scene graph change events and maintains the platform-specific tree. It runs on the main thread and is updated during Stage 2 (Local Feedback) for focus changes and during Stage 4 (Scene Commit) for content changes.

---

## 6. Local Feedback Contract

### 6.1 Principle

The human must never feel like they are "clicking through a cloud roundtrip." Visual acknowledgement of input happens locally and instantly, in the same frame as the input event. Remote semantics (agent logic, content changes) follow asynchronously.

This is not a performance optimization — it is a correctness requirement. Any interaction model where local feedback waits for agent response is wrong by definition.

### 6.1a Headless Testability (DR-I11)

All input behavior defined in this RFC must be exercisable without a display server or physical GPU. This is a hard requirement (validation.md DR-V2, DR-V5):

- The hit-test pipeline (§7.2) operates on pure Rust data structures — no GPU or winit required.
- `HitRegionLocalState` updates (pressed/hovered/focused) must be assertable from Layer 0 tests with injected input events.
- The gesture recognizer state machines (§3.4) must accept synthetic event streams with injectable timestamps.
- The a11y tree (§5.2) must expose a programmatic query API for headless verification (see §11.5 for the open question on module boundary).
- The test scene registry includes `input_highlight` (local feedback state validation) and `chatty_dashboard_touch` (input responsiveness under coalescing load). Both must pass in headless CI on mesa llvmpipe, WARP, and Metal.

### 6.2 Latency Budgets

From validation.md §3:

| Metric | Budget | Measurement point |
|--------|--------|-------------------|
| `input_to_local_ack` | p99 < 4ms | Input event arrival → local state update written to render state |
| `input_to_scene_commit` | p99 < 50ms | Input event arrival → agent response applied to scene graph |
| `input_to_next_present` | p99 < 33ms | Input event arrival → next presented frame containing local state |

The local feedback path (stages 1+2 in RFC 0002 §3.2) executes entirely on the main thread with no locks on the mutable scene graph. It reads from an atomic snapshot of tile bounds. Stage 1+2 combined budget: < 1ms, providing substantial headroom against the 4ms local-ack target.

### 6.3 What Is Local (Runtime-Owned)

The runtime updates these states immediately, without agent involvement:

| State | When updated | Visual effect |
|-------|-------------|---------------|
| `HitRegionLocalState.pressed` | `PointerDownEvent` | Pressed visual (darkening, inset shadow) |
| `HitRegionLocalState.hovered` | `PointerEnterEvent` / `PointerLeaveEvent` | Hover highlight |
| `HitRegionLocalState.focused` | Focus transfer | Focus ring |
| Scroll position (V1 deferred) | Scroll event | Scroll offset |
| Drag ghost (V1 deferred) | Drag gesture | Translucent drag image |

Visual representations of local state are rendered by the runtime's compositor, not by agent content. The hit region node type includes a `local_style` configuration that specifies how local states appear visually, allowing agents to customize while keeping rendering fully local.

### 6.4 What Is Remote (Agent-Owned)

The runtime forwards events to agents; agents produce scene mutations in response. These are remote and asynchronous:

- Content changes in response to clicks (agent computes new content and submits mutation batch)
- Custom animations or transitions triggered by input
- Business logic (form validation, state machine transitions)
- Any interaction that requires agent-side computation

### 6.5 Local Feedback Rendering

Local state is encoded in `SceneLocalPatch`, produced in Stage 2:

```rust
pub struct SceneLocalPatch {
    pub timestamp: Instant,
    pub local_state_updates: Vec<LocalStateUpdate>,
}

pub struct LocalStateUpdate {
    pub node_id: SceneId,
    pub pressed: Option<bool>,
    pub hovered: Option<bool>,
    pub focused: Option<bool>,
}
```

The `SceneLocalPatch` is forwarded to the compositor thread via a dedicated channel (separate from the `MutationBatch` channel) and applied in Stage 4 before render encoding. It does not go through lease validation or budget checks — local state is always applied.

**Rendering:** The compositor renders local state as a compositing modifier on the affected node's visual output:
- `pressed`: multiply by 0.85 (darkening)
- `hovered`: add 0.1 white overlay (lightening)
- `focused`: draw 2px focus ring at node bounds

These defaults are overridable per `HitRegionNode` via `local_style`.

### 6.6 Rollback

If an agent rejects the interaction (e.g., returns a mutation batch rejection indicating the action is invalid in the current state), the local feedback is reverted. Rollback is animated — a brief (100ms) reverse transition to prevent jarring visual discontinuity.

Rollback is rare and only occurs on explicit agent rejection. It is not triggered by agent latency or silence — local state persists until the agent produces a mutation or the interaction ends naturally.

```
Timeline:
  t=0ms   PointerDown → pressed=true (local, immediate)
  t=2ms   Event dispatched to agent
  t=25ms  Agent returns MutationBatch (accepted) → content changes
          pressed=false (natural interaction end on PointerUp)

  --- OR (rejection case) ---

  t=0ms   PointerDown → pressed=true (local, immediate)
  t=2ms   Event dispatched to agent
  t=30ms  Agent returns rejection { reason: "disabled" }
  t=30ms  pressed=false + rollback animation (100ms)
```

---

## 7. Hit-Region Node Primitives

### 7.1 HitRegionNode (V1 Interactive Primitive)

`HitRegionNode` is the sole interactive primitive in V1, defined in RFC 0001 §2.4. This section specifies the full behavioral contract.

**Definition** (extending RFC 0001):

```rust
pub struct HitRegionNode {
    // Inherited from RFC 0001:
    pub bounds: Rect,               // Relative to tile origin
    pub interaction_id: String,     // Agent-defined; forwarded in events so agents can correlate without maintaining a SceneId→semantic mapping (see RFC 0001 §2.4)
    pub accepts_focus: bool,        // Whether keyboard focus can land here
    pub accepts_pointer: bool,      // Whether pointer events are captured here

    // Additional V1 fields:
    pub auto_capture: bool,         // Acquire pointer capture automatically on PointerDown
    pub release_on_up: bool,        // Release capture on PointerUp (default: true)
    pub cursor_style: CursorStyle,  // Pointer cursor when hovering
    pub tooltip: Option<String>,    // Tooltip text (shown after 500ms hover)
    pub event_mask: EventMask,      // Which events to receive (bitmask)
    pub accessibility: AccessibilityMetadata,
    pub local_style: LocalFeedbackStyle,
}

pub struct LocalFeedbackStyle {
    pub hover_tint: Option<Rgba>,
    pub press_tint: Option<Rgba>,
    pub focus_ring_color: Option<Rgba>,
    pub focus_ring_width_px: f32,  // default: 2.0
}

pub enum CursorStyle {
    Default, Pointer, Text, Crosshair, Move,
    ResizeN, ResizeS, ResizeE, ResizeW,
    ResizeNE, ResizeNW, ResizeSE, ResizeSW,
    NotAllowed, Grab, Grabbing,
}

pub struct EventMask {
    pub pointer_move: bool,    // default: false (saves agent bandwidth)
    pub pointer_enter: bool,   // default: true
    pub pointer_leave: bool,   // default: true
    pub pointer_down: bool,    // default: true
    pub pointer_up: bool,      // default: true
    pub click: bool,           // default: true
    pub double_click: bool,    // default: false
    pub context_menu: bool,    // default: false
    pub key_events: bool,      // default: true (when focused)
}
```

### 7.2 Hit-Region Bounds

Bounds are relative to the tile origin, matching RFC 0001 §5.2. Hit-test traversal for `HitRegionNode` follows the order defined in RFC 0001 §5.2:

1. Chrome layer (always wins)
2. Content layer tiles (z-order descending)
3. Within each tile: nodes in reverse tree order (last child first)
4. First `HitRegionNode` whose bounds contain the point wins

**Performance requirement:** < 100μs for a single point query against 50 tiles (from RFC 0001 §5.1).

### 7.3 Pointer Event Types

```protobuf
message PointerDownEvent {
  string tile_id          = 1;
  string node_id          = 2;   // SceneId of the hit HitRegionNode
  string device_id        = 3;
  PointerButton button    = 4;
  float  x                = 5;   // node-local coordinates
  float  y                = 6;
  float  display_x        = 7;   // display-space coordinates
  float  display_y        = 8;
  Modifiers modifiers     = 9;
  int64  timestamp_us     = 10;  // hardware timestamp (microseconds)
  string interaction_id   = 11;  // Agent-defined (from HitRegionNode); forwarded for semantic correlation
}

message PointerUpEvent {
  string tile_id        = 1;
  string node_id        = 2;
  string device_id      = 3;
  PointerButton button  = 4;
  float  x              = 5;
  float  y              = 6;
  float  display_x      = 7;
  float  display_y      = 8;
  Modifiers modifiers   = 9;
  int64  timestamp_us   = 10;
  string interaction_id = 11;  // Agent-defined; forwarded for semantic correlation
}

message PointerMoveEvent {
  string tile_id     = 1;
  string node_id     = 2;
  string device_id   = 3;
  float  x           = 4;
  float  y           = 5;
  float  display_x   = 6;
  float  display_y   = 7;
  float  dx          = 8;   // delta from last move event
  float  dy          = 9;
  Modifiers modifiers = 10;
  int64  timestamp_us = 11;
}

message PointerEnterEvent {
  string tile_id   = 1;
  string node_id   = 2;
  string device_id = 3;
  float  x         = 4;
  float  y         = 5;
  int64  timestamp_us = 6;
}

message PointerLeaveEvent {
  string tile_id   = 1;
  string node_id   = 2;
  string device_id = 3;
  float  x         = 4;
  float  y         = 5;
  int64  timestamp_us = 6;
}

message ClickEvent {
  string tile_id        = 1;
  string node_id        = 2;
  string device_id      = 3;
  PointerButton button  = 4;
  float  x              = 5;
  float  y              = 6;
  Modifiers modifiers   = 7;
  int64  timestamp_us   = 8;
  string interaction_id = 9;   // Agent-defined; forwarded for semantic correlation
}

message DoubleClickEvent {
  string tile_id        = 1;
  string node_id        = 2;
  string device_id      = 3;
  PointerButton button  = 4;
  float  x              = 5;
  float  y              = 6;
  Modifiers modifiers   = 7;
  int64  timestamp_us   = 8;
  string interaction_id = 9;   // Agent-defined; forwarded for semantic correlation
}

message ContextMenuEvent {
  string tile_id   = 1;
  string node_id   = 2;
  float  x         = 3;
  float  y         = 4;
  int64  timestamp_us = 5;
}

message PointerCancelEvent {
  string tile_id   = 1;
  string node_id   = 2;
  string device_id = 3;
  int64  timestamp_us = 4;
}

enum PointerButton {
  PRIMARY   = 0;  // Left mouse button, primary touch
  SECONDARY = 1;  // Right mouse button
  MIDDLE    = 2;
}

message Modifiers {
  bool shift = 1;
  bool ctrl  = 2;
  bool alt   = 3;
  bool meta  = 4;  // Command on macOS, Win key on Windows
}
```

### 7.4 Keyboard Event Types (Focused Node)

When a `HitRegionNode` has focus (it or its containing tile is the focus owner), it receives keyboard events from the owning agent via its event stream.

```protobuf
message KeyDownEvent {
  string tile_id     = 1;
  string node_id     = 2;
  string key_code    = 3;   // Physical key: "KeyA", "ArrowLeft", "Enter", etc. (DOM KeyboardEvent.code)
  string key         = 4;   // Logical key value: "a", "A", "ArrowLeft" (DOM KeyboardEvent.key)
  Modifiers modifiers = 5;
  bool   repeat      = 6;   // true = key is held (auto-repeat)
  int64  timestamp_us = 7;
}

message KeyUpEvent {
  string tile_id     = 1;
  string node_id     = 2;
  string key_code    = 3;
  string key         = 4;
  Modifiers modifiers = 5;
  int64  timestamp_us = 6;
}

message CharacterEvent {
  string tile_id   = 1;
  string node_id   = 2;
  string character = 3;   // Unicode character(s) produced by the key press (post-IME)
  int64  timestamp_us = 4;
}
```

`CharacterEvent` carries post-IME committed characters. `KeyDownEvent` carries raw key codes. Agents that implement text editing use `CharacterEvent` for input characters and `KeyDownEvent` for navigation/editing keys (arrows, backspace, enter, etc.).

---

## 8. Event Dispatch Protocol

### 8.1 Event Flow

```
OS hardware event (keyboard, mouse, touch)
            │
            ▼
     winit event loop
     (main thread)
            │
            ▼
  ┌─────────────────────┐
  │  Stage 1:           │
  │  Input Drain        │  Attach hardware + arrival timestamps
  │  (< 500μs p99)      │  Produce InputEvent{kind, pos, ts_hw, ts_arrival}
  └──────────┬──────────┘  Enqueue to InputEvent channel
             │
             ▼
  ┌─────────────────────┐
  │  Stage 2:           │
  │  Local Feedback     │  Hit-test against bounds snapshot (< 100μs)
  │  (< 500μs p99)      │  Update HitRegionLocalState (pressed/hovered/focused)
  └──────────┬──────────┘  Produce SceneLocalPatch
             │
             │  (events also forwarded to compositor thread)
             ▼
  ┌─────────────────────┐
  │  Compositor Thread  │
  │  (Stage 3-4)        │  Apply SceneLocalPatch to render state
  └──────────┬──────────┘  Route InputEvent to owning agent
             │
             ▼
  ┌─────────────────────┐
  │  Event Router       │  Resolve owning agent via scene graph
  │  (< 2ms from        │  Serialize event to protobuf
  │   hit-test)         │  Enqueue to per-agent EventBatch
  └──────────┬──────────┘
             │
             ▼
  ┌─────────────────────┐
  │  Network Thread     │
  │                     │  Send EventBatch on agent's gRPC session stream
  └─────────────────────┘
```

### 8.2 Event Routing

The event router resolves the owning agent for each input event:

1. Run hit-test query (see RFC 0001 §5) to find `HitTestResult`.
2. Map `HitTestResult` to owning agent session:
   - `NodeHit` → look up tile's lease owner → owning session
   - `TileHit` → look up tile's lease owner → owning session
   - `Chrome` → runtime handles locally; no agent notification
   - `Passthrough` → overlay mode: pass to desktop; fullscreen: discard
3. For keyboard events: route to the session owning the currently focused tile/node.
4. For captured pointer events: route to the capturing session (bypasses hit-test).

**Budget:** Event routing (hit-test + session lookup) must complete in < 2ms from Stage 2 completion.

### 8.3 Event Serialization

Events are serialized as protobuf messages and multiplexed over the agent's existing gRPC session stream (from RFC 0002 §1). The session stream carries a `SessionEnvelope` that can hold either `MutationBatchAck` (runtime → agent), `EventBatch` (runtime → agent), or `MutationBatch` (agent → runtime).

```protobuf
// Multiplexed on the session stream (runtime → agent direction)
message EventBatch {
  int64             frame_number = 1;
  int64             batch_ts_us  = 2;   // Timestamp when batch was assembled
  repeated InputEnvelope events  = 3;
}

message InputEnvelope {
  oneof event {
    PointerDownEvent     pointer_down     = 1;
    PointerUpEvent       pointer_up       = 2;
    PointerMoveEvent     pointer_move     = 3;
    PointerEnterEvent    pointer_enter    = 4;
    PointerLeaveEvent    pointer_leave    = 5;
    ClickEvent           click            = 6;
    DoubleClickEvent     double_click     = 7;
    ContextMenuEvent     context_menu     = 8;
    PointerCancelEvent   pointer_cancel   = 9;
    KeyDownEvent         key_down         = 10;
    KeyUpEvent           key_up           = 11;
    CharacterEvent       character        = 12;
    FocusGainedEvent     focus_gained     = 13;
    FocusLostEvent       focus_lost       = 14;
    GestureEvent         gesture          = 15;
    ImeCompositionStartedEvent   ime_started   = 16;
    ImeCompositionUpdatedEvent   ime_updated   = 17;
    ImeCompositionCommittedEvent ime_committed = 18;
    ImeCompositionCancelledEvent ime_cancelled = 19;
    CaptureReleasedEvent capture_released  = 20;
  }
}
```

### 8.4 Event Batching

Events that occur within the same frame are batched into a single `EventBatch` message. Batching is per-agent: events for different agents are in separate batches.

**Batching rules:**
- Events with the same `tile_id` and `node_id` in the same frame are grouped.
- Multiple `PointerMoveEvent` for the same node in the same frame are coalesced to the final position (latest-wins for moves).
- `PointerDownEvent`, `PointerUpEvent`, `ClickEvent`, `KeyDownEvent`, `KeyUpEvent`, and all transactional events (focus, capture, IME) are never coalesced — all are delivered in chronological order.

**Ordering guarantee:** Within a batch, events are ordered by `timestamp_us` (hardware timestamp, ascending). Events from different devices are interleaved by timestamp. An agent receiving an `EventBatch` can reconstruct the chronological event sequence by sorting on `timestamp_us`.

### 8.5 Backpressure and Coalescing

If an agent's event queue is full (the agent is slow to consume events):

1. **PointerMove events** are coalesced: only the latest position is retained.
2. **Hover state changes** (enter/leave) are coalesced: only the net state (currently inside or outside) is emitted.
3. **Transactional events** (down, up, click, key, focus, capture, IME) are never dropped. If the queue is full and a transactional event arrives, the oldest coalesced event is removed to make room.
4. If the queue remains full after coalescing, the oldest transactional event is logged as an overflow (visible in telemetry) but not dropped — the queue grows up to a hard cap of 4096 events per agent.
5. Beyond the hard cap, events are dropped and `telemetry_overflow_count` is incremented. This is a pathological condition; a well-behaved agent processes events within one frame.

**Event queue depth:** Default 256 events per agent. Hard cap 4096. Configurable per agent.

---

## 9. Protobuf Schema

### 9.1 input.proto

```protobuf
syntax = "proto3";
package tze_hud.input.v1;

// ─── Focus ────────────────────────────────────────────────────────────────

message FocusRequest {
  string session_id = 1;
  string tile_id    = 2;
  string node_id    = 3;
  bool   steal      = 4;
}

message FocusResponse {
  enum Result { GRANTED = 0; DENIED = 1; INVALID = 2; }
  Result result = 1;
  string reason = 2;
}

message FocusGainedEvent {
  string tile_id = 1;
  string node_id = 2;
  enum Source { CLICK = 0; TAB_KEY = 1; PROGRAMMATIC = 2; }
  Source source  = 3;
}

message FocusLostEvent {
  string tile_id = 1;
  string node_id = 2;
  enum Reason {
    CLICK_ELSEWHERE = 0; TAB_KEY = 1; PROGRAMMATIC = 2;
    TILE_DESTROYED = 3; TAB_SWITCHED = 4; LEASE_REVOKED = 5;
  }
  Reason reason  = 3;
}

// ─── Capture ──────────────────────────────────────────────────────────────

message CaptureRequest {
  string session_id = 1;
  string tile_id    = 2;
  string node_id    = 3;
  string device_id  = 4;
}

message CaptureResponse {
  enum Result { GRANTED = 0; DENIED = 1; INVALID = 2; }
  Result result = 1;
  string reason = 2;
}

message CaptureReleaseRequest {
  string session_id = 1;
  string tile_id    = 2;
  string node_id    = 3;
  string device_id  = 4;
}

message CaptureReleasedEvent {
  string tile_id   = 1;
  string node_id   = 2;
  string device_id = 3;
  enum Reason {
    AGENT_RELEASED = 0; POINTER_UP = 1;
    RUNTIME_REVOKED = 2; LEASE_REVOKED = 3;
  }
  Reason reason    = 4;
}

// ─── Pointer events ───────────────────────────────────────────────────────

enum PointerButton { PRIMARY = 0; SECONDARY = 1; MIDDLE = 2; }

message Modifiers {
  bool shift = 1; bool ctrl = 2; bool alt = 3; bool meta = 4;
}

message PointerDownEvent {
  string tile_id     = 1; string node_id    = 2; string device_id  = 3;
  PointerButton button = 4;
  float x = 5; float y = 6; float display_x = 7; float display_y = 8;
  Modifiers modifiers = 9; int64 timestamp_us = 10;
  string interaction_id = 11;  // Forwarded from HitRegionNode for agent correlation
}

message PointerUpEvent {
  string tile_id     = 1; string node_id    = 2; string device_id  = 3;
  PointerButton button = 4;
  float x = 5; float y = 6; float display_x = 7; float display_y = 8;
  Modifiers modifiers = 9; int64 timestamp_us = 10;
  string interaction_id = 11;  // Forwarded from HitRegionNode for agent correlation
}

message PointerMoveEvent {
  string tile_id     = 1; string node_id    = 2; string device_id  = 3;
  float x = 4; float y = 5; float display_x = 6; float display_y = 7;
  float dx = 8; float dy = 9;
  Modifiers modifiers = 10; int64 timestamp_us = 11;
}

message PointerEnterEvent {
  string tile_id = 1; string node_id = 2; string device_id = 3;
  float x = 4; float y = 5; int64 timestamp_us = 6;
}

message PointerLeaveEvent {
  string tile_id = 1; string node_id = 2; string device_id = 3;
  float x = 4; float y = 5; int64 timestamp_us = 6;
}

message ClickEvent {
  string tile_id = 1; string node_id = 2; string device_id = 3;
  PointerButton button = 4;
  float x = 5; float y = 6; Modifiers modifiers = 7; int64 timestamp_us = 8;
  string interaction_id = 9;   // Forwarded from HitRegionNode for agent correlation
}

message DoubleClickEvent {
  string tile_id = 1; string node_id = 2; string device_id = 3;
  PointerButton button = 4;
  float x = 5; float y = 6; Modifiers modifiers = 7; int64 timestamp_us = 8;
  string interaction_id = 9;   // Forwarded from HitRegionNode for agent correlation
}

message ContextMenuEvent {
  string tile_id = 1; string node_id = 2;
  float x = 3; float y = 4; int64 timestamp_us = 5;
}

message PointerCancelEvent {
  string tile_id = 1; string node_id = 2;
  string device_id = 3; int64 timestamp_us = 4;
}

// ─── Keyboard events ──────────────────────────────────────────────────────

message KeyDownEvent {
  string tile_id = 1; string node_id = 2;
  string key_code = 3; string key = 4;
  Modifiers modifiers = 5; bool repeat = 6; int64 timestamp_us = 7;
}

message KeyUpEvent {
  string tile_id = 1; string node_id = 2;
  string key_code = 3; string key = 4;
  Modifiers modifiers = 5; int64 timestamp_us = 6;
}

message CharacterEvent {
  string tile_id = 1; string node_id = 2;
  string character = 3; int64 timestamp_us = 4;
}

// ─── Gesture events ───────────────────────────────────────────────────────

message GestureEvent {
  string tile_id   = 1;
  string node_id   = 2;
  string device_id = 3;
  int64  timestamp_us = 4;

  oneof gesture {
    TapGesture        tap         = 10;
    DoubleTapGesture  double_tap  = 11;
    LongPressGesture  long_press  = 12;
    DragGesture       drag        = 13;
    PinchGesture      pinch       = 14;
    SwipeGesture      swipe       = 15;
  }
}

message TapGesture {
  float x = 1; float y = 2; Modifiers modifiers = 3;
}

message DoubleTapGesture {
  float x = 1; float y = 2; Modifiers modifiers = 3;
}

message LongPressGesture {
  float x = 1; float y = 2;
  enum Phase { BEGAN = 0; ENDED = 1; CANCELLED = 2; }
  Phase phase = 3;
}

message DragGesture {
  float x = 1; float y = 2;
  float dx = 3; float dy = 4;         // delta from last update
  float total_dx = 5; float total_dy = 6; // delta from drag start
  enum Phase { BEGAN = 0; CHANGED = 1; ENDED = 2; CANCELLED = 3; }
  Phase phase = 7;
}

message PinchGesture {
  float center_x = 1; float center_y = 2;
  float scale = 3;         // relative to pinch start (1.0 = no change)
  float velocity = 4;      // scale units per second
  enum Phase { BEGAN = 0; CHANGED = 1; ENDED = 2; CANCELLED = 3; }
  Phase phase = 5;
}

message SwipeGesture {
  enum Direction { UP = 0; DOWN = 1; LEFT = 2; RIGHT = 3; }
  Direction direction = 1;
  float velocity = 2;  // pixels per second
}

// ─── IME events ───────────────────────────────────────────────────────────

message SetImePositionRequest {
  string session_id  = 1;
  string tile_id     = 2;
  string node_id     = 3;
  float  cursor_x    = 4;
  float  cursor_y    = 5;
  float  line_height = 6;
}

message ImeCompositionStartedEvent {
  string tile_id = 1; string node_id = 2;
}

message ImeCompositionUpdatedEvent {
  string tile_id    = 1; string node_id = 2;
  string text       = 3;
  uint32 cursor_pos = 4;
  uint32 sel_start  = 5;
  uint32 sel_end    = 6;
}

message ImeCompositionCommittedEvent {
  string tile_id = 1; string node_id = 2;
  string text    = 3;
}

message ImeCompositionCancelledEvent {
  string tile_id = 1; string node_id = 2;
}

// ─── Dispatch batch ───────────────────────────────────────────────────────

message InputEnvelope {
  oneof event {
    PointerDownEvent     pointer_down     = 1;
    PointerUpEvent       pointer_up       = 2;
    PointerMoveEvent     pointer_move     = 3;
    PointerEnterEvent    pointer_enter    = 4;
    PointerLeaveEvent    pointer_leave    = 5;
    ClickEvent           click            = 6;
    DoubleClickEvent     double_click     = 7;
    ContextMenuEvent     context_menu     = 8;
    PointerCancelEvent   pointer_cancel   = 9;
    KeyDownEvent         key_down         = 10;
    KeyUpEvent           key_up           = 11;
    CharacterEvent       character        = 12;
    FocusGainedEvent     focus_gained     = 13;
    FocusLostEvent       focus_lost       = 14;
    GestureEvent         gesture          = 15;
    ImeCompositionStartedEvent   ime_started   = 16;
    ImeCompositionUpdatedEvent   ime_updated   = 17;
    ImeCompositionCommittedEvent ime_committed = 18;
    ImeCompositionCancelledEvent ime_cancelled = 19;
    CaptureReleasedEvent capture_released  = 20;
  }
}

message EventBatch {
  int64                    frame_number = 1;
  int64                    batch_ts_us  = 2;
  repeated InputEnvelope   events       = 3;
}

// ─── HitRegion configuration ──────────────────────────────────────────────
// NOTE: This RFC extends the HitRegionNode message defined in RFC 0001 §9.
// The unified wire message is HitRegionNode (RFC 0001); fields 5–11 below
// are added by this RFC. Do NOT use a separate HitRegionConfig message —
// implementations use the single merged HitRegionNode with all 11 fields.
// See RFC 0001 §2.4 and §9 for the base definition.
//
// (Reproduced here for readability; the canonical definition is RFC 0001 §9)
//
// message HitRegionNode {           // from RFC 0001 §9
//   Rect   bounds          = 1;
//   string interaction_id  = 2;    // Forwarded in events for agent correlation
//   bool   accepts_focus   = 3;
//   bool   accepts_pointer = 4;
//   bool   auto_capture    = 5;    // Added by this RFC
//   bool   release_on_up   = 6;
//   CursorStyle cursor_style = 7;
//   string tooltip         = 8;
//   EventMaskConfig event_mask = 9;
//   AccessibilityConfig accessibility = 10;
//   LocalStyleConfig local_style = 11;
// }

message EventMaskConfig {
  bool pointer_move    = 1;
  bool pointer_enter   = 2;
  bool pointer_leave   = 3;
  bool pointer_down    = 4;
  bool pointer_up      = 5;
  bool click           = 6;
  bool double_click    = 7;
  bool context_menu    = 8;
  bool key_events      = 9;
}

message AccessibilityConfig {
  string label       = 1;
  string role_hint   = 2;
  string description = 3;
  bool   live        = 4;
  enum LivePoliteness { POLITE = 0; ASSERTIVE = 1; OFF = 2; }
  LivePoliteness live_politeness = 5;
}

message LocalStyleConfig {
  Rgba  hover_tint       = 1;
  Rgba  press_tint       = 2;
  Rgba  focus_ring_color = 3;
  float focus_ring_width = 4;
}

message Rgba { float r = 1; float g = 2; float b = 3; float a = 4; }
message Rect  { float x = 1; float y = 2; float w = 3; float h = 4; }

enum CursorStyle {
  DEFAULT = 0; POINTER = 1; TEXT = 2; CROSSHAIR = 3; MOVE = 4;
  RESIZE_N = 5; RESIZE_S = 6; RESIZE_E = 7; RESIZE_W = 8;
  RESIZE_NE = 9; RESIZE_NW = 10; RESIZE_SE = 11; RESIZE_SW = 12;
  NOT_ALLOWED = 13; GRAB = 14; GRABBING = 15;
}
```

---

## 10. Diagrams

### 10.1 Event Flow: OS to Agent

```
┌─────────────────────────────────────────────────────────────────────────┐
│  OS / Hardware                                                          │
│  keyboard, mouse, touchscreen, tablet                                   │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │  raw OS events
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  winit event loop  (main thread)                                        │
│  WindowEvent::KeyboardInput, CursorMoved, MouseInput, Touch, ...        │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │
                                ▼
                    ┌──────────────────────┐
                    │  Stage 1: Input      │
                    │  Drain               │  < 500μs p99
                    │  • Attach hw + mono  │
                    │    timestamps        │
                    │  • Produce InputEvent│
                    │  • Enqueue (non-blk) │
                    └──────────┬───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │  Stage 2: Local      │
                    │  Feedback            │  < 500μs p99
                    │  • Hit-test bounds   │  (< 100μs hit-test)
                    │    snapshot          │
                    │  • Update pressed /  │
                    │    hovered / focused │
                    │  • Produce           │
                    │    SceneLocalPatch   │
                    │  • Update a11y tree  │
                    │    (focus changes)   │
                    └────┬─────────────────┘
                         │                │
                         │ InputEvent     │ SceneLocalPatch
                         ▼                ▼
              ┌──────────────────────────────────────┐
              │  Compositor Thread                   │
              │  Stage 3: Mutation Intake            │
              │  Stage 4: Scene Commit               │
              │    • Apply SceneLocalPatch           │
              │    • Route InputEvent:               │
              │      - Run hit-test (full)           │
              │      - Resolve owning session        │
              │      - < 2ms from Stage 2            │
              │    • Serialize to protobuf           │
              │    • Enqueue to per-agent EventBatch │
              └────────────────┬─────────────────────┘
                               │ EventBatch (per agent)
                               ▼
              ┌──────────────────────────────────────┐
              │  Network Thread                      │
              │  • gRPC stream write (agent session) │
              │  • Ordered by timestamp_us           │
              └────────────────┬─────────────────────┘
                               │ gRPC EventBatch
                               ▼
                         Agent Process
```

### 10.2 Focus Tree with Chrome/Content Separation

```
tze_hud Window
├── Chrome Layer  (Tab cycle excluded; accessed via platform shortcut)
│   ├── TabBar
│   │   ├── [Tab "Morning"  selected=true ]  ← chrome focus when active
│   │   └── [Tab "Work"     selected=false]
│   └── SystemIndicators
│
└── Content Layer  (Tab key cycle)
    │
    Active Tab ("Morning"):
    │
    ├── Tile T1  z=1  "Weather"
    │   ├── HitRegion N1  accepts_focus=true   ← Tab stop 1
    │   └── HitRegion N2  accepts_focus=true   ← Tab stop 2
    │
    ├── Tile T2  z=3  "News Feed"
    │   └── HitRegion N3  accepts_focus=true   ← Tab stop 3
    │
    └── Tile T3  z=8  "Status Bar"
        └── (no HitRegion with accepts_focus)
            → Tile-level focus if input_mode != Passthrough  ← Tab stop 4

Tab key traversal order (by z ascending, tree order within tile):
  T1/N1 → T1/N2 → T2/N3 → T3 → (wrap to T1/N1)

Chrome focus:
  F6 / platform shortcut switches between chrome and content focus.
  Chrome focus does not participate in Tab cycle.

Focus state per tab (suspended tabs preserve state, no events):
  Active tab:    FocusOwner::Node { tile_id: T1, node_id: N1 }  ← current
  Suspended tab: FocusOwner::Tile { tile_id: T5 }               ← preserved
```

### 10.3 Gesture Arbitration Pipeline

```
  Touch event stream (example: a drag starting as a tap candidate)

  t=0ms  PointerDown at (100, 200)
         │
         ├──► TapRecognizer:      state=POSSIBLE
         ├──► LongPressRecognizer: state=POSSIBLE  (timer started: 500ms)
         ├──► DragRecognizer:     state=POSSIBLE
         └──► PinchRecognizer:    state=FAILED     (need 2 fingers)

  t=5ms  PointerMove to (108, 200)   (8px delta)
         │
         ├──► TapRecognizer:      FAILED (moved > 5px threshold)
         ├──► LongPressRecognizer: FAILED (moved > 5px threshold)
         └──► DragRecognizer:     state=BEGAN (threshold crossed)

  t=5ms  ARBITER:
         ├── DragRecognizer = RECOGNIZED (sole surviving recognizer)
         ├── TapRecognizer  = FAILED → PointerCancelEvent to any interested party
         └── LongPressRecognizer = FAILED → cancel timer

  t=5ms  → GestureEvent { drag { phase=BEGAN, x=108, y=200, dx=8, dy=0 } }
            dispatched to owning agent

  t=10ms PointerMove to (130, 200)
         → GestureEvent { drag { phase=CHANGED, dx=22, dy=0, total_dx=30 } }

  t=50ms PointerUp
         → GestureEvent { drag { phase=ENDED, total_dx=52, total_dy=0 } }


  Multi-touch pinch example:

  t=0ms  Touch1Down at (100, 200) + Touch2Down at (200, 200)  ← same frame
         │
         ├──► PinchRecognizer:    state=POSSIBLE (2 contacts, spread=100px)
         └──► DragRecognizer:     state=POSSIBLE (multi-touch drag)

  t=3ms  Touch1Move (90,200), Touch2Move (210,200)  spread=120px
         │
         ├──► PinchRecognizer:    RECOGNIZED  scale=1.2
         └──► DragRecognizer:     FAILED (pinch takes priority, specificity rule)

  t=3ms  ARBITER: PinchRecognizer wins (higher specificity)
         → GestureEvent { pinch { phase=BEGAN, scale=1.2, ... } }
```

### 10.4 Local Feedback vs Remote Response Timeline

```
t=0ms    ─── PointerDown event arrives at main thread (winit) ───────────►

t=0.3ms  Stage 1 (Input Drain): timestamp attached, enqueued

t=0.8ms  Stage 2 (Local Feedback):
         • Hit-test bounds snapshot → NodeHit(T2, N1) [< 100μs]
         • HitRegionLocalState.pressed = true
         • SceneLocalPatch produced
         • A11y: focus state updated if needed
                                              ← LOCAL ACK COMPLETE (< 1ms)

t=1.0ms  Compositor thread receives SceneLocalPatch:
         • pressed=true applied to render state immediately

t=1.6ms  Frame renders: HitRegion N1 draws with press tint
         DISPLAY: user sees pressed state   ← input_to_next_present < 16.6ms

t=2.0ms  Event Router: routing resolves, event serialized to protobuf

t=2.5ms  Network Thread: EventBatch sent on agent's gRPC stream

┄┄┄┄┄ network / agent processing latency ┄┄┄┄┄

t=25ms   Agent processes ClickEvent, constructs MutationBatch
         (e.g., update text node to "selected state")

t=26ms   MutationBatch arrives at compositor thread

t=27ms   Stage 4 (Scene Commit): mutation applied, content updated
                                              ← SCENE COMMIT (~27ms)

t=28ms   Frame renders: content change visible
         DISPLAY: agent's response visible   ← input_to_scene_commit < 50ms

── REJECTION CASE ────────────────────────────────────────────────────────

t=0ms    PointerDown → pressed=true (local, immediate)
t=2.5ms  Event dispatched to agent

t=30ms   Agent returns rejection { code: ELEMENT_DISABLED }

t=30ms   Runtime receives rejection:
         • SceneLocalPatch { pressed=false }
         • Rollback animation: 100ms lerp from press tint to normal
         DISPLAY: brief press flash → rollback to unpressed (100ms anim)
```

---

## 11. Open Questions

These questions require decisions before implementation of the input subsystem begins. They are not blockers for RFC approval.

### 11.1 Drag-and-Drop

V1 does not specify drag-and-drop between tiles or agents. The `DragGesture` event covers single-tile drag interactions. Cross-tile and cross-agent DnD requires a separate protocol (drag offer, drop target negotiation) and is deferred post-V1. If a tile needs drag-and-drop in V1, it must implement a custom protocol over pointer events.

### 11.2 Scroll Events

Scroll (mouse wheel, touchpad two-finger swipe, touchpad momentum) is not fully specified in this RFC, but it is **not optional**: failure.md §"What the user always sees" lists "screen responsive to touch/input within 4ms" as an invariant, and presence.md §Interaction establishes local-first scroll feedback as a core commitment.

**Scope decision:** Scroll position update and visual scroll feedback are local-only operations (no agent roundtrip) and therefore fall under the local feedback contract (§6). They must be added before implementation begins. The open questions are the mechanics: snap points, momentum physics, scroll boundary behavior (rubber-band vs hard stop), and whether scroll offset is exposed to agents as a scene mutation or inferred from content height.

**Action required before implementation:** Add §6.x Scroll Feedback to this RFC defining: scroll offset as compositor-managed local state per tile, `ScrollEvent` proto message, momentum model (OS-provided vs runtime-implemented), and agent notification semantics (agent learns the current scroll offset via an event, but does not drive it).

### 11.3 Gamepad / Controller Input

Not addressed in this RFC. The architecture can accommodate it via a new device class in the Input Drain stage, but the routing model (which tile receives gamepad events?) and the event types need specification.

### 11.4 Stylus / Pressure Input

Pointer events in this RFC carry basic coordinates. Stylus-specific properties (pressure, tilt, twist) are not included. This should be a future extension to `PointerDownEvent` / `PointerMoveEvent`.

### 11.5 Accessibility Tree Storage Strategy

The a11y tree is currently specified as in-memory only. For headless test environments, the a11y tree should be accessible via a programmatic API (for Layer 0 scene graph assertions). The module boundary for the a11y bridge and its test surface needs to be specified before implementation.

### 11.6 Key Code Normalization

`KeyDownEvent.key_code` uses DOM `KeyboardEvent.code` identifiers ("KeyA", "ArrowLeft"). winit provides its own key code enumeration. The normalization layer (winit code → DOM code string) needs a complete mapping table, particularly for platform-specific keys (Windows key, Menu key, media keys).

---

## 12. RFC Dependency Map

```
RFC 0001 (Scene Contract)
  └── §2.4 HitRegionNode definition
  └── §5   Hit-testing algorithm and performance requirement

RFC 0002 (Runtime Kernel)
  └── §3.2 Stage 1 (Input Drain) and Stage 2 (Local Feedback) specifications
  └── §2   Thread model (main thread vs compositor thread)

RFC 0003 (Lease Model)
  └── Lease ownership → event routing (who owns the tile = who receives events)
  └── Lease revocation → capture release, focus loss

RFC 0004 (this)
  └── Input model: focus, capture, gestures, IME, a11y, local feedback,
      hit-region primitives, event dispatch protocol, protobuf schema
```

---

## 13. Non-Goals (V1)

The following are explicitly deferred to post-V1:

- Drag-and-drop between tiles or agents (§11.1)
- Scroll events and momentum physics (§11.2)
- Gamepad/controller input (§11.3)
- Stylus/pressure input (§11.4)
- Multi-pointer hover (distinct hover states for multiple cursors simultaneously)
- Touch force (3D Touch / haptic pressure)
- Pointer lock (mouse grab for FPS-style input)
- Custom gesture recognizers defined by agents (agents receive gesture events; they cannot add recognizer types)
- Dynamic a11y role changes at runtime (roles are set at node creation, not mutated)
