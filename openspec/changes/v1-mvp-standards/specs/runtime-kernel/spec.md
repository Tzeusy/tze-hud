# Runtime Kernel Specification

Domain: FOUNDATION
Source RFC: 0002 (Runtime Kernel)

---

## ADDED Requirements

### Requirement: Single-Process Model
tze_hud MUST run as a single OS process. Agents SHALL be external gRPC clients that do not share the compositor's address space. The compositor MUST be the trusted, sovereign process that owns the GPU context, scene state, input stream, and window surface.
Source: RFC 0002 §1.1
Scope: v1-mandatory

#### Scenario: Process isolation
- **WHEN** an agent connects to the runtime
- **THEN** the agent MUST communicate via gRPC or MCP over IPC/network; it MUST NOT share the compositor's address space

### Requirement: Thread Model
The runtime MUST use a fixed, small set of threads with explicit responsibilities: main thread (winit event loop, input drain, local feedback, frame presentation), compositor thread (scene commit, render encode, GPU submit), network thread(s) (Tokio multi-thread runtime for gRPC server, MCP bridge, session management), and telemetry thread (async structured emission). Thread count MUST be determined at startup; no dynamic thread spawning during normal operation.
Source: RFC 0002 §2.1
Scope: v1-mandatory

#### Scenario: Thread responsibilities
- **WHEN** the runtime is running
- **THEN** the main thread MUST handle input drain, local feedback, and surface.present(); the compositor thread MUST handle scene commit, render encode, and GPU submit; network threads MUST NOT touch the scene graph or GPU state directly

#### Scenario: No dynamic thread spawning
- **WHEN** the runtime is in normal operation
- **THEN** no new OS threads SHALL be spawned beyond the threads created at startup

### Requirement: Main Thread Responsibilities
The main thread MUST run the winit event loop. It SHALL drain OS input events, apply press/hover local feedback, call surface.present() when signaled by the compositor thread, handle resize events, and initiate shutdown. The main thread MUST NOT encode render commands or submit GPU work. Main thread SHALL be elevated to high priority at startup using platform-appropriate mechanisms; failure to elevate MUST NOT fail startup, and the runtime MUST log a warning and continue at normal OS-default priority.
Source: RFC 0002 §2.2
Scope: v1-mandatory

#### Scenario: Input drain does not block on GPU
- **WHEN** the compositor thread is encoding render commands
- **THEN** the main thread MUST continue processing input events without waiting for the compositor thread

#### Scenario: Thread priority elevation fallback
- **WHEN** the runtime lacks privileges for elevated thread priority
- **THEN** the main thread MUST log a warning and continue at normal priority without failing startup

### Requirement: Compositor Thread Ownership
The compositor thread MUST own the wgpu Device and Queue exclusively. No other thread SHALL touch the device. The main thread SHALL hold the surface handle and be the only thread that calls surface.present(). This split is driven by macOS/Metal requirements (present must be on main thread) and frame-budget separation.
Source: RFC 0002 §2.3, §2.7
Scope: v1-mandatory

#### Scenario: GPU device ownership
- **WHEN** any code path attempts to call wgpu Device or Queue methods
- **THEN** only the compositor thread SHALL execute those calls

#### Scenario: Surface present on main thread
- **WHEN** a frame is ready for presentation
- **THEN** the compositor thread MUST signal the main thread via FrameReadySignal, and only the main thread SHALL call surface.present()

### Requirement: Frame Pipeline Stages
Each frame MUST pass through 8 stages in order. Stages 1-2 run on the main thread; stages 3-7 run on the compositor thread; stage 8 runs on the telemetry thread. The pipeline MUST support temporal overlap: GPU work for frame N executes concurrently with input drain for frame N+1.
Source: RFC 0002 §3.1
Scope: v1-mandatory

#### Scenario: Pipeline ordering
- **WHEN** a frame is processed
- **THEN** stages MUST execute in order 1 through 8, with stages 1-2 on main thread, 3-7 on compositor thread, and 8 on telemetry thread

#### Scenario: Pipeline overlap
- **WHEN** GPU work for frame N is executing
- **THEN** the compositor thread MUST be able to begin Stage 3 mutation intake for frame N+1 concurrently

### Requirement: Stage 1 Input Drain
Stage 1 (Input Drain) MUST run on the main thread with a p99 budget of < 500us. It SHALL drain all pending OS input events from the winit event queue, attach hardware timestamps, produce InputEvent records, and enqueue to the InputEvent channel. Input drain MUST never block on downstream processing; if the compositor is slow, inputs SHALL be queued or dropped.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Input drain latency
- **WHEN** Stage 1 Input Drain is executed
- **THEN** it MUST complete in < 500us p99

