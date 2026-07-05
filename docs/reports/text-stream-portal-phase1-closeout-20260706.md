# Text Stream Portal Phase-1 — Closeout Reconciliation

Date: 2026-07-06
Issue: `hud-bm1tn` (child C8 of `hud-5jbra`, task 8.1)
OpenSpec change: `text-stream-portal-phase1`
Promotion gate: **PASS** — owner PROMOTE decision (`hud-qfyfg`, 2026-07-05,
`docs/reports/hud-qfyfg-text-stream-portal-phase1-promotion-gate-20260705.md`)

## Purpose

Reconcile the implementation against **every** ADDED and MODIFIED requirement in
the `text-stream-portal-phase1` delta spec before archiving the change (tasks.md
§8.1). Each requirement is cited to concrete code (`file:symbol`). Deferred /
promotion-era items are listed with rationale rather than "fixed" — they are
tracked as follow-up beads, not Phase-1 gaps.

Both changes validate strict:
`openspec validate text-stream-portal-phase1 --strict` → valid;
`openspec validate portal-chat-grade-affordances --strict` → valid.

The Phase-1 delta requirements are **already synced** into
`openspec/specs/text-stream-portals/spec.md` (lines 279–570 for the ten ADDED
requirements; the two MODIFIED requirements at lines 85 and 111 already carry the
Phase-1 wording). Archiving therefore only relocates the change directory and
confirms the spec is in sync.

## ADDED requirements — reconciliation

| # | Requirement | Status | Implementation (file:symbol) |
|---|---|---|---|
| 1 | Phase-1 Markdown Rendering Subset | **SATISFIED** | `crates/tze_hud_compositor/src/markdown.rs::parse_markdown_subset` (ATX headings, strong/emphasis, inline code, fenced/indented code, ordered/unordered nested lists, links-as-styled-text); excluded constructs fall through to literal text; `StyledSpan` carries token-resolved styling from `MarkdownTokens`. Token wiring: `hud-f8jb0` (heading_scale, code_background, bold_weight), `hud-1uh1l` (part-token inventory). |
| 2 | Markdown Parsing Outside the Frame Loop | **SATISFIED** | `crates/tze_hud_compositor/src/markdown.rs::MarkdownPrimer` (commit-time `prime()`, `arc_swap::ArcSwap<MarkdownCache>` atomic swap, `AtomicU64` published_version). Render path calls `renderer/mod.rs::markdown_cache()` → `MarkdownCache::load()` (zero per-frame parse for unchanged content, BLAKE3 content-identity key). |
| 3 | Transcript Overflow and Ellipsis Contract | **SATISFIED** | `crates/tze_hud_compositor/src/overflow.rs::truncate_for_ellipsis` / `truncate_line_to_ellipsis` (word-boundary + shaped-ellipsis-glyph measurement), grapheme-cluster fallback (test `single_long_word_truncated_at_grapheme_boundary`); `truncate_tail_anchored` + follow-tail whole-line advancement; property tests `proptest_tail_anchored_*`. |
| 4 | Local-First Composer Draft Editing | **SATISFIED** | `crates/tze_hud_input/src/composer_draft.rs::ComposerDraft` / `ComposerDraftManager` (runtime-owned plain-text buffer, caret/selection/word-wise delete, coalescible `DraftStateNotification`, transactional `DraftSubmission`/`DraftCancel`); safe-mode `suspended` gate (`EditOutcome::Suspended`). Local echo render: `hud-r3ax6`, `hud-2zyt9`. Wiring: `windowed/input_dispatch.rs`, `windowed/portal.rs`. |
| 5 | Composer Draft Bounds and Paste Caps | **SATISFIED** | `crates/tze_hud_input/src/composer_draft.rs` — `cap ≤ MAX_DRAFT_BYTES` (65535), paste/insert truncated at grapheme-cluster boundary, `EditOutcome::AtCapacity`, no over-cap content leaves the runtime. Clipboard injection: `windowed/lifecycle.rs::drain_paste_inject` + `inject_composer_paste` MCP tool (`hud-k1uun`). |
| 6 | Portal Window Management | **SATISFIED** | `crates/tze_hud_runtime/src/windowed/portal.rs::apply_portal_resize_pointer_event`, `apply_portal_resize_hotkey`, `commit_portal_group_resize`; `windowed/hittest.rs::resize_grip_hover_target`; Ctrl+`+`/`-` focus-scoped hotkeys (`windowed/keyboard.rs::portal_resize_key_code`); token-styled geometry-only scroll indicators; `PortalWindowTokens` clamp bounds; mid-drag re-truncation (`hud-ghhxa`); geometry → adapter as coalescible snapshots (`hud-npq6g`). |
| 7 | Sustained Streaming Cadence | **SATISFIED (soak partial-accepted)** | `crates/tze_hud_projection/src/portal_cadence.rs::PortalCadenceCoalescer` (work-conserving, round-robin cross-portal fairness — `fairness_probe_*` tests); arrival-to-present measurement (`hud-zmt1a`); perf-assert CI lane (`hud-94vm5`). Live cadence within budget: overhead p95 0.003 ms, `over_budget_count=0` (gate report). 60-min full-duration soak accepted **partial** (lease-fix + flat memory proven live 57.6 min / 27–34 MiB) — full 3600 s rerun deferred to `hud-5kq8k`. |
| 8 | Portal Component Profile Styling | **SATISFIED (pre-promotion scope)** | Exemplar publish path sources every visual value from resolved tokens (`hud-1uh1l`, `hud-dcynv`); portal part inventory (frame/header/composer/transcript/divider/collapsed card); redaction-safe collapsed/expanded transitions (`hud-2ps6p`). Post-promotion `text-portal` component-type contract is explicitly a **separate** component-shape-language delta (openspec change `text-portal-component-type`, promotion-era — not a Phase-1 gap). |
| 9 | Phase-1 Promotion Evidence Gate | **SATISFIED** | Gate assessed against all five RFC 0013 §7.2 criteria; owner PROMOTE decision `hud-qfyfg` (2026-07-05). Live evidence: `docs/evidence/text-stream-portals/liveverify-signoff-20260705-105700/` + `liveverify-reconcile-20260705-1716/`. |
| 10 | Promotion Scope Boundary | **SATISFIED (governance contract in force)** | Normative boundary requirement (what promotion may/may not open). Enforced as a reviewer contract; no code artifact — the non-goals (no PTY/VT, no scene-graph history, no chrome UI, no portal transport, no process ownership) held throughout implementation and remain in force post-promotion. |

