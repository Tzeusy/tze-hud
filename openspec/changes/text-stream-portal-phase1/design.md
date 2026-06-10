# Text Stream Portal Phase-1 Design

## 1. Scope Decision

Phase 1 is a depth investment in one flow, not a breadth expansion. Everything in this change is expressible on the Phase-0 raw-tile pilot until the promotion gate passes; nothing here requires a new node type, a new transport, or chrome involvement to begin implementation. The promotion gate is sequenced last on purpose: the evidence that justifies a first-class surface is produced by shipping the Phase-1 behaviors on raw tiles first.

In scope:

- markdown rendering subset with token-driven styling
- normative overflow/ellipsis contract
- runtime-owned, local-first composer draft editing (bounded)
- sustained streaming cadence under engineering-bar budgets, with publish-to-present-vs-RTT overhead evidence (amendment, §8)
- viewer window management: pointer resize, focus-scoped resize hotkeys, scroll-position indicators (amendment, §8)
- portal component profile and collapsed/expanded transitions
- RFC 0013 §7.2 promotion evidence gate — including the agent-ergonomics criterion (amendment, §8) — and promotion scope boundary

Out of scope (unchanged non-goals from RFC 0013 §1.2/§4.4): terminal emulation, PTY hosting, ANSI cursor addressing, full scene-graph transcript history, chrome-layer portal UI, dedicated portal transport, runtime process-lifecycle ownership, IME composition, markdown tables/images/HTML, link navigation.

## 2. Markdown Subset and Parser Placement

**Decision: parse a defined CommonMark subset in the runtime, at content-commit time, never on the render path.**

The scene contract (RFC 0001 §TextMarkdownNode; scene-graph spec, Node Types requirement) already names `TextMarkdownNode` content as CommonMark. Phase 0 approximates this with `strip_markdown_v1`. Phase 1 defines the subset normatively: ATX headings 1–6, strong/emphasis, inline code, fenced and indented code blocks, ordered/unordered lists (bounded nesting), and links rendered as styled non-navigable text. Tables, images, raw HTML, blockquotes, footnotes, strikethrough, task lists, and autolinks are excluded — excluded constructs render as literal source text rather than being silently dropped, so transcript content is never lost.

Placement options considered:

1. **Adapter-side style runs** (extend the `color_runs` pattern to general style runs). Rejected as the primary mechanism: every adapter family would re-implement the same parser, the wire payload grows on the hot state-stream path, and divergent adapter parsers would fracture the "one scene model" rule. `color_runs` remains valid for ANSI-derived coloring per the Phase-0 raw-tile requirement.
2. **Per-frame parsing in the compositor.** Rejected outright: it places content-proportional CPU work inside the frame loop, violating the stage budgets (Stage 5 Layout Resolve < 1 ms, Stage 6 Render Encode < 4 ms; engineering-bar §2 / RFC 0003 §5.1).
3. **Parse-on-commit with cached styled runs.** Chosen. The session-side mutation decode path (before mutations enter the per-frame intake queue) parses changed `TextMarkdownNode` content into an immutable styled-run representation, cached by content identity. The frame pipeline consumes cached runs; a node whose content did not change costs zero parse work per frame. Parse cost is bounded by the existing 65535-byte content ceiling.

This honors the core rules: the LLM never sits in the frame loop, and neither does its markdown. The parser runs where arrival-time work belongs; presentation timing remains the runtime's.

Styling comes from design tokens only (heading scale, code family/background, link color/underline), resolved at startup into `RenderingPolicy`-style values per the component-shape-language token rules. No hardcoded styling enters the compositor — this is the "visual identity is modular" rule applied to text fidelity.

Message-class note: markdown appends remain **state-stream** traffic (RFC 0013 §5). Parsing on commit does not change delivery semantics; coalescing still operates on logical transcript units, and the Coherent Transcript Coalescing requirement is unaffected.

## 3. Overflow and Ellipsis Algorithm

**Decision: measured word-boundary truncation with the ellipsis glyph inside the measurement, and whole-line window stability under streaming append.**

Phase 0 approximates `TextOverflow::Ellipsis` by truncating the visible line count and appending `…` (compositor `text.rs` module note), which can clip glyphs horizontally and lets the truncation point oscillate as content streams in. The Phase-1 contract:

