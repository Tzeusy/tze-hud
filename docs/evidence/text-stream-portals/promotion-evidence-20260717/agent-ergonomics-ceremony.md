# Agent-Ergonomics Demonstration — Ceremony Log

Gate criterion: an LLM session drives the full portal lifecycle exclusively through the
vendored skill surface, with zero scene-graph mutations authored in the LLM's context.

Session: Claude (claude-fable-5) in Claude Code on the Linux rig, driving the tzehouse
overlay (exe sha256 `26bedaca…`, deployed this session) via
`.claude/skills/hud-projection/scripts/portal_client.py` over MCP `tools/call`.
projection_id `claude-promo-demo`, provider_kind `claude`. Date 2026-07-17.

## Op-by-op ceremony

| # | Operation | Calls | Notes |
|---|---|---|---|
| 1 | `attach` | 1 | owner token + continuity file handled entirely by the client; never entered LLM context |
| 2 | `publish` (assistant, tool, status kinds) | 3 | per-turn attribution content for the live attribution check |
| 3 | `publish_status` (active) | 1 | header lifecycle text |
| 4 | `publish` ×3 with `coalesce_key` | 3 | rendered as ONE in-place line — verified on screenshot (`shots/cooperative-demo-portal-2.png`) |
| 5 | `publish` question turn | 1 | drove `1 unread` + `─── unread ───` divider on screen |
| 6 | `poll` (long-poll 30 s × 4) | 1 | returned `PROJECTION_NOT_FOUND` — runtime had restarted hours after deploy; error was **self-describing** with `recovery_operation: portal_projection_attach` hint |
| 7 | re-`attach` (idempotency replay) | 1 | `continuity_replayed_count: 5` — client-authored tail reconstructed the portal on the fresh runtime; no manual state handling |
| 8 | `poll` (long-poll 30 s × 4) | 1 | clean empty (`no pending input`), projection alive |
| 9 | re-`attach` (after deliberate orphan-path idle test reaped the session) | 1 | `continuity_replayed_count: 5` again — third successful replay |
| 10 | `detach` (reason recorded) | 1 | `projection detached and private state purged` |

**Totals**: 15 tool invocations for a full multi-turn lifecycle including a
runtime-restart recovery AND a lease-reap recovery. Minimal "attach and say hello": **2 calls**. Canonical
conversational turn: **3 calls** (publish → long-poll → ack); the measured token cost of
that flow is 683 tokens on the approved hud-ht1k7 baseline (piggyback proposal
hud-vconx would collapse it to 2 calls).

## Glue required outside the skill surface

- Endpoint/PSK resolution: one `eval "$(tzehouse_env.sh)"` (environment, not protocol).
- Everything else — auth headers, request ids, token custody, continuity retention and
  replay, ack bookkeeping — lived inside the deterministic client. Zero scene-graph
  mutations, zero tile/geometry/styling payloads authored in LLM context.
- Minor gap: `portal_client.py publish` exposes no `--expects-reply` passthrough
  (the MCP tool supports it); the question turn relied on default attention semantics.

## Rendering evidence

`shots/cooperative-demo-portal-2.png`: two-pane INPUT|OUTPUT chrome, header
`Claude — Promotion Evidence Demo · Active · 1 unread`, composer placeholder, coalesced
stream line, unread divider above the question turn — all runtime-authored; the LLM sent
semantic text fragments only.
