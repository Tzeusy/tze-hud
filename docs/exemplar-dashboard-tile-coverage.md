# Exemplar Dashboard Tile — Spec-to-Code Coverage Report

**Issue:** hud-i6yd.9 (gen-1 reconciliation)
**Spec:** `openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md`
**Audit date:** 2026-03-31

## Deliverable Index

| File | Issue | Purpose |
|---|---|---|
| `crates/tze_hud_protocol/tests/dashboard_tile_agent.rs` | hud-i6yd.3 | Session and lease acquisition |
| `tests/integration/dashboard_tile_creation.rs` | hud-i6yd.4 | Resource upload, atomic tile creation, content update, z-order |
| `tests/integration/dashboard_tile_input.rs` | hud-i6yd.5 | Input capture, local feedback, focus cycling |
| `crates/tze_hud_protocol/tests/dashboard_tile_agent_callbacks.rs` | hud-i6yd.6 | Agent callbacks on button activation |
| `crates/tze_hud_protocol/tests/lease_governance.rs` | hud-i6yd.7 | Full lease state machine, orphan/expiry, namespace isolation |
| `tests/integration/dashboard_tile_lifecycle.rs` | hud-i6yd.8 | End-to-end lifecycle, headless coverage, namespace isolation |

---

## Requirement 1: Dashboard Tile Composition

Spec requires exactly 6 nodes (SolidColorNode, StaticImageNode, 2× TextMarkdownNode, 2× HitRegionNode) in flat-tree painter's-model order, tile at (50,50,400×300), z_order=100, opacity=1.0.

| Scenario | Status | Evidence |
|---|---|---|
| All four node types present (6-node count, correct order) | **COVERED** | `dashboard_tile_creation.rs::scene_graph_has_6_nodes_in_correct_tree_order` asserts `node_count()=6`, root=SolidColor, children=[StaticImage, TextMarkdown×2, HitRegion×2] in that exact order |
| Background node covers full tile bounds (Rgba 0.07/0.07/0.07/0.90, bounds 0,0,400,300) | **COVERED** | `dashboard_tile_creation.rs::background_node_covers_full_tile_bounds_with_correct_color` |
| Icon image references uploaded resource | **COVERED** | `dashboard_tile_creation.rs::static_image_node_references_uploaded_resource_id_with_correct_geometry` verifies ResourceId, 48×48, Contain fit, position (16,16) |
| Painter's model compositing order | **COVERED** | `scene_graph_has_6_nodes_in_correct_tree_order` asserts child order is [StaticImage, TextMarkdown, TextMarkdown, HitRegion, HitRegion]; also cross-covered in `dashboard_tile_lifecycle.rs::headless_tile_creation_produces_6_nodes_in_correct_tree_order` |

---

## Requirement 2: Atomic Tile Creation Batch

Spec requires CreateTile + SetTileRoot/AddNode + UpdateTileOpacity + UpdateTileInputMode submitted atomically; partial failure rejects entire batch; fewer than 1000 mutations.

| Scenario | Status | Evidence |
|---|---|---|
| Successful atomic tile creation | **COVERED** | `dashboard_tile_creation.rs::atomic_tile_creation_batch_accepted` verifies batch accepted, opacity=1.0, input_mode=Passthrough, z_order=100, bounds correct |
| Partial failure rejects entire batch | **COVERED** | `dashboard_tile_creation.rs::partial_failure_rejects_entire_batch_atomically` submits batch with invalid width=0, asserts `!result.applied`, tile_count unchanged, created_ids empty, rejection populated |
| Batch does not exceed mutation limits | **PARTIAL** | The 4-mutation (or 9-mutation with AddNode) batch is exercised and passes, implicitly satisfying the <1000 check. However, there is no explicit test that asserts `batch.mutations.len() < 1000` or checks the size enforcement gate at exactly the limit. The functional path is covered but the boundary condition is not tested in isolation. |
| Rejection includes failing mutation_index and error code | **PARTIAL** | `partial_failure_rejects_entire_batch_atomically` asserts `result.rejection.is_some()`. The spec says rejection SHALL include `mutation_index` and `error_code`; these are structurally present in `ValidationRejection` (confirmed in `crates/tze_hud_scene/tests/batch_atomicity.rs`), but the exemplar test does not assert the specific `mutation_index` or `error_code` values in the rejection. |

---

## Requirement 3: Resource Upload Before Tile Creation

Spec requires icon PNG uploaded before tile creation, ResourceId = BLAKE3 hash, StaticImageNode references it; unknown ResourceId is rejected.

