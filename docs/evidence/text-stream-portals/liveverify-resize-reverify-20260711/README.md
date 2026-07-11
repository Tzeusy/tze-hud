# liveverify — whole-portal resize RE-VERIFY on first-class surface post-#1129 (2026-07-11, hud-8agm0)

Live on-device **re-verification** of whole-portal resize after #1129
(`fix(portal): wire whole-portal resize on the first-class surface path
[hud-yno2r]`). The prior run (hud-egn13, PR #1127,
`../liveverify-resize-injection-20260711/`) found pointer resize **inert**
(Δw=0) on the pre-fix build and root-caused it as a first-class-surface product
gap (finding P1). This run re-runs the **same proven harness verbatim** against a
FRESH build containing the fix, on the autonomous `windows-vm.example` HUD
testhost (single 1280×800 display), driving the resident-gRPC first-class
`PortalSurface` path.

**Headline:** the fix works — whole-portal **pointer** resize now applies end to
end via real OS pointer input (frame `860×680 → 1020×735`, whole portal scales,
transcript re-wraps, hit regions re-anchor), **closing the hud-rpmwt dynamic
re-wrap CANNOT-VERIFY axis** for the resize→re-wrap direction. A **new** distinct
gap surfaced at the *republish* step: the outer geometry holds (no snap-back) but
an adapter republish reverts the **inner** nodes (transcript wrap width + hit
regions) to declared attach-time geometry — the opposite of the unit-tested
reconcile-on-republish contract (finding **R1**). The #1109 keyboard chord is
still **injection-limited** (finding **K1**). See **`VERDICTS.md`** for
per-check verdicts and root-cause analysis.

## Contents

| Path | What |
|------|------|
| `VERDICTS.md` | Per-check verdicts, findings R1 (new) + K1, evidence table, caveats |
| `resize_injection_driver.py` | The evidence harness — **byte-identical copy** of the hud-egn13 driver (incl. the H1 hidden-console injection fix); reused verbatim |
| `run.sh` | Reproduce the run (points `--outdir` here; otherwise identical to hud-egn13) |
| `logs/exe.sha256` | sha256 of the deployed binary (fresh build, git 82790cf7, includes #1129) |
| `logs/timeline.json` | Machine timeline of every phase (connect, lease, each injection, each snapshot, verdicts, cleanup) |
| `logs/verdicts_computed.json` | Machine-computed per-check deltas + pass/fail |
| `logs/geometry.json` | Portal placement, affordance corner, focus/header points, grow deltas |
| `logs/tile_ids.json` | UUIDs of the six portal member tiles (snapshot cross-reference) |
| `snapshots/00-baseline.json` | Pristine portal at `(210,60,860,680)` |
| `snapshots/01-pointer-resized.json` | After the OS pointer-drag — **frame grows to 1020×735, whole portal scales (PASS)** |
| `snapshots/02-pointer-republish.json` | Adapter republish — outer bounds hold, **inner nodes revert to declared geometry (R1)** |
| `snapshots/03-persist.json` | After settle — outer geometry stable, no snap-back |
| `snapshots/04-kbd-resized.json` | After the #1109 Ctrl+Shift+Right chord — no change (K1) |
| `snapshots/05-kbd-republish.json` | Adapter republish after the chord |
| `snapshots/06-pre-move.json` | Immediately before the move probe |
| `snapshots/07-move-probe.json` | After the header-drag — **frame moved 210→102.4 (PASS)** |
| `snapshots/99-clean.json` | Post-cleanup — zero tiles / zero portal_surfaces |

**No screenshots committed** — GDI capture does not composite the transparent
Vulkan overlay on this software-GPU VM (as in hud-om69w / hud-egn13); the
authoritative evidence is the gRPC `SceneSnapshot` bounds.

## Default-config note

The prior deployment on this host was the bridge-routing run (hud-rw8eo). Before
this run the runtime was confirmed in DEFAULT config: `tze_hud.toml` =
`full-display` with agent-alpha registered, launched via the `TzeHudFullscreen`
task (`--window-mode fullscreen --config … --bind-all-interfaces`), and **no
`TZE_HUD_RESIDENT_GRPC_PORTAL` bridge flag** at any env scope — the bridge run
left no persistent bridge config. The fresh exe was deployed onto that same
default `TzeHudFullscreen` task; on-device sha256 == the committed
`logs/exe.sha256`.

## Hygiene

Placeholders only: `windows-vm.example` (testhost), `proxmox-host.example`
(hypervisor), `agent-alpha` / `admin-user` (registered agent / injector account).
No PSK, no real IPs/hostnames, no full-desktop frames. VM left clean (lease
released, zero tiles, injector tasks/scripts removed, default config).
