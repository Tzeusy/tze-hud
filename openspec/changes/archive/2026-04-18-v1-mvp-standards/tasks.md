<!-- TERMINAL DISPOSITION PASS — 2026-04-18 (hud-cek0, Model A archive)
     DONE: 109 | TRACKED: 0 | DEFERRED: 9 | NEW: 0
     Disposition method: matched against closed/open beads + verified Rust crate
     implementations in crates/. All 12 specs synced to openspec/specs/ via
     hud-etry/hud-h3eb/hud-65i8/hud-hc86/hud-2z4b/hud-7f7m/hud-ksnl/hud-0agx/
     hud-cs50/hud-66gj/hud-3daf/hud-xttl. This change is frozen pending archive
     by hud-d9il. Do NOT amend further; route new deltas to openspec/specs/.
-->

## 1. Scene Graph Foundation
<!-- Spec synced: hud-66gj. Implementation: crates/tze_hud_scene/ -->

- [x] 1.1 Implement SceneId (UUIDv7) and ResourceId (BLAKE3) identity types with validation
  <!-- DONE: tze_hud_scene/src/types.rs, cross-spec vocab fixed by hud-of3 -->
- [x] 1.2 Implement scene graph data structure: Tab, Tile, Node hierarchy with namespace isolation
  <!-- DONE: tze_hud_scene/src/graph.rs -->
- [x] 1.3 Implement four v1 node types: SolidColorNode, TextMarkdownNode, StaticImageNode, HitRegionNode
  <!-- DONE: tze_hud_scene/src/types.rs -->
- [x] 1.4 Implement atomic batch mutation pipeline (stage → validate → commit or reject all)
  <!-- DONE: tze_hud_scene/src/mutation.rs -->
- [x] 1.5 Implement tile CRUD operations with z-order management and field invariants
  <!-- DONE: tze_hud_scene/src/graph.rs, tze_hud_scene/src/invariants.rs -->
- [x] 1.6 Implement tab CRUD operations with active tab switching
  <!-- DONE: tze_hud_scene/src/graph.rs -->
- [x] 1.7 Implement static zone registry loaded from configuration (subtitle, notification, status_bar, pip, ambient_background, alert_banner)
  <!-- DONE: tze_hud_scene/src/ zone support, compositor rendering confirmed by hud-s5dr.1 -->
- [x] 1.8 Implement zone publishing mutations with contention policies (latest-wins, stack, merge-by-key, replace)
  <!-- DONE: tze_hud_scene/src/, hud-s5dr.1 fixed Stack/MergeByKey compositor rendering -->
- [x] 1.9 Implement hit-testing pipeline with layer ordering (chrome → content z-order → background)
  <!-- DONE: tze_hud_scene/src/, tze_hud_input/src/hit_test.rs, tests/hit_test.rs -->
- [x] 1.10 Implement scene snapshot serialization and full-snapshot delivery for reconnection
  <!-- DONE: tze_hud_scene/src/replay.rs, tests/snapshot.rs -->
- [x] 1.11 Write Layer 0 tests: scene graph assertions for all CRUD, mutations, zones, hit-testing
  <!-- DONE: crates/tze_hud_scene/tests/ (batch_atomicity.rs, hit_test.rs, zone_interaction.rs, zone_ontology.rs, proptest_invariants.rs, fuzz_scene_graph.rs) -->

## 2. Timing Model
<!-- Spec synced: hud-etry. Implementation: crates/tze_hud_scene/src/timing/ -->

- [x] 2.1 Implement clock domain types with _wall_us and _mono_us naming convention and type safety
  <!-- DONE: tze_hud_scene/src/timing/domains.rs, suffix violations fixed by hud-wdz4 -->
- [x] 2.2 Implement TimingHints struct (present_at_wall_us, expires_at_wall_us, sequence, priority, coalesce_key, sync_group)
  <!-- DONE: tze_hud_scene/src/timing/hints.rs -->
- [x] 2.3 Implement frame deadline and presentation scheduling (strict no-earlier-than)
  <!-- DONE: tze_hud_scene/src/timing/scheduling.rs -->
