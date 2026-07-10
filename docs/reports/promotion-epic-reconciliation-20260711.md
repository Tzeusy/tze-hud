# Promotion Epic Reconciliation (gen-1) — First-Class Portal Surface

**Epic:** hud-g1ena — *Phase-1 portal: first-class portal surface + multi-node composer layout + text-portal component type*
**Reconciliation bead:** hud-sy3rz (P2, `reconciliation`)
**Date:** 2026-07-11
**Auditor:** worker hud-sy3rz (report-only; no code mutated)
**Method:** four parallel adversarial dimension audits + direct file:line verification of every headline finding.

---

## Scope

Deep spec-to-code audit of the **completed** promotion epic before the coordinator closes it. Constituent merged PRs audited:

| Bead | PR | Deliverable | Bead status |
|---|---|---|---|
| hud-2ey2w (P1) | #957 | `text-portal` component-shape-language delta (OpenSpec change) | closed |
| hud-8691s (P2) | — | canonicalize portal token keys | closed |
| hud-tc153 (P3) | #1092 | first-class `PortalSurface` scene-mutation schema | closed |
| hud-s4lrw (P4) | #1099 | multi-node per-part layout (one scene node per part) | closed |
| hud-rpm9s (P5) | #1108 | migrate exemplar + cooperative adapters onto first-class surface | closed |
| hud-8z3w3 (P6) | #1111 | governance parity + `set_portal_surface` Transactional fix | closed |
| hud-9gyao | #1113 | bounded composer at-capacity per-line color run | **in_progress** |
| hud-scgyw | #1006 | multi-line composer selection highlight | closed |
| hud-ruynm | #1098 | portal surface descriptors in SceneSnapshot (reconnect parity) | closed |
| hud-a745w | #1104 | lease-gate the lifecycle-accent overlay (reference pattern) | closed |
| g1ena.1–.7 | #1081–#1090 | ambient render children (cursor, divider, unread pill, timestamps, empty-state, degraded treatment) | — |

Contracts read in full: `openspec/specs/text-stream-portals/spec.md` (587 L), `openspec/specs/component-shape-language/spec.md` (839 L), `about/legends-and-lore/rfcs/0013-text-stream-portals.md` (351 L), and the `openspec/changes/text-portal-component-type/` delta.

---

## Findings table

| ID | Severity | File:line | Summary |
|---|---|---|---|
| F1 | **major** | `crates/tze_hud_projection/src/resident_grpc.rs:1162` | Interaction-enabled portal emits a composer-hit-region `AddNode` **every render** → whole streaming batch classes Transactional (`traffic.rs:128`) → bypasses StateStream latest-wins coalescing under freeze/backpressure. |
| F2 | **major** | `openspec/specs/component-shape-language/spec.md:321` | P1 `text-portal` delta (type + 8-part Part Model + readability enforcement) never synced to canonical spec nor archived; change dir 0/12 tasks; bead hud-2ey2w closed despite incomplete spec delivery. Canonical spec still knows only 6 component types. |
| F3 | **major** | `crates/tze_hud_config/src/portal_tokens.rs:106` | P2 (hud-8691s, closed) shipped a **59-key `portal.*` token namespace**, but the P1 delta's deferral scenario says P2 "MUST reference only pre-existing canonical keys" and that no `portal.*` key "MUST be present." Code contradicts the (unsynced) spec delta. |
| F4 | **major** | `crates/tze_hud_config/src/component_types.rs:121` | `text-portal` is not in `ComponentType::ALL`; readability enforcement (`component_startup.rs:290`) never validates portal parts; `PortalPartProto` carries no RenderingPolicy/readability field. The delta's "Text-Portal Readability Enforcement" SHALL has **zero runtime path**. |
| F5 | minor | `crates/tze_hud_scene/src/graph/queries.rs:49` | Dual-path retained: raw-tile assembly still paints pixels and drives drag-band hit-testing; adapter declares only 5/8 parts with empty backing nodes. Intentional/documented/tracked under hud-s4lrw, but hud-s4lrw is **closed** while this residual remains. |
| F6 | minor | `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py:95` | Exemplar `frame` default `#0000004D` (~0.30 α) is below the `OpaqueBackdrop` 0.8 threshold; latent — would fail readability once F4's enforcement path is wired. |
| F7 | minor | `crates/tze_hud_scene/src/graph/overlay.rs:565` | `clear_portal_surface` is an **unchecked** (non lease-gated) portal-surface mutator with **zero callers** — the exact checked/unchecked-sibling trap that bit the accent overlay before #1104. Delete or gate. |
| F8 | minor | (bead hygiene) | hud-9gyao is `in_progress` although its PR #1113 (`efd1e288`) is merged to `main` — a constituent bead left open; blocks a clean epic close. |
| F9 | minor | `crates/tze_hud_projection/src/resident_grpc.rs:831` | Unreachable raw-only `ReusePortalTile` arm of `ensure_portal_tile_message` (production always takes the create arm). Dead-code cleanup. |
| F10 | nit | `crates/tze_hud_protocol/proto/session.proto:859` | `SceneSnapshot.snapshot_json` doc "Includes:" list omits `portal_surfaces` (also `widget_registry`, `display_area`); payload carries it, only the comment is stale. |
| F11 | nit | `crates/tze_hud_projection/src/bin/projection_authority.rs:684` | Demo route still emits `"materialization": "resident_raw_tile"`; post-#1108 a portal is first-class-surface + raw-tile. Cosmetic label drift. |

