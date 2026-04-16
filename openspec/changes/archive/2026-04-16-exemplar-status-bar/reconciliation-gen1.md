# Spec-to-Code Reconciliation Report — exemplar-status-bar (gen-1)

**Date:** 2026-04-02
**Issue:** hud-t1in.6
**Spec:** `openspec/changes/exemplar-status-bar/specs/exemplar-status-bar/spec.md`

---

## Summary

All five sibling implementation beads (hud-t1in.1 through hud-t1in.5) are closed. This
report maps every spec requirement and scenario to its implementing bead, notes the
files/tests that satisfy each, and records two minor gaps discovered during audit.

**Verdict: Full coverage. No gap beads required. One minor discovered issue filed separately.**

---

## Requirement Checklist

### §Status Bar Visual Specification → hud-t1in.1

| Scenario | Status | Evidence |
|---|---|---|
| Status bar renders at bottom edge with opaque backdrop | COVERED | `profiles/exemplar-status-bar/zones/status-bar.toml`: `backdrop_opacity = 0.9`, `backdrop_color = "{{color.backdrop.default}}"`. Zone default in `types.rs:2125` uses `DisplayEdge::Bottom, height_pct: 0.04, width_pct: 1.0, margin_px: 0.0`. |
| Status bar readability passes OpaqueBackdrop check | COVERED | `crates/tze_hud_config/src/component_profiles.rs:1791` — `exemplar_status_bar_opaque_backdrop_readability_passes` test verifies opacity 0.9 >= 0.8 threshold. |
| Status bar chrome layer renders above content tiles | COVERED | `crates/tze_hud_scene/src/types.rs:2138` — `layer_attachment: LayerAttachment::Chrome`. `crates/tze_hud_scene/tests/zone_ontology.rs:614` — `default_zones_have_correct_layer_attachments` asserts `status-bar` maps to `Chrome`. `crates/tze_hud_compositor/src/renderer.rs:3488` — `test_chrome_always_above_max_zorder_tile` proves Chrome layer always renders above max-z-order content tile. |

### §Status Bar Component Profile → hud-t1in.1

| Scenario | Status | Evidence |
|---|---|---|
| Profile loads with correct component type | COVERED | `profiles/exemplar-status-bar/profile.toml`: `component_type = "status-bar"`. `crates/tze_hud_config/src/component_profiles.rs:1770` — `exemplar_status_bar_profile_loads_without_errors` integration test. |
| Token references resolve from global token map | COVERED | `profiles/exemplar-status-bar/zones/status-bar.toml` uses `{{color.text.secondary}}`, `{{color.backdrop.default}}`, `{{typography.body.size}}`, `{{spacing.padding.medium}}`. `profile.toml` `[token_overrides]` section provides canonical fallback values. Integration test constructs token map and exercises resolution. |
| Profile activatable in configuration | COVERED | Profile directory `profiles/exemplar-status-bar/` follows the component-shape-language convention; `profile.toml` includes `name`, `version`, `component_type`, and `description`. Test at `component_profiles.rs:1770` loads the directory and asserts fields. |

### §Merge-by-Key Contention Semantics → hud-t1in.2