- [x] 2.4 Implement sync group membership, lifecycle, and AllOrDefer/AvailableMembers commit policies
  <!-- DONE: tze_hud_scene/src/timing/sync_group.rs, sync_commit.rs; tests/sync_group_coordination.rs -->
- [x] 2.5 Implement injectable Clock trait for deterministic testing and headless virtual clock
  <!-- DONE: tze_hud_scene/src/timing/domains.rs + clock.rs -->
- [x] 2.6 Implement expiration policy enforcement (non-negotiable under load)
  <!-- DONE: tze_hud_scene/src/timing/expiration.rs -->
- [x] 2.7 Implement relative scheduling primitives (after_us, frames_from_now, next_frame)
  <!-- DONE: tze_hud_scene/src/timing/relative.rs -->
- [x] 2.8 Write Layer 0 tests: clock domain separation, sync group invariants, timing validation
  <!-- DONE: crates/tze_hud_scene/tests/sync_group_coordination.rs + inline unit tests -->

## 3. Runtime Kernel
<!-- Spec synced: hud-2z4b. Implementation: crates/tze_hud_runtime/; hud-7yaf covered config+startup convergence -->

- [x] 3.1 Implement thread model: main thread (input + local feedback), compositor thread (GPU ownership), telemetry thread, network threads
  <!-- DONE: tze_hud_runtime/src/component_startup.rs, pipeline.rs -->
- [x] 3.2 Implement 8-stage frame pipeline with per-stage budget tracking
  <!-- DONE: tze_hud_runtime/src/pipeline.rs -->
- [x] 3.3 Implement bounded event channels (Transactional: 256, StateStream: 512, Ephemeral: ring buffer 256)
  <!-- DONE: tze_hud_runtime/src/channels.rs -->
- [x] 3.4 Implement fullscreen window mode (guaranteed all platforms)
  <!-- DONE: app/tze_hud_app/ windowed path, convergence confirmed by hud-7yaf -->
- [x] 3.5 Implement overlay/HUD window mode with platform-specific click-through (Windows WM_NCHITTEST, macOS hitTest, X11 XShape, wlroots wlr-layer-shell)
  <!-- DONE: compositor crate windowing; Windows per-pixel transparent overlay confirmed working (see memory: reference_windows_transparent_overlay.md) -->
- [x] 3.6 Implement headless mode with offscreen texture surface and HEADLESS_FORCE_SOFTWARE env var
  <!-- DONE: tze_hud_runtime/src/headless.rs -->
- [x] 3.7 Implement 5-level degradation ladder with frame_time_p95 > 14ms trigger and hysteresis recovery
  <!-- DONE: tze_hud_runtime/src/degradation.rs -->
- [x] 3.8 Implement tile shedding: sort by (lease_priority ASC, z_order DESC)
  <!-- DONE: tze_hud_runtime/src/degradation.rs + tze_hud_scene/src/lease/ -->
- [x] 3.9 Implement admission control and per-agent session envelope
  <!-- DONE: tze_hud_runtime/src/admission.rs, tze_hud_scene/src/lease/enforcement.rs -->
- [x] 3.10 Implement compositor surface trait abstraction
  <!-- DONE: tze_hud_compositor crate -->
- [x] 3.11 Write Layer 1 tests: headless rendering, pixel readback, z-order compositing, alpha blending
  <!-- DONE: crates/tze_hud_compositor/tests/ (alert_banner_rendering.rs, subtitle_rendering.rs, notification_rendering.rs, rounded_rect_rendering.rs, etc.), crates/tze_hud_validation/tests/layer2_headless.rs -->

## 4. Input Model
<!-- Spec synced: hud-3daf. Implementation: crates/tze_hud_input/ -->

- [x] 4.1 Implement focus tree structure with acquisition, cycling, and focus events
  <!-- DONE: tze_hud_input/src/focus.rs, focus_tree.rs -->
- [x] 4.2 Implement pointer capture protocol (acquire, release, theft, auto-capture on HitRegionNode)
  <!-- DONE: tze_hud_input/src/capture.rs -->
- [x] 4.3 Implement HitRegionNode primitive with local pressed/hovered state and agent callback
  <!-- DONE: tze_hud_input/src/pointer.rs, scene/types.rs (HitRegionNode) -->
