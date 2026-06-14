# scene graph (`graph.rs`) Module Split Plan

**Issue**: hud-mu55c (planning child of hud-luovo god-module-split epic)
**Date**: 2026-06-14
**Author**: agent/hud-mu55c
**Status**: Draft

---

## 1. Purpose

`crates/tze_hud_scene/src/graph.rs` is 10,666 lines — the largest file in
`tze_hud_scene` and a frequent merge-conflict source. It contains:

- ≈ 4,472 production lines (L1–4191): `SceneGraph` struct, `RuntimeOverlayState`,
  module-level helpers (`default_clock`, `ContentionRecord`, `apply_contention`,
  `now_micros`, `SyncGroupCommitDecision`), and a single `impl SceneGraph` block
  subdivided by section banners.
- ≈ 6,194 test lines (L4193–10644): `#[cfg(test)] mod tests { ... }` with sub-banners
  for zone tests, lease/budget tests, widget tests, spec-scenario tests, and
  cycle-guard tests.
- 22 lines (L10646–10666): `pub fn validate_text_markdown_node_data` stranded
  after the test module close brace.

This document plans converting `graph.rs` into a `graph/` directory module
(`graph/mod.rs` + submodule files) along its existing section-banner seams,
with mechanical (move-only) commits that leave all observable behaviour unchanged.

This is **planning only** — no Rust code is changed in this document.

---

## 2. Guiding Principles

1. **Move-only commits**: no logic changes in any split commit. Reviewers must be
   able to verify with `diff -u old.rs submodule/*.rs` that nothing was added or
   deleted — with the explicit exception of **visibility modifiers**. Items that
   were implicitly private to a single file become `pub(super)` (visible to the
   parent module and its children) or `pub(crate)` when moved into a child module
   and called from a sibling or the parent `mod.rs`. These are the minimal
   mechanical additions required by Rust's module privacy rules and are expected
   in every split commit. Execution PRs must list which items gained
   `pub(super)`/`pub(crate)` in their PR description.
2. **API preservation via `pub use`**: callers outside `tze_hud_scene` must not
   need to update import paths. `lib.rs` already re-exports everything from `graph`;
   `mod.rs` will re-export from each submodule.
3. **One submodule per commit** (or tightly coupled pair): keeps each PR
   reviewable in isolation.
4. **Tests move last**: the massive `mod tests { ... }` block moves to
   `graph/tests.rs` after all production code is split.
5. **Stranded function reunited before tests move**: `validate_text_markdown_node_data`
   (currently below the test module close brace) is moved into `graph/tiles.rs`
   (the submodule that calls it) before the test step, eliminating the stranded
   placement.
6. **Line numbers are approximate**: use section banner text as the anchor, not
   line numbers. Both files receive frequent edits. Verify banner positions with
   `rg -n "// ─── <banner name>"` before each PR.

---

## 3. File Structure

**Location**: `crates/tze_hud_scene/src/graph.rs`
**Lines (at plan date)**: 10,666
**Production lines (excluding tests)**: ≈ 4,472 (L1–4191)
**Test lines**: ≈ 6,194 (L4193–10644)
**Stranded post-test function**: 22 lines (L10646–10666)

### 3.1 Verified Section-Banner Seams

