# Multi-Portal Management UX — Design Proposal

**Issue**: hud-bq0gl.13 (child of flagship epic hud-bq0gl "text-stream portal excellent end-to-end")
**Date**: 2026-06-20
**Author**: agent/hud-bq0gl.13
**Status**: Draft — DESIGN-FIRST, awaiting owner/product direction
**Depends on / contextualizes**: RFC 0013 (Text Stream Portals), RFC 0001 (Scene Contract),
RFC 0004 (Input Model), RFC 0007 (System Shell), RFC 0008 (Lease Governance),
`openspec/changes/portal-disconnect-resume-ux/` (sibling viewer-facing portal work)

> This is a **design proposal**, not a settled contract and not an implementation.
> It surveys what exists, frames the open question, lays out 2–3 directions with
> tradeoffs, recommends one, and enumerates the owner/product decisions that must
> be made **before** any implementation bead is opened. It is upstream of the
> OpenSpec change a future worker would author once a direction is chosen. No Rust
> is changed here.

---

## 0. Problem Statement (verbatim from the bead)

> Coalescer cross-portal fairness exists, but there is no viewer-facing story for
> arranging, focusing, or closing several concurrent portals.

The runtime can already *drive* N concurrent portals fairly (the coalescer round-robins
output across them; see §1.1). It does **not** offer the viewer a coherent way to
**arrange** them on screen, **focus** one for input, or **close** one (or all) as a
deliberate act. Today a second portal is just another tile that lands wherever its
adapter's default geometry puts it, with no relationship to the first. This is the
"several windows, no window manager" gap. RFC 0013 §8 open question 4 names the
adjacent ambiguity directly: *"If multiple portal sources coexist for the same agent,
is that one portal with multiple streams or multiple portals?"*

This proposal scopes the **viewer-facing management model** for the
already-decided "multiple portals" case.

---

## 1. Current State (with file:line citations)

All paths are relative to the tze-hud repo root (`mayor/rig/`).

### 1.1 Cross-portal coalescer fairness — EXISTS, complete

The fairness substrate the bead references is implemented and tested.

- **`crates/tze_hud_projection/src/portal_cadence.rs:127`** — `PortalCadenceCoalescer`
  holds `pending: HashMap<String, PendingPortalSnapshot>` keyed by `projection_id`
  and a `service_order: VecDeque<String>` (`:132`) maintained as the round-robin
  service queue.
- **`portal_cadence.rs:177`** `record_append(...)` — latest-wins coalescing per portal;
  a portal absent from `pending` but present in `service_order` is re-enqueued for
  round-robin continuity (`:224`).
- **`portal_cadence.rs:252`** `next_ready_portal()` — rotates `service_order` to pick
  the next portal with pending output; this is the fairness oracle. The doc comment
  (`portal_cadence.rs:18`) states the structural guarantee: under equal sustained
  input rates across N portals, no portal's presentation lag diverges from another's
  by more than one service round.
- **`crates/tze_hud_projection/src/bin/projection_authority.rs`** drives the drain loop
  via `authority.next_due_projection_id()` until no portal is due.

**Takeaway:** the *driving* side of multi-portal already exists and is fair. The gap is
purely viewer-facing arrangement/focus/close — the management layer on top.

### 1.2 Portal lifecycle, identity, and lease handling — EXISTS

- **`crates/tze_hud_projection/src/authority.rs:88`** — `ProjectionAuthority` holds
  `sessions: HashMap<String, ProjectionSession>` (one entry per portal) plus a single
  shared coalescer instance.
- **`authority.rs:26`** — `ProjectionSession` carries the per-portal state:
  `projection_id` (external key) and a distinct internal `portal_id`,
  `portal_presentation` (Expanded/Collapsed), `lifecycle_state`, `advisory_lease`,
  `reconnect` bookkeeping, and `pending_geometry_batch`.
- Lifecycle entry points: `handle_attach` (`authority.rs:643`), `handle_detach`
  (`authority.rs:1254`), `handle_cleanup` (`authority.rs:1308`),
  `expire_projection` (`authority.rs:543`), `next_due_projection_id` (`authority.rs:495`).
- **Each portal owns its own advisory lease.** There is no group lease and no
  "portal set" object — N portals are N independent sessions that happen to share a
  coalescer. This is the structural fact every option below must reckon with.

