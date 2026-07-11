#!/usr/bin/env python3
"""Live OS-injection whole-portal resize driver (§6b.7 / bead hud-egn13).

Injects **actual Windows OS pointer and keyboard events** into a running
first-class text-stream portal on the autonomous ``windows-vm.example`` HUD
testhost (single 1280x800 display, so the bottom-right resize affordance IS
pointer-reachable — the multi-monitor blocker from the 2026-07-11 motion sweep
does not apply here) and verifies the whole-portal resize applies end to end.

This is an EVIDENCE harness, not product code. It reuses the tracked
``text_stream_portal_exemplar`` building blocks (the ``HudClient`` resident-gRPC
transport, the six-tile portal assembly, the first-class ``PortalSurface``
declaration, and the scheduled-task OS-input injector) and adds the resize
control flow the exemplar's phase machinery does not script.

Runtime facts that shape the harness (crates/tze_hud_runtime/src/windowed):
  * The overlay is click-through by default; ``set_cursor_hittest`` flips to
    capture only when the OS cursor is over an active hit-region OR (for resize)
    over a **focused** portal's affordance band — recomputed per render frame
    (hittest.rs). So the portal MUST be focused before the resize affordance can
    capture OS pointer input.
  * ``apply_portal_resize_pointer_event`` (portal.rs) gates pointer resize on a
    focused **scrollable** portal tile, resolves the whole portal group, and
    tests ``hit_affordance`` against the **frame** rect with
    ``window_resize_affordance_px`` (default 8px). BottomRight band on an
    860x680 frame at (210,60) => x in [1062,1070], y in [732,740].
  * The header drag-handle capture is snapshot-wide (NOT focus-gated), so a
    header-drag MOVE is a clean focus-independent probe of whether OS pointer
    reaches the portal at all.
  * ``viewer_geometry_locked`` is ``#[serde(skip)]`` — NOT in the snapshot. Its
    consequence (a post-resize republish that re-wraps to the NEW bounds instead
    of snapping back to the declared pane) is the observable proof the lock held.

Checks (evidence == gRPC SceneSnapshot Tile.bounds + TextMarkdown node bounds):
  1. focus a scrollable pane, then pointer-drag the frame's bottom-right
     affordance -> every portal member tile's bounds grow (whole-portal resize).
  2. post-resize adapter republish -> the transcript markdown node re-wraps to
     the NEW output-pane width (hud-rpmwt; proves the geometry-lock held).
  3. #1109 keyboard resize: focus a NON-composer pane, inject Ctrl+Shift+Right
     (virtual-key chord) -> same grow + re-wrap. Recorded injection-limited if
     the chord does not land.
  4. persistence: resized bounds do not snap back and the resize band stays
     aligned to the new corner.
  M. move probe (focus-independent): header-drag -> frame tile x/y changes,
     confirming OS pointer reaches and manipulates the portal.

Run: see ``run.sh``. Requires the resident gRPC port reachable, an SSH-reachable
VM for OS injection, and ``TZE_HUD_PSK`` in the environment.
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import json
import os
import sys
import time
import uuid
from typing import Any, Optional

_REPO_ROOT = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "..")
)
_SCRIPTS = os.path.join(_REPO_ROOT, ".claude", "skills", "user-test", "scripts")
sys.path.insert(0, _SCRIPTS)
sys.path.insert(0, os.path.join(_SCRIPTS, "proto_gen"))

from hud_grpc_client import HudClient  # noqa: E402
import text_stream_portal_exemplar as ex  # noqa: E402


BODY_MARKER = "REWRAP-PROBE-EGN13"
LONG_PARA = (
    f"{BODY_MARKER}: this is a deliberately long single paragraph of transcript "
    "content whose soft-wrap column is a direct function of the output pane "
    "width, so that a whole-portal resize that widens the pane forces the "
    "markdown adapter to re-wrap this text to a wider column on the next "
    "republish — the observable end-to-end consequence of the viewer geometry "
    "lock being taken by the resize commit across every portal group member."
)
BASELINE_BODY = ex.join_transcript_entries(
    ["**[unit A]** portal established — baseline transcript", LONG_PARA]
)

TITLE = "liveverify · OS-injection resize"
SUBTITLE = "portal §6b.7 · hud-egn13"
FOOTER = "resident-gRPC first-class surface"


# The interactive scheduled task launches a visible powershell.exe console that
# becomes the foreground/topmost window and INTERCEPTS all synthetic pointer +
# keyboard input before it can reach the transparent HUD overlay (verified: with
# the console visible, WindowFromPoint over every portal coordinate returns the
# injector's own console, and no gesture reaches the runtime). Hiding the console
# first (ShowWindow SW_HIDE) lets OS input reach the HUD end to end. This is the
# key fix over the exemplar's diagnostic-input path, whose `ok:true` only ever
# meant "SendInput returned", never "the portal reacted" (hud-ofe76).
HIDE_CONSOLE_PS = (
    "Add-Type -Name Win -Namespace Native -MemberDefinition "
    "'[DllImport(\"kernel32.dll\")] public static extern IntPtr GetConsoleWindow(); "
    "[DllImport(\"user32.dll\")] public static extern bool ShowWindow(IntPtr h, int n);'\n"
    "$hudConsole = [Native.Win]::GetConsoleWindow(); "
    "[void][Native.Win]::ShowWindow($hudConsole, 0)\n"
    "Start-Sleep -Milliseconds 400\n"
)


def _now_us() -> int:
    return int(time.time() * 1_000_000)


class suppress_all:
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return True


async def observe_snapshot(target: str, psk: str, label: str) -> Optional[dict]:
    obs = HudClient(target, psk, agent_id=f"resize-observer-{label}")
    raw = None
    try:
        await obs.connect()
        raw = obs.scene_snapshot_json
    finally:
        with suppress_all():
            await obs.disconnect(graceful=True)
    if not raw:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def _tile_id_str(tile_id: bytes) -> str:
    return str(uuid.UUID(bytes=tile_id))


def _rect_of(b: Optional[dict]) -> Optional[dict]:
    if not b:
        return None
    return {
        "x": round(float(b.get("x", 0.0)), 1),
        "y": round(float(b.get("y", 0.0)), 1),
        "w": round(float(b.get("width", 0.0)), 1),
        "h": round(float(b.get("height", 0.0)), 1),
    }


def _tile_rect(t: dict) -> Optional[dict]:
    return _rect_of(t.get("bounds"))


def _node_bounds(nval: dict) -> Optional[dict]:
    """Node bounds live at data.<Variant>.bounds, not top-level."""
    data = nval.get("data")
    if not isinstance(data, dict):
        return None
    for variant in data.values():
        if isinstance(variant, dict) and "bounds" in variant:
            return _rect_of(variant["bounds"])
    return None


def summarize(snapshot: Optional[dict], tile_ids: dict[str, str]) -> dict:
    out: dict[str, Any] = {"tiles": {}, "transcript_node": None, "resize_hit_node": None}
    if not snapshot:
        return out
    tiles = snapshot.get("tiles") or {}
    id_to_role = {v: k for k, v in tile_ids.items()}
    for key, tval in tiles.items():
        tid = str(tval.get("id") or key)
        role = id_to_role.get(tid) or id_to_role.get(str(key))
        if role:
            out["tiles"][role] = {"bounds": _tile_rect(tval), "z": tval.get("z_order")}
    nodes = snapshot.get("nodes") or {}
    best = None
    for nid, nval in nodes.items():
        blob = json.dumps(nval)
        if BODY_MARKER in blob:
            r = _node_bounds(nval)
            if r and (best is None or r["w"] > best["bounds"]["w"]):
                best = {"node": str(nid), "bounds": r,
                        "content_len": len((((nval.get("data") or {}).get("TextMarkdown") or {}).get("content") or ""))}
        if "portal-resize-bottom-right" in blob:
            out["resize_hit_node"] = {"node": str(nid), "bounds": _node_bounds(nval)}
    out["transcript_node"] = best
    return out


def emit(timeline: list, phase: str, detail: dict) -> None:
    entry = {"t_wall_us": _now_us(), "phase": phase, **detail}
    timeline.append(entry)
    print(f"[timeline] {phase}: {json.dumps(detail)[:400]}", flush=True)


# ── Virtual-key chord injection (Ctrl+Shift+ArrowRight, #1109) ─────────────────
# NB: PowerShell has no [ushort] accelerator — use [uint16]. Send-Text's
# KEYEVENTF_UNICODE path (exemplar) is unreliable for chords; a modifier chord
# needs virtual-key SendInput. VK_CONTROL=0x11, VK_SHIFT=0x10, VK_RIGHT=0x27.
def keychord_input_script(vks: list[int], *, repeat: int) -> str:
    lines = [
        "$ErrorActionPreference = 'Stop'",
        "Add-Type -TypeDefinition @\"",
        "using System;",
        "using System.Runtime.InteropServices;",
        "public static class HudKeyChord {",
        "  [DllImport(\"user32.dll\", SetLastError=true)] public static extern uint SendInput(uint n, INPUT[] p, int cb);",
        "  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION U; }",
        "  [StructLayout(LayoutKind.Explicit)] public struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; [FieldOffset(0)] public HARDWAREINPUT hi; }",
        "  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public UIntPtr dwExtraInfo; }",
        "  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public UIntPtr dwExtraInfo; }",
        "  [StructLayout(LayoutKind.Sequential)] public struct HARDWAREINPUT { public uint uMsg; public ushort wParamL; public ushort wParamH; }",
        "}",
        "\"@",
        "$INPUT_KEYBOARD = 1",
        "$KEYEVENTF_KEYUP = 0x0002",
        "$InputSize = [System.Runtime.InteropServices.Marshal]::SizeOf([type][HudKeyChord+INPUT])",
        "function Send-Vk([uint16]$vk, [bool]$up) {",
        "  $inputs = [HudKeyChord+INPUT[]]::new(1)",
        "  $inputs[0].type = $INPUT_KEYBOARD",
        "  $inputs[0].U.ki.wVk = $vk",
        "  $inputs[0].U.ki.wScan = 0",
        "  $inputs[0].U.ki.dwFlags = 0",
        "  if ($up) { $inputs[0].U.ki.dwFlags = $KEYEVENTF_KEYUP }",
        "  $sent = [HudKeyChord]::SendInput(1, $inputs, $InputSize)",
        "  if ($sent -ne 1) {",
        "    $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()",
        "    Write-Output ('keychord-warning:SendInput failed sent=' + $sent + ' last_error=' + $err)",
        "  }",
        "  Start-Sleep -Milliseconds 40",
        "}",
    ]
    press = ", ".join(f"0x{v:02X}" for v in vks)
    rel = ", ".join(f"0x{v:02X}" for v in reversed(vks))
    lines.append(f"$press = @({press})")
    lines.append(f"$release = @({rel})")
    lines.append(f"for ($r = 0; $r -lt {max(1, int(repeat))}; $r++) {{")
    lines.append("  Write-Output ('keychord:press-repeat ' + $r)")
    lines.append("  foreach ($vk in $press) { Send-Vk ([uint16]$vk) $false }")
    lines.append("  Start-Sleep -Milliseconds 60")
    lines.append("  foreach ($vk in $release) { Send-Vk ([uint16]$vk) $true }")
    lines.append("  Start-Sleep -Milliseconds 140")
    lines.append("}")
    return "\n".join(lines) + "\n"


async def run_ssh_task(host: str, user: str, ssh_key: str, task_script: str, timeout_s: float) -> dict:
    cmd = [
        "ssh", "-i", ssh_key, "-o", "BatchMode=yes", "-o", "ConnectTimeout=6",
        "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no",
        f"{user}@{host}", "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass",
        "-Command", "-",
    ]
    started = time.monotonic()
    proc = await asyncio.create_subprocess_exec(
        *cmd, stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    try:
        stdout, stderr = await asyncio.wait_for(
            proc.communicate(task_script.encode("utf-8")), timeout=timeout_s
        )
    except asyncio.TimeoutError:
        with contextlib.suppress(Exception):
            proc.kill()
        return {"ok": False, "error": "timeout", "duration_s": round(time.monotonic() - started, 3)}
    out = stdout.decode("utf-8-sig", errors="replace").strip()
    err = stderr.decode("utf-8", errors="replace").strip()
    if proc.returncode == 0:
        with contextlib.suppress(json.JSONDecodeError):
            r = json.loads(out)
            if isinstance(r, dict):
                r["duration_s"] = round(time.monotonic() - started, 3)
                return r
    return {"ok": proc.returncode == 0, "returncode": proc.returncode,
            "stdout": out, "stderr": err, "duration_s": round(time.monotonic() - started, 3)}


async def inject_focus_then_chord(args, click_actions: list[dict], vks: list[int], *, repeat: int,
                                  scene_w: float, scene_h: float, timeout_s: float = 50.0) -> dict:
    """Focus clicks + a virtual-key chord in ONE process so the overlay's OS
    keyboard focus (acquired by the composer click via focus_window_for_text_input)
    is still held when the chord is injected. Concatenates the exemplar mouse
    script and the keychord script under one hidden-console scheduled task."""
    mouse = ex.windows_diagnostic_input_script(click_actions, scene_width=scene_w, scene_height=scene_h)
    keys = keychord_input_script(vks, repeat=repeat)
    body = HIDE_CONSOLE_PS + mouse + "\nStart-Sleep -Milliseconds 700\n" + keys
    run_id = uuid.uuid4().hex[:8]
    task = ex.windows_diagnostic_task_script(body, user=args.admin_user, timeout_s=timeout_s, run_id=run_id)
    return await run_ssh_task(args.win_host, args.admin_user, args.ssh_key, task, timeout_s + 10.0)


async def inject_pointer(args, actions: list[dict], scene_w: float, scene_h: float, timeout_s: float = 50.0) -> dict:
    # Reuse the exemplar's mouse-injection logic, but prepend the console-hide so
    # the events land on the HUD overlay instead of the injector's own console,
    # and wrap in the interactive scheduled-task launcher.
    ptr = ex.windows_diagnostic_input_script(actions, scene_width=scene_w, scene_height=scene_h)
    body = HIDE_CONSOLE_PS + ptr
    run_id = uuid.uuid4().hex[:8]
    task = ex.windows_diagnostic_task_script(body, user=args.admin_user, timeout_s=timeout_s, run_id=run_id)
    return await run_ssh_task(args.win_host, args.admin_user, args.ssh_key, task, timeout_s + 10.0)


async def run(args: argparse.Namespace) -> int:
    psk = os.environ.get(args.psk_env, "")
    if not psk:
        print(f"FATAL: {args.psk_env} not set", file=sys.stderr)
        return 2
    if not args.win_host:
        print("FATAL: --win-host / TZE_HUD_TEST_HOST required for OS injection", file=sys.stderr)
        return 2

    target = args.target
    outdir = args.outdir
    os.makedirs(os.path.join(outdir, "snapshots"), exist_ok=True)
    os.makedirs(os.path.join(outdir, "logs"), exist_ok=True)
    timeline: list = []

    scene_w, scene_h = args.tab_width, args.tab_height
    portal_x = max(0.0, (scene_w - ex.PORTAL_W) / 2.0)
    portal_y = max(0.0, (scene_h - ex.PORTAL_H) / 2.0)

    AFF = 8.0  # window_resize_affordance_px (default). Aim inside the band.
    corner_x = portal_x + ex.PORTAL_W - AFF / 2.0  # 1066 for defaults
    corner_y = portal_y + ex.PORTAL_H - AFF / 2.0  # 736 for defaults

    grow_dx = min(args.grow_dx, max(0.0, scene_w - (portal_x + ex.PORTAL_W)))
    grow_dy = min(args.grow_dy, max(0.0, scene_h - (portal_y + ex.PORTAL_H)))

    _in_rect, out_rect = ex.portal_pane_rects()
    focus_x = portal_x + out_rect.x + out_rect.w / 2.0
    focus_y = portal_y + out_rect.y + out_rect.h / 2.0
    header_x = portal_x + ex.PORTAL_W / 2.0
    header_y = portal_y + ex.HEADER_H / 2.0

    with open(os.path.join(outdir, "logs", "geometry.json"), "w") as f:
        json.dump({"scene": [scene_w, scene_h], "portal": [portal_x, portal_y, ex.PORTAL_W, ex.PORTAL_H],
                   "affordance_px": AFF, "resize_corner": [corner_x, corner_y],
                   "grow": [grow_dx, grow_dy], "focus_click": [focus_x, focus_y],
                   "header_drag": [header_x, header_y]}, f, indent=2)

    client = HudClient(
        target, psk, agent_id=args.agent_id,
        capabilities=["create_tiles", "modify_own_tiles", "access_input_events"],
    )
    tile_ids: dict[str, str] = {}
    lease_id: Optional[bytes] = None
    hb_task: Optional[asyncio.Task] = None

    async def snap(label: str) -> dict:
        s = await observe_snapshot(target, psk, label)
        with open(os.path.join(outdir, "snapshots", f"{label}.json"), "w") as f:
            json.dump(s or {}, f, indent=2)
        summ = summarize(s, tile_ids)
        emit(timeline, f"snapshot:{label}", {"summary": summ})
        return summ

    try:
        await client.connect()
        emit(timeline, "connect", {"session_id": client.session_id.hex()})
        lease_id = await client.request_lease(ttl_ms=args.lease_ttl_ms, priority=2)
        emit(timeline, "lease", {"lease_id": lease_id.hex()})

        hb_interval = (client.heartbeat_interval_ms or 5000) / 1000.0

        async def _hb() -> None:
            while True:
                await asyncio.sleep(hb_interval)
                with suppress_all():
                    await client.send_heartbeat()
        hb_task = asyncio.create_task(_hb())

        tiles = await ex.create_portal_tiles(client, lease_id, portal_x, portal_y, scene_w, scene_h)
        tile_ids = {
            "capture_backstop": _tile_id_str(tiles.capture_backstop),
            "frame": _tile_id_str(tiles.frame),
            "input_scroll": _tile_id_str(tiles.input_scroll),
            "output_scroll": _tile_id_str(tiles.output_scroll),
            "drag_shield": _tile_id_str(tiles.drag_shield),
            "minimized_icon": _tile_id_str(tiles.minimized_icon),
        }
        with open(os.path.join(outdir, "logs", "tile_ids.json"), "w") as f:
            json.dump(tile_ids, f, indent=2)
        await ex.publish_portal(client, lease_id, tiles, TITLE, SUBTITLE, BASELINE_BODY, FOOTER,
                                include_tile_setup=True)
        emit(timeline, "baseline-published", {"portal": [portal_x, portal_y, ex.PORTAL_W, ex.PORTAL_H]})
        await asyncio.sleep(args.settle_s)
        base = await snap("00-baseline")

        def live_corner(summ: dict) -> tuple[float, float]:
            """Compute the frame's bottom-right affordance point from a live
            snapshot so a few px of position drift never misses the 8px band."""
            fb = (summ["tiles"].get("frame") or {}).get("bounds") or {}
            fx = fb.get("x", portal_x)
            fy = fb.get("y", portal_y)
            fw = fb.get("w", ex.PORTAL_W)
            fh = fb.get("h", ex.PORTAL_H)
            return (fx + fw - AFF / 2.0, fy + fh - AFF / 2.0)

        # ── Check 1: focus a scrollable pane, then pointer-drag the frame corner ─
        # Portal is pristine here (move probe runs LAST). Compute the corner from
        # the live baseline snapshot; aim inside the 8px BottomRight band. One
        # injection so focus is fresh and the cursor path is contiguous.
        cx, cy = live_corner(base)
        resize_seq = [
            {"kind": "click", "label": "focus-output-pane", "x": focus_x, "y": focus_y},
            {"kind": "drag", "label": "resize-frame-corner",
             "start_x": cx, "start_y": cy,
             "end_x": cx + grow_dx, "end_y": cy + grow_dy, "steps": 16},
        ]
        rshot = await inject_pointer(args, resize_seq, scene_w, scene_h)
        emit(timeline, "inject:pointer-resize",
             {"focus": [focus_x, focus_y], "corner": [cx, cy],
              "to": [cx + grow_dx, cy + grow_dy],
              "ok": rshot.get("ok"), "stdout": rshot.get("stdout")})
        await asyncio.sleep(args.settle_s)
        after_ptr = await snap("01-pointer-resized")

        # ── Check 2: adapter republish re-wraps/clips to the NEW bounds ───────
        await ex.publish_portal(client, lease_id, tiles, TITLE, SUBTITLE, BASELINE_BODY, FOOTER,
                                include_tile_setup=False)
        emit(timeline, "republish:post-pointer", {"note": "adapter republish, include_tile_setup=False"})
        await asyncio.sleep(args.settle_s)
        after_ptr_rep = await snap("02-pointer-republish")

        # ── Check 4: persistence — settle, confirm no snap-back ───────────────
        await asyncio.sleep(args.persist_s)
        persist = await snap("03-persist")

        # ── Check 3: #1109 keyboard whole-portal resize ───────────────────────
        # The overlay only acquires OS keyboard focus when TEXT INPUT (composer)
        # is focused (focus_window_for_text_input's AttachThreadInput workaround,
        # hud-dwcr7). A non-composer focus alone never routes OS key events. So
        # reproduce the real keyboard-viewer path: focus the composer first (OS
        # keyboard focus acquired), then click the transcript pane to move scene
        # focus to a NON-composer surface while the overlay keeps OS key focus —
        # exactly the "click transcript / Tab to a control" path #1109 documents.
        composer_x = portal_x + _in_rect.x + _in_rect.w / 2.0
        composer_y = portal_y + _in_rect.y + min(_in_rect.h - 10.0, 72.0)
        focus_clicks = [
            {"kind": "click", "label": "focus-composer", "x": composer_x, "y": composer_y},
            {"kind": "click", "label": "focus-output-pane", "x": focus_x, "y": focus_y},
        ]
        kshot = await inject_focus_then_chord(
            args, focus_clicks, [0x11, 0x10, 0x27], repeat=args.kbd_repeat,
            scene_w=scene_w, scene_h=scene_h)
        emit(timeline, "inject:focus+keychord-ctrl-shift-right",
             {"composer": [composer_x, composer_y], "output": [focus_x, focus_y],
              "ok": kshot.get("ok"), "stdout": kshot.get("stdout"), "stderr": kshot.get("stderr")})
        await asyncio.sleep(args.settle_s)
        after_kbd = await snap("04-kbd-resized")
        await ex.publish_portal(client, lease_id, tiles, TITLE, SUBTITLE, BASELINE_BODY, FOOTER,
                                include_tile_setup=False)
        await asyncio.sleep(args.settle_s)
        after_kbd_rep = await snap("05-kbd-republish")

        # ── Move probe (focus-independent, LAST because it shifts position) ────
        pre_move = await snap("06-pre-move")
        mshot = await inject_pointer(args, [{"kind": "drag", "label": "move-portal-header",
                 "start_x": header_x, "start_y": header_y,
                 "end_x": header_x + args.move_dx, "end_y": header_y + args.move_dy, "steps": 12}],
                 scene_w, scene_h)
        emit(timeline, "inject:move-probe",
             {"from": [header_x, header_y], "to": [header_x + args.move_dx, header_y + args.move_dy],
              "ok": mshot.get("ok"), "stdout": mshot.get("stdout")})
        await asyncio.sleep(args.settle_s)
        after_move = await snap("07-move-probe")

        verdicts = compute_verdicts(base, pre_move, after_move, after_ptr, after_ptr_rep,
                                    persist, after_kbd, after_kbd_rep)
        emit(timeline, "verdicts", verdicts)
        with open(os.path.join(outdir, "logs", "verdicts_computed.json"), "w") as f:
            json.dump(verdicts, f, indent=2)

    finally:
        if hb_task:
            hb_task.cancel()
            with suppress_all():
                await hb_task
        if lease_id is not None:
            with suppress_all():
                await client.release_lease(lease_id)
                emit(timeline, "cleanup-lease-released", {"lease_id": lease_id.hex()})
        with suppress_all():
            await client.disconnect(graceful=True, reason="resize liveverify done")
        clean = await observe_snapshot(target, psk, "99-clean")
        with open(os.path.join(outdir, "snapshots", "99-clean.json"), "w") as f:
            json.dump(clean or {}, f, indent=2)
        emit(timeline, "cleanup-verify", {"tiles_remaining": len((clean or {}).get("tiles") or {})})

    with open(os.path.join(outdir, "logs", "timeline.json"), "w") as f:
        json.dump(timeline, f, indent=2)
    print(f"\nTimeline -> {outdir}/logs/timeline.json", flush=True)
    return 0


def _delta(a: Optional[dict], b: Optional[dict], key: str) -> Optional[float]:
    if not a or not b:
        return None
    ab, bb = a.get("bounds") or {}, b.get("bounds") or {}
    if key not in ab or key not in bb:
        return None
    return round(bb[key] - ab[key], 1)


def compute_verdicts(base, pre_move, after_move, ptr, ptr_rep, persist, kbd, kbd_rep) -> dict:
    v: dict[str, Any] = {}
    bf = base["tiles"].get("frame")

    # Move probe (focus-independent OS-reach test): header-drag moved the frame.
    pm = pre_move["tiles"].get("frame")
    mv = after_move["tiles"].get("frame")
    v["probeM_move"] = {
        "frame_before": (pm or {}).get("bounds"), "frame_after": (mv or {}).get("bounds"),
        "frame_dx": _delta(pm, mv, "x"), "frame_dy": _delta(pm, mv, "y"),
        "pass": bool(abs(_delta(pm, mv, "x") or 0) > 5.0 or abs(_delta(pm, mv, "y") or 0) > 5.0),
    }

    # Check 1: pointer resize grew the frame + members (vs pristine baseline).
    pf = ptr["tiles"].get("frame")
    dw, dh = _delta(bf, pf, "w"), _delta(bf, pf, "h")
    v["check1_pointer_resize"] = {
        "frame_dw": dw, "frame_dh": dh,
        "members": {r: {"dw": _delta(base["tiles"].get(r), ptr["tiles"].get(r), "w"),
                        "dh": _delta(base["tiles"].get(r), ptr["tiles"].get(r), "h")}
                    for r in ("frame", "input_scroll", "output_scroll", "capture_backstop")},
        "pass": bool(dw and dw > 5.0),
    }

    # Check 2: republish re-wrap — transcript node width tracks resized pane.
    base_w = ((base.get("transcript_node") or {}).get("bounds") or {}).get("w")
    rep_w = ((ptr_rep.get("transcript_node") or {}).get("bounds") or {}).get("w")
    v["check2_republish_rewrap"] = {
        "transcript_width_before": base_w, "transcript_width_after_republish": rep_w,
        "output_tile_width_after": ((ptr_rep["tiles"].get("output_scroll") or {}).get("bounds") or {}).get("w"),
        "pass": bool(base_w and rep_w and rep_w > base_w + 5.0),
    }

    # Check 4: persistence — no snap-back between republish and settle.
    pr_f = ptr_rep["tiles"].get("frame")
    ps_f = persist["tiles"].get("frame")
    v["check4_persistence"] = {
        "frame_republish": (pr_f or {}).get("bounds"),
        "frame_after_settle": (ps_f or {}).get("bounds"),
        "resize_band_node_after_settle": persist.get("resize_hit_node"),
        "pass": bool(pr_f and ps_f and pr_f.get("bounds") == ps_f.get("bounds")),
    }

    # Check 3: keyboard resize (width grow) vs persist baseline.
    base_k = persist["tiles"].get("frame")
    kf = kbd["tiles"].get("frame")
    kdw = _delta(base_k, kf, "w")
    v["check3_keyboard_resize"] = {
        "frame_dw": kdw,
        "kbd_republish_transcript_w": ((kbd_rep.get("transcript_node") or {}).get("bounds") or {}).get("w"),
        "pass": bool(kdw and kdw > 5.0),
        "note": "injection-limited if frame_dw is null/0 (virtual-key chord did not land)",
    }
    return v


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--target", default=os.environ.get("TZE_HUD_GRPC_TARGET", "127.0.0.1:50051"))
    p.add_argument("--psk-env", default="TZE_HUD_PSK")
    p.add_argument("--agent-id", default="agent-alpha")
    p.add_argument("--tab-width", type=float, default=1280.0)
    p.add_argument("--tab-height", type=float, default=800.0)
    p.add_argument("--grow-dx", type=float, default=160.0)
    p.add_argument("--grow-dy", type=float, default=55.0)
    p.add_argument("--move-dx", type=float, default=-110.0)
    p.add_argument("--move-dy", type=float, default=40.0)
    p.add_argument("--kbd-repeat", type=int, default=6)
    p.add_argument("--lease-ttl-ms", type=int, default=300000)
    p.add_argument("--settle-s", type=float, default=3.0)
    p.add_argument("--persist-s", type=float, default=5.0)
    p.add_argument("--outdir", default=os.path.dirname(os.path.abspath(__file__)))
    p.add_argument("--win-host", default=os.environ.get("TZE_HUD_TEST_HOST", ""))
    p.add_argument("--admin-user", default="admin-user")
    p.add_argument("--ssh-key", default=os.path.expanduser("~/.ssh/hud-ssh-key"))
    return p.parse_args()


if __name__ == "__main__":
    sys.exit(asyncio.run(run(parse_args())))
