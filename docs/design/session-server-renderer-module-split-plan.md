# session_server.rs and renderer.rs Module Split Plan

**Issue**: hud-se14n (planning prep for hud-luovo)
**Date**: 2026-06-13
**Author**: agent/hud-se14n
**Status**: Executed — §3 (`session_server.rs`) split merged via hud-5bal5 (SS-1..SS-9);
§4 (`renderer.rs`) split also merged. Both files are now directory modules. This
document is retained as a historical record of the plan and its seams.

**Plan re-verification (2026-06-15, hud-uwvid)**: Each §3 seam was re-verified by
banner text (not line number) against the as-executed submodules in
`crates/tze_hud_protocol/src/session_server/`. Corrections applied — see the
inline **[hud-uwvid 2026-06-15]** notes in §3.1, §3.2, §3.3, and §3.4. The headline
fix: the plan's `ephemeral.rs ← EphemeralQueue` step was **phantom** — no
`EphemeralQueue` struct, no `// ─── Ephemeral send buffer ──` banner, and no
`ephemeral.rs` file ever existed in the real `session_server.rs` (the plan was
drafted against a misread of an older file state). `git grep "EphemeralQueue"`
and `git grep "Ephemeral send buffer"` both return zero hits across the codebase.
All references to it have been removed.

---

## 1. Purpose

`session_server.rs` (16,868 lines) and `renderer.rs` (18,140 lines) are the two
largest files in the codebase and the primary merge-conflict hotspots (~33–35
commits per last 500 in git log). This document plans splitting each file into
submodules along their existing section-banner seams, targeting ≤ 4 k production
lines per file post-split with mechanical (move-only) commits that leave all
observable behaviour unchanged.

This is **planning only** — no Rust code is changed in this document.

---

## 2. Guiding Principles

1. **Move-only commits**: no logic changes in any split commit. Reviewers should
   be able to verify with `diff -u old.rs submodule/*.rs` that nothing was added
   or deleted — with the explicit exception of **visibility modifiers**. Items
   that were implicitly private to a single file become `pub(super)` (visible to
   the parent module and its children) or `pub(crate)` when moved into a child
   module and called from a sibling or the parent `mod.rs`. These are the minimal
   mechanical additions required by Rust's module privacy rules and are expected
   in every split commit. Execution PRs must list which items gained
   `pub(super)`/`pub(crate)` in their PR description.
2. **API preservation via `pub use`**: callers must not need to update import
   paths. The parent module re-exports everything from each submodule.
3. **One submodule per commit** (or tightly coupled pair): keeps each PR
   reviewable in isolation.
4. **Tests move with their host**: the massive `mod tests { ... }` blocks in both
   files move to a `tests/` subdirectory as a final step.
5. **Prerequisite beads first**: hud-4eobj (mutation clone dedup),
   hud-8uafa (frame pipeline dedup), hud-r5q6p (zone publish dedup) must be
   closed before execution. Splitting duplicated logic into separate modules
   would scatter the subsequent dedup diff.
6. **Line numbers are approximate**: `git rebase origin/main` before each commit
   because both files receive frequent edits. Treat line ranges as directional
   guides, not absolute addresses.

---

## 3. `session_server.rs`

**Location (at plan date)**: `crates/tze_hud_protocol/src/session_server.rs` (monolith)
**Location (as executed)**: `crates/tze_hud_protocol/src/session_server/` (directory module — already split)
**Lines (at plan date)**: 16,868
**Production lines (excluding tests)**: ≈ 7,258 (L1–7257)
**Test lines**: ≈ 9,610 (L7258–16867)

### 3.1 Verified Section-Banner Seams

> **[hud-uwvid 2026-06-15]** Line numbers below are the plan-date (pre-split)
> positions and are now stale. Each seam is re-verified by banner TEXT against the
> as-executed submodule that received it. The "As-executed location" column records
> where each banner/struct actually landed (verified with `git grep`).

