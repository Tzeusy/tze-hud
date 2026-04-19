# RFC 0002: Runtime Kernel

**Status:** Draft
**Issue:** rig-5vq.2
**Date:** 2026-03-22
**Authors:** tze_hud architecture team

---

## Summary

This RFC specifies the Runtime Kernel — the execution model for the tze_hud compositor process. It defines the process architecture, thread model, frame pipeline, admission control, budget enforcement, degradation policy, window surface management, and headless mode. This is the execution contract that all other implementation decisions depend on.

The Runtime Kernel RFC complements RFC 0001 (Scene Contract). RFC 0001 defines *what* is in the scene; this RFC defines *how the process runs* to render it at 60fps with governed latency.

---

## Motivation

tze_hud gives LLMs governed, performant presence on real screens. That presence is only meaningful if the runtime delivers it predictably — consistent frame timing, bounded latency, safe degradation under load. Without a precise execution model:

- Frame timing budgets cannot be enforced or measured.
- Thread boundaries are unclear, making data races and priority inversions likely.
- Admission control is ad-hoc, allowing misbehaving agents to destabilize the runtime.
- Degradation under load is reactive and inconsistent rather than designed.

The Runtime Kernel resolves all of these by specifying the process as a collection of well-defined threads, bounded channels, and per-frame pipeline stages with hard time budgets.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| DR-V2: Headless rendering | Offscreen texture surface; same pipeline, no display server. |
| DR-V3: Structured telemetry | Telemetry thread with per-frame emission in the pipeline. |
| DR-V5: Trivial headless invocation | Headless mode is a runtime flag, not a compile fork. |
| DR-V6: No physical GPU required for CI | HEADLESS_FORCE_SOFTWARE env var forces llvmpipe/WARP on all platforms (§8.3). |

---

## 1. Process Architecture

### 1.1 Single-Process Model

tze_hud runs as a single OS process. Agents are external gRPC clients; they do not share the compositor's address space. The compositor is the trusted, sovereign process — it owns the GPU context, the scene state, the input stream, and the window surface. Agents interact through the gRPC resident control plane (RFC 0005) and the MCP compatibility plane; the Timing RFC (RFC 0003) defines timing semantics for payloads on both planes. (T-12: corrected erroneous reference to RFC 0003 as "the gRPC control plane.")

```
┌──────────────────────────────────────────────────────────────────┐
│  tze_hud compositor process                                       │
│                                                                   │
│  ┌──────────────┐  ┌────────────────┐  ┌──────────────────────┐  │
│  │  Main thread  │  │ Compositor     │  │  Network thread(s)   │  │
│  │               │  │ thread         │  │  (tokio runtime)     │  │
│  │  winit loop   │  │                │  │                      │  │
│  │  input drain  │  │  scene commit  │  │  gRPC server         │  │
│  │  local ack    │  │  render encode │  │  agent sessions      │  │
│  │  presentation │  │  GPU submit    │  │  MCP bridge          │  │
│  └──────┬────────┘  └──────┬─────────┘  └──────────┬───────────┘  │
│         │                  │                        │              │
│         │    channels      │    channels            │              │
│         └──────────────────┴────────────────────────┘             │
│                                                                   │
│  ┌──────────────────────────────────────────────────┐            │
│  │  Telemetry thread                                │            │
│  │  async structured emission, non-blocking         │            │
│  └──────────────────────────────────────────────────┘            │
│                                                                   │
│  ┌──────────────────────────────────────────────────┐            │
│  │  Media Worker Pool (post-v1, not spawned in v1)  │            │
│  │  managed by GStreamer's internal scheduler        │            │
│  │  decode, clock sync, timed metadata              │            │
│  │  → DecodedFrameReady channel → compositor thread │            │
│  └──────────────────────────────────────────────────┘            │
│                                                                   │
│  ┌──────────────────────────────────────────────────┐            │
│  │  wgpu Device / Queue (GPU state)                 │            │
│  │  owned by compositor thread; main thread has      │            │
│  │  surface handle for present() (see §2.7)         │            │
│  └──────────────────────────────────────────────────┘            │
└──────────────────────────────────────────────────────────────────┘

          ▲                              ▲
          │  gRPC (protobuf/HTTP2)        │  MCP (JSON-RPC)
          │                              │
   ┌──────┴──────┐                ┌──────┴──────┐
   │  Agent A    │                │  Agent B    │
   │  (external) │                │  (external) │
   └─────────────┘                └─────────────┘
```

### 1.2 Entry Point

The compositor entry point initializes components in this order:

1. **Configuration load.** Parse and validate config (TOML). Fail loudly with structured errors on any invalid field.
2. **Telemetry thread start.** Async structured emitter starts first so all subsequent initialization is observable.
3. **Tokio runtime start.** Multi-thread Tokio runtime for gRPC and network work.
4. **GPU device init.** wgpu instance, adapter selection, device + queue creation. Fatal if no suitable adapter exists.
5. **Window/surface creation.** winit window in configured mode (fullscreen or overlay). In headless mode, creates an offscreen texture surface.
6. **Scene graph init.** Empty scene, zone registry loaded from config.
7. **gRPC server bind.** tonic server binds and starts accepting connections.
8. **MCP bridge bind.** JSON-RPC endpoint bound.
9. **winit event loop run.** This call blocks until shutdown. All subsequent execution is event-driven.

### 1.3 Headless Mode

Headless mode uses the same process, the same code path, and the same pipeline. The only difference is the render surface:

- **Windowed:** `wgpu::Surface` backed by a `winit::Window`.
- **Headless:** `wgpu::Texture` created with `COPY_SRC` usage for pixel readback.

The mode is selected at startup via config or `--headless` flag. No runtime fork. No conditional compilation for the render path. The compositor does not know or care which surface it is rendering into — the surface abstraction is behind a trait.

```rust
pub trait CompositorSurface: Send + 'static {
    /// Acquire the next frame. The returned CompositorFrame must be kept alive
    /// for the duration of the render pass and passed to present() when done.
    /// For WindowSurface this wraps wgpu::SurfaceTexture (which must outlive
    /// its TextureView); for HeadlessSurface the guard is a no-op.
    fn acquire_frame(&self) -> CompositorFrame;
    fn present(&self, frame: CompositorFrame);
    fn size(&self) -> (u32, u32);
}

/// Bundles the TextureView with the implementation-specific ownership guard.
/// Dropping a CompositorFrame before present() is called is a correctness
/// error — the guard keeps the underlying SurfaceTexture alive.
pub struct CompositorFrame {
    pub view: wgpu::TextureView,
    // Holds the SurfaceTexture for WindowSurface, or a no-op for HeadlessSurface.
    // Box<dyn Any + Send> avoids making CompositorSurface generic over frame types.
    _guard: Box<dyn std::any::Any + Send>,
}

pub struct WindowSurface { /* winit + wgpu::Surface */ }
pub struct HeadlessSurface { /* wgpu::Texture, optionally with readback buffer */ }
```

**Soundness note (T-1):** The previous `current_texture() -> wgpu::TextureView` signature was unsound for `WindowSurface`: `wgpu::Surface::get_current_texture()` returns a `SurfaceTexture` that must remain alive until after `present()`. Discarding it at the trait boundary causes the `TextureView` to dangle. `CompositorFrame` makes the ownership explicit.

### 1.4 Graceful Shutdown

Shutdown is triggered by OS signal (SIGTERM/SIGINT), explicit shutdown RPC, or fatal internal error. The shutdown sequence is ordered:

1. **Stop accepting new connections.** gRPC and MCP servers stop accepting; existing sessions are notified.
2. **Drain active mutations.** Signal the compositor thread to stop accepting new frames after the current one completes. Wait up to 500ms for the compositor thread to finish its in-progress frame (including Stage 7 GPU Submit + Present) and return to its inter-frame idle state. This is a wait on the compositor thread's frame-completion signal, not a `MutationBatch` channel drain — the compositor thread must not begin a new frame after receiving the shutdown signal. GPU work in progress completes normally within Stage 7's 8ms budget; shutdown initiation and frame completion are therefore non-circular. (T-3)
3. **Revoke all leases.** Send revocation events to all connected agents. Do not wait for acknowledgement.
4. **Flush telemetry.** Flush the telemetry queue with up to 200ms grace.
5. **Terminate agent sessions.** Drop all gRPC and MCP connections.
6. **GPU drain.** Call `device.poll(wgpu::Maintain::Wait)` to ensure all GPU submissions from step 2 have completed. This is a safety step; the compositor thread's frame completion in step 2 already implies GPU work for the last frame is submitted. Step 6 ensures the device is fully idle before resource release.
7. **Release resources.** Drop GPU device, surface, and scene graph. Resource reference counts must reach zero cleanly.
8. **Exit process.** Exit code 0 for clean shutdown, non-zero for error.