| # | Approx. line | Banner text | Contents |
|---|---|---|---|
| — | L1–211 | (preamble) | Module doc, imports, `default_clock` fn, `RuntimeOverlayState` struct, `SceneGraph` struct + constants, `ContentionRecord` trait + impls, `apply_contention` fn |
| 1 | L212 | `// ─── Contention policy helper` | `ContentionRecord` trait, its two impls for `ZonePublishRecord` / `WidgetPublishRecord`, and the generic `apply_contention<R>` free function (L212–318) |
| — | L320–415 | (impl SceneGraph preamble) | `impl SceneGraph {` opens; `coerce_widget_param_value` (private, widget param validation helper) |
| 2 | L417 | `// ─── Notification auto-dismiss TTL constants` | Three `pub const` TTL values; `new`, `new_with_clock` constructors; overlay drain methods; scroll config accessors (L417–535) |
| 3 | L536 | `// ─── Follow-tail anchor` | `set_tile_follow_tail_at_tail`, `tile_follow_tail_at_tail` (L536–577) |
| 4 | L579 | `// ─── Resource registry` | `register_resource`, `is_resource_registered`, `resource_ref_count`, `inc_resource_ref`, `dec_resource_ref` (L579–636) |
| 5 | L637 | `// ─── Tab operations` | `create_tab`, `create_tab_with_lease`, `create_tab_checked`, `delete_tab*`, `rename_tab*`, `reorder_tab*`, `switch_active_tab*`, `set_tab_switch_on_event`, `find_tab_for_event` (L637–1019) |
| 6 | L897 (sub) | `// ─── Capability helpers` | `require_capability`, `require_active_lease` — sub-banner inside impl block; lives within tab operations region but is a true cross-cutting helper (L897–957) |
| 7 | L1020 | `// ─── Lease operations` | All lease grant/revoke/renew/suspend/resume/disconnect/reconnect/expire functions; lease state machine; `clear_zone_publications_for_namespace` (L1020–1516) |
| 8 | L1517 | `// ─── Budget enforcement` | `lease_resource_usage`, `check_budget`, `is_lease_budget_warning`, `count_nodes_in_tile`, `count_node_subtree*`, `sum_texture_bytes*`, `count_node_tree*`, `count_texture_bytes_in_node`, `initiate_budget_revocation`, `finalize_budget_revocation` (L1517–1780) |
| 9 | L1781 | `// ─── Tile operations` | `create_tile`, `create_tile_checked`, `create_tile_impl`, `update_tile_bounds`, `update_tile_z_order`, `update_tile_opacity`, `update_tile_input_mode`, `update_tile_expiry`, `delete_tile`, `get_tile_lease_checked`, `set_tile_root`, `set_tile_root_checked`, `set_tile_root_impl`, `add_node_to_tile*`, `update_node_content*` (L1781–2483) |
| 10 | L2484 | `// ─── Sync group operations` | `create_sync_group`, `delete_sync_group`, `join_sync_group`, `leave_sync_group`, `evaluate_sync_group_commit`, `join_sync_group_checked`, `sync_group_count` (L2484–2717) |
| 11 | L2719 | `// ─── Node tree helpers` | `insert_node_tree`, `remove_node_tree`, `remove_tile_and_nodes` (L2719–2762) |
| 12 | L2764 | `// ─── Queries` (first) | `visible_tiles`, `hit_test`, `update_hover_state`, `update_pressed_state`, `update_focused_state`, drag-active helpers, `hit_test_node*`, `is_node_in_subtree*` (L2764–3098) |
| 13 | L3099 | `// ─── Zone operations` | `register_zone`, `unregister_zone`, `publish_to_zone*`, `coerce_widget_param_value` (already at preamble), `publish_to_widget`, `set_widget_param_local`, `resolve_lease_state_for_namespace`, `initiate/finalize_budget_revocation` (already in budget banner), `clear_zone`, `clear_zone_for_publisher`, `dismiss_notification`, `clear_widget_publications_for_namespace`, `clear_widget_for_publisher`, `refresh_widget_current_params`, `content_media_type` (L3099–3914) |
| 14 | L3916 | `// ─── Queries` (second) | `snapshot_json`, `from_json`, `take_snapshot`, `node_count`, `tile_count` (L3916–4073) |
| 15 | L4075 | `// ─── Sequence number (RFC 0001 §3.5)` | `next_sequence_number` (L4075–4085) |
| 16 | L4086 | `// ─── Clock accessor` | `now_millis` (L4086–4091) |
| 17 | L4093 | `// ─── Zone publication expiry` | `drain_expired_zone_publications` (L4093–4126) |
| 18 | L4128 | `// ─── Widget publication expiry` | `drain_expired_widget_publications` (L4128–4171) |
| — | L4174–4191 | (module-level helpers, post-impl) | `now_micros` fn, `SyncGroupCommitDecision` enum |
| 19 | L4193–10644 | `#[cfg(test)] mod tests` | All test sub-banners (zone tests, lease/budget, widget, spec-scenario, cycle-guard) |
| 20 | L10646–10666 | `// ─── Helper for TextMarkdownNode content size validation` | `pub fn validate_text_markdown_node_data` (stranded after test close brace) |

### 3.2 Logical Cluster Summary

After grouping the sub-banners into meaningful submodules:

| Cluster | Key items | Approx. prod lines |
|---|---|---|
| **contention** | `ContentionRecord` trait + impls, `apply_contention` | ~110 |
| **overlay** | `RuntimeOverlayState` struct, overlay drain/accessor methods, scroll config, follow-tail | ~310 |
| **resources** | `register_resource`, ref-count helpers (`inc_resource_ref`, `dec_resource_ref`) | ~60 |
| **tabs** | All tab CRUD + capability helpers (`require_capability`, `require_active_lease`) + event routing | ~385 |
| **leases** | Lease grant/revoke/renew/suspend/expire state machine | ~500 |
| **budget** | Budget accounting, node count helpers, texture byte helpers | ~265 |
| **tiles** | Tile CRUD + node tree mutations (set_tile_root, add_node, update_node) + `validate_text_markdown_node_data` | ~705 |
| **sync_groups** | Sync group CRUD + commit evaluation | ~235 |
| **node_tree** | `insert_node_tree`, `remove_node_tree`, `remove_tile_and_nodes` | ~45 |
| **queries** | Hit-testing, visible tiles, drag state, hover/press, node subtree checks | ~340 |
| **zone_ops** | Zone register/unregister/publish/clear, widget publish/param/clear, budget revocation (calls budget module), expiry drains | ~840 |
| **snapshot** | `snapshot_json`, `from_json`, `take_snapshot`, `node_count`, `tile_count` | ~160 |
| **mod.rs remainder** | `SceneGraph` struct + `default_clock`, constructors (`new`, `new_with_clock`), clock accessor, sequence number, `now_micros`, `SyncGroupCommitDecision`, notification TTL constants + `pub use` re-exports | ~200 |

---

## 4. Proposed Submodule Breakdown

Target directory: `crates/tze_hud_scene/src/graph/`

`graph.rs` becomes a directory module (`graph/mod.rs`) with re-exports.

```
graph/
├── mod.rs               # SceneGraph struct, default_clock, constructors (new/new_with_clock),
│                        # notification TTL constants, clock accessor (now_millis),
│                        # sequence number (next_sequence_number), now_micros,
│                        # SyncGroupCommitDecision, pub use * from each submodule
├── contention.rs        # ContentionRecord trait + impls + apply_contention<R> free fn
├── overlay.rs           # RuntimeOverlayState struct; overlay drain/accessor methods
│                        # (enqueue_widget_svg_asset, drain_pending_widget_svg_assets,
│                        # drain_removed_tile_ids); scroll config accessors;
│                        # follow-tail anchor accessors
├── resources.rs         # register_resource, is_resource_registered, resource_ref_count,
│                        # inc_resource_ref (pub(super)), dec_resource_ref (pub(super))
├── tabs.rs              # Tab CRUD (create_tab*, delete_tab*, rename_tab*, reorder_tab*,
│                        # switch_active_tab*); require_capability, require_active_lease;
│                        # set_tab_switch_on_event, find_tab_for_event
├── leases.rs            # Lease constants (DEFAULT_MAX_SUSPENSION_MS etc.); grant_lease*,
│                        # try_grant_lease_for_session, revoke_lease, revoke_capability,
│                        # lease_capabilities, renew_lease, suspend_lease, resume_lease,
│                        # disconnect_lease, reconnect_lease, suspend_all_leases,
│                        # resume_all_leases, expire_leases*, clear_zone_publications_for_namespace
├── budget.rs            # lease_resource_usage, check_budget, is_lease_budget_warning,
│                        # count_nodes_in_tile, count_node_subtree, count_node_subtree_inner,
│                        # sum_texture_bytes, sum_texture_bytes_inner, count_node_tree,
│                        # count_node_tree_deep, count_texture_bytes_in_node,
│                        # initiate_budget_revocation, finalize_budget_revocation;
│                        # ResourceUsage type (if defined here) or imported from types
├── tiles.rs             # create_tile*, create_tile_impl, update_tile_bounds,
│                        # update_tile_z_order, update_tile_opacity, update_tile_input_mode,
│                        # update_tile_expiry, delete_tile, get_tile_lease_checked,
│                        # set_tile_root*, set_tile_root_impl, add_node_to_tile*,
│                        # add_node_to_tile_impl, update_node_content*,
│                        # update_node_content_impl;
│                        # validate_text_markdown_node_data (moved from post-test stranded position)
├── sync_groups.rs       # Sync group constants (MAX_SYNC_GROUPS_PER_NAMESPACE etc.);
│                        # create_sync_group, delete_sync_group, join_sync_group,
│                        # leave_sync_group, evaluate_sync_group_commit,
│                        # join_sync_group_checked, sync_group_count;
│                        # SyncGroupCommitDecision stays in mod.rs (public enum, re-exported)
├── node_tree.rs         # insert_node_tree (pub(super)), remove_node_tree (pub(crate)),
│                        # remove_tile_and_nodes (pub(crate))
├── queries.rs           # visible_tiles, hit_test, update_hover_state, update_pressed_state,
│                        # update_focused_state, set_drag_handle_hovered, set_drag_active,
│                        # clear_drag_active, is_drag_active, set_drag_handle_pressed,
│                        # hit_test_node, hit_test_node_inner, is_node_in_subtree (pub(crate)),
│                        # is_node_in_subtree_inner
├── zone_ops.rs          # register_zone, unregister_zone, publish_to_zone*,
│                        # publish_to_widget, coerce_widget_param_value (pub(super)),
│                        # set_widget_param_local, resolve_lease_state_for_namespace,
│                        # clear_zone, clear_zone_for_publisher, dismiss_notification,
│                        # clear_widget_publications_for_namespace, clear_widget_for_publisher,
│                        # refresh_widget_current_params, content_media_type,
│                        # drain_expired_zone_publications, drain_expired_widget_publications
└── tests.rs             # existing #[cfg(test)] mod tests { ... } content (6,194 lines)
```

