# hud-po7iz Release Soak Coverage Reconciliation - 2026-05-12

Issue: `hud-po7iz`
Parent evidence: `hud-nfl7n`
Decision surface: `openspec/changes/windows-first-performant-runtime/`

## Verdict

Do not narrow the release criterion to widget soak plus separate MCP smokes.

`hud-nfl7n` is valid widget-soak evidence, but it does not satisfy the active
Windows-first release criterion for a 60-minute resident workload. The release
bar remains blocked until a comparable live run concurrently covers resident
scene tiles, widgets, zone publishing, and lease cleanup on the reference
Windows runtime.

This reconciliation is source/spec/evidence only. It intentionally does not
start, stop, reconfigure, or restart the shared TzeHouse HUD runtime because a
parallel worker may be using that runtime for live validation.

## Source Of Truth

`openspec/changes/windows-first-performant-runtime/design.md` defines the
shipping target as a 60-minute soak with three concurrent resident agents
publishing scene, widget, and zone updates.

`openspec/changes/windows-first-performant-runtime/tasks.md` section 5.1 keeps
that requirement operational: "60-minute multi-agent soak with three concurrent
resident agents publishing scenes/widgets/zones; verify no leaks, no
regressions, no jitter excursions."

The broader validation doctrine also expects hours-long tests to cover repeated
agent connects, disconnects, reconnects, lease grants, revocations, zone
publishes, and content updates, with no orphaned resources or accumulating scene
graph garbage.

Lease-governance specs make zone coverage inseparable from lease coverage for
resident agents: zone publish requires an active lease, and revoked or expired
leases must clear their zone publications.

## Existing Evidence

`docs/reports/hud-nfl7n_windows_soak_20260512.md` records a successful
60-minute three-agent run of the canonical `widget_soak_runner.py` path:

- `agent-alpha`, `agent-beta`, and `agent-gamma` published `10,800/10,800`
  accepted durable widget updates with zero errors.
- Live frame/input metrics were collected and parsed.
- Resource sampling found exactly one benchmark-config HUD process on every
  sample.
- Private-memory drift was `+1.56 MiB`, inside the 5 MiB gate.
- Cleanup evidence proves `main-progress` widget publications were cleared.

That evidence is not sufficient for task 5.1 because the run kind is
`widget_publish_three_agent_soak`, and its command lines contain only
`WidgetPublish` traffic against `main-progress`. It did not create resident
scene tiles, submit scene mutation batches, publish zones through the resident
session stream, or prove lease-driven cleanup of tiles and zone publications.

`app/tze_hud_app/config/benchmark.toml` already has the right registered
benchmark identities for the missing coverage. `agent-alpha`, `agent-beta`, and
`agent-gamma` are granted `create_tiles`, `modify_own_tiles`, widget publish
capabilities for `main-progress`, `main-gauge`, and `main-status`, and zone
publish capabilities for `subtitle`, `notification-area`, and `status-bar`.

## Why MCP Smokes Are Not Enough

Separate MCP zone smokes are still useful preflight checks, but they do not
replace the resident release soak for three reasons:

1. The release criterion names resident agents and scene/widget/zone concurrency,
   not a collection of isolated endpoint smokes.
2. MCP `publish_to_zone` traffic does not prove resident gRPC lease ownership,
   `LeaseRequest` handling, `MutationBatch` tile updates, or `LeaseRelease`
   cleanup.
3. The leak/cleanup risk is cross-surface: a release-quality soak must prove
   that scene graph state, widget publications, zone publications, and lease
   lifecycle cleanup remain coherent under the same long-lived sessions.

## Required Live Workload

A merge-ready replacement or extension for `hud-nfl7n` should produce a new
artifact directory under `docs/reports/artifacts/` and a report under
`docs/reports/` with this minimum shape.

### Runtime Preconditions

- Use TzeHouse reference hardware with the benchmark HUD task/config:
  `C:\tze_hud\benchmark.toml`.
- Verify Tailscale, non-interactive SSH, gRPC `:50051`, MCP `/mcp`, and process
  sampling before starting.
- Use the benchmark task's non-default PSK without writing the secret to repo
  artifacts.
- Coordinate the shared Windows HUD/GPU runtime first; do not restart or
  reconfigure it while another live worker is using it.

### Three Resident Sessions

Run three concurrent gRPC resident sessions for 3600 seconds using
`agent-alpha`, `agent-beta`, and `agent-gamma`.

Each session must:

- request a lease with at least `create_tiles`, `modify_own_tiles`, and the
  relevant `publish_zone:*` capabilities, with a TTL long enough for the full
  run plus cleanup;
- create at least one resident tile and update its root or child nodes during
  the run;
- publish durable widget updates to one of the benchmark widget instances;
- publish durable zone updates to at least one benchmark zone with the correct
  zone media type;
- maintain per-request counts, accepted counts, p50/p99 RTTs, and error codes
  separately for scene mutations, widget publishes, and zone publishes.

To avoid hiding contention, rotate the surfaces rather than sending all traffic
to one endpoint:

| Agent | Tile lane | Widget lane | Zone lane |
|---|---|---|---|
| `agent-alpha` | one small scene tile updated at 1 Hz | `main-progress` | `subtitle` stream text |
| `agent-beta` | one small scene tile updated at 1 Hz | `main-status` | `notification-area` short text |
| `agent-gamma` | one small scene tile updated at 1 Hz | `main-gauge` | `status-bar` key/value pairs |

### Cleanup Proof

The artifact must prove cleanup through the lease system, not just endpoint
clear calls:

- `agent-alpha`: explicit `LeaseRelease`; verify its tile is removed and its
  zone publications are no longer active.
- `agent-beta`: graceful `SessionClose(expect_resume=false)`; verify cleanup
  occurs within the implementation's allowed grace ceiling.
- `agent-gamma`: ungraceful transport drop; wait heartbeat detection plus the
  reconnect grace window, then verify orphaned lease expiry removes its tile and
  zone publications.

The report should include post-cleanup scene/element/zone snapshots or a
machine-readable equivalent showing zero residual tiles and zone publications
for the three benchmark namespaces.

### Metrics And Gates

Carry forward the `hud-nfl7n` release-gate metrics:

- live frame-time p50/p99/p99.9 and input-latency triples;
- per-agent accepted/error counts for scene, widget, and zone operations;
- private-memory drift `<= 5 MiB`;
- resource samples with the benchmark HUD process selected by
  `--windows-process-command-match 'C:\tze_hud\benchmark.toml'`;
- jitter/outlier classification;
- transparent-overlay composite delta and idle GPU evidence, or separate
  blocking follow-ups if those remain unavailable.

## Blocker

The remaining blocker is live validation, not source ambiguity.

Unblock condition: after the shared Windows runtime is available and not being
used by another worker, run the resident 60-minute workload above and commit a
new report/artifact set proving scene, widget, zone, and lease cleanup coverage.

Until that artifact exists, do not tag the Windows release and do not archive
`windows-first-performant-runtime`.
