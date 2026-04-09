# Input Model Specification

Source: RFC 0004 (Input Model)
Domain: Hot Path
Depends on: scene-graph, runtime-kernel, timing-model, session-protocol, lease-governance

---

## ADDED Requirements

### Requirement: Focus Tree Structure
The runtime SHALL maintain a per-tab focus tree where at most one focus owner exists per tab. The focus owner SHALL be one of: None, a chrome element, a tile, or a specific HitRegionNode within a tile. Each tab SHALL maintain independent focus state. Switching tabs SHALL suspend the current tab's focus state and restore the target tab's preserved focus state without generating events on the suspended tab.
Source: RFC 0004 §1.1
Scope: v1-mandatory

#### Scenario: Single focus owner per tab
- **WHEN** a tab has focus owner set to Node(tile_id=T2, node_id=N1)
- **THEN** no other element in the same tab SHALL report itself as the focus owner

#### Scenario: Focus state preserved across tab switch
- **WHEN** Tab A has focus on Node(T2,N1) and the user switches to Tab B
- **THEN** Tab A's focus state (Node(T2,N1)) SHALL be preserved in memory without generating FocusLostEvent, and switching back to Tab A SHALL restore focus to Node(T2,N1) without generating FocusGainedEvent from the restoration

---

### Requirement: Click-to-Focus Acquisition
The runtime SHALL transfer focus to the hit target before forwarding the pointer event to the agent when a pointer event produces a NodeHit or TileHit result and the target accepts focus (tile has input_mode != Passthrough, node has accepts_focus: true). Focus transfer SHALL occur before the PointerDownEvent is dispatched to the owning agent.
Source: RFC 0004 §1.2
Scope: v1-mandatory

#### Scenario: Click on focusable node transfers focus
- **WHEN** the user clicks on a HitRegionNode with accepts_focus=true in tile T1
- **THEN** the runtime SHALL set the focus owner to Node(T1, node_id) and dispatch FocusGainedEvent(source=CLICK) before dispatching the PointerDownEvent to the agent

#### Scenario: Click on passthrough tile does not acquire focus
- **WHEN** the user clicks on a tile with input_mode=Passthrough
- **THEN** the focus owner SHALL NOT change and no FocusGainedEvent SHALL be dispatched

---

### Requirement: Programmatic Focus Request
An agent SHALL be able to request focus for a node it owns via FocusRequest. The runtime SHALL deny the request with DENIED if steal=false and another agent holds focus. The runtime SHALL deny with INVALID if the tile/node does not exist or is not owned by the requesting agent. The runtime MAY deny a steal=true request if the current focus owner has an active interaction in progress.
Source: RFC 0004 §1.2
Scope: v1-mandatory

#### Scenario: Programmatic focus granted
- **WHEN** an agent sends FocusRequest(tile_id=T1, node_id=N1, steal=false) and no other agent holds focus
- **THEN** the runtime SHALL respond with FocusResponse(result=GRANTED) and dispatch FocusGainedEvent(source=PROGRAMMATIC) to the agent

#### Scenario: Programmatic focus denied without steal
- **WHEN** an agent sends FocusRequest(steal=false) while another agent holds focus
- **THEN** the runtime SHALL respond with FocusResponse(result=DENIED)

---

### Requirement: Focus Transfer on Destruction
When a focused tile or node is destroyed, the runtime SHALL fall back focus to the previously focused element on the same tab, or to FocusOwner::None if no prior focus exists.
Source: RFC 0004 §1.2
Scope: v1-mandatory

#### Scenario: Focused node destroyed
- **WHEN** the currently focused node N1 in tile T1 is destroyed and the previous focus was on Node(T2, N2)
- **THEN** focus SHALL transfer to Node(T2, N2) and FocusLostEvent(reason=TILE_DESTROYED) SHALL be dispatched to the agent that owned T1

---

### Requirement: Focus Isolation Between Agents
An agent SHALL NOT be able to observe or query the focus state of tiles it does not own. The only focus events an agent receives SHALL be FocusGainedEvent and FocusLostEvent for its own tiles and nodes.
Source: RFC 0004 §1.2
Scope: v1-mandatory

#### Scenario: Agent cannot see other agent's focus
- **WHEN** Agent A owns tile T1 and Agent B owns tile T2 with current focus on T2/N1
- **THEN** Agent A SHALL NOT receive any event or query result indicating that T2/N1 has focus

---