**`mod.rs` retains after full split (≈ 200 production lines)**:
- Module-level `use` imports
- `fn default_clock() -> Arc<dyn Clock>`
- `pub struct SceneGraph { ... }` with all 13 fields
- `impl SceneGraph { fn new(...) ... fn new_with_clock(...) ... }`
- Notification TTL constants (`NOTIFICATION_TTL_INFO_US`, etc.)
- `pub(crate) fn now_millis`, `pub(crate) fn next_sequence_number`
- `fn now_micros() -> u64`
- `pub enum SyncGroupCommitDecision { ... }`
- Module declarations (`mod contention; mod overlay; ...`)
- `pub use` re-exports for every item moved to submodules

**Approximate post-split production line counts**:
- `mod.rs`: ≈ 200 lines
- `zone_ops.rs`: ≈ 840 lines (largest — contains all zone + widget publish paths)
- `tiles.rs`: ≈ 705 lines
- `leases.rs`: ≈ 500 lines
- `queries.rs`: ≈ 340 lines
- `tabs.rs`: ≈ 385 lines
- `budget.rs`: ≈ 265 lines
- `sync_groups.rs`: ≈ 235 lines
- `overlay.rs`: ≈ 310 lines
- `snapshot.rs`: ≈ 160 lines
- `contention.rs`: ≈ 110 lines
- `resources.rs`: ≈ 60 lines
- `node_tree.rs`: ≈ 45 lines
- `tests.rs`: ≈ 6,194 lines (test-only, not counted against production target)

---

## 5. Cross-Section Coupling

