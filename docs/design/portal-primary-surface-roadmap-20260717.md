# Text-Stream Portal as Primary Human↔LLM Surface — Brainstorm & Specification Sequence

Date: 2026-07-17 · Status: proposal (owner: Tzeusy) · Origin: owner mandate to optimize the
text-stream portal (UX, speed, performance, intuitiveness, token efficiency) toward
daily-driver viability as the primary way to interact with an LLM session.

Provenance: four independent code-grounded exploration verticals (implementation state,
MCP/token surface, spec landscape, onboarding/daily-driver UX) run 2026-07-17 against
main @ 09f69c6d, each verifying claims in code rather than trusting bead/spec prose.
Umbrella epic: **hud-wse80**; token-efficiency work coordinates with **hud-f670c**.

---

## 1. Verified current state (corrections to received wisdom)

The portal is substantially more finished than the bead trail suggests:

- **Two-pane INPUT|OUTPUT chrome is shipped** in the production render path
  (`crates/tze_hud_projection/src/resident_grpc.rs:1517` `expanded_portal_geometry`;
  header drag band, divider, token-styled panes). hud-z5uem is truthfully closed.
- **Long-poll exists**: `portal_projection_get_pending_input` takes `wait_ms`
  (≤30 000 ms, served on the async MCP side, `crates/tze_hud_mcp/src/tools.rs:2557-2617`).
  Busy-polling is already unnecessary; efficiency doctrine explicitly blesses this shape.
- **Disconnect/stale/reconnect-resume UX is specced (v1-mandatory) and largely shipped**
  for HUD-side disconnects (`PORTAL_DISCONNECT_MARKER_LINE`, `resident_grpc.rs:29`).
- Viewer echo two-stream split, clearance-filtered unread divider, delivery state machine
  (Pending/Deferred/Delivered + `✓✓` cues), idempotent attach recovery, IME candidate
  anchoring, resize wrap-reflow, cross-portal fairness — all shipped and coherent.
- Zero TODO/FIXME markers in `tze_hud_projection`; the gaps are documented phase-gates,
  not stubs.
- MCP tool descriptions are lean (~1 KB total). The standing token tax is the **derived
  `inputSchema` payload: ~20 KB ≈ 5 k tokens per session** (`schema.rs:191-216` budget
  test). Per-turn marginal cost ≈ **502 tokens** (publish 153 + poll 210 + ack 139,
  revised candidate packet on the unmerged hud-pngbn/hud-8eguk work).

### Where the real gaps concentrate

1. **Liveness observability** — a dead LLM session is indistinguishable from a thinking
   one. `last_heartbeat_wall_us` exists (`authority.rs:366-388`) but no sweep compares it
   to a degraded threshold and nothing renders agent-side staleness; the stale marker
   fires only on HUD disconnect. The v1-mandatory **Portal Stale-Content Degradation
   Contract** (`openspec/specs/text-stream-portals/spec.md:637`) already specifies
   exactly this behavior → this is spec-to-code divergence, not new design.
2. **Continuity across runtime restart** — all portal state is memory-only
   (`contract.rs:729`). Runtime-side persistence would *violate* the Cooperative
   Projection State Externality requirement (`text-stream-portals/spec.md:278`); the
   spec-compliant answer is **client-side retention + replay** via the already-specced
   `logical_unit_id` idempotency / `coalesce_key` in-place-update resume semantics.
3. **Token overhead per turn** — 3 calls/turn with echo fields the client already knows.
4. **Escape hatches & discoverability** — no copy-out of transcript, no link affordance,
   no shortcut hints, no session enumeration op, stale "known bug" doc caveats.

### The single highest-leverage existing bead

**hud-clu38** (Phase-1 promotion evidence package). Owner approved promotion 2026-07-17;
this evidence package is the only thing gating **seven** already-specced chat-grade
requirements (per-turn delivery acks, unread divider + ambient count, jump-to-latest,
ambient timestamps, activity/streaming cue, first-run treatment, connecting-state
distinction) plus hud-uym23 (per-turn transcript nodes). No new spec or brainstorm output
here can match its unlock ratio.

---

## 2. Brainstorm inventory

Kept (→ becomes a spec/bead below): actionable MCP error taxonomy; publish-response
input piggyback; response-field diet; schema standing-tax ratchet; numeric token budgets
in spec; agent-liveness degradation wiring; client-side continuity replay; transcript
copy-out; link affordance; session enumeration op; shortcut discoverability; monotonic
input timestamps; benign-reconnect lease retention; pending-drain item-cap raise; stale
docs fix; `--emit-mcp-config`.