### Requirement: Focus Cycling
Focus cycling SHALL follow the NAVIGATE_NEXT and NAVIGATE_PREV abstract commands. Traversal order SHALL follow tile z-order (lowest z first) and within each tile, depth-first left-to-right tree order of HitRegionNode elements with accepts_focus=true. Tiles with input_mode=Passthrough SHALL be excluded from traversal. A non-Passthrough tile with no focusable HitRegionNodes SHALL receive tile-level focus (FocusOwner::Tile) as a single step in the cycle. After the last focusable element, focus SHALL wrap to the first. The chrome layer tab bar SHALL be excluded from the content focus cycle.
Source: RFC 0004 §1.3
Scope: v1-mandatory

#### Scenario: Tab key cycles focus in z-order
- **WHEN** the scene has Tile(z=1) with nodes N1,N2, Tile(z=3) with node N3, and Tile(z=8) with no focusable nodes, and focus is on N2
- **THEN** pressing Tab SHALL move focus to N3, and pressing Tab again SHALL move focus to Tile(z=8) at tile level (FocusOwner::Tile), and pressing Tab again SHALL wrap to N1

#### Scenario: Passthrough tile excluded from cycle
- **WHEN** focus cycling encounters a tile with input_mode=Passthrough
- **THEN** that tile and all its nodes SHALL be skipped in the traversal order

---

### Requirement: Focus Events Dispatch
The runtime SHALL dispatch FocusGainedEvent to the owning agent when focus transfers to a tile or node, including the source of acquisition (CLICK, TAB_KEY, PROGRAMMATIC, COMMAND_INPUT). The runtime SHALL dispatch FocusLostEvent with the reason for loss (CLICK_ELSEWHERE, TAB_KEY, PROGRAMMATIC, TILE_DESTROYED, TAB_SWITCHED, LEASE_REVOKED, AGENT_DISCONNECTED, COMMAND_INPUT).
Source: RFC 0004 §1.4
Scope: v1-mandatory

#### Scenario: Focus gained via tab key
- **WHEN** the user presses Tab and focus moves to node N1 in tile T1
- **THEN** the agent owning T1 SHALL receive FocusGainedEvent(tile_id=T1, node_id=N1, source=TAB_KEY) and the previous focus owner's agent SHALL receive FocusLostEvent(reason=TAB_KEY)

---

### Requirement: Pointer Capture Semantics
Only one node SHALL hold pointer capture at a time globally across the entire scene. Capture SHALL be associated with a specific pointer device identified by device_id. Capture SHALL only be acquired in response to a PointerDownEvent (not on PointerMove or PointerUp). While capture is active, all pointer events from the captured device SHALL be routed to the capturing node, bypassing normal hit-testing.
Source: RFC 0004 §2.1, §2.2
Scope: v1-mandatory

#### Scenario: Captured pointer events bypass hit-test
- **WHEN** node N1 in tile T1 holds pointer capture for device D1 and the pointer moves outside T1's bounds
- **THEN** all pointer events from device D1 SHALL be routed to N1 regardless of which tile the pointer is visually over

#### Scenario: Capture denied for already-captured device
- **WHEN** node N1 holds capture for device D1 and node N2 requests capture for the same device D1
- **THEN** the runtime SHALL respond with CaptureResponse(result=DENIED)

---

### Requirement: Capture Release
Capture SHALL be released on: explicit CaptureReleaseRequest from the owning agent, PointerUpEvent for the captured device (when release_on_up=true), or runtime capture theft. The runtime SHALL send CaptureReleasedEvent with the appropriate reason (AGENT_RELEASED, POINTER_UP, RUNTIME_REVOKED, LEASE_REVOKED).
Source: RFC 0004 §2.2, §2.3
Scope: v1-mandatory

#### Scenario: Auto-release on pointer up
- **WHEN** a node with release_on_up=true holds capture and a PointerUpEvent arrives for the captured device
- **THEN** capture SHALL be released and CaptureReleasedEvent(reason=POINTER_UP) SHALL be dispatched to the owning agent

---

### Requirement: Capture Theft by Runtime
The runtime SHALL be able to revoke capture unconditionally for system events: Alt+Tab (window switch), system notifications requiring full screen, agent lease revocation, or user-initiated tab switch. When capture is stolen, the runtime SHALL send PointerCancelEvent followed by CaptureReleasedEvent(reason=RUNTIME_REVOKED). The agent MUST treat PointerCancelEvent as terminal.
Source: RFC 0004 §2.4
Scope: v1-mandatory

