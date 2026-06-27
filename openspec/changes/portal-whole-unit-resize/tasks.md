# Tasks — Portal Whole-Unit Resize

This change is **spec ahead of code**: the delta clarifies that a portal moves
and resizes as a coherent unit. It stays OPEN until the implementation lands and
is verified on the reference hardware, then it is synced + archived.

## 1. Contract and review

- [x] 1.1 Validate this change with `openspec validate portal-whole-unit-resize --strict`
- [x] 1.2 Confirm doctrine alignment: "screen is sovereign", "local feedback first", "one scene model, two profiles" (CLAUDE.md, RFC 0013 §4.1/§4.2)
- [x] 1.3 Confirm the delta adds no new transport/RPC and does not change focus scoping, the resize step/clamp/lease-budget contract, the release-only key-up fallback, or the adapter-cannot-override-gesture rule

## 2. Implement — runtime portal-group-aware resize (`hud-fb3en`)

- [ ] 2.1 Give the runtime a way to enumerate a portal's constituent surfaces (lightest: reuse the existing `Tile.sync_group` member-set — first verify it does not perturb `evaluate_sync_group_commit` timing/commit machinery; else add a dedicated `portal_group_id`). Coordinate with the promotion epic `hud-g1ena` (owns the portal component model).
- [ ] 2.2 In `apply_portal_resize_hotkey` and `apply_portal_resize_pointer_event` (`crates/tze_hud_runtime/src/windowed/portal.rs`): resolve the portal anchor (frame surface), clamp against the lease budget on the whole-portal rect, scale ALL member surfaces around the anchor preserving relative layout, broadcast geometry per member. Mirror the client-side `portal_bounds_mutations` grouping pattern.
- [ ] 2.3 Test: focusing the composer and applying Ctrl+=/Ctrl+- (and the pointer affordance) grows/shrinks the whole portal as a unit; no constituent surface moves independently.

## 3. Reconcile and close

- [ ] 3.1 Re-verify live on the reference hardware (operator keypress) that the whole portal grows/shrinks.
- [ ] 3.2 After 2.x lands and is verified, sync the delta to `openspec/specs/text-stream-portals/spec.md` and archive (`openspec archive portal-whole-unit-resize`).
- [ ] 3.3 Close `hud-fb3en` on merge of the implementation.