| # | Approx. line | Banner text | Contents | As-executed location |
|---|---|---|---|---|
| 1 | L63 | `// ─── Session Configuration ───` | `SessionConfig` struct (53 lines) | `config.rs` (banner intact) |
| 2 | L116 | `// ─── Session Lifecycle State Machine ──` | `SessionState` enum (41 lines) | `lifecycle.rs` (banner intact) |
| 3 | L157 | `// ─── Traffic Class ───` | `TrafficClass` enum + `classify_server_payload` fn (105 lines) | `traffic.rs` L1 (banner intact) |
| 4 | L262 | `// ─── Inbound mutation traffic class ──` | `InboundTrafficClass` + `classify_inbound_batch` fn (56 lines) | `traffic.rs` L109 (banner intact) |
| 5 | L318 | `// ─── Per-session freeze queue ──` | `SessionFreezeQueue`, `FrozenMutation`, `FreezeEnqueueResult` (205 lines) | `freeze_queue.rs` L15 (banner intact) |
| 6 | L560 | `// ─── Constants ───` | Heartbeat/sequence constants (19 lines) | `mod.rs` L111 (banner intact) |
| 7 | L579 | `// ─── Helper ──` | `process_start`, time helpers, scene-id helpers, element store helpers (369 lines) | `mod.rs` L130 (banner intact) |
| 8 | L948 | `// ─── Upload state types ──` | Upload state types (`ResidentUploadState`, `UploadByteRateLimiter`, `UploadWorkerCommand`, `UploadWorkerEvent`) + `run_upload_worker` fn (465 lines) | `upload.rs` L20 + L479 `// ─── Upload helper functions ──` |
| 9 | L1413 | `// ─── Session state ──` | `StreamSession` struct with all per-session fields (155 lines) | `stream_session.rs` L15 (banner intact) |
| 10 | L1568 | `// ─── Capability Revocation Event ──` | `CapabilityRevocationEvent` struct (21 lines) | `stream_session.rs` L163 (banner intact) |
| 11 | L1589 | `// ─── Service implementation ──` | `HudSessionImpl` struct, constructors, runtime methods, `async fn session` (618-line tokio::select loop) (1,004 lines) | `service.rs` L25 (struct/methods) + `mod.rs` L413 (session loop) |
| 12 | L2593 | `// ─── Handshake handlers ──` | `authorization_scope_for_agent`, `handle_session_init`, `handle_session_resume` (395 lines) | `handshake.rs` (no banner; functions present) |
| 13 | L2988 | `// ─── Message handlers ──` | Dispatcher + all `handle_*` functions — see sub-breakdown below (4,144 lines) | `mod.rs` L775 (dispatcher) + per-domain handler submodules |
| 14 | L7132 | `// ─── Agent Scene Event Emission handler ──` | `handle_emit_scene_event`, `validate_emission` (126 lines) | `emit_scene_event.rs` L8 (banner intact) |
| 15 | L7258 | `// ─── Tests ──` | `mod tests { ... }` (9,610 lines) | `tests.rs` (banner intact) |

> **[hud-uwvid 2026-06-15] Seam corrections vs. the original plan table:**
> - **Removed phantom row 6** (`// ─── Ephemeral send buffer ──` / `EphemeralQueue`,
>   37 lines): this banner and struct do not exist anywhere in the codebase
>   (`git grep "EphemeralQueue"` → 0 hits; `git grep "Ephemeral send buffer"` → 0
>   hits). No `ephemeral.rs` file was created by the executed split. The plan's
>   §3.4 SS-1 had listed `ephemeral.rs ← EphemeralQueue` as a fourth move — it was
>   never executable. Rows below it were renumbered (old 7→6, 8→7, …, 16→15).
> - **Corrected banner-9 text** (old row 9): the plan cited `// ─── Shared agent
>   event emission types ──` for the upload-worker block, but the upload types
>   actually live under `// ─── Upload state types ──` (now `upload.rs` L20). The
>   `// ─── Shared agent event emission types ──` banner is a *separate* header
>   that remained in `mod.rs` (L397) for a different block; it was never the
>   upload seam. Corrected to the real banner text.
> - **Banner-12 (`// ─── Handshake handlers ──`)**: the executed `handshake.rs`
>   does not carry this banner comment (the functions moved without a section
>   header). Anchor on the function names `handle_session_init` /
>   `handle_session_resume` instead of the banner.

**Sub-breakdown of banner 13 (`Message handlers`, L2988–7131)**

These sub-regions have no explicit banners but are clearly delimited by handler
function groups:

| Sub-region | Approx. lines | Contents |
|---|---|---|
| Dispatcher | L2988–3387 | `handle_client_message` match arms routing to handlers (plan earlier called this `dispatch_message`; actual name is `handle_client_message`) |
| Media ingress | L3388–3639 | `handle_media_ingress_open`, `handle_media_ingress_close` |
| Interaction | L3640–4067 | Input/gesture event handlers |
| Mutation batch | L4068–4720 | `handle_mutation_batch` (~653 lines, biggest single handler) |
| Lease | L4721–5288 | `handle_lease_request`, `handle_lease_renew`, `handle_lease_release` |
| Subscription/capability | L5289–5771 | `handle_subscription_change`, `handle_capability_request/revocation`, `handle_list_elements_request` |
| Zone publish | L5772–6212 | `handle_zone_publish` |
| Widget/asset | L6213–7038 | `handle_widget_asset_register`, `handle_widget_publish` |
| Input | L7039–7131 | Remaining input handlers |

### 3.2 Proposed Submodule Breakdown

Target directory: `crates/tze_hud_protocol/src/session_server/`

Parent module (`session_server.rs`) becomes a directory module
(`session_server/mod.rs`) with re-exports:

```
session_server/
├── mod.rs               # pub use * from each submodule; HudSessionImpl::session() stays here
├── config.rs            # SessionConfig (banner 1)
├── lifecycle.rs         # SessionState enum (banner 2)
├── traffic.rs           # TrafficClass, InboundTrafficClass, classifiers (banners 3–4)
├── freeze_queue.rs      # SessionFreezeQueue, FrozenMutation, FreezeEnqueueResult (banner 5)
├── upload.rs            # Upload state types + run_upload_worker (banner 8)
├── stream_session.rs    # StreamSession struct + CapabilityRevocationEvent (banners 9–10)
├── service.rs           # HudSessionImpl struct + constructors + runtime helpers (banner 11, minus session loop)
├── handshake.rs         # handle_session_init, handle_session_resume (banner 12)
├── mutations.rs         # handle_mutation_batch (from banner 13)
├── leases.rs            # handle_lease_* (from banner 13)
├── media.rs             # handle_media_ingress_* (from banner 13)
├── zone_publish.rs      # handle_zone_publish (from banner 13)
├── widgets.rs           # handle_widget_asset_register, handle_widget_publish (from banner 13)
├── input.rs             # Input/interaction handlers (from banner 13)
├── subscriptions_cap.rs # handle_subscription_change, handle_capability_* (from banner 13)
├── emit_scene_event.rs  # handle_emit_scene_event, validate_emission (banner 14)
└── tests.rs             # existing mod tests { ... } content (banner 15; executed as tests.rs, not tests/mod.rs)
```

**Line counts (approximate post-split production lines in mod.rs)**:
- `mod.rs`: constants (L560–578, 19 lines) + helpers (L579–947, 369 lines) +
  `async fn session` loop (~618 lines) + `pub use` stubs ≈ 1,100 lines
- All other submodules: each ≤ 700 lines
- `mutations.rs`: ~653 lines (largest submodule)

### 3.3 Cross-Section Coupling

These coupling points complicate the split and require careful import ordering:

| Coupling | Where used | Mitigation |
|---|---|---|
| `StreamSession` struct | Every handler in banners 12–14 borrows or mutates it | Move to `stream_session.rs` first; it has no dependencies on handlers — all handler files import from `stream_session.rs` |
| `SharedState` (`Arc<Mutex<SharedState>>`) | All handler functions receive it as parameter | Defined outside `session_server.rs` (in `session.rs`); no change needed — handlers import from `crate::session` |
| Constants (L560–578) | `classify_server_payload`, `run_upload_worker`, `session` loop | Keep in `mod.rs` (19 lines, not worth a dedicated file); each submodule imports via `super::CONST_NAME` |
| Helper functions (L579–947) | Spread across handshake, mutation, zone, widget handlers | Keep in `mod.rs` initially; after split, measure which helpers are used by only one submodule and migrate them if cleaner |
| `SessionFreezeQueue` | `handle_mutation_batch` primarily | Move to `freeze_queue.rs`; only `mutations.rs` needs it after split |
| `run_upload_worker` spawned in `session` loop | Banner 8 (`Upload state types`) defined, banner 11 (`Service implementation`) used | `upload.rs` exports it; `mod.rs` imports it |
| `async fn session` 618-line loop | Central dispatch — 15+ `on_*` arms | **Do not split this loop in the initial cut.** Leave in `mod.rs`. A subsequent refactor can extract each arm as an `on_*` helper method on `HudSessionImpl` |
| Test helpers | `mod tests` uses types from every banner | Move tests last; the test module imports everything via `super::*` |

