# hud-gog64.8 + hud-156qr — Authenticated 10-min media-ingress record-only soak + resource samples

- Date: 2026-06-20
- Beads: `hud-gog64.8` (10-min record-only soak), `hud-156qr` (soak acceptance evidence)
- Host: `tzehouse-windows.parrot-hen.ts.net` (`100.87.181.125`)
- HUD: isolated `TzeHud8dht5Media`, enabled config, gRPC 50052 / MCP 9092, PID 32604
- PSK: dedicated user-test PSK (`MCP_TEST_PSK`); producer matched

## Result: PASS

Authenticated synthetic producer admitted a video-only stream to `media-pip`
(`selected_codec=VIDEO_H264_BASELINE`, `stream_epoch=1`) and held it for the full
**600 s** record-only window. `producer_exit=0`, `sampler_exit=0`. HUD process
(PID 32604) stayed up for the whole run.

### Resource samples (21 valid samples @ 30 s, `resource-samples-summary.json`)

| Metric | avg | min | max |
|---|---:|---:|---:|
| CPU % (tze_hud) | 4.81 | 4.73 | 4.86 |
| GPU 3D util % (sum) | 4.48 | 2.95 | 7.17 |
| nvidia GPU util % | 12.14 | 5.0 | 24.0 |
| nvidia GPU mem used (MB) | 4282 | 4228 | 4378 |

- `private_memory_drift_bytes = -212992` (−0.2 MB) and
  `working_set_drift_bytes = -59867136` (−57 MB): **no memory growth** across the
  10-min hold — both negative, so no ingress leak.
- 20 logical processors; CPU steady ~4.8%.

This is the CPU/GPU/memory acceptance evidence that `hud-156qr` recorded as
missing. The `MEDIA_DISABLED` half of `hud-156qr` is covered separately by
`hud-8dht5` (see `docs/evidence/media-ingress/hud-8dht5/`).

## Observation (non-blocking)

The 600 s hold's `close_media_ingress` returned `close_reason =
SESSION_DISCONNECTED` (the 5 s admission smoke test returned the clean
`AGENT_CLOSED`). The stream was admitted and held the full window and all 21
samples were captured, so the record-only soak objective is met; the long-hold
close path returning `SESSION_DISCONNECTED` rather than `AGENT_CLOSED` is worth a
follow-up look at session keepalive vs. the producer's explicit close on
long-lived streams, but does not affect this soak's acceptance.

## Artifacts

- `producer-soak-evidence.json` — admitted stream, 600 s hold.
- `resource-samples-summary.json` / `resource-samples-raw.json` — CPU/GPU/mem.
- `soak-status.txt` — `sampler_exit=0 producer_exit=0`.