**Rejected / deliberately not pursued:**

- **gRPC push stream for pending input** — violates the Promotion Scope Boundary
  ("no dedicated portal transport / second long-lived stream",
  `text-stream-portals/spec.md:582`); long-poll is doctrine-blessed and the 150 ms
  internal poll interval is far below human-conversation latency floors.
- **Runtime-side transcript persistence** — violates State Externality (above); replaced
  by client-side replay (S6).
- **Per-portal mute/DND** — Ambient Portal Attention Defaults already make portals
  non-interruptive; system-shell owns override surfaces. No demonstrated need.
- **Markdown render diffing in the adapter** (`render_portal_markdown` String rebuild) —
  path is rate-limited to 10 updates/s and commit-time MarkdownCache work (hud-5wx09,
  hud-wrlv1) already removed the per-frame costs; marginal.
- **Inline IME preedit** — explicitly v1-reserved (`widgets.rs:169-171`); revisit
  post-promotion, not now.
- **Re-speccing** two-pane chrome, disconnect/resume UX, multi-line composer,
  pointer-free focus, whole-unit resize, multi-portal management — all already specced,
  in-flight openspec changes, or awaiting owner direction on an existing draft proposal
  (`docs/design/multi-portal-management-ux-proposal.md`).

---

## 3. Specification sequence

Ordered so measurement precedes optimization, and spec deltas precede implementation.

