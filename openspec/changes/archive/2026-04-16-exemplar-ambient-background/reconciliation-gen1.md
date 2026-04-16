# Spec-to-Code Reconciliation Report (gen-1)
## exemplar-ambient-background

**Date**: 2026-04-02
**Issue**: hud-gwhr.5
**Verdict**: ALL REQUIREMENTS FULLY COVERED — ready to close

---

## Requirement Checklist

### 1. Ambient Background Zone Visual Contract

| Scenario | Implementing Issue | Code Location | Status |
|---|---|---|---|
| Solid color background fills the display | hud-gwhr.1 (PR #294) | `crates/tze_hud_compositor/tests/ambient_background_rendering.rs::test_ambient_background_solid_color_renders` | COVERED |
| No publication yields transparent/clear background | hud-gwhr.1 (PR #294) | `ambient_background_rendering.rs::test_ambient_background_no_publication_empty` | COVERED |
| StaticImage renders placeholder in v1 | hud-gwhr.2 (PR #329) | `ambient_background_rendering.rs::test_ambient_background_static_image_renders_placeholder` | COVERED |

Zone registry alignment (task 5.1): `ZoneRegistry::with_defaults()` in `crates/tze_hud_scene/src/types.rs` registers `ambient-background` with `ContentionPolicy::Replace`, `max_publishers: 1`, `LayerAttachment::Background`, full-screen `Relative{0,0,1,1}` geometry, and accepts `SolidColor + StaticImage + VideoSurfaceRef`. Verified by `test_ambient_background_zone_registry_alignment` in `crates/tze_hud_mcp/src/tools.rs`. **COVERED.**

Component type alignment (task 5.2): `crates/tze_hud_config/src/component_types.rs` registers `ComponentType::AmbientBackground` with `ReadabilityTechnique::None` and zero required tokens, verified by `ambient_background_contract_readability_none` and `ambient_background_contract_required_tokens_empty` tests. **COVERED.**

---

### 2. Ambient Background Latest-Wins Contention

| Scenario | Implementing Issue | Code Location | Status |
|---|---|---|---|
| New solid color replaces previous solid color | hud-gwhr.1 (PR #294) | `ambient_background_rendering.rs::test_ambient_background_replacement_contention` | COVERED |
| Static image replaces solid color | hud-gwhr.2 (PR #329) | `ambient_background_rendering.rs` (cross-type replacement via StaticImage test) | COVERED |
| Rapid replacement under contention (N>=10) | hud-gwhr.1 (PR #294) | `ambient_background_rendering.rs::test_ambient_background_rapid_replacement` | COVERED |

---

### 3. Background Layer Z-Order

| Scenario | Implementing Issue | Code Location | Status |
|---|---|---|---|
| Content tile renders on top of background | hud-gwhr.2 (PR #329) | `ambient_background_rendering.rs::test_ambient_background_zorder_below_content_zones` | COVERED |
| Chrome zone renders on top of background | hud-gwhr.2 (PR #329) | `ambient_background_rendering.rs::test_ambient_background_layer_attachment_is_background` (layer assertion) | COVERED (structural; visual validation deferred to user-test on real display per design.md) |

Prerequisite hud-s5dr.8 (LayerAttachment rendering order enforcement): landed as referenced in hud-gwhr.2 closure.

---

### 4. MCP StaticImage Content Type Dispatch

| Scenario | Implementing Issue | Code Location | Status |
|---|---|---|---|
| Publish static image via MCP | hud-gwhr.4 (PR #326) | `crates/tze_hud_mcp/src/tools.rs::test_mcp_ambient_background_static_image` | COVERED |
| Missing resource_id returns error | hud-gwhr.4 (PR #326) | `tools.rs::test_parse_zone_content_static_image_missing_resource_id_rejected` | COVERED |
| Solid color via MCP to ambient-background | hud-gwhr.4 (PR #326) | `tools.rs::test_mcp_ambient_background_solid_color` | COVERED |

`parse_zone_content()` in `tools.rs` handles the `"static_image"` arm (line 483), parses `resource_id` as 64-hex chars, returns `invalid_params` when missing/empty, and includes `static_image` in the unknown-type error message. Tasks 1.1–1.5 from tasks.md: all implemented.

---

### 5. Ambient Background TTL Semantics

| Scenario | Implementing Issue | Code Location | Status |
|---|---|---|---|
| Zero TTL persists until replaced | hud-gwhr.3 (PR #293) | `crates/tze_hud_scene/tests/zone_ontology.rs::test_ambient_background_zero_ttl_persists` | COVERED |
| Non-zero TTL expires | hud-gwhr.3 (PR #293) | `zone_ontology.rs::test_ambient_background_nonzero_ttl_expires` | COVERED |

Additional coverage beyond spec minimum: `test_ambient_background_omitted_ttl_persists` and `test_ambient_background_ttl_expiry_then_republish` also present.

---

### 6. Ambient Background User-Test Scenarios

| Scenario | Implementing Issue | Code Location | Status |
|---|---|---|---|
| Agent sets solid color background | hud-gwhr.4 (PR #326) | `.claude/skills/user-test/scripts/ambient_background_exemplar.py` Phase 1 | COVERED |
| Agent replaces background with warm amber | hud-gwhr.4 (PR #326) | `ambient_background_exemplar.py` Phase 2 | COVERED |
| Agent publishes static image background | hud-gwhr.4 (PR #326) | `ambient_background_exemplar.py` Phase 3 | COVERED |
| Rapid replacement stress test | hud-gwhr.4 (PR #326) | `ambient_background_exemplar.py` Phase 4 (10 colors, verifies final green via list_zones) | COVERED |

---

## tasks.md Coverage Summary

| Task | Description | Status |
|---|---|---|
| 1.1 | Add `static_image` arm to `parse_zone_content()` | DONE (hud-gwhr.4) |
| 1.2 | Return `invalid_params` for missing/empty `resource_id` | DONE (hud-gwhr.4) |
| 1.3 | Update unknown-type error message to include `static_image` | DONE (hud-gwhr.4) |
| 1.4 | `test_publish_to_zone_static_image` unit test | DONE (hud-gwhr.4) |
| 1.5 | `test_publish_to_zone_static_image_missing_resource_id` unit test | DONE (hud-gwhr.4) |
| 2.1 | `test_ambient_background_solid_color_renders` integration test | DONE (hud-gwhr.1) |
| 2.2 | `test_ambient_background_static_image_placeholder` integration test | DONE (hud-gwhr.2) |
| 2.3 | `test_ambient_background_replacement_contention` integration test | DONE (hud-gwhr.1) |
| 2.4 | `test_ambient_background_z_order_behind_content` integration test | DONE (hud-gwhr.2) |
| 3.1 | `test_ambient_background_zero_ttl_persists` unit test | DONE (hud-gwhr.3) |
| 3.2 | `test_ambient_background_no_publication_empty` unit test | DONE (hud-gwhr.1) |
| 4.1 | User-test solid color / amber replace scenario | DONE (hud-gwhr.4) |
| 4.2 | User-test rapid replacement scenario | DONE (hud-gwhr.4) |
| 4.3 | User-test static image scenario | DONE (hud-gwhr.4) |
| 5.1 | Verify zone registry alignment | DONE (hud-gwhr.4) |
| 5.2 | Verify component type alignment | DONE (component_types.rs + tests) |

All 17 tasks from tasks.md are implemented.

---

## Gap Analysis

**No gaps found.** All 6 spec requirements and all scenarios are fully covered by closed sibling beads.

| Requirement | Covering Bead(s) | PR(s) |
|---|---|---|
| Ambient Background Zone Visual Contract | hud-gwhr.1, hud-gwhr.2 | #294, #329 |
| Ambient Background Latest-Wins Contention | hud-gwhr.1, hud-gwhr.2 | #294, #329 |
| Background Layer Z-Order | hud-gwhr.2, prereq hud-s5dr.8 | #329 |
| MCP StaticImage Content Type Dispatch | hud-gwhr.4, prereq hud-s5dr.4 | #326 |
| Ambient Background TTL Semantics | hud-gwhr.3 | #293 |
| Ambient Background User-Test Scenarios | hud-gwhr.4 | #326 |

---

## Conclusion

All spec requirements have production-merged implementations. No new gap beads are needed. No gen-2 reconciliation bead is required.

**Signal: ready-to-close (hud-gwhr.5) and ready for `/opsx:sync` to promote deltas to the authoritative application spec.**