#### Scenario: Alt+Tab revokes capture
- **WHEN** a node holds pointer capture and the user presses Alt+Tab
- **THEN** the runtime SHALL send PointerCancelEvent to the capturing node, followed by CaptureReleasedEvent(reason=RUNTIME_REVOKED)

---

### Requirement: Auto-Capture on PointerDown
When a HitRegionNode has auto_capture=true, the runtime SHALL automatically acquire pointer capture for that node on PointerDownEvent without requiring an explicit CaptureRequest from the agent.
Source: RFC 0004 §2.2, §7.1
Scope: v1-mandatory

#### Scenario: Auto-capture triggers on pointer down
- **WHEN** a PointerDownEvent hits a HitRegionNode with auto_capture=true
- **THEN** the runtime SHALL acquire capture for that node and device without the agent sending CaptureRequest

---

### Requirement: Local Feedback Latency — input_to_local_ack
The runtime SHALL update local visual state (pressed, hovered, focused) at p99 < 4ms of input event arrival. This is a non-negotiable doctrinal rule, not an aspirational target — any interaction model where local feedback waits for an agent roundtrip is wrong by definition. The local feedback path (Stages 1+2) SHALL execute entirely on the main thread with no locks on the mutable scene graph, using an atomic snapshot of tile bounds, with a combined Stage 1+2 budget of < 1ms.
Source: RFC 0004 §6.1, §6.2, validation.md §3, DR-I1
Scope: v1-mandatory

#### Scenario: Press state within 4ms p99
- **WHEN** a PointerDownEvent arrives at the main thread
- **THEN** HitRegionLocalState.pressed SHALL be set to true at p99 < 4ms of event arrival, and the SceneLocalPatch SHALL be produced and forwarded to the compositor thread

---

### Requirement: Local Feedback Latency — input_to_scene_commit
The runtime SHALL apply agent responses to the scene graph within p99 < 50ms of input event arrival for local agents.
Source: RFC 0004 §6.2, validation.md §3, DR-I3
Scope: v1-mandatory

#### Scenario: Agent mutation applied within 50ms
- **WHEN** a PointerDownEvent arrives and the local agent processes it and returns a MutationBatch
- **THEN** the mutation SHALL be applied to the scene graph within 50ms (p99) of the original input event arrival

---

### Requirement: Local Feedback Latency — input_to_next_present
The runtime SHALL present the next frame containing local state updates within p99 < 33ms of input event arrival.
Source: RFC 0004 §6.2, validation.md §3, DR-I4
Scope: v1-mandatory

#### Scenario: Visual update within one frame
- **WHEN** a PointerDownEvent triggers pressed=true local state
- **THEN** the next presented frame SHALL contain the pressed visual state within 33ms (p99) of the input event

---

### Requirement: Runtime-Owned Local State Updates
The runtime SHALL update the following states immediately without agent involvement: HitRegionLocalState.pressed on PointerDownEvent, HitRegionLocalState.hovered on PointerEnterEvent/PointerLeaveEvent, HitRegionLocalState.focused on focus transfer, and tile scroll offset on ScrollEvent. Visual representations of local state SHALL be rendered by the runtime's compositor, not by agent content.
Source: RFC 0004 §6.3
Scope: v1-mandatory

#### Scenario: Hover state without agent roundtrip
- **WHEN** the pointer enters a HitRegionNode's bounds
- **THEN** HitRegionLocalState.hovered SHALL be set to true and the hover visual (default: 0.1 white overlay) SHALL be rendered by the compositor in the same frame, without waiting for any agent response

---

### Requirement: Local Feedback Rendering via SceneLocalPatch
Local state SHALL be encoded in SceneLocalPatch, produced in Stage 2, containing LocalStateUpdate (node_id, pressed, hovered, focused) and ScrollOffsetUpdate (tile_id, offset_x, offset_y). The SceneLocalPatch SHALL be forwarded to the compositor via a dedicated channel separate from the MutationBatch channel and SHALL be applied in Stage 4 before render encoding without going through lease validation or budget checks.
Source: RFC 0004 §6.5
Scope: v1-mandatory

#### Scenario: SceneLocalPatch bypasses lease validation
- **WHEN** a pressed state change is produced by Stage 2
- **THEN** the SceneLocalPatch SHALL be applied by the compositor without checking the tile's lease status or budget

---

### Requirement: Local Feedback Rollback on Agent Rejection
If an agent explicitly rejects an interaction, the local feedback SHALL be reverted with a 100ms reverse animation. Rollback SHALL only occur on explicit agent rejection, not on agent latency or silence.
Source: RFC 0004 §6.6
Scope: v1-mandatory