#### Scenario: Non-blocking input
- **WHEN** the compositor thread is overloaded
- **THEN** the main thread MUST continue draining input events, dropping oldest if the channel is full

### Requirement: Stage 2 Local Feedback
Stage 2 (Local Feedback) MUST run on the main thread with a p99 budget of < 500us. It SHALL hit-test input events against the current tile bounds snapshot (stored as ArcSwap for lock-free access) and update pressed/hovered local state flags. Local feedback MUST complete within 1ms of input arrival (stages 1+2 combined) without waiting for agent response, scene commit, or any network round-trip.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Local feedback latency
- **WHEN** a pointer event arrives and is processed through stages 1 and 2
- **THEN** the combined stage 1+2 time MUST be < 1ms p99, providing visual feedback without agent involvement

#### Scenario: Lock-free hit-test snapshot
- **WHEN** the main thread reads the hit-test snapshot during Stage 2
- **THEN** it MUST use ArcSwap (pointer-width atomic load) with no mutex, seeing either the old or new snapshot consistently

### Requirement: Stage 3 Mutation Intake
Stage 3 (Mutation Intake) MUST run on the compositor thread with a p99 budget of < 1ms. It SHALL drain the MutationBatch channel and apply agent envelope limits (max_nodes_per_tile, max_texture_bytes). Batches MUST never be coalesced; each MutationBatch is the unit of atomicity with independent acknowledgement.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Mutation intake latency
- **WHEN** mutation batches are drained from the channel
- **THEN** Stage 3 MUST complete in < 1ms p99

#### Scenario: Batches never coalesced
- **WHEN** multiple MutationBatches arrive in the same frame
- **THEN** each batch MUST be validated, committed, and acknowledged independently; batches MUST NOT be merged

### Requirement: Stage 4 Scene Commit
Stage 4 (Scene Commit) MUST run on the compositor thread with a p99 budget of < 1ms. It SHALL apply validated mutation batches to the scene graph with all-or-nothing semantics per batch. After commit, it MUST publish the updated hit-test snapshot via ArcSwap.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Scene commit latency
- **WHEN** validated mutation batches are committed to the scene graph
- **THEN** Stage 4 MUST complete in < 1ms p99

### Requirement: Stage 5 Layout Resolve
Stage 5 (Layout Resolve) MUST run on the compositor thread with a p99 budget of < 1ms. It SHALL recompute layout for tiles that changed this frame only (incremental). It MUST validate bounds, recompute z-order stack, and compute compositing regions.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Incremental layout
- **WHEN** only 3 tiles changed in a frame with 50 total tiles
- **THEN** Stage 5 MUST recompute layout only for the 3 changed tiles, completing in < 1ms p99

### Requirement: Stage 6 Render Encode
Stage 6 (Render Encode) MUST run on the compositor thread with a p99 budget of < 4ms. It SHALL build wgpu CommandEncoder from the RenderFrame, issue draw calls for tile nodes (solid color, text, image), encode alpha-blend passes for transparent tiles, and encode the chrome layer. Render encoding MUST NOT submit to the GPU queue.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Render encode latency
- **WHEN** the render frame is encoded
- **THEN** Stage 6 MUST complete in < 4ms p99

### Requirement: Stage 7 GPU Submit and Present
Stage 7 (GPU Submit + Present) MUST run on the compositor thread (submit) and main thread (present) with a combined p99 budget of < 8ms. The compositor thread SHALL submit encoded CommandBuffer to the wgpu queue and signal the main thread via FrameReadySignal. The main thread SHALL call surface.present(). In headless mode, present() is a no-op.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: GPU submit latency
- **WHEN** a frame is submitted and presented
- **THEN** Stage 7 MUST complete in < 8ms p99 (including GPU execution time)

### Requirement: Stage 8 Telemetry Emit
Stage 8 (Telemetry Emit) MUST run on the telemetry thread with a p99 budget of < 200us. The compositor thread SHALL send a TelemetryRecord to the telemetry thread via a non-blocking bounded channel send. If the channel is full, the compositor thread MUST drop the oldest unprocessed record and emit a telemetry_overflow counter. Telemetry MUST never block the frame pipeline.
Source: RFC 0002 §3.2
Scope: v1-mandatory