### 1.3 Scene-graph representation of portals — EXISTS (per-portal, no grouping)

- **`crates/tze_hud_scene/src/types.rs:447`** — `Tile { id, tab_id, lease_id,
  bounds: Rect (:452), z_order: u32 (:453), opacity, input_mode, root_node, ... }`.
  A portal is a Phase-0 four-tile resident surface (frame tile + transparent capture
  tiles for header drag / composer / scroll) per RFC 0013 §3.4, bound to one
  `lease_id` and living in the **content layer**.
- **`crates/tze_hud_scene/src/types.rs:305`** — `Tab { id, name, display_order,
  tab_switch_on_event }`. Tiles carry `tab_id`; the scene already has a **workspace
  abstraction** (tabs) above tiles, with `active_tab` and `switch_active_tab`
  (`crates/tze_hud_scene/src/mutation.rs:718`; invariant
  `crates/tze_hud_scene/src/invariants.rs:451`). **This is the single most useful
  existing primitive for arrangement** — see §3.
- There is **no** data structure that groups several portals into a managed set.
  Portals relate to each other only implicitly (same `tab_id`, same coalescer).

### 1.4 Focus and z-order — PRIMITIVES EXIST, no portal-aware policy

- **Z-order**: `Tile.z_order: u32` (`types.rs:453`); mutated by
  `update_tile_z_order` (`crates/tze_hud_scene/src/graph/tiles.rs:195`), which
  enforces `z_order < ZONE_TILE_Z_MIN` so agent content stays below runtime zone
  tiles (RFC 0001 §2.3). There is **no** "raise to front / focus-stack" policy that
  ties z-order to focus — z-order is a raw field an adapter sets.
- **Focus**: `crates/tze_hud_input/src/focus.rs` enforces a **single focus owner
  per tab** (`focus.rs:5`, `:155`), with `FocusSource` (`focus.rs:33`:
  PointerClick / Keyboard / Programmatic). The composer `HitRegionNode`
  (`interaction_id="composer_input"`) accepts keyboard focus (RFC 0004 §7.1).
  Focus is per-tab and per-hit-region; **there is no notion of "the focused portal"**
  as a first-class concept — it falls out of which hit-region last took focus.

### 1.5 Geometry: move/resize — EXISTS per-tile, viewer-driven, no arrangement

- `update_tile_bounds` (`tiles.rs:155`) validates and applies a new `Rect`, clamped
  to the display area, bumping `scene.version`.
- Pointer-driven drag-to-move and resize live in
  `crates/tze_hud_runtime/src/windowed/portal.rs`:
  `apply_drag_handle_pointer_event` (`:73`) and `compute_portal_max_dims` (`:273`,
  intersecting lease budget with the display boundary). Geometry changes are coalesced
  back to the authority via `push_geometry_snapshot` (`authority.rs:195`).
- **Each portal is dragged/resized independently.** There is no tiling, snapping,
  cascade, grid, or "arrange all" operation. Two portals can fully overlap with no
  runtime help.

### 1.6 Protocol entry points — EXIST per-portal

- **MCP** (driving/authoring, daemon-side `ToolClass::Resident`, not LLM-facing):
  `handle_portal_projection_attach` (`crates/tze_hud_mcp/src/tools.rs:2230`),
  `..._publish` (`:2336`), `..._detach` (`:2621`), `..._cleanup` (`:2712`).
  These are **per-`projection_id`**; there is no multi-portal/group MCP verb.
- **gRPC / runtime**: viewer pointer gestures (drag/resize, composer input) flow
  through the runtime windowed path (`windowed/portal.rs`) and the resident gRPC
  adapter (`crates/tze_hud_projection/src/resident_grpc.rs`), which materializes
  tiles/nodes and carries local-first input feedback. **Viewer-originated** focus/close
  today is whatever pointer/keyboard already does to a tile; there is no
  "focus portal" / "close portal" / "arrange portals" command surface.

### 1.7 Summary of the gap

