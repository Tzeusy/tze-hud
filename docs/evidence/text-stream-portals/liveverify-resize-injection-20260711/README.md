# liveverify — OS-injection whole-portal resize (2026-07-11, hud-egn13)

Live on-device evidence for the **§6b.7 live phase** of hud-5jbra.9: inject real
Windows OS pointer/keyboard events into a running text-stream portal and verify
whole-portal resize geometry end to end. Historically deferred (tzehouse
unreachable, then its 3-monitor layout put the resize handle out of the
injector's reach). Run here against the autonomous `windows-vm.example` HUD
testhost — a **single 1280×800 display**, so the bottom-right resize affordance
is pointer-reachable — driving the resident-gRPC first-class `PortalSurface`
path (hud-rpm9s) on a current-`main` binary.

**Headline:** real OS input reaches and drives the HUD end to end once a
test-harness blocker (the injector's own foreground console) is removed — the
whole-portal **move** applies and is gRPC-verified. But whole-portal **resize**
(pointer and #1109 keyboard) **does not apply** via real OS input: the
pointer-resize failure is a **genuine product gap** the headless suite does not
cover (VERDICTS finding **P1**); the keyboard failure is **injection-limited**
(**K1**). See **`VERDICTS.md`** for per-check verdicts, root-cause analysis, and
caveats.

## Contents

| Path | What |
|------|------|
| `VERDICTS.md` | Per-check verdicts, findings H1/P1/K1 (root-caused), caveats |
| `resize_injection_driver.py` | The evidence harness (reuses the tracked exemplar building blocks; adds the resize/move/keychord control flow + the console-hide injection fix) |
| `run.sh` | Reproduce the run |
| `logs/timeline.json` | Machine timeline of every phase (connect, lease, each injection, each snapshot, verdicts, cleanup) |
| `logs/verdicts_computed.json` | Machine-computed per-check deltas + pass/fail |
| `logs/geometry.json` | Portal placement, affordance corner, focus/header points, grow deltas |
| `logs/tile_ids.json` | UUIDs of the six portal member tiles (for snapshot cross-reference) |
| `logs/exe.sha256` | sha256 of the deployed binary (current `main`) |
| `snapshots/00-baseline.json` | Pristine portal at `(210,60,860,680)` |
| `snapshots/01-pointer-resized.json` | After the OS pointer-drag on the frame corner — **no bounds change (P1)** |
| `snapshots/02-pointer-republish.json` | Adapter republish — transcript width unchanged (blocked by P1) |
| `snapshots/03-persist.json` | After settle — declared geometry stable, no snap-back |
| `snapshots/04-kbd-resized.json` | After the #1109 Ctrl+Shift+Right chord — no change (K1) |
| `snapshots/05-kbd-republish.json` | Adapter republish after the chord |
| `snapshots/06-pre-move.json` | Immediately before the move probe |
| `snapshots/07-move-probe.json` | After the header-drag — **frame moved `210,60`→`102.4,96` (PASS)** |
| `snapshots/99-clean.json` | Post-cleanup — zero tiles / zero portal_surfaces |

**No screenshots committed** — GDI capture does not composite the transparent
Vulkan overlay on this software-GPU VM (as in hud-om69w), and the resize under
test never applied. The authoritative evidence is the gRPC `SceneSnapshot`
bounds captured above.

## The key methodology fix (finding H1)

The interactive scheduled-task injector launches a **visible PowerShell console**
that grabs the foreground and intercepts all synthetic input before it reaches
the transparent HUD overlay. The harness hides that console
(`ShowWindow(SW_HIDE)`) at the top of every injected script; only then does OS
input reach the HUD (proven by the move probe). Prior OS-injection "successes"
(hud-ofe76) asserted only that `SendInput` returned, never that the portal
reacted — so this blocker was latent. Future OS-injection evidence should adopt
the console-hide and assert an end-to-end state change, not just injector exit
code.

## Hygiene

Placeholders only: `windows-vm.example` (testhost), `proxmox-host.example`
(hypervisor), `agent-alpha` / `admin-user` (registered agent / SSH user). No PSK,
no real IPs/hostnames, no full-desktop frames. VM left clean.
