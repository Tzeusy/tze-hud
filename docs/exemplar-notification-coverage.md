# Exemplar Notification — Spec-to-Code Coverage Report

**Issue:** hud-j5g5.7 (gen-1 reconciliation)
**Spec:** `openspec/changes/exemplar-notification/specs/exemplar-notification/spec.md`
**Audit date:** 2026-04-02

## Deliverable Index

| File / Artifact | Issue | PR | Purpose |
|---|---|---|---|
| `crates/tze_hud_scene/src/types.rs` (zone defaults) | hud-j5g5.1 | #279 | max_depth=5, auto_clear_ms=8000 |
| `crates/tze_hud_compositor/src/renderer.rs` (slot layout) | hud-j5g5.1 | #279 | Per-notification vertical slot positioning |
| `crates/tze_hud_compositor/src/renderer.rs` (urgency backdrop + borders) | hud-j5g5.2 | #287 | Token-resolved urgency colors, 4-quad borders |
| `crates/tze_hud_compositor/src/renderer.rs` (text + TTL fade) | hud-j5g5.3 | #288 | Typography tokens, per-publication fade-out |
| `profiles/notification-stack-exemplar/profile.toml` | hud-j5g5.4 | (on main) | Exemplar component profile with urgency token overrides |
| `profiles/notification-stack-exemplar/zones/notification-area.toml` | hud-j5g5.4 | (on main) | Zone rendering policy overrides |
| `crates/tze_hud_mcp/src/tools.rs` (integration tests) | hud-j5g5.5 | #310 | 5 MCP integration tests: ordering, eviction, TTL, urgency, independence |
| `.claude/skills/user-test/scripts/notification_exemplar.py` | hud-j5g5.6 | #317 | 4-phase user-test scenario script |

---

## Requirement 1: Notification Stack Vertical Layout