Fatal GPU errors (device lost, out of memory) trigger an emergency path: flush telemetry, log structured error, enter safe mode (RFC 0007 §5.1, `CRITICAL_ERROR` reason) to inform the viewer before process exit, then trigger graceful shutdown with non-zero exit code. If the safe mode overlay cannot render (GPU already unusable), skip directly to graceful shutdown. See §7.3 and RFC 0009 §5 for the authoritative GPU failure response procedure. (T-7: aligns with RFC 0009 §5 resolution.)

---

## 2. Thread Model

### 2.1 Overview

The compositor uses a fixed, small set of threads with explicit responsibilities and typed channels between them. Thread count is determined at startup; no dynamic thread spawning during normal operation.

```
┌─────────────────────────────────────────────────────────────────────┐
│ THREAD MODEL                                                         │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Main Thread (winit event loop)                               │    │
│  │  • Owns: winit event loop, window handle, surface handle     │    │
│  │  • Runs: input drain, local feedback, frame presentation     │    │
│  │  • Receives: FrameReadySignal from compositor thread         │    │
│  │  • Sends: InputEvent → compositor thread                     │    │
│  └──────────────────────┬──────────────────────────────────────┘    │
│                         │                                            │
│          InputEvents ───┼──► MutationRequests                        │
│          FrameReady  ◄──┘    (bounded, backpressure)                │
│                         │                                            │
│  ┌──────────────────────▼──────────────────────────────────────┐    │
│  │ Compositor Thread                                             │    │
│  │  • Owns: scene graph, wgpu Device/Queue, render state        │    │
│  │  • Runs: mutation intake, scene commit, layout resolve,      │    │
│  │          render encode, GPU submit                           │    │
│  │  • Receives: MutationBatch from network thread               │    │
│  │  • Sends: FrameReadySignal to main thread                    │    │
│  │           TelemetryRecord to telemetry thread                │    │
│  └──────────────────────┬──────────────────────────────────────┘    │
│                         │                                            │
│     MutationBatch   ────┼──────────────────────┐                    │
│     (bounded)           │                       │                    │
│                         │                       ▼                    │
│  ┌──────────────────────┴──────────────────────────────────────┐    │
│  │ Network Thread(s) — Tokio Multi-Thread Runtime               │    │
│  │  • Owns: gRPC server, MCP bridge, agent session state        │    │
│  │  • Runs: auth, capability negotiation, stream multiplexing   │    │
│  │  • Receives: gRPC frames from agents                         │    │
│  │  • Sends: MutationBatch to compositor thread                 │    │
│  │           SceneEvent (three traffic-class lanes) to agents   │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Telemetry Thread                                              │    │
│  │  • Owns: telemetry sink (file, stdout, remote endpoint)      │    │
│  │  • Runs: async structured emission                           │    │
│  │  • Receives: TelemetryRecord from compositor thread          │    │
│  │  • Sends: nothing (fire and forget)                          │    │
│  └─────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

A media worker pool boundary is reserved for post-v1 GStreamer and WebRTC integration (§2.8). It is not spawned in v1. The channel interface and ownership rules are pre-defined so that adding media workers does not require restructuring the thread model.

### 2.2 Main Thread

The main thread runs the winit event loop — it cannot be moved to another thread because winit requires this on most platforms. Responsibilities:

- **Input drain.** Process OS input events (mouse, touch, keyboard) within the winit event callback. Immediately produce `InputEvent` records with hardware timestamps.
- **Local feedback.** Apply press/hover state changes to the scene's hit-region nodes for immediate visual response. This happens before any agent involvement.
- **Frame presentation.** Call `surface.present()` when signaled by the compositor thread via `FrameReadySignal` that a new frame is ready. This is the only thread that calls `present()`. See §2.7 for the ADR explaining why `present()` is pinned to the main thread.
- **Resize handling.** Resize events reconfigure the surface and notify the compositor thread to rebuild the render pipeline.
- **Shutdown initiation.** `CloseRequested` and OS signals initiate the shutdown sequence.

The main thread does **not** encode render commands or submit GPU work. It receives a `FrameReadySignal` from the compositor thread, then calls `present()`. The compositor thread owns the GPU queue.

**Thread priority.** The main thread is elevated at startup to reduce scheduling jitter on the input/presentation path, which is the most latency-sensitive path in the system (input_to_local_ack p99 < 4ms). Platform-specific mechanism:
- **Linux:** `pthread_setschedparam(SCHED_RR, priority=1)` for the main thread. Requires appropriate RLIMIT_RTPRIO or CAP_SYS_NICE. Falls back silently if the privilege is not available — log a warning but do not fail startup.
- **macOS:** `pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0)`.
- **Windows:** `SetThreadPriority(THREAD_PRIORITY_TIME_CRITICAL)` on the main thread handle.

The compositor thread is elevated to the same class. Network and telemetry threads run at normal priority.

### 2.3 Compositor Thread

A dedicated `std::thread` spawned at startup. Runs a tightly controlled loop:

- **Mutation intake.** Drain the `MutationBatch` channel. Each batch is validated and committed independently — batches are never coalesced.
- **Scene commit.** Apply validated mutation batches to the scene graph. Reject invalid mutations with structured errors.
- **Layout resolve.** Recompute tile bounds, z-order, and compositing regions. Only runs for tiles that changed.
- **Render encode.** Build wgpu render passes and encode command buffers.
- **GPU submit.** Submit command buffers to the wgpu queue. Signal the main thread to present when submission completes.
- **Telemetry emit.** Send per-frame `TelemetryRecord` to the telemetry thread.

The compositor thread owns the `wgpu::Device` and `wgpu::Queue`. No other thread touches the device. The main thread holds the surface handle and is the only thread that calls `surface.present()`. This split is an intentional architectural decision driven by platform constraints (macOS/Metal requires presentation on the main thread) and frame-budget separation (GPU submission must not block input drain). See §2.7 for the full ADR.

The compositor loop runs at the display refresh rate (default 60Hz). If a frame takes longer than the budget, the pipeline is marked as overbudget and the frame-time guardian evaluates degradation (§5.2).

### 2.4 Network Thread(s)

A Tokio multi-thread runtime (default: `tokio::runtime::Builder::new_multi_thread()` with thread count = number of logical CPUs, capped at 8). Responsibilities:

- **gRPC server.** tonic acceptor and per-agent session stream handlers.
- **MCP bridge.** JSON-RPC over stdio or Streamable HTTP.
- **Session management.** Auth handshake, capability negotiation, session lifecycle.
- **Mutation batching.** Collect individual RPC mutations into batches before forwarding to the compositor thread.
- **Event fan-out.** When the compositor commits a scene change, notify subscribed agent sessions via the appropriate traffic-class lane (transactional, state-stream, or ephemeral — see §2.6).

Network threads do **not** touch the scene graph or GPU state directly. They receive frames from agents, validate basic protocol structure, batch mutations, and forward them to the compositor thread. Scene validation (lease checks, budget enforcement, invariant verification) happens on the compositor thread, which is the sole owner of scene state.

### 2.5 Telemetry Thread

A single `std::thread` running an async executor (can share the Tokio runtime or be isolated — isolation is preferred for observability under load). Responsibilities:

- Receive `TelemetryRecord` from the compositor thread via a bounded channel (capacity: 256 records).
- Format as structured JSON.
- Write to configured sink: stdout, file, or remote endpoint.

The telemetry channel is **non-blocking** on the send side. If the channel is full (telemetry sink backpressure), the compositor thread drops the oldest unprocessed record and emits a `telemetry_overflow` counter. Telemetry must never block the frame pipeline.

### 2.6 Channel Topology

All inter-thread communication uses bounded channels. No unbounded queues.

| Channel | Type | Capacity | On Full |
|---------|------|----------|---------|
| `InputEvent` (main → compositor) | ring buffer (crossbeam or custom) | 256 | Oldest input dropped, logged |
| `SceneLocalPatch` (main → compositor) | ring buffer (custom) | 64 | Oldest dropped (latest hit-state wins) |
| `MutationBatch` (network → compositor) | `crossbeam::bounded` | 64 | Agent back-pressured (gRPC flow control) |
| `FrameReadySignal` (compositor → main) | `tokio::sync::watch` | N/A (latest value wins) | New value overwrites (latest frame wins) |
| `SceneEventTransactional` (compositor → network) | `crossbeam::bounded` | 256 | Compositor back-pressured (never dropped) |
| `SceneEventStateStream` (compositor → network) | bounded + coalesce map | 512 | Coalesce-key merging (intermediate states skipped, not dropped) |
| `SceneEventEphemeral` (compositor → network) | ring buffer (custom) | 256 | Oldest dropped, overflow counted |
| `DecodedFrameReady` (media pool → compositor) | ring buffer (custom) | 4 per stream | Oldest dropped (decoder runs ahead; compositor picks latest ready frame) |
| `TelemetryRecord` (compositor → telemetry) | ring buffer (custom) | 256 | Oldest dropped, overflow counted |

**`SceneLocalPatch` type.** A `SceneLocalPatch` carries only the hit-region local state changes produced by Stage 2 (pressed/hovered flags). It does not carry mutations that require lease validation or invariant checking. The compositor thread drains the `SceneLocalPatch` channel alongside `InputEvent` and `MutationBatch` at the start of Stage 3 and applies local state patches directly to the scene without a full commit cycle. The `ArcSwapFull<HitTestSnapshot>` is updated after applying local patches (T-11: resolves missing channel in topology table). (T-11)

```rust
pub struct SceneLocalPatch {
    pub changes: Vec<(SceneId, LocalStateFlags)>, // (hit_region_node_id, new_flags)
}

