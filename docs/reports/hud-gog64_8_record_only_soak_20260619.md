# hud-gog64.8 Windows Media Ingress Record-Only Soak

Date: 2026-06-19
Host: TzeHouse (`tzehouse-windows.parrot-hen.ts.net`)
Worker branch: `agent/hud-gog64.8`

## Result

Blocked for acceptance closeout.

The authenticated local producer did successfully connect as
`windows-local-media-producer`, open the approved `media-pip` media-ingress
surface on isolated gRPC `50052`, hold the session for 10 minutes, and close the
stream normally. Production `TzeHudOverlay` was restored after each isolated
run and verified listening on `50051/9090`.

Two acceptance items remain incomplete:

- CPU/GPU/memory samples for the 10-minute interval were not captured. The final
  producer run succeeded, but the local sampler produced `sample_count=0`.
- The `operator_disabled=true` config did not produce an in-protocol
  `MEDIA_DISABLED` admission rejection. The disabled isolated HUD failed to bind
  `50052`, so the producer failed at connection time instead.

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
- Target: `tzehouse-windows.parrot-hen.ts.net:50052`
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

The final producer run succeeded, but the resource sampler failed to capture
samples:

```text
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/resource-samples-local-summary.json
docs/reports/artifacts/hud-gog64.8-record-only-soak-20260619T113954Z/soak-metrics-command-status.json
```

Observed:

- `producer_status=0`
- `sample_count=0`
- `valid_sample_count=0`

Earlier sampler attempts are preserved in the artifact directory. They either
emitted empty output or failed before writing usable JSON. Because of this, the
CPU/GPU/memory acceptance evidence is not complete.

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

Remote cleanup:

- `remote-cleanup-metrics.txt`: `cleanup_ok`
- `remote-cleanup-disabled.txt`: `cleanup_ok`

No PSK value is printed in any committed artifact; recovery steps record only
that the value was recovered from scheduled-task XML and intentionally omitted.

## Follow-Ups For Coordinator

```json
[
  {
    "title": "Capture reliable Windows media-ingress CPU/GPU/memory samples during 10-minute soak",
    "type": "bug",
    "priority": 1,
    "depends_on": "hud-gog64.8",
    "rationale": "The authenticated 10-minute producer hold succeeded, but all resource sampler attempts produced zero usable samples, leaving CPU/GPU/memory acceptance evidence incomplete."
  },
  {
    "title": "Make operator-disabled media ingress bind and reject with MEDIA_DISABLED",
    "type": "bug",
    "priority": 1,
    "depends_on": "hud-gog64.8",
    "rationale": "Starting the media HUD with operator_disabled=true did not bind gRPC 50052, so the producer observed connection failure instead of an authenticated MEDIA_DISABLED admission rejection."
  }
]
```