#### Scenario: Agent rejection triggers rollback animation
- **WHEN** a PointerDownEvent causes pressed=true and the agent returns a rejection for the interaction
- **THEN** pressed SHALL be set to false and a 100ms reverse transition animation SHALL play

#### Scenario: Agent silence does not trigger rollback
- **WHEN** a PointerDownEvent causes pressed=true and the agent does not respond within 50ms
- **THEN** pressed SHALL remain true until the interaction ends naturally (e.g., PointerUp)

---

### Requirement: Local Feedback Defaults and Customization
The compositor SHALL render default local feedback: pressed=multiply by 0.85 (darkening), hovered=add 0.1 white overlay, focused=2px focus ring at node bounds. These defaults SHALL be overridable per HitRegionNode via the local_style field (LocalFeedbackStyle: hover_tint, press_tint, focus_ring_color, focus_ring_width_px).
Source: RFC 0004 §6.5
Scope: v1-mandatory

#### Scenario: Custom press tint applied
- **WHEN** a HitRegionNode has local_style.press_tint set to Rgba(0.0, 0.0, 1.0, 0.3) and the node is pressed
- **THEN** the compositor SHALL apply the custom blue tint instead of the default 0.85 darkening

---

### Requirement: Scroll Local Feedback
Scroll SHALL be a local-first operation. The compositor SHALL maintain a scroll offset per scrollable tile and update it in the same frame the scroll event arrives, without an agent roundtrip. Tiles SHALL opt in to scroll via ScrollConfig. The scroll latency budget SHALL be the same as press state: input_to_local_ack p99 < 4ms. Agents SHALL receive scroll offset via ScrollOffsetChangedEvent (non-transactional, coalesced). If an agent-set offset and a user scroll arrive in the same frame, user scroll SHALL take priority.
Source: RFC 0004 §6.7
Scope: v1-mandatory

#### Scenario: Scroll offset updated in same frame
- **WHEN** a scroll wheel event arrives on a tile with ScrollConfig(scrollable_y=true)
- **THEN** the compositor SHALL update the tile's scroll offset_y in the same frame without waiting for any agent response

#### Scenario: User scroll overrides agent scroll request
- **WHEN** an agent sends SetScrollOffsetRequest and a user scroll event arrives in the same frame
- **THEN** the user scroll delta SHALL take priority and the agent request SHALL be discarded

---

### Requirement: HitRegionNode Primitive
HitRegionNode SHALL be the sole interactive primitive in v1. It SHALL carry: bounds (relative to tile origin), interaction_id (agent-defined, forwarded in events), accepts_focus, accepts_pointer, auto_capture, release_on_up, cursor_style, tooltip (shown after 500ms hover), event_mask, accessibility metadata, and local_style configuration.
Source: RFC 0004 §7.1, RFC 0001 §2.4
Scope: v1-mandatory

#### Scenario: Event mask filters delivery
- **WHEN** a HitRegionNode has event_mask.pointer_move=false
- **THEN** PointerMoveEvent SHALL NOT be dispatched to the owning agent for that node, saving agent bandwidth

#### Scenario: interaction_id forwarded in events
- **WHEN** a HitRegionNode has interaction_id="submit-button" and receives a ClickEvent
- **THEN** the ClickEvent dispatched to the agent SHALL include interaction_id="submit-button"

---

### Requirement: Hit-Test Performance
Hit-test traversal for a single point query against 50 tiles SHALL complete in < 100 microseconds. Hit-test order SHALL follow: chrome layer (always wins), content layer tiles by z-order descending, within each tile nodes in reverse tree order (last child first), first HitRegionNode whose bounds contain the point wins.
Source: RFC 0004 §7.2, RFC 0001 §5.1, DR-I2
Scope: v1-mandatory

#### Scenario: Hit-test under 100 microseconds
- **WHEN** a point query is executed against a scene with 50 tiles, each containing multiple HitRegionNodes
- **THEN** the hit-test SHALL complete in < 100 microseconds

#### Scenario: Chrome layer always wins hit-test
- **WHEN** a click lands on a pixel where a chrome element and a content tile overlap
- **THEN** the chrome element SHALL win the hit-test and the content tile SHALL NOT receive the event

---

### Requirement: Pointer Event Types
The runtime SHALL support the following pointer event types: PointerDownEvent, PointerUpEvent, PointerMoveEvent, PointerEnterEvent, PointerLeaveEvent, ClickEvent, DoubleClickEvent, ContextMenuEvent, and PointerCancelEvent. All pointer events SHALL carry appropriate fields including tile_id, node_id, device_id, coordinates (node-local and display-space), modifiers, and timestamp_mono_us (OS hardware event timestamp, monotonic domain).
Source: RFC 0004 §7.3
Scope: v1-mandatory