pub struct LocalStateFlags {
    pub pressed: bool,
    pub hovered: bool,
}
```

**`DecodedFrameReady` (post-v1, reserved).** This channel is not created in v1. Its entry in the topology table reserves the interface so that implementors building the v1 channel graph do not design around it. See §2.8 for the full ownership and threading rules.

**Traffic-class lane split (T-13).** The previous design placed all outbound `SceneEvent` notifications in a single `EventNotification` ring buffer with uniform drop-oldest semantics. This violated the session protocol's delivery guarantees (RFC 0005 §2.5, §5.1): transactional messages (lease revocations, `MutationResult` acks, `DegradationNotice`) are contractually never dropped, and state-stream messages must be coalesced rather than dropped. The three-lane split aligns the internal channel topology with the traffic-class delivery contracts:

- **`SceneEventTransactional`** — `crossbeam::bounded` with capacity 256. When full, the compositor thread blocks (backpressure). Transactional events are low-rate (lease grants, revocations, mutation acks, degradation notices) so blocking is acceptable and correct: it propagates HTTP/2 flow control to the sending agent rather than silently discarding a lease revocation.
- **`SceneEventStateStream`** — bounded channel (capacity 512) augmented with a per-session coalesce map keyed by `(tile_id, event_kind)`. When the channel is full, the runtime applies coalesce-key merging: a new state-stream event for the same tile and kind replaces the pending queued entry rather than being dropped or blocking. Intermediate states are skipped; the latest state always wins. This matches RFC 0005 §3.2 coalesce semantics.
- **`SceneEventEphemeral`** — ring buffer (capacity 256) with drop-oldest. Latest-wins by design. Overflow is counted and emitted in `TelemetryRecord`. This is the behavior previously applied uniformly to all events.

The compositor thread classifies each outbound `SceneEvent` by traffic class at emission time and enqueues it to the corresponding lane. The network thread drains all three lanes into the per-session gRPC send buffer, preserving per-lane ordering. Cross-lane ordering is best-effort and is not required by the protocol.

**Implementation note:** "Oldest dropped" semantics require a ring-buffer implementation, not a standard bounded channel. Standard `crossbeam::bounded` and `tokio::sync::mpsc` channels apply backpressure (blocking or error) when full — they do not drop the oldest entry. Channels that require drop-oldest behavior (`InputEvent`, `SceneLocalPatch`, `SceneEventEphemeral`, `TelemetryRecord`) must use a ring buffer (e.g., `crossbeam::ArrayQueue` with try_push + manual eviction, or a dedicated ring-buffer crate). `FrameReadySignal` is best served by `tokio::sync::watch`, which always delivers the latest value and naturally discards stale signals. `SceneEventTransactional` uses `crossbeam::bounded` directly (backpressure is intentional). `SceneEventStateStream` requires a custom structure: a bounded channel paired with a `HashMap<CoalesceKey, QueueSlot>` to enable in-place replacement of pending entries.

Backpressure on the `MutationBatch` channel propagates naturally to gRPC flow control: tonic's `AsyncRead`/`AsyncWrite` buffers fill up and the TCP window shrinks. Agents that send faster than the compositor can process will see their streams slow — this is correct behavior, not an error.

Backpressure on `SceneEventTransactional` is bounded by the rate of transactional events (lease operations, mutation acks). These are low-rate by design (at most one per agent mutation batch, capped by `max_update_rate_hz`). A full `SceneEventTransactional` channel indicates a severely stalled or unresponsive agent — acceptable to apply backpressure in that case.

### 2.7 ADR: Thread Ownership of surface.present() vs GPU Submit

**Decision:** `surface.present()` is called exclusively by the main thread. GPU command submission (`wgpu::Queue::submit`) is performed exclusively by the compositor thread. These two operations are assigned to different threads by design and must never be migrated.

#### Context and Constraints

wgpu's threading model imposes platform-specific constraints that directly determine which thread may call which GPU operations:

- **`wgpu::Queue::submit()`** is CPU-intensive command recording and is safe to call from any thread that owns the `wgpu::Queue`. There is no platform restriction. The compositor thread owns the `wgpu::Device` and `wgpu::Queue`; no other thread may call any method on these objects.
- **`wgpu::Surface::get_current_texture()`** (frame acquisition) and **`wgpu::SurfaceTexture::present()`** (frame presentation) have a platform-critical constraint: on macOS (Metal) and iOS (Metal), these calls **must** occur on the main thread. This is a Metal/Core Animation requirement propagated through wgpu: Metal's `CAMetalLayer` is tied to the run loop of the thread that created the window. Calling `present()` from a non-main thread on macOS results in undefined behavior, visual corruption, or a crash.
- **winit** requires the event loop to run on the main thread on all supported platforms. Since winit owns the window and surface handle, and since the main thread is the only legal thread for `present()` on macOS, the main thread is the only viable thread for presentation.

#### The Split and Why It Is Correct

The split — compositor thread submits GPU work; main thread presents — follows directly from the constraints above:

1. **GPU submission on compositor thread.** Command encoding and queue submission are the CPU-heavy, latency-sensitive work. They must not run on the main thread, which also handles input drain. Running GPU submission on the main thread would either block input processing (violating the `input_to_local_ack` p99 < 4ms budget) or require the input drain to compete with GPU submission for the same thread.

2. **`surface.present()` on main thread.** This is forced by the macOS/Metal requirement. The surface handle is held by the main thread; `present()` is a lightweight call (it signals the display server; it does not re-encode or re-submit GPU work). The cost is negligible.

3. **The `_guard` in `CompositorFrame`.** wgpu requires that the `SurfaceTexture` returned by `get_current_texture()` remains alive until after `present()`. The `CompositorFrame._guard: Box<dyn Any + Send>` carries this ownership across the thread boundary safely. The `Send` bound is required because `CompositorFrame` is transferred from the compositor thread (where it is created during render encode) to the main thread (where `present()` is called). The `_guard` is `Send` because `wgpu::SurfaceTexture` implements `Send`. (T-1: this resolves the soundness issue in the earlier `current_texture() -> wgpu::TextureView` signature.)

#### Safety Boundary: Which wgpu Calls Are Legal From Which Thread

| wgpu call | Thread | Rationale |
|-----------|--------|-----------|
| `Device::create_*` (buffers, textures, pipelines) | Compositor only | Device owned by compositor thread |
| `Queue::submit()` | Compositor only | Queue owned by compositor thread |
| `Queue::write_buffer()` | Compositor only | Queue owned by compositor thread |
| `Surface::get_current_texture()` | Main thread only | macOS/Metal main-thread requirement; surface held by main thread |
| `SurfaceTexture::present()` | Main thread only | macOS/Metal main-thread requirement; must follow get_current_texture() on same thread |
| `TextureView` creation from `SurfaceTexture` | Compositor thread | View is created from the guard before transfer; the guard (not the view) is what must be kept alive |
| `device.poll()` | Main thread (shutdown only) | Shutdown is coordinated on main thread after compositor thread has idled |

The `CompositorSurface` trait (§1.3) encodes this boundary structurally: `acquire_frame()` and `present()` are called in Stage 7 on the compositor thread for the `HeadlessSurface` case, and on the main thread for the `WindowSurface` case. Implementors of `CompositorSurface` must document which thread their implementation requires for each method. `WindowSurface` must document: acquire_frame() — main thread; present() — main thread.

**Correction to §1.3 trait semantics:** The `CompositorSurface` trait as sketched has `acquire_frame()` and `present()` called from the compositor thread in the pipeline description (Stage 7). For `WindowSurface`, both must instead be called from the main thread. The compositor thread produces a `CommandBuffer` and sends a `FrameReadySignal`; the main thread calls `acquire_frame()` followed by `present()` on the `WindowSurface`. This requires that the `CompositorFrame` is acquired and presented on the main thread. The `_guard` transfer in the `HeadlessSurface` case remains a no-op. See the synchronization protocol below for the handoff mechanism.

#### Synchronization Mechanism: FrameReadySignal

The handoff between compositor thread (GPU submit) and main thread (present) is `FrameReadySignal`, a `tokio::sync::watch` channel. The protocol is:

```
Compositor Thread                          Main Thread (winit event loop)
─────────────────                          ─────────────────────────────
Stage 6: encode CommandBuffer
Stage 7: Queue::submit(command_buffers)
         → GPU begins executing
         → send FrameReadySignal           ← watch::Receiver sees new value
                                           call surface.acquire_frame()
                                           call surface.present()
                                           → frame visible on screen