- [x] 4.4 Implement pointer events (down, up, move, enter, leave, click, cancel) and keyboard events (key_down, key_up, character)
  <!-- DONE: tze_hud_input/src/events.rs, keyboard.rs, pointer.rs -->
- [x] 4.5 Implement local feedback guarantee: runtime-owned visual state, SceneLocalPatch rendering, rollback on agent rejection
  <!-- DONE: tze_hud_input/src/local_feedback.rs, scene_local_patch.rs -->
- [x] 4.6 Implement event dispatch pipeline: capture → hit-test → route → deliver with bubbling (node → tile → tab → runtime)
  <!-- DONE: tze_hud_input/src/dispatch.rs -->
- [x] 4.7 Implement event serialization, batching, coalescing, and backpressure
  <!-- DONE: tze_hud_input/src/batching.rs, coalescing.rs, envelope.rs -->
- [x] 4.8 Implement abstract command input model: CommandInputEvent with 7 actions (NAVIGATE_NEXT, NAVIGATE_PREV, ACTIVATE, CANCEL, CONTEXT, SCROLL_UP, SCROLL_DOWN), sources (KEYBOARD, DPAD, VOICE, REMOTE_CLICKER, ROTARY_DIAL, PROGRAMMATIC), transactional delivery, and routing to focused session (RFC 0004 §10)
  <!-- DONE: tze_hud_input/src/command.rs -->
- [x] 4.9 Write Layer 0 tests: focus tree invariants, hit-test correctness, event routing
  <!-- DONE: crates/tze_hud_scene/tests/hit_test.rs, tze_hud_input/src/hit_test.rs -->

## 5. Session Protocol
<!-- Spec synced: hud-65i8. Contradictions fixed: hud-mptb, hud-d9ur, hud-x6g6, hud-f2tm. Implementation: crates/tze_hud_protocol/ -->

- [x] 5.1 Implement session lifecycle state machine (Connecting → Handshaking → Active ⇄ Disconnecting → Closed → Resuming)
  <!-- DONE: tze_hud_runtime/src/session.rs, tze_hud_scene/tests/session_lifecycle.rs -->
- [x] 5.2 Implement SessionInit/SessionEstablished handshake with capability negotiation (enforce real capability gating — do NOT grant all requested)
  <!-- DONE: tze_hud_protocol/, tze_hud_config/src/capability.rs -->
- [x] 5.3 Implement ClientMessage/ServerMessage multiplexed envelope with sequence numbers
  <!-- DONE: tze_hud_protocol/ (proto definitions), hud-mptb fixed message inventory -->
- [x] 5.4 Implement traffic class routing: Transactional (reliable, ordered, acked), State-stream (reliable, coalesced), Ephemeral realtime (low-latency, droppable)
  <!-- DONE: tze_hud_runtime/src/channels.rs, pipeline.rs -->
- [x] 5.5 Implement heartbeat protocol (5000ms interval, 3 missed threshold, 15s grace period)
  <!-- DONE: tze_hud_protocol/, hud-mptb fixed Heartbeat contradiction -->
- [x] 5.6 Implement lease management RPCs (LeaseRequest ACQUIRE/RENEW/RELEASE, LeaseResponse, LeaseStateChange)
  <!-- DONE: tze_hud_scene/src/lease/state_machine.rs, ttl.rs, types.rs -->
- [x] 5.7 Implement subscription management with category filtering (9 categories)
  <!-- DONE: tze_hud_runtime/src/event_bus/ -->
- [x] 5.8 Implement reconnection with full SceneSnapshot delivery and lease reclaim within grace period
  <!-- DONE: tze_hud_runtime/src/session.rs, tze_hud_scene/tests/session_lifecycle.rs, tze_hud_scene/tests/lease_lifecycle_presence_card.rs -->
- [x] 5.9 Implement MCP bridge: guest tools (publish_to_zone, list_zones, list_scene) requiring no lease; resident tools (create_tab, create_tile, set_content, dismiss) gated by resident_mcp capability
  <!-- DONE: tze_hud_mcp crate, hud-8ss fixed redaction/config contradiction, hud-bd60 wired text renderer -->
- [x] 5.10 Implement SessionSuspended/SessionResumed for safe mode signaling
  <!-- DONE: tze_hud_runtime/src/shell/safe_mode.rs, session.rs -->