| Concern | Driving side (adapter→runtime) | Viewer side (human→runtime) |
|---|---|---|
| Output fairness across N portals | ✅ coalescer (§1.1) | n/a |
| Per-portal lifecycle/lease | ✅ (§1.2) | partial (dismiss removes a tile) |
| Per-portal geometry | ✅ move/resize (§1.5) | ✅ drag one at a time |
| **Arrange several portals** | ❌ | ❌ |
| **Focus one of several portals** | ❌ (no concept) | ❌ (falls out of last hit-region) |
| **Close one / close all deliberately** | per-portal cleanup only | ❌ no group close |
| **A "portal set" the viewer reasons about** | ❌ | ❌ |

---

## 2. The Problem, Precisely

"Arranging, focusing, closing several concurrent portals" requires the runtime and the
viewer to share a model with these capabilities, all under existing doctrine
(screen is sovereign; leases with TTL; one-scene-model-two-profiles; the model never
sits in the frame loop; local-first feedback):

1. **Identity & enumeration.** A way for the viewer (and the runtime's input router) to
   know "these K tiles are the K portals currently present, in this order." Today this
   must be reconstructed from `sessions` + `tab_id`; there is no ordered, viewer-facing
   list.
2. **Arrangement.** A policy for where a *new* portal lands relative to existing ones,
   and an operation to *re-arrange* the set (so portals don't silently overlap). This is
   the "window manager" piece the runtime currently delegates entirely to adapter
   default geometry.
3. **Focus.** A first-class "active portal" the runtime tracks, so input (keyboard,
   command input, "reply") has an unambiguous target, and so focus can be moved between
   portals deliberately (cycle / click-to-focus) under the per-tab single-focus
   invariant (§1.4).
4. **Close.** Viewer-initiated, deliberate close of one portal — and a defined
   "close all / close others" — that maps cleanly onto the existing lease teardown
   path (detach/cleanup → lease release → tile removal) without inventing portal-specific
   lifecycle.
5. **Profile parity.** The same model must degrade onto the Mobile Presence Node (one
   small screen, likely one portal visible at a time) without forking the API
   (one-scene-model-two-profiles). Arrangement on desktop ≈ navigation on mobile.
6. **Governance fidelity.** Arrange/focus/close are **viewer/runtime** authority, never
   adapter authority. An adapter must not be able to seize focus, force its portal to
   the front, or evict another portal. Screen is sovereign; the model drives content,
   not the window manager.

The crux question for the owner: **is multi-portal management a spatial problem
(tiling/arranging surfaces) or a navigation problem (one-at-a-time focus with a
switcher)?** The three options below stake out that spectrum.

---

## 3. Options

All three reuse the existing scene primitives (tiles, `z_order`, `bounds`, tabs, focus
tree, per-portal leases) — they differ in *what new runtime-owned policy/state* sits on
top, and *which protocol plane* carries the new verbs.

> **Plane rule of thumb** (from the three-plane architecture): **arrange/focus/close are
> viewer→runtime control, not LLM authoring.** They are **gRPC resident control-plane**
> operations (or pure local runtime input that needs no new RPC), **not MCP**. MCP stays
> the *driving* plane (attach/publish/detach a portal you own). No arrange/focus/close
> verb should be added to MCP — that would let the model manage the viewer's screen,
> violating "screen is sovereign." This holds for all three options.

### Option A — Operator-Arranged Free Surfaces (minimal; "it's just tiles, better")

Keep portals as independent free-floating tiles (status quo geometry), but add the thin
missing management layer:

- **State**: a runtime-owned, per-tab **ordered portal registry** (a view over
  `sessions` filtered to the active tab, with a stable z-ordered list). No new grouping
  object; it's a projection of existing state.
- **Arrange**: runtime supplies a **non-overlapping placement policy for new portals**
  (cascade/offset, then first-empty-slot) so a second portal never lands exactly on the
  first. Plus one viewer command: **"tidy" / cascade** that re-lays the current set to a
  cascade. Manual drag/resize (§1.5) remains the primary arrangement tool.
- **Focus**: promote "active portal" to first-class runtime state; **click-to-focus
  raises z-order** (focus-follows-raise), and a **cycle-focus** command (e.g. a chrome
  hotkey) moves the active portal through the registry. Reuses the per-tab single-focus
  invariant (`focus.rs:155`).
- **Close**: viewer **dismiss** already removes a tile; add **"close active portal"** and
  **"close all portals (this tab)"** as runtime commands that fan out to the existing
  detach/cleanup lease path (`authority.rs:1254/1308`).

**Maps onto existing model:** almost entirely additive over `z_order`, `bounds`, focus
tree, and per-portal leases. No new scene node types, no grouping object.

**Protocol:** focus/cycle/close/tidy are **runtime-local input + gRPC control**; no MCP
changes; no new long-lived stream (RFC 0013 §2.1 forbids a second per-portal stream).

| Pros | Cons |
|---|---|
| Smallest change; respects RFC 0013 Phase-0 raw-tile scope | Overlap is still possible (only *new* placement is helped) |
| No new scene primitives; lowest risk to the frame loop | "Arrange" stays mostly manual — weak answer to the bead's "arranging" |
| Trivially degrades to mobile (one portal focused, others behind) | No structural guarantee portals stay legible as a set |
| Focus-as-first-class is independently valuable | Doesn't establish a reusable layout concept |

### Option B — Runtime-Managed Layout Slots (tiling / grid; "window manager")

Introduce a runtime-owned **layout** for the active tab that assigns each portal to a
**slot** in a chosen template (e.g. single / split-2 / grid-2x2). Portals stop being
free-floating; the runtime computes their `bounds` from the layout.

- **State**: a per-tab **PortalLayout** (template + ordered slot→portal assignment),
  runtime-owned. Portal tiles' `bounds` become **derived** from the slot rect (manual
  resize becomes "resize the split," not the tile).
- **Arrange**: a small set of **layout templates**; a new portal fills the next free
  slot; viewer can pick template and swap slot assignments. Overlap is structurally
  impossible.
- **Focus**: the focused slot is the active portal; cycle = move through slots; the
  focused slot may get a subtle (token-resolved, non-chrome) emphasis. Single-focus
  invariant unchanged.
- **Close**: closing a portal frees its slot; layout reflows (or leaves a hole, per
  policy). Maps to existing detach/cleanup leases.

**Maps onto existing model:** reuses `Tile.bounds`/`z_order` as *outputs* of layout;
needs a new runtime-owned `PortalLayout` state and a reflow step that writes tile bounds
(outside the frame loop, as a mutation batch). Must respect lease spatial budgets
(`compute_portal_max_dims`, `windowed/portal.rs:273`) — a slot can't exceed a portal's
lease budget, so layout is budget-clamped.

**Protocol:** new **gRPC resident control verbs** (set-layout, assign-slot, focus-slot,
close-slot). Still no MCP arrange verbs. Layout is viewer/runtime authority.

| Pros | Cons |
|---|---|
| Strong, legible answer to "arranging" — overlap impossible | Largest new runtime concept (layout engine + reflow) |
| Predictable, screenshot-stable surfaces | Tension with per-portal lease budgets (slot vs budget clamp) |
| Maps cleanly to mobile (template = "single", others off-screen) | Risks "tiling WM" scope creep beyond presence thesis |
| Reusable beyond portals (any content tiles) | Manual free placement is lost (or becomes a "floating" exception) |

### Option C — Focus-Stack with Peek (navigation, not spatial; "one-at-a-time")

Treat multi-portal as a **navigation** problem: at most one portal is **expanded/active**
at a time; the rest are **collapsed cards** (RFC 0013 §3.2 already defines
Collapsed/Expanded states) arranged in a compact, runtime-owned **strip/dock**.

- **State**: a per-tab **focus stack** (MRU order) of portals; one expanded, the rest
  collapsed. Reuses `portal_presentation` (Expanded/Collapsed) already on
  `ProjectionSession` (`authority.rs:26`).
- **Arrange**: the runtime lays out the collapsed strip (deterministic); the expanded
  portal takes the main region. "Arrangement" becomes "which one is expanded + strip
  order." A "peek" gesture temporarily expands a card.
- **Focus**: focus == the expanded portal; cycle expands the next card. Naturally one
  input target. Single-focus invariant trivially satisfied.
- **Close**: close = remove from stack (detach/cleanup lease); next MRU portal expands.

**Maps onto existing model:** reuses Expanded/Collapsed presentation + per-portal leases;
needs runtime-owned strip layout + focus-stack ordering. No grid math, no slot/budget
tension (only one large surface at a time).

**Protocol:** **gRPC control** for expand/collapse/cycle/close; no MCP arrange verbs.

| Pros | Cons |
|---|---|
| **Best one-scene-model-two-profiles fit** — desktop strip == mobile switcher | Only one portal richly visible at once (no side-by-side) |
| Reuses Collapsed/Expanded already in the contract | Weaker for "monitor several live streams simultaneously" |
| Smallest spatial surface area; no overlap, no grid engine | "Strip/dock" risks looking like chrome — must stay content-layer (RFC 0013 §3.1) |
| Strong attention story (one focus, others ambient) — RFC 0013 §6.5 | If several portals are equally important, forces serialization |

---

## 4. Recommendation

**Recommend a phased path: ship Option A first, designed so it can grow into Option C,
and treat Option B as a deferred, separately-approved enhancement.**

Rationale, tied to doctrine and the current state:

- **Option A is the smallest doctrine-safe step that closes the worst of the gap**
  (no overlap on spawn, a real "active portal," deliberate close-one/close-all) without
  adding a new scene concept or risking the frame loop. It is almost entirely additive
  over primitives that already exist (§1.4, §1.5), so it is low-risk and respects
  RFC 0013's Phase-0 raw-tile scope and §7.2 promotion gate (don't promote to a
  first-class portal surface without evidence).