### 3.4 Incremental Sequencing

Perform one step per PR. Each step is a pure move with no logic changes.

**Step SS-1: Leaf types (no deps on each other)**
Move simultaneously (single PR):
- `config.rs` ← `SessionConfig` (L63–115)
- `lifecycle.rs` ← `SessionState` (L116–156)
- `traffic.rs` ← `TrafficClass`, `InboundTrafficClass`, classifiers (L157–317)

Add `pub use` in `mod.rs` for all moved types.

> **[hud-uwvid 2026-06-15]** The original SS-1 listed a fourth move,
> `ephemeral.rs ← EphemeralQueue (L523–559)`. **Removed** — `EphemeralQueue`, the
> `// ─── Ephemeral send buffer ──` banner, and any `ephemeral.rs` file never
> existed in the real `session_server.rs` (`git grep` → 0 hits). The as-executed
> SS-1 moved only the three leaf types above. The freeze-queue seam (SS-2) followed
> directly after `traffic.rs` in the pre-split file (`// ─── Per-session freeze
> queue ──`, now `freeze_queue.rs`), with the `// ─── Constants ──` banner
> immediately after it — there was no ephemeral seam between them.

**Step SS-2: Freeze queue**
- `freeze_queue.rs` ← `SessionFreezeQueue`, `FrozenMutation`, `FreezeEnqueueResult` (L318–522)

Depends on `traffic.rs` being in place (FreezeEnqueueResult references TrafficClass).

**Step SS-3: Upload worker**
- `upload.rs` ← upload state types + `run_upload_worker` (L948–1412)

**Step SS-4: Session state types**
- `stream_session.rs` ← `StreamSession` + `CapabilityRevocationEvent` (L1413–1588)

This is the most import-sensitive step — every subsequent submodule depends on
`StreamSession`. Merge and verify CI before proceeding.

**Step SS-5: Service struct**
- `service.rs` ← `HudSessionImpl` struct + `new()` constructors + non-session runtime methods (L1589–2592 minus `async fn session`)
- `async fn session` stays in `mod.rs`

**Step SS-6: Handshake handlers**
- `handshake.rs` ← `authorization_scope_for_agent`, `handle_session_init`, `handle_session_resume` (L2593–2987)

**Step SS-7: Message handler extraction (one PR per sub-region)**

Each of the following is a separate PR (can be done in parallel once SS-6 is merged):

- SS-7a: `mutations.rs` ← `handle_mutation_batch` (L4068–4720)
- SS-7b: `leases.rs` ← lease handlers (L4721–5288)
- SS-7c: `media.rs` ← media ingress handlers (L3388–3639)
- SS-7d: `zone_publish.rs` ← zone publish handler (L5772–6212)
- SS-7e: `widgets.rs` ← widget/asset handlers (L6213–7038)
- SS-7f: `input.rs` ← input/interaction handlers (L3640–4067, L7039–7131)
- SS-7g: `subscriptions_cap.rs` ← subscription/capability handlers (L5289–5771)
- SS-7h: `emit_scene_event.rs` ← emit scene event + validate (L7132–7257)

The dispatcher remains in `mod.rs` and is trimmed as each handler moves — the
match arm calls the function via module path.

> **[hud-uwvid 2026-06-15]** The dispatcher function is named `handle_client_message`
> (`mod.rs` L777, under the `// ─── Message handlers ──` banner at L775), not
> `dispatch_message` as the plan body and §3.1 sub-breakdown say. Anchor on
> `handle_client_message` when locating it.

**Step SS-8: Tests**
- `tests.rs` ← entire `mod tests { ... }` block (L7258–16867). *(Executed as
  `session_server/tests.rs`, not `tests/mod.rs`.)*

**Step SS-9: Helper migration (optional, follow-on)**
After the split is stable, audit which helpers in `mod.rs` (L579–947) are used
by only one submodule. Migrate those to reduce `mod.rs` to purely the session
loop and dispatcher.

