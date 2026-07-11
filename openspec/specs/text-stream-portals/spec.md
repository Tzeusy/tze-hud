# text-stream-portals Specification
Status: implemented

## Purpose
Defines transport-agnostic text stream portal behavior for resident raw-tile pilots, including output rendering, bounded input, session metadata, and cooperative projection integration points.

Implementation: crates/tze_hud_projection/; crates/tze_hud_runtime/src/portal_projection_driver.rs; crates/tze_hud_compositor/src/markdown.rs; crates/tze_hud_compositor/src/overflow.rs; crates/tze_hud_input/src/composer_draft.rs; crates/tze_hud_input/src/portal_resize.rs; crates/tze_hud_projection/src/portal_cadence.rs; crates/tze_hud_config/src/portal_tokens.rs
## Requirements
### Requirement: Transport-Agnostic Stream Boundary

The text stream portal capability SHALL be defined in terms of generic output text streams, bounded input submission, session identity, and session status metadata. The runtime-facing contract MUST NOT depend on tmux-specific, PTY-specific, terminal-emulator-specific, or chat-provider-specific semantics. Resident portal traffic MUST continue to ride the existing primary bidirectional session stream rather than introducing a second long-lived portal stream per agent. If portal payloads carry ordering, expiry, unread-window, or latency metadata, those fields MUST follow the existing clock-domain contract: wall-clock fields end in `_wall_us`, monotonic fields end in `_mono_us`, and arrival timestamps remain advisory rather than presentation-authoritative.

#### Scenario: tmux adapter satisfies contract

- **WHEN** a tmux adapter publishes incremental text output and accepts bounded input submission
- **THEN** it SHALL satisfy the portal capability without the runtime needing tmux window IDs, pane IDs, or PTY control semantics

#### Scenario: non-tmux adapter satisfies contract

- **WHEN** a chat-platform adapter or LLM-session adapter publishes the same generic output/input/session signals
- **THEN** it SHALL be usable through the same portal capability without changing the runtime contract

#### Scenario: resident portal does not create a second stream

- **WHEN** a resident adapter drives a text stream portal
- **THEN** all portal-related publication, control, and lease traffic SHALL remain on the existing primary session stream

#### Scenario: portal timing metadata uses typed clock domains

- **WHEN** a portal adapter supplies expiry, unread-window, scheduling, or latency metadata
- **THEN** wall-clock semantics SHALL use `_wall_us`, monotonic semantics SHALL use `_mono_us`, and those fields SHALL NOT override runtime presentation control

### Requirement: Content-Layer Portal Surface

Text stream portals SHALL render as content-layer surfaces in the pilot phase. Portal affordances and transcript state MUST NOT live in the chrome layer.

#### Scenario: portal renders below chrome

- **WHEN** a portal tile is visible and the chrome layer is also visible
- **THEN** the portal tile SHALL render below chrome like any other content-layer surface

#### Scenario: portal affordance not addressable as chrome

- **WHEN** an agent queries scene topology or receives shell-state information
- **THEN** no portal affordance SHALL appear as a chrome element or shell-owned status control

### Requirement: Phase-0 Raw-Tile Pilot

The first implementation of text stream portals SHALL be expressible as a resident raw-tile pilot using existing node types and existing resident-session contracts. The phase-0 pilot MUST NOT require a terminal-emulator node, transcript-specific node, tmux-aware runtime subsystem, or portal-specific transport RPC.

#### Scenario: pilot uses existing node types

- **WHEN** the pilot portal is rendered
- **THEN** it SHALL be constructible from existing text, solid-color, image, and hit-region primitives rather than a new dedicated node type

#### Scenario: adapter-side ANSI color with inline runs

- **WHEN** a portal adapter receives ANSI-colorized terminal output (e.g. `\e[31mERROR\e[0m: disk full`)
- **THEN** the adapter SHALL parse ANSI escape sequences and map each color run to a `TextColorRun` entry on the `TextMarkdownNode`
- **AND** the adapter SHALL strip ANSI escape sequences from the `content` string and populate `color_runs` with byte offsets into the stripped content
- **THEN** the runtime SHALL render the colored spans without a terminal-emulator node
- **AND** terminal emulation (PTY semantics, cursor positioning, scrollback) remains out of scope per Phase 0 boundary

Note: `TextMarkdownNode.color_runs` (RFC 0001 §TextMarkdownNode, scene-graph/spec.md §TextMarkdownNode inline color_runs) enables this adapter pattern. No new node type is required.

#### Scenario: pilot uses resident session flow

- **WHEN** the pilot portal is active
- **THEN** the owning authority SHALL be a resident authenticated actor using the existing session and lease model

### Requirement: Bounded Transcript Viewport

Phase-0 portal transcript rendering SHALL be a bounded viewport rather than an unbounded transcript store. Any transcript text materialized into scene nodes MUST remain within existing node-size and per-tile resource budgets. Full retained history, if any, SHALL live outside the scene graph.

#### Scenario: visible transcript fits node budget

- **WHEN** a portal renders an expanded transcript window
- **THEN** the text content placed into scene nodes SHALL stay within the existing TextMarkdown node size limits and tile resource budgets

#### Scenario: full history not mirrored into scene nodes

- **WHEN** a portal session has transcript history larger than the currently visible window
- **THEN** the runtime SHALL represent only the bounded visible or immediately scrollable window in scene nodes rather than mirroring the full retained history

### Requirement: Low-Latency Text Interaction

Text stream portals SHALL support low-latency incremental output, bounded viewer input submission, and local-first interaction feedback. Output transcript updates SHALL behave as state-stream traffic. Viewer submit and explicit control actions SHALL behave as transactional traffic. Activity indicators and similar transient state SHALL behave as ephemeral realtime traffic. Composer draft-change notifications SHALL behave as state-stream traffic coalescible to the latest draft snapshot. Local-first interaction feedback for portal affordances and composer editing SHALL meet the engineering-bar latency budgets: input to local ack p99 < 4 ms (≤ 2 ms under the Windows locked lane) and input to next present p99 < 33 ms (≤ 16.6 ms under the Windows locked lane), per `about/craft-and-care/engineering-bar.md` §2.

#### Scenario: output updates stream incrementally

- **WHEN** new text output arrives in multiple increments
- **THEN** the portal SHALL present those increments as an ordered streamed interaction rather than only as a final snapshot

#### Scenario: viewer submit is transactional

- **WHEN** the viewer submits a reply or activates an interrupt-style control
- **THEN** that action SHALL be treated as transactional input rather than coalescible state-stream traffic

#### Scenario: portal interaction meets latency budgets

- **WHEN** the viewer activates a portal affordance or edits the composer draft on the reference Windows lane
- **THEN** local acknowledgement SHALL render within the input-to-local-ack budget
- **AND** the resulting presentation SHALL occur within the input-to-next-present budget

#### Scenario: draft notifications are state-stream class

- **WHEN** composer draft changes are delivered to the owning adapter
- **THEN** they SHALL be classified as state-stream traffic eligible for latest-snapshot coalescing
- **AND** they SHALL NOT be classified as transactional or ephemeral-realtime traffic

### Requirement: Transcript Interaction Contract

Expanded text stream portals SHALL support scrollable transcript viewing, focusable interaction affordances, bounded composer draft editing, and bounded reply submission under the existing local-feedback contract. Portal interaction MUST reuse runtime-owned focus, command-input, and local visual acknowledgement rules. User scroll input MUST remain authoritative over any adapter-driven attempt to reposition the transcript viewport. Composer draft editing — caret movement, selection, deletion, and capped paste — SHALL receive local-first visual feedback from the runtime-owned draft state without requiring an adapter round trip, while submission remains a bounded transactional action.

#### Scenario: transcript scroll is local-first

- **WHEN** the viewer scrolls an expanded transcript surface
- **THEN** the visible scroll offset SHALL update locally before any adapter acknowledgement is required

#### Scenario: portal control uses local feedback

- **WHEN** the viewer activates an expand, collapse, or reply affordance
- **THEN** the runtime SHALL provide local-first interaction feedback under the same latency contract as other interactive surfaces

#### Scenario: caret and selection update locally

- **WHEN** the viewer moves the caret or adjusts a selection in the composer draft
- **THEN** the caret and selection rendering SHALL update locally from runtime-owned draft state
- **AND** no adapter response SHALL be required for the visible update

#### Scenario: adapter cannot reposition viewport or draft against the user

- **WHEN** an adapter attempts to reposition the transcript viewport or alter the draft while the viewer is actively scrolling or editing
- **THEN** user scroll input SHALL remain authoritative over the viewport
- **AND** the locally rendered draft state SHALL NOT be overwritten mid-edit by adapter traffic

### Requirement: Coherent Transcript Coalescing

State-stream coalescing for portal output MUST preserve a coherent retained transcript window. Intermediate render states MAY be skipped, but already-committed logical transcript units within the retained on-screen history window MUST NOT be lost merely because updates were coalesced.

#### Scenario: coalescing preserves retained window

- **WHEN** several portal append operations are coalesced under backpressure
- **THEN** the runtime MAY render a newer complete transcript window snapshot but SHALL NOT collapse the retained window into only the latest line

### Requirement: Governance, Privacy, and Override Compliance

Text stream portals SHALL obey the same lease, privacy, redaction, dismissal, freeze, and safe-mode rules as any other governed surface. Portal identity, transcript content, and activity metadata MUST NOT bypass viewer-class filtering or shell overrides because they are text. Collapsed portal cards MUST NOT be treated as automatically safe metadata. When the owning session disconnects, the lease orphan lifecycle and the viewer-facing disconnect/resume presentation SHALL stay coherent: the visible disconnect, stale, and resume treatments defined by the Portal Disconnect Presentation, Portal Stale-Content Degradation Contract, and Portal Reconnect and Resume Presentation requirements operate within the bounds of the lease orphan/grace lifecycle, and grace expiry removes the surface under the existing lease rules.

