# E24 In-Process Tokio Media Worker Posture — Security.md Verdict

**Bead:** hud-ora8.1.1 (Phase 1 open-item-3)
**Parent:** hud-ora8.1 (v2 phase-1 media plane + bounded-ingress absorption)
**Decision input:** `openspec/changes/v2-embodied-media-presence/signoff-packet.md` E24, open item 3
**Doctrine source:** `about/heart-and-soul/security.md`
**RFC source:** `about/legends-and-lore/rfcs/0002-runtime-kernel.md` §2.8 (Media Worker Boundary)
**Architecture source:** `about/heart-and-soul/architecture.md` §"The screen is sovereign", §"Multiple video feeds are a compositor problem"

---

## Verdict

**COMPATIBLE.** tze_hud's agent-isolation posture admits in-process tokio
media workers (E24) without requiring a subprocess-isolation pivot. Phase 1
implementation may proceed under the in-process worker model, with the
security.md amendment to be authored under hud-ora8.1.7.

## Rationale

E24's "in-process tokio tasks, not subprocess isolation, with aggressive
budget limits + watchdog" is **already the model the v1 architecture commits
to** for media. Treating it as new isolation surface area misreads where the
trust boundary lives in tze_hud.

The four load-bearing observations:

### 1. Agents already live outside the compositor's address space

`RFC 0002 §1.1 "Single-Process Model"` (line 46) is unambiguous (the "screen is sovereign" section of `architecture.md` also establishes this principle at the doctrine level):

> tze_hud runs as a single OS process. Agents are external gRPC clients;
> they do not share the compositor's address space. The compositor is the
> trusted, sovereign process — it owns the GPU context, the scene state,
> the input stream, and the window surface.

Agents reach the compositor through the gRPC resident control plane (RFC
0005) and the MCP compatibility plane. They never load code into the
compositor process. E24's media workers are *compositor-internal* threads
that decode media on behalf of an admitted stream — they are not agent
code. There is no agent-supplied bytecode, plugin, or shared memory region
involved at any point in the pipeline.

The trust boundary tze_hud cares about is the **gRPC/MCP wire**, not an
internal thread boundary inside the compositor. In-process workers do not
weaken that boundary; they sit entirely on the trusted side of it.

### 2. security.md "agent isolation" is agent-to-agent, not agent-to-runtime

The §"Agent isolation" block (security.md lines 41–53) enumerates four
isolation invariants:

> - An agent cannot read the content of another agent's tiles.
> - An agent cannot intercept another agent's input events.
> - An agent cannot access another agent's media streams.
> - An agent cannot modify another agent's leases or resources.

All four are properties the **runtime mediates between agents**. None of
them require, or even imply, that the runtime decode video for agent A in
a process separate from the runtime decoding video for agent B. The
runtime is a single trusted mediator; whether it does its work in one
tokio task pool or N subprocesses is irrelevant to those invariants, as
long as the mediator correctly tags streams with their owning agent and
denies cross-agent reads.

E24's shared worker pool with priority-based preemption preserves these
invariants intact: streams are owned by an agent's session, scheduled
under that session's budget, and torn down on session revocation. No
cross-agent leak is created by sharing a tokio runtime any more than it
is created by sharing the gRPC server's tokio runtime today.

### 3. RFC 0002 §2.8 already pre-declared in-process media workers

