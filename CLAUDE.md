# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

**tze_hud** — an agent-native presence engine. A local, high-performance, compute- and token-efficient display runtime that gives LLMs safe, synchronized, live, interactive presence on real screens — desktop overlays and wall displays today; smart glasses and VR headsets are the declared eventual goal. See `about/heart-and-soul/` for full doctrine (start with `about/heart-and-soul/README.md`).

This is **not** a dashboard or chatbot UI. It's an operating environment for model presence: spatial ownership, timed media, bidirectional interaction, governed leases.

## Status

Active development. ~260k lines of Rust across 16 crates (13 active + 3 parked platform stubs: `tze_hud_a11y`, `tze_hud_media_apple`, `tze_hud_media_android`) plus the `tze_hud_app` binary. Doctrine is in `about/heart-and-soul/`; design contracts in `about/legends-and-lore/` (14 RFCs); capability specs in `openspec/`. See `about/heart-and-soul/v1.md` for scope boundary.

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
- **Visual identity is modular.** Every visual component (subtitle, notification, etc.) separates contract from implementation. Design tokens provide shared visual vocabulary; component profiles are the swappable unit. Never hardcode colors, fonts, or styling in the compositor — use `RenderingPolicy` fields populated from design tokens.
- **Efficiency is doctrine — compute and tokens** (`about/heart-and-soul/efficiency.md`). Idle screens cost ~nothing; work is proportional to change; nothing may assume desktop-class headroom without a degradation path (the eventual envelope is glasses/VR-class). LLM-facing surfaces are token-minimal: semantic intent in a few deterministic calls; layout/styling/chrome never pass through model context.

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
- Hardcoded visual properties in the compositor (use design tokens and RenderingPolicy)
- Monolithic visual implementations that can't be swapped (use component profiles)
- Layout, styling, or geometry data passing through LLM token context (the runtime does the design; models state intent)
- Re-rendering unchanged content, or idle scenes that burn CPU/GPU (see `about/heart-and-soul/efficiency.md`)

## LLM Self-Projection

If you are an LLM session that wants to project itself onto the HUD — to show your output, status, or live transcript on screen — use the **`hud-projection`** skill (`.claude/skills/hud-projection/SKILL.md`).

Trigger phrases: "project this session to the HUD", "attach this agent to HUD", "show this LLM session in a text-stream portal", "check HUD input", "publish status to screen".

This is cooperative opt-in projection, not PTY capture or terminal scraping. The `ProjectionAuthority` runs in-process inside the tze_hud runtime; you reach it via the full set of `portal_projection_*` MCP tools (attach, publish, publish_status, get_pending_input, acknowledge_input, detach, cleanup). These are Resident tools — an external session reaches them as the resident principal (set `TZE_HUD_MCP_RESIDENT_PRINCIPAL` equal to the PSK and send the PSK as the MCP bearer). For one-shot zone publishing (no session lifecycle), use the **`th-hud-publish`** skill instead.

## Beads Database Routing

This workspace has **two separate beads databases** on the Dolt server (port 3307):

| Working directory | Database | Prefix | Contains |
|---|---|---|---|
| `tze_hud/` (project root) | `tze_hud` | `th-` | Structural beads only (rig identity, patrol molecules) |
| `tze_hud/mayor/rig/` | `hud` | `hud-` | **All implementation work** (features, bugs, epics, tasks) |

**Always run `bd` from `mayor/rig/`** to see implementation beads.

## Worker Isolation for Rust Code Changes

This repo (`mayor/rig/`) is a **nested git repo** inside the monorepo (`~/gt`). Monorepo worktrees created by `bd worktree create` do NOT contain the Rust crate code.

For **Rust code workers**, use `git worktree` on the **tze-hud repo itself**:

```bash
# From mayor/rig/ (the tze-hud repo root):
git worktree add .worktrees/agent-hud-XXXX -b agent/hud-XXXX
# Worker operates in .worktrees/agent-hud-XXXX/
```

**Do NOT** have workers `git checkout -b agent/...` directly in the main checkout — this leaves `mayor/rig/` on a non-main branch and blocks other workers.