#### Scenario: portal content redacts under viewer policy

- **WHEN** portal content exceeds the current viewer's permitted classification
- **THEN** the portal SHALL be redacted under the runtime's existing privacy policy rather than exposing the transcript content

#### Scenario: portal suspends under safe mode

- **WHEN** the runtime enters safe mode while a portal is active
- **THEN** portal updates SHALL suspend under the same shell and lease rules as other content-layer surfaces

#### Scenario: collapsed portal preserves geometry while redacted

- **WHEN** the current viewer is not permitted to see a portal's identity or transcript state
- **THEN** the portal SHALL preserve its geometry, suppress transcript previews and activity details, and replace visible content with the runtime's neutral redaction treatment

#### Scenario: disconnected portal follows orphan path

- **WHEN** the owning resident portal session disconnects unexpectedly
- **THEN** the lease SHALL transition through the normal orphan lifecycle, the visible portal SHALL present the degraded/stale treatment over its last coherent state per the Portal Disconnect Presentation requirement, and grace expiry SHALL remove the governed surface under the existing lease rules
- **AND** a re-attach before grace expiry SHALL resume per the Portal Reconnect and Resume Presentation requirement rather than starting a fresh portal

#### Scenario: freeze does not disclose viewer intent

- **WHEN** a portal is active while the runtime freezes visible scene mutation
- **THEN** adapters SHALL observe only the existing generic queue-pressure or dropped-mutation semantics rather than a portal-specific freeze signal

### Requirement: Ambient Portal Attention Defaults

Text stream portal activity indicators, unread state, and transcript churn SHALL default to ambient or gentle presentation. Portal activity MUST NOT self-escalate interruption class merely because a stream is active, a backlog is growing, or new transcript units continue to arrive.

#### Scenario: unread backlog does not auto-upgrade urgency

- **WHEN** a portal accumulates unread transcript updates without explicit higher-priority content policy
- **THEN** the runtime SHALL keep the portal's activity presentation ambient or gentle rather than upgrading it to a stronger interruption class solely because of backlog growth

#### Scenario: typing indicator remains ambient

- **WHEN** a live portal exposes typing or activity indicators
- **THEN** those indicators SHALL remain subordinate to the runtime's existing attention model and SHALL NOT behave like repeated notifications

### Requirement: External Adapter Isolation

Any adapter that emits portal output, accepts viewer input, or requests portal visibility SHALL remain external to the runtime core and SHALL pass through existing authentication and capability boundaries. The runtime core MUST NOT gain implicit authority over external process or transport lifecycles as a side effect of supporting text stream portals.

#### Scenario: local adapter still authenticates

- **WHEN** a local adapter process connects to drive a text stream portal
- **THEN** it SHALL authenticate and operate under explicit capability grants rather than implicit local trust

#### Scenario: runtime does not become process host

- **WHEN** a tmux-backed portal is active
- **THEN** the runtime SHALL remain unaware of tmux-specific lifecycle management beyond the generic stream signals it receives

### Requirement: Cooperative LLM Projection Adapter
Text stream portals SHALL support cooperative LLM-session projection as a valid non-tmux adapter family. A cooperative projection adapter SHALL publish generic output stream updates, session identity, lifecycle status, and bounded input state to the portal contract while keeping provider-specific LLM behavior outside the runtime core. Cooperative projection MUST NOT add PTY attachment, terminal-emulator semantics, provider-specific portal RPCs, or runtime ownership of LLM process lifecycles.

#### Scenario: cooperative adapter satisfies portal boundary
- **WHEN** a Codex, Claude, opencode, or similar LLM session opts into HUD projection through an external daemon
- **THEN** the resulting portal SHALL use the existing text-stream portal semantics for output, bounded input, session identity, status metadata, and lifecycle state
- **AND** the runtime-facing behavior SHALL remain independent of the provider's CLI or chat implementation details

#### Scenario: cooperative adapter does not imply process hosting
- **WHEN** a cooperative projection shows an already-running LLM session on the HUD
- **THEN** the runtime SHALL remain unaware of that LLM process lifecycle beyond generic portal session metadata
- **AND** the runtime SHALL NOT acquire authority to start, stop, inspect, or inject input into the provider process

### Requirement: Cooperative Projection Input Mapping
HUD input submitted through a cooperative projected portal SHALL remain bounded viewer input under the existing portal input contract. The portal adapter SHALL map submitted text into its cooperative inbox or equivalent semantic input mechanism, not into raw terminal keystrokes, shell commands, or provider-specific byte streams.

#### Scenario: submitted text maps to semantic inbox
- **WHEN** the operator submits text from an expanded cooperative projected portal
- **THEN** the adapter SHALL treat the submission as a transactional bounded input item for the owning projected session
- **AND** it SHALL expose that input to the active LLM session through the cooperative projection contract

#### Scenario: raw keystroke passthrough remains out of scope
- **WHEN** the operator edits text in the portal composer before submitting
- **THEN** runtime character, focus, and command events MAY support local composition behavior
- **BUT** the adapter SHALL NOT interpret ordinary editing keystrokes as terminal input to the projected LLM process

### Requirement: Viewer Reply Echo

A text stream portal SHALL maintain two distinct, separately-bounded histories: an INPUT history of the viewer's own accepted submissions, and an OUTPUT transcript of agent-authored content only. The two histories are separate streams and SHALL NOT be materialized as a single combined transcript unit sequence.

When a viewer submits a reply through a text stream portal composer and the submission is accepted, the runtime SHALL echo the submitted text into the portal's INPUT history as a viewer-authored turn, so the two-way conversation is visible on the surface rather than the viewer's own words disappearing into the adapter inbox. The INPUT history SHALL be presented in the portal's input region beneath a top-anchored composer, with successive viewer turns stacked and separated by token-styled turn dividers (the viewer-echo stack); the runtime SHALL retain only a bounded, newest-fit window of the INPUT history, obeying the Bounded Transcript Viewport rules for its own region rather than mirroring unbounded input history into scene nodes. An accepted viewer submission SHALL NOT be appended to the OUTPUT/agent transcript stream, and SHALL NOT jump, republish, or otherwise mutate the OUTPUT transcript's scroll position — the viewer's submitted words appear in the INPUT history only, never doubled into the agent-authored transcript.

The viewer turn SHALL be authored by the runtime at submit time and SHALL be distinguishable from agent-authored transcript units by a dedicated viewer turn kind. The output-publication contract addresses the OUTPUT transcript only: an adapter MUST NOT author, publish, or otherwise forge a viewer turn through the output-publication contract, an adapter has no path to write into the INPUT history at all, and a publish that attempts to use the viewer turn kind SHALL be rejected. The echoed viewer turn SHALL carry the submission's content classification and SHALL obey the same redaction, safe-mode, freeze, and Bounded Transcript Viewport rules as agent-authored transcript content — it is not automatically safe because it is the viewer's own text. The viewer echo is local-first presentation, not a new attention event: it SHALL NOT increment the portal's unread-output count and SHALL NOT escalate interruption class, because the viewer has by definition already seen their own message, consistent with the Ambient Portal Attention Defaults requirement. The echo is a presentation of an already-accepted submission and SHALL NOT alter the existing submission contract: the submitted text SHALL still be delivered transactionally to the adapter's semantic input mechanism per the Cooperative Projection Input Mapping requirement, and a submission that is rejected SHALL NOT be echoed. Visual differentiation of viewer versus agent turns (the two-region layout, alignment, role accent, attribution affordance, and divider treatment) is governed by the portal's component-profile and design tokens and is not mandated at the pixel level by this requirement; the requirement establishes that viewer turns are first-class, kind-distinct units of the INPUT history, held separately from the agent-authored OUTPUT transcript.