| Scenario | Status | Evidence |
|---|---|---|
| Resource uploaded and referenced (BLAKE3 hash returned, 32 bytes) | **COVERED** | `dashboard_tile_creation.rs::resource_upload_48x48_png_returns_blake3_resource_id` asserts ResourceId equals `BLAKE3(raw_bytes)` and is 32 bytes; deduplication also tested |
| Unknown resource rejected | **NOT COVERED** | No exemplar test submits a tile creation batch with an unregistered ResourceId and asserts `ResourceNotFound`. `ValidationError::ResourceNotFound` exists in the implementation (`crates/tze_hud_scene/src/validation.rs:70`) and is exercised in the scene validation unit tests, but there is no exemplar-specific test for this scenario. |

---

## Requirement 4: Lease Request With AutoRenew

Spec requires LeaseRequest with ttl_ms=60000, [create_tiles, modify_own_tiles], lease_priority; server responds granted=true with UUIDv7 LeaseId; server-side AutoRenew policy; tile creation without active lease rejected.

| Scenario | Status | Evidence |
|---|---|---|
| Lease granted with requested parameters (granted=true, UUIDv7 lease_id, ttl, capabilities, priority) | **COVERED** | `dashboard_tile_agent.rs::exemplar_lease_grant_returns_granted_true_and_uuidv7_lease_id` asserts all fields |
| Tile creation requires active lease (rejected with LeaseNotFound or LeaseNotActive) | **COVERED** | `dashboard_tile_agent.rs::exemplar_mutation_without_active_lease_is_rejected` sends MutationBatch with random lease_id, asserts `!accepted` and non-empty `error_code` |
| Lease auto-renews at 75% TTL | **COVERED** | `lease_governance.rs::auto_renewal_fires_at_75_percent_ttl` uses TestClock, advances to 44999ms (no trigger), then 45000ms (trigger), resets window and re-asserts trigger at next 75% mark; also `auto_renewal_arm_armed_at_activation` |

---

## Requirement 5: Periodic Content Update

Spec requires agent to update body TextMarkdownNode every 5 seconds via SetTileRoot (full tree swap), content < 65535 bytes, active lease required.

| Scenario | Status | Evidence |
|---|---|---|
| Successful content update (SetTileRoot accepted, node count stays 6, body content changed) | **COVERED** | `dashboard_tile_creation.rs::content_update_with_active_lease_accepted` and `dashboard_tile_lifecycle.rs::full_lifecycle_connect_lease_upload_create_update_refresh_dismiss` Phase 4 both verify |
| Content update with expired lease rejected (LeaseExpired error code) | **COVERED** | `dashboard_tile_creation.rs::content_update_with_expired_lease_rejected` advances TestClock past TTL, asserts `!applied`, checks `ValidationErrorCode::LeaseExpired` in rejection |
| Content does not exceed 65535 UTF-8 bytes | **COVERED** | `dashboard_tile_creation.rs::body_content_within_65535_utf8_byte_limit` asserts `content.len() < MAX_MARKDOWN_BYTES` |

---

## Requirement 6: HitRegionNode Local Feedback

Spec requires pressed=true within p99 < 4ms of PointerDown; hovered=true on PointerEnter, cleared on PointerLeave; focus ring on focus; pressed cleared on PointerUp with release_on_up=true.

| Scenario | Status | Evidence |
|---|---|---|
| Button pressed state on pointer down (within p99 < 4ms, default press visual) | **COVERED** | `dashboard_tile_input.rs::pointer_down_at_refresh_returns_node_hit_with_refresh_interaction_id` and `pressed_state_set_within_4ms_p99_on_refresh` (100-sample p99 measurement vs calibrated budget) |
| Button hovered state on pointer enter (default 0.1 white overlay) | **COVERED** | `dashboard_tile_input.rs::hover_state_set_on_pointer_enter_dismiss` and `hover_state_set_on_pointer_enter_refresh` assert `hovered=true` and verify `local_patch` node_updates |
| Focus ring on focused button | **COVERED** | `dashboard_tile_input.rs::focus_ring_bounds_computed_in_display_space_for_refresh` and `focus_ring_produced_on_tab_navigation_to_dismiss` verify `FocusRingUpdate.bounds` in display-space |
| Pressed state cleared on pointer up (release_on_up=true) | **COVERED** | `dashboard_tile_input.rs::pointer_up_clears_pressed_and_releases_capture_on_refresh` and `pointer_up_clears_pressed_and_releases_capture_on_dismiss` assert `pressed=false`, capture released, `CaptureReleasedReason::PointerUp` |
| Hover visual (0.1 white overlay) rendered by compositor | **NOT COVERED** | Tests confirm `hovered=true` state and `local_patch` patch production, but do not assert the actual rendered pixel color or that a 0.1-white overlay is applied. This is a GPU-layer concern and may be out of scope for Layer 0 headless tests, but the spec explicitly names the visual. |
| Press visual (0.85 darkening) rendered by compositor | **NOT COVERED** | Same as above: `pressed=true` is asserted, but the 0.85 multiply darkening is not verified. Layer 0 tests cannot confirm compositor pixel output. |