Stage 3 (next frame): mutation intake
  (runs concurrently with present above)
```

**Invariants enforced by this protocol:**

- **No present-before-submit:** The main thread only calls `present()` after receiving `FrameReadySignal`, which is sent only after `Queue::submit()` returns. `present()` before `submit()` is structurally impossible.
- **No double-present:** `tokio::sync::watch` delivers the latest value; a stale signal is never re-delivered. The main thread processes each signal value at most once per winit event loop tick. If the main thread is slower than 60fps, a frame signal is overwritten by the next one — the main thread skips to the newest frame rather than presenting stale frames twice.
- **No present-during-submit:** `Queue::submit()` completes before `FrameReadySignal` is sent. There is no window where present() could observe a partially submitted frame.

The `FrameReadySignal` channel (capacity: N/A, latest value wins — `tokio::sync::watch`) is designed so that if the main thread falls behind, the compositor thread is never blocked. The compositor thread sends the signal and immediately begins Stage 3 for the next frame. GPU execution for frame N is pipelined with CPU work for frame N+1.

#### Platform-Specific Presentation Notes

| Platform | Presentation thread | Notes |
|----------|--------------------|-|
| macOS (Metal) | Main thread only | Metal/Core Animation hard requirement. No workaround. |
| iOS (Metal) | Main thread only | Same as macOS. |
| Linux (Vulkan/X11, Vulkan/Wayland) | Any thread with surface ownership | The compositor thread could call `present()` directly. Current design uses main thread for uniformity. |
| Windows (Vulkan/D3D12) | Any thread with surface ownership | Same as Linux — no hard restriction. |

**Future optimization opportunity.** On Linux and Windows, `present()` can be moved to the compositor thread, eliminating the cross-thread signal for `FrameReadySignal` on those platforms. This would reduce one inter-thread coordination step and slightly tighten the `input_to_next_present` budget. This is deferred as a post-v1 platform-specific optimization. Any implementation must preserve the macOS/iOS constraint and must gate the optimization on platform detection at startup, not compile time.

### 2.8 Future: Media Worker Boundary

> **This section is a reservation, not an implementation spec.** Nothing in this section is built in v1. It documents where media workers will live in the thread model and what constraints must be preserved so that adding them post-v1 does not require restructuring the v1 design.

#### Motivation

Post-v1 integration of GStreamer media pipelines and WebRTC will require threads that are neither Tokio tasks nor `std::thread`s owned by the compositor. GStreamer has its own internal thread pool (managed by its scheduler and element graph). WebRTC ICE/DTLS threads are managed by the WebRTC library. These cannot be collapsed into the Tokio runtime — GStreamer's threading model is incompatible with cooperative async scheduling, and WebRTC libraries typically manage their own OS threads internally.

Without a pre-defined boundary, a future implementor faces three bad options: break the "fixed thread model" contract (§2.1), hack media decode onto the compositor thread (violates frame budget), or restructure the entire thread model in a disruptive RFC revision. This section defines the boundary now at low cost.

This is also required by architecture.md §Media: GStreamer ("media is not an add-on") and §Multiple video feeds are a compositor problem ("isolate decode, scene update, and presentation work").

#### The Boundary

**The media worker pool is managed entirely by GStreamer's internal scheduler.** From the compositor's perspective, the media pool is a black box that delivers decoded frames. The compositor does not spawn, manage, or join any GStreamer thread. It only interacts with GStreamer pipelines via the `DecodedFrameReady` channel (§2.6) and GStreamer's Rust pipeline API.

**Thread count:** Determined by GStreamer at pipeline construction time (based on element graph topology and system core count). Not under compositor control. Not observable as named threads in the compositor's thread table.

**Lifetime:** The media pool is created when the first media pipeline is started (post-v1) and torn down when the last pipeline is stopped. It does not exist at all in v1.

#### GPU Device Ownership Invariant

This is the critical constraint that the boundary must preserve:

**The compositor thread is the sole owner of the wgpu `Device` and `Queue`. No media worker thread may access the GPU device directly.**

The reason: wgpu (and the underlying Vulkan/D3D12/Metal backends) do not permit concurrent access to a `Device` from multiple threads without explicit synchronization. The compositor thread already owns the device. Allowing GStreamer threads to submit GPU work would require either a second device (expensive, no zero-copy) or explicit mutex-guarded device sharing (frame-pipeline budget risk, correctness hazard).

The consequence: decoded video frames must be uploaded to GPU textures by the compositor thread, not by media worker threads. The flow is:

```
GStreamer decode thread
  → decoded CPU buffer (or mapped DMA-BUF on Linux)
  → DecodedFrameReady { texture_data, presentation_ts, stream_id }
  → [ring buffer, capacity 4 per stream, drop-oldest]
Compositor thread (Stage 3 or dedicated sub-stage)
  → drain DecodedFrameReady signals
  → device.create_texture() + queue.write_texture() (CPU-side upload)
     OR: import DMA-BUF as wgpu texture (zero-copy, Linux/Vulkan only, post-v1 optimization)
  → GPU-resident texture handle stored in tile's media node
Stage 6: Render Encode
  → blit GPU texture into tile's compositing region
```

On Linux with Vulkan and DMA-BUF, a zero-copy path is possible (GStreamer produces a DMA-BUF handle; the compositor imports it as a wgpu external texture). This optimization is post-v1 and must not influence v1 channel design.

#### Channel Interface

The `DecodedFrameReady` channel (see §2.6 Channel Topology table) carries:

```rust
// Non-normative sketch. Authoritative definition deferred to the post-v1 Media RFC.
pub struct DecodedFrameReady {
    pub stream_id: SceneId,         // Which media stream / tile this frame belongs to
    pub presentation_ts: Duration,  // GStreamer running-time timestamp (media clock)
    pub data: MediaFrameData,       // CPU-side decoded data or OS-specific zero-copy handle
    pub sequence: u64,              // Monotonically increasing per stream; compositor skips gaps
}

pub enum MediaFrameData {
    CpuRgba { width: u32, height: u32, bytes: Vec<u8> },
    // Post-v1: DmaBuf { fd: RawFd, planes: Vec<DmaBufPlane> }, // Linux/Vulkan zero-copy
}
```

The channel is a ring buffer with capacity 4 per active stream (drop-oldest). The compositor reads the latest ready frame during Stage 3 (or a dedicated sub-stage inserted between Stage 3 and Stage 4). It uploads the texture to the GPU and updates the tile's media node with the new texture handle. The compositor may skip frames if the decoder runs ahead — the ring buffer's drop-oldest semantics ensure the compositor always sees the freshest decoded frame, never a stale one.

#### What Must NOT Change in v1 to Accommodate This

- The compositor thread's exclusive ownership of `wgpu::Device` and `wgpu::Queue` must not be relaxed.
- The `MutationBatch` channel and the compositor thread's main loop structure must not be changed to make room for media. Media input arrives on its own channel, drained in a distinct step.
- The Tokio runtime must not be used to schedule GStreamer pipeline state changes. GStreamer has its own pipeline state machine and must be driven from a dedicated control point (likely a thin wrapper on the compositor thread or a helper `std::thread`).
- The v1 channel topology table (§2.6) already includes the `DecodedFrameReady` row. Any v1 implementation that creates the channel infrastructure must leave the slot empty (channel not created) rather than removing the row — removing it would require a channel topology amendment at integration time.

---

## 3. Frame Pipeline

### 3.1 Pipeline Overview

Each frame passes through 8 stages in order. Stages 1–2 run on the main thread; stages 3–7 run on the compositor thread; stage 8 runs on the telemetry thread. The pipeline supports temporal overlap: GPU work for frame N executes concurrently with input drain for frame N+1.

```
FRAME PIPELINE (target: p99 total < 16.6ms at 60fps)

Main Thread ──────────────────────────────────────────────────►
  │
  │  ┌──────────────┐  ┌──────────────────┐
  │  │ 1. Input     │  │ 2. Local         │   Main thread
  │  │    Drain     │  │    Feedback      │   stages
  │  │  <500μs p99  │  │  <500μs p99      │
  │  └──────┬───────┘  └────────┬─────────┘
  │         │                   │
  │  InputEvents           SceneLocalPatch
  │         │                   │
  │         ▼                   ▼
Compositor Thread ────────────────────────────────────────────►
  │
  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
  │  │ 3. Mutation  │  │ 4. Scene     │  │ 5. Layout    │
  │  │    Intake    │  │    Commit    │  │    Resolve   │
  │  │  <1ms p99    │  │  <1ms p99    │  │  <1ms p99    │
  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
  │         │                 │                  │
  │         ▼                 ▼                  ▼
  │  ┌──────────────┐  ┌──────────────┐
  │  │ 6. Render    │  │ 7. GPU       │
  │  │    Encode    │  │    Submit +  │
  │  │  <4ms p99    │  │    Present   │
  │  └──────┬───────┘  │  <8ms p99    │
  │         │          └──────┬───────┘
  │         │                 │
  │         └────────┬────────┘
  │                  │
  │  ┌───────────────▼──────────────┐
  │  │ 8. Telemetry Emit            │   Telemetry thread
  │  │    <200μs p99, non-blocking  │
  │  └──────────────────────────────┘
  │
  │  ◄─── GPU frame N overlaps with input drain frame N+1 ──────►
