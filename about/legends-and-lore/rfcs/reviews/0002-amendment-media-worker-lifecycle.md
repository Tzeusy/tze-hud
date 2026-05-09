# RFC 0002: Runtime Kernel — Amendment: Media Worker Lifecycle

**Amendment ID:** 0002-amendment-media-worker-lifecycle
**Issue:** hud-ora8.1.9
**Date:** 2026-04-19
**Author:** hud-ora8.1.9 (parallel-agents worker)
**Predecessor:** hud-ora8.1.1 — E24 security-posture verdict: COMPATIBLE
**Parent task:** hud-ora8.1 (v2 embodied media presence program preparation)
**Depends on:**
- RFC 0002 §1.1 (Single-Process Model) — in-process compositor contract
- RFC 0002 §2.8 (Media Worker Boundary) — pre-declared in-process worker reservation
- RFC 0008 Amendment A1 (C13 Capability Dialog) — activation gate: `media-ingress` capability grant required before any worker spawns
- RFC 0009 Amendment A1 (C12 Role-Based Operators) — role authority governing who may grant the activation capability
- E24 COMPATIBLE verdict: `docs/decisions/e24-in-process-worker-posture.md`
**Forward reference:** RFC 0014 (Media Plane — mechanism layer) will own the authoritative GStreamer pipeline and WebRTC transport details. This amendment establishes the lifecycle contract that RFC 0014 must implement.

---

## Scope and Purpose

RFC 0002 §2.8 ("Future: Media Worker Boundary") was authored in v1 as a
reservation for in-process GStreamer and WebRTC threads. It established the
channel interface (`DecodedFrameReady`), the GPU-ownership invariant (media
workers never touch the wgpu device), and the boundary between
compositor-managed tokio tasks and GStreamer's internal thread pool.

This amendment converts that reservation into a normative lifecycle
specification. It documents:

1. The **media worker state machine**: states, transitions, and terminal
   conditions.
2. The **activation gate**: the ordered set of preconditions that must all
   hold before a worker is spawned.
3. **Shared worker pool semantics**: pool size, priority-based preemption,
   and budget-pressure behavior.
4. **Decode/render degradation trigger authority**: who may demand a
   degradation step and under what conditions.