### 3.5 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Circular imports between submodules | Medium | Enforce strict ordering: leaf types → shared state types → service → handlers. No handler imports another handler. |
| `async fn session` loop references 10+ types | High (it already does) | Leave loop in `mod.rs` until after SS-7 is complete; then optionally split arms into `on_*` methods in a follow-on task |
| `handle_mutation_batch` (653 lines) itself needs a dedup cleanup (hud-4eobj) | High | Do NOT split it before hud-4eobj closes — the dedup diff will be cleaner against a single file |
| Merge conflicts during parallel SS-7 PRs | Medium | CI will catch them; coordinate with other workers to not edit the same message handler region concurrently |
| Test file imports become deeply namespaced | Low | Use `use super::super::*` or workspace-level `use crate::session_server::*` — the test module is internal |
| Line numbers drift before execution | High (these files are hot) | Use section banner text as the anchor, not line numbers; verify with `grep -n "^// ───"` before each PR |

---

## 4. `renderer.rs`

**Location**: `crates/tze_hud_compositor/src/renderer.rs`
**Lines (at plan date)**: 18,140
**Production lines (excluding tests)**: ≈ 9,803 (L1–8336)
**Test lines**: ≈ 9,804 (L8337–18140)

### 4.1 Verified Section-Banner Seams

`renderer.rs` has only 6 explicit section banners. The logical structure is
primarily encoded by the `impl Compositor { ... }` method group progression
rather than banner comments.

| # | Approx. line | Banner text | Contents |
|---|---|---|---|
| 1 | L43 | `// ─── Severity token fallback colors ──` | `SEVERITY_*` color constants (27 lines) |
| 2 | L70 | `// ─── Notification urgency token fallback colors ──` | Urgency color constants + helper fns (156 lines) |
| 3 | L226 | `// ─── Tile background token fallback colors ──` | Tile BG token constants + free color fns (518 lines) |
| 4 | L744 | `// ─── Notification icon helpers ──` | `parse_notification_icon` fn (19 lines) |
| 5 | L763 | `// ─── Image fit mode UV calculations ──` | `TexturedDrawCmd`, `VideoFrameDrawCmd`, `DragHandleEntry`, `compute_fit_mode` (388 lines) |
| 6 | L1151 | `// ─── Image texture cache ──` | `ImageTextureEntry`, `LocalComposerState`; `Compositor` struct begins at L1251 |

**Logical method-group clusters within `impl Compositor` (no explicit banners)**

| Cluster | Approx. lines | Methods |
|---|---|---|
| Compositor struct | L1251–1524 | `pub struct Compositor` (65 fields) + `ZoneSlotLayout` |
| Constructors | L1525–2166 | `new_headless`, `new_windowed`, `new_windowed_inner`, pipeline creation |
| Token/cache setup | L2167–2406 | `set_token_map`, `drain_local_composer_state`, `prime_markdown_cache`, `prime_truncation_cache` |
| Font/image loading | L2407–3335 | `init_text_renderer`, `load_font_bytes`, `register_image_bytes`, `ensure_image_texture`, `ensure_icon_texture`, `evict_unused_image_textures` |
| Video surface | L3336–3545 | `handle_media_event`, `upload_video_frame`, `evict_video_frame_texture`, `collect_video_frame_cmds`, `encode_video_frame_pass`, `init_widget_renderer`, `sync_widget_textures` |
| Text collection | L3546–4287 | `collect_text_items` (~L3546, 751-line method), `status_bar_icon_text_items`, `collect_text_items_from_node` |
| Animation state | L4288–5095 | `update_zone_animations`, `update_portal_tile_animations`, `update_stream_reveals`, `update_publication_animations`, `prune_faded_publications` |
| Frame vertices | L5096–5295 | `build_frame_vertices` |
| Render entry points | L5296–5849 | `render_frame`, `render_frame_headless`, `render_frame_with_chrome` |
| Color helpers + encode passes | L5850–6065 | Color helper fns, `encode_widget_pass`, `encode_drag_handle_pass`, `encode_image_pass` |
| Rounded rect | L6066–6380 | `collect_all_rounded_rect_cmds`, `encode_rounded_rect_pass` |
| Zone rendering | L6381–6880 | `render_zone_content` |
| Zone layout | L6881–7083 | `collect_sorted_status_bar_entries`, `per_slot_heights`, `zone_slot_layout`, `slot_offsets`, `resolve_zone_geometry` |
| Widget/drag geometry | L7084–7345 | `resolve_widget_geometry`, `drag_handle_bounds`, `collect_drag_handle_entries`, `append_drag_handle_vertices` |
| Hit regions | L7346–7899 | `populate_drag_handle_hit_regions`, `populate_zone_hit_regions` |
| Tile rendering | L7900–8213 | `render_composer_overlay`, `render_node` |
| Module-level helpers | L8214–8336 | `collect_ellipsis_text_items_from_node`, `resolve_widget_pixel_size` |
| Tests | L8337–18140 | `mod tests { ... }` (9,804 lines) |

