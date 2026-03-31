# Exemplar Presence Card — Spec-to-Code Coverage Report

**Issue**: hud-apoe.5
**Date**: 2026-03-31
**Spec**: `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md`
**Sibling beads**: hud-apoe.1 (closed), hud-apoe.2 (closed), hud-apoe.3 (blocked), hud-apoe.4 (blocked)

---

## Summary

The spec defines 10 requirements and 24 scenarios. The work delivered by
completed beads (hud-apoe.1 and hud-apoe.2) provides solid coverage of the
tile construction and lease lifecycle requirements. The two blocked beads
(hud-apoe.3 and hud-apoe.4) leave meaningful gaps: the periodic content
update loop, the full three-agent concurrent test with timestamp formatting,
the heartbeat-based disconnect detection integration test, and the reconnect
within grace period scenario are all untested or only partially covered.
Badge rendering infrastructure exists and is tested at the chrome layer, but
no integration test wires badge state to the lease `ORPHANED` transition in
the context of the presence card exemplar.

| Requirement | Status |
|---|---|
| R1: Presence Card Tile Geometry | Covered |
| R2: Multi-Agent Vertical Stacking | Covered |
| R3: Presence Card Node Tree | Covered |
| R4: Lease Lifecycle for Presence Cards | Covered |
| R5: Periodic Content Update | Partial |
| R6: Agent Disconnect and Orphan Handling | Partial |
| R7: Multi-Agent Isolation During Disconnect | Partial |
| R8: Resource Upload for Avatar Icons | Covered |
| R9: gRPC Test Sequence | Partial |
| R10: User-Test Scenario | Covered (this bead) |

**Overall: 5 requirements fully covered, 4 partially covered, 0 missing.**

---

## Requirement 1: Presence Card Tile Geometry

**Status: COVERED**

Spec requires: 200x80 tile, anchored bottom-left with 16px margins, opacity
1.0, input mode Passthrough.

**Test coverage** (`tests/integration/presence_card_tile.rs`):

- `presence_card_y_offsets_match_spec` — verifies y-offset formula for all 3
  agent indices against the spec values (tab_height − 96, − 184, − 272).
- `presence_card_bounds_no_overlap` — verifies tile y-ranges are disjoint for
  all 3 agents.
- `presence_card_z_orders_sequential_and_below_zone_min` — verifies z_orders
  are 100/101/102 and all < `ZONE_TILE_Z_MIN`.
- `create_tile_batch_accepted_with_opacity_and_input_mode` — verifies
  `CreateTile` + `UpdateTileOpacity(1.0)` + `UpdateTileInputMode(Passthrough)`
  batch is accepted and tile has correct fields.
- `full_presence_card_batch_visible_in_snapshot` — verifies `opacity = 1.0`,
  `input_mode = Passthrough` in `SceneSnapshot`.

Passthrough hit-test behaviour is covered by the general hit-test suite
(`crates/tze_hud_scene/tests/hit_test.rs`):
- `passthrough_tile_skipped_reveals_tile_below`
- `widget_passthrough_skips_to_agent_tile_below`

**Gaps:** None.

---

## Requirement 2: Multi-Agent Vertical Stacking

**Status: COVERED**

Spec requires: 3 agents with distinct y-offsets (−96, −184, −272), 8px gaps,
z_orders 100/101/102, no overlap, all z-orders below `ZONE_TILE_Z_MIN`.

**Test coverage**:

- `presence_card_y_offsets_match_spec` — y-offset formula verified.
- `presence_card_bounds_no_overlap` — non-overlap verified.
- `presence_card_z_orders_sequential_and_below_zone_min` — z-order values and
  `ZONE_TILE_Z_MIN` constraint verified.
- `three_agents_non_overlapping_presence_cards` — 3 agents create tiles with
  unique namespaces; verifies 3 tiles in scene, all non-overlapping, each
  namespace owns only its tile.

**Gaps:** None. (The spec note about `ZOrderConflict` not applying to
Passthrough tiles is an implementation detail; the z-order values are
validated by the `<ZONE_TILE_Z_MIN` assertion.)

---

## Requirement 3: Presence Card Node Tree

**Status: COVERED**

Spec requires: flat 3-node tree — `SolidColorNode` (bg, `Rgba{0.08,0.08,0.08,0.78}`,
full tile bounds) → `StaticImageNode` (avatar 32x32 at (8,24), Cover) +
`TextMarkdownNode` (agent name bold + "Last active: now", 14px, near-white,
Ellipsis, at (48,8), 144x64).