- [x] 5.11 Remove legacy unary scene service — streaming session protocol is the single authoritative resident path
  <!-- DONE: hud-7yaf convergence epic removed stale public commands -->
- [x] 5.12 Write protocol conformance tests: schema validation, error structure, version negotiation
  <!-- DONE: crates/tze_hud_protocol/tests/ -->

## 6. Configuration
<!-- Spec synced: hud-cs50. Implementation: crates/tze_hud_config/; hud-7yaf convergence -->

- [x] 6.1 Implement TOML configuration schema with file resolution order
  <!-- DONE: tze_hud_config/src/loader.rs, raw.rs -->
- [x] 6.2 Implement display profiles: full-display and headless with budget parameters
  <!-- DONE: tze_hud_config/src/profile.rs -->
- [x] 6.3 Implement mobile profile rejection: fail at startup with CONFIG_MOBILE_PROFILE_NOT_EXERCISED
  <!-- DONE: tze_hud_config/src/profile.rs (fail-closed semantics), hud-7yaf hardened release startup -->
- [x] 6.4 Implement auto-detection rules (headless branch, full-display branch, explicit-required fallback)
  <!-- DONE: tze_hud_config/src/profile.rs -->
- [x] 6.5 Implement zone registry configuration with v1 minimum zones
  <!-- DONE: tze_hud_config/src/, zone registry in config -->
- [x] 6.6 Implement agent registration with per-agent budget overrides and canonical capability vocabulary
  <!-- DONE: tze_hud_config/src/capability.rs, agents.rs; hud-of3 fixed cross-spec vocabulary -->
- [x] 6.7 Implement configuration validation at load time with structured error codes
  <!-- DONE: tze_hud_config/src/loader.rs, tests.rs -->
- [x] 6.8 Implement profile budget escalation prevention
  <!-- DONE: tze_hud_config/src/profile.rs (fail-closed), hud-7yaf -->

## 7. Lease Governance
<!-- Spec synced: hud-0agx. Implementation: crates/tze_hud_scene/src/lease/ -->

- [x] 7.1 Implement lease state machine with all transitions (REQUESTED → ACTIVE → EXPIRED/ORPHANED/RECLAIMED/REVOKED/SUSPENDED)
  <!-- DONE: tze_hud_scene/src/lease/state_machine.rs; hud-iq2x confirmed policy wiring -->
- [x] 7.2 Implement static priority assignment at grant time (0-255, chrome=0)
  <!-- DONE: tze_hud_scene/src/lease/priority.rs -->
- [x] 7.3 Implement auto-renewal policies (MANUAL, AUTO_RENEW at 75% TTL, ONE_SHOT)
  <!-- DONE: tze_hud_scene/src/lease/ttl.rs -->
- [x] 7.4 Implement disconnect grace period (15s) with orphan → revoke transition
  <!-- DONE: tze_hud_scene/src/lease/orphan.rs, suspension.rs, cleanup.rs -->
- [x] 7.5 Implement resource budget enforcement: soft warning at 80%, hard reject at 100%, three-tier enforcement ladder
  <!-- DONE: tze_hud_scene/src/lease/budget.rs, enforcement.rs -->
- [x] 7.6 Implement capability scope enforcement (create_tiles, modify_own_tiles, manage_tabs, publish_zone:<type>, read_scene_topology, resident_mcp)
  <!-- DONE: tze_hud_scene/src/lease/capability.rs, tze_hud_config/src/capability.rs -->
- [x] 7.7 Implement zone interaction: guest publishing does not acquire leases
  <!-- DONE: tze_hud_scene/src/lease/ (guest zone path), MCP bridge (5.9) -->
- [x] 7.8 Write Layer 0 tests: lease state machine transitions, budget enforcement, capability gating
  <!-- DONE: crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs + inline tests -->

## 8. System Shell
<!-- Spec synced: hud-xttl. Implementation: crates/tze_hud_runtime/src/shell/ -->

- [x] 8.1 Implement chrome layer (always on top, independent of agent state)
  <!-- DONE: tze_hud_runtime/src/shell/chrome.rs -->