```

### 3.2 Stage Specifications

#### Stage 1: Input Drain
**Thread:** Main | **Budget:** p99 < 500μs

Drain all pending OS input events from the winit event queue. For each event:
- Attach hardware timestamp (from OS event) and monotonic arrival timestamp.
- Produce `InputEvent { kind, position, timestamp_hw, timestamp_arrival, device_id }`.
- Enqueue to `InputEvent` channel (main → compositor). Non-blocking; drop oldest if full.

Input drain must never block on downstream processing. If the compositor is slow, inputs are queued or dropped — the main thread stays live.

#### Stage 2: Local Feedback
**Thread:** Main | **Budget:** p99 < 500μs

Process input events that have immediate visual response requirements:
- **Press/hover.** For each input event, hit-test against the current snapshot of active tile bounds. If a hit-region node is under the pointer, update its `pressed` or `hovered` local state flag.
- **Produce `SceneLocalPatch`.** A lightweight update containing only the changed local state flags. This is forwarded to the compositor thread but does not require a full mutation batch.

Local feedback is always within 1ms of input arrival (stages 1+2 combined). It does not wait for agent response, scene commit, or any network round-trip. This satisfies the `input_to_local_ack` p99 < 4ms budget with substantial headroom.

The hit-test used here uses the last committed tile bounds snapshot. The snapshot is stored as `Arc<HitTestSnapshot>` inside an `ArcSwapFull<HitTestSnapshot>` (from the `arc-swap` crate or equivalent). The main thread calls `arc_swap.load()` at the start of Stage 2, receives a guard holding the current `Arc<HitTestSnapshot>`, and uses it for the duration of Stage 2. No mutex is held. The compositor thread swaps in a new `Arc<HitTestSnapshot>` after Stage 4 commit using `arc_swap.store(Arc::new(new_snapshot))`. This is a pointer-width atomic write — the main thread either sees the old or new snapshot, never a torn intermediate state. The old snapshot is dropped when the last `Arc` reference falls away (T-2: eliminates data race between Stage 2 main-thread read and Stage 4 compositor-thread write).

#### Stage 3: Mutation Intake
**Thread:** Compositor | **Budget:** p99 < 1ms

Drain the `MutationBatch` channel. Apply agent envelope limits:
- Reject mutations that would exceed `max_nodes_per_tile` or `max_texture_bytes`.
- Queue valid batches for scene commit.

**Batches are never coalesced.** Each `MutationBatch` is the unit of atomicity: it carries a `batch_id` and receives an independent `MutationResult` acknowledgement. Merging two batches would collapse their `batch_id`s into one, breaking the per-batch deduplication and retransmission contract defined in RFC 0005 §5.2–5.3. The compositor may process multiple batches in a single frame tick — draining all available batches before advancing to Scene Commit — but each batch is validated, committed, and acknowledged independently.

**State-stream coalescing** (reducing update frequency under load) applies only to outbound `SceneEvent` notifications (RFC 0005 §3.2), not to inbound `MutationBatch` messages. See §6.2 (Degradation Ladder Level 1) for the coalescing policy.

#### Stage 4: Scene Commit
**Thread:** Compositor | **Budget:** p99 < 1ms

Apply validated mutation batches to the scene graph (RFC 0001 §4 — Mutation Pipeline). Scene commit is all-or-nothing per batch: either the entire batch applies or it is rejected with a structured error. Lease validation, budget checks, and invariant verification happen here.

After commit: publish the updated hit-test snapshot by constructing a new `Arc<HitTestSnapshot>` and calling `arc_swap.store(new_arc)`. The main thread picks up the new snapshot at the start of its next Stage 2 cycle. See Stage 2 above for the full synchronization protocol (T-2).

#### Stage 5: Layout Resolve
**Thread:** Compositor | **Budget:** p99 < 1ms

Recompute layout for tiles that changed this frame. Layout resolve is incremental — unchanged tiles skip this stage. For changed tiles:
- Validate bounds (tiles must not exceed tab bounds).
- Recompute z-order stack for the affected tab.
- Compute compositing regions (opaque tiles mask lower-z tiles in their region).

Output: `RenderFrame { dirty_tiles, composition_plan, viewport_size }`.

#### Stage 6: Render Encode
**Thread:** Compositor | **Budget:** p99 < 4ms

Build wgpu `CommandEncoder` from the `RenderFrame`. For each tile in the composition plan:
- Issue draw calls for the tile's nodes (solid color fill, text rasterization, image blit).
- Encode alpha-blend passes for transparent tiles.
- Encode chrome layer (tab bar, system indicators, disconnection badges).

Render encoding does not submit to the GPU queue. It only prepares `CommandBuffer` objects.

Media tiles (deferred to post-v1) will add video surface compositing here. The compositor thread will drain `DecodedFrameReady` signals during Stage 3 (Mutation Intake) or a dedicated sub-stage, upload decoded GPU textures (preserving GPU device ownership — see §2.8), and blit the resulting textures during Stage 6. No media decode work happens on the compositor thread.

#### Stage 7: GPU Submit + Present
**Thread:** Compositor | **Budget:** p99 < 8ms

Submit the encoded `CommandBuffer` to the wgpu queue. Signal the main thread via `FrameReadySignal`. The main thread calls `surface.present()`. In headless mode, the surface is a texture — `present()` is a no-op (pixel readback is on-demand via separate RPC).

This stage includes GPU execution time, which is not fully under software control. The 8ms budget accounts for GPU execution and presentation overhead. If this stage exceeds budget, the frame-time guardian (§5.2) activates.

**Pipeline overlap:** After GPU submission, the compositor thread immediately begins stage 3 for the next frame. GPU execution for frame N runs concurrently with mutation intake for frame N+1. The pipeline is effectively double-buffered on the CPU side.

#### Stage 8: Telemetry Emit
**Thread:** Telemetry | **Budget:** p99 < 200μs (non-blocking on compositor thread)

The compositor thread sends a `TelemetryRecord` to the telemetry thread. The send is non-blocking (the record is copied into the bounded channel and the compositor thread continues immediately). The telemetry thread formats and emits asynchronously.

`TelemetryRecord` contains: frame_number, stage_durations_us[8], tile_count, draw_call_count, mutation_count_this_frame, active_sessions, active_leases, texture_memory_bytes, degradation_level, shed_count, telemetry_overflow_count, timing_record (Option&lt;FrameTimingRecord&gt;). See Appendix A for the full Rust struct and RFC 0003 §FrameTimingRecord for the protobuf extension that embeds per-stage timestamps. (T-10: field list updated to match Appendix A struct.)

---

## 4. Admission Control

### 4.1 Connection Lifecycle

Agent connections proceed through a defined handshake before any scene access is granted:

```
Agent                                     Runtime
  │                                          │
  │─── TCP connect ──────────────────────────►│
  │◄── TLS/socket accept ────────────────────│
  │                                          │
  │─── AuthRequest { identity, token } ─────►│
  │                                          │   Auth validation
  │                                          │   (pluggable: PSK / mTLS / OIDC)
  │◄── AuthResponse { session_id, caps } ────│
  │                                          │
  │─── SessionOpen { protocol_version } ─────►│
  │                                          │   Capability negotiation
  │◄── SessionAck { negotiated_caps, limits} ─│   (version, budgets)
  │                                          │
  │    ← RESIDENT SESSION ESTABLISHED →      │
  │                                          │
  │─── MutationBatch ────────────────────────►│   Normal operation
  │◄── EventStream ──────────────────────────│
```

Total time from TCP connect to session established: **< 50ms** on loopback. This budget covers: TCP handshake, auth validation (PSK: < 1ms; OIDC: < 30ms external), capability negotiation, session stream setup.

### 4.2 Session Limits

Configurable limits with defaults:

| Parameter | Default | Max |
|-----------|---------|-----|
| Resident agent sessions | 16 | 256 |
| Guest agent sessions | 64 | 1024 |
| Total concurrent sessions | 80 | 1280 |
| Protocol negotiation timeout | 5s | Not configurable |
| Auth timeout | 10s | Not configurable |
| Heartbeat interval | 15s | Not configurable |
| Heartbeat timeout | 45s | Not configurable |

When the resident session limit is reached, new resident connection attempts receive a `RESOURCE_EXHAUSTED` gRPC error with a structured body indicating current capacity and an estimated wait hint. New guest connections always succeed (guest sessions are cheap).

### 4.3 Per-Agent Envelope

Each session is assigned resource limits at capability negotiation time. Defaults (overridable by config per namespace):

| Parameter | Default | Hard Max |
|-----------|---------|----------|
| `max_tiles` | 8 | 64 |
| `max_texture_bytes` | 256 MiB | 2 GiB |
| `max_update_rate_hz` | 30 | 120 |
| `max_nodes_per_tile` | 32 | 256 |
| `max_active_leases` | 8 | 64 |
| Session memory overhead | < 64 KB | — |

Session memory overhead (metadata, session state, event subscription buffers) must be < 64 KB per session, exclusive of content (textures, node data).

### 4.4 Hot-Connect

Agents connecting while a scene is active (tiles held by other agents, zones active) receive a full scene snapshot as part of `SessionAck`. The snapshot is the current committed state of the scene graph as a serialized `SceneSnapshot` (RFC 0001 §7). No frame is skipped; the incoming agent's snapshot delivery is handled on the network threads and does not block the compositor thread.

Hot-connect is non-disruptive: the new agent's session is established and it receives its snapshot while the compositor continues rendering frames for existing agents uninterrupted.

---

## 5. Budget Enforcement

### 5.1 Per-Agent Resource Tracking

The compositor thread maintains per-agent resource counters, updated each frame:

```rust
pub struct AgentResourceState {
    pub session_id: SceneId,
    pub namespace: String,