| Coupling | Description | Mitigation |
|---|---|---|
| `SceneGraph` struct (13 fields) | Every method in every cluster borrows `&self` or `&mut self` | Keep struct in `mod.rs`; submodule methods are `impl SceneGraph` blocks in submodule files — Rust's split impl pattern is valid within a single module |
| `apply_contention<R>` | Called by `publish_to_zone` and `publish_to_widget` (both in `zone_ops.rs`) | Move to `contention.rs`; `zone_ops.rs` imports via `use super::contention::apply_contention` |
| `ContentionRecord` trait | Implemented for `ZonePublishRecord` and `WidgetPublishRecord` (types from `crate::types`) | Both impls stay in `contention.rs` with the trait — no cross-submodule impl needed |
| `validate_text_markdown_node_data` | Called by `set_tile_root_impl`, `add_node_to_tile_impl`, `update_node_content_impl` (all in `tiles.rs`) and re-exported from `lib.rs` | Move to `tiles.rs`; `mod.rs` adds `pub use tiles::validate_text_markdown_node_data` to preserve the public re-export chain (`lib.rs` does not need to change) |
| `require_capability` + `require_active_lease` | Private helpers; called by tab ops AND lease ops AND tile ops | Move to `tabs.rs` (their natural home); any method in another submodule that needs them imports via `use super::tabs::SceneGraph` — the split impl pattern means the method is still `impl SceneGraph` in `tabs.rs` but visible via `pub(super)` |
| `inc_resource_ref` / `dec_resource_ref` | Private; called from `insert_node_tree` (node_tree.rs) and `remove_node_tree` (node_tree.rs) | `resources.rs` declares them `pub(super)`; `node_tree.rs` imports via `use super::resources` — both are within the `graph` module |
| `insert_node_tree` | Called by `set_tile_root_impl`, `add_node_to_tile_impl` (in `tiles.rs`) | `node_tree.rs` declares it `pub(super)`; `tiles.rs` imports via `super::node_tree::SceneGraph` — all within `graph` module |
| `remove_tile_and_nodes` | Called externally (mutation.rs, session_server), declared `pub(crate)` | Keep `pub(crate)` visibility; `mod.rs` re-exports it as before |
| `count_node_subtree` | Declared `pub(crate)`; called from `invariants.rs` | Keep `pub(crate)` in `budget.rs`; re-exported via `mod.rs pub use budget::*` |
| `now_micros` free fn | Called by `create_sync_group` (sync_groups.rs) | Keep in `mod.rs` (9 lines, not worth a dedicated file); `sync_groups.rs` calls via `super::now_micros()` |
| `SyncGroupCommitDecision` enum | Public type, re-exported from `lib.rs` | Keep in `mod.rs` alongside the enum it annotates; `sync_groups.rs` imports via `super::SyncGroupCommitDecision` |
| `MAX_TABS`, `MAX_TILES_PER_TAB`, etc. | Module-level pub consts referenced by both prod code and tests | Keep as module-level consts in `mod.rs`; all submodules import via `super::*` via the `#[cfg(test)] use super::*` pattern already in tests |
| `clear_zone_publications_for_namespace` | Logically a zone clean-up op but placed inside the lease banner | Move to `zone_ops.rs` (logical home); the lease revoke path in `leases.rs` calls it via `self.clear_zone_publications_for_namespace(...)` — split impl means the call is direct on `self` |
| `initiate_budget_revocation` / `finalize_budget_revocation` | In budget banner but delete tiles and affect leases | Keep in `budget.rs`; they call `self.revoke_lease` (from `leases.rs`) and `self.remove_tile_and_nodes` (from `node_tree.rs`) — split impl, all calls are on `self`, no explicit import needed |
| Test helpers in `mod tests` | Use `use super::*` which brings in everything from `mod.rs` and its re-exports | After split, `tests.rs` uses `use super::*` — unchanged, because `mod.rs` re-exports everything via `pub use submodule::*` |

---

## 6. Incremental Sequencing

Perform one step per PR. Each step is a pure move with no logic changes.

**Step G-1: Contention policy helper (leaf, no deps on other submodules)**
- `contention.rs` ← `ContentionRecord` trait + two `impl ContentionRecord for ...` blocks
  + `apply_contention<R: ContentionRecord>` free fn (banner: `// ─── Contention policy helper`)
- `mod.rs` adds: `mod contention; pub(super) use contention::apply_contention;`
  (pub(super) since only `zone_ops.rs` will call it — but a `pub use` is cleaner for
  discoverability; use `pub(crate)` for `apply_contention` and the trait)
- No downstream callers yet; verifies the trait compiles in isolation

**Step G-2: Overlay state (leaf, no deps on impl SceneGraph methods)**
- `overlay.rs` ← `RuntimeOverlayState` struct definition + all `impl SceneGraph` overlay
  drain/accessor methods (banners: preamble `RuntimeOverlayState`, `// ─── Follow-tail anchor`,
  scroll config methods between `// ─── Notification auto-dismiss TTL constants` and
  `// ─── Resource registry`)
- `mod.rs` keeps the struct field `pub overlay: RuntimeOverlayState` and the `#[serde(skip, default)]` annotation; `mod.rs` adds `mod overlay; pub use overlay::RuntimeOverlayState;` and re-exports methods via the split impl
- These methods only touch `self.overlay.*` and `self.tiles` (for existence checks)
- Visibility additions: none expected (all methods already `pub`)

**Step G-3: Resource registry (leaf, no deps on other submodule methods)**
- `resources.rs` ← `register_resource`, `is_resource_registered`, `resource_ref_count`,
  `inc_resource_ref`, `dec_resource_ref` (banner: `// ─── Resource registry`)
- `inc_resource_ref` and `dec_resource_ref` become `pub(super)` (currently private to the
  file; after split they must be visible to sibling `node_tree.rs`)
- `mod.rs` adds `mod resources; pub use resources::{...public items...};`

