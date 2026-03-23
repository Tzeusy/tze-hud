## 1. Scene Graph Foundation

- [ ] 1.1 Implement SceneId (UUIDv7) and ResourceId (BLAKE3) identity types with validation
- [ ] 1.2 Implement scene graph data structure: Tab, Tile, Node hierarchy with namespace isolation
- [ ] 1.3 Implement four v1 node types: SolidColorNode, TextMarkdownNode, StaticImageNode, HitRegionNode
- [ ] 1.4 Implement atomic batch mutation pipeline (stage → validate → commit or reject all)
- [ ] 1.5 Implement tile CRUD operations with z-order management and field invariants
- [ ] 1.6 Implement tab CRUD operations with active tab switching
- [ ] 1.7 Implement static zone registry loaded from configuration (subtitle, notification, status_bar, ambient_background)
- [ ] 1.8 Implement zone publishing mutations with contention policies (latest-wins, stack, merge-by-key, replace)
- [ ] 1.9 Implement hit-testing pipeline with layer ordering (chrome → content z-order → background)
- [ ] 1.10 Implement scene snapshot serialization and full-snapshot delivery for reconnection
- [ ] 1.11 Write Layer 0 tests: scene graph assertions for all CRUD, mutations, zones, hit-testing

## 2. Timing Model

- [ ] 2.1 Implement clock domain types with _wall_us and _mono_us naming convention and type safety
- [ ] 2.2 Implement TimingHints struct (present_at_wall_us, expires_at_wall_us, sequence, priority, coalesce_key, sync_group)
- [ ] 2.3 Implement frame deadline and presentation scheduling (strict no-earlier-than)
- [ ] 2.4 Implement sync group membership, lifecycle, and AllOrDefer/AvailableMembers commit policies
- [ ] 2.5 Implement injectable Clock trait for deterministic testing and headless virtual clock
- [ ] 2.6 Implement expiration policy enforcement (non-negotiable under load)
- [ ] 2.7 Implement relative scheduling primitives (after_us, frames_from_now, next_frame)
- [ ] 2.8 Write Layer 0 tests: clock domain separation, sync group invariants, timing validation

## 3. Runtime Kernel

- [ ] 3.1 Implement thread model: main thread (input + local feedback), compositor thread (GPU ownership), telemetry thread, network threads
- [ ] 3.2 Implement 8-stage frame pipeline with per-stage budget tracking
- [ ] 3.3 Implement bounded event channels (Transactional: 256, StateStream: 512, Ephemeral: ring buffer 256)
- [ ] 3.4 Implement fullscreen window mode (guaranteed all platforms)
- [ ] 3.5 Implement overlay/HUD window mode with platform-specific click-through (Windows WM_NCHITTEST, macOS hitTest, X11 XShape, wlroots wlr-layer-shell)
- [ ] 3.6 Implement headless mode with offscreen texture surface and HEADLESS_FORCE_SOFTWARE env var
- [ ] 3.7 Implement 5-level degradation ladder with frame_time_p95 > 14ms trigger and hysteresis recovery
- [ ] 3.8 Implement tile shedding: sort by (lease_priority ASC, z_order DESC)
- [ ] 3.9 Implement admission control and per-agent session envelope
- [ ] 3.10 Implement compositor surface trait abstraction
- [ ] 3.11 Write Layer 1 tests: headless rendering, pixel readback, z-order compositing, alpha blending

## 4. Input Model

- [ ] 4.1 Implement focus tree structure with acquisition, cycling, and focus events
- [ ] 4.2 Implement pointer capture protocol (acquire, release, theft, auto-capture on HitRegionNode)
- [ ] 4.3 Implement HitRegionNode primitive with local pressed/hovered state and agent callback
- [ ] 4.4 Implement pointer events (down, up, move, enter, leave, click, cancel) and keyboard events (key_down, key_up, character)
- [ ] 4.5 Implement local feedback guarantee: runtime-owned visual state, SceneLocalPatch rendering, rollback on agent rejection
- [ ] 4.6 Implement event dispatch pipeline: capture → hit-test → route → deliver with bubbling (node → tile → tab → runtime)
- [ ] 4.7 Implement event serialization, batching, coalescing, and backpressure
- [ ] 4.8 Implement abstract command input model: CommandInputEvent with 7 actions (NAVIGATE_NEXT, NAVIGATE_PREV, ACTIVATE, CANCEL, CONTEXT, SCROLL_UP, SCROLL_DOWN), sources (KEYBOARD, DPAD, VOICE, REMOTE_CLICKER, ROTARY_DIAL, PROGRAMMATIC), transactional delivery, and routing to focused session (RFC 0004 §10)
- [ ] 4.9 Write Layer 0 tests: focus tree invariants, hit-test correctness, event routing