    // Per-frame tracking: sliding window for Hz limit enforcement (T-5).
    // See §5.3 for eviction and comparison logic.
    pub update_timestamps: VecDeque<Instant>,

    // Cumulative tracking
    pub texture_bytes_used: u64,
    pub node_count: u32,
    pub tile_count: u32,
    pub lease_count: u32,

    // Budget violation state
    pub budget_state: BudgetState,
    pub budget_state_entered: Option<Instant>,
}

pub enum BudgetState {
    Normal,
    Warning { first_exceeded: Instant },
    Throttled { throttled_since: Instant },
    Revoked { reason: RevocationReason },  // Terminal state: session teardown in progress
}

pub enum RevocationReason {
    BudgetThrottleSustained,    // Throttle sustained for 30s without recovery
    CriticalLimitExceeded,      // OOM attempt, texture hard-max exceeded
    RepeatedInvariantViolation, // > 10 invariant violations in session
    ProtocolViolation,          // Forged session IDs or malicious protocol abuse
}
```

### 5.2 Budget Tiers and Frame-Time Guardian

**Per-agent budget enforcement** operates on a three-tier ladder:

| Tier | Trigger | Duration | Action |
|------|---------|----------|--------|
| Warning | Any limit exceeded | — | Send `BudgetWarning` event to agent |
| Throttle | Warning unresolved for 5s | Until resolved | Coalesce outbound `SceneEvent` notifications more aggressively; reduce effective `max_update_rate_hz` by 50% |
| Revocation | Throttle sustained for 30s, or critical limit (e.g., OOM attempt) | Immediate | Revoke all leases; terminate session |

Critical triggers bypass the warning/throttle ladder and go directly to revocation:
- Attempt to allocate texture memory that would exceed the hard max.
- Repeated invariant violations (> 10 in a session).
- Protocol violations that indicate malicious intent (e.g., forged session IDs).

**Resource cleanup on revocation.** When a session is revoked (budget tier or critical trigger), the compositor thread executes the following on the same frame tick:
1. Move agent's `BudgetState` to `Revoked`.
2. Enqueue a `LeaseRevocationEvent` for all of the agent's active leases.
3. Mark all agent-owned tiles as orphaned (rendered frozen at last state, disconnection badge applied).
4. Unlike unexpected disconnects, which trigger a reconnection grace period for session resumption (see RFC 0005 §1.4), policy-driven revocations do not grant a grace period. Leases are marked for immediate reclamation; the session resumption window in RFC 0005 §4.2 is bypassed.
5. After a configurable post-revocation delay (default: 100ms, to allow `LeaseRevocationEvent` fan-out), free all agent-owned textures and node data. Reference counts drop to zero; resources are released.
6. Remove `AgentResourceState` from the compositor's per-agent table.

The post-revocation resource footprint for a revoked agent must be zero (per architecture.md §Resource lifecycle). This is verified by the `disconnect_reclaim_multiagent` test scene.

**Frame-time guardian** operates at the frame level, not the per-agent level. If the compositor thread detects that the current frame is on track to exceed 16.6ms:

1. **Check at stage 5 (Layout Resolve).** If cumulative time for stages 3–5 exceeds 3ms, shed work.
2. **Shed lowest-priority tiles.** Sort tiles by priority using `(lease_priority ASC, z_order DESC)` — lower `lease_priority` values (0 = highest priority) are preserved first; within the same priority class, higher `z_order` wins. Tiles with the highest `lease_priority` values and lowest `z_order` are shed first. Skip render encoding for the lowest-priority tiles until the workload fits within budget. (T-8: aligns sort direction phrasing with RFC 0008 §2.2 canonical formulation.)
3. **Emit shed event.** `TelemetryRecord.shed_count` incremented. If shedding occurs for > 3 consecutive frames, trigger degradation policy evaluation (§6).

### 5.3 Budget Accounting Accuracy

Per-frame resource accounting uses integer arithmetic to avoid floating-point non-determinism. Texture memory is tracked in bytes. Update rates are tracked as a sliding window of event arrival timestamps over the last 1 second. Specifically: each agent's `AgentResourceState` carries a `VecDeque<Instant>` of mutation batch arrival timestamps. On each mutation intake (Stage 3), timestamps older than `now - 1s` are evicted from the front of the deque. After eviction, `deque.len()` is compared against the agent's `max_update_rate_hz` limit. A mutation batch is rejected if appending it would push `len` above the limit. This is a sliding window that allows short bursts up to the limit within any 1-second window (T-5: makes enforcement semantics unambiguous). A token-bucket alternative is a post-v1 consideration if burst tolerance proves problematic in practice.

Budget checks happen in stage 3 (Mutation Intake) before the scene is modified. A mutation batch that would push the agent over budget is rejected in whole with a structured error. Partial acceptance within a batch is not supported — all-or-nothing is simpler to reason about and prevents partial state.

---

## 6. Degradation Policy

### 6.1 Trigger Condition

The degradation policy evaluates after every frame. Trigger: `frame_time_p95 > 14ms` measured over a rolling 10-frame window.

The 10-frame window (166ms at 60fps) gives the system time to absorb transient spikes (a single expensive frame during a large scene change) without triggering degradation for a momentary blip.

### 6.2 Degradation Ladder

```
DEGRADATION LADDER

Normal ──────────────────────────────────────────────────────────────
  │  frame_time_p95 > 14ms over 10 frames
  ▼
Level 1: Coalesce
  • Reduce outbound SceneEvent notification frequency for state-stream tiles
  • Coalesce ratio: 2× (30Hz → 15Hz effective update rate for state-stream tiles)
  • Inbound MutationBatch messages are never coalesced (each retains its batch_id
    and independent MutationResult); "coalesce" here applies only to outbound
    SceneEvent fan-out to subscribers
  │  frame_time_p95 > 14ms over 10 frames (still)
  ▼
Level 2: Reduce Texture Quality
  • Scale down texture resolution for large image tiles (> 512×512)
  • Target: 50% linear dimensions (25% pixel area)
  • Video tiles: reduce to 15fps decode rate (deferred to post-v1)
  │  frame_time_p95 > 14ms over 10 frames (still)
  ▼
Level 3: Disable Transparency
  • Force all semi-transparent tiles to fully opaque
  • Skip alpha-blend passes in render encoder
  • Significant GPU savings for scenes with many overlapping transparent tiles
  │  frame_time_p95 > 14ms over 10 frames (still)
  ▼
Level 4: Shed Tiles
  • Sort active tiles by (lease_priority ASC, z_order DESC) — see RFC 0008 §2.2 for canonical sort semantics
  • Remove lowest-priority tiles from render pass
  • Remove one tier of tiles (approximately 25% of active tiles) per level
  • Removed tiles remain in scene graph — they are present but not rendered
  │  frame_time_p95 > 14ms over 10 frames (still)
  ▼
Level 5: Emergency
  • Render only: chrome layer + highest-priority single tile
  • All other agent tiles visually suppressed (rendering-only suppression — leases remain ACTIVE, NOT in SUSPENDED state; see RFC 0008 §3.3)
  • Human override controls always visible
  │  frame_time_p95 returns to < 12ms over 30 frames → recover one step
  ▲