- **Option C is the best long-term fit for one-scene-model-two-profiles** — the
  desktop collapsed-strip and the mobile portal-switcher are the *same* model at
  different budgets, which is exactly the doctrine. Because Collapsed/Expanded already
  exist in the contract and on `ProjectionSession`, A's "active portal + registry"
  state is the natural seed for C's focus-stack. Designing A's registry as an *ordered,
  MRU-capable* list now avoids a rewrite later.
- **Option B (tiling/grid) is powerful but is the highest scope-creep risk** relative
  to the presence thesis ("not a window manager"), and it collides with per-portal lease
  spatial budgets. It should be gated behind explicit owner approval and real evidence
  that side-by-side simultaneous viewing is a required workflow (RFC 0013 §7.2-style
  evidence). It is a *follow-on*, not the first step.

**The first concrete value to ship is "active portal" as first-class runtime state**
(focus + raise + cycle), because every option needs it and it is independently the most
glaring missing primitive (§1.4).

### Phased implementation sketch (candidate follow-up beads)

Numbered as would-be beads for the coordinator to create. Each is independently
shippable and verifiable.

1. **Spec the chosen direction as an OpenSpec change.** Author
   `openspec/changes/multi-portal-management/` (proposal + design + specs + tasks),
   ADDED requirements on `text-stream-portals` for: ordered portal registry, active
   portal, viewer arrange/focus/close semantics, and the explicit non-goal of an MCP
   arrange surface. *Blocked on the §5 owner decisions.* (sibling of the existing
   `portal-disconnect-resume-ux` change.)
