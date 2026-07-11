# liveverify — disconnect→stale→reconnect→resume (2026-07-11, hud-om69w)

Live on-device evidence for openspec `portal-disconnect-resume-ux` **task 5.1**,
run against the autonomous `windows-vm.example` HUD testhost on a fresh
current-`main` binary, driving the **resident-gRPC first-class `PortalSurface`**
path (hud-rpm9s) end to end.

**Headline result:** over the real resident-gRPC transport, the surface lifecycle
walks `Active → Degraded → Degraded (survives an ungraceful channel drop under
lease grace) → Active`, and a `SessionResume` within grace restores the same
lease and resumes the transcript — units A/B/C persist exactly once and unit D is
appended (no loss, no duplication), with a stable `session_id` across the
disconnect boundary. See **`VERDICTS.md`** for per-check PASS/FAIL and caveats.

## Contents

| Path | What |
|------|------|
| `VERDICTS.md` | Per-check verdicts, the #1098 snapshot-parity table, and fidelity caveats |
| `logs/timeline.json` | Machine timeline of every phase (connect, lease, publish, stale, drop, resume, cleanup) |
| `logs/exe.sha256` | sha256 of the deployed binary (built from current `main`) |
| `logs/crop_box.json` | Portal-region crop box used by the (host-limited) capture path |
| `snapshots/01-baseline.json` | Observer `SceneSnapshot` — lifecycle `Active`, units A/B/C |
| `snapshots/02-stale.json` | lifecycle `Degraded` (staleness), transcript preserved |
| `snapshots/03-dropped.json` | after the ungraceful transport drop — surface persists, still `Degraded` |
| `snapshots/04-resumed.json` | after `SessionResume` — lifecycle `Active`, units A/B/C **+D** |
| `snapshots/99-clean.json` | post-cleanup — `portal_surfaces` empty |
| `disconnect_resume_driver.py` | The evidence harness (reuses the tracked exemplar building blocks; adds `connect_init`/`resume`) |
| `crop_portal_region.py` | Crops full-desktop captures to the portal region before commit |
| `run.sh` | Reproduce the run |

**No screenshots are committed.** Visual capture is unreliable on this
software-GPU Proxmox VM (GDI capture does not composite the transparent Vulkan
overlay; the compositor present loop is unstable — see `VERDICTS.md` §caveat B).
The authoritative parity source the task names is the #1098
`SceneSnapshot.portal_surfaces[].lifecycle` descriptor, captured here; the
pixel-level degraded render stays headless-verified in
`crates/tze_hud_compositor/src/renderer/tile_render.rs`.

## Hygiene

Placeholders only: `windows-vm.example` (testhost), `proxmox-host.example`
(hypervisor), `agent-alpha`/`admin-user` (registered agent / SSH user). No PSK,
no real IPs/hostnames, no full-desktop frames.