- Truncation occurs at the last word boundary whose shaped width, **plus the shaped ellipsis glyph in the same style run**, fits the content box. If no word boundary fits, truncation falls back to the last grapheme-cluster boundary that fits — never mid-glyph.
- Vertically, the last line is either fully visible or not rendered. No partially clipped glyph rows.
- Under streaming append with follow-tail active, the window advances by whole layout lines; while the viewer is scrolled away from the tail, the visible prefix does not reflow because of appends beyond the viewport (this extends the Phase-0 scroll-authority rule in the Transcript Interaction Contract).

Trade-off: word-boundary measurement requires shaping the candidate tail before truncation, which costs more than line counting. The cost lands in layout resolve and must fit Stage 5 < 1 ms (engineering-bar §2); the mitigation is that shaping results are cached with the styled runs from §2 and recomputed only when content or geometry changes, not per frame. We considered binary-searching raw byte offsets without shaping (cheaper, but wrong for proportional fonts and emphasis-styled runs) and rejected it: "no clipped glyphs" is the bar, and approximations are what Phase 0 already has.

## 4. Composer Editing State Ownership

**Decision: runtime-owned bounded draft buffer with local echo; adapter observes draft state as coalesced state-stream events; submission stays transactional. RFC 0013 §8 open question 1 is answered: the product needs an in-surface editable draft model, bounded.**

Options:

1. **Adapter-owned echo (Phase-0 status quo).** The adapter re-publishes the composer `TextMarkdownNode` and caret node per keystroke (RFC 0013, 2026-04-27 implementation note). Editing feel is capped at a full event→adapter→mutation→commit→present round trip. The 2026-04-28 evidence shows this works but cannot meet the local-feedback bar: typing latency is adapter latency.
2. **Runtime-owned draft with local echo.** The runtime maintains a bounded plain-text draft buffer per focused composer region and renders text, caret, and selection locally, inside the local-feedback path (input_to_local_ack p99 < 4 ms; Windows locked lane ≤ 2 ms — engineering-bar §2). The adapter receives coalesced draft-change notifications as **state-stream** traffic and the final submission as **transactional** traffic. Chosen.
3. **Hybrid prediction/reconciliation** (runtime predicts, adapter remains source of truth, divergences reconcile). Rejected: reconciliation flicker and dual-ownership complexity for no governance benefit.

Option 2 is the only one consistent with "local feedback first": keystroke acknowledgement must happen locally and instantly, with remote semantics following. The risk is the one RFC 0013 §4.3 names — drifting into a general-purpose inline editor. The delta spec bounds the primitive hard:

- plain text only; the draft is never markdown-rendered while being edited,
- editing operations limited to caret movement, selection, backspace/delete (character and word-wise), and paste,
- paste is size-capped and truncated at a UTF-8 character boundary; the hard ceiling is the existing 65535-byte text-node limit, with a configurable lower cap,
- no IME composition (v1-reserved in the input-model spec; unchanged),
- no undo/redo contract, no rich text, no multi-caret,
- editing keystrokes are never interpreted as terminal input (the existing Cooperative Projection Input Mapping requirement already forbids this; Phase 1 keeps it).

Governance: the draft is viewer-authored input destined for the owning adapter, so delivering draft state to that adapter leaks nothing new. Draft content obeys the same redaction/safe-mode rules as the portal surface — safe mode captures input in chrome and the draft surface suspends with the rest of the portal.

Message-class mapping (per the four-class taxonomy): local echo is not a message at all (it never leaves the runtime before acknowledgement); draft-change notifications are state-stream, coalescible to the latest draft snapshot; submission and cancel are transactional.

## 5. Sustained Streaming Cadence

**Decision: define normative workloads, hold the existing locked budgets under them, and state fairness as a liveness property rather than inventing an arrival-to-present number.**

Workload definitions (test parameters, not budgets):

- *Sustained stream*: appends totaling ≥ 200 Unicode scalar values per second, delivered in ≥ 10 increments per second, sustained ≥ 60 s — representative of a fast LLM token stream.
- *Burst*: ≥ 4096 bytes of transcript arriving within 250 ms — representative of a tool-output flush.