- [x] 8.2 Implement safe mode protocol: entry suspends (not revokes) agent leases, overlay indicator
  <!-- DONE: tze_hud_runtime/src/shell/safe_mode.rs; hud-8ss fixed redaction contradiction -->
- [x] 8.3 Implement freeze, mute, dismiss-all override controls
  <!-- DONE: tze_hud_runtime/src/shell/freeze.rs -->
- [x] 8.3a Implement operator override input bindings for system-shell actions: dismiss, safe-mode, freeze, mute, tab-switch, focus-cycle, quit — these are local operator hotkeys that bypass the agent event pipeline and execute instantly (Level 0 arbitration)
  <!-- DONE: tze_hud_runtime/src/shell/ (operator hotkeys wired to shell actions) -->
- [x] 8.4 Implement disconnection badges and budget warning badges
  <!-- DONE: tze_hud_runtime/src/shell/badges.rs -->
- [x] 8.5 Implement tab bar rendering with keyboard shortcuts
  <!-- DONE: tze_hud_runtime/src/shell/chrome.rs + compositor tab bar rendering -->
- [x] 8.6 Implement backpressure signals to agents
  <!-- DONE: tze_hud_runtime/src/channels.rs (bounded channels), session.rs -->
- [x] 8.7 Implement audit events for operator actions (with privacy constraints)
  <!-- DONE: tze_hud_runtime/src/shell/, tze_hud_policy/src/privacy.rs -->
- [x] 8.8 Implement v1 diagnostic surface: CLI-based scene graph dump, lease listing, resource utilization, zone state, telemetry snapshot
  <!-- DONE: tze_hud_runtime/src/ + app diagnostics CLI; hud-7yaf convergence -->

## 9. Policy Arbitration
<!-- Spec synced: hud-7f7m. Implementation: crates/tze_hud_policy/; direction work: hud-iq2x (closed) -->

- [x] 9.1 Implement 7-level arbitration stack with cross-level conflict resolution (higher level always wins)
  <!-- DONE: tze_hud_policy/src/stack.rs -->
- [x] 9.2 Implement Level 0 (Human Override): dismiss, safe mode, freeze, mute — local, instant, never interceptable
  <!-- DONE: tze_hud_policy/src/override_queue.rs + shell/chrome.rs (Level 0 bypass) -->
- [x] 9.3 Implement Level 1 (Safety): GPU failure two-phase response (safe mode first, shutdown if unrecoverable)
  <!-- DONE: tze_hud_policy/src/safety.rs -->
- [x] 9.4 Implement Level 2 (Privacy): viewer context enforcement, redaction (privacy owns all redaction)
  <!-- DONE: tze_hud_policy/src/privacy.rs; hud-8ss fixed redaction contradiction -->
- [x] 9.5 Implement Level 3 (Security): capability enforcement, lease validity checks
  <!-- DONE: tze_hud_policy/src/security.rs, tze_hud_scene/src/lease/capability.rs -->
- [x] 9.6 Implement Level 4 (Attention): interruption classification enforcement, quiet hours
  <!-- DONE: tze_hud_policy/src/attention.rs, interruption.rs; runtime/src/quiet_hours/ -->
- [x] 9.7 Implement Level 5 (Resource): budget enforcement, degradation ladder integration
  <!-- DONE: tze_hud_policy/src/resource.rs + tze_hud_runtime/src/degradation.rs -->
- [x] 9.8 Implement Level 6 (Content): zone contention resolution, z-order arbitration
  <!-- DONE: tze_hud_policy/src/content.rs, tze_hud_scene/src/ zone contention -->
- [x] 9.9 Implement per-mutation evaluation pipeline with < 100μs latency budget
  <!-- DONE: tze_hud_policy/src/mutation.rs + pipeline.rs (latency tracking) -->
- [x] 9.10 Write Layer 0 tests: arbitration stack precedence, conflict resolution scenarios
  <!-- DONE: tze_hud_policy/src/tests.rs -->

## 10. Scene Events
<!-- Spec synced: hud-hc86. Implementation: crates/tze_hud_runtime/src/event_bus/ -->

- [x] 10.1 Implement event taxonomy: input events, scene events, system events with SceneEvent envelope
  <!-- DONE: tze_hud_runtime/src/event_bus/, tze_hud_scene/src/events/ -->