| Scenario | Status | Evidence |
|---|---|---|
| Single notification renders at top of zone | **COVERED** | hud-j5g5.1 (#279): `slot_height = font_size_px + 18`; slot_index=0 positions at zone_y. Verified by compositor slot layout tests in PR #279. |
| Three notifications stack vertically newest-on-top | **COVERED** | hud-j5g5.1 (#279): Iteration is newest-first; slot_index 0=newest, 1=middle, 2=oldest. y_offset = zone_y + slot_index * slot_height. |
| Notifications exceeding zone height are clipped | **COVERED** | hud-j5g5.1 (#279): Clipping at zone boundary unchanged from prior logic; notifications whose y_offset + slot_height > zone_y + zone_height are skipped. |

---

## Requirement 2: Urgency-Tinted Notification Backdrops

| Scenario | Status | Evidence |
|---|---|---|
| Low urgency (0) renders dark gray (#2A2A2A) backdrop at 0.9 opacity | **COVERED** | hud-j5g5.2 (#287): token fallback `color.notification.urgency.low` = `#2A2A2A`, fixed 0.9 opacity per-notification. |
| Normal urgency (1) renders dark blue (#1A1A3A) backdrop at 0.9 opacity | **COVERED** | hud-j5g5.2 (#287): token fallback `#1A1A3A`. |
| Urgent (2) renders amber (#8B6914) backdrop at 0.9 opacity | **COVERED** | hud-j5g5.2 (#287): token fallback `#8B6914`. |
| Critical (3) renders red (#8B1A1A) backdrop at 0.9 opacity | **COVERED** | hud-j5g5.2 (#287): token fallback `#8B1A1A`. |
| Out-of-range urgency (5) clamped to critical | **COVERED** | hud-j5g5.2 (#287): urgency >= 3 clamped at critical (3) before color lookup. Integration test in hud-j5g5.5 (#310) verifies urgency=5 storage; compositor clamps at render time. |
| Notification-area does not use `color.severity.*` tokens | **COVERED** | hud-j5g5.2 (#287): Urgency color lookup uses `color.notification.urgency.*` exclusively; `color.severity.*` namespace not referenced. |

Note: Token resolution from active notification profile wired by hud-fpof (#313), pixel-level verification added by hud-oj2x (#312) — post-j5g5 issues that further strengthen coverage.

---

## Requirement 3: Notification Border Rendering

| Scenario | Status | Evidence |
|---|---|---|
| Notification has visible 1px border (4 quads) | **COVERED** | hud-j5g5.2 (#287): Four thin quads (top, right, bottom, left) rendered at 1px using `color.border.default` token. |
| Border renders inside backdrop bounds | **COVERED** | hud-j5g5.2 (#287): Borders positioned at inner edges of backdrop quad, not outside. |

---

## Requirement 4: Notification Text Rendering

| Scenario | Status | Evidence |
|---|---|---|
| Text uses body typography tokens (16px, family, left-aligned) | **COVERED** | hud-j5g5.3 (#288): `typography.body.size` (default 16px) resolved from design tokens; left-aligned text. 15 tests covering typography tokens, text inset, and clipping. |
| Text is inset by padding from border (x+9, y+9) | **COVERED** | hud-j5g5.3 (#288): 9px inset = 1px border + 8px padding (`spacing.padding.medium`). |
| Long text clips within content area | **COVERED** | hud-j5g5.3 (#288): Text clipped at content area boundary, no wrapping in v1. |

---

## Requirement 5: Notification Stack Max Depth Eviction

| Scenario | Status | Evidence |
|---|---|---|
| Sixth notification evicts oldest | **COVERED** | hud-j5g5.1 (#279): max_depth=5 enforced; oldest publication evicted when 6th arrives. Verified by integration test in hud-j5g5.5 (#310): "Max depth eviction: publish 6 notifications; assert only 5 newest remain." |
| Evicted notification has no fade-out | **COVERED** | hud-j5g5.5 (#310): "oldest is evicted immediately (no fade-out for Stack overflow)." |

---

## Requirement 6: Notification TTL Auto-Dismiss with Fade-Out

| Scenario | Status | Evidence |
|---|---|---|
| Notification auto-dismisses after 8 seconds (default TTL) | **COVERED** | hud-j5g5.3 (#288): Default TTL=8000ms from zone auto_clear_ms. Compositor checks publication age per frame. |
| Fade-out interpolates opacity over 150ms (0.5 at 75ms) | **COVERED** | hud-j5g5.3 (#288): `PublicationAnimationState` with linear lerp over 150ms. Test verifies ~0.5 at midpoint. |
| Notification removed from active_publishes after fade completes | **COVERED** | hud-j5g5.3 (#288): `prune_faded_publications` removes when opacity reaches 0.0. |
| Custom TTL (ttl_ms=3000) respected | **COVERED** | hud-j5g5.3 (#288) + hud-j5g5.5 (#310): `ttl_ms` field in NotificationPayload; test uses ttl_ms=100 to verify custom TTL. |
| Multiple notifications fade simultaneously and independently | **COVERED** | hud-j5g5.3 (#288): Each publication has its own `PublicationAnimationState`; simultaneous fades are independent. Test verifies two overlapping fades. |

---

## Requirement 7: Multi-Agent Notification Publishing

| Scenario | Status | Evidence |
|---|---|---|
| Three agents publish simultaneously (correct stack order and urgency coloring) | **COVERED** | hud-j5g5.5 (#310): Test 1 — 3 agents (alpha/beta/gamma) publish; arrival order and slot assignment verified (alpha at index 0, gamma at index 2, newest at top). |
| Agents do not interfere with each other's notifications | **COVERED** | hud-j5g5.5 (#310): Test 5 — "Agent independence: expire alpha's 8s TTL; beta's 30s-TTL record survives." |

---

## Requirement 8: Notification Stack Exemplar Component Profile

| Scenario | Status | Evidence |
|---|---|---|
| Exemplar profile loads successfully with token overrides | **COVERED** | hud-j5g5.4 (on main): `profiles/notification-stack-exemplar/profile.toml` with `component_type = "notification"` and all four urgency token overrides (`color.notification.urgency.{low,normal,urgent,critical}`). Profile verified loadable. |
| Profile passes OpaqueBackdrop readability (backdrop_opacity=0.9 >= 0.8) | **COVERED** | hud-j5g5.4 (on main): `opacity.backdrop.opaque = "0.9"` in profile.toml; `backdrop_opacity = 0.9` in zones/notification-area.toml. |

---

## Requirement 9: Notification Exemplar MCP Integration Test

| Scenario | Status | Evidence |
|---|---|---|
| MCP publish from 3 agents with mixed urgency | **COVERED** | hud-j5g5.5 (#310): 5 integration tests in `crates/tze_hud_mcp/src/tools.rs`; Test 1 covers 3-agent mixed urgency ordering. |
| Max depth eviction via MCP burst (6 publishes → 5 remain) | **COVERED** | hud-j5g5.5 (#310): Test 2 — publishes 6, asserts 5 remain and 1st is evicted. |
| TTL expiry removes notification from active_publishes | **COVERED** | hud-j5g5.5 (#310): Test 3 — uses TestClock, advances past TTL, drain_expired_zone_publications removes the record. |

---

## Requirement 10: Notification Exemplar User-Test Scenario

| Scenario | Status | Evidence |
|---|---|---|
| Burst publish with visual inspection pauses (all 4 phases) | **COVERED** | hud-j5g5.6 (#317): `.claude/skills/user-test/scripts/notification_exemplar.py` — 4-phase scenario (initial burst, stack growth, TTL expiry, max_depth eviction). |
| Script publishes via MCP publish_to_zone with NotificationPayload | **COVERED** | hud-j5g5.6 (#317): Uses MCP publish_to_zone with `{"type":"notification","text":"...","icon":"...","urgency":N}` payload structure. |

---

## Summary

**Coverage: FULL — all 30 scenarios across 10 requirements are covered by the 6 sibling beads.**

| Requirement | Scenarios | Status |
|---|---|---|
| §Notification Stack Vertical Layout | 3 | FULL |
| §Urgency-Tinted Notification Backdrops | 6 | FULL |
| §Notification Border Rendering | 2 | FULL |
| §Notification Text Rendering | 3 | FULL |
| §Notification Stack Max Depth Eviction | 2 | FULL |
| §Notification TTL Auto-Dismiss with Fade-Out | 5 | FULL |
| §Multi-Agent Notification Publishing | 2 | FULL |
| §Notification Stack Exemplar Component Profile | 2 | FULL |
| §Notification Exemplar MCP Integration Test | 3 | FULL |
| §Notification Exemplar User-Test Scenario | 2 | FULL |
| **Total** | **30** | **30/30 COVERED** |

No gaps found. No child beads needed. No gen-2 reconciliation required.

## OpenSpec Sync

Delta spec synced to `openspec/specs/exemplar-notification/spec.md` (created from `openspec/changes/exemplar-notification/specs/exemplar-notification/spec.md`). All ADDED requirements applied to the main spec.