## 5. Session Protocol

- [ ] 5.1 Implement session lifecycle state machine (Connecting → Handshaking → Active ⇄ Disconnecting → Closed → Resuming)
- [ ] 5.2 Implement SessionInit/SessionEstablished handshake with capability negotiation (enforce real capability gating — do NOT grant all requested)
- [ ] 5.3 Implement ClientMessage/ServerMessage multiplexed envelope with sequence numbers
- [ ] 5.4 Implement traffic class routing: Transactional (reliable, ordered, acked), State-stream (reliable, coalesced), Ephemeral realtime (low-latency, droppable)
- [ ] 5.5 Implement heartbeat protocol (5000ms interval, 3 missed threshold, 15s grace period)
- [ ] 5.6 Implement lease management RPCs (LeaseRequest ACQUIRE/RENEW/RELEASE, LeaseResponse, LeaseStateChange)
- [ ] 5.7 Implement subscription management with category filtering (9 categories)
- [ ] 5.8 Implement reconnection with full SceneSnapshot delivery and lease reclaim within grace period
- [ ] 5.9 Implement MCP bridge: guest tools (publish_to_zone, list_zones, list_scene) requiring no lease; resident tools (create_tab, create_tile, set_content, dismiss) gated by resident_mcp capability
- [ ] 5.10 Implement SessionSuspended/SessionResumed for safe mode signaling
- [ ] 5.11 Remove legacy unary scene service — streaming session protocol is the single authoritative resident path
- [ ] 5.12 Write protocol conformance tests: schema validation, error structure, version negotiation

## 6. Configuration

- [ ] 6.1 Implement TOML configuration schema with file resolution order
- [ ] 6.2 Implement display profiles: full-display and headless with budget parameters
- [ ] 6.3 Implement mobile profile rejection: fail at startup with CONFIG_MOBILE_PROFILE_NOT_EXERCISED
- [ ] 6.4 Implement auto-detection rules (headless branch, full-display branch, explicit-required fallback)
- [ ] 6.5 Implement zone registry configuration with v1 minimum zones
- [ ] 6.6 Implement agent registration with per-agent budget overrides and canonical capability vocabulary
- [ ] 6.7 Implement configuration validation at load time with structured error codes
- [ ] 6.8 Implement profile budget escalation prevention

## 7. Lease Governance

- [ ] 7.1 Implement lease state machine with all transitions (REQUESTED → ACTIVE → EXPIRED/ORPHANED/RECLAIMED/REVOKED/SUSPENDED)
- [ ] 7.2 Implement static priority assignment at grant time (0-255, chrome=0)
- [ ] 7.3 Implement auto-renewal policies (MANUAL, AUTO_RENEW at 75% TTL, ONE_SHOT)
- [ ] 7.4 Implement disconnect grace period (15s) with orphan → revoke transition
- [ ] 7.5 Implement resource budget enforcement: soft warning at 80%, hard reject at 100%, three-tier enforcement ladder
- [ ] 7.6 Implement capability scope enforcement (create_tiles, modify_own_tiles, manage_tabs, publish_zone:<type>, read_scene, resident_mcp)
- [ ] 7.7 Implement zone interaction: guest publishing does not acquire leases
- [ ] 7.8 Write Layer 0 tests: lease state machine transitions, budget enforcement, capability gating

## 8. System Shell

- [ ] 8.1 Implement chrome layer (always on top, independent of agent state)
- [ ] 8.2 Implement safe mode protocol: entry suspends (not revokes) agent leases, overlay indicator
- [ ] 8.3 Implement freeze, mute, dismiss-all override controls
- [ ] 8.3a Implement operator override input bindings for system-shell actions: dismiss, safe-mode, freeze, mute, tab-switch, focus-cycle, quit — these are local operator hotkeys that bypass the agent event pipeline and execute instantly (Level 0 arbitration)
- [ ] 8.4 Implement disconnection badges and budget warning badges
- [ ] 8.5 Implement tab bar rendering with keyboard shortcuts
- [ ] 8.6 Implement backpressure signals to agents
- [ ] 8.7 Implement audit events for operator actions (with privacy constraints)
- [ ] 8.8 Implement v1 diagnostic surface: CLI-based scene graph dump, lease listing, resource utilization, zone state, telemetry snapshot

## 9. Policy Arbitration