**Test coverage** (`tests/integration/presence_card_tile.rs`):

- `node_tree_builder_three_nodes` — verifies:
  - Root is `SolidColorNode` with correct bounds and color (`r=0.08, a=0.78`).
  - Two children: `StaticImageNode` (first) and `TextMarkdownNode` (second).
  - `StaticImageNode` has correct `resource_id`, 32x32 dimensions, `Cover`
    fit mode, and bounds `(8, 24, 32, 32)`.
  - `TextMarkdownNode` contains agent name + "Last active: now", `font_size_px
    = 14.0`, color `r=0.94, a=1.0`, `overflow = Ellipsis`, bounds
    `(48, 8, 144, 64)`.
  - Total 3 nodes in scene.
- `full_presence_card_batch_visible_in_snapshot` — verifies all 3 node types
  appear in `SceneSnapshot` after batch submission.

**Gaps:** None.

---

## Requirement 4: Lease Lifecycle for Presence Cards

**Status: COVERED**

Spec requires: `ttl_ms = 120000`, capabilities `[create_tiles, modify_own_tiles]`,
`AutoRenew` policy at 75% TTL (90s), REQUESTED → ACTIVE transition,
mutation rejection on expired/no lease.

**Test coverage** (`crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs`):

- `test_presence_card_lease_request_granted` — verifies `REQUESTED → ACTIVE`
  transition, non-nil `LeaseId`, correct `ttl_ms`, correct capabilities.
- `test_presence_card_auto_renew_fires_at_75_percent_ttl` — verifies
  `TtlCheck::AutoRenewDue` fires at exactly 90,000ms, is one-shot per window,
  and re-arms after `reset_renewal_window`.
- `test_auto_renew_not_applicable_for_other_policies` — confirms `AutoRenew`
  is specific; Manual/OneShot policies do not fire `AutoRenewDue`.
- `test_auto_renew_disarmed_during_budget_warning` — verifies server-side
  disarm/rearm semantics.
- `test_mutation_rejected_with_expired_lease` — verifies `CreateTile` is
  rejected (not silent) when lease is `Revoked`.
- `test_mutation_rejected_after_ttl_expiry` — verifies rejection after real
  TTL elapsed; also verifies tile is removed on expiry sweep.
- `test_mutation_rejected_with_no_lease` — verifies `LeaseNotFound` on
  non-existent `lease_id`.
- `test_presence_card_tile_binds_to_lease` — verifies `tile.lease_id` binding,
  tile removal on lease revocation.
- `test_presence_card_lease_state_machine_transitions` — verifies observable
  `REQUESTED → ACTIVE → ORPHANED → EXPIRED` transitions with JSON audit trail.
- `test_auto_renew_ttl_reset_reflects_new_window` — verifies remaining TTL
  resets to ~120s after `reset_renewal_window`.

**Gaps:** The spec says "Tile creation and node mutations MUST be rejected if
the lease is not ACTIVE." Revoked/Expired and missing-lease cases are tested.
The `ORPHANED` rejection case is covered by
`test_presence_card_full_lifecycle_integration` (step 5 of that test verifies
`SetTileRoot` is rejected while lease is `Orphaned`).

---

## Requirement 5: Periodic Content Update

**Status: PARTIAL**

Spec requires: `SetTileRoot` every 30s with full rebuilt tree;
`TextMarkdownNode` updated to "Last active: Ns ago" with human-friendly
formatting ("now", "30s ago", "1m ago", "2m ago"); `SolidColorNode` and
`StaticImageNode` included unchanged; batch contains exactly 1 mutation.

**Test coverage**:

- `test_presence_card_full_lifecycle_integration` (in
  `crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs`) — covers the
  30s content update mechanically: advances clock 30s, submits a `SetTileRoot`
  batch, and verifies it is accepted. However, this test uses a `SolidColorNode`
  (not a full 3-node tree) and does not verify the human-friendly time string.

**Gaps (hud-apoe.3 scope):**

- No test verifies the human-friendly time formatting ("now", "30s ago",
  "1m ago", "2m ago").
- No test verifies the `SetTileRoot` batch contains exactly 1 mutation and
  includes all 3 nodes (SolidColor + StaticImage + TextMarkdown) with only the
  text node content changed.