- [x] 10.2 Implement interruption classification (CRITICAL/HIGH/NORMAL/LOW/SILENT)
  <!-- DONE: tze_hud_policy/src/interruption.rs; hud-of3 fixed interruption taxonomy -->
- [x] 10.3 Implement quiet hours enforcement with queue semantics
  <!-- DONE: tze_hud_runtime/src/quiet_hours/ -->
- [x] 10.4 Implement subscription model with 9 category types and prefix filtering
  <!-- DONE: tze_hud_runtime/src/event_bus/ (subscription + category filtering) -->
- [x] 10.5 Implement event bus pipeline: classify → filter → coalesce → deliver
  <!-- DONE: tze_hud_runtime/src/event_bus.rs -->
- [x] 10.6 Implement tab_switch_on_event contract
  <!-- DONE: tze_hud_runtime/src/event_bus/ (tab switch integration) -->
- [x] 10.7 Implement agent event emission with capability gating and rate limiting (10/s per agent, 1000/s aggregate)
  <!-- DONE: tze_hud_runtime/src/agent_events/ -->
- [x] 10.8 Write Layer 0 tests: event classification, subscription filtering, quiet hours, tab switch triggers
  <!-- DONE: crates/tze_hud_runtime/tests/ + inline tests in event_bus modules -->

## 11. Resource Store
<!-- Spec synced: hud-ksnl. Implementation: crates/tze_hud_resource/; hud-ooj1 (resident upload), hud-lviq (SVG/asset store) -->

- [x] 11.1 Implement content-addressed ResourceId (BLAKE3, 32 bytes) with resource immutability
  <!-- DONE: tze_hud_resource/src/types.rs, dedup.rs; hud-of3 fixed ResourceId encoding vocab -->
- [x] 11.2 Implement upload protocol: session stream upload with chunked flow and hash verification
  <!-- DONE: tze_hud_resource/src/upload.rs; hud-ooj1 (resident upload epic, closed) -->
- [x] 11.3 Implement v1 resource type validation (IMAGE_RGBA8/PNG/JPEG, FONT_TTF/OTF)
  <!-- DONE: tze_hud_resource/src/validation.rs -->
- [x] 11.4 Implement content-addressed deduplication (< 100μs lookup)
  <!-- DONE: tze_hud_resource/src/dedup.rs -->
- [x] 11.5 Implement reference counting with per-agent accounting and cross-agent sharing semantics
  <!-- DONE: tze_hud_resource/src/refcount.rs, sharing.rs -->
- [x] 11.6 Implement GC with grace period (60s), cycle timing (30s, 5ms budget), and frame render isolation
  <!-- DONE: tze_hud_resource/src/gc.rs -->
- [x] 11.7 Implement per-resource size limits and per-runtime totals
  <!-- DONE: tze_hud_resource/src/budget.rs -->
- [x] 11.8 Implement font asset management with fallback chain and LRU cache (64 MiB)
  <!-- DONE: tze_hud_resource/src/font_cache.rs, font_bytes_store.rs; hud-bd60 wired init_text_renderer -->
- [x] 11.9 Write Layer 0 tests: upload, dedup, refcounting, GC lifecycle, budget enforcement
  <!-- DONE: crates/tze_hud_resource/ (upload.rs, dedup.rs, gc.rs, refcount.rs, budget.rs all contain inline tests) -->

## 12. Validation Framework
<!-- Spec synced: hud-h3eb. Implementation: crates/tze_hud_validation/ -->

- [x] 12.1 Implement hardware-normalized calibration harness with 3 workloads (scene-graph CPU, fill/composition GPU, upload-heavy resource)
  <!-- DONE: tze_hud_scene/src/calibration.rs -->
- [x] 12.2 Implement Layer 0 test infrastructure: pure logic, < 2s, 60%+ coverage target
  <!-- DONE: per-crate unit tests exist across all 12 subsystem crates (see sections 1–11 above) -->
- [x] 12.3 Implement Layer 1 test infrastructure: headless pixel readback with tolerance assertions (±2/channel software GPU)
  <!-- DONE: crates/tze_hud_validation/tests/layer2_headless.rs, crates/tze_hud_compositor/tests/ (pixel readback tests) -->