## MODIFIED requirements — reconciliation

| Requirement | Status | Notes |
|---|---|---|
| Low-Latency Text Interaction | **SATISFIED** | Main spec (line 85) carries the Phase-1 wording: composer draft-change notifications are state-stream / latest-snapshot coalescible; latency budgets (input-to-local-ack p99 < 4 ms, ≤ 2 ms Windows lane). Implemented by `composer_draft.rs::DraftStateNotification` (coalescible) + `DraftSubmission` (transactional). |
| Transcript Interaction Contract | **SATISFIED** | Main spec (line 111) carries bounded composer draft editing under the local-feedback contract; user scroll authoritative over adapter. Implemented by `ComposerDraft` local caret/selection render + `windowed/keyboard.rs::reset_input_history_scroll_to_tail`. |

## Verdict

All ten ADDED and both MODIFIED **Phase-1** requirements are satisfied by shipped
code. The promotion gate PASSED (owner decision). The only non-complete tasks in
`tasks.md` are (a) individual live-exemplar phase checkboxes (§4.8, §5.6, §5.7,
§6.5, §6b.7) whose evidence was produced collectively in the 2026-07-05 owner
live sign-off pass, (b) the full-duration soak (§5.5, accepted partial), and
(c) promotion-era authoring (§7.5) tracked as separate changes/beads. **No
required Phase-1 requirement is unmet.** Archiving is authorized.

## Deferred / promotion-era items (tracked, not Phase-1 gaps)

These are follow-ups for the COORDINATOR to file/verify — this worker does not
mutate bead lifecycle. Cross-referenced to existing beads where known.

- **`hud-5kq8k`** — clean full-duration (3600 s) streaming soak rerun on a
  LAN-local path (§5.5). Lease-fix + flat memory already proven live; full
  completion is the residual.
- **`hud-n5bqp`** (P2) — transient `mutation_result` timeout under sustained
  streaming that aborted the soak 146 s short.
- **`hud-tc153`** — first-class portal surface + scene-mutation schema additions
  (promotion authorizes this; §7.5 / Promotion Scope Boundary).
- **`text-portal-component-type`** (openspec change, 0/12) — the `text-portal`
  component-type contract + canonical token keys (post-promotion component-shape
  -language delta named in Portal Component Profile Styling).
- **`hud-s4lrw`** — multi-node composer layout (promotion-era rendering under
  the g1ena epic).
- **`hud-zlq2v`** — precise streaming-cursor positioning (g1ena chat-grade
  affordance, promotion-scoped).
- **`hud-hwk2m`** — bridged unread count (g1ena chat-grade affordance).
- **`hud-zn6yw`** — header/section layout (g1ena chat-grade affordance).
- **`hud-tlx5c`** — profile-swap reskin owner eyes (§6.5 live verification).
- **`hud-t2k55`** (P3) — OS-injection resize live phase (§6b.7).
- **`hud-4e6c0`** (P3) — exemplar hardcodes minimize/compact control colors.

## Related change: `portal-chat-grade-affordances` (NOT archived)

The chat-grade affordances change (delivery-ack cue, unread divider, jump-to
-latest, per-turn timestamps, streaming cursor, empty-state, connecting/degraded
distinction, and the two-pane INPUT/OUTPUT Viewer Reply Echo modification) is a
**separate, still-open, spec-only** change (14/15 tasks). Its requirements are
explicitly `Scope: promotion (rendering under hud-g1ena)`. Most cues are already
rendered (`crates/tze_hud_projection/src/resident_grpc.rs`:
`delivery_cue_class`/`delivery_cue_color_runs`, `unread_indicator_line`/
`unread_divider_boundary`, `format_wall_clock_arrival_hhmm`,
`activity_cue_color_runs`/`streaming_cursor_color_runs`, `empty_state_color_runs`,
`connecting_color_runs`). Two items remain unrendered and are correctly deferred:
the **jump-to-latest affordance** (no render code found) and the two-pane
**Viewer Reply Echo** modification (main spec still carries the single-stream
version at line 229). This change is **not** part of the Phase-1 archive and
must retain its own lifecycle; it is cross-referenced here only because its
render work shipped alongside Phase-1 this session.