### 4.2 Proposed Submodule Breakdown

Target directory: `crates/tze_hud_compositor/src/renderer/`

Parent module (`renderer.rs`) becomes a directory module
(`renderer/mod.rs`) with re-exports:

```
renderer/
├── mod.rs               # pub use * from each submodule; Compositor struct stays here
├── token_colors.rs      # banners 1–3: severity/urgency/tile color constants + free color fns (L43–743)
├── icon.rs              # banner 4: parse_notification_icon (L744–762)
├── draw_cmds.rs         # banner 5: TexturedDrawCmd, VideoFrameDrawCmd, DragHandleEntry, compute_fit_mode (L763–1150)
├── image_cache.rs       # banner 6 + image loading cluster: ImageTextureEntry, LocalComposerState, ensure_image_texture, ensure_icon_texture, evict_unused_image_textures (L1151–1250 + L2407–3335)
├── video.rs             # video surface cluster: handle_media_event, upload_video_frame, evict_video_frame_texture, collect_video_frame_cmds, encode_video_frame_pass, init_widget_renderer, sync_widget_textures (L3336–3545)
├── text.rs              # text collection cluster: collect_text_items, status_bar_icon_text_items, collect_text_items_from_node (L3546–4287)
├── animation.rs         # animation state cluster: update_zone_animations, update_portal_tile_animations, update_stream_reveals, update_publication_animations, prune_faded_publications (L4288–5095)
├── frame.rs             # frame vertices + render entry points: build_frame_vertices, render_frame, render_frame_headless, render_frame_with_chrome (L5096–5849)
├── encode_pass.rs       # rounded rect + encode passes: encode_widget_pass, encode_drag_handle_pass, encode_image_pass, collect_all_rounded_rect_cmds, encode_rounded_rect_pass (L5850–6380)
├── zone_render.rs       # zone rendering + layout: render_zone_content, collect_sorted_status_bar_entries, per_slot_heights, zone_slot_layout, slot_offsets, resolve_zone_geometry (L6381–7083)
├── widget_geometry.rs   # widget/drag geometry: resolve_widget_geometry, drag_handle_bounds, collect_drag_handle_entries, append_drag_handle_vertices (L7084–7345)
├── hit_regions.rs       # hit region population: populate_drag_handle_hit_regions, populate_zone_hit_regions (L7346–7899)
├── tile_render.rs       # tile/node rendering: render_composer_overlay, render_node (L7900–8213)
└── tests/
    └── mod.rs           # existing mod tests { ... } content (L8337–18140)
```

**`mod.rs` retains**:
- `use` imports for wgpu, winit, tonic, etc.
- `pub struct Compositor { ... }` with all 65 fields (L1251–1524)
- `ZoneSlotLayout` (L1524, companion to struct)
- Constructors: `new_headless`, `new_windowed`, `new_windowed_inner`, pipeline creation (L1525–2166) — these construct `Compositor` in one shot and cannot be cleanly separated from the struct definition in phase 1
- Token/cache setup: `set_token_map`, `drain_local_composer_state`, `prime_markdown_cache`, `prime_truncation_cache` (L2167–2406)
- `pub use` for everything in submodules

**Approximate `mod.rs` size post-split**: ≈ 1,350 lines (imports + struct + constructors + token setup + pub use)

### 4.3 Cross-Section Coupling