`about/legends-and-lore/rfcs/0002-runtime-kernel.md` §2.8 ("Future: Media
Worker Boundary") was authored in v1 as a *reservation* for exactly this
case. It explicitly states (line 396):

> Post-v1 integration of GStreamer media pipelines and WebRTC will require
> threads that are neither Tokio tasks nor `std::thread`s owned by the
> compositor. GStreamer has its own internal thread pool (managed by its
> scheduler and element graph). WebRTC ICE/DTLS threads are managed by the
> WebRTC library.

And line 414:

> The compositor thread is the sole owner of the wgpu `Device` and `Queue`.
> No media worker thread may access the GPU device directly.

The RFC reserves *in-process* worker threads inside the compositor
process, communicating with the compositor thread via the
`DecodedFrameReady` channel (§2.6). It nowhere reserves subprocess
isolation. Pivoting to subprocess isolation would *contradict* RFC
0002 §2.8 — not extend it — and would require re-opening v1's
runtime-kernel design.

One nuance worth noting: RFC 0002 §2.8 says GStreamer decode threads are
"neither Tokio tasks nor `std::thread`s owned by the compositor" — those
are GStreamer-managed. E24's Tokio-task layer sits *above* that: the
budget watchdog and session-attribution coordinator are compositor-owned
Tokio tasks that *manage* GStreamer pipelines as a black box (via the
Rust GStreamer API), consistent with §2.8's defined boundary. The Tokio
tasks do not replace or own the GStreamer internal thread pool; they
orchestrate pipeline lifecycle and enforce budgets on top of it.

### 4. The capability/budget/watchdog mechanism *is* the isolation enforcement

security.md's enforcement model is layered:

1. **Authentication** establishes identity.
2. **Capability scopes** (per-session, additive, revocable, auditable) say
   what each session may do — including the new "stream media" capability.
3. **Resource governance** (texture memory, bandwidth, concurrent streams,
   CPU time) says how much. The runtime monitors in real time and applies
   warning → throttle → revocation on overage.

E24's "aggressive budget limits + watchdog" is the *implementation* of the
resource-governance layer for the media class of work. It is in-process
because the resources it governs (GPU texture memory, decoded-frame ring
buffers, GStreamer pipeline lifetimes) are themselves in-process resources
held by the compositor. Trying to govern them from across a subprocess
boundary would require either (a) a second wgpu device (expensive, no
zero-copy — RFC 0002 §2.8 explicitly rejects this), or (b) cross-process
texture sharing primitives that vary per-platform and add a large attack
surface of their own.

The security model expects the runtime to police itself against noisy and
buggy agents — not to police itself against itself. A subprocess pivot
would solve a problem (compositor compromise from a malicious decoder) we
are not in a position to solve in v2 anyway: GStreamer and the WebRTC
stack are already shipped as a single trust unit with the compositor, and
codec sandboxing is a much larger program than the v2 calendar admits.

## Caveats and explicit non-claims

This verdict is narrow. It does **not** claim:

- That codec-level memory-safety risk is zero. It is not. Media stacks have
  historically been a CVE-rich surface. Mitigation is *defense-in-depth*
  inside the compositor process: bounded ring buffers (already in RFC
  0002), aggressive E24 budget limits, watchdog-driven decoder restart,
  and an E25 degradation ladder that can shed media entirely when a
  stream misbehaves. A future tightening to per-codec sandbox processes
  remains a legitimate post-v2 hardening item, but it does not block
  phase 1.
- That subprocess isolation is the wrong *eventual* answer. If a future
  threat model (e.g., codecs from untrusted upstreams, agent-supplied
  decoder plugins) appears, revisit. v2's scope is bounded ingress from
  trusted-codec sources; that scope does not justify the cost.
- That the in-process model is free of work. Phase 1 must still:
  - implement the budget watchdog with measurable thresholds,
  - wire stream-owning-session attribution end-to-end so cross-agent
    isolation invariants are testable,
  - exercise the E25 degradation ladder under simulated decoder
    misbehavior in validation.

These are implementation tasks for hud-ora8.1.* under the existing E24/E25
work, not blockers on the security posture verdict.

## Downstream impact

- **hud-ora8.1.7** (security.md amend) is unblocked. The amendment should
  cite this verdict and restate the rationale in security.md's §"Agent
  isolation" surface — likely as a short paragraph clarifying that
  in-process compositor workers (media decode, scheduler, watchdog) sit
  on the trusted side of the agent-runtime boundary and are governed by
  resource budgets rather than process isolation.
- **hud-ora8.1.22 – hud-ora8.1.27** (Tasks 1.1–1.5 + closeout) are
  unblocked at the security-posture gate. Other gates (RFC 0014 review,
  doctrine-before-RFC sequencing per F30) still apply on their own
  schedule.
- **No phase-1 pivot bead is required.** The "Incompatible" branch of
  open item 3 does not fire. The risk noted in signoff-packet.md
  §"Key risks" item 3 ("In-process tokio media workers (E24)") is
  resolved as posture-compatible; it remains a watch item for codec-CVE
  defense-in-depth but is not a structural blocker.

## Suggested amendment text for hud-ora8.1.7

A concrete draft to seed the security.md amendment work (final wording is
hud-ora8.1.7's call):

> ### In-process media and runtime workers
>
> The compositor is a single trusted OS process. Agents reach it across
> gRPC and MCP wires; they never load code into it. Media decode,
> scheduling, and watchdog work happen on tokio tasks and library-managed
> threads (notably GStreamer's pipeline pool and WebRTC's transport
> threads) inside that one process — not in subprocesses. This is
> intentional and compatible with the agent-isolation model:
>
> - The trust boundary protected by this document is the agent-runtime
>   wire, not an internal thread boundary inside the runtime.
> - Cross-agent isolation invariants (no read of another agent's tiles,
>   input, media streams, or leases) are properties the runtime mediates;
>   they do not depend on per-agent process separation.
> - Resource governance — texture memory, bandwidth, concurrent streams,
>   CPU time, decoder slots — is enforced by the in-process budget
>   watchdog with the warning → throttle → revocation cascade described
>   above.
> - GPU device ownership remains exclusive to the compositor thread (see
>   RFC 0002 §2.8); media workers deliver decoded frames over a bounded
>   ring buffer and never touch wgpu directly.
>
> Subprocess isolation of codecs is a legitimate post-v2 defense-in-depth
> hardening if the threat model later admits untrusted-codec or
> agent-supplied-decoder cases. v2's bounded-ingress scope does not.

## References

- `about/heart-and-soul/security.md` (whole file, especially §"Agent
  isolation" and §"Resource governance")
- `about/heart-and-soul/architecture.md` §"The screen is sovereign",
  §"Multiple video feeds are a compositor problem", §"Media: GStreamer"
- `about/legends-and-lore/rfcs/0002-runtime-kernel.md` §2.8 "Future:
  Media Worker Boundary"
- `openspec/changes/v2-embodied-media-presence/signoff-packet.md` §E
  (E24, E25), §"Open items" item 3, §"Key risks" item 3
- `openspec/changes/v2-embodied-media-presence/specs/media-plane/spec.md`
- `openspec/changes/v2-embodied-media-presence/proposal.md` (capability
  scope)
- `openspec/changes/v2-embodied-media-presence/design.md` (phased
  expansion, "media stays governed")