#### Scenario: PointerDownEvent carries all required fields
- **WHEN** the user presses the left mouse button on a HitRegionNode
- **THEN** the dispatched PointerDownEvent SHALL include tile_id, node_id, device_id, button=PRIMARY, node-local x/y, display-space x/y, modifiers, timestamp_mono_us, and interaction_id

---

### Requirement: Keyboard Event Types
The runtime SHALL support KeyDownEvent, KeyUpEvent, and CharacterEvent for focused nodes. KeyDownEvent SHALL carry physical key_code (DOM KeyboardEvent.code), logical key value (DOM KeyboardEvent.key), modifiers, repeat flag, and timestamp_mono_us. CharacterEvent SHALL carry post-IME committed Unicode characters. Keyboard events SHALL target the focused node first, then bubble to the tile if the node does not consume them.
Source: RFC 0004 §7.4
Scope: v1-mandatory

#### Scenario: KeyDownEvent targets focused node
- **WHEN** node N1 in tile T1 has focus and the user presses the 'A' key
- **THEN** the agent SHALL receive KeyDownEvent(tile_id=T1, node_id=N1, key_code="KeyA", key="a") and CharacterEvent(character="a")

#### Scenario: Key events bubble to tile
- **WHEN** a focused node does not consume a key event
- **THEN** the key event SHALL bubble to the tile-level handler for the owning agent

---

### Requirement: Event Dispatch Flow
Event dispatch SHALL follow a five-phase pipeline (RFC 0004 §8.1): Stage 1 (Input Drain, < 500 microseconds p99) attaches hardware and arrival timestamps and enqueues the raw input; Stage 2 (Local Feedback, < 500 microseconds p99) performs hit-testing against the bounds snapshot, updates HitRegionLocalState, and produces SceneLocalPatch; the compositor thread (Stage 3-4) applies SceneLocalPatch to render state and forwards events to the event router; the event router resolves owning agents (< 2ms from Stage 2 completion) and serializes events to protobuf per-agent EventBatch; the network thread delivers EventBatch on the agent's gRPC session stream. Stages 1 and 2 execute entirely on the main thread; Stage 3-4 and the event router run on the compositor thread.
Source: RFC 0004 §8.1
Scope: v1-mandatory

#### Scenario: End-to-end dispatch within budget
- **WHEN** a PointerDownEvent enters Stage 1
- **THEN** local feedback SHALL be complete within 1ms (Stages 1+2 combined, p99), and event routing to the agent's gRPC stream SHALL complete within 2ms of Stage 2 completion (p99)

---

### Requirement: Event Routing Resolution
The event router SHALL resolve owning agents: NodeHit maps to the tile's lease owner session; TileHit maps to the tile's lease owner session; Chrome events are handled locally with no agent notification; Passthrough events pass to the desktop in overlay mode or are discarded in fullscreen. Keyboard events SHALL route to the session owning the currently focused tile/node. Captured pointer events SHALL route to the capturing session, bypassing hit-test. CommandInputEvent SHALL route to the focused session; if focus is None, NAVIGATE_NEXT SHALL advance to the first focusable element.
Source: RFC 0004 §8.2
Scope: v1-mandatory

#### Scenario: Event routed to lease owner
- **WHEN** a PointerDownEvent hits node N1 in tile T1, which is leased by Agent A
- **THEN** the event SHALL be routed to Agent A's session stream

#### Scenario: Command input with no focus advances focus
- **WHEN** a NAVIGATE_NEXT command arrives and focus is None
- **THEN** the runtime SHALL advance focus to the first focusable element in the active tab

---

### Requirement: Event Serialization and Batching
Events SHALL be serialized as protobuf messages in InputEnvelope wrapped in EventBatch. Multiple input events for the same agent within a single frame SHALL be batched into a single EventBatch and delivered as a single SessionMessage (field 34). Within a batch, events SHALL be ordered by timestamp_mono_us ascending.
Source: RFC 0004 §8.3, §8.4
Scope: v1-mandatory

#### Scenario: Same-frame events batched
- **WHEN** two pointer events for the same agent occur within the same frame
- **THEN** they SHALL be delivered in a single EventBatch with events ordered by timestamp_mono_us

---