5. The **in-process tokio task model** ratified by the E24 COMPATIBLE
   verdict (E24 = "in-process tokio tasks, not subprocess isolation, with
   aggressive budget limits + watchdog").
6. **Watchdog targets**: the per-worker resources the watchdog observes.

This amendment is **documentary only** for phase 1. No Rust struct
definitions or protobuf schema changes are introduced here. Implementation
details — GStreamer pipeline wiring, `DecodedFrameReady` channel
instantiation, watchdog task structure — are owned by downstream tasks
under hud-ora8.1 and the forthcoming RFC 0014.

---

## Background: E24 COMPATIBLE Verdict

The E24 verdict (`docs/decisions/e24-in-process-worker-posture.md`) resolves
open item 3 from the v2-embodied-media-presence signoff packet. The verdict
is **COMPATIBLE**: tze_hud's agent-isolation posture admits in-process tokio
media workers without a subprocess-isolation pivot.

Four load-bearing reasons justify the verdict:

1. **Agents already live outside the compositor's address space.** RFC 0002
   §1.1 establishes the compositor as a single trusted OS process. Agents
   reach it across gRPC and MCP wires; they never load code into it. E24's
   media workers are compositor-internal threads, not agent code. No
   agent-supplied bytecode or shared memory region is involved.

2. **Agent isolation invariants are agent-to-agent, not agent-to-runtime.**
   The four invariants in `security.md` §"Agent isolation" (no cross-agent
   tile read, no cross-agent input intercept, no cross-agent media stream
   access, no cross-agent lease modification) are mediated by the runtime
   between agents. They require correct stream tagging and access denial, not
   per-agent process separation.

3. **RFC 0002 §2.8 already pre-declared in-process media workers.** The
   reservation explicitly states that GStreamer threads are "neither Tokio
   tasks nor `std::thread`s owned by the compositor" and that "no media
   worker thread may access the GPU device directly." This amendment does not
   introduce subprocess isolation because RFC 0002 §2.8 never reserved it.

4. **Budget limits + watchdog are the isolation enforcement.** E24's watchdog
   governs in-process resources (GPU textures, ring-buffer occupancy, CPU
   time, decoder lifetimes) that cannot be governed from across a subprocess
   boundary without a second wgpu device — which RFC 0002 §2.8 explicitly
   rejects as too expensive and incompatible with zero-copy decode paths.

This amendment's worker-pool design is the direct implementation expression
of the E24 COMPATIBLE verdict.

---

## Amendment Content

### A1. Media Worker State Machine

A media worker is a compositor-owned coordination unit consisting of:

- A tokio task (the **session coordinator**) that manages a GStreamer
  pipeline on behalf of one agent session and one active `media-ingress`
  stream.
- The underlying GStreamer pipeline thread pool managed by GStreamer's
  internal scheduler (not compositor-controlled; treated as a black box).
- Budget watchdog subscription (shared watchdog task; see §A4).

The state machine for a single media worker:

```
         ┌────────────────────────────────────────────────────┐
         │  PRECONDITION CHECK (not a state; gating logic)     │
         │                                                      │
         │  1. capability grant: media-ingress (RFC 0008 A1)   │
         │  2. budget headroom check (§A2.3)                   │
         │  3. role authority (RFC 0009 A1): owner or admin    │
         │     authorized the capability grant                  │
         │                                                      │
         │  All three must hold → proceed to SPAWNING          │
         │  Any fails → reject with structured error; no state  │
         └────────────────────────────────────────────────────┘
                          │ all pass
                          ▼
             ┌────────────────────────┐
             │       SPAWNING         │
             │                        │
             │  tokio task started    │
             │  GStreamer pipeline     │
             │  being constructed      │
             └────────────┬───────────┘
                          │
           ┌──────────────┴──────────────────────────────────┐
           │ pipeline ready                                    │ construction failed
           ▼                                                   ▼
 ┌──────────────────────┐                        ┌────────────────────────┐
 │       RUNNING         │                        │       FAILED           │
 │                        │                        │  (terminal)            │
 │  GStreamer pipeline    │                        │                        │
 │  active; decoded       │                        │  GStreamer construction │
 │  frames flowing into   │                        │  error; or pipeline    │
 │  DecodedFrameReady     │                        │  never reached PLAYING  │
 │  ring buffer           │                        │  within spawn_timeout   │
 │                        │                        │  (default: 5s)         │
 └──────────┬─────────────┘                        └────────────────────────┘
            │
  ┌─────────┴──────────────────────────┐
  │ any of:                             │
  │  - agent requests close             │
  │  - session revoked (budget/policy)  │
  │  - lease revoked (RFC 0008)         │
  │  - operator mutes stream            │
  │  - watchdog threshold exceeded      │
  │  - degradation step demands shed    │
  │    (see §A3; runtime or operator)   │
  ▼
 ┌──────────────────────┐
 │       DRAINING        │
 │                        │
 │  No new frames         │
 │  accepted from         │
 │  GStreamer; existing   │
 │  decoded frames in     │
 │  ring buffer are       │
 │  consumed by           │
 │  compositor; pipeline  │
 │  EOS injected          │
 └──────────┬─────────────┘
            │ ring buffer empty
            │ AND pipeline EOS confirmed
            │ (drain_timeout default: 500ms;
            │  ring buffer force-cleared on timeout)
            ▼
 ┌──────────────────────┐
 │     TERMINATED        │
 │  (terminal)           │
 │                        │
 │  GStreamer pipeline     │
 │  destroyed; tokio task  │
 │  exits; slot returned   │
 │  to pool               │
 └──────────────────────┘
```

**State invariants:**

| State | Invariant |
|-------|-----------|
| SPAWNING | Pool slot is reserved; no `DecodedFrameReady` messages sent; spawn timer running |
| RUNNING | Exactly one GStreamer pipeline active for this worker's stream; frames flow into ring buffer at pipeline rate |
| DRAINING | GStreamer pipeline received EOS; no new decode requests accepted; ring buffer drains naturally |
| TERMINATED | Pool slot released; all resources freed; `AgentResourceState` counters decremented |
| FAILED | Pool slot released immediately; error surfaced as `MediaIngressCloseNotice` (RFC 0005 A1, ServerMessage field 52) |

**FAILED is not a recoverable state for the same worker instance.** The agent
may request a new stream (triggering a fresh SPAWNING), subject to the
activation gate (§A2) passing again.

---

### A2. Activation Gate

A media worker is spawned only when all three activation conditions hold.
They are evaluated in order; the first failure short-circuits with a
structured error and no worker is created.

#### A2.1 Capability Grant: `media-ingress`

The requesting agent session must hold a valid `media-ingress` capability
grant (defined in RFC 0008 Amendment A1, §A1 capability taxonomy). This grant
is obtained through the per-session operator dialog flow (RFC 0008 A1, §A2):
the first request fires the dialog; a valid 7-day remember record or an
in-session cached grant skips it on subsequent requests within the session.

If the grant is absent or has been revoked mid-session, the spawn attempt is
rejected with `DenyReason::CAPABILITY_REQUIRED` before any pool slot is
reserved or any budget check is performed.

#### A2.2 Budget Headroom Check

Before a pool slot is claimed, the compositor evaluates three headroom
conditions:

1. **Pool slot available.** Active worker count < pool size (N = 2–4; see
   §A3). If the pool is full, the spawn is rejected with
   `DenyReason::RESOURCE_EXHAUSTED` and the agent receives a `worker_pool_full`
   reason in the `MediaIngressOpenResult` (RFC 0005 A1, ServerMessage field
   50).

2. **Per-session stream cap not exceeded.** The agent session may not exceed
   its negotiated `max_concurrent_media_streams` limit (default: 1; see §A4
   watchdog table for the per-session stream-count counter). Rejection reason:
   `DenyReason::SESSION_STREAM_LIMIT`.

3. **Global texture memory headroom.** Remaining GPU texture budget
   (system-level, not per-agent) must exceed a configurable minimum threshold
   (default: 128 MiB free). A new stream's decoded frames would consume GPU
   texture memory; starting when memory is nearly exhausted would immediately
   trigger the degradation ladder. If headroom is insufficient, the spawn is
   deferred (not rejected) with a `worker_spawn_deferred` telemetry event;
   the spawn is retried once per frame for up to `worker_defer_timeout_s`
   (default: 10s) before converting to rejection.

#### A2.3 Role Authority

The `media-ingress` capability grant must have been authorized by an operator
principal with sufficient role authority per RFC 0009 Amendment A1, §A1.3:
`owner` or `admin` role is required to grant `media-ingress`. A grant
authorized by a `member` or `guest` principal is invalid and must not reach
this check (RFC 0008 A1 §A2 rejects it at dialog time). This gate is a
defense-in-depth re-check, not the primary enforcement point.

**Evaluation order:** capability check (A2.1) → budget check (A2.2) →
role-authority re-check (A2.3) → SPAWNING.

---

### A3. Shared Worker Pool Semantics

#### A3.1 Pool Size

The media worker pool holds N workers, where N is configured at startup:

| Config key | Default | Min | Max | Notes |
|---|---|---|---|---|
| `media.worker_pool_size` | 2 | 1 | 4 | Total concurrent media worker slots |
| `media.worker_pool_size_max_budget_pressure` | 1 | 1 | N | Effective pool size under budget pressure (see §A3.3) |

Default N = 2 balances two concurrent media streams (typical for a picture-in-
picture use case) against the GStreamer thread overhead of N concurrent
pipelines. N = 4 is the hard maximum; above this, the per-pipeline GStreamer
thread overhead begins to contend with the compositor thread's Stage 3–7
budgets on typical hardware.

#### A3.2 Priority-Based Preemption

When all N pool slots are occupied and a new high-priority stream spawn is
requested, the pool may preempt the lowest-priority active worker:

**Preemption is triggered only when both conditions hold:**

1. The requesting agent's session holds a `lease:priority:1` (High) or higher
   lease — established as the stream's associated tile lease.
2. At least one active worker is servicing a stream whose associated lease
   priority is strictly lower than the requesting agent's lease priority.

**Preemption sequence:**

1. The lowest-priority active worker (highest `lease_priority` value, breaking
   ties by `z_order ASC` — background tiles before foreground, matching the
   shedding order in RFC 0008 §2.2) transitions to DRAINING.
2. A `MediaIngressCloseNotice` with `reason: PREEMPTED` is enqueued to the
   preempted agent (RFC 0005 A1, ServerMessage field 52).
3. The pool slot is reserved for the new worker. Spawning begins immediately;
   it does not wait for DRAINING to complete (DRAINING is asynchronous).
4. A `worker_preempted` telemetry event is emitted.

**Preemption is not used to enforce equal scheduling between same-priority
streams.** If all active workers have equal or higher priority than the
requester, the spawn is rejected with `DenyReason::RESOURCE_EXHAUSTED`.

Priority mapping follows RFC 0008 §2.2 canonical sort semantics: lower
`lease_priority` integer = higher priority (0 = Critical, 1 = High,
2 = Standard, 3 = Low, 4 = Background).

#### A3.3 Budget-Pressure Pool Contraction

When the runtime degradation level (RFC 0002 §6.2) reaches Level 2
("Reduce Texture Quality") or higher, the effective pool size contracts to
`media.worker_pool_size_max_budget_pressure` (default: 1). The pool does not
immediately preempt existing workers on contraction; it simply stops accepting
new spawn requests once the effective limit is reached. Active workers above
the contracted limit are allowed to complete naturally but are not renewed if
they reach TERMINATED while the system remains at Level 2 or above.

Pool expansion back to `media.worker_pool_size` occurs when the runtime
returns to Normal or Level 1 (degradation hysteresis, RFC 0002 §6.3).

---

### A4. Decode/Render Degradation Trigger Authority

**Who may demand a degradation step that sheds or degrades a media stream:**

| Actor | Mechanism | Notes |
|---|---|---|
| **Runtime (automatic)** | Degradation ladder advance (RFC 0002 §6.2) | Level 2 reduces video decode rate (15fps); Level 4 sheds tiles including media tiles; Level 5 suppresses all but the highest-priority tile. This is the primary degradation path. |
| **Runtime watchdog (automatic)** | Watchdog threshold crossed → DRAINING | If any single worker exceeds a watchdog threshold (see §A4.1), the watchdog transitions it to DRAINING. This is per-worker, not a global degradation step; it does not advance the ladder level. |
| **Operator (manual)** | Lease revocation via human override controls | Any operator at the screen may mute or dismiss a media stream via RFC 0007 §4 override controls, immediately transitioning the worker to DRAINING. This fires at Level 0 (RFC 0009 §"Human override"). |

**Who may NOT demand degradation:**

- **Agents.** Agents may request that their own stream be closed
  (`MediaIngressClose`, RFC 0005 A1, ClientMessage field 51), which
  transitions their worker to DRAINING. This is a self-service teardown, not
  a degradation demand. Agents cannot demand degradation of other agents'
  streams. Agents cannot trigger the runtime degradation ladder advance
  through any protocol message.

The degradation trigger authority aligns with the E25 degradation ladder
described in `about/heart-and-soul/failure.md` and RFC 0002 §6.2. The E25
ladder amendment (expected under a separate bead) will add the media-specific
axes ("reduce concurrent streams", "audio-first fallback") that RFC 0002 §6.2
currently defers to post-v1. This amendment's trigger authority table is
written to be forward-compatible with that E25 amendment.

#### A4.1 Watchdog Targets

The budget watchdog observes the following per-worker resources. Crossing
any threshold triggers a RUNNING → DRAINING transition for that worker and
a structured telemetry event. Thresholds are configurable; defaults are
conservative baselines.

| Resource | Metric | Default threshold | Notes |
|---|---|---|---|
| CPU time | Rolling 10s CPU-time budget for the compositor-managed tokio tasks associated with this worker (excludes GStreamer's internal thread time, which is not under compositor control) | 200ms / 10s window (2% per core) | Per-worker; not shared across pool |
| GPU texture occupancy | Texture memory held by this worker's decoded-frame ring buffer | 256 MiB | Includes all frames in the ring buffer plus the current compositor-uploaded texture |
| Ring-buffer occupancy | Fraction of the 4-slot `DecodedFrameReady` ring buffer (per RFC 0002 §2.6) that has been at or above 75% full for a sustained period | ≥ 75% full for 30 consecutive frames (500ms at 60fps) | Indicates decoder running too far ahead; may signal a stalled compositor |
| Decoder lifetime | Wall-clock time since this worker entered RUNNING | 24 hours | Forces a clean pipeline teardown and re-spawn to prevent long-running GStreamer resource leaks |
| Leases held | Active lease count for the session owning this worker | Governed by `max_active_leases` (RFC 0002 §4.3 default: 8); no additional media-specific limit | Lease-count limit already enforced by per-agent envelope; watchdog does not re-enforce it |

**Watchdog implementation note:** The watchdog is a single tokio task shared
across all workers in the pool. It evaluates thresholds on a configurable
interval (default: 1s). It does not run on the compositor thread and must not
block the frame pipeline.

**Watchdog → degradation ladder interaction:** A watchdog-triggered DRAINING
on one worker does NOT automatically advance the runtime degradation level.
The frame-time guardian (RFC 0002 §5.2) and the degradation trigger (RFC 0002
§6.1) continue to evaluate frame time independently. If shedding one worker's
stream is sufficient to bring `frame_time_p95` below the 12ms recovery
threshold, the degradation level may recover naturally. If not, the ladder
continues advancing.

---

### A5. In-Process Tokio Task Model (E24 COMPATIBLE)

The media worker pool is implemented as compositor-owned tokio tasks managing
GStreamer pipelines as black boxes. This is the model pre-declared in RFC 0002
§2.8 and ratified by the E24 verdict.

#### A5.1 Layer Separation

```
┌─────────────────────────────────────────────────────────────┐
│  Media Worker Layer (compositor-owned tokio tasks)           │
│                                                              │
│  Session Coordinator (tokio task, one per active worker)     │
│    ├─ Manages GStreamer pipeline lifecycle via Rust API       │
│    ├─ Enforces per-worker budget thresholds (A4.1)           │
│    ├─ Tags frames with owning session_id for cross-agent     │
│    │  isolation enforcement                                  │
│    └─ Sends DecodedFrameReady messages to compositor thread  │
│                                                              │
│  Watchdog (tokio task, shared across pool, periodic eval)    │
│    └─ Polls per-worker resource metrics; triggers DRAINING   │
│                                                              │
│  Pool Manager (compositor thread responsibility at          │
│   Stage 3 / spawn-request handling)                         │
│    └─ Evaluates activation gate; claims / releases slots     │
└─────────────────────────────────────────────────────────────┘
         │ GStreamer Rust pipeline API
         ▼
┌─────────────────────────────────────────────────────────────┐
│  GStreamer Pipeline Layer (GStreamer-managed thread pool)     │
│                                                              │
│  Internal GStreamer scheduler — NOT compositor-controlled    │
│  Thread count: GStreamer-determined at pipeline construction  │
│  Lifetime: created on SPAWNING; destroyed on TERMINATED      │
│                                                              │
│  Decoded frames → DecodedFrameReady ring buffer              │
│                  (4 slots per stream, drop-oldest, §2.6)     │
└─────────────────────────────────────────────────────────────┘
         │ DecodedFrameReady channel
         ▼
┌─────────────────────────────────────────────────────────────┐
│  Compositor Thread (frame pipeline Stage 3 / Stage 6)        │
│                                                              │
│  Drains DecodedFrameReady at Stage 3 or dedicated sub-stage  │
│  Uploads decoded frames to GPU texture (device.create_texture│
│    + queue.write_texture) — sole wgpu device owner (§2.8)    │
│  Blits GPU texture into tile compositing region at Stage 6   │
└─────────────────────────────────────────────────────────────┘
```

**The trust boundary** is the gRPC/MCP wire, not the tokio task boundary. All
layers above operate on the trusted side of that boundary. Cross-agent
isolation is enforced by `session_id` tagging on `DecodedFrameReady` messages
and by the compositor thread's refusal to blit a frame tagged with session A's
`session_id` into session B's tile — exactly as `security.md` §"In-process
media and runtime workers" states.

#### A5.2 GPU Device Ownership (Unchanged from §2.8)

RFC 0002 §2.8's GPU device ownership invariant is **unchanged** by this
amendment:

> The compositor thread is the sole owner of the wgpu `Device` and `Queue`.
> No media worker thread may access the GPU device directly.

Media workers deliver decoded frames over `DecodedFrameReady` as CPU-side
buffers (or Linux DMA-BUF handles in the post-v1 zero-copy path). The
compositor thread performs all GPU uploads. This invariant must not be
relaxed by RFC 0014 or any downstream implementation task.

#### A5.3 Tokio Runtime Sharing

The session coordinator and watchdog tokio tasks run on the **network
thread's Tokio runtime** (RFC 0002 §2.4: multi-thread, capped at 8 threads).
They are I/O-bound coordination tasks, not compute-heavy. Compute-heavy media
work (decode) is performed by GStreamer's own thread pool, not by tokio tasks.

Media worker tokio tasks must not perform blocking I/O or long synchronous
waits. GStreamer pipeline state transitions (NULL → READY → PAUSED → PLAYING)
must be driven via tokio `spawn_blocking` if they are not already async.

---

### A6. Cross-References to RFC 0014

RFC 0014 (Media Plane) is the forthcoming mechanism RFC that owns the
authoritative GStreamer pipeline and WebRTC transport specification for the
v2 embodied media presence program. This amendment establishes the lifecycle
contract; RFC 0014 will specify how to implement it.

Points RFC 0014 must honor from this amendment:

1. The worker state machine in §A1 is normative. RFC 0014 must not introduce
   additional states or skip the DRAINING → TERMINATED sequence.
2. The activation gate in §A2 is normative. RFC 0014 must not spawn workers
   that have not passed all three gate conditions.
3. Pool size (N = 2–4) and the hard N = 4 maximum are normative. RFC 0014
   may narrow the configurable range but must not widen it.
4. The GPU device ownership invariant (§A5.2) is normative. RFC 0014 must
   not propose any path by which GStreamer or WebRTC threads access the wgpu
   device directly.
5. The `DecodedFrameReady` channel shape (§2.6 + §2.8 in RFC 0002) is
   normative. RFC 0014 may add fields to the message but must not change the
   channel semantics (ring buffer, 4-slot capacity per stream, drop-oldest).
6. Cross-agent isolation via `session_id` tagging on `DecodedFrameReady` is
   normative. RFC 0014 must specify how the `session_id` field is populated
   and how the compositor thread enforces the no-cross-agent-blit rule.

---

## Changes to RFC 0002

This amendment adds a **Review Record** section (§12) to RFC 0002 with an
A1 row recording this amendment. It does not add inline content to §2.8 —
the §2.8 reservation text stands as the v1 baseline; this amendment's
standalone document is the post-v1 normative specification.

### Changelog Row (to be added as §12 in RFC 0002)

```
## 12. Review Record

| Round | Date | Reviewer | Focus | Changes |
|-------|------|----------|-------|---------|
| A1 | 2026-04-19 | hud-ora8.1.9 | Amendment: media worker lifecycle | Converted RFC 0002 §2.8 ("Future: Media Worker Boundary") from reservation to normative lifecycle spec. Added worker state machine (SPAWNING → RUNNING → DRAINING → TERMINATED; FAILED terminal state). Defined three-condition activation gate: capability grant (RFC 0008 A1 `media-ingress`), budget headroom check (pool slot, per-session stream cap, global texture headroom), and role-authority re-check (RFC 0009 A1: owner or admin). Specified shared worker pool: N = 2–4 slots, priority-based preemption (lease_priority sort per RFC 0008 §2.2), budget-pressure contraction to 1 slot at degradation Level 2+. Defined degradation trigger authority: runtime-automatic (ladder advance), watchdog-automatic (per-worker threshold), operator-manual (Level 0 override); agents may only self-close, not demand degradation. Specified watchdog targets: CPU time (200ms/10s), GPU texture occupancy (256 MiB), ring-buffer occupancy (75%/30-frame sustained), decoder lifetime (24h), leases held (per §4.3 envelope). Documented in-process tokio task model (E24 COMPATIBLE verdict, e24-in-process-worker-posture.md): session coordinator + watchdog tasks on network tokio runtime; GStreamer pipeline pool as black box; GPU device ownership invariant unchanged from §2.8; cross-agent isolation via session_id tagging on DecodedFrameReady. Added RFC 0014 forward cross-references. Full amendment document: `about/legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md` (issue hud-ora8.1.9). |
```

---

## References

- `about/legends-and-lore/rfcs/0002-runtime-kernel.md` §1.1 (Single-Process
  Model), §2.6 (Channel Topology), §2.8 (Future: Media Worker Boundary),
  §5.2 (Frame-Time Guardian), §6 (Degradation Policy)
- `docs/decisions/e24-in-process-worker-posture.md` — E24 COMPATIBLE verdict
  and full rationale
- `about/heart-and-soul/security.md` §"In-process media and runtime workers"
  (PR #511 amendment, hud-ora8.1.7)
- `about/legends-and-lore/rfcs/reviews/0008-amendment-c13-capability-dialog.md`
  — capability taxonomy (A1), per-session dialog flow (A2), 7-day remember
  (A3)
- `about/legends-and-lore/rfcs/0009-policy-arbitration.md` Amendment A1
  (C12 role-based operators) — role authority for capability grants
- `about/heart-and-soul/failure.md` — E25 degradation ladder and axes
  (authoritative doctrine; RFC 0014 media-specific axes to be inserted)
- `openspec/changes/_deferred/v2-embodied-media-presence/signoff-packet.md` §E (E24,
  E25), §"Open items" item 3
- RFC 0014 (forthcoming) — mechanism layer for media plane; must implement
  this amendment's lifecycle contract