---

## Requirement 7: Agent Callback on Button Activation

Spec requires ClickEvent (PointerDown + PointerUp) or CommandInputEvent(ACTIVATE) dispatched via gRPC EventBatch with interaction_id, tile_id, node_id, coordinates, device_id, timestamp_mono_us.

| Scenario | Status | Evidence |
|---|---|---|
| Click on Refresh button dispatches ClickEvent to agent | **COVERED** | `dashboard_tile_agent_callbacks.rs::click_on_refresh_delivers_click_event_with_refresh_interaction_id` asserts all required fields (interaction_id, tile_id, node_id, local_x, local_y, button) |
| ACTIVATE command on focused Dismiss button | **COVERED** | `dashboard_tile_agent_callbacks.rs::activate_on_dismiss_delivers_command_input_event_with_activate_and_keyboard_source` asserts interaction_id, action=ACTIVATE, source=KEYBOARD, tile_id, node_id |
| Agent handles refresh callback (submits content update MutationBatch) | **COVERED** | `dashboard_tile_agent_callbacks.rs::refresh_callback_triggers_mutation_batch_content_update` verifies the full round-trip: receive ClickEvent → send MutationBatch → receive MutationResult with echoed batch_id |
| Agent handles dismiss callback (LeaseRelease, tile removed) | **COVERED** | `dashboard_tile_agent_callbacks.rs::dismiss_callback_triggers_lease_release_and_tile_removal` verifies ClickEvent → LeaseRelease → LeaseResponse(granted=true) → LeaseStateChange(ACTIVE→RELEASED) |
| timestamp_mono_us field present and correct | **PARTIAL** | All tests set `timestamp_mono_us: 0` (deterministic placeholder). The spec requires the real monotonic clock value from the runtime; the tests assert field presence but not that the runtime populates this from an actual clock. |

---

## Requirement 8: Focus Cycling Between Buttons

Spec requires Tab (NAVIGATE_NEXT) cycles Refresh → Dismiss → wrap; Shift+Tab (NAVIGATE_PREV) reverses; FocusLostEvent/FocusGainedEvent dispatched; cross-tile focus; all buttons reachable without pointer.