### Requirement: Event Coalescing Under Backpressure
PointerMoveEvent SHALL be coalesced under backpressure: only the latest position is retained. Hover state changes SHALL be coalesced to net state. ScrollOffsetChangedEvent SHALL be coalesced per tile to latest offset. Transactional events (down, up, click, key, focus, capture, IME) SHALL NEVER be dropped or coalesced. The event queue default depth SHALL be 256 events per agent with a hard cap of 4096.
Source: RFC 0004 §8.5
Scope: v1-mandatory

#### Scenario: PointerMove coalesced under backpressure
- **WHEN** the agent's event queue is full and 10 PointerMoveEvents arrive for the same node
- **THEN** only the final position SHALL be retained and delivered to the agent

#### Scenario: Transactional events never dropped
- **WHEN** the agent's event queue is at hard cap and a PointerDownEvent arrives
- **THEN** the PointerDownEvent SHALL be enqueued; non-transactional events SHALL be dropped to make room if necessary

---

### Requirement: Event Dispatch to Agent Latency
Event dispatch to the owning agent (hit-test + session lookup + serialization + enqueue) SHALL complete in < 2ms from Stage 2 completion.
Source: RFC 0004 §8.2, DR-I5
Scope: v1-mandatory

#### Scenario: Routing within 2ms
- **WHEN** Stage 2 completes and produces an InputEvent for Agent A
- **THEN** the event SHALL be serialized and enqueued to Agent A's EventBatch within 2ms

---

### Requirement: Protobuf Schema for Input Events
The input event system wire format is defined in events.proto (package tze_hud.protocol.v1) per the normative three-file proto layout mandated by session-protocol. There is no separate input.proto package. The schema SHALL include: InputEnvelope (22-variant oneof covering all pointer, keyboard, focus, capture, gesture, IME, scroll, and command events), EventBatch (frame_number, batch_ts_us, repeated InputEnvelope), and all individual event messages. All tile_id and node_id fields SHALL use SceneId (imported from types.proto). All timestamp fields SHALL use the _mono_us suffix for hardware timestamps (monotonic domain).
Source: RFC 0004 §9.1
Scope: v1-mandatory

#### Scenario: SceneId used for tile and node identifiers
- **WHEN** a PointerDownEvent is serialized
- **THEN** tile_id and node_id SHALL be encoded as SceneId (16-byte little-endian UUIDv7), not as strings

---

### Requirement: Command Input Model
The runtime SHALL support seven abstract command actions: NAVIGATE_NEXT, NAVIGATE_PREV, ACTIVATE, CANCEL, CONTEXT, SCROLL_UP, SCROLL_DOWN. These commands SHALL be delivered via CommandInputEvent in the EventBatch pipeline. CommandInputEvent SHALL carry tile_id, node_id, interaction_id, timestamp_mono_us, device_id, action, and source (KEYBOARD, DPAD, VOICE, REMOTE_CLICKER, ROTARY_DIAL, PROGRAMMATIC). CommandInputEvent SHALL be a transactional event (never coalesced or dropped).
Source: RFC 0004 §10
Scope: v1-mandatory

#### Scenario: D-pad maps to NAVIGATE_NEXT
- **WHEN** a glasses temple D-pad down button is pressed with focus on a HitRegionNode
- **THEN** the runtime SHALL dispatch CommandInputEvent(action=NAVIGATE_NEXT, source=DPAD) and execute focus cycling

---

### Requirement: ACTIVATE Local Feedback
The ACTIVATE command SHALL trigger the same local feedback as PointerDownEvent on the focused HitRegionNode: pressed state via SceneLocalPatch in Stage 2. The latency budget SHALL be the same: input_to_local_ack p99 < 4ms. The rollback path on agent rejection SHALL apply.
Source: RFC 0004 §10.5
Scope: v1-mandatory

#### Scenario: ACTIVATE produces pressed state
- **WHEN** a CommandInputEvent(action=ACTIVATE) arrives for a focused HitRegionNode
- **THEN** the runtime SHALL set pressed=true on the node in the same frame and dispatch CommandInputEvent to the owning agent

---

### Requirement: Focus Ring Visual Indication
The runtime SHALL render a focus ring on the currently focused HitRegionNode or tile boundary. The focus ring SHALL be rendered in the chrome layer (above all agent content). Default style SHALL be 2px solid ring with system accent color and minimum 3:1 contrast ratio against the tile's background. Focus ring updates SHALL occur within Stage 2 of the frame pipeline.
Source: RFC 0004 §5.6
Scope: v1-mandatory

