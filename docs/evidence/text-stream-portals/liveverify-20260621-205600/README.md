# Live-verify — text-stream portal disconnect→resume + tools/list (2026-06-21)

On-device live capture against the **real Windows reference HUD**
(`tzehouse-windows`, Singapore tz), driven over the tailnet from Linux.

## Build / deploy provenance

- Binary built from `main` @ `aa67a6e5` (all of today's portal merges: #967 Wave-1,
  #973 disconnect/degraded UX, #977 hud-yqe79 `wait_ms`, #978 hud-h3mvo forced repaint).
- Cross-build `x86_64-pc-windows-gnu` release, sha256 `4eb655843a5c4c20970fb3ad94b8a124640a3db5d2dd66522e7b7f5d979f59ef`.
- Deployed via `user-test` portal-hud-deploy; checksum verified local==remote; relaunched
  overlay scheduled task, pid 46704, gRPC 50051 + MCP 9090 bound; MCP reachability gate passed.
- The previously-deployed exe (10:32 SGT) predated every portal merge today, so this rebuild
  was required before any live-verify could reflect shipped behavior.

## hud-yqe79 — tools/list advertises `wait_ms` (FULLY verified live)

`hud-yqe79-tools-list-wait_ms.json` — the live MCP `tools/list` response's
`portal_projection_get_pending_input` schema, captured from the running HUD, now carries the
optional `wait_ms` property (PR #977). This is the end-to-end on-hardware confirmation that the
tools/list introspection matches the real long-poll param.

## portal-disconnect-resume-ux task 5.1 — disconnect→degraded→resume (live VISUAL + continuity)

Three full-screen captures of the overlay, driven via the in-process MCP `portal_projection_*`
tools on a live `liveverify-5p1` projection:

> **Images removed 2026-07-11 (privacy):** `lv-1-baseline.png`, `lv-2-degraded.png`, and
> `lv-3-resume.png` were full-screen desktop captures that incidentally exposed the operator's
> environment (taskbar, desktop-icon inventory, and an editor window whose title bar carried a
> real tailnet hostname). They are deleted from the working tree; the phase descriptions below
> stand as the textual record. Git history retains the pre-removal blobs (history rewrite is out
> of scope). See [hud-ryawj].

| Phase | File | What it shows |
|-------|------|---------------|
| Baseline | `lv-1-baseline.png` _(removed)_ | Portal expanded, state `active ● active`, committed units A/B/C, composer ready. |
| Degraded | `lv-2-degraded.png` _(removed)_ | After `publish_status lifecycle_state=hud_unavailable`: header shows `● hudunavailable` + status line "upstream link lost"; committed transcript preserved. |
| Resume | `lv-3-resume.png` _(removed)_ | After `publish_status active` + a continuation publish: state back to `● active`, units A/B/C **persist** and new unit D appears — resume continuity with no loss and no duplication. |

### Fidelity note (read before citing)

These captures exercise the **lifecycle-state** disconnect/resume presentation path
(`portal_projection_publish_status`) — the visible degraded/active treatment and transcript
continuity — which is what a cooperative LLM session drives over MCP.

They do **not** exercise the separate `connection_degraded` latch, which is set only by an
upstream link loss (`mark_hud_disconnected`: MCP `portal_op` channel close / pure gRPC bidi
drop) and is the trigger for the additional transcript **dimming** + the hud-h3mvo forced
repaint. That latch is not reachable from a single stateless MCP HTTP call, so it is covered
**headlessly** instead:

- `disconnect_then_reconnect_within_grace_resumes_same_surface_without_duplication`
  (`crates/tze_hud_runtime/src/portal_projection_driver.rs`) — drop → degraded → reconnect →
  resume on the same tile, committed units present exactly once.
- `pure_drop_forces_degraded_repaint_without_subsequent_publish` (same file, PR #978) — a pure
  drop forces the degraded repaint within one frame without a subsequent publish.

So: live capture demonstrates the lifecycle disconnect/resume **visual + continuity** on real
hardware; the connection-latch dim/forced-repaint remains headless-verified (false-positive-free
by construction, not reachable to drive externally).

## Cleanup

Test projection `liveverify-5p1` detached (private state purged); the `TzeHudCapture` scheduled
task and temp PNGs were removed from the host. The HUD remains running on the current binary.