Source: RFC 0013 §3.3 and §4.3, `about/heart-and-soul/vision.md` ("a persistent on-screen portal where a person can converse"), `about/heart-and-soul/presence.md` (Interaction — local-first), owner live round-6 decision (2026-07-04, hud-egf39 / PR #1038: "route viewer submissions to INPUT-pane history, not OUTPUT transcript"), `crates/tze_hud_projection/src/contract.rs` (OutputKind::Viewer), `crates/tze_hud_projection/src/authority.rs` (append_viewer_echo on submit_portal_input), `crates/tze_hud_runtime/src/windowed/portal.rs` (append_raw_tile_viewer_echo → viewer_echo_queue → compositor viewer-echo stack, #1020/hud-hsc1t), `crates/tze_hud_runtime/src/portal_projection_driver.rs` (parse_output_kind rejects adapter-supplied viewer), exemplar `text_stream_portal_exemplar.py` (append_input_history records into input_history, never body_full)
Scope: v1-mandatory

#### Scenario: accepted reply appears in the input history

- **WHEN** a viewer submits a reply that the portal accepts
- **THEN** the submitted text SHALL appear in the portal's INPUT history as a viewer-authored, kind-distinct turn beneath the composer, stacked with token-styled turn dividers
- **AND** the submission SHALL still be delivered transactionally to the adapter's semantic input mechanism per the existing Cooperative Projection Input Mapping requirement

#### Scenario: viewer submission never enters the output transcript

- **WHEN** a viewer reply is echoed into the INPUT history
- **THEN** the submitted text SHALL NOT be appended to the OUTPUT/agent transcript stream
- **AND** the OUTPUT transcript's scroll position SHALL NOT jump or republish as a side effect of the submission
- **AND** the viewer's words SHALL appear once, in the INPUT history only, never doubled into the agent-authored transcript

#### Scenario: viewer echo does not count as unread or escalate attention

- **WHEN** a viewer reply is echoed into the INPUT history
- **THEN** the portal's unread-output count SHALL NOT increase
- **AND** the echo SHALL NOT raise the portal's interruption class beyond the ambient default

#### Scenario: adapter cannot forge a viewer turn

- **WHEN** an adapter publishes transcript output using the viewer turn kind
- **THEN** the runtime SHALL reject the publish
- **AND** the adapter SHALL have no path to write into the INPUT history; only the runtime's submit path SHALL author viewer turns

#### Scenario: viewer echo redacts like transcript content

- **WHEN** the current viewer's policy redacts the portal's transcript
- **THEN** the echoed viewer turn in the INPUT history SHALL be redacted under the same policy as agent-authored content
- **AND** it SHALL NOT bypass viewer-class filtering because it is the viewer's own submitted text

#### Scenario: rejected submission is not echoed

- **WHEN** a viewer submission is rejected (for example because the HUD is unavailable or the input queue is full)
- **THEN** no viewer turn SHALL be appended to the INPUT history
- **AND** the existing rejection feedback SHALL convey why the submission did not land

### Requirement: Cooperative Projection State Externality
Cooperative projection adapters SHALL keep retained transcript history, pending input queues, acknowledgement state, and reconnection metadata outside the scene graph and outside the runtime core. The text-stream portal surface SHALL materialize only the bounded visible transcript window, compact collapsed state, and policy-permitted status metadata.

#### Scenario: full projection state remains external
- **WHEN** a cooperative projection has retained transcript history larger than the visible portal viewport
- **THEN** the adapter or projection daemon SHALL retain that history outside the scene graph
- **AND** the portal SHALL render only the bounded visible or immediately scrollable window

#### Scenario: pending input not mirrored into scene graph
- **WHEN** multiple HUD input submissions are waiting for the LLM session
- **THEN** the portal MAY show a bounded pending count or preview permitted by policy
- **BUT** it SHALL NOT mirror the full pending input queue into scene nodes

### Requirement: Phase-1 Markdown Rendering Subset

Text stream portal surfaces SHALL render a defined CommonMark subset for transcript content: ATX headings (levels 1–6), strong emphasis, emphasis, inline code, fenced and indented code blocks, ordered and unordered lists (including nesting), and links rendered as styled non-navigable text. All subset styling (heading scale, emphasis weight/style, code font family and background, list indentation, link treatment) MUST resolve from design tokens rather than hardcoded values. Constructs outside the subset — tables, images, raw HTML, blockquotes, footnotes, strikethrough, task lists, and autolinks — MUST NOT be parsed in Phase 1 and SHALL render as literal source text rather than being silently dropped. Link destinations MUST NOT be navigable, fetched, or previewed; only the link text is styled. Rendered markdown content remains subject to the existing bounded-viewport, node-size, and per-tile resource budget rules.

Source: RFC 0001 §TextMarkdownNode, RFC 0013 §3.4, `about/heart-and-soul/vision.md` (visual identity is modular)

#### Scenario: subset constructs render with token-driven styling

- **WHEN** a portal transcript update contains a heading, bold and italic spans, inline code, a fenced code block, a nested list, and a link
- **THEN** each construct SHALL render with the visual treatment resolved from the active design tokens
- **AND** no construct's color, font, size, or background SHALL come from a hardcoded compositor value

#### Scenario: excluded constructs degrade to literal text

- **WHEN** a portal transcript update contains a markdown table, an image reference, or raw HTML
- **THEN** the surface SHALL render the construct's literal source text
- **AND** no transcript content SHALL be silently dropped

#### Scenario: links are styled text only

- **WHEN** a transcript update contains `[release notes](https://example.com)`
- **THEN** the link text SHALL render with the token-defined link treatment
- **AND** activating, hovering, or rendering the link SHALL NOT trigger navigation, fetching, or preview of the destination

#### Scenario: markdown rendering respects node budgets

- **WHEN** a markdown-heavy transcript window is rendered into scene nodes
- **THEN** the materialized content SHALL stay within the existing text-node size limits and per-tile resource budgets required by the Bounded Transcript Viewport requirement

### Requirement: Markdown Parsing Outside the Frame Loop

Markdown parsing for portal content SHALL occur when content is committed, outside the per-frame stage pipeline, producing a cached styled-run representation keyed by content identity. The frame pipeline MUST consume cached styled runs and MUST NOT re-parse markdown on the render path. A node whose content has not changed MUST incur zero parse cost per frame. Parse work MUST NOT cause the per-frame stage budgets in `about/craft-and-care/engineering-bar.md` §2 (Stage 3 Mutation Intake < 1 ms, Stage 4 Scene Commit < 1 ms, Stage 5 Layout Resolve < 1 ms) to be exceeded.

Source: RFC 0003 §5.1, `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rule "LLMs must never sit in the frame loop"

#### Scenario: unchanged content costs no parse work

- **WHEN** a portal renders for many consecutive frames without a transcript content change
- **THEN** no markdown parsing SHALL execute during those frames
- **AND** rendering SHALL consume the cached styled-run representation

#### Scenario: parse cost does not displace stage budgets

- **WHEN** a maximum-size (65535-byte) markdown payload is committed while the frame pipeline is running
- **THEN** the frame pipeline's Mutation Intake, Scene Commit, and Layout Resolve stages SHALL remain within their engineering-bar budgets
- **AND** parsing SHALL complete off the render path before the styled result is swapped in atomically

### Requirement: Transcript Overflow and Ellipsis Contract

Portal text surfaces using ellipsis overflow SHALL truncate at the last word boundary whose shaped width, including the shaped ellipsis glyph in the same style run, fits the content box; when no word boundary fits, truncation SHALL fall back to the last grapheme-cluster boundary that fits. The surface MUST NOT render partially clipped glyphs, horizontally or vertically: the final visible line is either fully visible or not rendered. Under streaming append, layout MUST remain stable: with follow-tail active the visible window advances by whole layout lines, and while the viewer is scrolled away from the tail the visible content MUST NOT reflow or shift due to appends outside the viewport.

Overflow shaping SHALL run off the per-frame render path: `truncate_for_ellipsis` / `truncate_tail_anchored` (`crates/tze_hud_compositor/src/overflow.rs`) execute at content-commit or geometry-change time, and their result SHALL be cached by `(content_hash, bounds_width, bounds_height, font_size_px)` (the `TruncationCache`, `crates/tze_hud_compositor/src/text.rs`) so the steady-state per-frame cost is a single O(1) lookup. Because each truncation call shapes the entire input string once before locating the cut point, its cost grows with the input length and can exceed the Stage-5 Layout Resolve budget (< 1 ms, `about/craft-and-care/engineering-bar.md` §2) at large transcript windows: the `overflow_truncate` benchmark measures a single uncached call at ~10 ms for an ~8 KiB window and ~94 ms for the ~64 KiB module ceiling, while a ~540 B / 5-line window stays under 1 ms. To keep word-boundary shaping within budget without reverting to clipped glyphs, the surface SHALL invoke the **viewport-adjacent-window fallback**: when a single uncached truncation call for the committed content and geometry would shape an input larger than the viewport-adjacent budget, the input passed to truncation SHALL be restricted to a viewport-adjacent window — the visible layout lines plus a bounded overscan margin of whole lines on each side — rather than the full retained transcript. The trigger SHALL be defined by an input-size threshold, not a per-frame timer: the fallback SHALL engage when the bytes that would be shaped exceed a configured `max_truncation_input_bytes` bound whose default SHALL keep a single uncached call within the Stage-5 budget (≤ the small-window regime, on the order of a few KiB, well below the ~8 KiB point where the benchmark first exceeds 1 ms). When the fallback engages, the visible window and ellipsis placement SHALL be identical to the full-input result for the lines actually within the viewport-adjacent window, and the whole-line and no-partial-glyph guarantees above SHALL still hold; the fallback narrows the measured input, it never relaxes the visual contract.

Source: RFC 0013 §3.4 and §4.2, RFC 0001 §TextMarkdownNode overflow semantics, `about/craft-and-care/engineering-bar.md` §2 (Stage 5 Layout Resolve), `openspec/changes/text-stream-portal-phase1/design.md` §9

#### Scenario: ellipsis lands on a word boundary

- **WHEN** transcript text exceeds the content box with ellipsis overflow active
- **THEN** the truncation point SHALL be the last word boundary that fits together with the measured ellipsis glyph
- **AND** no glyph SHALL be partially clipped at the truncation edge

#### Scenario: long unbroken token falls back to grapheme boundary

- **WHEN** a single unbroken token (for example a long URL or hash) is wider than the content box
- **THEN** truncation SHALL occur at the last grapheme-cluster boundary that fits with the ellipsis glyph
- **AND** the rendered output SHALL NOT split a glyph or combining sequence

#### Scenario: append does not disturb a scrolled-back viewport

- **WHEN** the viewer is scrolled away from the transcript tail and new transcript units append beyond the viewport
- **THEN** the visible lines SHALL NOT reflow, shift, or change truncation points because of the append
- **AND** follow-tail SHALL resume only under the existing user-authoritative scroll contract

#### Scenario: follow-tail advances by whole lines

- **WHEN** follow-tail is active during sustained streaming append
- **THEN** the visible window SHALL advance in whole layout lines
- **AND** no frame SHALL present a vertically half-clipped final line

#### Scenario: large transcript triggers the viewport-adjacent-window fallback

- **WHEN** a content commit would require a single uncached overflow truncation to shape an input larger than the configured `max_truncation_input_bytes` bound (e.g. an ~8 KiB-or-larger retained window that would otherwise exceed the Stage-5 Layout Resolve budget)
- **THEN** truncation SHALL be restricted to a viewport-adjacent window of the visible layout lines plus a bounded whole-line overscan margin, rather than shaping the full retained transcript
- **AND** the resulting visible window and ellipsis placement SHALL match the full-input result for the lines within that window
- **AND** the no-partial-glyph and whole-line visibility guarantees SHALL still hold

#### Scenario: small transcript does not trigger the fallback

- **WHEN** a content commit's truncation input is within the `max_truncation_input_bytes` bound (e.g. a ~540 B / 5-line window that shapes within the Stage-5 budget)
- **THEN** truncation SHALL measure the full committed input
- **AND** the viewport-adjacent-window fallback SHALL NOT engage
#### Scenario: single-line follow-tail surface shows newest content

- **WHEN** a follow-tail surface has room for exactly one layout line and the transcript has more content than fits
- **THEN** the single visible line SHALL show the newest layout line's content with an inline leading ellipsis (`…newest content`), not a dedicated bare ellipsis line that shows no content
- **AND** the leading ellipsis SHALL preserve the omission signal for the dropped earlier lines (and, when the newest line itself overflows horizontally, its dropped leading portion)
- **AND** no glyph SHALL be partially clipped at the truncation edge

### Requirement: Local-First Composer Draft Editing

Expanded portal composers SHALL support bounded draft editing with runtime-owned draft state and local-first echo. The runtime SHALL maintain a plain-text draft buffer per focused composer region and SHALL render draft text, caret, and selection locally within the local-feedback latency contract (input to local ack p99 < 4 ms per `about/craft-and-care/engineering-bar.md` §2; ≤ 2 ms p99 under the Windows locked lane). Supported editing operations are: caret movement (character, word, line-start/end), selection (keyboard and pointer), backspace and delete (character and word-wise), and paste. Draft-change notifications to the owning adapter SHALL be state-stream traffic, coalescible to the latest draft snapshot; draft submission and cancel SHALL remain transactional. Draft editing MUST NOT include IME composition (which remains v1-reserved under the input-model specification), undo/redo, rich text, multi-caret editing, or any interpretation of editing keystrokes as terminal input. Draft content and caret presentation are subject to the same redaction, safe-mode, and focus rules as the rest of the portal surface.

Source: RFC 0013 §4.3 and §8 (open question 1), RFC 0004 focus semantics, `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rule "local feedback first"

#### Scenario: keystroke echoes locally before adapter acknowledgement

- **WHEN** the viewer types a character into a focused portal composer
- **THEN** the character, updated caret position, and any selection change SHALL render locally within the input-to-local-ack budget
- **AND** the visible echo SHALL NOT depend on an adapter round trip

#### Scenario: word-wise delete operates on the local draft

- **WHEN** the viewer performs a word-wise backspace in a non-empty draft
- **THEN** the runtime SHALL remove the preceding word from the local draft buffer and update the rendering locally
- **AND** the owning adapter SHALL observe the change only through a coalescible draft-state notification

#### Scenario: draft state notifications coalesce

- **WHEN** the viewer edits rapidly enough that multiple draft changes occur between adapter deliveries
- **THEN** the adapter MAY receive a single latest-draft snapshot rather than per-keystroke events
- **AND** the submitted draft, when submission occurs, SHALL be delivered transactionally and exactly reflect the local buffer at submit time

#### Scenario: editing keystrokes are never terminal input

- **WHEN** the viewer edits a draft in a portal backed by any adapter family
- **THEN** no editing keystroke SHALL be forwarded as a terminal keystroke, shell input, or provider byte stream
- **AND** only the bounded submitted text SHALL reach the adapter's semantic input mechanism per the existing Cooperative Projection Input Mapping requirement

#### Scenario: draft suspends under safe mode

- **WHEN** the runtime enters safe mode while a composer draft is focused
- **THEN** input SHALL be captured in chrome per the existing override contract
- **AND** the draft surface SHALL suspend with the portal without leaking draft content past the active redaction policy

### Requirement: Composer Draft Bounds and Paste Caps

The composer draft buffer SHALL be size-bounded. The hard ceiling is the existing 65535-byte text-node content limit; the runtime MAY enforce a lower configured cap. A paste operation that would exceed remaining capacity SHALL be truncated at a UTF-8 character boundary at the cap, SHALL produce locally visible at-capacity feedback, and MUST NOT corrupt the existing draft or forward untruncated content to the adapter. Cap enforcement is local and synchronous: oversized input MUST be bounded before any draft-state notification leaves the runtime.

Source: RFC 0013 §4.3 (bounded input), scene-graph spec TextMarkdownNode content limit

#### Scenario: oversized paste truncates at the cap

- **WHEN** the viewer pastes content larger than the draft buffer's remaining capacity
- **THEN** the draft SHALL contain the pasted prefix truncated at a UTF-8 character boundary at the cap
- **AND** the surface SHALL show locally visible at-capacity feedback within the local-feedback contract

#### Scenario: cap violation never reaches the adapter

- **WHEN** any combination of typing and paste attempts to exceed the configured draft cap
- **THEN** every draft-state notification and the eventual transactional submission SHALL contain at most the capped draft content
- **AND** the runtime SHALL NOT transmit the rejected overflow bytes to the adapter

### Requirement: Portal Window Management

Expanded portal surfaces SHALL support viewer-driven move and resize with local-first feedback. Move continues through the existing header drag affordance. Resize SHALL be available through both (a) pointer-driven resize affordances on the portal frame (corner or edge capture regions in the content layer) and (b) focus-scoped keyboard shortcuts: while the portal surface holds keyboard focus, Ctrl+`+` (and its unshifted form Ctrl+`=`) SHALL grow and Ctrl+`-` SHALL shrink the portal by a token-defined step. Shortcuts MUST be focus-scoped: a portal that does not hold focus MUST NOT consume them, chrome- and shell-reserved shortcuts take precedence, and safe-mode input capture overrides them entirely. Resize-shortcut handling SHALL be robust to release-only key delivery: when the host input source delivers the resize chord as a key release with no preceding matching resize key press — as Windows `SendInput` does for a held-modifier `=`/`-` chord — the runtime SHALL apply the resize step on the key release as a fallback, and SHALL deduplicate consumed press/release pairs so a normal physical key-down/key-up cycle resizes exactly once. Geometry feedback during a move or resize gesture SHALL render locally within the input-to-local-ack budget (`about/craft-and-care/engineering-bar.md` §2); the owning adapter SHALL observe geometry changes only as coalescible state-stream snapshots and MUST NOT veto or reposition the surface mid-gesture. Resize SHALL clamp to token-defined minimum legible bounds and to maxima within the portal's lease bounds and scene budgets. At every intermediate and final geometry, pane layout SHALL re-resolve under the Transcript Overflow and Ellipsis Contract: no partially clipped glyphs. When transcript or composer content overflows its pane, the surface SHALL show a token-styled scroll-position indicator; the indicator conveys geometry only and SHALL remain present under redaction without revealing content.

Source: RFC 0013 §4.1 and §4.2, RFC 0004 (input model, focus, key press/release semantics), `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rules "local feedback first" and "screen is sovereign"

#### Scenario: pointer resize is local-first

- **WHEN** the viewer drags a portal resize affordance
- **THEN** the portal's geometry SHALL update locally within the input-to-local-ack budget for the duration of the gesture
- **AND** the owning adapter SHALL observe the geometry change only through coalescible state-stream snapshots after the fact

#### Scenario: focused portal responds to resize shortcuts

- **WHEN** a portal surface holds keyboard focus and the viewer presses Ctrl+`+` or Ctrl+`-`
- **THEN** the portal SHALL grow or shrink by the token-defined step with local-first feedback
- **AND** pane layout SHALL re-resolve without partially clipped glyphs at the new geometry

#### Scenario: unfocused portal ignores resize shortcuts

- **WHEN** the viewer presses Ctrl+`+` or Ctrl+`-` while no portal surface holds keyboard focus
- **THEN** no portal SHALL change size
- **AND** the key events SHALL remain available to chrome and other focus targets per the existing input-routing contract

#### Scenario: resize is robust to release-only key streams

- **WHEN** a focused portal receives a resize chord (Ctrl+`=`/`+` or Ctrl+`-`) as a key release with no preceding matching resize key press, as occurs with Windows `SendInput` for a held-modifier chord
- **THEN** the runtime SHALL apply the resize step on the key release as a fallback so the focused portal still grows or shrinks
- **AND** a normal physical key-down/key-up cycle SHALL resize exactly once, because consumed press/release pairs are deduplicated rather than double-applied

#### Scenario: resize clamps to bounds

- **WHEN** repeated shrink or grow operations are applied past the configured limits
- **THEN** the portal SHALL clamp at the token-defined minimum legible bounds and at the maxima permitted by its lease bounds and scene budgets
- **AND** no intermediate geometry SHALL render partially clipped glyphs

#### Scenario: adapter cannot override an active gesture

- **WHEN** the owning adapter publishes portal content or geometry while a viewer move or resize gesture is in progress
- **THEN** the viewer's gesture SHALL remain authoritative for surface geometry until the gesture ends
- **AND** the adapter's content updates SHALL apply within the gesture-defined geometry

#### Scenario: scroll-position indicator is geometry-only under redaction

- **WHEN** transcript content overflows its pane for a viewer whose policy redacts the portal's content
- **THEN** the token-styled scroll-position indicator SHALL remain present and reflect scroll position
- **AND** the indicator SHALL NOT convey transcript content beyond geometry

### Requirement: Sustained Streaming Cadence

Portal output presentation SHALL remain within the engineering-bar budgets under representative streaming workloads. For a sustained stream (appends totaling at least 200 Unicode scalar values per second, in at least 10 increments per second, for at least 60 seconds) and for bursts (at least 4096 bytes arriving within 250 ms), the runtime SHALL hold: total frame time within the frame budget (< 16.6 ms p99 general; `high_mutation` p99 ≤ 8.3 ms and p99.9 ≤ 16.6 ms under the Windows locked lane), concurrent input responsiveness within input-to-local-ack and input-to-next-present budgets, per-stage budgets per `about/craft-and-care/engineering-bar.md` §2, and aggregate portal event traffic within the 1000 events/second ceiling. Coalescing SHALL be work-conserving: when committed transcript units are pending and render capacity exists, a newer coherent window snapshot SHALL be presented. With multiple portals streaming concurrently, coalescing MUST NOT starve any portal: under equal sustained rates, presented-window progress across portals MUST NOT diverge unboundedly. The fairness bound is quantified as a bounded service-count skew: across `N` concurrently streaming portals served by the work-conserving cadence coalescer, the difference between the most-serviced and least-serviced portal SHALL satisfy `max_services - min_services ≤ N` over any measurement interval under equal sustained input rates. This bound is structural — the coalescer services portals in round-robin order (`crates/tze_hud_projection/src/portal_cadence.rs`, `PortalCadenceCoalescer::next_ready_portal`), so no portal's accumulated presentation lag can exceed any other's by more than one complete service round (`≤ N` services) — rather than a hard real-time deadline. The fairness metric SHALL be measured with the `FairnessProbe` service-count instrument (`assert_fair()` enforces `max_services - min_services ≤ portal_count`) over a sustained run satisfying the cadence workload (≥ 200 scalars/s in ≥ 10 increments/s per portal); a portal that records zero services SHALL be counted as starved. Because the coalescer retains only the latest snapshot per portal (state-stream latest-wins), this bounds presentation-round skew, not per-append delivery. Arrival timestamps remain advisory; this requirement defines no arrival-to-presentation deadline, because transcript appends are state-stream traffic whose presentation timing belongs to the runtime. Runtime processing overhead is nonetheless bounded: for an append that is presented (not superseded by a newer coalesced window), elapsed time from mutation arrival to the next present SHALL stay within the `high_mutation` input-to-next-present budget (p99 ≤ 16.6 ms under the Windows locked lane), so the end-to-end latency a remote agent observes is transport RTT plus bounded runtime overhead. Live cadence evidence SHALL record a transport RTT baseline and per-append publish-to-present measurements so the runtime-added overhead is reported explicitly, separate from transport latency. A 60-minute sustained-streaming soak SHALL stay within the ≤ 5 MiB memory-drift budget.

Source: `about/craft-and-care/engineering-bar.md` §2, RFC 0013 §5, CLAUDE.md core rule "arrival time ≠ presentation time"

#### Scenario: sustained stream holds frame and input budgets

- **WHEN** a portal receives a sustained stream while the viewer scrolls and types in the composer
- **THEN** frame time SHALL stay within the applicable frame budgets
- **AND** input-to-local-ack and input-to-next-present SHALL stay within their engineering-bar budgets throughout the stream

#### Scenario: burst collapses to a coherent window, not a stall

- **WHEN** a 4096-byte-in-250-ms burst arrives during an active stream
- **THEN** the runtime MAY coalesce the burst into a newer complete window snapshot per the Coherent Transcript Coalescing requirement
- **AND** the retained window SHALL NOT collapse to only the latest line
- **AND** frame time SHALL remain within budget during the burst

#### Scenario: concurrent portals are not starved

- **WHEN** `N` portals stream at equal sustained rates under coalescing pressure
- **THEN** each portal's presented window SHALL continue to advance with no portal recording zero services
- **AND** the service-count skew across portals SHALL satisfy `max_services - min_services ≤ N`, measured by the `FairnessProbe` instrument

#### Scenario: runtime overhead beyond transport RTT is bounded and evidenced

- **WHEN** the live cadence phase streams appends while recording a transport RTT baseline and per-append publish-to-present timestamps
- **THEN** for appends that are presented rather than coalesced away, arrival-to-present SHALL stay within the `high_mutation` input-to-next-present budget
- **AND** the evidence artifact SHALL report measured runtime overhead separately from transport RTT

#### Scenario: streaming soak does not leak

- **WHEN** a portal sustains the representative stream for 60 minutes
- **THEN** runtime memory drift attributable to the portal path SHALL stay within the ≤ 5 MiB soak budget
- **AND** the bounded viewport SHALL still hold only the retained window at soak end

### Requirement: Portal Component Profile Styling

Portal surface visual identity — frame, header, composer, transcript body, divider, and collapsed card — SHALL resolve from design tokens, with profile-scoped overrides able to reskin the portal without changing adapter logic or runtime behavior. Before promotion, the exemplar adapter MUST source every published visual value from the runtime's resolved token set rather than literal values. After promotion, the portal SHALL be styled through a `text-portal` component type contract defined by a separate component-shape-language delta, consuming `RenderingPolicy` fields like other component types. Collapsed/expanded state transitions SHALL use token-derived treatments built on the existing zone-transition mechanics, and a transition MUST NOT reveal content past the active redaction policy at any frame: a restricted viewer sees the redaction placeholder throughout the transition.

Source: `about/heart-and-soul/vision.md` (visual identity is modular), `about/heart-and-soul/v1.md` (component shape language), RFC 0013 §3.2 and §3.3

#### Scenario: profile swap reskins the portal

- **WHEN** the operator activates a different component profile with portal-relevant token overrides and the portal is re-rendered
- **THEN** the portal's frame, header, composer, transcript, divider, and collapsed-card treatments SHALL reflect the new token values
- **AND** no adapter code change or runtime behavior change SHALL be required

#### Scenario: no literal styling in the exemplar publish path

- **WHEN** the pre-promotion exemplar adapter publishes the portal surface
- **THEN** every color, typography, spacing, and stroke value it publishes SHALL originate from the runtime's resolved design tokens
- **AND** a token value change SHALL propagate to the published portal on the next publish cycle

#### Scenario: transition never flashes redacted content

- **WHEN** a collapsed portal expands while the current viewer is not permitted to see its transcript
- **THEN** every frame of the transition SHALL show the neutral redaction treatment in place of transcript content
- **AND** geometry SHALL be preserved per the existing redaction requirement

### Requirement: Phase-1 Promotion Evidence Gate

Promotion from the raw-tile pilot to a first-class portal surface SHALL occur only after a refreshed live evidence package satisfies all RFC 0013 §7.2 criteria. The package SHALL consist of live exemplar runs on the reference Windows host covering markdown fidelity, overflow correctness, composer draft editing, window management, sustained streaming cadence (including the publish-to-present-vs-RTT overhead measurement), and profile-swap styling, executed against at least two distinct adapter families (the exemplar script adapter and the cooperative projection adapter), with artifacts recorded under `docs/evidence/text-stream-portals/` carrying the engineering-bar reference hardware tag. The package SHALL also include an agent-ergonomics demonstration: an LLM session driving the full portal lifecycle — attach or create, stream output, poll and acknowledge input, detach — exclusively through the vendored skill surface (the cooperative projection contract or its successor), with zero scene-graph mutations authored in the LLM's context; the ceremony observed (operation count, glue required outside the skill) SHALL be recorded alongside the raw-tile complexity observations. The package SHALL also record raw-tile complexity observations (tile counts, mutation batch shapes, workarounds) as the recurring-complexity evidence, and SHALL confirm that governance behavior (redaction, safe mode, freeze, orphan path) remained correct during the Phase-1 runs. A failed or incomplete gate SHALL leave the raw-tile pilot as the authoritative scope; Phase-1 behavioral requirements remain in force on raw tiles regardless of gate outcome.

Source: RFC 0013 §7.2, `about/craft-and-care/engineering-bar.md` §2 (reference hardware tag), `.claude/skills/user-test/SKILL.md` (live exemplar)

#### Scenario: gate passes with complete evidence

- **WHEN** the refreshed live runs cover all six Phase-1 axes across both adapter families with budget-passing, reference-tagged artifacts, governance confirmation, and the agent-ergonomics demonstration
- **THEN** promotion to a first-class portal surface MAY proceed under this change's approval
- **AND** the evidence package SHALL be linked from the promotion implementation work

#### Scenario: agent ergonomics is a gate criterion

- **WHEN** the evidence package lacks an LLM-driven run that exercises the portal lifecycle exclusively through the vendored skill surface
- **THEN** the gate SHALL NOT pass
- **AND** a run in which the LLM authors scene-graph mutations or tile assembly directly in its context SHALL NOT satisfy this criterion

#### Scenario: gate fails closed

- **WHEN** any Phase-1 axis misses its budget, any governance behavior regresses, or only one adapter family produces evidence
- **THEN** promotion SHALL NOT proceed
- **AND** the raw-tile pilot SHALL remain the authoritative portal scope per RFC 0013 §7.2

#### Scenario: evidence without reference tag is informational only

- **WHEN** a live run cannot prove the reference hardware tag and command shape required by the engineering bar
- **THEN** that run SHALL NOT count toward the promotion gate
- **AND** it MAY be recorded as informational evidence only

### Requirement: Promotion Scope Boundary

Promotion, once gated, SHALL permit only: a first-class portal surface or node type (including the scene-mutation schema additions that surface requires) and a `text-portal` component type contract. Promotion MUST NOT change the standing non-goals: no terminal emulation (VT100/ANSI cursor addressing, alternate screen, PTY hosting), no full transcript history materialized in the scene graph, no chrome-layer portal UI, no dedicated portal transport or second long-lived portal stream outside the primary session stream, and no runtime ownership of external process lifecycles. The promoted surface SHALL remain lease-governed, content-layer, redactable, and subordinate to the attention model exactly as the raw-tile pilot is.

Source: RFC 0013 §1.2, §4.4, §7.2, `about/heart-and-soul/v1.md`

#### Scenario: promoted surface keeps governance contracts

- **WHEN** a first-class portal surface replaces the raw-tile assembly after the gate passes
- **THEN** lease ownership, redaction, safe mode, freeze, dismissal, orphan handling, and ambient attention defaults SHALL apply unchanged
- **AND** the surface SHALL remain a content-layer surface below chrome

#### Scenario: promotion does not open excluded scope

- **WHEN** promotion-era implementation proposes PTY hosting, a portal transport stream, chrome-layer portal affordances, scene-graph transcript history, or runtime control of adapter processes
- **THEN** reviewers SHALL reject that work as outside this change and outside the promotion approval
- **AND** such scope SHALL require a new RFC-level decision per RFC 0013 §4.4

### Requirement: Portal Disconnect Presentation

When the stream or session driving a text stream portal drops mid-stream, the portal SHALL retain its last coherent transcript window and SHALL present a visible degraded treatment rather than blanking, freezing silently as if live, or fabricating continued liveness. The retained window MUST preserve every already-committed logical transcript unit per the existing Coherent Transcript Coalescing requirement; a disconnect MUST NOT collapse the retained window or drop committed units. The degraded treatment — dimming, stale marker, and a disconnect affordance — SHALL resolve entirely from design tokens through the existing component-profile path and MUST NOT use hardcoded compositor styling. Live-only activity signals (typing/activity indicators, ephemeral-realtime hover or interim state) SHALL clear on disconnect so the surface does not imply an active stream. The disconnect affordance and any stale marker SHALL convey only connection geometry/state and MUST remain present and content-free under redaction, exactly like the existing scroll-position indicator: a viewer not permitted to see the transcript still sees that the portal is disconnected, but no transcript content is revealed by the disconnect treatment. The disconnect presentation MUST NOT itself escalate interruption class; a portal going stale is ambient state, not a notification, consistent with the Ambient Portal Attention Defaults requirement.

Source: RFC 0013 §3.2 and §4.4, `about/heart-and-soul/vision.md` (visual identity is modular), CLAUDE.md anti-pattern "treating graceful degradation as a bug", `crates/tze_hud_projection/src/contract.rs` (ProjectionLifecycleState::Degraded, HudUnavailable), `crates/tze_hud_projection/src/authority.rs` (mark_hud_disconnected)
Scope: v1-mandatory

#### Scenario: mid-stream drop retains last coherent window

- **WHEN** the driving stream or session drops while a portal has a non-empty retained transcript window
- **THEN** the portal SHALL continue to display the last coherent transcript window
- **AND** no already-committed logical transcript unit within that window SHALL be dropped because of the disconnect

#### Scenario: stale treatment is token-resolved, not hardcoded

- **WHEN** a portal enters the disconnected state and renders its degraded treatment
- **THEN** the dimming, stale marker, and disconnect affordance SHALL resolve from the active design tokens
- **AND** no color, opacity, typography, or stroke value of the degraded treatment SHALL come from a hardcoded compositor value

#### Scenario: liveness signals clear on disconnect

- **WHEN** a portal that was showing a typing or activity indicator loses its driving stream
- **THEN** the typing and activity indicators SHALL clear
- **AND** the surface SHALL NOT present any signal implying the stream is still active

#### Scenario: disconnect indicator is geometry-only under redaction

- **WHEN** the driving stream drops for a portal whose transcript the current viewer is not permitted to see
- **THEN** the disconnect indicator SHALL remain present and reflect the disconnected state
- **AND** it SHALL NOT reveal transcript content or identity beyond the existing neutral redaction treatment

#### Scenario: going stale does not self-escalate attention

- **WHEN** a portal transitions to the disconnected/stale state
- **THEN** the portal's attention presentation SHALL remain ambient or gentle
- **AND** the disconnect SHALL NOT be raised to a stronger interruption class merely because the stream dropped

### Requirement: Portal Stale-Content Degradation Contract

Text stream portals SHALL define when displayed content is considered live versus stale, bounded by the existing lease orphan/grace lifecycle rather than a second independent timer authority. After a bounded liveness gap on the driving stream — no committed transcript progress and no heartbeat/liveness signal within the configured degraded threshold — the portal's connection SHALL be treated as degraded and its displayed content SHALL be marked stale. The degraded window SHALL be bounded by the lease grace already defined for the orphan path: when the lease grace expires, the governed surface is removed under the existing lease rules and the stale content is no longer displayed. Entering and presenting the degraded/stale state is runtime-owned presentation timing: arrival timestamps remain advisory and the runtime decides when to render the degraded transition, consistent with the arrival-time-versus-presentation-time contract. Liveness, disconnect, and degraded-threshold metadata that the surface consumes MUST follow the existing typed clock-domain convention (`_wall_us` for wall-clock, `_mono_us` for monotonic) and MUST NOT introduce a presentation-authoritative arrival deadline.

Source: RFC 0013 §4.4, RFC 0008 (lease grace/orphan lifecycle), `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rule "arrival time ≠ presentation time", `crates/tze_hud_projection/src/authority.rs` (last_disconnect_wall_us, mark_hud_disconnected)
Scope: v1-mandatory

#### Scenario: content goes stale after the degraded threshold

- **WHEN** a portal's driving stream produces no committed progress and no liveness signal for longer than the configured degraded threshold
- **THEN** the portal's connection SHALL be treated as degraded
- **AND** the displayed content SHALL be marked stale under the disconnect presentation treatment

#### Scenario: staleness is bounded by lease grace

- **WHEN** a portal remains disconnected until its lease grace expires
- **THEN** the governed surface SHALL be removed under the existing lease orphan rules
- **AND** the stale content SHALL no longer be displayed after grace expiry

#### Scenario: degraded transition timing is runtime-owned

- **WHEN** the runtime detects the liveness gap that qualifies a portal as degraded
- **THEN** the runtime SHALL decide when to present the degraded transition rather than treating any arrival timestamp as a presentation deadline
- **AND** the degraded treatment SHALL still appear within the bounded liveness/grace window

#### Scenario: degradation metadata uses typed clock domains

- **WHEN** the surface consumes disconnect, heartbeat, or degraded-threshold timing metadata
- **THEN** wall-clock fields SHALL use `_wall_us` and monotonic fields SHALL use `_mono_us`
- **AND** none of these fields SHALL override runtime presentation control

### Requirement: Portal Reconnect and Resume Presentation

When a portal's driving session re-attaches before lease grace expiry, the portal SHALL resume from the retained coherent visible transcript window the projection authority preserved, clear the degraded/stale treatment, and restore live presentation without losing already-committed transcript units. Resume SHALL preserve transcript identity continuity using the projection authority's existing keys: `logical_unit_id` continues to provide idempotency continuity so a replayed publish reusing an already-seen `logical_unit_id` is accepted idempotently without duplicating the unit, and a continuation that updates a unit in place SHALL reuse that unit's `coalesce_key` so the authority replaces it in the retained window rather than appending a duplicate (per the existing coalesce-key in-place update path). Resume MUST NOT require a change to the existing `logical_unit_id` idempotency semantics. Resumed appends SHALL coalesce under the existing state-stream Coherent Transcript Coalescing and Sustained Streaming Cadence rules. Resume SHALL materialize only the bounded retained visible window into scene nodes per the Bounded Transcript Viewport requirement; it MUST NOT reconstruct full transcript history into the scene graph. Pending HUD input and acknowledgement state restored on resume SHALL follow the existing input-inbox contract and MUST NOT be silently dropped by the reconnect. After lease grace expiry (session death), the surface is gone: a subsequent attach SHALL start a fresh portal under a new lease rather than silently reviving the removed surface or presenting pre-death stale content as live. Resume MUST respect the current viewer's redaction policy at every frame of the transition: a restricted viewer never sees transcript content flash during the stale-to-live transition.

Source: RFC 0013 §3.3 and §4.4, `openspec/specs/cooperative-hud-projection/spec.md` (External Projection State Authority — reconnect preserves projection state), `openspec/specs/external-agent-projection-authority/spec.md` (Multi-Session Lifecycle Management — reconnect bookkeeping), `crates/tze_hud_projection/src/contract.rs` (ReconnectBookkeeping struct: reconnect_count, last_reconnect_wall_us), `crates/tze_hud_projection/src/authority.rs` (reconnect-bookkeeping updates, coalesce-key in-place update path), `.claude/skills/hud-projection/SKILL.md` (detach/re-attach)
Scope: v1-mandatory

#### Scenario: reconnect before grace resumes from retained window

- **WHEN** a portal's driving session re-attaches before lease grace expiry
- **THEN** the portal SHALL resume from the retained coherent visible transcript window
- **AND** the degraded/stale treatment SHALL clear and live presentation SHALL resume
- **AND** no already-committed transcript unit from the retained window SHALL be lost

#### Scenario: continued logical unit updates in place via coalesce key

- **WHEN** a logical transcript unit that was in progress at disconnect is continued after reconnect by republishing it with the same `coalesce_key`
- **THEN** the portal SHALL update that unit in place under the existing coalesce-key in-place update path
- **AND** it SHALL NOT render the continuation as a duplicate transcript unit

#### Scenario: replayed logical unit id stays idempotent

- **WHEN** a publish replays an already-seen `logical_unit_id` during or after reconnect
- **THEN** the authority SHALL accept it idempotently without appending or mutating the transcript
- **AND** resume SHALL NOT redefine `logical_unit_id` to mean an in-place update

#### Scenario: resume materializes only the bounded window

- **WHEN** a portal resumes a session whose retained transcript history exceeds the visible viewport
- **THEN** only the bounded retained visible or immediately scrollable window SHALL be materialized into scene nodes
- **AND** the full retained history SHALL NOT be reconstructed into the scene graph

#### Scenario: resume preserves pending input

- **WHEN** the portal resumes a session that had non-terminal pending HUD input at disconnect
- **THEN** the preserved pending input and acknowledgement state SHALL remain available through the existing input-inbox contract
- **AND** the reconnect SHALL NOT silently drop those non-terminal items

#### Scenario: attach after grace starts a fresh portal

- **WHEN** a session attaches after the prior portal's lease grace already expired
- **THEN** the runtime SHALL start a fresh portal under a new lease
- **AND** it SHALL NOT silently revive the removed surface or present pre-death stale content as live

#### Scenario: stale-to-live transition respects redaction

- **WHEN** a portal resumes for a viewer whose policy redacts its transcript
- **THEN** every frame of the stale-to-live transition SHALL show the neutral redaction treatment in place of transcript content
- **AND** no transcript content SHALL flash during the transition

### Requirement: Viewer Turn Delivery Acknowledgement

The portal SHALL present an ambient per-turn delivery cue on the viewer's echoed turn reflecting the runtime's already-tracked input delivery state, so the viewer can see whether their reply reached the owning adapter without asking. The cue SHALL distinguish at least three presentation classes: in-flight (Pending or Deferred), delivered (Delivered or Handled), and failed (Rejected or Expired). The cue is local presentation of state the runtime already owns: rendering it MUST NOT introduce a new adapter round trip, and the viewer's read/seen state MUST NOT be disclosed back to the adapter as a side effect of rendering. The cue SHALL resolve its visual treatment from design tokens via the portal's component profile, SHALL remain subordinate to the Ambient Portal Attention Defaults requirement (a delivery transition is not an attention event), and a failed cue SHALL stay on the affected turn rather than escalating interruption class. Delivery cues are portal-surface presentation and SHALL redact together with the transcript under the Governance, Privacy, and Override Compliance requirement.

Source: §Viewer Reply Echo, `crates/tze_hud_projection/src/contract.rs` (InputDeliveryState: Pending, Delivered, Deferred, Handled, Rejected, Expired), `crates/tze_hud_projection/src/authority.rs` (delivery state tracked into ProjectedPortalState), `crates/tze_hud_projection/src/resident_grpc.rs` (DeliveryCueClass, delivery_cue_color_runs)
Scope: promotion

#### Scenario: pending then delivered tick

- **WHEN** a viewer submits an accepted reply and the adapter subsequently acknowledges taking delivery of it
- **THEN** the echoed viewer turn SHALL first present the in-flight cue and then transition to the delivered cue
- **AND** neither transition SHALL raise the portal's interruption class or unread count

#### Scenario: failed delivery is visible on the turn

- **WHEN** a submitted reply's delivery state becomes Rejected or Expired after it was echoed
- **THEN** the echoed viewer turn SHALL present the failed cue in place of the delivered cue
- **AND** the failure SHALL be presented ambiently on the turn itself rather than as a separate notification

#### Scenario: delivery cue requires no adapter round trip

- **WHEN** the runtime renders or updates a delivery cue
- **THEN** the cue SHALL be driven entirely from runtime-owned delivery state
- **AND** no additional adapter request SHALL be issued to render it
- **AND** no viewer read/seen signal SHALL be sent to the adapter as a side effect

### Requirement: Unread Divider and Ambient Unread Count

The portal SHALL render its already-tracked unread output count as (a) an in-transcript unread divider marking the boundary between seen and unseen transcript units when the viewer returns to a portal with unread content in the retained window, and (b) a compact ambient unread count in the portal's collapsed or peripheral presentation. Both presentations SHALL clear locally when the viewer views the tail (local-first; no adapter round trip to clear). This local clearing MAY be satisfied by the runtime's existing publish/drain cadence — draining a pending portal update already zeroes the unread state locally, with no adapter round trip — and does not require a dedicated, separate viewport-at-tail signal. The unread presentation MUST stay within the Ambient Portal Attention Defaults: a growing count SHALL update the rendered number or divider position but MUST NOT self-escalate interruption class, animate repeatedly, or behave like a notification. The divider SHALL count only agent-authored transcript units — echoed viewer turns are never unread per the Viewer Reply Echo requirement. When unread transcript units have been coalesced or have scrolled out of the bounded retained window, the count MAY exceed the units visibly below the divider, and the divider SHALL sit at the oldest retained unseen unit.

Source: §Ambient Portal Attention Defaults, §Viewer Reply Echo, §Bounded Transcript Viewport, `crates/tze_hud_projection/src/contract.rs` (unread_output_count), `crates/tze_hud_projection/src/authority.rs` (take_due_portal_update local drain-cadence clearing, ~line 460), `crates/tze_hud_runtime/src/portal_projection_driver.rs` (carry_drained_unread_count applied to visible_unread_output_count at all real drain call sites), `crates/tze_hud_projection/src/resident_grpc.rs` (unread_divider_boundary)
Scope: promotion

#### Scenario: divider marks the unread boundary

- **WHEN** a viewer focuses a portal whose retained window contains transcript units appended since the viewer last viewed the tail
- **THEN** an unread divider SHALL render at the boundary before the oldest retained unseen unit
- **AND** the divider and count SHALL clear locally once the viewer views the tail

#### Scenario: unread count stays ambient under backlog growth

- **WHEN** the unread count grows while the viewer is away or scrolled up
- **THEN** the rendered count SHALL update in place with ambient, token-styled treatment
- **AND** the portal SHALL NOT escalate interruption class, flash, or re-animate per increment

#### Scenario: viewer echo never counts as unread

- **WHEN** a viewer's own reply is echoed while the viewer is scrolled away from the tail
- **THEN** the unread divider and count SHALL NOT include the echoed viewer turn

### Requirement: Jump-to-Latest Affordance

When the viewer's scroll position leaves the transcript tail, the portal SHALL present a local-first jump-to-latest affordance that, when activated, returns the viewport to the tail and resumes tail-follow. While the viewer is scrolled away, incoming appends MUST NOT move the viewer's viewport (scroll position remains authoritative per the Transcript Interaction Contract); tail-follow resumes only through the affordance or the viewer scrolling back to the tail themselves. The affordance MAY carry the ambient unread count. Activation SHALL acknowledge locally under the existing interaction latency contract, and an adapter MUST NOT be able to trigger the jump on the viewer's behalf.

Source: §Transcript Interaction Contract, §Ambient Portal Attention Defaults, `crates/tze_hud_input/src/jump_to_latest.rs`, `crates/tze_hud_input/src/lib.rs` (ScrollState::reset_to_tail, FollowTailAnchor)
Scope: promotion

#### Scenario: affordance appears when scrolled away and returns to tail

- **WHEN** the viewer scrolls the transcript away from the tail
- **THEN** a jump-to-latest affordance SHALL become visible with ambient treatment
- **AND** activating it SHALL return the viewport to the tail locally and resume tail-follow

#### Scenario: appends do not yank the viewport while scrolled away

- **WHEN** new transcript units arrive while the viewer is scrolled away from the tail
- **THEN** the viewer's scroll position SHALL NOT change
- **AND** the affordance (and any unread count it carries) SHALL update ambiently instead

#### Scenario: adapter cannot force the jump

- **WHEN** an adapter attempts to reposition the viewport to the tail while the viewer is scrolled away
- **THEN** the runtime SHALL disregard the repositioning per the Transcript Interaction Contract
- **AND** only viewer action SHALL resume tail-follow

### Requirement: Ambient Per-Turn Timestamps

The portal SHALL be able to present per-turn arrival times sourced from the transcript unit's typed wall-clock arrival metadata (`appended_at_wall_us`, wall-clock domain). Timestamp presentation SHALL be ambient and subordinate: token-styled, visually secondary to turn content, and never the source of attention escalation. Timestamps are presentation of arrival metadata the runtime already retains; rendering them MUST NOT require adapter cooperation, and adapter-supplied content MUST NOT be able to forge the runtime-assigned arrival time. Clock-domain typing SHALL be preserved end to end: wall-clock arrival time SHALL NOT be conflated with media/display-clock presentation timing. Timestamp visibility and granularity (per-turn, grouped, or on-demand) are governed by the portal's component profile and design tokens rather than mandated at the pixel level.

Source: §Transport-Agnostic Stream Boundary (typed clock domains), `crates/tze_hud_projection/src/contract.rs` (appended_at_wall_us), `crates/tze_hud_projection/src/resident_grpc.rs` (format_wall_clock_arrival_hhmm)
Scope: promotion

#### Scenario: turns present runtime-assigned arrival time

- **WHEN** the portal presents timestamps for retained transcript units
- **THEN** each presented time SHALL derive from the runtime-assigned wall-clock arrival metadata of that unit
- **AND** adapter-supplied content SHALL NOT override the runtime-assigned arrival time

#### Scenario: timestamps stay visually subordinate

- **WHEN** timestamps are visible in the transcript
- **THEN** their treatment SHALL resolve from design tokens as secondary presentation
- **AND** timestamp changes or boundaries SHALL NOT raise the portal's interruption class

### Requirement: Agent Activity and Streaming Cue

While the owning adapter is actively appending to the transcript, the portal SHALL be able to present an ambient activity cue: a streaming cursor or equivalent live-writing treatment at the transcript tail, and optionally a compact typing-style indicator in the portal's header or collapsed presentation. The cue SHALL derive from observed append activity or the existing activity metadata — it MUST NOT require a new adapter-side "typing" protocol message. The cue SHALL remain strictly subordinate to the Ambient Portal Attention Defaults requirement: continuous streaming SHALL NOT re-trigger attention, and the cue SHALL quiesce promptly when appends stop. The activity cue is activity metadata under the Governance, Privacy, and Override Compliance requirement: it SHALL suppress together with transcript previews when the portal is redacted for the current viewer and SHALL freeze under safe-mode and freeze rules like other portal presentation.

Source: §Ambient Portal Attention Defaults (typing indicator remains ambient), §Governance, Privacy, and Override Compliance (collapsed portal preserves geometry while redacted), `crates/tze_hud_projection/src/resident_grpc.rs` (activity_cue_color_runs, streaming_cursor_color_runs)
Scope: promotion

#### Scenario: streaming cursor while agent writes

- **WHEN** transcript appends are actively streaming into the portal
- **THEN** the tail MAY present a streaming cursor or live-writing treatment with ambient token-styled presentation
- **AND** the cue SHALL quiesce promptly once appends stop

#### Scenario: activity cue is not a notification

- **WHEN** an adapter streams appends continuously over an extended period
- **THEN** the activity cue SHALL NOT re-escalate or repeat attention behavior
- **AND** the portal's interruption class SHALL remain at its policy-assigned level

#### Scenario: activity cue suppressed under redaction

- **WHEN** the current viewer's policy redacts the portal
- **THEN** the activity cue SHALL be suppressed along with transcript previews and activity details

### Requirement: First-Run Empty Portal Treatment

A portal surface whose retained transcript window contains no transcript units SHALL present a friendly, token-styled empty-state treatment identifying the portal and inviting interaction (for example, identity plus a short ready line), rather than a literal placeholder string such as `<empty projection stream>`. The empty-state treatment SHALL resolve from design tokens via the portal's component profile, SHALL respect redaction (identity and inviting copy suppress under the collapsed-redaction rules), and SHALL yield immediately to real content on the first transcript unit. When the portal is attached but not yet connected, the Connecting State Distinction requirement takes precedence over the plain empty treatment.

Source: `crates/tze_hud_projection/src/resident_grpc.rs` (empty_state_color_runs, replacing the `<empty projection stream>` literal), §Governance, Privacy, and Override Compliance
Scope: promotion

#### Scenario: empty portal renders a designed empty state

- **WHEN** a connected portal has an empty retained transcript window
- **THEN** the surface SHALL present the token-styled empty-state treatment instead of a literal placeholder string
- **AND** the first appended transcript unit SHALL replace the empty state immediately

#### Scenario: empty state redacts like identity metadata

- **WHEN** the current viewer is not permitted the portal's identity
- **THEN** the empty-state treatment SHALL suppress identity and inviting copy under the existing redaction treatment

### Requirement: Connecting State Distinction

The portal SHALL distinguish three connection presentations: connecting (attached but the owning session has not yet established its first connection), connected (live), and degraded/disconnected (previously connected, now dropped). The connecting treatment SHALL be visually distinct from the degraded treatment so a portal that is starting up does not read as failing. Transition into and out of the connecting presentation SHALL be ambient (not an attention event). The degraded/disconnected path — freeze at last coherent state, orphan lifecycle, grace expiry — remains governed by the existing Governance, Privacy, and Override Compliance requirement and is unchanged by this requirement.

Source: §Governance, Privacy, and Override Compliance (disconnected portal follows orphan path), `crates/tze_hud_projection/src/contract.rs` (has_ever_connected, connection_degraded), `crates/tze_hud_projection/src/resident_grpc.rs` (PORTAL_CONNECTING_LINE, connecting_color_runs)
Scope: promotion

#### Scenario: never-connected portal shows connecting, not degraded

- **WHEN** a portal is attached and its owning session has not yet established its first connection
- **THEN** the portal SHALL present the connecting treatment
- **AND** the degraded/disconnected treatment SHALL NOT be used for the never-connected case

#### Scenario: first connection transitions ambiently

- **WHEN** the owning session establishes its first connection
- **THEN** the portal SHALL transition from connecting to connected presentation without raising interruption class

#### Scenario: drop after connection uses degraded treatment

- **WHEN** a previously connected portal's session drops
- **THEN** the portal SHALL present the existing degraded/disconnected treatment and follow the existing orphan lifecycle

### Requirement: Conversational Turn Model and Per-Turn Role Attribution

The OUTPUT transcript of a text stream portal SHALL be presented as a sequence
of discrete conversational turns — one per retained `TranscriptUnit` — rather
than as an opaque single blob of text. Adjacent turns SHALL be visually
separated by a token-styled turn divider, and each turn SHALL carry token-styled
role attribution derived from its runtime-assigned output kind, so a viewer can
distinguish the assistant's own conversational turns from tool, status, error,
or other agent-side output within the same OUTPUT transcript.

Attribution SHALL resolve entirely from design tokens through the portal's
component-profile path and MUST NOT use any hardcoded compositor color: the
assistant's own turns present in the base transcript text color, and non-
assistant agent-side turns (tool, status, error, other) present in a distinct
attribution token color. Attribution is derived solely from the runtime-assigned
`output_kind` of each unit and MUST NOT be forgeable by adapter-supplied content;
it is presentation of metadata the runtime already retains and MUST NOT require
adapter cooperation. This requirement governs the agent-authored OUTPUT
transcript only and does not alter the two-history INPUT/OUTPUT split of the
Viewer Reply Echo requirement — viewer turns remain first-class, kind-distinct
units of the separately-bounded INPUT history and are not attributed by this
requirement.

Per-turn attribution is ambient, subordinate presentation consistent with the
Ambient Portal Attention Defaults requirement: it MUST NOT escalate the portal's
interruption class and MUST NOT count as unread. Attribution is content-adjacent
and SHALL obey the same redaction, safe-mode, freeze, and Bounded Transcript
Viewport rules as the transcript text it decorates — a restricted viewer whose
`visible_transcript` is emptied upstream sees no turns and therefore no
attribution, so attribution discloses nothing a redacted viewer could not
already see.

Turn presentation MUST preserve the existing coalescing invariant: per-render
turn presentation MUST NOT introduce a per-node `AddNode` fan-out that would
reclassify the hot transcript-republish batch as Transactional and defeat
StateStream latest-wins coalescing (Coherent Transcript Coalescing requirement).
Until a compositor vertical-flow layout capability exists to position per-turn
scene nodes, the turn model SHALL be carried within the single coalescible
transcript node (turn dividers in the transcript markdown, attribution as
token-resolved color-run spans over each attributed turn's text). Materializing
each turn as its own scene node via inline `NodeProto.children` is scene-mutation
schema work permitted only once portal promotion is gated (Promotion Scope
Boundary requirement) and additionally gated on that vertical-flow layout
capability; a future structural node split SHALL satisfy this same contract
without re-opening the attribution decision.

Source: RFC 0013 §4.3, hud-0yrix audit (`vd-no-conversational-turn-model` /
`chat-no-turn-structure-attribution`), §Viewer Reply Echo, §Ambient Portal
Attention Defaults, §Portal Component Profile Styling, §Promotion Scope Boundary,
turn separators from hud-nx7yq.4 (`crates/tze_hud_projection/src/resident_grpc.rs`
`visible_transcript_markdown_with`), `crates/tze_hud_projection/src/contract.rs`
(`OutputKind`), PR #1149 / hud-ga4md (`NodeProto.children` materialization)
Scope: v1

#### Scenario: adjacent turns are separated and role-attributed

- **WHEN** an expanded portal renders a retained OUTPUT transcript window with more than one turn
- **THEN** adjacent turns SHALL be separated by a token-styled turn divider
- **AND** each non-assistant agent-side turn (tool, status, error, other) SHALL be presented in the distinct attribution token color rather than the base assistant transcript color
- **AND** assistant-authored turns SHALL present in the base transcript text color

#### Scenario: attribution derives from runtime output kind, not adapter content

- **WHEN** the portal attributes a turn's role
- **THEN** the role SHALL derive from that unit's runtime-assigned `output_kind`
- **AND** adapter-supplied transcript text SHALL NOT be able to change or forge the attributed role

#### Scenario: attribution resolves from tokens, not hardcoded color

- **WHEN** the portal presents per-turn role attribution
- **THEN** every attribution color SHALL resolve from the active design tokens through the component-profile path
- **AND** no attribution color SHALL come from a hardcoded compositor value

#### Scenario: attribution stays ambient and redaction-safe

- **WHEN** per-turn attribution is present in the transcript
- **THEN** it SHALL NOT raise the portal's interruption class and SHALL NOT count as unread
- **AND** when the current viewer's policy redacts the portal, the emptied transcript SHALL present no turns and therefore no attribution

#### Scenario: turn attribution does not break coalescing

- **WHEN** an expanded portal republishes its transcript on each append under state-stream backpressure
- **THEN** the turn model SHALL be carried on the single coalescible transcript node without a per-turn `AddNode` fan-out
- **AND** the republish batch SHALL remain a coalescible StateStream update rather than being reclassified Transactional