#### Scenario: Non-blocking telemetry
- **WHEN** the telemetry sink is slow and the channel is full
- **THEN** the compositor thread MUST drop the oldest record and continue without blocking; telemetry_overflow_count MUST be incremented

### Requirement: Frame Pipeline Total Budget
The total frame pipeline (Stage 1 start through Stage 7 end) MUST complete in p99 < 16.6ms (normalized to reference hardware) at 60fps. The combined Stages 1+2 MUST complete in p99 < 1ms. The input_to_local_ack latency MUST be p99 < 4ms. The input_to_scene_commit latency MUST be p99 < 50ms for local agents (covering network round-trip through agent response). The input_to_next_present latency MUST be p99 < 33ms.
Source: RFC 0002 §9
Scope: v1-mandatory

#### Scenario: Total frame time
- **WHEN** a frame is processed under normal load
- **THEN** the total pipeline time (Stage 1 start to Stage 7 end) MUST be < 16.6ms p99 on reference hardware

#### Scenario: Input to local acknowledgement
- **WHEN** an input event arrives
- **THEN** local visual feedback MUST be provided within 4ms p99

#### Scenario: Input to scene commit
- **WHEN** an input event arrives and is processed by a local agent
- **THEN** the agent's response mutation MUST be applied to the scene graph within 50ms p99 of the original input event (input_to_scene_commit, covering full network round-trip)

### Requirement: Window Modes
The runtime MUST support two window modes configured at startup: Fullscreen (compositor owns entire display, all input captured, all platforms) and Overlay/HUD (transparent borderless always-on-top window, per-region input passthrough, platform-specific). Runtime mode switching MUST be supported but is a disruptive operation requiring surface recreation.
Source: RFC 0002 §7.1
Scope: v1-mandatory

#### Scenario: Fullscreen mode
- **WHEN** the runtime starts in fullscreen mode
- **THEN** the compositor MUST own the entire display with an opaque background and all input captured

#### Scenario: Overlay click-through
- **WHEN** the runtime is in overlay mode and a pointer event lands outside any active hit-region
- **THEN** the event MUST pass through to the underlying desktop

#### Scenario: Unsupported overlay fallback
- **WHEN** the runtime starts in overlay mode on GNOME Wayland (no layer-shell)
- **THEN** the runtime MUST fall back to fullscreen silently with a startup warning logged

### Requirement: Platform GPU Backends
The runtime MUST support these GPU backends per platform: Vulkan on Linux, D3D12 and Vulkan on Windows, Metal on macOS. GPU device initialization MUST be fatal if no suitable adapter exists.
Source: RFC 0002 §1.2, §8.3
Scope: v1-mandatory

#### Scenario: No GPU adapter
- **WHEN** no suitable GPU adapter is found during initialization
- **THEN** the runtime MUST fail with a fatal error and structured error message

### Requirement: Headless Mode
Headless mode MUST use the same process, code path, and pipeline as windowed mode. The only difference SHALL be the render surface: a wgpu::Texture with RENDER_ATTACHMENT | COPY_SRC usage instead of a window-backed surface. Headless mode MUST be selectable via config or --headless flag. No conditional compilation for the render path. Headless surface present() MUST be a no-op; pixel readback MUST be on-demand.
Source: RFC 0002 §1.3, §8.1
Scope: v1-mandatory

#### Scenario: Headless same code path
- **WHEN** the runtime starts in headless mode
- **THEN** the same frame pipeline MUST execute; only the render surface implementation differs

#### Scenario: Headless pixel readback
- **WHEN** a test requests frame contents via ReadbackFrame RPC or in-process readback
- **THEN** the runtime MUST return the last rendered frame's pixel data via copy_texture_to_buffer

### Requirement: Headless Software GPU
The headless mode MUST support a configurable environment variable to force software GPU fallback. When set, the wgpu adapter selection MUST request a software fallback (force_fallback_adapter = true). Software fallback MUST be available when the platform provides one. In CI, software fallback MUST be enabled.
Source: RFC 0002 §8.3
Scope: v1-mandatory

#### Scenario: Software GPU in CI
- **WHEN** software GPU fallback is enabled via the configured environment variable (see Implementation Notes for the conventional variable name)
- **THEN** the runtime MUST use the available software GPU fallback regardless of hardware GPU availability