All 5 spec scenarios plus the Coalesced Update Delivery scenario are covered by 6 tests in
`crates/tze_hud_scene/tests/zone_ontology.rs` (PR #274):

| Scenario | Test | Line |
|---|---|---|
| Three agents publish different keys — all coexist | `exemplar_status_bar_three_agents_coexist` | 1456 |
| Agent updates its own key — value replaced | `exemplar_status_bar_key_update_replaces` | 1519 |
| Key removed by empty value publish | `exemplar_status_bar_key_removal_empty_value` | 1583 |
| Key removed by TTL expiry | `exemplar_status_bar_key_removal_ttl_expiry` | 1640 |
| Max keys eviction | `exemplar_status_bar_max_keys_eviction` | 1714 |

### §Coalesced Update Delivery → hud-t1in.2

| Scenario | Test | Line |
|---|---|---|
| Rapid updates within one frame coalesce | `exemplar_status_bar_rapid_updates_coalesce_to_latest` | 1784 |

### §Multi-Agent Coexistence → hud-t1in.3

All 3 spec scenarios are covered by 2 tests in `zone_ontology.rs` (PR #276). Note: the
first scenario ("Three agents with distinct namespaces publish simultaneously") is
validated as a precondition inside both isolation tests and is also directly covered by
`exemplar_status_bar_three_agents_coexist`.

| Scenario | Test | Line |
|---|---|---|
| Three agents with distinct namespaces publish simultaneously | `exemplar_status_bar_three_agents_coexist` (shared with contention suite) | 1456 |
| One agent's clear does not affect other agents | `exemplar_status_bar_clear_per_publisher` | 1195 |
| Agent lease expiry removes only that agent's publications | `exemplar_status_bar_lease_expiry_isolation` | 1303 |

### §MCP Integration Test Scenarios → hud-t1in.4

All 4 spec scenarios are covered by 4 tests in
`crates/tze_hud_mcp/tests/publish_to_zone_status_bar.rs` (PR #315). Note: tasks.md §4
listed only 3 tasks; the 4th scenario (key removal via empty value) was added during
t1in.4 implementation — good.

| Scenario | Test | Line |
|---|---|---|
| MCP publish single status key | `test_status_bar_single_key_publish` | 74 |
| MCP publish from multiple agents with different keys | `test_status_bar_multi_agent_publish` | 136 |
| MCP key update preserves other keys | `test_status_bar_key_update` | 208 |
| MCP key removal via empty value | `test_status_bar_key_removal_via_empty_value` | 310 |

### §User-Test Scenario → hud-t1in.5

All 10 steps are covered by `.claude/skills/user-test/scripts/status_bar_exemplar.py`
(PR #324):

| Steps | Coverage |
|---|---|
| Steps 1–3: three agents publish weather/battery/time | `step1_weather_initial`, `step2_battery_initial`, `step3_time_initial` |
| Step 4: visual check — all 3 visible | `step4_visual_check_all_visible` |
| Step 5: agent A updates weather | `step5_weather_update` |
| Step 6: visual check — weather updated | `step6_visual_check_weather_updated` |
| Step 7: agent A publishes empty value for weather | `step7_weather_empty` |
| Step 8: visual check — weather gone | `step8_visual_check_weather_gone` |
| Step 9: wait for agent B TTL expiry | `step9_wait_battery_ttl` |
| Step 10: visual check — battery gone | `step10_visual_check_battery_gone` |

---

## Gaps / Discovered Issues

### Gap 1: `make_status_bar_zone()` test helper uses `max_keys: 8` instead of spec-required `max_keys: 32`

- **Location:** `crates/tze_hud_scene/src/graph.rs:3882`
- **Spec requirement:** §Merge-by-Key Contention Semantics: `MergeByKey { max_keys: 32 }`
- **Production default** (`types.rs:2133`) correctly uses `max_keys: 32`.
- The `make_status_bar_zone()` helper is used only in `graph.rs` unit tests (not in the
  exemplar integration tests, which use `ZoneRegistry::with_defaults()`). The discrepancy
  means any tests that use this helper are testing a weaker contention limit. Not a
  correctness bug in the exemplar path, but the helper should match the spec.
- **Suggested:** `task`, P3 (low — production path is correct; only test helper is wrong).

### Gap 2: OpenSpec canonical specs directory not yet synced

- **Location:** `openspec/specs/` — directory contains only `exemplar-notification/`; no
  `exemplar-status-bar/` entry.
- **Spec task §6.1–6.2** and issue description step 8 require running `/opsx:sync` to
  copy the change-level spec into the authoritative `openspec/specs/` directory once all
  requirements are covered.
- This is a process step, not a code gap. The spec content exists correctly in
  `openspec/changes/exemplar-status-bar/specs/exemplar-status-bar/spec.md`.
- **Action:** Run `/opsx:sync` for the `exemplar-status-bar` change as part of this
  reconciliation or a follow-up task.

---

## Spec Coverage Matrix

| Requirement Section | Sibling Bead | Status |
|---|---|---|
| §Status Bar Visual Specification | hud-t1in.1 | FULL |
| §Status Bar Component Profile | hud-t1in.1 | FULL |
| §Merge-by-Key Contention Semantics (5 scenarios) | hud-t1in.2 | FULL |
| §Coalesced Update Delivery (1 scenario) | hud-t1in.2 | FULL |
| §Multi-Agent Coexistence (3 scenarios) | hud-t1in.3 | FULL |
| §MCP Integration Test Scenarios (4 scenarios) | hud-t1in.4 | FULL |
| §User-Test Scenario (10 steps) | hud-t1in.5 | FULL |

**All 7 requirement sections: FULL COVERAGE. No gap beads required.**

---

## Next Steps

1. Fix `make_status_bar_zone()` test helper (`max_keys: 8` → `max_keys: 32`) — P3 task,
   not a blocker for this reconciliation.
2. Run `/opsx:sync` for the `exemplar-status-bar` change to promote the delta spec to the
   authoritative `openspec/specs/` directory.
3. Close hud-t1in.6 with this reconciliation summary.