| Coupling | Description | Mitigation |
|---|---|---|
| `Compositor` struct (65 fields) | Every method in every cluster borrows `&self` or `&mut self` — the struct is the root coupling | Keep the struct in `mod.rs`; submodule methods are `impl Compositor` blocks defined in submodule files via Rust's split impl pattern (`impl Compositor { ... }` can appear in multiple files in the same module) |
| `collect_text_items` (751 lines) | Calls `collect_text_items_from_node` and `status_bar_icon_text_items` in same cluster | All three move together to `text.rs` — no cross-submodule call needed |
| `render_zone_content` calls into text + encode | `render_zone_content` in `zone_render.rs` calls `collect_text_items` from `text.rs` and encode helpers from `encode_pass.rs` | Standard Rust imports: `zone_render.rs` imports `use super::text::*; use super::encode_pass::*;` |
| `render_frame` orchestration | `render_frame` calls into zone, tile, hit-region, encode, animation clusters | These calls become cross-submodule imports; no circular dependency as long as `frame.rs` imports from the others (frame is the terminal, others are leaf) |
| `LocalComposerState` used in `drain_local_composer_state` | Defined in `image_cache.rs` but drained in `mod.rs` | Either move `drain_local_composer_state` to `image_cache.rs` (preferred, step R-4) or import `LocalComposerState` from `image_cache.rs` into `mod.rs` |
| Color constants used file-wide | `SEVERITY_*`, urgency, tile BG constants referenced across multiple methods | Move to `token_colors.rs`; all submodules import via `use super::token_colors::*` |
| `DragHandleEntry` used in both geometry and hit regions | Defined in `draw_cmds.rs`; `widget_geometry.rs` and `hit_regions.rs` both use it | Both import from `draw_cmds.rs` — no circular dep |
| `TexturedDrawCmd`/`VideoFrameDrawCmd` used in video, zone, frame | Defined in `draw_cmds.rs` | All submodules import from `draw_cmds.rs` — `draw_cmds.rs` is a pure-leaf module with no deps on other submodules |
| `hud-8uafa` frame pipeline dedup | `collect_video_frame_cmds` and `build_frame_vertices` are targeted by this dedup | Do not split video/frame clusters before hud-8uafa closes |
| `hud-r5q6p` zone publish dedup | `render_zone_content` is affected | Do not split `zone_render.rs` before hud-r5q6p closes |

### 4.4 Incremental Sequencing

**Step R-1: Pure free-function modules (single PR, no struct deps)**
Move:
- `token_colors.rs` ← L43–743 (severity/urgency/tile color constants + free color fns)
- `icon.rs` ← L744–762 (`parse_notification_icon`)
- `draw_cmds.rs` ← L763–1150 (`TexturedDrawCmd`, `VideoFrameDrawCmd`, `DragHandleEntry`, `compute_fit_mode`, animation state types like `ZoneAnimationState`, `PublicationAnimationState`, `StreamRevealState`)

These have no `impl Compositor` methods and no deps on each other except that
draw command types reference animation state types — include animation state
types in `draw_cmds.rs` or make it a separate `anim_types.rs` first.

**Step R-2: Image cache + texture types**
- `image_cache.rs` ← `ImageTextureEntry`, `LocalComposerState`, and the image loading methods (`ensure_image_texture`, `ensure_icon_texture`, `evict_unused_image_textures`, `register_image_bytes`, `load_font_bytes`, `init_text_renderer`)

`LocalComposerState` is defined just before `Compositor`; moving it to
`image_cache.rs` removes it from `mod.rs`, so `drain_local_composer_state` in
`mod.rs` must import `LocalComposerState` from `image_cache.rs`.

**Step R-3: Video surface**
- `video.rs` ← video surface methods (L3336–3545)

Wait for hud-8uafa (frame pipeline dedup) to close first.

**Step R-4: Text collection**
- `text.rs` ← text collection cluster (L3546–4287)

`collect_text_items` is the 751-line method; move all three text methods together.
Also move `collect_ellipsis_text_items_from_node` from module-level helpers
(L8214–8336) here since it's textually related.

**Step R-5: Animation state**
- `animation.rs` ← animation update methods (L4288–5095)

**Step R-6: Encode passes and rounded rect**
- `encode_pass.rs` ← encode passes + rounded rect (L5850–6380)

**Step R-7: Zone rendering and layout**
- `zone_render.rs` ← zone render + zone layout methods (L6381–7083)

Wait for hud-r5q6p (zone publish dedup) to close first.

**Step R-8: Widget geometry and hit regions (parallel PRs)**
- `widget_geometry.rs` ← widget/drag geometry (L7084–7345)
- `hit_regions.rs` ← hit region population (L7346–7899)

**Step R-9: Tile rendering**
- `tile_render.rs` ← tile/node rendering (L7900–8213)

**Step R-10: Frame vertices + render entry points**
- `frame.rs` ← `build_frame_vertices` + `render_frame*` (L5096–5849)

This is done last among production code because `render_frame` calls into all
other clusters and will import from nearly every submodule.

**Step R-11: Tests**
- `tests/mod.rs` ← `mod tests { ... }` (L8337–18140)

