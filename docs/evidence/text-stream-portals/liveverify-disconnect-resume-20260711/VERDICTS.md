# VERDICTS — live disconnect→stale→reconnect→resume (portal §5.1 / hud-om69w)

**Date:** 2026-07-11 · **Testhost:** autonomous `windows-vm.example` (Proxmox
`proxmox-host.example`), 1280×800 · **Binary:** current `main`, cross-built
`x86_64-pc-windows-gnu` release, sha256 `af6b215ccb74bb7f22f7d12099760d91a0132fc7a2b00d07ec66cab6179c75f2`
(see `logs/exe.sha256`) · **Driver:** `disconnect_resume_driver.py` (committed
here) over the **resident-gRPC first-class `PortalSurface`** path (hud-rpm9s).

Change under evidence: openspec `portal-disconnect-resume-ux`, task **5.1** —
"Add a live/integration disconnect→stale→reconnect→resume run … recording the
degraded treatment and resume continuity."

## Timeline (see `logs/timeline.json`)

1. **baseline** — SessionInit (agent `agent-alpha`), lease granted, portal built
   via `create_portal_tiles` + `publish_portal`; first-class `PortalSurface`
   declared (`SetPortalSurface`), lifecycle **Active**, transcript units A/B/C.
2. **stale** — coalescible `UpdatePortalSurfaceState(lifecycle=Degraded)` — the
   reachable staleness signal for a first-class surface; transcript preserved.
3. **drop** — `drop_connection()` closed the resident gRPC bidi channel **without
   `SessionClose`/detach and without releasing the lease** (true ungraceful drop).
4. **wait** — 20 s: the session server detects the drop by missed heartbeats,
   orphans the lease under the resume grace, and stages the resume-token entry.
5. **resume** — `SessionResume(agent_id, resume_token)` on a fresh stream →
   `accepted=true`, same lease restored; lifecycle **Active**, unit **D** appended.
6. **cleanup** — lease released; overlay verified empty.

## Snapshot parity — #1098 `SceneSnapshot.portal_surfaces` descriptors

Each phase snapshot was taken by an independent observer session (see
`snapshots/*.json`). `lifecycle` is the #1098 portal-surface descriptor field;
`units` are the committed transcript entries found in the surface's raw
transcript tile inside the same snapshot.

| phase        | `lifecycle` | `identity.session_id`          | transcript units |
|--------------|-------------|--------------------------------|------------------|
| 01-baseline  | **Active**  | `text-stream-portal-exemplar`  | A B C            |
| 02-stale     | **Degraded**| `text-stream-portal-exemplar`  | A B C            |
| 03-dropped   | **Degraded**| `text-stream-portal-exemplar`  | A B C            |
| 04-resumed   | **Active**  | `text-stream-portal-exemplar`  | A B C **D**      |
| 99-clean     | (surface removed) | —                        | —                |

`session_id` is stable across the entire disconnect boundary (identity
continuity, RFC 0013 §2.1); the resumed surface carries A/B/C **exactly once**
plus the new D (no loss, no duplication).

## Per-check verdicts

| # | Check | Verdict | Evidence |
|---|-------|---------|----------|
| 1 | Degraded/stale treatment appears on disconnect | **PASS** (descriptor-level) | `02-stale.json` lifecycle=`Degraded`, transcript preserved; timeline `stale-marked` |
| 2 | Staleness treatment persists through the transport drop | **PASS** | `03-dropped.json` lifecycle still `Degraded` after `drop_connection()`, surface + A/B/C retained under grace |
| 3 | Real disconnect **without** detaching | **PASS** | timeline `transport-dropped` (channel closed, no `SessionClose`/detach/lease-release); surface survives (check 2) |
| 4 | Reconnect resumes with transcript continuity (snapshot parity) | **PASS** | `resume accepted=true`; `04-resumed.json` lifecycle `Active`, units A/B/C **+D**, stable `session_id`, same lease |
| 5 | Resume treatment clears the stale state | **PASS** | `04-resumed.json` lifecycle back to `Active` (Degraded cleared on `record_hud_connection`) |
| 6 | Cleanup — overlay clean, no residue | **PASS** | `99-clean.json` `portal_surfaces` empty; remote captures + capture task removed; only `TzeHudFullscreen` running |
| 7 | Visual pixel-level degraded render (dim + `⊘ disconnected — stream stale` marker) | **DEFERRED — not capturable on this host** | see caveat below; render-verified headlessly in `crates/tze_hud_compositor/src/renderer/tile_render.rs` |

## Fidelity caveats (read before citing)

**A. Degraded is driven cooperatively, not by the transport drop — by design.**
Dropping the resident gRPC bidi stream does **not** auto-flip the surface to
Degraded. Per `hud-b2llg` (closed **wontfix**), the resident gRPC bidi stream-end
is a recoverable mirror-reconnect blip (bounded backoff + state replay), not an
authoritative per-portal degrade trigger; there is no per-session→projection
ownership map on that path. The authoritative degrade trigger is the MCP
`portal_op` channel close (`hud-5i16d`) + forced repaint (`hud-h3mvo`). For a
**first-class surface** the reachable staleness signal is
`UpdatePortalSurfaceState(lifecycle=Degraded)`, which this run drives in phase 2.
This is consistent with the spec model (§1.4: staleness is bounded by the
existing lease grace, not a second timer authority) and with the 20260621 package
fidelity note. The run then performs the **real** transport drop (phase 3) to
prove persistence-under-grace + `SessionResume` continuity — the part a stateless
MCP call cannot exercise.

**B. No portal-region screenshots are committed — visual capture is unreliable on
this VM.** Full-desktop `CopyFromScreen` (GDI) on this software-GPU Proxmox VM
captured the foreground diagnostic console, not the transparent Vulkan overlay,
and `hud-diag.log` shows the compositor present loop repeatedly
`EXITED … (HB-2)` then restarting — i.e. the VM cannot reliably present/capture
the layered overlay. The four captured frames showed no portal (and were
full-desktop, so forbidden to commit); they were deleted locally and on the VM.
The **scene-descriptor** evidence in this package (the #1098
`portal_surfaces[].lifecycle` the task names) is the authoritative machine-checked
parity source and is complete. The **pixel-level** degraded treatment (transcript
dim + `⊘ disconnected — stream stale` amber marker + composer suppression) stays
render-verified by the headless compositor tests in
`crates/tze_hud_compositor/src/renderer/tile_render.rs` and the driver tests
`disconnect_then_reconnect_within_grace_resumes_same_surface_without_duplication`
/ `pure_drop_forces_degraded_repaint_without_subsequent_publish`
(`crates/tze_hud_runtime/src/portal_projection_driver.rs`).

**Net:** the live, on-device run confirms — over the real resident-gRPC transport
on a fresh-main binary — the full disconnect→stale→reconnect→resume **state**
lifecycle and transcript continuity with #1098 snapshot parity (checks 1–6 PASS).
The visual render treatment is host-limited here and remains headless-verified
(check 7 DEFERRED).

## Reproduce

`./run.sh` after `eval "$(…/hud_vm_env.sh)"; export TZE_HUD_PSK="$HUD_MCP_PSK"
TZE_HUD_GRPC_TARGET="$TZE_HUD_TEST_HOST:50051"`. See `run.sh` header. Add
`SCREENSHOTS=1` to also drive the (host-limited) capture path.