### S1 — Actionable MCP error taxonomy (no spec delta; hardening of existing contract)
`handle_portal_projection_*` collapses authority rejections into `McpError::Internal`
(`tools.rs:2582-2587`): an expired 24 h owner token is indistinguishable from a broken
runtime. Map `ProjectionErrorCode` onto distinct MCP error codes with self-describing,
recovery-prescribing messages (expired/invalid token → "re-attach with your original
idempotency_key to rotate the token"). Acceptance: every `ProjectionErrorCode` variant
has a deterministic MCP error mapping + message naming the recovery op; tests assert an
unattended client can distinguish reattach-needed from runtime-broken.

### S2 — Publish-response pending-input piggyback (delta: cooperative-hud-projection)
Add bounded `pending_input` items (+ `remaining_count`) to the publish response —
opt-in via a request flag or implied by `expects_reply` — so the canonical turn
(publish → long-poll → ack) collapses from 3 calls to 2. Measured candidate economics:
~210 of ~502 tokens/turn (~42%). Constraints: response stays bounded per the Low-Token
Operations requirement; items delivered this way follow the identical delivery-state
machine (flip to Delivered, schedule repaint) so `✓✓` semantics are unchanged; empty
piggyback adds ≤ ~10 tokens. Sequenced **after** the token-footprint harness merges
(hud-pngbn) so the win is measured, not asserted.

### S3 — Response-field diet + schema standing-tax ratchet (no delta; impl under Low-Token ops)
(a) Stop echoing per-item `projection_id` in `PendingInputEntry` (`portal_op.rs:283-298`);
omit near-constant `delivery_state`/`content_classification` when default; make
`status_summary` omittable on success; drop the `lifecycle_state` echo from
`publish_status` (`tools.rs:2451`). (b) Second slimming pass on derived `inputSchema`
(~6 836 B portal / ~20 KB total) and **lower** the budget-test ceilings (`schema.rs:191-216`)
so the tax can only ratchet down. Sequenced after hud-pngbn for before/after numbers.

### S4 — Numeric token budgets for canonical portal flows (delta: new efficiency-budget requirements)
Doctrine says "token cost is a product metric" naming the portal conversation flow, yet
`grep -riE "token (budget|footprint|cost)"` over openspec/specs + RFCs returns zero hits.
Once the owner approves the baseline (hud-ht1k7), encode the approved integers as spec
requirements with a CI-enforced regression gate (fail-closed on unexplained growth),
complementing the compute-side `profile-runtime-budget-envelope` change. Blocked by
hud-ht1k7; harness from hud-pngbn is the measurement authority.

### S5 — Agent-liveness degradation wiring (no delta; implements v1-mandatory Stale-Content Degradation)
When a cooperative projection produces no publish/poll/heartbeat within the configured
degraded threshold, transition to Degraded, render the token-styled stale treatment, and
clear live-only cues; recover on the next authenticated op. Bounded by lease grace per
the existing contract. Data (`last_heartbeat_wall_us`) exists; missing are the sweep,
threshold config, and lifecycle transition. Acceptance: operator can visually distinguish
*thinking* (live, recent ops) from *gone* (degraded) without typing into the void.

### S6 — Client-side continuity replay (tooling/skill; State-Externality-compliant durability)
`scripts/portal_client.py` + hud-projection skill retain a rolling transcript tail
locally (bounded, client-side) and, on reattach after runtime restart/lease death,
replay it under existing `logical_unit_id` idempotency + `coalesce_key` semantics before
resuming live publishing. No runtime changes; codify the resume ceremony in the skill.

### S7 — Transcript copy-out (delta: text-stream-portals, new requirement)
The operator must be able to get text *off* the portal (commands, snippets, URLs).
Local-only clipboard export honoring redaction (a restricted viewer copies nothing a
redacted view doesn't show); v1 shape = keyboard-driven copy of last agent turn /
focused turn (pointer range selection may phase in later); no adapter round trip; no
scene-graph history reconstruction. New requirement + scenarios, then impl child.

### S8 — Link affordance in the markdown subset (delta: text-stream-portals)
Links render distinguishable (token-styled) and are actionable safely: v1 = copy-URL
affordance (keyboard-first), never an automatic browser launch from portal content;
opening externally requires explicit operator confirmation and is auditable. Delta then
impl child.

### S9 — Projection enumeration op (delta: cooperative-hud-projection)
`portal_projection_list`: bounded summaries (projection_id, display_name, lifecycle,
unread/pending counts) scoped to the resident principal — the recovery/reconcile
primitive for orphaned sessions (today recovery requires knowing both projection_id and
the original idempotency key). Must respect Low-Token bounds; no transcript content.

### S10 — Shortcut discoverability (align with gated First-Run treatment; no new surface)
Existing affordances (Ctrl+=/Ctrl+-, Tab ring, collapse, Esc-to-composer) are invisible.
Token-styled ambient hints (composer placeholder rotation and/or focused-portal hint
line) consistent with — not duplicating — the promotion-gated First-Run Empty Portal
Treatment. Small delta if any new rendering is required.

### Quick wins needing no specification
- **Stale doc caveats**: SKILL.md:49-53/:86-90 + docs/QUICKSTART.md:238 still tell every
  fresh agent that fixed bugs (hud-09emd `tools/call`, hud-d5rcd `No active tab`) are
  broken — the top adoption-funnel defect found.
- **Monotonic timestamps on input dispatch** (`input_dispatch.rs:63,449,482`) — without
  them operator→LLM latency is unmeasurable and no input-path budget can be enforced.
- **Advisory lease retention on benign reconnect** (`authority.rs:339-341`).
- **Pending-drain item cap** (`max_poll_items` default 8, `lib.rs:47`) — raise/make
  adaptive; bytes are already budgeted separately.
- **`quickstart.sh --emit-mcp-config`** — emit the client MCP JSON directly; the PSK
  hand-copy is the last manual onboarding step.

---

## 4. Bead map

| Spec | Bead | P | Blocks/blocked-by |
|---|---|---|---|
| docs fix | hud-i16zd | P1 | — |
| S5 liveness | hud-ccj2o | P1 | — |
| S1 errors | hud-w2h5c | P2 | — |
| S2 piggyback | hud-vconx | P2 | blocked-by hud-pngbn |
| S4 token budgets | hud-gt92q | P2 | blocked-by hud-ht1k7; child of hud-f670c |
| S6 continuity | hud-9139y | P2 | — |
| S7 copy-out | hud-atk3t | P2 | — |
| S3 diet+ratchet | hud-uoec0 | P3 | blocked-by hud-pngbn |
| S8 links | hud-26m49 | P3 | blocked-by hud-atk3t (clipboard seam) |
| S9 list op | hud-j352s | P3 | — |
| S10 discoverability | hud-md8od | P3 | — |
| mono clock hud-bik7q / lease hud-ie7oa / poll cap hud-f5ya2 / emit-mcp-config hud-af0wg | — | P3 | — |

All are children of hud-wse80 unless noted. Existing beads deliberately **not**
duplicated: hud-clu38 (promotion evidence — highest leverage, recommend prioritizing),
hud-uym23, hud-pngbn, hud-ht1k7, hud-acfvp, hud-jezmt, hud-2v8br, hud-pncm3, the
live-verify set (hud-sp8l7, hud-vvdvy, hud-2u5j7, hud-gmwuf, hud-pdl1d), and the three
in-flight openspec changes (portal-bottom-chat-composer,
portal-composer-interaction-completeness, portal-whole-unit-resize).
