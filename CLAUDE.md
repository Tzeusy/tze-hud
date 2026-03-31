# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

**tze_hud** — an agent-native presence engine. A local, high-performance display runtime that gives LLMs safe, synchronized, live, interactive presence on real screens (wall displays to smart glasses). See `about/heart-and-soul/` for full doctrine (start with `about/heart-and-soul/README.md`).

This is **not** a dashboard or chatbot UI. It's an operating environment for model presence: spatial ownership, timed media, bidirectional interaction, governed leases.

## Status

Pre-code. Doctrine is written (`about/heart-and-soul/`); the next artifact is an RFC with scene object model, session/lease model, protobuf definitions, and core RPCs. See `about/heart-and-soul/v1.md` for scope boundary.

## Locked-In Technology Decisions

These are intentional and should not be second-guessed:

- **Rust** for the latency-sensitive core (compositor, control plane)
- **Tokio** for async runtime; **tonic** for gRPC
- **wgpu + winit** for cross-platform native GPU rendering and input
- **GStreamer** (with Rust bindings) for media ingest, decode, timing, synchronization
- **WebRTC** for live interactive audio/video
- **TypeScript** only for inspectors, admin panels, authoring tools — never in the hot path

## Three-Plane Protocol Architecture

1. **MCP (compatibility plane)** — JSON-RPC for LLM interoperability: create tab, mount widget, set markdown. Not for high-rate traffic or media.
2. **gRPC (resident control plane)** — protobuf over HTTP/2 for scene diffs, leases, event subscriptions, hot dashboards. Long-lived bidirectional streams.
3. **WebRTC (media plane)** — live camera feeds, voice/video sessions, low-latency bidirectional AV.

Do not collapse these into one protocol.

## Core Rules

- **LLMs must never sit in the frame loop.** Models drive the scene; the runtime composits.
- **Arrival time ≠ presentation time.** All payloads carry timing semantics (present_at, expires_at, sync_group, etc.).
- **Local feedback first.** Touch/interaction acknowledgement happens locally and instantly; remote semantics follow.
- **One scene model, two profiles.** Desktop (Full Display Node) and mobile (Mobile Presence Node) share the same API; differences are negotiated capabilities/budgets, not separate architectures.
- **Screen is sovereign.** The runtime owns pixels, timing, composition, permissions, arbitration. Models request via leases with TTL, capability scopes, and revocation semantics.

## Four Message Classes

Design transport around these — they have different delivery semantics:

| Class | Semantics | Examples |
|-------|-----------|----------|
| Transactional | Reliable, ordered, acked | create tile, grant lease, switch tab |
| State-stream | Reliable, ordered, coalesced | dashboard updates, scene patches |
| Ephemeral realtime | Low-latency, droppable, latest-wins | hover, cursor trails, interim speech |
| Clocked media/cues | Scheduled against media/display clock | AV frames, subtitles, word-highlighting |

## Anti-Patterns

- Browser as main renderer (use it for tooling only)
- Verbose JSON on hot paths
- Touch depending on remote roundtrip
- Forking API into desktop vs mobile versions
- Unbounded agent screen territory (use leases)
- Treating graceful degradation as a bug
