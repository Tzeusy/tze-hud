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

Text stream portals SHALL obey the same lease, privacy, redaction, dismissal, freeze, and safe-mode rules as any other governed surface. Portal identity, transcript content, and activity metadata MUST NOT bypass viewer-class filtering or shell overrides because they are text. Collapsed portal cards MUST NOT be treated as automatically safe metadata.

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
- **THEN** the lease SHALL transition through the normal orphan lifecycle, the visible portal SHALL freeze at its last coherent state or runtime placeholder policy, and grace expiry SHALL remove the governed surface under the existing lease rules

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

When a viewer submits a reply through a text stream portal composer and the submission is accepted, the runtime SHALL echo the submitted text into the portal's retained transcript as a viewer-authored turn, so the two-way conversation is visible on the surface rather than the viewer's own words disappearing into the adapter inbox. The viewer turn SHALL be authored by the runtime at submit time and SHALL be distinguishable from agent-authored transcript units by a dedicated viewer turn kind; an adapter MUST NOT author, publish, or otherwise forge a viewer turn through the output-publication contract, and a publish that attempts to use the viewer turn kind SHALL be rejected. The echoed viewer turn SHALL carry the submission's content classification and SHALL obey the same redaction, safe-mode, freeze, and Bounded Transcript Viewport rules as agent-authored transcript content — it is not automatically safe because it is the viewer's own text. The viewer echo is local-first presentation, not a new attention event: it SHALL NOT increment the portal's unread-output count and SHALL NOT escalate interruption class, because the viewer has by definition already seen their own message, consistent with the Ambient Portal Attention Defaults requirement. The echo is a presentation of an already-accepted submission and SHALL NOT alter the existing submission contract: the submitted text SHALL still be delivered transactionally to the adapter's semantic input mechanism per the Cooperative Projection Input Mapping requirement, and a submission that is rejected SHALL NOT be echoed. Visual differentiation of viewer versus agent turns (alignment, role accent, attribution affordance) is governed by the portal's component-profile and design tokens and is not mandated at the pixel level by this requirement; the requirement establishes that viewer turns are first-class, kind-distinct units within the bounded retained transcript window.

Source: RFC 0013 §3.3 and §4.3, `about/heart-and-soul/vision.md` ("a persistent on-screen portal where a person can converse"), `about/heart-and-soul/presence.md` (Interaction — local-first), `crates/tze_hud_projection/src/contract.rs` (OutputKind::Viewer), `crates/tze_hud_projection/src/authority.rs` (append_viewer_echo on submit_portal_input), `crates/tze_hud_runtime/src/portal_projection_driver.rs` (parse_output_kind rejects adapter-supplied viewer)
Scope: v1-mandatory

#### Scenario: accepted reply appears as a viewer turn

- **WHEN** a viewer submits a reply that the portal accepts
- **THEN** the submitted text SHALL appear in the retained visible transcript as a viewer-authored, kind-distinct turn
- **AND** the submission SHALL still be delivered transactionally to the adapter's semantic input mechanism per the existing Cooperative Projection Input Mapping requirement

#### Scenario: viewer echo does not count as unread or escalate attention

- **WHEN** a viewer reply is echoed into the transcript
- **THEN** the portal's unread-output count SHALL NOT increase
- **AND** the echo SHALL NOT raise the portal's interruption class beyond the ambient default

#### Scenario: adapter cannot forge a viewer turn

- **WHEN** an adapter publishes transcript output using the viewer turn kind
- **THEN** the runtime SHALL reject the publish
- **AND** only the runtime's submit path SHALL author viewer turns

#### Scenario: viewer echo redacts like transcript content

- **WHEN** the current viewer's policy redacts the portal's transcript
- **THEN** the echoed viewer turn SHALL be redacted under the same policy as agent-authored content
- **AND** it SHALL NOT bypass viewer-class filtering because it is the viewer's own submitted text

#### Scenario: rejected submission is not echoed

- **WHEN** a viewer submission is rejected (for example because the HUD is unavailable or the input queue is full)
- **THEN** no viewer turn SHALL be appended to the transcript
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

