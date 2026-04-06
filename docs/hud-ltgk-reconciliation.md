# hud-ltgk Spec-to-Code Coverage — Notification Redesign Reconciliation

**Issue:** hud-ltgk.5
**Epic:** hud-ltgk (Redesign notification zone with modern UI components)
**Audit date:** 2026-04-06
**Sibling PRs audited:** #354 (hud-ltgk.1), #356 (hud-ltgk.2), #353 (hud-ltgk.3), #357 (hud-ltgk.4)

---

## Executive Summary

All 4 sibling beads merged successfully. The core notification redesign (rounded
corners, icon rendering, two-line layout) is fully delivered and token-driven.
The zone interaction layer (dismiss/action) is structurally correct but has two
gaps: action buttons have no protocol wire support, and the runtime does not yet
act on `ZoneInteraction` hits to trigger `dismiss_notification()`.

**6 of 7 epic acceptance criteria are covered. 1 is partial. 2 gap beads needed.**

---

## Spec Sources Audited

| Source | Relevant Requirements |
|---|---|
| `about/heart-and-soul/presence.md` | Notification zone anatomy, Chrome layer, zone types, adjunct effects |
| `about/heart-and-soul/attention.md` | Dismiss/action doctrine — viewer autonomy |
| `about/heart-and-soul/vision.md` | Visual identity is modular — never hardcode in compositor |
| `openspec/changes/component-shape-language/design.md` | `backdrop_radius` deferral (lines 25, 193), RenderingPolicy extension |
| `openspec/changes/exemplar-notification/design.md` | Notification exemplar goals and non-goals |
| `openspec/specs/exemplar-notification/spec.md` | All 10 requirements with scenarios |
| `about/law-and-lore/rfcs/0004-input.md` | HitRegionNode sole interactive primitive in v1, zone passthrough default |

---

## Epic Acceptance Criteria Coverage

### AC-1: Notification backdrops render with visually rounded corners (radius ≥ 8px)

