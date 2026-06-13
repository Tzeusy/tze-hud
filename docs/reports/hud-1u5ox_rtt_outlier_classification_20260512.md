# hud-1u5ox RTT Outlier Classification - 2026-05-12

Issue: `hud-1u5ox`
Source soak: `hud-nfl7n`
Artifact root: `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/`

## Verdict

The 3.8 second max RTT samples are best classified as one shared transient stall
in the benchmark path, not as shutdown/drain artifacts and not as sustained
material jitter.

Release latency confidence for the 60-minute widget workload remains valid for
the reported p99 RTT envelope: all three agents completed `3600/3600` publishes
with zero errors, and p99 stayed below 58 ms. The outliers do limit any claim
about max-tail latency because the saved artifacts do not include per-request
sequence/timestamp data or runtime-side ack timing that would identify whether
the pause occurred on the client host, tailnet transport, Windows host, or
runtime response drain.

## Evidence

| Agent | Requests | Success | Errors | p50 RTT | p95 RTT | p99 RTT | Max RTT | Ack drain after final send |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `agent-alpha` | 3600 | 3600 | 0 | 10,676 us | 26,107 us | 57,690 us | 3,831,916 us | 9,803 us |
| `agent-beta` | 3600 | 3600 | 0 | 10,648 us | 25,914 us | 53,925 us | 3,831,298 us | 9,769 us |
| `agent-gamma` | 3600 | 3600 | 0 | 10,488 us | 24,617 us | 47,467 us | 3,835,304 us | 9,948 us |

The three max RTT values are separated by only 4,006 us. Independent per-agent
outliers landing in the same narrow 4 ms band during a 3,600 second run is much
more consistent with one shared pause than with unrelated request-level jitter.

The shutdown path does not explain these maxima:

- Each agent recorded `success_count == request_count == 3600` and
  `error_count == 0`.
- `aggregate_ack_drain_time_us` was only 9.7-10.0 ms after the send loop
  finished, so all expected `WidgetPublishResult` acks arrived before
  `SessionClose`.
- The warning `stream closed while waiting for WidgetPublishResult acks` is the
  normal response-drain terminal condition after `SessionClose`, not evidence of
  missing late acks in this run.

The p99 path remained clean:

- p99 RTT was 47.5-57.7 ms, at least 3.77 seconds below each max sample.
- The harness percentile function selects p99 from the sorted 3,600-sample RTT
  vector, so a single max sample per agent cannot influence p99 unless the tail
  is broad. The saved p95/p99 values show the tail was not broad.
- The live compositor benchmark artifact was also present and reported
  `live_metrics.ok=true` with no missing frame/input metrics.

## Classification

| Candidate | Classification | Rationale |
|---|---|---|
| Shutdown/drain artifact | Ruled out | All expected acks were received before close; final ack drain was about 10 ms; the terminal stream-close warning was recorded after `SessionClose`. |
| Transient host/benchmark-path stall | Most likely | The nearly identical max RTTs across three concurrent agents point to one shared pause in the client/runtime/network path. Existing artifacts cannot localize the pause further. |
| Material jitter | Not supported for p99 release confidence | p95 and p99 stayed low, errors were zero, and the outlier was isolated to the max tail. It is material only for max-tail claims, which this artifact cannot certify. |

## Residual Attribution Gap

The existing artifacts are sufficient to classify the outliers at release-gate
level, but not to root-cause the exact component that paused. Missing telemetry:

- Per-request `request_sequence`, send timestamp, ack timestamp, and RTT samples.
- A histogram or tail sample file; the harness writes `histogram_path: null`.
- Runtime-side timestamp for each `WidgetPublishResult`.
- Host scheduler / network samples at sub-5-second resolution around the outlier.

Future long soaks should emit a bounded top-N RTT tail file with request
sequence, send/ack monotonic timestamps, and wall-clock approximations. That
would turn this classification from shared-stall inference into direct
localization without rerunning a full soak.

## Release Impact

For `hud-1u5ox`, the RTT outlier follow-up can be treated as classified:

- It does not block confidence in the reported p99 RTT envelope for this
  1 rps, three-agent widget soak.
- It does block any stronger statement that max RTT stayed below a low-latency
  budget during the 60-minute run.
- Other `hud-nfl7n` blockers remain separate: overlay composite cost, idle GPU
  budget evidence, and scene/zone/lease coverage.