Recovery (hysteresis) ───────────────────────────────────────────────
```

**V1 scope note.** The doctrine degradation ladder (failure.md) defines six ordered axes: coalesce, reduce media quality, reduce concurrent streams, simplify rendering, shed tiles, and audio-first fallback. This RFC's five-level ladder maps to the doctrine as follows:

| Doctrine axis | V1 Level | Notes |
|---|---|---|
| Coalesce | Level 1 | Implemented — outbound SceneEvent fan-out only; inbound MutationBatch never coalesced |
| Reduce media quality | Level 2 | Texture resolution only; video decode deferred (no media in v1) |
| Reduce concurrent streams | — | Deferred to post-v1; no media streams in v1 |
| Simplify rendering | Level 3 | Disable transparency blending |
| Shed tiles | Level 4 | Priority-ordered tile removal |
| Audio-first fallback | — | Deferred to post-v1; no audio in v1 |
| Emergency: chrome + one tile | Level 5 | Extends doctrine with an explicit last resort |

Post-v1 RFC revisions must re-insert "reduce concurrent streams" (between Levels 2 and 3) and "audio-first fallback" (after Level 4) when GStreamer/WebRTC are integrated.

### 6.3 Hysteresis

Recovery requires `frame_time_p95 < 12ms` sustained over a 30-frame window (500ms at 60fps). This prevents oscillation between levels. Recovery moves up one level at a time; reaching Normal from Level 5 requires 5 × 30 frames of clean performance.

The 12ms recovery threshold vs 14ms trigger threshold is a 2ms hysteresis band. This absorbs measurement noise and prevents flickering between states at the boundary.

### 6.4 Degradation Observability

Each degradation level change is emitted as a telemetry event:

```json
{
  "event": "degradation_level_change",
  "from_level": 0,
  "to_level": 1,
  "trigger_p95_ms": 14.7,
  "window_frames": 10,
  "timestamp_ms": 1234567890
}
```

The current degradation level is always visible in the `TelemetryRecord`. Agents can subscribe to `DegradationEvent` notifications to reduce their own update rate proactively.

---

## 7. Window Surface Management

### 7.1 Window Modes

The compositor supports two modes, configured at startup. The same binary supports both.

**Fullscreen Mode:**
- Compositor owns the entire display.
- Background layer: opaque (solid color or ambient fill).
- All input captured — no passthrough.
- Supported: all platforms.

**Overlay/HUD Mode:**
- Transparent borderless always-on-top window over the user's desktop.
- Background layer: fully transparent.
- Input routing: per-region. Tiles with active leases and input affordances capture input. All other regions pass input through to the underlying desktop.
- Supported: Windows (Win32), macOS, X11, wlroots Wayland (Sway, Hyprland). Falls back to fullscreen on GNOME/KDE Wayland and unsupported compositors.

The mode is determined at startup from config or command-line. **Runtime mode switching** (fullscreen ↔ overlay without restart) is supported but is a disruptive operation: the surface must be recreated, the render pipeline rebuilt, and a brief blank frame is unavoidable. Mode switches are expected to be rare (user configuration, not agent control).

### 7.2 Click-Through Implementation

Overlay mode requires per-region input passthrough. The implementation is platform-specific:

**Windows (Win32):**
```
WS_EX_LAYERED | WS_EX_TRANSPARENT on the window.
Override WM_NCHITTEST:
  - For points within any active hit-region: return HTCLIENT
  - For all other points: return HTTRANSPARENT
```
The compositor maintains an `InputRegionMask` — a set of `Rect` values corresponding to active hit-regions in the current committed scene. This mask is stored as `Arc<InputRegionMask>` inside an `ArcSwapFull<InputRegionMask>` (the same pattern as `HitTestSnapshot` in §3.2 Stage 2). After each Stage 4 scene commit, the compositor thread constructs a new `Arc<InputRegionMask>` and stores it via `arc_swap.store()`. The WM_NCHITTEST handler on the main thread calls `arc_swap.load()` to read the current mask with no lock contention (T-2: eliminates the data race between the WM_NCHITTEST callback and Stage 4 commit).

**macOS:**
```
NSWindow.ignoresMouseEvents = false (window-level passthrough off)
Override NSView.hitTest(_:):
  - Points within active hit-regions: return self (capture)
  - Other points: return nil (pass through)
```

**Linux X11:**
```
XShapeSelectInput to configure the input shape mask.
After each scene commit: call XShapeCombineRectangles with the current hit-regions.
```

**Linux Wayland (wlroots):**
```
zwlr_layer_shell_v1 with LAYER_TOP and KEYBOARD_INTERACTIVITY_NONE by default.
Set input region via wl_surface.set_input_region.
After each scene commit: update input region to match active hit-regions.
```

On platforms that do not support per-region passthrough (GNOME Wayland, KDE Wayland without layer-shell), the overlay mode falls back to fullscreen silently with a startup warning logged.

### 7.3 Surface Lifecycle

The window surface is owned by the main thread. The compositor thread owns the `wgpu::Device` and `wgpu::Queue`. For the rationale behind this split, see §2.7 (Thread-Ownership ADR). Surface resize:

1. Main thread receives `Resized(width, height)` from winit.
2. Main thread sends `SurfaceResized(width, height)` to compositor thread via a dedicated channel (capacity: 1, overwrite).
3. Compositor thread, between frames, calls `surface.configure(device, &config)` with new dimensions.
4. Next frame render is at the new resolution.

Surface recreation (mode switch, GPU device lost recovery):
1. Notify compositor thread to drain and idle.
2. Destroy old surface.
3. Recreate window/texture with new configuration.
4. Rebuild render pipeline.
5. Signal compositor thread to resume.

GPU device lost (rare, but must be handled):
1. Compositor thread detects `SurfaceError::Lost` or `SurfaceError::Outdated`.
2. Flush telemetry with error event.
3. Attempt surface reconfiguration. If successful, resume normally.
4. If reconfiguration fails (device truly lost): enter safe mode (RFC 0007 §5.1, `CRITICAL_ERROR` reason) to inform the viewer before process exit. If safe mode overlay renders within 2 seconds, display it briefly; then trigger graceful shutdown with non-zero exit code. If the overlay cannot render (GPU already unusable), skip directly to graceful shutdown. (T-7: required by RFC 0009 §5.3.)

---

## 8. Headless Mode

### 8.1 Surface

In headless mode, the compositor creates a `wgpu::Texture` with `RENDER_ATTACHMENT | COPY_SRC` usage instead of a window-backed surface. The texture dimensions are set from config (`headless_width`, `headless_height`, default: 1920×1080).

```rust
let headless_texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("headless_render_target"),
    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::Rgba8UnormSrgb,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    view_formats: &[],
});
```

`HeadlessSurface::present()` is a no-op. The texture accumulates the last rendered frame. Pixel readback is performed on demand.

### 8.2 Pixel Readback

Tests and CI scripts retrieve frame contents via:

1. **gRPC RPC:** `ReadbackFrame` RPC (control plane). Returns a `FramePixels { width, height, format, data: bytes }` containing a CPU-side copy of the last rendered frame.
2. **Direct in-process:** For Layer 1 tests that run the compositor in-process, `HeadlessSurface::readback()` returns a `Vec<u8>` synchronously.

Readback triggers a `wgpu::Buffer` (COPY_DST | MAP_READ) copy:

```
headless_texture → copy_texture_to_buffer → map → Vec<u8>
```

This is not on the hot path. It is an explicit, test-time operation. Readback latency is not part of the frame pipeline budget.

### 8.3 Software GPU Backends

| Platform | Headless Backend |
|----------|-----------------|
| Linux | mesa llvmpipe (Vulkan software rasterizer) |
| Windows | WARP (Windows Advanced Rasterization Platform) |
| macOS | Metal (hardware-backed; no proven software-only path) |

The wgpu adapter selection in headless mode explicitly requests a software fallback if no hardware GPU is found:

```rust
let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
    power_preference: wgpu::PowerPreference::None,
    compatible_surface: None,
    force_fallback_adapter: std::env::var("HEADLESS_FORCE_SOFTWARE").is_ok(),
}).await.expect("no wgpu adapter found");
```

In CI, `HEADLESS_FORCE_SOFTWARE=1` is set to ensure tests use llvmpipe/WARP regardless of GPU availability.

### 8.4 Headless Timing

Without a display refresh signal (vsync), the headless compositor runs at a configurable frame rate (default: 60fps target, driven by a `tokio::time::interval`). Frame timing is less precise than vsync-driven rendering, but is sufficient for correctness testing and performance benchmarking (with appropriate normalization per validation.md).

### 8.5 Test Assertions

Layer 1 tests (headless render + pixel readback) use:

```rust
// Assert a rectangular region matches expected RGBA color within tolerance
fn assert_region_color(
    pixels: &[u8], width: u32,
    region: Rect, expected: Rgba, tolerance: u8
) { ... }