**Step G-4: Node tree helpers (depends on G-3 for pub(super) resource ref helpers)**
- `node_tree.rs` ← `insert_node_tree`, `remove_node_tree` (pub(crate)), `remove_tile_and_nodes` (pub(crate)) (banner: `// ─── Node tree helpers`)
- These three methods call `self.inc_resource_ref` / `self.dec_resource_ref` (from G-3,
  accessible via split impl since they're all in the same module)
- `mod.rs` adds `mod node_tree; pub use node_tree::{remove_node_tree, remove_tile_and_nodes};`

**Step G-5: Tab operations + capability helpers (depends on G-4 for remove_tile_and_nodes call in delete_tab)**
- `tabs.rs` ← banner `// ─── Tab operations` + inline sub-banner
  `// ─── Capability helpers` (these live inside tab ops region):
  `create_tab*`, `delete_tab*`, `rename_tab*`, `reorder_tab*`, `switch_active_tab*`,
  `require_capability`, `require_active_lease`, `set_tab_switch_on_event`, `find_tab_for_event`
- `require_capability` and `require_active_lease` become `pub(super)` (called by tiles,
  leases, and zone_ops in later steps via split impl — they call `self.require_capability`
  directly, no explicit import needed)
- `delete_tab` calls `self.remove_tile_and_nodes` (from `node_tree.rs`) — split impl, no import
- `mod.rs` adds `mod tabs; pub use tabs::{...};`

**Step G-6: Lease operations (depends on G-5 for require_capability/require_active_lease)**
- `leases.rs` ← banner `// ─── Lease operations`:
  lease constants (`DEFAULT_MAX_SUSPENSION_MS`, `DEFAULT_GRACE_PERIOD_MS`,
  `BUDGET_SOFT_LIMIT_PCT`, `MAX_RUNTIME_LEASES`, `DEFAULT_MAX_LEASES_PER_SESSION`,
  `MAX_LEASES_PER_SESSION`, `MAX_TILES_PER_LEASE`), all grant/revoke/renew/suspend/expire
  methods, `clear_zone_publications_for_namespace`
- `try_grant_lease_for_session` calls `self.require_capability` (from `tabs.rs`) — split impl
- `expire_leases*` calls `self.remove_tile_and_nodes` (from `node_tree.rs`) — split impl
- `clear_zone_publications_for_namespace` moves here from its banner position inside lease ops
  (logically belongs with zone cleanup called at lease revoke time; a follow-on can relocate
  to `zone_ops.rs` if preferred — see Section 8)
- `mod.rs` adds `mod leases; pub use leases::{...};`
- **Most import-sensitive step** — all subsequent submodules depend on lease state. Merge
  and verify CI before proceeding with G-7.

**Step G-7: Budget enforcement (depends on G-6 for lease reads)**
- `budget.rs` ← banner `// ─── Budget enforcement`:
  `lease_resource_usage`, `check_budget`, `is_lease_budget_warning`,
  all `count_*` and `sum_*` private helpers,
  `initiate_budget_revocation`, `finalize_budget_revocation`
- `lease_resource_usage` and `check_budget` read `self.leases` (from `leases.rs`) —
  split impl, direct field access via `self.leases`
- `finalize_budget_revocation` calls `self.revoke_lease` (from `leases.rs`) and
  `self.remove_tile_and_nodes` (from `node_tree.rs`) — both are `impl SceneGraph` methods
  accessible via split impl
- `count_node_subtree` stays `pub(crate)` (called from `invariants.rs`)
- `mod.rs` adds `mod budget; pub use budget::{lease_resource_usage, check_budget, ...};`

**Step G-8: Tile operations (depends on G-3/G-4 for node tree, G-5/G-6 for capability/lease checks)**
- `tiles.rs` ← banner `// ─── Tile operations`:
  `create_tile*`, all `update_tile_*`, `delete_tile`, `get_tile_lease_checked`,
  `set_tile_root*`, `add_node_to_tile*`, `update_node_content*`;
  **also** `validate_text_markdown_node_data` (moved from its stranded post-test position
  at L10646–10666 — this is its natural home since all three call sites are in this cluster)
- `set_tile_root_impl` calls `validate_text_markdown_node_data` (now in same file) and
  `self.insert_node_tree` / `self.remove_node_tree` (split impl)
- `create_tile_impl` calls `self.require_active_lease`, `self.require_capability` (split impl)
- `mod.rs` adds `mod tiles; pub use tiles::{create_tile, ..., validate_text_markdown_node_data};`
  (the `pub use tiles::validate_text_markdown_node_data` preserves the re-export from `lib.rs`)

**Step G-9: Sync group operations (depends on G-5 for tile existence checks)**
- `sync_groups.rs` ← banner `// ─── Sync group operations`:
  sync group constants, `create_sync_group`, `delete_sync_group`, `join_sync_group`,
  `leave_sync_group`, `evaluate_sync_group_commit`, `join_sync_group_checked`, `sync_group_count`
- `create_sync_group` calls `now_micros()` — import via `super::now_micros` or keep
  `now_micros` in `mod.rs` and call it as `super::now_micros()`
- `SyncGroupCommitDecision` stays in `mod.rs`; `evaluate_sync_group_commit` returns it
  via `super::SyncGroupCommitDecision`
- `mod.rs` adds `mod sync_groups; pub use sync_groups::{...};`

**Step G-10: Queries / hit-testing (depends on G-5/G-6 for lease priority reads)**
- `queries.rs` ← banners `// ─── Queries` (first occurrence, L2764) and
  `// ─── Node tree helpers` sub-cluster queries:
  `visible_tiles`, `hit_test`, `update_hover_state`, `update_pressed_state`,
  `update_focused_state`, drag-state helpers (`set_drag_handle_hovered`, `set_drag_active`,
  `clear_drag_active`, `is_drag_active`, `set_drag_handle_pressed`),
  `hit_test_node`, `hit_test_node_inner`, `is_node_in_subtree` (pub(crate)),
  `is_node_in_subtree_inner`
- All methods only read `self.tiles`, `self.nodes`, `self.leases`, `self.overlay` —
  split impl, no explicit cross-submodule calls needed
- `mod.rs` adds `mod queries; pub use queries::{visible_tiles, hit_test, ...};`

**Step G-11: Zone + widget operations (depends on G-1 for apply_contention, G-6 for lease state)**
- `zone_ops.rs` ← banner `// ─── Zone operations` and banner `// ─── Zone publication expiry`
  and banner `// ─── Widget publication expiry`:
  `register_zone`, `unregister_zone`, `publish_to_zone*`, `publish_to_widget`,
  `coerce_widget_param_value` (pub(super) within `zone_ops.rs` — it's private, only called
  within this file), `set_widget_param_local`, `resolve_lease_state_for_namespace`,
  `clear_zone`, `clear_zone_for_publisher`, `dismiss_notification`,
  `clear_widget_publications_for_namespace`, `clear_widget_for_publisher`,
  `refresh_widget_current_params`, `content_media_type`,
  `drain_expired_zone_publications`, `drain_expired_widget_publications`
- `publish_to_zone` and `publish_to_widget` call `apply_contention` (from `contention.rs`)
  via `super::contention::apply_contention` or via `use super::contention::apply_contention`
- `resolve_lease_state_for_namespace` reads `self.leases` (split impl)
- `mod.rs` adds `mod zone_ops; pub use zone_ops::{register_zone, publish_to_zone, ...};`
- This is the largest submodule (≈ 840 lines); if it grows uncomfortable, the widget
  publish cluster can be extracted as `widget_ops.rs` in a follow-on

**Step G-12: Snapshot + scalar queries (depends on G-11 for zone registry read)**
- `snapshot.rs` ← banner `// ─── Queries` (second occurrence, L3916):
  `snapshot_json`, `from_json`, `take_snapshot`, `node_count`, `tile_count`
- `take_snapshot` reads all fields of `self` (split impl, direct field access)
- `mod.rs` adds `mod snapshot; pub use snapshot::{snapshot_json, take_snapshot, ...};`

**Step G-13: Tests**
- `tests.rs` ← entire `#[cfg(test)] mod tests { ... }` block (L4193–10644)
- File: `graph/tests.rs` (not a directory — single file is sufficient for the test block)
- `mod.rs` adds `#[cfg(test)] mod tests;` (or `#[cfg(test)] include!("tests.rs")`)
- Test module opens with `use super::*;` — unchanged; after split, `super::*` brings in
  everything re-exported from `mod.rs`

---

## 7. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Circular imports via split impl | Low | Rust's split impl does not create imports — it extends the same type in the same module. No submodule needs to import another submodule's methods as functions; they call `self.method()` directly. |
| `require_capability` / `require_active_lease` called from multiple submodules | High (by design) | Split impl means `tiles.rs`, `leases.rs`, `zone_ops.rs` all call `self.require_capability(...)` without any import — the method lives in `tabs.rs` as an `impl SceneGraph` block in the same `graph` module. No circular dep. |
| `validate_text_markdown_node_data` stranded after test close brace | Certain (already is) | G-8 explicitly relocates it to `tiles.rs` before the test step. The `pub use tiles::validate_text_markdown_node_data` in `mod.rs` preserves the `lib.rs` re-export without any caller changes. |
| `coerce_widget_param_value` in pre-banner preamble | Medium | It is a private helper only called within the zone/widget publish paths; relocating it to `zone_ops.rs` (G-11) is natural. No callers outside that cluster. |
| `now_micros` free fn called from `sync_groups.rs` | Low | Keep `now_micros` in `mod.rs`; `sync_groups.rs` calls `super::now_micros()`. Nine lines — not worth a dedicated file. |
| `SyncGroupCommitDecision` enum visibility | Low | Keep in `mod.rs`; it's a public enum used by callers outside the crate and is already `pub` in `lib.rs` re-exports. |
| Test file size (6,194 lines) | Low | The test block moves as one atomic step (G-13). No test refactoring in the split. |
| `clear_zone_publications_for_namespace` logical home ambiguity | Low | It sits in the lease banner (L1507) but is a zone cleanup op. G-6 places it in `leases.rs` (matching its current banner home). A follow-on task (see Section 8) can migrate it to `zone_ops.rs` if desired — it is a one-function move that poses no risk. |
| Merge conflicts during parallel execution of G-10/G-11/G-12 | Medium | These three steps are parallelizable after G-9 is merged. CI catches conflicts; workers should not edit the same banner region concurrently. |
| Line numbers drift before execution | High (graph.rs receives frequent edits) | Use section banner text as the anchor, not line numbers. Verify with `rg -n "// ─── <banner>"` before each PR. |
| `zone_ops.rs` ≈ 840 lines (largest submodule) | Medium | Acceptable for phase 1. Zone and widget publish paths are tightly coupled through `apply_contention` and `content_media_type`; premature separation would scatter that coupling. Follow-on: extract `widget_ops.rs` once the split is stable (see Section 8). |

---

## 8. Discovered Follow-Ups

These are separate tasks, not part of the mechanical split:

| Bead candidate | Description |
|---|---|
| Extract `widget_ops.rs` from `zone_ops.rs` | After G-11, if `zone_ops.rs` is still ≈ 840 lines, split the widget publish/param/clear cluster (`publish_to_widget`, `set_widget_param_local`, `coerce_widget_param_value`, `clear_widget*`, `refresh_widget_current_params`) into a sibling `widget_ops.rs`. Move-only; `zone_ops.rs` would shrink to ≈ 440 lines. |
| Relocate `clear_zone_publications_for_namespace` to `zone_ops.rs` | Currently in the lease banner; logically a zone cleanup op called from `revoke_lease`. Post-split, a one-function migration from `leases.rs` to `zone_ops.rs` with a `pub use` bridge. |
| Extract constructors to `constructors.rs` | After G-1..G-13, `mod.rs` will be ≈ 200 lines. If the `new` + `new_with_clock` constructors grow (e.g., post-v1 capability wiring), extract to `constructors.rs` as a follow-on. Not needed for phase 1. |
| Dedup near-identical `ZonePublishRecord` construction blocks | `publish_to_zone` has three entry points (`publish_to_zone`, `publish_to_zone_with_lease`, `publish_to_zone_with_breakpoints`) with overlapping record-building logic. Post-split, a builder extraction is cleaner. Separate task; do NOT combine with any split PR. |

---

## 9. Acceptance Criteria Checklist

Per hud-mu55c:

- [x] Split plan with module boundaries and migration order written and reviewed (this document)
- [ ] `graph.rs` production lines ≤ 500 in `mod.rs` post-split (target ≈ 200 lines)
- [ ] All submodules ≤ 850 lines each (target: largest is `zone_ops.rs` at ≈ 840)
- [ ] `validate_text_markdown_node_data` relocated from stranded post-test position into `tiles.rs` in step G-8 (before tests step G-13)
- [ ] All splits are mechanical move-only commits (verifiable by diff); each PR description lists items that gained `pub(super)` or `pub(crate)` visibility as part of the move
- [ ] `lib.rs` re-export list unchanged (callers outside `tze_hud_scene` need no import path updates)
- [ ] Test suite green after each step (no behaviour change)
- [ ] Churn hotspot concentration measurably reduced (each submodule sees commits only when its domain changes)

Items 2–8 are execution targets, fulfilled when per-cluster execution tasks close.