#### Scenario: Focus ring appears on focused node
- **WHEN** focus transfers to a HitRegionNode
- **THEN** a focus ring SHALL be rendered at the node's bounds in the same frame as the focus transfer event

---

### Requirement: Pointer-Free Navigation
All interactions achievable with pointer input MUST also be achievable without a pointer using only command input. This includes: click (ACTIVATE), context menu (CONTEXT), scroll (SCROLL_UP/SCROLL_DOWN), tab close (focus + CANCEL), and focus cycling (NAVIGATE_NEXT/NAVIGATE_PREV). The runtime SHALL provide these command events; agents that handle ACTIVATE and CANCEL SHALL work on all input profiles without modification.
Source: RFC 0004 §5.7, DR-I9, DR-I12
Scope: v1-mandatory

#### Scenario: All elements reachable on pointer-free device
- **WHEN** the display node has InputCapabilitySet(has_pointer=false, has_dpad=true)
- **THEN** every interactive HitRegionNode with accepts_focus=true SHALL be reachable via NAVIGATE_NEXT/NAVIGATE_PREV commands, and ACTIVATE SHALL produce the same agent-facing effect as a click

---

### Requirement: Headless Testability
All input behavior SHALL be exercisable without a display server or physical GPU (DR-I11). The hit-test pipeline SHALL operate on pure Rust data structures. HitRegionLocalState updates SHALL be assertable from Layer 0 tests with injected input events. The gesture recognizer state machines SHALL accept synthetic event streams with injectable timestamps. The test scenes input_highlight and chatty_dashboard_touch SHALL pass in headless CI.
Source: RFC 0004 §6.1a, DR-I11
Scope: v1-mandatory

#### Scenario: Hit-test in headless environment
- **WHEN** a Layer 0 test injects a synthetic PointerDownEvent at coordinates (50, 50) against a scene graph with known tile bounds
- **THEN** the hit-test SHALL return the correct HitTestResult without requiring a GPU, display server, or winit instance

---

### Requirement: ContextMenu Dispatch (Pointer)
On pointer platforms, a right-click SHALL be mapped directly to ContextMenuEvent by the event preprocessor, bypassing the gesture recognizer pipeline. ContextMenuEvent SHALL NOT be dispatched as a GestureEvent.
Source: RFC 0004 §3.2, §3.3
Scope: v1-mandatory

#### Scenario: Right-click produces ContextMenuEvent
- **WHEN** the user right-clicks on a HitRegionNode with event_mask.context_menu=true
- **THEN** the agent SHALL receive a ContextMenuEvent (not a GestureEvent) with the node's tile_id, node_id, coordinates, and device_id

---

### Requirement: Gesture Recognition Latency
Each gesture recognizer update SHALL complete in < 50 microseconds. Total gesture recognition from the final event to winner selection SHALL complete in < 1ms.
Source: RFC 0004 §3.4, DR-I6
Scope: v1-reserved

#### Scenario: Tap recognition within budget
- **WHEN** a PointerUpEvent completes a tap sequence (down+up within 150ms, < 10px movement)
- **THEN** the Tap recognizer SHALL reach RECOGNIZED state and the GestureEvent SHALL be dispatched within 1ms of the PointerUpEvent

---

### Requirement: V1 Gesture Fallback
V1 MAY ship with tap/click recognition only (Tap, DoubleTap, ContextMenu via right-click). The full gesture pipeline including LongPress, Drag, Pinch, Swipe, and the full arbiter is v1-reserved. When the full pipeline is not present, pointer events SHALL carry raw down/up/move events and agents MAY implement their own gesture logic.
Source: RFC 0004 §3.0
Scope: v1-reserved

#### Scenario: Tap and DoubleTap available in v1
- **WHEN** v1 ships with the minimal gesture set
- **THEN** Tap (pointer_up within 150ms of pointer_down, <= 10px movement) and DoubleTap (two taps within 300ms, <= 20px apart) SHALL be recognized and dispatched as GestureEvents

---

### Requirement: IME V1 Fallback
V1 MAY ship without active IME composition support. Direct ASCII and basic Unicode keyboard input via KeyDownEvent and CharacterEvent SHALL be v1-mandatory. The full IME composition protocol (ImeCompositionStarted, Updated, Committed, Cancelled) is v1-reserved with stable schema defined in events.proto.
Source: RFC 0004 §4.0
Scope: v1-reserved

#### Scenario: CharacterEvent for basic text input
- **WHEN** the user types 'a' on a keyboard with no IME active, and a node has focus
- **THEN** the agent SHALL receive CharacterEvent(character="a") as a v1-mandatory event

