# Reconciliation Gen-5: Post-MVP Feature Expansion

**Date:** 2026-04-04
**Issue:** hud-a1va
**Scope:** Post-gen-4 feature work (174 commits since gen-4 baseline, 2026-03-27 to 2026-04-04)

## 1. Context

Gen-4 (2026-03-27) was the v1-MVP closure snapshot covering the 58-requirement baseline.
This generation covers post-MVP feature expansion: widget system, component shape language,
exemplar components, runtime app binary, input capture, resource management, and stress testing.

## 2. Coverage Update

### Carried forward from Gen-4 (unchanged)
- **54 FULL** — all gen-4 FULL items remain FULL
- **4 RFC-ONLY** — I2 (touch), Pl1-Pl3 (platform CI) still deferred to v1.1

### New work since Gen-4

| Area | Status | Evidence |
|------|--------|----------|
| **Widget system** (5 delta specs) | FULL (functionally) | Widget ontology, parameter schema, SVG rasterization, publishing, contention all implemented. GAP-1 (ClearWidgetMutation) closed by hud-ziov (#249, 2026-03-30). GAP-2 (Widget TTL expiry) closed by hud-2c5g (#248, 2026-03-30). GAP-3 (type ID validation) resolved by hud-qdmf (#349, 2026-04-05). 1 P2 gap remains: policy arbitration doc mismatch. |
| **Component shape language** (RFC 0012, 3 delta specs) | FULL | Design tokens, component profiles, visual extensibility implemented. Exemplar profiles use the new token system. |
| **Exemplar components** (10 exemplars) | FULL (functionally) | subtitle, alert-banner, notification, status-bar, status-indicator, progress-bar, dashboard-tile, gauge-widget, ambient-background, presence-card — all have component profiles, rendering, MCP fixtures, integration tests, user-test scenarios. **P3 gap**: subtitle and alert-banner profiles not wired into production config. |
| **Runtime app binary** (3 specs) | FULL | Canonical `tze_hud_app` binary with windowed runtime, headless mode, fullscreen/overlay modes. Network services (gRPC, MCP) start with the runtime. |
| **MCP stress testing** (2 specs) | FULL | Load profiles (idle/low/medium/high/burst), host telemetry integration, CI-gated. |
| **Input capture & focus** | FULL | Focus cycling, input capture request/release, command input events, IME composition — all exercised in integration tests (PR #340). |
| **Resource ref-count tracking** | FULL | Resource registration during batch mutation, ref-count on lease expiry, decoded-byte budget GC (PR #342). |
| **Session protocol evolution** | SPEC UPDATED | Proto layout migrated: events.proto now has RFC 0004 19-variant InputEnvelope with bytes IDs; legacy types moved to events_legacy.proto (deprecated). Session-protocol openspec refreshed to match actual 4-file layout and field allocations. |

### Coverage Summary (Gen-5)

| Status | Count | Percentage | Notes |
|--------|-------|------------|-------|
| FULL | 54 + new areas | ~95% | Gen-4 baseline + post-MVP features; widget system P1 gaps and GAP-3 closed |
| PARTIAL | 0 | 0% | No open P1 gaps; widget system GAP-3 also resolved |
| RFC-ONLY | 4 | ~5% | I2, Pl1-Pl3 (unchanged from gen-4) |
| ABSENT | 0 | 0% | No gaps without spec coverage |

## 3. P1 Gaps (all resolved)

### ~~GAP-1: ClearWidgetMutation not wired (P1)~~ — CLOSED (hud-ziov)
- **Resolution**: Full ClearWidget support implemented across proto, scene graph, gRPC, and MCP
  layers in commit `42e3b50` (2026-03-30, hud-ziov, #249). Added `ClearWidgetMutation` to
  `MutationProto.oneof` (field 5), `clear_widget_for_publisher` and
  `clear_widget_publications_for_namespace` in `graph.rs`, handler in `session_server.rs`,
  `handle_clear_widget` tool in `tools.rs`, and `clear_widget` registered as Guest-class tool
  in `server.rs`. 11 new tests (6 scene-graph, 5 MCP). Gen-5 initial assessment incorrectly
  treated this as still open.

### ~~GAP-2: Widget TTL expiry not enforced (P1)~~ — CLOSED (hud-2c5g)
- **Resolution**: `drain_expired_widget_publications()` implemented in `crates/tze_hud_scene/src/graph.rs`
  and called in both `windowed.rs` and `headless.rs` frame loops alongside
  `drain_expired_zone_publications()`. 6 spec-scenario tests added. Closed by commit `9eeb28a`
  (2026-03-30, hud-bkdg, #248). Gen-5 reconciliation incorrectly assessed this as still open.

## 4. Open P2 Gaps

### ~~GAP-3: Widget Type ID format validation (P2)~~ — RESOLVED (hud-qdmf)

**Resolution (2026-04-05)**: Validation was already implemented in the loader before this
reconciliation was written. Assessment "Loader accepts any string" was incorrect.

- `is_valid_widget_type_id()` in `crates/tze_hud_widget/src/loader.rs` enforces `[a-z][a-z0-9-]*`
- Called in `load_bundle_dir_inner()` immediately after the `name` field is extracted
- Returns `BundleError::InvalidName` with wire code `WIDGET_BUNDLE_INVALID_NAME` and a descriptive message for non-conforming names
- Integration tests in `crates/tze_hud_widget/tests/bundle_loader.rs` cover: uppercase, digit-start, hyphen-start, underscore, space, special characters (e.g. `"My Gauge!"`), wire-code, and valid name acceptance
- Unit tests in `crates/tze_hud_widget/src/loader.rs` (12 cases) verify the regex logic in isolation
- **Bead**: hud-qdmf (closed)

### GAP-4: Governance authority doc/code mismatch (P2)
- **Spec**: tze_hud_runtime lib.rs claims policy arbitration via tze_hud_policy
- **Code**: No Cargo.toml dependency on tze_hud_policy; no code imports PolicyContext
- **Bead**: hud-qqha

## 5. Spec Refreshes Applied

1. **Session-protocol openspec** (this gen): Proto File Layout updated from 3 files to 4 files. events.proto description rewritten for RFC 0004 InputEnvelope format. events_legacy.proto documented as deprecated bridge. ServerMessage field allocation updated (fields 36-47 now in use, not reserved). Widget session messages added (WidgetPublish, WidgetPublishResult). Scene event messages added (EmitSceneEvent, EmitSceneEventResult). RuntimeTelemetryFrame documented. RuntimeService.ReloadConfig RPC documented. ErrorCode enum expanded from 16 to 24 codes.

2. **Law-and-lore README**: RFC count corrected from "11 (0001–0011)" to "12 (0001–0012)" to include RFC 0012 Component Shape Language.

## 6. Deferred (unchanged from Gen-4)

- **I2**: Touch input — no test hardware
- **Pl1-Pl3**: Platform CI — runners not provisioned
- **Config contract alignment**: App startup behavior vs spec "MUST refuse" rule (hud-gxny, P1 decision needed)

## 7. Verification

```
cargo check --workspace  # passes with zero errors
cargo test --workspace   # all tests pass
```

Gen-5 is a **progress snapshot**, not a closure point. All widget system P1 gaps (GAP-1: ClearWidgetMutation, GAP-2: Widget TTL expiry) and the P2 GAP-3 (widget type ID validation) were already resolved prior to or during this generation and are now correctly marked as closed. The remaining open item is P2 GAP-4 (governance authority doc/code mismatch) and the long-standing P1 config contract decision (hud-gxny).
