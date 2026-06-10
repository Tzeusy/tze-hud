## ADDED Requirements

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

Source: RFC 0013 §3.4 and §4.2, RFC 0001 §TextMarkdownNode overflow semantics

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

### Requirement: Sustained Streaming Cadence

Portal output presentation SHALL remain within the engineering-bar budgets under representative streaming workloads. For a sustained stream (appends totaling at least 200 Unicode scalar values per second, in at least 10 increments per second, for at least 60 seconds) and for bursts (at least 4096 bytes arriving within 250 ms), the runtime SHALL hold: total frame time within the frame budget (< 16.6 ms p99 general; `high_mutation` p99 ≤ 8.3 ms and p99.9 ≤ 16.6 ms under the Windows locked lane), concurrent input responsiveness within input-to-local-ack and input-to-next-present budgets, per-stage budgets per `about/craft-and-care/engineering-bar.md` §2, and aggregate portal event traffic within the 1000 events/second ceiling. Coalescing SHALL be work-conserving: when committed transcript units are pending and render capacity exists, a newer coherent window snapshot SHALL be presented. With multiple portals streaming concurrently, coalescing MUST NOT starve any portal: under equal sustained rates, presented-window progress across portals MUST NOT diverge unboundedly. Arrival timestamps remain advisory; this requirement defines no arrival-to-presentation deadline, because transcript appends are state-stream traffic whose presentation timing belongs to the runtime. A 60-minute sustained-streaming soak SHALL stay within the ≤ 5 MiB memory-drift budget.

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

- **WHEN** two portals stream at equal sustained rates under coalescing pressure
- **THEN** each portal's presented window SHALL continue to advance
- **AND** neither portal's presented progress SHALL fall unboundedly behind the other's

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

Promotion from the raw-tile pilot to a first-class portal surface SHALL occur only after a refreshed live evidence package satisfies all RFC 0013 §7.2 criteria. The package SHALL consist of live exemplar runs on the reference Windows host covering markdown fidelity, overflow correctness, composer draft editing, sustained streaming cadence, and profile-swap styling, executed against at least two distinct adapter families (the exemplar script adapter and the cooperative projection adapter), with artifacts recorded under `docs/evidence/text-stream-portals/` carrying the engineering-bar reference hardware tag. The package SHALL also record raw-tile complexity observations (tile counts, mutation batch shapes, workarounds) as the recurring-complexity evidence, and SHALL confirm that governance behavior (redaction, safe mode, freeze, orphan path) remained correct during the Phase-1 runs. A failed or incomplete gate SHALL leave the raw-tile pilot as the authoritative scope; Phase-1 behavioral requirements remain in force on raw tiles regardless of gate outcome.

Source: RFC 0013 §7.2, `about/craft-and-care/engineering-bar.md` §2 (reference hardware tag), `.claude/skills/user-test/SKILL.md` (live exemplar)

#### Scenario: gate passes with complete evidence

- **WHEN** the refreshed live runs cover all five Phase-1 axes across both adapter families with budget-passing, reference-tagged artifacts and governance confirmation
- **THEN** promotion to a first-class portal surface MAY proceed under this change's approval
- **AND** the evidence package SHALL be linked from the promotion implementation work

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

## MODIFIED Requirements

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