| Scenario | Status | Evidence |
|---|---|---|
| Tab key cycles Refresh → Dismiss (FocusLost to Refresh, FocusGained to Dismiss) | **COVERED** | `dashboard_tile_input.rs::tab_advances_focus_from_refresh_to_dismiss` asserts FocusOwner change, FocusLostEvent(reason=TabKey), FocusGainedEvent(source=TabKey) with correct node_ids and namespace |
| Tab key wraps from Dismiss to Refresh | **COVERED** | `dashboard_tile_input.rs::tab_wraps_focus_from_dismiss_back_to_refresh` asserts wrap behavior and dispatches FocusLostEvent+FocusGainedEvent |
| Shift+Tab reverses (Dismiss → Refresh) | **COVERED** | `dashboard_tile_input.rs::shift_tab_cycles_focus_dismiss_to_refresh` and `shift_tab_wraps_refresh_to_dismiss` |
| All buttons reachable without pointer (NAVIGATE_NEXT + ACTIVATE) | **COVERED** | `dashboard_tile_input.rs::both_buttons_reachable_and_activatable_via_navigate_next_and_activate` and `dashboard_tile_agent_callbacks.rs::pointer_free_navigate_next_then_activate_delivers_same_callback_as_click` |
| Cross-tile Tab navigation (moves to next tile's first focusable) | **NOT COVERED** | Spec states "Cross-tile Tab navigation moves focus to the next tile's first focusable HitRegionNode." All focus cycling tests operate in a single-tile scene. No multi-tile focus cycling test exists in the exemplar deliverables. The FocusManager supports multi-tile in principle; the gap is exemplar test coverage. |

---

## Requirement 9: Lease Orphan Handling on Disconnect

Spec requires: ACTIVE→ORPHANED on disconnect, DisconnectionBadge within 1 frame, tile frozen; reconnect within 30s → ACTIVE, badge cleared; grace period expiry → EXPIRED, tile removed.

| Scenario | Status | Evidence |
|---|---|---|
| Disconnection triggers orphan state and badge (within 1 frame) | **COVERED** | `lease_governance.rs::disconnect_transitions_to_orphaned_and_sets_disconnection_badge` asserts LeaseState::Orphaned and TileVisualHint::DisconnectionBadge synchronously; `dashboard_tile_lifecycle.rs::disconnect_during_lifecycle_orphans_tile_with_badge` also covers this in a full-lifecycle context |
| Reconnection within grace period restores tile (ACTIVE, badge cleared, mutations accepted) | **COVERED** | `lease_governance.rs::reconnect_within_grace_period_restores_active_and_clears_badge` asserts LeaseState::Active, TileVisualHint::None, and that a subsequent tile creation succeeds; `reconnect_at_grace_period_boundary_is_accepted` tests the boundary |
| Grace period expiry removes tile (EXPIRED, tile removed from scene graph) | **COVERED** | `lease_governance.rs::grace_period_expiry_removes_tile_and_nodes` and `grace_period_expiry_removes_all_tile_nodes` assert tile/node removal; `dashboard_tile_lifecycle.rs::grace_period_expiry_removes_tile_after_disconnect` tests the full lifecycle path using `ORPHAN_GRACE_PERIOD_MS` constant |

---

## Requirement 10: Lease Expiry Without Renewal Removes Tile

Spec requires: TTL expiry (no renewal or auto-renewal disabled) → EXPIRED, tile and nodes removed, resources freed.

| Scenario | Status | Evidence |
|---|---|---|
| Lease expiry removes tile (TTL elapsed without renewal) | **COVERED** | `lease_governance.rs::ttl_expiry_without_renewal_removes_tile` advances clock 61000ms past 60000ms TTL, calls `expire_leases()`, asserts tile removed |
| Resources freed after expiry (ref count drops) | **COVERED** | `lease_governance.rs::resource_ref_count_drops_after_lease_expiry` asserts `lease_resource_usage().tiles=0` and `texture_bytes=0` after expiry. Note: this tests scene-layer `ResourceUsage` tracking, not `ResourceStore` ref-counts, with a note that the concrete ResourceStore ref-counting implementation is not yet in-tree. |

---

## Requirement 11: Z-Order Compositing at Content Layer

Spec requires dashboard tile at z_order=100 renders below zone tiles (≥0x8000_0000) and widget tiles (≥0x9000_0000), above tiles with z_order<100, chrome always on top. Hit-test respects z-order.

| Scenario | Status | Evidence |
|---|---|---|
| Dashboard tile below zone tiles | **COVERED** | `dashboard_tile_creation.rs::z_order_100_is_in_agent_owned_band_below_zone_tile_z_min` asserts `TILE_Z_ORDER < ZONE_TILE_Z_MIN` |
| Chrome layer above dashboard tile | **COVERED** | `dashboard_tile_creation.rs::chrome_z_order_renders_above_dashboard_tile` asserts `ZONE_TILE_Z_MIN + 1 > TILE_Z_ORDER` |
| Hit-test respects z-order (higher z wins in overlap) | **NOT COVERED** | The spec scenario requires a pointer event in the overlap region of the dashboard tile (z=100) and another agent tile (z=200) to hit the z=200 tile. No exemplar test constructs a two-tile scene and exercises hit-test z-order precedence in the overlap region. |

---

## Requirement 12: Headless Test Coverage

Spec requires all exemplar behaviors testable headlessly: hit-testing, node composition, lease state transitions, mutation validation, event dispatch.

| Scenario | Status | Evidence |
|---|---|---|
| Headless tile creation test (6 nodes, correct tree order, no GPU) | **COVERED** | `dashboard_tile_lifecycle.rs::headless_tile_creation_produces_6_nodes_in_correct_tree_order` |
| Headless input test (synthetic PointerDown returns NodeHit) | **COVERED** | `dashboard_tile_lifecycle.rs::headless_pointer_down_at_refresh_bounds_returns_node_hit` (also `dashboard_tile_input.rs` comprehensively) |
| Headless lease expiry test (time advancement → EXPIRED → tile removed) | **COVERED** | `dashboard_tile_lifecycle.rs::headless_lease_expiry_advances_to_expired_and_removes_tile` |

---

## Requirement 13: Full Lifecycle User-Test Scenario

Spec defines a 9-step lifecycle: session connect → lease with AutoRenew → resource upload → atomic tile creation → tile visible → periodic content update → Refresh click + callback → Dismiss click + lease release → tile removed.

| Scenario | Status | Evidence |
|---|---|---|
| End-to-end lifecycle completes successfully | **COVERED** | `dashboard_tile_lifecycle.rs::full_lifecycle_connect_lease_upload_create_update_refresh_dismiss` exercises all 6 scene-layer phases (lease, upload, create, update, Refresh, Dismiss). gRPC-layer handshake and EventBatch delivery are cross-covered by `dashboard_tile_agent.rs` and `dashboard_tile_agent_callbacks.rs`. |
| Disconnect during lifecycle triggers orphan path | **COVERED** | `dashboard_tile_lifecycle.rs::disconnect_during_lifecycle_orphans_tile_with_badge` and `grace_period_expiry_removes_tile_after_disconnect` |
| Namespace isolation during lifecycle | **COVERED** | `dashboard_tile_lifecycle.rs::second_agent_cannot_mutate_dashboard_tile` and `dashboard_agent_cannot_mutate_foreign_namespace_tile`; also covered at the scene-API level in `lease_governance.rs::second_agent_cannot_mutate_dashboard_tile` and `second_agent_cannot_delete_dashboard_tile` |
| Full gRPC session handshake as part of lifecycle (step 1) | **PARTIAL** | `dashboard_tile_agent.rs::exemplar_session_establishment_produces_session_established` covers the session protocol. The lifecycle test in `dashboard_tile_lifecycle.rs` proxies the session outcome (grants a lease directly on the scene graph) rather than exercising the full gRPC wire path from session init through tile creation in one test. The integration is real but not end-to-end in a single test. |

---

## Summary of Gaps

| # | Gap | Requirement | Severity |
|---|---|---|---|
| G-1 | Unknown resource (unregistered ResourceId) rejected with ResourceNotFound | Req 3 — Scenario: Unknown resource rejected | Medium — functional gap in exemplar coverage; the validation code exists but is not exercised by an exemplar-specific test |
| G-2 | Batch rejection does not assert `mutation_index` or `error_code` in the exemplar test | Req 2 — Scenario: Partial failure rejects entire batch | Low — `mutation_index` is asserted in `crates/tze_hud_scene/tests/batch_atomicity.rs`; the exemplar test only checks `rejection.is_some()` |
| G-3 | Batch size limit (<1000 mutations) boundary condition not explicitly tested | Req 2 — Scenario: Batch does not exceed mutation limits | Low — functional path passes; no boundary/limit test |
| G-4 | Cross-tile Tab navigation not tested (multi-tile focus cycling) | Req 8 — Scenario: Tab wraps from last to first (cross-tile clause) | Low — single-tile wrap is tested; cross-tile path has no exemplar test |
| G-5 | Hit-test z-order precedence (dashboard tile z=100 loses to z=200 tile in overlap) | Req 11 — Scenario: Hit-test respects z-order | Low — z-order ordering is asserted by constant comparison; actual hit-test with two overlapping tiles in the content band is not tested |
| G-6 | Compositor visual rendering not verified (0.85 press darkening, 0.1 hover overlay) | Req 6 — Scenarios: Button pressed/hovered state | Accepted — headless Layer 0 constraint; GPU compositor output cannot be asserted without a rendering pipeline; document as known boundary |
| G-7 | `timestamp_mono_us` in EventBatch set to 0 in all tests rather than real runtime clock value | Req 7 — Agent Callback event fields | Low — placeholder value; runtime behavior relies on compositor populating the field; no test verifies a non-zero value is emitted by the server |
| G-8 | ResourceStore ref-count (not scene-layer texture_bytes) is not verified after expiry | Req 10 — Scenario: Resources freed after expiry | Low — scene-layer `ResourceUsage` is asserted; the `ResourceStore` trait's concrete ref-counting is noted in the test as not yet having an in-tree implementation |

---

## Coverage Totals

- **Requirements:** 13 (11 explicit + Headless Coverage + Full Lifecycle)
- **Scenarios:** 37 total
  - **COVERED:** 27
  - **PARTIAL:** 5 (G-2, G-3, G-7, G-8, lifecycle gRPC proxy)
  - **NOT COVERED:** 5 (G-1, G-4, G-5, G-6 ×2)
- **Accepted non-coverage:** G-6 (compositor pixel output, headless constraint)
- **Action-recommended gaps:** G-1, G-4, G-5, and G-2 (assertion precision)