No **blocking** findings surfaced. Nothing corrupts scene state, escapes lease governance in normal operation, or introduces an excluded-scope capability.

---

## Per-dimension analysis

### Dimension 1 — Spec-to-code (scene / surface contract): **spec-complete**

The scene-model half of the promotion maps cleanly to merged, tested code:

- **Eight-part model, one node per part.** `PortalPartKind` defines exactly the eight required variants with a canonical `ALL: [_;8]` (`crates/tze_hud_scene/src/types.rs:351`); each `PortalPart` carries `node: Option<SceneId>` (`types.rs:408`); `validate_structure` enforces ≤8 parts + no duplicate kind via bitset (`types.rs:472`). Wire mirror: `PortalPartKindProto` (8 values), `PortalPartProto.node` (`crates/tze_hud_protocol/proto/types.proto:353`). Geometry-only parts (divider/capture-backstop/gesture-shield) legitimately carry `node=None` per `is_text_bearing` (`types.rs:386`).
- **Reconnect parity (#1098).** `take_snapshot` copies `overlay.portal_surfaces` verbatim (`crates/tze_hud_scene/src/graph/snapshot.rs:58`); serde field with checksum back-compat (`types.rs:3082`); carried in `SceneSnapshot.snapshot_json` on connect (`session.proto:858`). Direct tests: `snapshot_carries_previously_declared_portal_surface` (`graph/tests.rs:437`), JSON round-trip with revalidation-nulled node ref (`tests.rs:457`), proto↔scene round-trip (`convert.rs:1972`).
- **§7.2 scope boundary held.** No terminal emulation, no scene-graph transcript history, no second stream. The schema materializes no transcript history (parts reference existing bounded-viewport nodes); rides the existing `MutationBatch` oneof (tags 14/15) with no new RPC/stream (`proto/types.proto:249`).

Residual (tracked, not a schema defect): the post-promotion `text-portal` component-type **styling** SHALL (`text-stream-portals/spec.md:518`) is unimplemented — see F4.

### Dimension 2 — Orphaned raw-tile-era special cases: **no orphans; dual-path is intentional and tracked**

- **`<empty projection stream>` sentinel fully removed** — survives only in doc comments describing its replacement, and tests now assert its **absence** (`resident_grpc.rs:4857,4900`). The token-styled empty state (`resident_grpc.rs:2234`) supersedes it.
- **No magic tile indices / `len()==6` sentinels** in production; "six-tile" appears only in doc comments describing the Phase-0 pilot.
- **Dual-path is by design.** `render_batch_with_surface` (the sole live render entry for both the in-process driver and the bridged transport, `resident_grpc.rs:875` / `portal_projection_driver.rs:1852-2322`) emits the first-class `SetPortalSurface`/`UpdatePortalSurfaceState` mutations **in addition to** raw-tile `PublishToTile` assembly. Both arms are pinned load-bearing by tests (`resident_grpc.rs:2811,2886`; `portal_projection_driver.rs:3904`).
- **But render-side per-part consumption DID land (#1099):** the renderer consumes per-part clip/scope envelopes (`renderer/text.rs:401,1350,1380`; `renderer/mod.rs:1602`). The remaining raw-tile dependence — empty backing nodes in adapter emission (`resident_grpc.rs:1356`) and drag-band anchoring off raw sibling tiles (`queries.rs:49`) — is the F5 residual. It is documented against hud-s4lrw, yet **hud-s4lrw is closed**, so this residual currently has no open owner.

### Dimension 3 — Design tokens / RenderingPolicy: **render path clean; spec reconciliation incomplete**

- **"Never hardcode visuals" upheld in the render path.** `renderer/text.rs`, `renderer/mod.rs`, `windowed/portal.rs` contain **zero** hardcoded hex/rgba/`Color::rgb` in non-test/comment code. Token keys are centralized named `pub const`s with a single-source-of-truth default module (`crates/tze_hud_config/src/portal_tokens.rs:106`); flow is `resolve_portal_tokens → PortalPartTokens → portal_visual_tokens_from_part_tokens → adapter` (`crates/tze_hud_runtime/src/portal_tokens.rs`). The only hardcoded compositor colors are generic HitRegion tints / missing-image placeholder (`renderer/tile_render.rs:2137`), which honor `local_style` overrides first — not portal styling.
- **Spec reconciliation is the gap** (F2/F3/F4): the canonical component-shape-language spec still enumerates only six component types, the P1 delta is unsynced/unarchived with 0/12 tasks, the shipped `portal.*` namespace contradicts the delta's deferral scenario, and the readability-enforcement requirement has no runtime path because `text-portal` is not a registered `ComponentType`.

### Dimension 4 — Doctrine (message classes + lease sovereignty): **lease-solid; one message-class gap**

- **Message-class declaration discipline is correct and tested.** `SetPortalSurface → Transactional` (structural, never evicted) and `UpdatePortalSurfaceState → StateStream` (coalescible) at both the protocol classifier (`traffic.rs:158,163`) and the runtime freeze mirror — the actual #1111 fix (`shell/freeze.rs:74`). Before #1111 a surface *declaration* could be evicted under freeze; now aligned.
- **Lease sovereignty holds for every model-driven mutation.** Enumerated and each gated with namespace + `require_active_lease` + `require_capability(ModifyOwnTiles)`: declare (`overlay.rs:494`), state patch (`overlay.rs:537`), part paint `set_tile_root_checked` (`tiles.rs:372`), composer node `add_node_to_tile_checked` (`tiles.rs:487`), accent overlay — the #1104 fix (`overlay.rs:427`), unread count (`overlay.rs:842`). The in-process apply path routes **all** arms through the checked variants (`convert.rs:1512`). Surface teardown is the lease-gated tile-removal/orphan path (`node_tree.rs:139`).
- **#1113 and #1006 correctly stay off the lease surface** — they are compositor render-side treatments of *local viewer* state (at-capacity draft strip, selection quads), consistent with "local feedback first / screen is sovereign"; #1113 even **removed** the dead zero-length `TextColorRunProto` wire sentinel. #1109 whole-portal resize is a chrome-layer viewer action, deliberately lease-bypassed but clamped by the lease's spatial budget (`windowed/portal.rs:479`).
- **The one gap is F1** — the composer hit-region `AddNode` re-emitted every render flips interaction-enabled streaming batches to Transactional, defeating StateStream coalescing exactly where it matters most (an LLM streaming output into an input-accepting portal). The project already routed the accent and unread-count mutations *around* this precise hazard for coalescing (hud-mzk74, see `resident_grpc.rs:1127`), but left the composer path unmitigated and untested. It degrades to bounded gRPC backpressure (not unbounded growth) and only bites the bridged transport under freeze, so it is major, not blocking.

---

## Verdict

**CLOSABLE WITH FOLLOW-UPS.**

The promotion is functionally complete and doctrine-solid on its two hardest guarantees: **lease sovereignty** (every model-driven surface mutation is namespace + active-lease + capability gated, including the two render-side PRs that correctly stay local) and **message-class declaration discipline** (the #1111 Transactional fix is real, mirrored in the freeze queue, and tested). The scene/surface schema, multi-node layout, and reconnect parity map to merged tested code with no orphaned raw-tile sentinels — the retained raw-tile path is a deliberate, test-pinned escape hatch, not migration debt.

The epic should **not** be closed as-is, but nothing found is blocking. Two items gate a *clean* close and the rest are tracked follow-ups:

**Gating (coordinator action before closing the epic):**
1. **Close hud-9gyao** — its PR #1113 is already merged to `main` (F8). A constituent left `in_progress` should not ride under a closed epic.
2. **Decide the owner for the F5 renderer-promotion residual** — hud-s4lrw is closed, yet adapter emission still declares empty backing nodes and drag-band hit-testing still keys off raw sibling tiles. This residual (and F4's readability-enforcement path) needs a *new* open bead; do not treat closed hud-s4lrw as covering it.

**Follow-ups (file as new beads; not filed by this report per instruction):**
- **[major] F1** — Emit the composer hit-region `AddNode` once at surface declaration (or route composer-interaction enablement through a coalescible state mutation, mirroring the accent/unread fix) so interaction-enabled streaming batches stay on the StateStream coalescible path; add the missing interaction-path regression test alongside the hud-mzk74 non-interactive ones.
- **[major] F2** — Run `openspec` sync/archive to land the `text-portal` delta into canonical `component-shape-language/spec.md` and move the change to `changes/archive/` (all sibling portal changes are archived; this one is not).
- **[major] F3** — Reconcile the shipped 59-key `portal.*` namespace against the P1 delta's "defer to P2, reuse canonical keys only" deferral scenario; when F2 syncs the delta, correct that now-false scenario to match what hud-8691s actually shipped.
- **[major] F4** — Register `text-portal` in `ComponentType::ALL` and wire per-part `OpaqueBackdrop`/`None` readability enforcement (add a RenderingPolicy/readability field to `PortalPartProto` or resolve it from backing nodes); tracked with the renderer promotion.
- **[minor] F6** — Raise the exemplar `frame` default opacity to ≥0.8 (or confirm the runtime canonical `#0A0D11` is authoritative) before F4's enforcement lands.
- **[minor] F7** — Delete or lease-gate the unchecked, caller-less `clear_portal_surface`.
- **[minor] F9 / nits F10, F11** — Dead-code and doc/label cleanup (unreachable reuse arm; stale `snapshot_json` comment; stale `resident_raw_tile` demo label).

---

*Reconciliation performed report-only; no runtime code was modified. All headline findings (F1 coalescing bypass, F2 spec-sync, F3 token divergence, F4 readability path, F7 dead unchecked mutator) were independently verified at the cited file:line by the reconciling worker in addition to the parallel dimension audits.*
