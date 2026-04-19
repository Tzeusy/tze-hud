# Media Doctrine

The media plane exists to let models be present as living, temporal entities — not
as static pages or text streams, but as entities that speak, hear, and show. A
runtime without media keeps agents behind a one-way glass. Media is the glass coming
down.

This document defines what the media plane IS, what it NEVER DOES, and the governance
posture that makes it safe. It precedes RFC 0014 (the mechanism layer). RFC 0014 will
specify wire format, state machines, and transport descriptors — those are not doctrine.
This is doctrine.

## What the media plane is

The media plane is the third of three protocol planes:

1. **MCP** — compatibility perimeter for one-shot semantic commands.
2. **gRPC** — resident control plane for scene diffs, leases, and event subscriptions.
3. **WebRTC (media plane)** — live, low-latency audio/video: camera feeds, voice
   sessions, smart-glasses surfaces, bidirectional AV.

The media plane activates only for **embodied agents** — the presence level that
requires the strongest trust, the most explicit operator permission, and the strictest
revocation semantics. A resident agent can hold tiles and publish to zones without
ever touching the media plane.

What the media plane does:

- Carries live audio and video surfaces from and to agents with admitted media capability.
- Delivers decoded frames into GPU-resident compositor surfaces without bypassing
  the compositor's ownership of the wgpu device.
- Synchronizes media timing against the compositor's clock so AV, subtitles, and
  scene events align correctly (arrival time is not presentation time — this rule does
  not relax because the payload is a video frame).
- Reports stream health and degradation state to the runtime so the degradation ladder
  can respond proportionally.

What the media plane is built on: **GStreamer** for decode, timing, and synchronization;
**WebRTC** for live bidirectional AV transport. These are not swappable choices — they
were selected because media is not an add-on and the integration substrate must be
purpose-built for it.

## What the media plane never does

The media plane is a governed surface, not a free pass.

**It never bypasses the agent-runtime trust boundary.** Agents reach the compositor
across gRPC and MCP wires; they never load code into it. Media workers are
in-process tokio tasks and library-managed threads on the trusted side of that wire
— the same side as the gRPC server's own runtime. This is the E24 verdict: in-process
is compatible with agent isolation because the isolation invariants are properties the
runtime mediates between agents, not an internal thread-boundary problem.

**It never grants agents direct GPU access.** Media workers deliver decoded frames
over bounded ring buffers. The wgpu device is exclusive to the compositor thread.
No media path crosses that line.

**It never admits a stream without explicit authority.** Media admission requires
capability gating (`media_ingress` or the applicable per-type capability), protocol
version negotiation, and operator policy clearance. A session without the right
capability gates receives a structured error and all v1 zone/widget behavior is
unaffected.

**It never circumvents privacy or attention governance.** A media stream is subject
to the full arbitration stack: capability gate → privacy/viewer gate → interruption
policy → attention budget → degradation budget. Live video from a doorbell camera
and ambient background audio are not the same class of concern — the runtime knows
that because policy evaluation runs in the same stack that governs subtitles and
notifications.

**It never keeps going when the budget says stop.** The degradation ladder governs
media explicitly: framerate → resolution → second stream → freeze-and-no-input →
tear down media (keep session) → revoke embodied → disconnect. (This list shows the
media-plane-specific subset; see failure.md E25 for the complete 10-step canonical
ladder.) The order is doctrine, not implementation preference. Agents may not initiate
their own degradation; the runtime degrades and reports.

**It never records content silently.** Recording is a separate capability, a separate
operator grant, and a separate doctrine concern (see the forthcoming `recording-ethics.md`).
The media plane carries live streams; recording policy governs whether any of that
is persisted.

## Governance posture

Media governance has four pillars:

**Capability-gated (per RFC 0008 Amendment 1 / C13).** Every media capability —
`media_ingress`, `microphone_ingress`, `audio_emit`, `recording`, `cloud_relay` — is
additive, per-session, revocable, and operator-authorized. The runtime configuration
sets the fundamental on/off boundary; per-session capability dialogs handle first-use
grants within enabled capabilities. There is no ambient media access.

**Role-arbitrated (per RFC 0009 Amendment 1 / C12).** Capability grants require an
authorizing operator principal with the appropriate role. Owner and admin roles may
grant high-priority embodied capabilities. Member and guest roles have no
capability-management authority for media. An agent's capability grants are separate
from the role of the human operator who configured them.

**In-process, budget-watched (E24 verdict + E25 ladder).** Media workers run inside
the compositor process under tokio task governance, not in subprocesses. The
in-process model is compatible with agent isolation because isolation is mediated at
the gRPC/MCP wire, not at an internal thread boundary. Budget enforcement (texture
memory, bandwidth, concurrent streams, CPU time, decoder slots) is the mechanism
that makes in-process safe: warning → throttle → revocation, with the E25 degradation
ladder governing media-specific shedding.

**Session-attributed end-to-end.** Every media stream is owned by an agent's session.
Cross-agent isolation invariants — no agent reads another agent's media streams — are
testable properties the runtime enforces by tagging streams to their owning session and
denying cross-session reads. Sharing a tokio runtime with other sessions does not
weaken this; it is no different from sharing the gRPC server's runtime.

## Sequencing note

This doctrine file precedes RFC 0014 (Media Plane Wire Protocol). RFC 0014 defines
mechanism: wire format, field allocations, state machines, transport descriptor
schema, reconnect semantics. This document defines posture: what the media plane is
for, what it refuses to do, and which governance pillars it operates under.

The sequence is not ceremony — it is how the non-negotiables get locked in before
the mechanism discussion starts and every constraint is still negotiable.

## Cross-references

- `presence.md` — embodied presence level; the trust ceiling that media activates under
- `security.md` — agent-runtime trust boundary; capability scopes; E24 in-process
  posture; resource governance cascade
- `failure.md` — E25 degradation ladder; what happens when media misbehaves
- `v1.md` — v1 defers media entirely; the post-v1 media tranche is defined here
- `v2.md` (forthcoming) — the v2 program structure; phase 1 bounded media activation;
  phase 2 embodied presence; phase 4 bidirectional AV
- RFC 0014 (forthcoming) — media-plane mechanism layer; MUST read with this doctrine