// Assert that a pixel falls within an expected alpha range
fn assert_alpha_range(pixels: &[u8], width: u32, x: u32, y: u32, min: u8, max: u8) { ... }
```

Software GPU tolerance: ±2 per channel for alpha-blended regions, ±1 for solid color fills (per validation.md §Layer 1).

---

## 9. Quantitative Requirements

All budgets are p99 unless otherwise noted. "Normalized" means hardware-normalized per validation.md §Hardware-normalized performance.

| Metric | Budget | Measurement Point |
|--------|--------|-------------------|
| Frame pipeline total | p99 < 16.6ms (normalized) | Stage 1 start → Stage 7 end |
| Input drain (Stage 1) | p99 < 500μs | Stage 1 |
| Local feedback (Stage 2) | p99 < 500μs | Stage 2 |
| Stages 1+2 combined | p99 < 1ms | Stages 1–2 |
| Mutation intake (Stage 3) | p99 < 1ms | Stage 3 |
| Scene commit (Stage 4) | p99 < 1ms | Stage 4 |
| Layout resolve (Stage 5) | p99 < 1ms | Stage 5 |
| Render encode (Stage 6) | p99 < 4ms | Stage 6 |
| GPU submit + present (Stage 7) | p99 < 8ms | Stage 7 |
| Telemetry emit (Stage 8) | p99 < 200μs (non-blocking) | Stage 8 |
| input_to_local_ack | p99 < 4ms | Stage 1 start → Stage 2 end |
| input_to_scene_commit | p99 < 50ms (local agents) | Input event arrival → agent response applied to scene (network round-trip included; see RFC 0004 §latency budgets) |
| input_to_next_present | p99 < 33ms | Stage 1 start → main thread `surface.present()` returns (after Stage 7 signals FrameReadySignal) |
| Mutation to next present | p99 < 33ms | MutationBatch enqueue → Stage 7 end |
| Agent connection (TCP → session) | < 50ms | Auth start → SessionAck |
| Degradation response | Within 1 frame | Trigger detected → Level 1 active |
| Session memory overhead | < 64 KB per agent (excl. content) | Measured at steady state |
| Telemetry emit | < 200μs per frame | Compositor thread send only |

---

## 10. Open Questions

1. **Compositor thread affinity.** Should the compositor thread be pinned to a specific CPU core? Pinning improves cache behavior and reduces scheduling jitter. It also makes the system less portable. Decision deferred to benchmarking.

2. **Render encoder parallelism.** Stage 6 (Render Encode) is currently single-threaded on the compositor thread. For scenes with many tiles, parallel encoder creation with multiple `CommandEncoder` instances (recorded in parallel, submitted in order) could reduce Stage 6 time. The tradeoff: complexity vs. budget headroom. Deferred to profiling data.

3. **Frame pacing on headless.** `tokio::time::interval` has jitter under load. An alternative is a spin-wait with yield for tighter frame pacing in headless benchmarks. Decision deferred to validation results.

4. **Telemetry sink protocol.** File, stdout, or remote endpoint are all specified. For remote telemetry (production deployment), a simple UDP or TCP line-protocol sink is likely sufficient. The exact wire format for remote emission is deferred to the Telemetry RFC.

5. **Session snapshot for large scenes.** Hot-connect delivers a full `SceneSnapshot` as defined in RFC 0001 §7. For very large scenes, this could be a significant payload. Incremental snapshot (diff from empty, rather than full state) is deferred to post-v1 per v1.md §Advanced protocol ("No resumable state sync"). The `SceneSnapshot` format is already specified in RFC 0001; the open question is only whether v1 needs a size budget or chunk-based delivery for pathological large scenes.

---

## 11. References

- RFC 0001: Scene Contract — scene graph types, mutation pipeline, identity model
- heart-and-soul/architecture.md — screen sovereignty, compositing model, session model
- heart-and-soul/security.md — resource governance, per-session limits, trust gradient
- heart-and-soul/failure.md — degradation axes, core principle, agent failure modes
- heart-and-soul/validation.md — performance budgets, DR-V2, DR-V3, hardware normalization
- heart-and-soul/v1.md — compositor scope, window modes, v1 boundary

---

## Appendix A: Key Rust Types (Sketch)

These are non-normative sketches to orient implementors. The final API is defined by the implementation.

```rust
// Entry point
pub struct CompositorConfig {
    pub headless: bool,
    pub headless_width: u32,
    pub headless_height: u32,
    pub window_mode: WindowMode,
    pub max_resident_sessions: u32,
    pub max_guest_sessions: u32,
    pub grpc_bind: SocketAddr,
    pub mcp_bind: Option<SocketAddr>,
    pub telemetry_sink: TelemetrySink,
}

pub enum WindowMode { Fullscreen, Overlay }

// Channel message types
pub struct MutationBatch {
    pub session_id: SceneId,
    pub namespace: String,
    pub mutations: Vec<SceneMutation>,  // from RFC 0001
    pub sequence: u64,
}

pub struct FrameReadySignal {
    pub frame_number: u64,
    pub render_complete_ts: Instant,
}

// TelemetryRecord — internal Rust type sent from compositor thread to telemetry thread.
// The wire-level protobuf extension that embeds per-stage timestamps is FrameTimingRecord,
// defined in RFC 0003 §FrameTimingRecord. RFC 0002 will adopt RFC 0003's named-field approach
// when the protobuf schema is finalized; the timing_record field below carries those per-stage
// timestamps on the wire. (T-10: struct was previously missing draw_call_count,
// mutation_count_this_frame, and timing_record fields that §3.2 Stage 8 prose referenced.)
pub struct TelemetryRecord {
    pub frame_number: u64,
    pub stage_durations_us: [u64; 8],  // Indexed 0–7 corresponding to pipeline stages 1–8
    pub tile_count: u32,
    pub draw_call_count: u32,          // T-10: draw calls issued in Stage 6 this frame
    pub mutation_count_this_frame: u32, // T-10: MutationBatch operations applied in Stage 4 this frame
    pub active_sessions: u32,
    pub active_leases: u32,
    pub texture_memory_bytes: u64,
    pub degradation_level: DegradationLevel,  // T-4: typed enum, not u8; maps to uint32 in protobuf wire format
    pub shed_count: u32,
    pub telemetry_overflow_count: u64,
    // Wire-level per-stage timestamp record (RFC 0003 §FrameTimingRecord).
    // None until RFC 0003 schema is finalized; populated once the protobuf is available.
    pub timing_record: Option<FrameTimingRecord>,  // T-10
}

// Degradation
pub enum DegradationLevel {
    Normal = 0,
    Coalesce = 1,
    ReduceTextureQuality = 2,
    DisableTransparency = 3,
    ShedTiles = 4,
    Emergency = 5,
}

// Hit-test snapshot (read by main thread, written by compositor thread).
// Synchronization: stored as ArcSwapFull<HitTestSnapshot> — see §3.2 Stage 2.
pub struct HitTestSnapshot {
    pub regions: Vec<(SceneId, Rect, InputMode)>, // (tile_id, bounds, mode)
}
// ArcSwapFull<HitTestSnapshot> usage:
//   Compositor thread writes: arc_swap.store(Arc::new(new_snapshot))
//   Main thread reads:        let guard = arc_swap.load(); // pointer-width atomic, no lock

// Note on stage_durations_us (T-6): the array-indexed form above is an internal Rust type
// used on the compositor thread. The wire-level representation is FrameTimingRecord as
// defined in RFC 0003 §FrameTimingRecord, which uses named per-stage fields. RFC 0002
// will adopt RFC 0003's named-field approach when the protobuf schema is finalized.
// See RFC 0003 round 1 review §Cross-RFC Consistency.

// FrameTimingRecord — opaque placeholder; the authoritative definition is in RFC 0003 §FrameTimingRecord.
// Used here only as the type of TelemetryRecord::timing_record. (T-10)
pub struct FrameTimingRecord {
    // Defined in RFC 0003. Named per-stage timestamp fields, one per pipeline stage.
    // This struct is populated and serialized to protobuf by the telemetry thread.
}
```

---

## 12. Review Record

| Round | Date | Reviewer | Focus | Changes |
|-------|------|----------|-------|---------|
| A1 | 2026-04-19 | hud-ora8.1.9 | Amendment: media worker lifecycle | Converted RFC 0002 §2.8 ("Future: Media Worker Boundary") from reservation to normative lifecycle spec. Added worker state machine (SPAWNING → RUNNING → DRAINING → TERMINATED; FAILED terminal state). Defined three-condition activation gate: capability grant (RFC 0008 A1 `media-ingress`), budget headroom check (pool slot, per-session stream cap, global texture headroom), and role-authority re-check (RFC 0009 A1: owner or admin). Specified shared worker pool: N = 2–4 slots, priority-based preemption (lease_priority sort per RFC 0008 §2.2), budget-pressure contraction to 1 slot at degradation Level 2+. Defined degradation trigger authority: runtime-automatic (ladder advance), watchdog-automatic (per-worker threshold), operator-manual (Level 0 override); agents may only self-close, not demand degradation. Specified watchdog targets: CPU time (200ms/10s), GPU texture occupancy (256 MiB), ring-buffer occupancy (75%/30-frame sustained), decoder lifetime (24h), leases held (per §4.3 envelope). Documented in-process tokio task model (E24 COMPATIBLE verdict, `docs/decisions/e24-in-process-worker-posture.md`): session coordinator + watchdog tasks on network tokio runtime; GStreamer pipeline pool as black box; GPU device ownership invariant unchanged from §2.8; cross-agent isolation via session_id tagging on DecodedFrameReady. Added RFC 0014 forward cross-references. Full amendment document: `about/legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md` (issue hud-ora8.1.9). |
