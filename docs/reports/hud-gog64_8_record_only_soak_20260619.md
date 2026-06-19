# hud-gog64.8 Windows Media Ingress Record-Only Soak

Date: 2026-06-19
Host: TzeHouse (`windows-host.example`)
Worker branch: `agent/hud-gog64.8`

## Result

Resource-sampling acceptance evidence captured.

The authenticated local producer did successfully connect as
`windows-local-media-producer`, open the approved `media-pip` media-ingress
surface on isolated gRPC `50052`, hold the session for 10 minutes, and close the
stream normally. Production `TzeHudOverlay` was restored after each isolated
run and verified listening on `50051/9090`.

The `hud-ith1h` follow-up completed the missing CPU/GPU/memory evidence for the
10-minute sampled soak. One acceptance item remains outside `hud-ith1h` scope:

- The `operator_disabled=true` config did not produce an in-protocol
  `MEDIA_DISABLED` admission rejection. The disabled isolated HUD failed to bind
  `50052`, so the producer failed at connection time instead.

## hud-ith1h Resource-Sampler Follow-Up

Worker branch: `agent/hud-ith1h`

The sampler path was repaired further after the first follow-up attempt. The
prior fix avoided the copied-script `param(...)` parse failure, but the full
generated PowerShell sampler exceeded the Windows SSH command-line limit when
sent as `-EncodedCommand`:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-samples.stderr
```

The sampler now streams the generated PowerShell over SSH stdin with
`powershell -Command -`, avoiding both the copied-script param-block regression
and the encoded-command length ceiling. The long-run timeout was also widened
after a 21-sample, 30-second interval pass hit the previous 670-second local
timeout after the producer had already completed. Focused validation passed:

```text
python3 -m unittest discover -s .claude/skills/user-test/tests -p 'test_windows_media_resource_sampler.py'
python3 -m py_compile .claude/skills/user-test/scripts/windows_media_resource_sampler.py .claude/skills/user-test/tests/test_windows_media_resource_sampler.py
```

A short sampler smoke against restored production `50051` captured usable
process CPU/GPU/memory samples:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-samples-production-smoke-summary.json
```

Summary:

- `sample_count=2`
- `valid_sample_count=2`
- CPU percent count: `1`
- GPU 3D utilization count: `2`
- NVIDIA GPU utilization count: `2`
- NVIDIA GPU memory count: `2`

The final resource-sampled pass used the stale-lock-safe isolated launch
sequence from `hud-5b6jc`: wait for the stopped production PID to exit, then
remove only a GPU lock that still names that dead PID. The isolated HUD bound
`50052/9092` as PID `66804`; startup removed the verified stale production lock
for PID `37296`.

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/start-resource-sampled-media-hud-final.json
```

Final sampled soak evidence:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-sampled-soak-final-command-status.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/producer-soak-resource-sampled-final-evidence.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-samples-10min-final-summary.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-samples-10min-final-raw.json
```

Summary:

- Producer status: `0`
- Sampler status: `0`
- Producer hold observed: `600.583 s`
- Admission: `admitted=true`
- Time to admission: `409.373 ms`
- Close reason: `AGENT_CLOSED`
- Dropped frames reported by live `MediaIngressState`: `0`
- Sampler interval: 21 samples at 30-second spacing
- Valid process/GPU/memory samples: `21/21`
- CPU percent: avg `4.803`, max `4.869`
- GPU 3D utilization sum: avg `4.713`, max `5.585`
- NVIDIA GPU utilization: avg `12.524`, max `18`
- NVIDIA GPU memory used: avg `5781.286 MiB`, max `5827 MiB`
- Private memory drift: `+3,522,560 bytes`
- Working set drift: `-11,677,696 bytes`
- Sampler errors: none

The sampler covered the producer's 10-minute window and continued through the
tail while the isolated HUD remained running. The first valid sample observed
the isolated HUD PID `66804` at `2026-06-19T14:14:34.8551258Z`; the last valid
sample observed the same PID at `2026-06-19T14:26:33.1640032Z`.

Evidence directory:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/
```

## Successful Record-Only Soak

Primary evidence:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/producer-soak-metrics-evidence.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/start-metrics-media-hud.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/restore-after-metrics-soak.json
```

Summary:

- Agent: `windows-local-media-producer`
- Target: `windows-host.example:50052`
- Zone: `media-pip`
- Source label: `synthetic-color-bars`
- Admission: `admitted=true`
- Time to admission: `1120.447 ms`
- Hold duration observed: `601.209 s`
- Close reason: `AGENT_CLOSED`
- Dropped frames reported by live `MediaIngressState`: `0`
- Heartbeat errors: none
- Final state: `MEDIA_SESSION_STATE_CLOSED`

First-frame timing is recorded as `null`: this record-only slice opens a
video-only `MediaIngressOpen` surface but does not activate decoded-frame
transport. Live state samples reported `effective_fps=0`,
`effective_width_px=0`, and `effective_height_px=0` throughout.

## Resource Metrics Gap

Resolved by `hud-ith1h`.

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-samples-10min-final-summary.json
```

Observed:

- `producer_status=0`
- `sampler_status=0`
- `sample_count=21`
- `valid_sample_count=21`

Earlier sampler attempts are preserved in the artifact directory. They either
emitted empty output, hit the encoded-command length ceiling, failed before the
stale-lock-safe isolated launch fix, or timed out with the old 670-second local
timeout after the producer had already completed.

## Operator-Disable Behavior

The disabled config used:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/windows-media-ingress-operator-disabled.toml
```

Expected behavior was a running HUD that rejects `MediaIngressOpen` with
`MEDIA_DISABLED`. Actual behavior:

- Temporary task recovered the PSK and stopped production.
- The disabled media HUD did not bind gRPC `50052`.
- The producer failed with process status `1` because there was no listener.
- Production was restored afterward.

Primary evidence:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/start-disabled-media-hud.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/operator-disabled-command-status.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/restore-after-disabled-proof.json
```

This does not satisfy the intended operator-disable acceptance check because it
does not prove an authenticated producer receives `MEDIA_DISABLED`.

## Recovery And Cleanup

Production restoration after the final metrics pass:

- `restore-after-metrics-soak.json`: production restored on `50051/9090`, PID
  `56632`, GPU lock returned to production.

Production restoration after disabled proof:

- `restore-after-disabled-proof.json`: production restored on `50051/9090`, PID
  `58524`, GPU lock returned to production.

Production restoration after final resource-sampled soak:

- `restore-after-resource-sampled-final-soak.json`: isolated PID `66804` exited,
  stale isolated GPU lock for PID `66804` was removed, and production restored
  on `50051/9090` as PID `55548`.
- `final-production-port-snapshot-after-resource-sampled-final.json`: only
  production listeners `50051/9090` remained after restoration.

Remote cleanup:

- `remote-cleanup-metrics.txt`: `cleanup_ok`
- `remote-cleanup-disabled.txt`: `cleanup_ok`

No PSK value is printed in any committed artifact; recovery steps record only
that the value was recovered from scheduled-task XML and intentionally omitted.

## Follow-Ups For Coordinator

```json
[
  {
    "title": "Make operator-disabled media ingress bind and reject with MEDIA_DISABLED",
    "type": "bug",
    "priority": 1,
    "depends_on": "hud-gog64.8",
    "rationale": "Starting the media HUD with operator_disabled=true did not bind gRPC 50052, so the producer observed connection failure instead of an authenticated MEDIA_DISABLED admission rejection."
  }
]
```