Pass criteria come from `about/craft-and-care/engineering-bar.md` §2, not from new numbers:

- frame time under sustained portal streaming holds the Windows locked `high_mutation` budgets (p99 ≤ 8.3 ms, p99.9 ≤ 16.6 ms; general bar < 16.6 ms total frame time),
- concurrent viewer input during streaming holds input_to_local_ack (p99 < 4 ms general; ≤ 2 ms Windows lane) and input_to_next_present (< 33 ms general; ≤ 16.6 ms Windows lane),
- per-stage budgets are not displaced: Mutation Intake < 1 ms, Scene Commit < 1 ms, Layout Resolve < 1 ms,
- portal event traffic (draft notifications plus stream status) stays within the 1000 events/second aggregate ceiling,
- a 60-minute streaming soak respects the ≤ 5 MiB memory-drift budget.

We deliberately did **not** define an arrival-to-presentation latency budget for transcript appends. Appends are state-stream traffic; coalescing policy makes arrival-to-present workload-dependent by design, and "arrival time ≠ presentation time" is doctrine. Inventing such a number would quietly convert state-stream semantics into clocked-media semantics. Smoothness is instead governed by frame budgets, and starvation by fairness:

- coalescing is work-conserving: when render capacity exists and committed units are pending, a newer coherent window snapshot is presented,
- with multiple portals streaming concurrently, coalescing must not starve any portal; under equal sustained rates, presented-window progress across portals must not diverge unboundedly,
- the retained-window coherence rule (Phase-0 Coherent Transcript Coalescing) holds under burst: bursts may collapse to a newer window snapshot but never to "latest line only."

## 6. Component Profile Structure

**Decision: portal visual identity resolves from design tokens through a swappable profile treatment, pre-promotion via token-resolved adapter styling, post-promotion via a `text-portal` component type contract in component-shape-language.**

Phase 0's exemplar hardcodes raw-tile colors in the adapter script. That is legal for a pilot (the compositor hardcodes nothing) but fails the "visual identity is modular" bar for an exemplar flow. Phase 1 structures the portal surface as named visual parts — frame, header, composer, transcript body, divider, collapsed card — each styled exclusively from design-token values, with profile-scoped overrides able to reskin the portal without touching adapter logic or runtime behavior.

Sequencing trade-off: the component-type registry currently defines six v1 types, and adding a `text-portal` component type is a component-shape-language change. Doing that before promotion would specify a first-class component contract for a surface that is still raw-tile assembly — exactly the premature promotion RFC 0013 §7 guards against. So:

- **Pre-promotion:** the exemplar adapter must source every visual value it publishes from the runtime's resolved token set (exposed through existing configuration/introspection paths), so a profile/token change reskins the portal end-to-end. No literal colors/sizes in adapter publish calls.
- **At promotion:** a follow-up delta to `component-shape-language` adds the `text-portal` component type contract and any new canonical token keys; the portal surface then consumes `RenderingPolicy` fields like every other component.

Collapsed/expanded transitions reuse the existing zone-transition opacity mechanics (v1 ships opacity fade on publish/clear) with token-derived durations; no new animation system. Transitions must not violate the redaction rule: a collapsed-to-expanded transition under a restricted viewer reveals the redaction placeholder, never a flash of transcript.

## 7. Promotion Gate Mechanics

**Decision: the gate is a refreshed-evidence checklist mapped one-to-one onto RFC 0013 §7.2, executed as new phases of the existing live exemplar, across two adapter families.**

Evidence plan:

- extend `text_stream_portal_exemplar.py` with `markdown`, `overflow`, `composer-edit`, `cadence`, and `profile-swap` phases, alongside the existing `baseline`/`scroll`/`streaming`/`rapid`/`diagnostic-input` phases,
- run the extended exemplar live on the reference Windows host for (a) the exemplar script adapter and (b) the cooperative projection adapter — two genuinely different adapter families, satisfying the "same pattern recurs across multiple adapters" criterion,
- record artifacts under `docs/evidence/text-stream-portals/` with the reference hardware tag required by engineering-bar §2,
- record raw-tile complexity observations (tile counts, mutation batch shapes, workaround inventory) as the "repeated complexity" evidence the RFC asks for.