- No test covers the "content update after 90 seconds" scenario (text reads
  "Last active: 1m ago").
- The concurrent 3-agent content update test (tasks 5.4) is not implemented.
  Namespace isolation during concurrent updates is only verified at the
  single-agent level in the full-lifecycle test.

---

## Requirement 6: Agent Disconnect and Orphan Handling

**Status: PARTIAL**

Spec requires: disconnect via gRPC stream close or heartbeat timeout (15s);
`ACTIVE → ORPHANED`; tile frozen; disconnection badge within 1 frame;
30s grace period; `ORPHANED → EXPIRED`; tile removed.

**Test coverage**:

- `test_presence_card_lease_state_machine_transitions` — verifies
  `disconnect_lease()` transitions lease to `Orphaned`, tile persists during
  grace, `expire_leases()` transitions to `Expired` and removes tile.
- `test_presence_card_full_lifecycle_integration` — verifies mutation rejection
  while `Orphaned`, grace period expiry, tile removal; tile count returns to 0.
- `crates/tze_hud_runtime/src/shell/badges.rs` (unit tests):
  - `spec_disconnection_badge_appears_on_disconnect` — verifies
    `build_badge_cmds` produces scrim + badge_bg + icon commands when
    `disconnected = true`.
  - `spec_disconnection_badge_clears_on_reconnect` — verifies no draw commands
    when `disconnected = false`.
  - `disconnection_badge_stays_within_tile_bounds`, icon opacity, scrim alpha
    tests — visual rendering constants verified.
- `crates/tze_hud_protocol/tests/heartbeat.rs`:
  - `heartbeat_interval_default_is_5000ms` — 5s heartbeat interval.
  - `orphan_timeout_is_three_times_heartbeat_interval` — 15s timeout.
  - `heartbeat_missed_threshold_is_3` — 3 missed heartbeats.
  - `missed_heartbeat_counter_triggers_at_threshold` — counter logic.

**Gaps (hud-apoe.4 scope):**

- No integration test wires the badge's `disconnected` flag to the presence
  card lease's `Orphaned` state. The badge unit tests and the lease unit tests
  are separate; the bridge (control plane sets `TileBadgeState.disconnected =
  true` when a lease transitions to `Orphaned`) is not end-to-end tested for
  this exemplar.
- The "disconnection badge appears within 1 frame" timing assertion is not
  tested programmatically (only stated in the badge module's documentation
  contract and manual user-test step 5).
- Reconnect within grace period is tested at the badge layer
  (`spec_disconnection_badge_clears_on_reconnect`) but NOT as an integration
  scenario: there is no test that (a) creates a presence card, (b) disconnects
  the agent, (c) reconnects within 30s, and (d) verifies `ORPHANED → ACTIVE`
  transition and badge clearance.
- Heartbeat-triggered disconnect detection (as opposed to `disconnect_lease()`
  API call) is not tested end-to-end for the presence card scenario.

---

## Requirement 7: Multi-Agent Isolation During Disconnect

**Status: PARTIAL**

Spec requires: agents A and B continue unaffected while C is disconnected;
no automatic repositioning of A/B's tiles after C's removal.

**Test coverage**:

- `test_presence_card_namespace_isolation` — verifies:
  - All 3 tiles present before disconnect.
  - Bravo disconnects; alpha and charlie remain `Active`.
  - Bravo's grace period expires; bravo's tile removed.
  - Alpha's and charlie's tiles survive bravo's expiry.
  - Alpha's and charlie's leases remain `Active`.
  - Both remaining agents can still submit `SetTileRoot` mutations after bravo
    is removed.

**Gaps (hud-apoe.4 scope):**

