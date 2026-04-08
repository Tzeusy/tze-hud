# Dynamic SVG Runtime Upload Implementation Report

**Issue:** `hud-lviq.7`  
**Epic:** `hud-lviq`  
**Date:** 2026-04-08  
**Scope:** Human-review summary of delivered implementation paths, test evidence, and residual risks for runtime dynamic SVG support.

## Executive Summary

Dynamic SVG support is implemented as a two-stage model across protocol and MCP surfaces:
1. stage-1 register/upload (`WidgetAssetRegister` / `register_widget_asset`)
2. stage-2 parameter publish (`WidgetPublish` / `publish_to_widget`)

Core behaviors for capability gating, hash-first dedup preflight, checksum/hash validation, and budget errors are covered by unit/integration tests. A durable runtime widget store exists with restart re-index tests and startup wiring. However, the current session register path still writes to an in-memory `WidgetAssetStore`, so restart durability for runtime-registered assets is not yet end-to-end proven on the live session path.

## Delivered Code Paths

### 1) Protocol surface and wire contracts

- Client/server field allocation and message schema:
  - [`crates/tze_hud_protocol/proto/session.proto`](../crates/tze_hud_protocol/proto/session.proto)
  - `ClientMessage.widget_asset_register = 34`, `ServerMessage.widget_asset_register_result = 48`
  - `WidgetPublish` remains field 35 (parameter publish stage)
- Session handler implementation:
  - [`crates/tze_hud_protocol/src/session_server.rs`](../crates/tze_hud_protocol/src/session_server.rs)
  - `handle_widget_asset_register` validates capability, type/filename, hash/payload/CRC, SVG validity, dedup, and budget errors
  - `handle_widget_publish` applies parameter-only widget publishes

### 2) Durable store + config plumbing

- Durable runtime store implementation:
  - [`crates/tze_hud_resource/src/runtime_widget_store.rs`](../crates/tze_hud_resource/src/runtime_widget_store.rs)
  - content-addressed dedup, atomic file writes, sidecar metadata, startup re-index, per-total and per-agent budgets
- Runtime store config resolution:
  - [`crates/tze_hud_config/src/runtime_widget_assets.rs`](../crates/tze_hud_config/src/runtime_widget_assets.rs)
  - default path/budgets, path resolution, writability probe, budget relation validation
- Runtime startup opens the store:
  - [`crates/tze_hud_runtime/src/headless.rs`](../crates/tze_hud_runtime/src/headless.rs)
  - [`crates/tze_hud_runtime/src/windowed.rs`](../crates/tze_hud_runtime/src/windowed.rs)

### 3) MCP register/publish surface

- Register tool implementation:
  - [`crates/tze_hud_mcp/src/tools.rs`](../crates/tze_hud_mcp/src/tools.rs)
  - `handle_register_widget_asset` mirrors protocol semantics (capability, preflight, checksum/hash, SVG validation, stable error codes)
- Publish tool implementation:
  - [`crates/tze_hud_mcp/src/tools.rs`](../crates/tze_hud_mcp/src/tools.rs)
  - `handle_publish_to_widget` publishes typed parameters only (no SVG payload)

### 4) Widget registry/runtime SVG scaffolding

- Runtime widget registration helper:
  - [`crates/tze_hud_runtime/src/widget_runtime_registration.rs`](../crates/tze_hud_runtime/src/widget_runtime_registration.rs)
  - validates runtime SVG compatibility, records runtime handle, enqueues SVG for compositor registration
- Scene registry/queue primitives:
  - [`crates/tze_hud_scene/src/types.rs`](../crates/tze_hud_scene/src/types.rs)
  - [`crates/tze_hud_scene/src/graph.rs`](../crates/tze_hud_scene/src/graph.rs)

## Required Behaviors Review

### Two-stage flow

- Stage 1 (register/upload): implemented in protocol + MCP paths via `WidgetAssetRegister` / `register_widget_asset`.
- Stage 2 (publish): implemented as parameter-only `WidgetPublish` / `publish_to_widget`.
- Wire schema reflects separation: `WidgetPublish` carries widget params, while `WidgetAssetRegister` carries hash/checksum/payload metadata.

### Restart durability

- Durable store behavior is implemented and tested (`RuntimeWidgetStore` re-index after restart, corrupt/partial artifact rejection).
- Runtime startup resolves and opens durable store config in both headless and windowed runtime initialization.
- **Observed gap:** session register handler currently persists into in-memory `SharedState.widget_asset_store` (`crates/tze_hud_protocol/src/session.rs`) rather than `RuntimeWidgetStore`; this leaves end-to-end restart durability for live runtime registration path as a residual risk.

### Dedup + checksum behavior

- Metadata preflight dedup hit path exists (known hash => accepted + `was_deduplicated=true`, no payload required).
- Unknown hash path requires payload and validates:
  - declared size vs payload length
  - optional `transport_crc32c`
  - BLAKE3 payload hash against declared identity
  - SVG structure validity
- Stable error taxonomy is wired and tested (`WIDGET_ASSET_*` codes).

## Test Evidence

- Protocol wire/message roundtrip tests:
  - [`crates/tze_hud_protocol/tests/widget_asset_register_integration.rs`](../crates/tze_hud_protocol/tests/widget_asset_register_integration.rs)
- Session server behavior tests:
  - [`crates/tze_hud_protocol/src/session_server.rs`](../crates/tze_hud_protocol/src/session_server.rs)
  - coverage: capability missing, preflight dedup hit, unknown hash requires payload, checksum/type/svg validation, budget exceeded
- Durable store tests:
  - [`crates/tze_hud_resource/src/runtime_widget_store.rs`](../crates/tze_hud_resource/src/runtime_widget_store.rs)
  - coverage: restart rehydrate, corrupt blob rejection, temp-file ignore, budget enforcement
- MCP tests:
  - [`crates/tze_hud_mcp/src/tools.rs`](../crates/tze_hud_mcp/src/tools.rs)
  - [`crates/tze_hud_mcp/src/server.rs`](../crates/tze_hud_mcp/src/server.rs)
  - coverage: preflight dedup, checksum mismatch, invalid SVG, capability gating
- Runtime registration scaffolding tests:
  - [`crates/tze_hud_runtime/src/widget_runtime_registration.rs`](../crates/tze_hud_runtime/src/widget_runtime_registration.rs)
  - [`crates/tze_hud_scene/src/graph.rs`](../crates/tze_hud_scene/src/graph.rs)

## Residual Risks and Recommended Follow-up

1. **Durability integration gap on live register path (High):** `handle_widget_asset_register` writes only to in-memory `widget_asset_store`; it is not currently wired to `RuntimeWidgetStore`.
2. **Runtime SVG handle plumbing appears incomplete (High):** `register_runtime_widget_svg_asset` exists, but no non-test call site currently invokes it; runtime SVG handle map/queue usage is therefore not end-to-end proven.
3. **Missing restart E2E for protocol path (Medium):** no integration test currently demonstrates `WidgetAssetRegister` -> restart -> dedup hit + successful post-restart publish in one flow.
4. **Dual implementation drift risk (Medium):** protocol and MCP register logic are duplicated; behavior may diverge without shared validation helpers or cross-surface parity tests.

