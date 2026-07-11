# Design: portal-turn-model-attribution

## Context

The OUTPUT transcript is lowered to one `TextMarkdownNode` in `portal_node`
(`crates/tze_hud_projection/src/resident_grpc.rs`). `visible_transcript_markdown_with`
already joins the `Vec<TranscriptUnit>` with `\n---\n` thematic breaks
(hud-nx7yq.4), so turn *structure* exists at the string level; turn
*attribution* does not — every unit renders in `transcript_text_color`. All the
portal's ambient cues (unread divider, timestamps, streaming cursor, lifecycle
accent) are already carried as `TextColorRunProto` entries on that single node,
so attribution fits the established pattern.

## Goals / Non-Goals

**Goals**: per-turn role attribution (assistant prose vs tool/status/error/other)
that is token-driven, ambient, redaction-safe, and coalescing-safe; formalize
the turn-structure contract; keep the single-node lowering.

**Non-Goals**: per-turn scene *nodes* / multi-node transcript layout; turn
alignment, name/avatar chips, bubble styling; a second attention channel;
terminal emulation; scene-graph transcript history.

## Decisions

1. **Attribution ships in the single node, not per-turn nodes.** Verified (two
   independent code investigations, 2026-07-11) that PR #1149's
   `NodeProto.children` ships atomic subtree *materialization*
   (`convert::proto_node_tree_to_scene` → `SceneMutation::SetTileRoot { descendants }`
   → `insert_node_tree`) but the compositor has **no vertical flow/stack
   layout**: `text.rs` plots every node at `tile_y + node.bounds.y`, children
   recursion advances no pen between siblings, and `PortalPart` bounds are coarse
   fixed section bands. The projection layer cannot measure wrapped turn heights,
   so per-turn child nodes would all paint at `y = 0` and overlap. A true node
   split is therefore blocked on a compositor vertical-flow layout capability and
   is filed as a follow-up; this change delivers the attribution half now.

2. **Attribution is a real color-run span, not a zero-length sentinel.** Unlike
   the divider/timestamp/lifecycle cues (which are byte-0 or content-end
   sentinels the compositor interprets against markdown markers), attribution
   colors each attributed turn's *text*. This is exactly the `TextColorRunProto`
   `[start_byte, end_byte)` span mechanism already used for adapter ANSI runs
   (Phase-0 Raw-Tile Pilot §adapter-side ANSI color), so no compositor change is
   needed — spans over the raw content bytes render natively.

3. **Segmentation source = `OutputKind`.** Within OUTPUT, `Assistant` is the
   model's conversational prose (base color); `Tool` / `Status` / `Error` /
   `Other` are agent-side scaffolding (attribution color). `Viewer` never appears
   in `visible_transcript` (it lives in the separate INPUT history), so no
   viewer attribution is computed here. One token (`transcript_system_color`)
   expresses the binary — no per-kind color proliferation (avoids gold-plating).

4. **Offsets computed at assembly time.** `portal_markdown_with` gains a
   spans-returning sibling that emits, for each attributed turn, its absolute
   `[start, end)` byte range in the assembled content (the transcript body's
   offset within the full blob plus the per-turn offset within the body, both
   known where the body is built). `portal_node` turns those into color runs.
   This keeps offsets exact and in one place rather than re-deriving them.

5. **Coalescing invariant is trivially preserved.** Attribution adds only
   `color_runs`; `children` stays empty; the emission stays one `PublishToTile`
   (→ internal `SetTileRoot`), so no `AddNode` fan-out and StateStream latest-
   wins coalescing is untouched (hud-mzk74).

## Risks / Trade-offs

- **[Attribution vs. eventual node split]** Shipping attribution on the single
  node now, with the node split deferred, risks the contract reading as "done"
  when the structural half is still owed. → Mitigation: the requirement states
  the node split explicitly as gated future work and the follow-up bead is
  filed; the contract is written to be satisfiable by either lowering.
- **[Byte-offset fragility]** Color-run spans over the assembled markdown must
  track separator/timestamp/unread-divider offsets exactly. → Mitigation:
  compute spans in the same pass that assembles the body (Decision 4), unit-test
  the offsets against the emitted content.
- **[One token may be too coarse]** A single attribution color for all non-
  assistant kinds cannot distinguish error from tool. → Accepted for v1 (not
  gold-plating); a future profile can split the token without changing the
  contract.

## Migration Plan

Spec-only delta + a bounded projection/config change. Validate `--strict`, land
with the hud-26869 implementation, then sync + archive per hud-hpuzp convention.
Rollback = drop the attribution color runs (revert to base color everywhere);
the turn separators are unaffected (they predate this change).

## Open Questions

- Whether `Error` turns eventually warrant a dedicated attention-adjacent token
  distinct from tool/status (defer to the visual-token compliance epic).
- Whether the eventual node split reuses `PortalPart` bands per turn or a new
  flow-layout part kind (decide with the compositor-layout follow-up).
