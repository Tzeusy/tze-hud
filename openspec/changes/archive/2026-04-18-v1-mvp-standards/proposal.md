## Why

tze_hud has mature doctrine (`about/heart-and-soul/`) and detailed design contracts (`about/legends-and-lore/rfcs/0001–0011`) but no formal specification artifacts that can be verified against implementation, diffed against changes, or used to generate implementation task lists. The RFCs define wire-level contracts; what's missing is a normative specification layer that bridges doctrine and code — one that an LLM implementer can load, an OpenSpec change can diff against, and CI can eventually validate.

This change creates the complete v1 MVP specification set: one spec per subsystem, each grounded in its corresponding RFC and the soul doctrine, scoped precisely to the v1 boundary defined in `about/heart-and-soul/v1.md`.

## What Changes

- Creates 12 new capability specs covering every v1 subsystem
- Each spec traces requirements to its source RFC and doctrine files
- V1 scope boundaries are normative: what ships, what defers, what's schema-reserved
- Quantitative budgets (latency, capacity, resource limits) are extracted into testable requirements
- Cross-subsystem integration contracts are explicit in each spec
- Provides the specification foundation for all subsequent implementation tasks

## Capabilities

### New Capabilities

- `scene-graph`: Scene data model — tabs, tiles, nodes, identity scheme (SceneId/ResourceId), mutation pipeline, atomic batches, zone registry, namespace isolation, hit-testing contract. Source: RFC 0001, presence.md.
- `runtime-kernel`: Execution model — process/thread architecture, 8-stage frame pipeline with per-stage budgets, admission control, degradation ladder (5 levels), window modes (fullscreen/overlay/headless), platform GPU backends. Source: RFC 0002, architecture.md, v1.md.
- `timing-model`: Temporal semantics — clock domains (`_wall_us`/`_mono_us`), sync groups, frame deadline contract, presentation scheduling, relative timing primitives. Source: RFC 0003, architecture.md.
- `input-model`: Interaction contract — focus tree, pointer capture protocol, hit-test pipeline, gesture recognition (v1 tier), local feedback guarantee (< 4ms p99), HitRegionNode primitive, event dispatch and routing, accessibility structure. Source: RFC 0004, presence.md.
- `session-protocol`: Wire protocol — bidirectional gRPC session stream, ClientMessage/ServerMessage envelope (47+ fields), session lifecycle state machine, reconnection with full snapshot, lease management RPCs, subscription system, MCP bridge (guest + resident tools), traffic class routing. Source: RFC 0005, architecture.md.
- `configuration`: Runtime config — TOML schema, display profiles (full-display/headless, mobile reserved), zone registry configuration, agent registration and budget defaults, capability vocabulary, auto-detection rules, validation with structured errors. Source: RFC 0006.
- `system-shell`: Chrome layer — safe mode protocol, freeze/mute/dismiss-all override controls, privacy-safe capture, disconnection badges, backpressure signals, diagnostic overlay (v1 minimal CLI surface), audit events. Source: RFC 0007.
- `lease-governance`: Lease state machine — REQUESTED→ACTIVE→EXPIRED/ORPHANED/REVOKED/SUSPENDED states, priority assignment (0–255), auto-renewal policies, grace periods, orphan handling, resource budget schema and enforcement ladder, zone interaction semantics. Source: RFC 0008, presence.md.
- `policy-arbitration`: Conflict resolution — 7-level arbitration stack (human override → safety → privacy → security → attention → resource → content), per-mutation/per-event/per-frame evaluation pipeline, GPU failure two-phase response, policy interaction matrix. Source: RFC 0009, security.md, privacy.md, attention.md.
- `scene-events`: Event system — event taxonomy (input/scene/system), interruption classification (CRITICAL/HIGH/NORMAL/LOW/SILENT), quiet hours, agent event emission, subscription model with category filtering, event bus architecture, `tab_switch_on_event` contract. Source: RFC 0010, privacy.md.
- `resource-store`: Asset lifecycle — content-addressed storage (BLAKE3), upload protocol, v1 resource types (IMAGE_RGBA8/PNG/JPEG, FONT_TTF/OTF), reference counting, garbage collection with grace period, cross-agent sharing, per-agent budget enforcement, ephemeral storage (no persistence in v1). Source: RFC 0011.
- `validation-framework`: Testing and observability — five validation layers (scene graph assertions → headless pixel readback → visual regression via SSIM → telemetry and performance validation → developer visibility artifacts), hardware-normalized calibration harness, test scene registry (25 named scenes), structured per-frame telemetry schema, LLM development loop contract, soak/leak testing requirements. Source: validation.md, v1.md success criteria.

### Modified Capabilities

(None — greenfield; no existing specs to modify.)

## Impact

- **Code**: All Rust crates under `crates/` will implement against these specs. `crates/tze_hud_protocol/proto/session.proto` already partially implements RFC 0005.
- **APIs**: gRPC service definition (`HudSession`), MCP tool surface, configuration schema, telemetry schema — all governed by these specs.
- **Dependencies**: wgpu, winit, tonic, tokio, GStreamer bindings (post-v1 only), platform windowing APIs.
- **Systems**: CI pipeline (headless rendering on llvmpipe/WARP/Metal), test scene registry, developer visibility artifact generation.
- **Doctrine alignment**: Every spec must trace requirements to heart-and-soul doctrine and respect the v1 scope boundary. The seven non-negotiable rules from the soul (LLMs never in frame loop, screen is sovereign, arrival ≠ presentation, local feedback first, presence requires governance, tests measure spirit, human always overrides) are load-bearing constraints across all specs.

## Known Non-Definitive Areas

The following areas are documented in these specs or referenced standards but are not yet fully resolved or are explicitly deferred:

- **Protocol cutover**: The legacy unary scene service is still compiled alongside the new bidirectional gRPC session protocol. Standards require removal of the legacy service in v1-final.
- **Capability vocabulary convergence**: The vertical_slice implementation still uses legacy names (e.g., `create_tile` in some contexts) that differ from the normalized vocabulary defined in the configuration spec. These will be converged during the capability standardization pass.
- **Calibration harness**: The validation spec references a hardware-normalized calibration harness for SSIM-based visual regression. The harness implementation itself is not yet landed; validation scenarios remain pending its completion.
- **Mobile profile**: The schema reserves fields and capability scopes for a Mobile Presence Node profile (smaller screen, touch-first interaction model). This profile is deferred post-v1; it fails at startup if attempted in v1 and will be fully specified and implemented in a subsequent release.