- [x] 12.4 Implement Layer 2 test infrastructure: SSIM visual regression (0.995 layout, 0.99 composition) with golden reference management
  <!-- DONE: tze_hud_validation/src/ssim.rs, golden.rs, layer2.rs, phash.rs, diff.rs -->
- [ ] 12.5 Implement Layer 3 benchmark binary: per-frame structured telemetry, JSON emission, split latency budget validation
  <!-- DEFERRED: tze_hud_validation/src/layer4.rs and tests/layer4.rs exist but the standalone benchmark binary with JSON emission is not confirmed complete -->
- [x] 12.6 Implement Layer 4 developer visibility artifacts: index.html gallery, manifest.json, per-scene outputs, CI integration
  <!-- DONE: tze_hud_validation/src/layer4.rs + tests/layer4.rs -->
- [ ] 12.7 Create initial test scene registry (25 named scenes per validation-framework spec)
  <!-- DEFERRED: tests/scenes/ has 3 named scene JSON files; 25-scene registry not confirmed complete -->
- [x] 12.8 Implement protocol conformance test suite
  <!-- DONE: crates/tze_hud_protocol/tests/ -->
- [ ] 12.9 Implement record/replay trace infrastructure for debugging and regression
  <!-- DEFERRED: tze_hud_scene/src/trace.rs exists but full record/replay infra not confirmed complete (tests/integration/trace_regression.rs is present; full replay not verified) -->
- [ ] 12.10 Implement soak/leak test harness (5% tolerance at hour N vs hour 1)
  <!-- DEFERRED: tests/integration/soak.rs exists but soak/leak tolerance validation (5% N vs 1) not confirmed complete -->

## 13. Integration and Convergence
<!-- All items DEFERRED: integration/convergence work is ongoing through hud-d9il and future epics.
     The v1-mvp-standards change is frozen pending archive. Route integration gaps to new epics. -->

- [ ] 13.1 Converge all capability names to canonical vocabulary from configuration spec (eliminate legacy naming)
  <!-- DEFERRED: hud-of3 fixed cross-spec vocab in specs; runtime code convergence ongoing — no dedicated bead; file new epic if material gaps found -->
- [ ] 13.2 Ensure MCP surface enforces guest/resident distinction — guest tools require no lease, resident tools gated by resident_mcp capability
  <!-- DEFERRED: architecture is in place (5.9); full enforcement audit not done -->
- [ ] 13.3 Validate all protobuf definitions match session-protocol spec envelope field allocations
  <!-- DEFERRED: proto fixes landed (hud-mptb, hud-d9ur, hud-x6g6); full field-by-field audit pending -->
- [ ] 13.4 Run full cross-spec integration test: 3 agents, leased tiles, zone publishing, input events, degradation, at 60fps
  <!-- DEFERRED: tests/integration/multi_agent.rs and tests/v1_proof/v1_thesis.rs exist; full 60fps CI gate not confirmed -->
- [ ] 13.5 Validate all quantitative budgets met on reference hardware with calibration normalization
  <!-- DEFERRED: calibration harness (12.1) exists; full budget gate on reference hardware is an ops/CI task -->

---

## Deferred Items Summary

The 9 deferred items above fall into three categories:

**Category A — Validation completeness (items 12.5, 12.7, 12.9, 12.10)**: Infrastructure files exist but completeness against spec (25-scene registry, standalone benchmark binary, full record/replay, soak tolerance) is not confirmed. File targeted beads under the validation framework if these gaps block v1 release.

**Category B — Integration audit (items 13.1, 13.2, 13.3)**: The underlying implementations exist; what's missing is a systematic cross-spec audit pass. Track as a single integration audit bead if/when the codebase reaches feature-complete status.

**Category C — Integration tests at scale (items 13.4, 13.5)**: Tests exist but the full multi-agent 60fps gate with calibration normalization on reference hardware is a CI/ops concern. File under infra/CI when hardware is ready.

All deferred items are out of scope for this change (which archives the v1-mvp-standards OpenSpec change). Future work routes to `openspec/specs/<capability>/spec.md` deltas or new implementation epics, not to this change directory.