**Step R-12 (optional): Constructor extraction**
After R-1 through R-11, `mod.rs` will contain only the `Compositor` struct,
constructors, token setup, and pub use. If constructors are >500 lines, extract
them to `constructors.rs` as a follow-on.

### 4.5 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `impl Compositor` blocks in multiple files | Medium | This is valid Rust (split impl); ensure all blocks are within the same module (`renderer`). Rustc enforces this — the compiler catches it immediately. |
| `render_frame` imports from 8+ submodules | High (it calls everything) | This is fine — it's the terminal aggregation point. Make `frame.rs` the last production code split so all its deps are already in place. |
| `collect_text_items` 751 lines itself needs dedup (4 near-identical TextItem literals) | Medium | File a follow-on task (see Section 6). Do not modify during the split. |
| Color constants are referenced by multiple submodules | High | `token_colors.rs` is the first split (R-1) so all other submodules can `use super::token_colors::*` once it's in place. |
| `LocalComposerState` straddles image cache and mod.rs | Medium | Move it to `image_cache.rs` in R-2 and update the `drain_local_composer_state` import in `mod.rs`. |
| Line numbers drift | High | Use method names as anchors, not line numbers. Verify with `rg "fn collect_text_items"` etc. before each PR. |
| hud-8uafa / hud-r5q6p not closed before R-3/R-7 | Medium | Enforce with bead blockers on the execution tasks. |
| Constructor cluster (L1525–2166, ~640 lines) uses private helpers defined later | Low | Rust doesn't care about definition order within a module; helpers can stay in `mod.rs` or move to `constructors.rs` without issue. |

---

## 5. Execution Prerequisites (Before Any Split PR)

1. **Close hud-4eobj** (mutation clone dedup) — affects `handle_mutation_batch`
2. **Close hud-8uafa** (frame pipeline dedup) — affects video/frame clusters in renderer
3. **Close hud-r5q6p** (zone publish dedup) — affects `render_zone_content` and `handle_zone_publish`
4. **Rebase** `agent/hud-se14n` (and any execution branch) on `origin/main` before each PR — both files receive frequent edits
5. **Verify section banners** with `grep -n "^// ─"` immediately before starting each step — line numbers will have shifted

---

## 6. Discovered Follow-Ups

These are separate tasks, not part of the mechanical split:

| Bead candidate | Description |
|---|---|
| session_server: extract `async fn session` arms | After SS-7, the 618-line tokio::select loop can have each arm extracted as an `on_*` helper method on `HudSessionImpl`. This is a logic refactor (not mechanical), so it must be a separate task with its own test verification. |
| session_server: migrate helpers post-split | After SS-9, audit which helpers in `mod.rs` (L579–947) are used by only one submodule and migrate them there. |
| renderer: `collect_text_items` dedup | The 751-line method has 4 near-identical 25-field `TextItem` literal blocks. Extract a builder or macro. |
| renderer: `Compositor` struct decomposition | Long-term: replace the 65-field monolith with owned sub-structs (e.g., `CompositorRenderState`, `CompositorTextureCache`). This is architecturally significant and out of scope for the mechanical split. |

---

## 7. Acceptance Criteria Checklist

Per hud-se14n:

- [x] Split plan with module boundaries and migration order written and reviewed (this document)
- [x] `session_server.rs` production lines ≤ 4 k post-split — **done** (`session_server/mod.rs` is 1,569 lines as of 2026-06-15; SS-1..SS-9 merged via hud-5bal5)
- [x] `renderer.rs` production lines ≤ 4 k post-split — **done** (`renderer/mod.rs` is 1,728 lines as of 2026-06-15; R-1..R-12 merged)
- [x] All splits are mechanical move-only commits (verifiable by diff); each PR description lists items that gained `pub(super)` or `pub(crate)` visibility as part of the move
- [x] Test suite green after each step (no behavior change)
- [x] Churn hotspot concentration measurably reduced (each submodule sees commits only when its domain changes)

> **[hud-uwvid 2026-06-15]** All items checked. The plan has been fully executed.
> This bead (hud-uwvid) audited only the **§3 (`session_server.rs`) seams** against
> the as-executed submodules, per its scope. The §4 (`renderer.rs`) seam text was
> not re-verified line-by-line here, but the renderer split is confirmed merged
> (the `renderer/` directory module exists with the planned submodules).