- [ ] 9.1 Implement 7-level arbitration stack with cross-level conflict resolution (higher level always wins)
- [ ] 9.2 Implement Level 0 (Human Override): dismiss, safe mode, freeze, mute — local, instant, never interceptable
- [ ] 9.3 Implement Level 1 (Safety): GPU failure two-phase response (safe mode first, shutdown if unrecoverable)
- [ ] 9.4 Implement Level 2 (Privacy): viewer context enforcement, redaction (privacy owns all redaction)
- [ ] 9.5 Implement Level 3 (Security): capability enforcement, lease validity checks
- [ ] 9.6 Implement Level 4 (Attention): interruption classification enforcement, quiet hours
- [ ] 9.7 Implement Level 5 (Resource): budget enforcement, degradation ladder integration
- [ ] 9.8 Implement Level 6 (Content): zone contention resolution, z-order arbitration
- [ ] 9.9 Implement per-mutation evaluation pipeline with < 100μs latency budget
- [ ] 9.10 Write Layer 0 tests: arbitration stack precedence, conflict resolution scenarios

## 10. Scene Events

- [ ] 10.1 Implement event taxonomy: input events, scene events, system events with SceneEvent envelope
- [ ] 10.2 Implement interruption classification (CRITICAL, HIGH, NORMAL, LOW, SILENT)
- [ ] 10.3 Implement quiet hours enforcement with queue semantics
- [ ] 10.4 Implement subscription model with 9 category types and prefix filtering
- [ ] 10.5 Implement event bus pipeline: classify → filter → coalesce → deliver
- [ ] 10.6 Implement tab_switch_on_event contract
- [ ] 10.7 Implement agent event emission with capability gating and rate limiting (10/s per agent, 1000/s aggregate)
- [ ] 10.8 Write Layer 0 tests: event classification, subscription filtering, quiet hours, tab switch triggers

## 11. Resource Store

- [ ] 11.1 Implement content-addressed ResourceId (BLAKE3, 32 bytes) with resource immutability
- [ ] 11.2 Implement upload protocol: session stream upload with chunked flow and hash verification
- [ ] 11.3 Implement v1 resource type validation (IMAGE_RGBA8/PNG/JPEG, FONT_TTF/OTF)
- [ ] 11.4 Implement content-addressed deduplication (< 100μs lookup)
- [ ] 11.5 Implement reference counting with per-agent accounting and cross-agent sharing semantics
- [ ] 11.6 Implement GC with grace period (60s), cycle timing (30s, 5ms budget), and frame render isolation
- [ ] 11.7 Implement per-resource size limits and per-runtime totals
- [ ] 11.8 Implement font asset management with fallback chain and LRU cache (64 MiB)
- [ ] 11.9 Write Layer 0 tests: upload, dedup, refcounting, GC lifecycle, budget enforcement

## 12. Validation Framework

- [ ] 12.1 Implement hardware-normalized calibration harness with 3 workloads (scene-graph CPU, fill/composition GPU, upload-heavy resource)
- [ ] 12.2 Implement Layer 0 test infrastructure: pure logic, < 2s, 60%+ coverage target
- [ ] 12.3 Implement Layer 1 test infrastructure: headless pixel readback with tolerance assertions (±2/channel software GPU)
- [ ] 12.4 Implement Layer 2 test infrastructure: SSIM visual regression (0.995 layout, 0.99 composition) with golden reference management
- [ ] 12.5 Implement Layer 3 benchmark binary: per-frame structured telemetry, JSON emission, split latency budget validation
- [ ] 12.6 Implement Layer 4 developer visibility artifacts: index.html gallery, manifest.json, per-scene outputs, CI integration
- [ ] 12.7 Create initial test scene registry (25 named scenes per validation-framework spec)
- [ ] 12.8 Implement protocol conformance test suite
- [ ] 12.9 Implement record/replay trace infrastructure for debugging and regression
- [ ] 12.10 Implement soak/leak test harness (5% tolerance at hour N vs hour 1)

## 13. Integration and Convergence

- [ ] 13.1 Converge all capability names to canonical vocabulary from configuration spec (eliminate legacy naming)
- [ ] 13.2 Ensure MCP surface enforces guest/resident distinction — guest tools require no lease, resident tools gated by resident_mcp capability
- [ ] 13.3 Validate all protobuf definitions match session-protocol spec envelope field allocations
- [ ] 13.4 Run full cross-spec integration test: 3 agents, leased tiles, zone publishing, input events, degradation, at 60fps
- [ ] 13.5 Validate all quantitative budgets met on reference hardware with calibration normalization