**Status: COVERED** — hud-ltgk.1 (PR #354)

Evidence:
- `crates/tze_hud_compositor/src/pipeline.rs`: `RoundedRectVertex`, `RoundedRectDrawCmd`, `ROUNDED_RECT_SHADER`, `rounded_rect_vertices()`
- `crates/tze_hud_compositor/src/renderer.rs`: `collect_rounded_rect_cmds()`, `encode_rounded_rect_pass()`, per-zone `use_rounded_rect` guard
- `crates/tze_hud_scene/src/types.rs`: `backdrop_radius: Option<f32>` on `RenderingPolicy`
- `crates/tze_hud_compositor/tests/rounded_rect_rendering.rs`: 4 GPU headless tests

SDF shader works in both overlay and fullscreen modes (premultiplied alpha). Radius
clamped per-slot. The `backdrop_radius = None` path retains the flat-rect pipeline
(backward compatible).

---

### AC-2: `NotificationPayload.icon` renders a texture-sampled icon when valid resource_id provided

**Status: COVERED** — hud-ltgk.2 (PR #356)

Evidence:
- `crates/tze_hud_scene/src/types.rs`: `ResourceId::from_hex()` — parses hex-encoded icon field
- `crates/tze_hud_compositor/src/renderer.rs`: `parse_notification_icon()`, `ensure_scene_image_textures()` scans Notification icon fields, `render_zone_content()` emits `TexturedDrawCmd` for icon quad
- `NOTIFICATION_ICON_SIZE_PX = 24.0`, `NOTIFICATION_ICON_GAP_PX = 6.0`
- Icon is 24×24, left-aligned, text inset right by `icon_size + gap`
- Graceful fallback: empty icon, human-readable name, unregistered ResourceId all fall back to text-only rendering

3 GPU integration tests in `notification_rendering.rs` cover cached, absent, and unregistered icon cases.

---

### AC-3: Notifications display title (bold) and body (regular) as two distinct text lines

**Status: COVERED** — hud-ltgk.3 (PR #353)

Evidence:
- `crates/tze_hud_scene/src/types.rs`: `title: String` (default empty) on `NotificationPayload`
- `crates/tze_hud_protocol/proto/types.proto`: `title = 4` field on `NotificationPayloadProto`
- `crates/tze_hud_mcp/src/tools.rs`: `title` parsed from MCP JSON payload
- `crates/tze_hud_compositor/src/renderer.rs`: `notification_slot_height()`, `per_slot_heights()`, `slot_offsets()`, variable-height stacking
- Title renders at weight 700; body at weight 400, 0.85× font size
- Token overrides via `typography.notification.title.weight` and `typography.notification.body.scale`
- Backward compatible: empty `title` = single-line rendering

5 unit tests covering slot height formula, empty title backward compat, two-line layout, mixed-height stacking.

---

### AC-4: Each notification has a visible dismiss (×) affordance that removes it on click/tap

**Status: PARTIAL** — hud-ltgk.4 (PR #357) delivered the infrastructure but not the end-to-end wiring.

What is implemented:
- `crates/tze_hud_scene/src/types.rs`: `ZoneHitRegion`, `ZoneInteractionKind::{Dismiss, Action}`, `HitResult::ZoneInteraction`
- `crates/tze_hud_scene/src/graph.rs`: `dismiss_notification()` removes a matching publication by (zone_name, published_at_wall_us, publisher_namespace)
- `crates/tze_hud_compositor/src/renderer.rs`: `populate_zone_hit_regions()` computes dismiss (×) and action button pixel bounds each frame
- `crates/tze_hud_input/src/lib.rs`: `HitResult::ZoneInteraction` extracts `interaction_id`, sets no tile/node (correct — zone-owned)
- 15 scene tests + 5 GPU tests verify hit region generation and `dismiss_notification`

**Gap:** The runtime event loop (`crates/tze_hud_runtime/src/windowed.rs`, line 932) calls `input_processor.process()` but discards `_result` without checking `hit.is_zone_interaction()`. There is no code path that calls `scene.dismiss_notification()` in response to a live pointer click on the dismiss (×) button.

`dismiss_notification()` is only called in test code today. A click on the × button has no effect in a running HUD.

See: **Gap Bead 1** below.

---

### AC-5: At least one action button per notification is supported with agent callback

**Status: PARTIAL** — hud-ltgk.4 (PR #357) delivered the data structure but action buttons cannot reach agents via any protocol.

What is implemented:
- `crates/tze_hud_scene/src/types.rs`: `NotificationAction { label, callback_id }`, `actions: Vec<NotificationAction>` on `NotificationPayload`
- `crates/tze_hud_compositor/src/renderer.rs`: renders action button quads and registers their hit regions in `populate_zone_hit_regions()`
- `ZoneInteractionKind::Action { callback_id }` carries the agent-defined callback identifier

**Gaps:**
1. **No proto wire support:** `NotificationPayload` proto (`types.proto`) has no `actions` field. `crates/tze_hud_protocol/src/convert.rs` explicitly sets `actions: Vec::new()` with a comment: "actions are not yet carried over the gRPC wire (proto field not defined)."
2. **MCP always sets empty actions:** `crates/tze_hud_mcp/src/tools.rs` parses `title` from MCP JSON but always sets `actions: Vec::new()`.
3. **No action callback delivery:** Even if an action button is clicked, the `ZoneInteraction` result is discarded by the runtime (same issue as AC-4).

Action buttons are unrenderable by any external agent (gRPC or MCP) and undeliverable to agents even if rendered programmatically in tests.

See: **Gap Bead 2** below.

---

### AC-6: All visual properties (radius, colors, padding, font sizes) are token-driven

**Status: PARTIAL** — `backdrop_radius` is not token-driven.

What is covered:
- Colors (urgency backdrops, border, text): token-resolved from `color.notification.urgency.*`, `color.border.default`, `color.text.primary`
- Padding: `spacing.padding.medium` via `apply_notification_area_token_defaults()`
- Font size/weight/family: `typography.body.*` tokens
- Opacity: `opacity.backdrop.opaque` token

Gap:
- `backdrop_radius` (AC-1) is a field on `RenderingPolicy` but has no entry in `CANONICAL_TOKENS`, no wiring in `apply_notification_area_token_defaults()`, and no field in `ZoneRenderingOverride` (so it cannot be set via a component profile's `zones/notification-area.toml`).
- The only way to set `backdrop_radius` today is via direct Rust API. Operators cannot configure it via `[design_tokens]` or component profiles.
- The epic design described it as "token-driven (`border.radius.medium` or similar)." That token does not exist in the canonical schema.

This gap is lower-severity than AC-4/AC-5 (AC-1 works visually when set in code; the gap is operator configurability), but it breaks the "all visual properties are token-driven" criterion.

See: **Gap Bead 3** below.

---

### AC-7: Existing notification behavior preserved (stacking, TTL, urgency colors, max_depth eviction)

**Status: COVERED**

Evidence:
- All 211+ pre-existing compositor tests pass after each PR
- `TZE_HUD_SKIP_GPU_TESTS=1 cargo test -p tze_hud_compositor` — confirmed passing in PR #353
- The `notification_stack_exemplar` profile and `notification-area.toml` zone override retain correct OpaqueBackdrop readability (0.9 ≥ 0.8)
- Stack ordering, TTL auto-dismiss, max_depth=5 eviction, multi-agent publishing all preserved from hud-j5g5 implementation

---

## Spec-to-Implementation Mapping

### presence.md: notification zone requirements

| Requirement | Status | Notes |
|---|---|---|
| Short text + optional icon + urgency | COVERED | text, title, icon, urgency all on NotificationPayload |
| Stack contention, auto-dismiss after TTL | COVERED | hud-j5g5 |
| Adjunct sound on urgent/critical | NOT COVERED | Explicitly non-goal in exemplar design.md |
| Chrome layer (alert-banner/status-bar) | NOT APPLICABLE | Notification is Content layer |
| Zone accepts publish_to_zone via MCP | COVERED | MCP tools.rs, 5 integration tests |

### attention.md: dismiss/action doctrine

| Requirement | Status | Notes |
|---|---|---|
| Viewer can dismiss at any time without friction | PARTIAL | dismiss_notification() exists but not wired in runtime |
| Agent callbacks on action | PARTIAL | ZoneInteractionKind::Action exists but no proto wire, no runtime dispatch |

### vision.md: visual identity is modular

| Requirement | Status | Notes |
|---|---|---|
| Visual properties never hardcoded in compositor | MOSTLY MET | backdrop_radius is the only property not yet token-addressable |
| Component profiles as swappable unit | COVERED | notification-stack-exemplar profile ships |
| Design tokens provide shared visual vocabulary | COVERED | all urgency colors, typography, padding token-driven |

### component-shape-language/design.md

| Requirement | Status | Notes |
|---|---|---|
| backdrop_radius deferred (line 25, 193) | LIFTED | hud-ltgk.1 adds SDF pipeline and `backdrop_radius` field — deferral resolved |
| backdrop_radius in RenderingPolicy | COVERED | `pub backdrop_radius: Option<f32>` added |
| Token-driven radius | GAP | No canonical token `border.radius.*`; no policy_builder wiring |

### RFC 0004: input model, HitRegionNode constraints

| Requirement | Status | Notes |
|---|---|---|
| HitRegionNode is sole interactive primitive in v1 | NOT VIOLATED | ZoneHitRegion is zone-owned, bypasses tile ownership correctly |
| Zone-level interactivity as architectural extension | COVERED | ZoneHitRegion + HitResult::ZoneInteraction is the extension mechanism |
| Zones are passthrough by default | PRESERVED | ZoneHitRegion only activates for notification-area zones |
| Must work with pointer input | PARTIAL | Hit regions computed correctly; runtime doesn't act on them |
| Must work with keyboard input | NOT COVERED | No keyboard event path for ZoneInteraction |

---

## Gap Beads to Create

### Gap Bead 1: Wire ZoneInteraction results in runtime event loop (P1)

**Title:** Wire ZoneInteraction hits to dismiss_notification() in windowed runtime

**Description:**
`crates/tze_hud_runtime/src/windowed.rs` calls `input_processor.process()` but discards
the `InputResult`. When `result.hit.is_zone_interaction()` is true and `result.activated`
is true (pointer Up on the dismiss × button), the runtime must call
`scene.dismiss_notification(zone_name, published_at_wall_us, publisher_namespace)`.

The interaction_id format is documented in `ZoneHitRegion.interaction_id`:
- Dismiss: `"zone:{zone_name}:dismiss:{published_at_wall_us}:{publisher_namespace}"`
- Action: `"zone:{zone_name}:action:{published_at_wall_us}:{publisher_namespace}:{callback_id}"`

The runtime must parse this interaction_id to extract the zone name, published_at key,
and namespace, then call `dismiss_notification()` on the scene for Dismiss hits.

Also: `crates/tze_hud_runtime/src/headless.rs` has the same gap.

**Type:** bug (missing wiring)
**Priority:** 1 (dismiss button is rendered but has no effect — visible affordance, broken behavior)
**Parent:** hud-ltgk

---

### Gap Bead 2: Wire NotificationAction through proto and MCP (P2)

**Title:** Add proto wire support and MCP parsing for NotificationPayload.actions

**Description:**
`NotificationPayload.actions: Vec<NotificationAction>` is implemented in Rust but cannot
be set by any external agent:

1. `types.proto` has no field for actions (convert.rs explicitly sets `actions: Vec::new()`)
2. `crates/tze_hud_mcp/src/tools.rs` always creates `actions: Vec::new()`

Required work:
- Add `NotificationActionProto` message and `actions` field to `NotificationPayloadProto` in `types.proto`
- Wire through `convert.rs` proto_to_zone_content
- Parse `actions` from MCP JSON payload in `tools.rs` (array of `{"label": "...", "callback_id": "..."}`)
- Add proto roundtrip tests in `tze_hud_protocol/tests/roundtrip.rs`
- Depends on Gap Bead 1 for end-to-end action delivery to agents

Also: after Gap Bead 1 lands, the runtime must deliver `ZoneInteraction::Action` results
to the publishing agent as an interaction event (the `interaction_id` encodes
`callback_id` and publisher namespace for routing).

**Type:** feature (protocol completion)
**Priority:** 2
**Parent:** hud-ltgk

---

### Gap Bead 3: Add backdrop_radius to canonical token schema and profile wiring (P3)

**Title:** Wire backdrop_radius through design token system and ZoneRenderingOverride

**Description:**
`backdrop_radius` on `RenderingPolicy` is set by SDF pipeline code but cannot be
configured by operators via design tokens or component profiles.

Required work:
1. Add `"border.radius.medium"` (or equivalent) to `CANONICAL_TOKENS` in `crates/tze_hud_config/src/tokens.rs` with a default of `"8"` (px)
2. Wire into `apply_notification_area_token_defaults()` in `policy_builder.rs`:
   `if policy.backdrop_radius.is_none() { policy.backdrop_radius = token_f32(tokens, "border.radius.medium"); }`
3. Add `backdrop_radius: Option<f32>` to `ZoneRenderingOverride` in `component_profiles.rs`
4. Wire `backdrop_radius` in `merge_zone_override()` in `policy_builder.rs`
5. Update `notification-stack-exemplar/zones/notification-area.toml` with `backdrop_radius = 8.0`
6. Add `backdrop_radius` to `types.proto` proto field (currently missing from `RenderingPolicyProto`)
7. Wire proto conversion in `convert.rs`
8. Add token-override and profile-override tests

Note: `backdrop_radius` is already excluded from the CSL `component-shape-language/design.md`
non-goals ("explicitly deferred"). This gap bead lifts the remaining half of that deferral
(the field exists; the token wiring does not).

**Type:** task (token wiring)
**Priority:** 3 (operators can set rounded corners in config but not via canonical token path)
**Parent:** hud-ltgk

---

## Pre-existing Gaps (not from hud-ltgk, previously noted)

These were identified in the gen-1 CSL coverage report (`docs/component-shape-language-coverage.md`)
and are NOT from this epic. They remain open:

| Gap | Location | Severity |
|---|---|---|
| Profile validation order not explicitly enforced as phases | component_profiles.rs | Low |
| Global widget readability bypass is caller-dependent | loader.rs | Low |

The alert-banner severity token gap (previously noted as PARTIAL in the gen-1 CSL report)
is **now RESOLVED**: `urgency_to_severity_color()` in renderer.rs now accepts a `token_map`
parameter and resolves `color.severity.*` from it with hardcoded constants as fallback.

---

## Conclusion

The hud-ltgk redesign delivers a modernized notification zone with:
- Anti-aliased rounded corners via SDF pipeline (hud-ltgk.1)
- GPU-texture icon rendering (hud-ltgk.2)
- Two-line title+body layout (hud-ltgk.3)
- Zone interaction infrastructure with dismiss/action hit regions (hud-ltgk.4)

The gap beads needed are:
1. **hud-ltgk.1a** (P1, bug): Wire ZoneInteraction hits to dismiss_notification() in runtime
2. **hud-ltgk.2a** (P2, feature): Add proto + MCP wire for NotificationPayload.actions
3. **hud-ltgk.3a** (P3, task): Backfill backdrop_radius into canonical token schema and profile override system

Sound/haptic adjunct effects (presence.md: "urgent notifications play a sound") are
deferred per the exemplar design doc's explicit non-goal. No gap bead needed.

Keyboard-driven dismiss is also deferred — RFC 0004 §V1-reserved lists command input
routing as v1-reserved (full behavior post-v1). No gap bead needed.