2. **First-class "active portal" runtime state + focus model.** Promote "active
   portal" to runtime-owned per-tab state; define click-to-focus → raise z-order and a
   cycle-focus command; reuse the per-tab single-focus invariant (`focus.rs:155`).
   Pure runtime/input; no MCP.
3. **Ordered per-tab portal registry (read model).** A runtime-owned ordered view over
   `sessions` filtered to a tab (z-ordered, MRU-capable so it can later feed Option C).
   Enumeration only; no new persistence.
4. **Non-overlapping new-portal placement + "tidy/cascade" command.** Placement policy
   for spawn; one viewer command to cascade the current set. Reuses `update_tile_bounds`
   (`tiles.rs:155`), clamped by lease budgets (`windowed/portal.rs:273`).
5. **Deliberate close: "close active" / "close all (tab)".** gRPC control verbs that
   fan out over the existing detach/cleanup lease path (`authority.rs:1254/1308`);
   confirm lease release + tile removal; honor dismiss/safe-mode/override (RFC 0013 §6.4).
6. **(If Option C chosen/added) Collapsed-strip + focus-stack.** Build on #2/#3: MRU
   focus stack, collapsed-card strip in the content layer, expand/collapse/cycle/peek.
   Verify mobile-profile parity.
7. **(Deferred, separately approved) Option B layout engine.** Only if §5 decisions and
   evidence justify simultaneous side-by-side: runtime `PortalLayout`, slot reflow as
   out-of-frame mutation batches, slot-vs-lease-budget clamping, layout gRPC verbs.
