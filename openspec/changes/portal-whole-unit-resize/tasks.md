# Tasks — Portal Whole-Unit Resize

This change is **spec ahead of code**: the delta clarifies that a portal moves
and resizes as a coherent unit. It stays OPEN until the implementation lands and
is verified on the reference hardware, then it is synced + archived.

## 1. Contract and review

- [x] 1.1 Validate this change with `openspec validate portal-whole-unit-resize --strict`
- [x] 1.2 Confirm doctrine alignment: "screen is sovereign", "local feedback first", "one scene model, two profiles" (CLAUDE.md, RFC 0013 §4.1/§4.2)
- [x] 1.3 Confirm the delta adds no new transport/RPC and does not change focus scoping, the resize step/clamp/lease-budget contract, the release-only key-up fallback, or the adapter-cannot-override-gesture rule

## 2. Implement — runtime portal-group-aware resize (`hud-fb3en`)

- [x] 2.1 Give the runtime a way to enumerate a portal's constituent surfaces. Chose neither `sync_group` (real timing/commit semantics — reuse would perturb `evaluate_sync_group_commit`) nor a new `Tile.portal_group_id` field (needs a protocol + producer change the live client would not populate). Instead `resolve_portal_group` (`portal.rs`) resolves the group structurally from existing scene state: the tiles sharing the focused surface's lease, with the largest-area member as the frame/anchor and spatial containment excluding the far-corner drag shield. Works on the live client-created exemplar with zero protocol change; a single-tile lease resolves to a one-member group (backward compatible). Swappable if the epic `hud-g1ena` adds an explicit group field later.
- [x] 2.2 In `apply_portal_resize_hotkey` and `apply_portal_resize_pointer_event` (`crates/tze_hud_runtime/src/windowed/portal.rs`): resolve the portal anchor (frame surface), clamp against the lease budget on the whole-portal rect, scale ALL member surfaces around the top-left anchor preserving relative layout (`scale_portal_members`/`commit_portal_group_resize`), broadcast geometry per member (hotkey path + `lifecycle.rs` pointer path). Mirrors the client-side `portal_bounds_mutations` grouping pattern.
- [x] 2.3 Test: `ctrl_resize_hotkey_scales_whole_portal_not_just_focused_surface`, `pointer_affordance_resize_scales_whole_portal_from_frame`, and `ctrl_resize_hotkey_ignored_when_focused_surface_is_not_a_portal` — focusing the composer and applying Ctrl+= (and the pointer affordance) grows the whole portal as a unit, preserving relative layout, anchored top-left; the drag shield stays put; non-portal surfaces are unaffected.

## 2b. Implement — resize text scaling (`hud-ovjxu.1`, §Portal Resize Text Scaling)

- [x] 2b.1 Viewer-local per-tile font-scale multiplier: `RuntimeOverlayState.tile_font_scale` (`crates/tze_hud_scene/src/graph/overlay.rs`) with `tile_font_scale`/`set_tile_font_scale`/`clear_tile_font_scale`; cleared on tile removal (`node_tree.rs`). Chosen over mutating the stored `font_size_px` so an adapter content republish (which carries the base font) can never reset the viewer's zoom — font scaling stays viewer-local presentation and never alters adapter-published content (spec requirement + doctrine). Contrast the `viewer_geometry_locked` lock, which protects bounds; a stored-font mutation would need an equivalent lock and lose on every content update.
- [x] 2b.2 Accumulate the whole-portal WIDTH ratio (single deterministic, non-anisotropic ratio) into each member's `tile_font_scale` in `commit_portal_group_resize` (`crates/tze_hud_runtime/src/windowed/portal.rs`), gated on a real ratio and co-located with the existing single per-step `scene.version` bump so a resize step re-shapes at most once.
- [x] 2b.3 Apply the multiplier at text-collection time in the compositor (`collect_text_items_from_node` + `collect_ellipsis_text_items_from_node`, `renderer/text.rs`): effective font = `base * scale` clamped to token-defined legible min/max (`portal.text.min_font_px`/`portal.text.max_font_px`, defaults 9/48 as tze_hud_scene consts — no hardcoded clamp in the compositor). Both collectors share `scale_portal_font_px` so the truncation key matches the shaped font. Shrink past the min holds the font at the floor while only the content window shrinks (spec scenario 3).
- [x] 2b.4 Tests: scene `tile_font_scale_defaults_to_one_and_round_trips`, `tile_font_scale_cleared_on_tile_removal`; runtime `whole_portal_resize_accumulates_font_scale_by_width_ratio` (width-ratio, accumulation, single version bump); compositor `scale_portal_font_px_scales_and_clamps` (grow/shrink/max/min/inverted) + `portal_resize_scales_and_clamps_text_font` (token-default clamp through the compositor).

## 3. Reconcile and close

- [ ] 3.1 Re-verify live on the reference hardware (operator keypress) that the whole portal grows/shrinks.
- [ ] 3.2 After 2.x lands and is verified, sync the delta to `openspec/specs/text-stream-portals/spec.md` and archive (`openspec archive portal-whole-unit-resize`).
- [ ] 3.3 Close `hud-fb3en` on merge of the implementation.