### Requirement: Degradation Ladder
The runtime MUST implement a 6-level degradation ladder: Level 0 Normal, Level 1 Coalesce (reduce outbound SceneEvent frequency for state-stream tiles by a configurable ratio), Level 2 ReduceTextureQuality (scale down large textures by a configurable factor), Level 3 DisableTransparency (force semi-transparent tiles to opaque, skip alpha-blend passes), Level 4 ShedTiles (remove lowest-priority tiles from render pass sorted by lease_priority ASC, z_order DESC), Level 5 Emergency (render only chrome layer + highest-priority single tile).
Source: RFC 0002 §6.2
Scope: v1-mandatory

#### Scenario: Level 1 coalescing
- **WHEN** frame_time_p95 > 14ms over 10 frames and the runtime is at Normal
- **THEN** the runtime MUST transition to Level 1 and reduce outbound state-stream notification frequency

#### Scenario: Level 4 tile shedding
- **WHEN** the runtime reaches Level 4
- **THEN** tiles MUST be sorted by (lease_priority ASC, z_order DESC) and the lowest-priority tiles removed from the render pass while remaining in the scene graph

#### Scenario: Level 5 emergency
- **WHEN** the runtime reaches Level 5
- **THEN** only the chrome layer and the single highest-priority tile MUST be rendered; all other tiles MUST be visually suppressed

### Requirement: Degradation Trigger
Degradation evaluation MUST occur after every frame. The trigger condition SHALL be: frame_time_p95 > 14ms measured over a rolling 10-frame window (166ms at 60fps). The 10-frame window gives the system time to absorb transient spikes without triggering degradation for a momentary blip.
Source: RFC 0002 §6.1
Scope: v1-mandatory

#### Scenario: Transient spike tolerance
- **WHEN** a single frame exceeds 14ms but the 10-frame p95 remains below 14ms
- **THEN** the runtime MUST NOT trigger degradation

#### Scenario: Sustained overbudget
- **WHEN** the 10-frame rolling p95 exceeds 14ms
- **THEN** the runtime MUST advance one degradation level within 1 frame

### Requirement: Degradation Hysteresis
Recovery from degradation MUST require frame_time_p95 < 12ms sustained over a 30-frame window (500ms at 60fps). Recovery SHALL move up one level at a time. The 2ms hysteresis band (14ms trigger vs 12ms recovery) MUST prevent oscillation between levels.
Source: RFC 0002 §6.3
Scope: v1-mandatory

#### Scenario: Recovery threshold
- **WHEN** the runtime is at Level 2 and frame_time_p95 < 12ms sustained over a 30-frame rolling window (500ms at 60fps)
- **THEN** the runtime MUST recover one level to Level 1

#### Scenario: Full recovery time
- **WHEN** the runtime is at Level 5
- **THEN** reaching Normal MUST require 5 successive 30-frame windows of clean performance (frame_time_p95 < 12ms per window), recovering one level per window (~2.5 seconds total at 60fps)

### Requirement: Tile Shedding Order
When shedding tiles (Level 4 or frame-time guardian), the runtime MUST sort tiles by (lease_priority ASC, z_order DESC). Lower lease_priority values (0 = highest priority) SHALL be preserved first; within the same priority class, higher z_order wins. Tiles with the highest lease_priority values and lowest z_order SHALL be shed first.
Source: RFC 0002 §5.2, §6.2
Scope: v1-mandatory

#### Scenario: Priority-based shedding
- **WHEN** the runtime enters Level 4 with tiles at priority 0, 1, and 2
- **THEN** priority-2 tiles MUST be shed first, then priority-1, preserving priority-0 tiles

### Requirement: Channel Topology
All inter-thread communication MUST use bounded channels. No unbounded queues. Channels with drop-oldest semantics (InputEvent, SceneLocalPatch, SceneEventEphemeral, TelemetryRecord) MUST use ring buffers. SceneEventTransactional (capacity 256) MUST use backpressure (never dropped). SceneEventStateStream (capacity 512) MUST use coalesce-key merging (intermediate states skipped, not dropped). FrameReadySignal MUST use tokio::sync::watch (latest value wins).
Source: RFC 0002 §2.6
Scope: v1-mandatory

#### Scenario: Transactional events never dropped
- **WHEN** the SceneEventTransactional channel is full
- **THEN** the compositor thread MUST block (apply backpressure) rather than dropping the event

#### Scenario: State-stream coalescing
- **WHEN** the SceneEventStateStream channel is full
- **THEN** a new state-stream event for the same (tile_id, event_kind) MUST replace the pending entry rather than being dropped or blocking

#### Scenario: Ephemeral event ring buffer
- **WHEN** the SceneEventEphemeral ring buffer is full
- **THEN** the oldest event MUST be dropped and overflow counted in telemetry