- The test above uses generic bounds (not the spec's exact y-offsets) and does
  not verify that alpha and charlie's tiles remain at their **original
  y-positions** (y = tab_height − 96 and y = tab_height − 184). It only
  verifies tile count and lease state.
- There is no test verifying that the runtime does NOT reposition remaining
  tiles after C's removal (the spec explicitly states "no automatic
  repositioning SHALL occur"). A position-equality assertion is missing.

---

## Requirement 8: Resource Upload for Avatar Icons

**Status: COVERED**

Spec requires: 32x32 PNG upload returns BLAKE3 `ResourceId`; three placeholder
colors (blue RGB 66,133,244; green RGB 52,168,83; orange RGB 251,188,4);
duplicate upload returns same `ResourceId`.

**Test coverage** (`tests/integration/presence_card_tile.rs`):

- `avatar_fixtures_are_non_empty` — verifies 3 distinct PNG byte arrays are
  produced; distinct colors produce distinct bytes.
- `avatar_upload_returns_resource_id` — verifies `ResourceId` equals BLAKE3
  hash of the raw PNG bytes.
- `avatar_upload_deduplicates` — verifies two agents uploading the same PNG
  receive the same `ResourceId` (content deduplication).
- `three_avatar_uploads_distinct_resource_ids` — verifies all 3 avatar colors
  produce distinct `ResourceId`s.

**Gaps:** None.

---

## Requirement 9: gRPC Test Sequence

**Status: PARTIAL**

Spec defines a canonical 7-step gRPC sequence per agent and a 3-agent
concurrent version. Steps: SessionInit → LeaseRequest → UploadResource →
MutationBatch (CreateTile + SetTileRoot + UpdateTileOpacity + UpdateTileInputMode)
→ 30s wait → SetTileRoot (update) → repeat, ending with SessionClose or drop.

**Test coverage**:

The individual steps of the sequence are tested in isolation:
- SessionInit/LeaseRequest: `test_presence_card_lease_request_granted`.
- UploadResource: `avatar_upload_returns_resource_id`,
  `avatar_upload_deduplicates`.
- MutationBatch (CreateTile + nodes + opacity + input_mode):
  `create_tile_batch_accepted_with_opacity_and_input_mode`,
  `full_presence_card_batch_visible_in_snapshot`.
- Periodic SetTileRoot: `test_presence_card_full_lifecycle_integration`
  (step 4).
- SessionClose/disconnect: state machine transition tests.
- Three-agent concurrent scenario: `three_agents_non_overlapping_presence_cards`
  (creation only; no concurrent periodic updates).

**Gaps (hud-apoe.3/4 scope):**

- No single test executes the complete 7-step sequence end-to-end for one
  agent over a simulated 120s lifetime.
- The three-agent concurrent sequence is partially covered (creation) but
  not the concurrent update loop (tasks 5.4).
- The test sequence does not verify the `SceneSnapshot` contains the correct
  per-agent namespaces, z-orders, and y-offsets simultaneously for all 3 agents
  (only single-agent snapshot accuracy is verified).

---

## Requirement 10: User-Test Scenario

**Status: COVERED**

Spec requires: documented 7-step user-test script with visual verification
and pass/fail criteria.

**Deliverable**: `docs/exemplar-presence-card-user-test.md` (this bead,
hud-apoe.5) defines the complete 7-step user-test scenario with per-step
pass criteria, fail criteria, and the final scenario verdict table.

**Gaps:** None (this is a documentation-only requirement).

---

## Gap Summary

The following spec scenarios are not yet covered by automated tests:

| Gap | Spec Scenario | Responsible Bead |
|-----|--------------|-----------------|
| G1 | "Content update after 90 seconds" — text reads "Last active: 1m ago" | hud-apoe.3 |
| G2 | "Only text node is replaced" — batch has exactly 1 SetTileRoot mutation with unchanged bg + avatar | hud-apoe.3 |
| G3 | Time formatting "now"→"30s ago"→"1m ago"→"2m ago" verified in test | hud-apoe.3 |
| G4 | 3-agent concurrent content updates verified simultaneously | hud-apoe.3 |
| G5 | "Disconnection badge within 1 frame" — lease ORPHANED → badge flag set in same render cycle | hud-apoe.4 |
| G6 | "Reconnect within grace period reclaims lease" — ORPHANED → ACTIVE + badge clears | hud-apoe.4 |
| G7 | "Disconnect detection via heartbeat timeout" — 15s detection integration test | hud-apoe.4 |
| G8 | "Tile frozen during orphan state" + "no mutations accepted" — presence-card-specific integration | hud-apoe.4 |
| G9 | "Disconnected agent tile removal does not shift others" — y-position equality assertion | hud-apoe.4 |
| G10 | Full 7-step gRPC sequence executed end-to-end for single agent | hud-apoe.3/4 |

**Gaps G1–G4** are in scope for hud-apoe.3 (currently blocked).
**Gaps G5–G10** are in scope for hud-apoe.4 (currently blocked).

No gaps exist for requirements R1, R2, R3, R4, R8, R10.
