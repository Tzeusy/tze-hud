# Text Stream Portal Phase-1 Live Evidence - hud-kylt0

Date: 2026-06-19
Issue: `hud-kylt0`
Adapter family: cooperative-projection MCP facade
Reference baseline: `hud-ofe76` evidence committed at `705c946e`

## Scope

This note packages the live cooperative-projection evidence observed by the
previous worker. It does not rerun the full live Windows test. The purpose is to
separate two facts:

- the Windows HUD runtime was reachable and accepting ordinary MCP requests
- the cooperative-projection facade was still blocked by resident-only
  capability gating and incomplete deployed method coverage

The note is sanitized for the public repository. Host, user, and key identifiers
use the established placeholders from the Windows operations docs. The recovered
runtime credential value is not recorded.

## Bootstrap And Reachability

- Worker context was valid at
  `.worktrees/parallel-agents/hud-kylt0` on branch `agent/hud-kylt0`.
- Both configured Windows SSH principals succeeded through the repo-private
  identity, represented here as `hud-user`, `admin-user`, and
  `~/.ssh/hud-ssh-key`.
- `TzeHudOverlay` was `Running`.
- From Windows localhost, both runtime ports were reachable:
  - MCP: `9090`
  - gRPC: `50051`
- Runtime MCP `list_zones` succeeded through
  `http://windows-host.example:9090/mcp` after recovering the non-default
  runtime credential from the scheduled-task XML. The credential value was used
  only for the live probe and is not stored in this artifact.

## Projection Facade Results

| Method | Live result | Recorded context |
|---|---|---|
| `portal_projection_attach` | `CAPABILITY_REQUIRED` | `tool=portal_projection_attach`, `hint.required_capability=resident_mcp` |
| `portal_projection_publish` | `CAPABILITY_REQUIRED` | `tool=portal_projection_publish`, `hint.required_capability=resident_mcp` |
| `portal_projection_get_pending_input` | `CAPABILITY_REQUIRED` | `tool=portal_projection_get_pending_input`, `hint.required_capability=resident_mcp` |
| `portal_projection_acknowledge_input` | `CAPABILITY_REQUIRED` | `tool=portal_projection_acknowledge_input`, `hint.required_capability=resident_mcp` |
| `portal_projection_detach` | `CAPABILITY_REQUIRED` | `tool=portal_projection_detach`, `hint.required_capability=resident_mcp` |
| `portal_projection_cleanup` | `Method not found` | deployed runtime lacked the method even though current source contains cleanup MCP ingress |
| `projection_operation` | `Method not found` | dispatcher-style facade is not deployed |
| `portal_projection_publish_status` | `Method not found` | status facade is not deployed |

## Interpretation

The live result matches the local `hud-projection` skill caveat: the output
facade is wired in-process, but normal external HTTP MCP callers do not receive
the `resident_mcp` capability. The input-return and lifecycle operations also
remain unavailable or gated through the deployed MCP surface. Therefore the
agent-ergonomics lifecycle demonstration could not progress through
attach -> stream -> poll/ack input -> detach using only the vendored skill
surface.

This is not a Windows reachability failure. SSH, scheduled task state, local
ports, and basic MCP discovery were healthy. The blocker is resident-capable
projection ingress plus deployed method parity for the remaining lifecycle
surface.

No scene-graph mutations, raw-tile assembly, or tile-shape workarounds were
authored by the worker during this cooperative-projection probe. The observed
ceremony was limited to runtime credential recovery and HTTP MCP facade calls;
the facade stopped before it could create or reuse a visible text-stream portal.

## Relationship To Recent Baseline

The immediately preceding `hud-ofe76` evidence package at commit `705c946e`
covered the raw-tile exemplar-script adapter path:

- cleanup errors were empty
- explicit lease release completed
- diagnostic input covered focus, drag, and scroll checkpoints
- cadence still failed the runtime-overhead budget

Together, `hud-ofe76` and `hud-kylt0` show that Phase-1 still has two separate
blocking axes: raw-tile exemplar cadence conformance, and cooperative-projection
resident-capable ingress. Cooperative projection remains gated by ingress and
method surface readiness rather than Windows host availability.