### Requirement: Agent Connection Latency
Total time from TCP connect to session established MUST be < 50ms on loopback. This budget covers TCP handshake, auth validation (PSK: < 1ms; OIDC: < 30ms), capability negotiation, and session stream setup.
Source: RFC 0002 §4.1, §9
Scope: v1-mandatory

#### Scenario: Loopback connection speed
- **WHEN** an agent connects to the runtime on loopback
- **THEN** the time from TCP connect to SessionAck MUST be < 50ms

### Requirement: Session Memory Overhead
Session memory overhead (metadata, session state, event subscription buffers) MUST be < 64 KB per agent session, exclusive of content (textures, node data).
Source: RFC 0002 §4.3, §9
Scope: v1-mandatory

#### Scenario: Memory per session
- **WHEN** an agent session is established at steady state
- **THEN** the session overhead (excluding content) MUST be less than 64 KB

### Requirement: Per-Agent Resource Envelope
Each session MUST be assigned configurable resource limits at capability negotiation: max_tiles (default 8, hard max 64), max_texture_bytes (default 256 MiB, hard max 2 GiB), max_update_rate_hz (default 30, hard max 120), max_nodes_per_tile (default 32, hard max 64), max_active_leases (default 8, hard max 64).
Source: RFC 0002 §4.3
Scope: v1-mandatory

#### Scenario: Default envelope enforcement
- **WHEN** an agent without custom config creates its 9th tile
- **THEN** the mutation MUST be rejected because max_tiles default is 8

### Requirement: Budget Enforcement Tiers
Per-agent budget enforcement MUST use a three-tier ladder: Warning (any limit exceeded; send BudgetWarning event), Throttle (warning unresolved for 5s; reduce max_update_rate_hz by 50%), Revocation (throttle sustained for 30s or critical limit; revoke all leases, terminate session). Critical triggers (OOM attempt, > 10 invariant violations, protocol violations) MUST bypass the ladder and go directly to revocation.
Source: RFC 0002 §5.2
Scope: v1-mandatory

#### Scenario: Budget warning
- **WHEN** an agent exceeds a resource limit
- **THEN** the runtime MUST send a BudgetWarning event and enter Warning tier

#### Scenario: Critical revocation
- **WHEN** an agent produces more than 10 invariant violations in a session
- **THEN** the runtime MUST immediately revoke all leases and terminate the session without the warning/throttle ladder

#### Scenario: Post-revocation cleanup
- **WHEN** a session is revoked
- **THEN** the runtime MUST free all agent-owned textures and node data after a configurable post-revocation delay in the range [0ms, 5000ms] (see Implementation Notes for default), reducing the agent's resource footprint to zero

### Requirement: Graceful Shutdown
Shutdown MUST follow an ordered sequence: (1) stop accepting new connections, (2) drain active mutations (wait up to a configurable drain timeout in the range [0ms, 60000ms] for compositor thread to finish current frame), (3) revoke all leases without waiting for acknowledgement, (4) flush telemetry with a configurable grace period in the range [0ms, 10000ms], (5) terminate agent sessions, (6) GPU drain via device.poll(Wait), (7) release resources with reference counts reaching zero, (8) exit process (code 0 for clean, non-zero for error). See Implementation Notes for suggested defaults.
Source: RFC 0002 §1.4
Scope: v1-mandatory

#### Scenario: Ordered shutdown
- **WHEN** the runtime receives SIGTERM
- **THEN** it MUST stop accepting connections, drain the compositor, revoke leases, flush telemetry, and exit cleanly with code 0

#### Scenario: Fatal GPU error
- **WHEN** the GPU device is lost
- **THEN** the runtime MUST flush telemetry, attempt safe mode overlay (RFC 0007), then trigger graceful shutdown with non-zero exit code

### Requirement: Hot-Connect
Agents connecting while a scene is active MUST receive a full scene snapshot as part of SessionAck. Snapshot delivery MUST be handled on network threads and MUST NOT block the compositor thread. Hot-connect MUST be non-disruptive to existing agents.
Source: RFC 0002 §4.4
Scope: v1-mandatory

#### Scenario: Non-disruptive hot-connect
- **WHEN** a new agent connects while the runtime is rendering frames for existing agents
- **THEN** the new agent MUST receive a SceneSnapshot and the compositor MUST continue rendering uninterrupted for existing agents

