# Proposal: portal-turn-model-attribution

## Why

The text-stream portal's OUTPUT transcript is still published as a **single**
`TextMarkdownNode` blob (`crates/tze_hud_projection/src/resident_grpc.rs`,
`portal_node`). Turn *structure* was partially delivered by the Phase-1 pilot —
`visible_transcript_markdown_with` re-encodes the `Vec<TranscriptUnit>` boundary
as `\n---\n` thematic-break separators (hud-nx7yq.4, #994) — but turn
*attribution* was explicitly deferred at that scoping ("no attribution
chips/alignment; promotion hud-g1ena owns the full turn model"). The promotion
epic hud-g1ena closed without a turn-model requirement, and the shipped
`portal-chat-grade-affordances` spec has no per-turn attribution requirement
either, so the Wave-3 item (`vd-no-conversational-turn-model` /
`chat-no-turn-structure-attribution`, from the hud-0yrix audit) never landed.
Every retained OUTPUT turn — the model's own prose and its tool/status/error
output alike — renders in the single `transcript_text_color`, so a viewer cannot
tell the assistant's conversational turns apart from tool/system scaffolding.

An architectural constraint bounds what "multi-node turn model" can mean today.
PR #1149 (hud-ga4md) shipped `NodeProto.children` + atomic
`SetTileRoot.descendants` subtree *materialization*, but the compositor has **no
vertical flow/stack layout**: every node paints at its own explicit `bounds.y`
(`crates/tze_hud_compositor/src/text.rs`), and the projection layer has no
text-measurement capability to compute per-turn Y offsets for wrapped turns.
Emitting one child `TextMarkdownNode` per turn would overlap them all at
`y = 0`. A true per-turn *node* transcript therefore requires a compositor
vertical-flow layout capability (or per-turn height feedback into projection)
that does not exist yet — that prerequisite is out of scope for this change and
is tracked as a follow-up. This change delivers the achievable, coalescing-safe
half of the turn model — per-turn role attribution — within the single
transcript node, and formalizes the contract so the node split has a spec to
grow into once the layout prerequisite lands.

## What Changes

- **Per-turn role attribution**: each OUTPUT transcript turn SHALL be presented
  with token-styled attribution distinguishing the assistant's own
  conversational turns (`OutputKind::Assistant`) from tool/status/error/other
  agent-side output (`Tool` / `Status` / `Error` / `Other`). Attribution is a
  real color-run span over each attributed turn's text (the same
  `TextColorRunProto` span mechanism used for adapter ANSI runs), sourced from a
  design token — never a hardcoded compositor color.
- **Turn structure formalized**: the existing `\n---\n` turn separators
  (hud-nx7yq.4) are lifted from an implementation detail into a first-class
  requirement — the OUTPUT transcript is a sequence of discrete, kind-attributed
  turns, not an opaque blob.
- **Coalescing invariant preserved**: attribution is carried on the existing
  single transcript node's `color_runs`; no per-turn `AddNode` fan-out, so the
  hot transcript-republish path stays a coalescible StateStream `SetTileRoot`
  update (hud-mzk74). `NodeProto.children` stays empty on this path until the
  layout prerequisite lands.
- **Node split explicitly deferred**: the requirement records that materializing
  each turn as its own scene node is gated on a compositor vertical-flow layout
  capability, so a future promotion can satisfy the same contract structurally
  without re-opening the attribution decision.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `text-stream-portals`: ADDS a "Conversational Turn Model and Per-Turn Role
  Attribution" requirement consistent with §Viewer Reply Echo (agent OUTPUT vs
  viewer INPUT split), §Ambient Portal Attention Defaults (attribution is
  ambient, never an attention event), §Portal Component Profile Styling (token
  driven), and §Promotion Scope Boundary (a node split is scene-mutation schema
  work permitted only once gated).

## Impact

- **Spec**: delta on `text-stream-portals`.
- **Code** (bead hud-26869): `crates/tze_hud_projection/src/resident_grpc.rs`
  (per-turn attribution color runs keyed on `OutputKind`; single node retained);
  `crates/tze_hud_config/src/portal_tokens.rs` (new `transcript_system_color`
  attribution token + default + resolution).
- **Interactions**: builds directly on the turn separators from
  `portal-bottom-chat-composer` / hud-nx7yq.4; does not alter the two-region
  INPUT/OUTPUT split from §Viewer Reply Echo (viewer turns already live in the
  separate INPUT history).
- **Non-goals**: per-turn scene *nodes* / multi-node transcript layout (gated on
  a compositor vertical-flow layout capability — filed as a follow-up); turn
  alignment, avatar/name chips, or bubble styling beyond a single token-driven
  role color; terminal emulation; scene-graph transcript history.
