# 60-min LAN-local text-stream portal soak — hud-5kq8k

**Result: PASS** — clean full-duration (3600 s) completion with no memory drift.

Date: 2026-07-10
Target: **hud-windows VM** (vmid 110 on `proxmox-host.example` Proxmox) — LAN/tailnet-local, `windows-vm.example:50051` (gRPC) / `:9090` (MCP).
Render: WARP software rendering, `--window-mode fullscreen`, scene 1280x800 (no discrete GPU — functional/protocol validation only, not a fidelity/perf run).
Build: local prebuilt `x86_64-pc-windows-gnu/release/tze_hud.exe` (len 24 627 924), worktree HEAD `345e7c39` (origin/main − 1; all portal fixes through hud-rpmwt present). SCP-deployed to `C:\tze_hud\tze_hud.exe` before the run.
Driver: `text_stream_portal_exemplar.py` `--phases soak` (has #1010 lease renewal, #1013 authoritative marker, hud-n5bqp bounded mutation-ack retry). Full invocation in `harness-invocation.txt`.

## Why this run exists

hud-sonj6 task 5.5 required a 60-min sustained-streaming soak with ≤5 MiB drift.
On tzehouse (remote, over WAN) the **drift budget passed** (RSS flat 27–34 MiB
across 57 min) but the run **aborted at 3453.9 s / cycle 13303** on a single
`Timed out waiting for mutation_result` — a WAN transport stall ~2.4 min short of
3600 s, not a runtime hang. hud-sonj6 AC#4 split the full-duration completion off
into this bead: rerun where transport RTT is not internet-bound.

## Result

| Axis | Criterion | Observed | Verdict |
|---|---|---|---|
| Full duration | reach 3600 s | **3600.2 s, 13 911 cycles** | **PASS** |
| Authoritative marker | `soak-complete.marker` (SOAK_COMPLETE) written | present in `soak-markers/` | **PASS** |
| Memory drift | ≤ 5 MiB | steady-state net **−4 MiB** (34→30), slope **−0.42 MiB/hr** | **PASS** |
| Lease self-termination | none (fixed by #1010/hud-hk8kl) | lease renewed for the full run, released cleanly | **PASS** |
| Transient robustness | survive mutation-ack blips (hud-n5bqp) | **0** timeout/retry/backpressure/error across 13 911 cycles | **PASS** |

The WAN-stall failure mode from tzehouse **did not recur** on LAN: zero transient
mutation-ack timeouts over the whole 60 min, so the bounded-retry path was never
even exercised. This is the clean full-duration artifact hud-sonj6 AC#4 asked for.

## Memory drift detail (independent HUD WorkingSet64 sampling, every ~120 s)

Full series in `soak-hud-rss.log` (30 samples). The HUD allocates during
startup + first tile setup (63 MiB @ t=0, 61 MiB @ 121 s), then settles into a
steady state by ~5 min and stays there:

- Steady-state (elapsed ≥ 300 s, n=27): min 30 / max 37 MiB, mean 31.9 MiB.
- **Net drift** first→last of steady state: **−4 MiB** (34 → 30 — memory went *down*).
- Linear trend: **−0.42 MiB/hour** → flat, no leak.
- The ~7 MiB steady-state band is instantaneous WARP working-set jitter, not
  monotonic growth. "Drift" = sustained growth; there is none. Matches the
  tzehouse observation (flat 27–34 MiB) on the axis that already passed there.

## Setup notes (reproducibility)

- `agent-alpha` toml capabilities temporarily extended with `read_telemetry`
  (lease-scope requirement, same as the tzehouse soak); config backed up to
  `tze_hud.toml.pre-soak-5kq8k.bak` and **restored to as-found after the run**.
- MCP `create_tab {"name":"Main"}` called once before the run (hud-d5rcd
  workaround: config `[[tabs]]` don't materialize without widgets).
- Cleanup: exemplar released the lease + removed all portal tiles on exit; toml
  restored; HUD left running (its default scheduled-task state). VM clean.

## Artifacts

- `soak.log` — full exemplar stdout (27 988 lines; session, lease grant/renew, per-cycle tile ops, completion).
- `soak-transcript.json` — structured step transcript.
- `soak-hud-rss.log` — independent HUD WorkingSet64 series with elapsed timestamps.
- `soak-markers/soak-complete.marker` — authoritative full-duration completion marker (SOAK_COMPLETE).
- `harness-invocation.txt` — exact command + environment.
- `run-state.txt` — driver pid + exit code (`soak_exit=0`).