### Requirement: Session Limits
Configurable session limits with defaults: resident agent sessions (default 16, max 256), guest agent sessions (default 64, max 1024), total concurrent sessions (default 80, max 1280). When the resident session limit is reached, new resident connections MUST receive RESOURCE_EXHAUSTED with a structured body indicating current capacity and estimated wait hint.
Source: RFC 0002 §4.2
Scope: v1-mandatory

#### Scenario: Resident session limit
- **WHEN** 16 resident sessions are active and a 17th attempts to connect
- **THEN** the runtime MUST reject with RESOURCE_EXHAUSTED and provide a structured capacity indicator

### Requirement: Compositor Surface Trait
The render surface MUST be abstracted behind a CompositorSurface trait with acquire_frame(), present(), and size() methods. CompositorFrame MUST bundle the TextureView with an ownership guard (_guard: Box<dyn Any + Send>) to keep the SurfaceTexture alive until after present(). WindowSurface and HeadlessSurface MUST implement this trait.
Source: RFC 0002 §1.3
Scope: v1-mandatory

#### Scenario: SurfaceTexture lifetime safety
- **WHEN** a frame is acquired and transferred between threads
- **THEN** the CompositorFrame _guard MUST keep the SurfaceTexture alive until present() is called

### Requirement: Media Worker Pool (Deferred)
The media worker pool boundary (GStreamer internal scheduler, DecodedFrameReady channel with capacity 4 per stream) is reserved for post-v1. The pool MUST NOT be spawned in v1. The channel interface and GPU device ownership rules are pre-defined so that adding media workers post-v1 does not require restructuring the thread model.
Source: RFC 0002 §2.8
Scope: post-v1

#### Scenario: Media pool not spawned
- **WHEN** the v1 runtime starts
- **THEN** no GStreamer or WebRTC threads SHALL be created; the DecodedFrameReady channel slot MUST remain empty

### Requirement: Parallel Render Encoding (Deferred)
Parallel render encoding (multiple CommandEncoder instances recorded in parallel for Stage 6) is deferred to post-v1 based on profiling data.
Source: RFC 0002 §10
Scope: post-v1

#### Scenario: Single-threaded render encoding
- **WHEN** Stage 6 executes in v1
- **THEN** render encoding MUST be single-threaded on the compositor thread

---

## Appendix: Implementation Notes

This appendix collects platform-specific implementation details and tunable defaults that inform but do not constitute normative requirements. Implementations MAY differ from these notes provided all normative requirements above are satisfied.

### Thread Priority Elevation

The main thread should be elevated to high priority at startup using platform-appropriate OS mechanisms. Suggested platform mappings:

- **Linux**: `SCHED_RR` real-time scheduling policy via `pthread_setschedparam`
- **macOS**: `QOS_CLASS_USER_INTERACTIVE` quality-of-service class
- **Windows**: `THREAD_PRIORITY_TIME_CRITICAL` thread priority

Elevation failure is non-fatal; the thread continues at normal OS-default priority with a warning logged.

### Headless Software GPU Fallback

The force-software environment variable is conventionally named `HEADLESS_FORCE_SOFTWARE`. When set to `1`, the wgpu adapter selection uses `force_fallback_adapter = true`. Platform-provided software GPU backends include llvmpipe on Linux and WARP on Windows. CI pipelines should set this variable to guarantee reproducible rendering without hardware GPU.

### Degradation Ladder Ratios

Suggested default ratios for the degradation levels. Implementations MAY expose configuration options to adjust these values:

- **Level 1 Coalesce**: reduce outbound state-stream SceneEvent frequency by 2x (one notification per two frames)
- **Level 2 ReduceTextureQuality**: scale down textures whose linear dimensions exceed 512px to 50% of their original linear dimensions
- **Level 3 DisableTransparency**: force all semi-transparent tiles to opaque; skip alpha-blend render passes entirely

These ratios represent validated defaults. Operators with specialized hardware profiles MAY override them using implementation-defined configuration mechanisms.

### Post-Revocation Cleanup Delay

The bounded delay between session revocation and resource release is configurable. Suggested default: 100ms. This window allows in-flight GPU commands referencing agent textures to drain before the backing memory is freed. Setting this value to zero is permitted for environments where GPU drain is guaranteed by other means.

### Graceful Shutdown Timeouts

Suggested defaults for the configurable shutdown grace periods:

- **Mutation drain timeout** (step 2): 500ms — allows the compositor thread to finish its current frame before shutdown proceeds
- **Telemetry flush grace** (step 4): 200ms — allows queued telemetry records to be emitted before process exit

These timeouts may be shortened in test environments or lengthened on slow hardware.