---

### Requirement: Accessibility Tree V1 Fallback
V1 SHALL ship the accessibility tree data structures (AccessibilityConfig on HitRegionNode) and metadata fields. The platform API bridge (AT-SPI2, UIA, NSAccessibility) is v1-reserved. The a11y tree SHALL be updated within 100ms of any scene change. Keyboard-only navigation SHALL be v1-mandatory as it depends only on the focus model and event routing.
Source: RFC 0004 §5.0, §5.2, DR-I10
Scope: v1-reserved

#### Scenario: A11y tree updated after scene change
- **WHEN** a new tile is created with accessibility_label="Weather Panel"
- **THEN** the a11y tree SHALL include a new node with role=Region and label="Weather Panel" within 100ms of the scene change

---

### Requirement: Full Gesture Pipeline
The full gesture pipeline SHALL support six recognizer types (Tap, DoubleTap, LongPress, Drag, Pinch, Swipe) running in parallel with an arbiter for conflict resolution. Conflict resolution SHALL follow specificity priority: Pinch > LongPress > Swipe > Drag > DoubleTap > Tap. Cross-tile gestures SHALL be owned by the tile where the gesture starts.
Source: RFC 0004 §3.1-§3.6
Scope: v1-reserved

#### Scenario: Gesture conflict resolved by specificity
- **WHEN** a touch sequence qualifies as both a Tap and the beginning of a LongPress
- **THEN** the LongPress recognizer SHALL delay recognition until the hold threshold (500ms) or Tap window closure, and the higher-specificity gesture SHALL win

---

### Requirement: Full IME Composition Protocol
The runtime SHALL support the full IME composition lifecycle: ImeCompositionStartedEvent, ImeCompositionUpdatedEvent (provisional text with cursor and selection), ImeCompositionCommittedEvent (final text), and ImeCompositionCancelledEvent. IME composition window positioning SHALL be provided to the OS via SetImePositionRequest. IME composition SHALL be cancelled on focus loss with ImeCompositionCancelledEvent dispatched before FocusLostEvent.
Source: RFC 0004 §4.2-§4.5
Scope: v1-reserved

#### Scenario: IME cancelled on focus loss
- **WHEN** a node with active IME composition loses focus
- **THEN** ImeCompositionCancelledEvent SHALL be dispatched before FocusLostEvent

---

### Requirement: Platform Accessibility Bridge
The tze_hud_a11y crate SHALL bridge the runtime's a11y tree to platform-native APIs: UI Automation on Windows, NSAccessibility on macOS, AT-SPI2 on Linux. Screen reader announcements SHALL be rate-limited to at most one assertive announcement per 500ms.
Source: RFC 0004 §5.8, §5.5
Scope: v1-reserved

#### Scenario: Screen reader announcement rate-limited
- **WHEN** a tile with live=true and live_politeness=ASSERTIVE changes content 5 times within 500ms
- **THEN** at most one assertive announcement SHALL be delivered to the platform a11y API within that 500ms window

---

### Requirement: Drag-and-Drop Protocol (Post-v1)
Cross-tile and cross-agent drag-and-drop is explicitly deferred to post-v1. V1 tiles that need drag-and-drop MUST implement a custom protocol over pointer events.
Source: RFC 0004 §12.1, §14
Scope: post-v1

#### Scenario: No built-in cross-tile DnD in v1
- **WHEN** a drag gesture starts in tile T1 and the pointer moves to tile T2
- **THEN** T1's agent SHALL receive all drag events (coordinates may extend beyond T1 bounds) but no built-in drag-and-drop handoff protocol SHALL exist between T1 and T2

---

### Requirement: Stylus and Pressure Input (Post-v1)
Stylus-specific properties (pressure, tilt, twist) are explicitly deferred post-v1, and v1 pointer events MUST carry basic coordinates only.
Source: RFC 0004 §12.4, §14
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** v1 ships
- **THEN** stylus pressure, tilt, and twist fields SHALL NOT be present in PointerDownEvent or PointerMoveEvent

---

### Requirement: Configurable Tab Order (Post-v1)
Agent-controlled tab_index on HitRegionNode is deferred post-v1. V1 SHALL use z-ascending tree-order traversal exclusively. The tab_index field SHALL be reserved in the proto schema for future use.
Source: RFC 0004 §1.5, §14
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** v1 ships
- **THEN** focus cycling SHALL use z-ascending tree order only; tab_index fields on HitRegionNode SHALL be ignored if present
