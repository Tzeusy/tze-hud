## MODIFIED Requirements

### Requirement: Transcript Interaction Contract

Expanded text stream portals SHALL support scrollable transcript viewing, focusable interaction affordances, bounded composer draft editing, and bounded reply submission under the existing local-feedback contract. Portal interaction MUST reuse runtime-owned focus, command-input, and local visual acknowledgement rules. Focusable interaction affordances — the composer and portal controls — SHALL be reachable without a pointer: the runtime SHALL provide a keyboard focus-traversal path (a focus-advance and focus-retreat key plus a token-defined focus chord) routed through the runtime-owned focus manager, so a portal is fully operable for input on pointer-less surfaces such as the Mobile Presence Node profile. Keyboard focus traversal SHALL respect the same scoping as other portal shortcuts: chrome- and shell-reserved keys take precedence and safe-mode input capture overrides it. User scroll input MUST remain authoritative over any adapter-driven attempt to reposition the transcript viewport. Composer draft editing — caret movement, selection, deletion, and capped paste — SHALL receive local-first visual feedback from the runtime-owned draft state without requiring an adapter round trip, while submission remains a bounded transactional action.

Source: RFC 0013 §4.3, RFC 0004 (input model, focus traversal), CLAUDE.md core rules "one scene model, two profiles" and "local feedback first"

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

#### Scenario: composer is focusable without a pointer

- **WHEN** the viewer advances focus to the composer using the keyboard focus-traversal path on a surface with no pointer device
- **THEN** the composer SHALL acquire focus and accept draft-editing keystrokes with local-first feedback
- **AND** the traversal SHALL respect chrome/shell focus precedence and safe-mode capture rather than stealing shell-reserved focus

#### Scenario: adapter cannot reposition viewport or draft against the user

- **WHEN** an adapter attempts to reposition the transcript viewport or alter the draft while the viewer is actively scrolling or editing
- **THEN** user scroll input SHALL remain authoritative over the viewport
- **AND** the locally rendered draft state SHALL NOT be overwritten mid-edit by adapter traffic

### Requirement: Local-First Composer Draft Editing

Expanded portal composers SHALL support bounded draft editing with runtime-owned draft state and local-first echo. The runtime SHALL maintain a plain-text draft buffer per focused composer region and SHALL render draft text, caret, and selection locally within the local-feedback latency contract (input to local ack p99 < 4 ms per `about/craft-and-care/engineering-bar.md` §2; ≤ 2 ms p99 under the Windows locked lane). When the draft text exceeds the visible composer width, the runtime SHALL maintain a horizontal scroll offset that keeps the active caret — and the moving edge of an active selection — within the visible composer region, so the caret never leaves the viewport while editing; this offset is local presentation state subject to the same redaction, safe-mode, and focus rules as the rest of the draft. Supported editing operations are: caret movement (character, word, line-start/end), selection (keyboard and pointer), backspace and delete (character and word-wise), and paste. Draft-change notifications to the owning adapter SHALL be state-stream traffic, coalescible to the latest draft snapshot; draft submission and cancel SHALL remain transactional. Draft editing MUST NOT include IME composition (which remains v1-reserved under the input-model specification), undo/redo, rich text, multi-caret editing, or any interpretation of editing keystrokes as terminal input. Draft content and caret presentation are subject to the same redaction, safe-mode, and focus rules as the rest of the portal surface.

Source: RFC 0013 §4.3 and §8 (open question 1), RFC 0004 focus semantics, `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rule "local feedback first"

#### Scenario: keystroke echoes locally before adapter acknowledgement

- **WHEN** the viewer types a character into a focused portal composer
- **THEN** the character, updated caret position, and any selection change SHALL render locally within the input-to-local-ack budget
- **AND** the visible echo SHALL NOT depend on an adapter round trip

#### Scenario: word-wise delete operates on the local draft

- **WHEN** the viewer performs a word-wise backspace in a non-empty draft
- **THEN** the runtime SHALL remove the preceding word from the local draft buffer and update the rendering locally
- **AND** the owning adapter SHALL observe the change only through a coalescible draft-state notification

#### Scenario: caret stays visible past the composer width

- **WHEN** the viewer types or moves the caret such that the draft extends beyond the visible composer width
- **THEN** the runtime SHALL shift the composer's horizontal scroll offset so the active caret remains within the visible composer region
- **AND** moving the caret back toward the start of the draft SHALL reveal the earlier text, keeping the caret visible throughout

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

## ADDED Requirements

### Requirement: Portal Keyboard Focus Recovery

Keyboard focus traversal within a portal MUST NOT strand the viewer: at all times either the composer holds focus (typing works), or the focused portal control presents a visible, token-driven focus ring, and a bounded number of focus-advance steps returns focus to the composer. A focusable portal control SHALL be activatable from the keyboard: an activation key (Enter or Space) on a focused control SHALL trigger the same interaction as pointer activation of that control, preserving the control's interaction identity. Typing SHALL route to the conversation like a chat application: when a printable character arrives while a non-composer portal control holds focus, the runtime SHALL refocus that portal's composer and apply the keystroke to the draft rather than silently discarding it; a dedicated recovery key (Escape) on any focused portal control SHALL likewise refocus the composer. Pointer click-to-focus on the composer SHALL restore composer focus from any control stop. The focus ring is geometry-only presentation resolved from design tokens; it SHALL render on transparent overlay surfaces the same as on opaque surfaces, SHALL remain present under redaction without revealing content, and focus transitions SHALL NOT raise interruption class.

Source: `openspec/specs/input-model/spec.md` (Focus Cycling, Focus Ring), RFC 0004 focus semantics, live defect hud-2v8br (2026-07-03 tzehouse-windows: Tab stranded typing with no visible ring; ring mechanism had no render consumer), PR #988

#### Scenario: focused control shows a visible ring on the overlay

- **WHEN** keyboard traversal moves focus to a non-composer portal control on a transparent overlay surface
- **THEN** the runtime SHALL render a token-driven focus ring around the focused control
- **AND** the ring SHALL be visible against the overlay without revealing redacted content

#### Scenario: typing on a control refocuses the composer

- **WHEN** a printable character arrives while a non-composer portal control holds focus
- **THEN** the runtime SHALL refocus the portal's composer and apply the keystroke to the draft with local-first echo
- **AND** the keystroke SHALL NOT be silently discarded

#### Scenario: activation keys operate the focused control

- **WHEN** the viewer presses Enter or Space while a portal control holds keyboard focus
- **THEN** the runtime SHALL activate the control with the same semantics as pointer activation, preserving its interaction identity

#### Scenario: escape and click both recover composer focus

- **WHEN** the viewer presses Escape while any portal control holds focus, or clicks the composer region
- **THEN** composer focus SHALL be restored and subsequent typing SHALL edit the draft
- **AND** repeated focus-advance steps SHALL also return to the composer within one full traversal cycle