8. **Conformance + evidence.** Tests for: focus single-owner across portals, no-overlap
   placement, close-all lease teardown, redaction/safe-mode behavior across the set, and
   a soak under cross-portal load (extend existing `docs/evidence/text-stream-portals/`
   soak harness). Feeds any RFC 0013 §7.2 promotion argument.

---

## 5. Owner / Product Decisions Required (do NOT guess)

These must be answered before bead #1 (the OpenSpec change) can be written. Each is a
genuine product/owner call, not an engineering detail.

1. **Spatial vs. navigation.** Is the target workflow **side-by-side simultaneous
   viewing** of several live portals (→ Option B / tiling), or **one-focused-at-a-time
   with fast switching** (→ Option C / focus-stack)? Option A defers this but the
   long-term shape depends on it. *This is the load-bearing decision.*

2. **How many concurrent portals must the viewer manage well?** The realistic ceiling
   (2? 4? 8?) changes everything: 2 → split is enough; 8 → a strip/switcher (C) beats a
   grid. (The coalescer already scales to N; this is purely the *human* ceiling.)

3. **One agent, many portals — one surface or many?** RFC 0013 §8 Q4 is still open:
   when a single agent drives multiple streams, is that **one portal with multiple
   streams** or **multiple portals**? This proposal assumes "multiple portals"; if the
   answer is "one portal, multiplexed streams," the management model changes
   substantially and this proposal must be re-scoped.

4. **Is a runtime-owned layout/dock acceptable, or does it read as chrome?** Options B
   and C introduce runtime-arranged surfaces (slots / collapsed strip). RFC 0013 §3.1
   forbids chrome-layer agent portal UI. Owner must confirm a **content-layer**
   runtime-arranged region is acceptable and not a violation of chrome sovereignty
   (RFC 0007).

5. **Does multi-portal arrangement persist across sessions/restarts?** v1 projection
   state is memory-only (per `cooperative-hud-projection`); should arrangement/focus be
   ephemeral, or part of saved workspace state? (Interacts with the tab model,
   `types.rs:305`.)

6. **Mobile parity expectations.** On the Mobile Presence Node, is "multi-portal" simply
   "one visible + a switcher," and is that acceptable as the *same* model at a smaller
   budget (one-scene-model-two-profiles), or does product want a distinct mobile
   behavior? (A distinct behavior would violate the no-fork doctrine and needs explicit
   sign-off.)

7. **Close semantics & confirmation.** Should "close all portals" require confirmation?
   Does close mean detach (revocable within grace, per
   `portal-disconnect-resume-ux`) or hard cleanup (irrevocable)? Default proposed:
   close = detach with grace; "force close" = cleanup.

8. **Input affordance surface.** Are arrange/focus/cycle/close driven by chrome hotkeys,
   on-surface hit-regions, or both? (Affects RFC 0004 input routing and the
   chrome-sovereignty boundary — affordances must not become agent chrome.)

---

## 6. Doctrine Compliance Notes

- **Screen is sovereign / model not in frame loop**: arrange/focus/close are
  viewer+runtime authority on the gRPC control plane; **no MCP arrange verbs** — the
  model never manages the viewer's window layout. Any reflow runs as an out-of-frame
  mutation batch, never in the compositor frame loop.
- **Leases with TTL**: close maps onto existing per-portal detach/cleanup → lease
  release; no group lease is invented; no lease bypass.
- **One scene model, two profiles**: the recommendation explicitly optimizes for the
  desktop-strip ≡ mobile-switcher equivalence; no API fork.
- **Local-first feedback**: focus/raise/close acknowledge locally before any adapter
  notification, reusing RFC 0004 local-first semantics.
- **No hardcoded visual properties**: any focus emphasis, strip styling, or slot chrome
  resolves from design tokens via the component-profile path (RFC 0001 / craft-and-care),
  never literals in the compositor.
- **Graceful degradation is not a bug**: arrangement coexists with the sibling
  `portal-disconnect-resume-ux` work — a disconnected portal in the set retains its
  slot/card and shows its degraded treatment rather than vanishing or faking liveness.