What promotion changes: it permits a first-class portal surface or node type (RFC 0013 §7.2 wording) and the `text-portal` component type contract. What promotion does **not** change: every non-goal stands — no terminal semantics, no chrome-layer portal UI, no second transport stream, no runtime process ownership, no scene-graph transcript history. A first-class surface that needs new scene-mutation protobuf messages is within the promotion approval; a portal-specific transport stream is not.

If the gate fails — budgets miss, governance behavior regresses, or raw-tile expression turns out not to be the bottleneck — the raw-tile pilot remains the correct scope per RFC 0013 §7.2, and the Phase-1 behavioral requirements still stand on raw tiles.

## 8. Amendment (2026-06-10): Window Management, Overhead Evidence, Agent Ergonomics

Owner-directed amendment after acceptance, before implementation start. Three additions; no prior decision is reversed.

**Window management is viewer-sovereign, local-first interaction — and hotkeys are focus-scoped, not global.** Move existed (header drag); Phase 1 adds resize through frame affordances and keyboard steps (Ctrl+`+`/`-`, with Ctrl+`=` as the unshifted plus). The shortcuts bind only while the portal holds keyboard focus: the runtime owns global input routing, chrome- and shell-reserved shortcuts always win, and a content-layer surface claiming global keymaps would violate "screen is sovereign." Gesture geometry is local-first like every other interaction; the adapter sees coalescible state-stream geometry snapshots and cannot fight an active gesture. Min bounds come from tokens (legibility), max bounds from lease and scene budgets (governance). Resize stresses the overflow contract on purpose: every intermediate geometry must satisfy "no clipped glyphs," which makes the resize path a continuous test of the ellipsis algorithm rather than a separate layout mode. Scroll-position indicators are geometry-only so they survive redaction without leaking content.

**Overhead is a processing bound, not a delivery deadline.** §5's refusal to define an arrival-to-present deadline stands: coalescing may supersede any individual append. What the amendment adds is the complementary bound the owner asked for — when an append *is* presented, arrival-to-next-present sits within the existing `high_mutation` input-to-next-present row (p99 ≤ 16.6 ms Windows lane). No new number is invented; the row already exists in the engineering bar. The real addition is evidential: the live cadence phase must measure a transport RTT baseline (session-stream echo) and per-append publish-to-present timestamps, and report runtime-added overhead explicitly. "End-to-end latency is RTT plus about one frame" becomes a measured artifact instead of an inference.

**Agent ergonomics joins the promotion gate.** The gate already measured raw-tile complexity from the implementer's seat (tile counts, batch shapes, workarounds). The amendment adds the agent's seat: an LLM session must drive the full portal lifecycle — attach, stream output, poll and acknowledge input, detach — purely through the vendored skill surface (the cooperative projection contract), with zero scene-graph ceremony in its context. This is the operational definition of the exemplar being "easy for a model to drive": if the demonstration needs hand-written tile assembly or mutation authoring in the LLM's context, the gate fails and that friction is exactly the promotion evidence RFC 0013 §7.2 asks for. Ceremony metrics (operation count, glue outside the skill) are recorded with the complexity observations.

## 9. Risks

- **Editor scope creep.** The draft buffer is the most likely place for "just one more editing feature" drift. The bounds in §4 are normative; anything beyond them (undo, rich text, IME) requires a new change.
- **Shaping cost in layout resolve.** Word-boundary ellipsis with styled runs could pressure the Stage 5 < 1 ms budget on large transcripts. Mitigation: cache shaped runs with content identity; re-shape only on content/geometry change. If the budget still misses, the fallback is restricting measured truncation to the viewport-adjacent window, not reverting to clipped glyphs.
- **Token coverage gaps.** The current canonical token set may lack keys the portal needs (code background, link underline). Pre-promotion these ride profile-scoped overrides; promotion-time canonical keys land in the component-shape-language delta.
- **Cadence evidence depends on rig availability.** The reference Windows host has a history of connectivity loss mid-run (2026-04-28 blocker, 2026-05-11 reachability gaps). The tasks sequence headless/integration verification before live phases so a rig outage delays evidence, not implementation.
