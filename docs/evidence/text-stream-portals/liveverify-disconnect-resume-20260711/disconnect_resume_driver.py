#!/usr/bin/env python3
"""Live disconnect->stale->reconnect->resume driver for the text-stream portal
first-class surface (openspec change ``portal-disconnect-resume-ux`` task 5.1 /
bead hud-om69w).

This is an EVIDENCE harness, not product code. It reuses the tracked
``text_stream_portal_exemplar`` building blocks (``HudClient`` transport, the
raw six-tile portal assembly, the first-class ``PortalSurface`` declaration, and
the scheduled-task VM screenshot path) and adds the disconnect/resume control
flow the exemplar's phase machinery does not script:

  1. baseline  - build the portal over a resident-gRPC ``HudSession`` (first-class
                 surface, lifecycle ACTIVE) and publish transcript units A/B/C.
  2. stale     - patch the surface lifecycle -> DEGRADED via the coalescible
                 ``UpdatePortalSurfaceState`` WITHOUT tearing down the committed
                 transcript (the reachable cooperative staleness signal).
  3. drop      - drop the resident gRPC channel WITHOUT ``SessionClose``/detach and
                 WITHOUT releasing the lease -> a true ungraceful transport drop.
  4. wait      - let the session server detect the drop (missed heartbeats) and
                 orphan the lease under the resume grace window, staging the
                 resume-token entry.
  5. resume    - reconnect via ``SessionResume`` (same agent_id + resume_token),
                 which restores the SAME lease; patch lifecycle -> ACTIVE and
                 append unit D (continuity: A/B/C persist + D, no duplication).
  6. cleanup   - release the lease, close the session, leave the overlay clean.

At every phase it (a) collects a scene snapshot from a throwaway observer session
-- ``portal_surfaces[<scene_id>].lifecycle`` is the #1098 SceneSnapshot
portal-surface descriptor used for parity -- and (b) triggers a full-screen VM
capture via the scheduled-task path. The full-screen PNGs are cropped to the
portal region by ``crop_portal_region.py`` BEFORE anything is committed; raw
full-desktop frames are never committed.

Run: see ``run.sh`` in this directory. Requires the resident gRPC port reachable
and ``TZE_HUD_PSK`` in the environment.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
import uuid
from typing import Any, Optional

# The tracked user-test scripts (HudClient + the exemplar building blocks) and
# their generated proto stubs. Resolve relative to the repo root so the harness
# is runnable from the evidence directory.
_REPO_ROOT = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "..")
)
_SCRIPTS = os.path.join(_REPO_ROOT, ".claude", "skills", "user-test", "scripts")
sys.path.insert(0, _SCRIPTS)
sys.path.insert(0, os.path.join(_SCRIPTS, "proto_gen"))

import session_pb2  # noqa: E402
import types_pb2  # noqa: E402
from hud_grpc_client import HudClient  # noqa: E402
import text_stream_portal_exemplar as ex  # noqa: E402


# ── Transcript content (stable, greppable units) ──────────────────────────────
UNIT_A = "**[unit A]** session established — baseline transcript line one"
UNIT_B = "**[unit B]** streaming continues — baseline transcript line two"
UNIT_C = "**[unit C]** pre-disconnect committed content (must survive resume)"
UNIT_D = "**[unit D]** post-resume continuation — appended after reconnect"

BASELINE_BODY = ex.join_transcript_entries([UNIT_A, UNIT_B, UNIT_C])
RESUME_BODY = ex.join_transcript_entries([UNIT_A, UNIT_B, UNIT_C, UNIT_D])

TITLE = "liveverify · disconnect→resume"
SUBTITLE = "portal §5.1 · hud-om69w"
FOOTER = "resident-gRPC first-class surface"


class ResumableHudClient(HudClient):
    """HudClient that captures the handshake ``resume_token`` and can reconnect
    to an orphaned session via ``SessionResume`` (the stock client does neither).
    """

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self.resume_token: bytes = b""

    # The stock connect() discards SessionEstablished.resume_token, so drive the
    # SessionInit handshake here and store the token for the later resume.
    async def connect_init(self) -> None:
        """SessionInit handshake that also stores ``resume_token``."""
        self._transport_closed = False
        self._session_close_sent = False
        self._response_queue = asyncio.Queue()
        self._deferred_responses = []
        import grpc  # local import; grpc is a client dep
        import session_pb2_grpc

        self._channel = grpc.aio.insecure_channel(self.target)
        stub = session_pb2_grpc.HudSessionStub(self._channel)
        self._send_queue = asyncio.Queue()
        self._stream = stub.Session(self._request_iterator())

        init_msg = session_pb2.ClientMessage(
            sequence=self._next_seq(),
            timestamp_wall_us=_now_us(),
            session_init=session_pb2.SessionInit(
                agent_id=self.agent_id,
                agent_display_name=self.agent_id,
                auth_credential=session_pb2.AuthCredential(
                    pre_shared_key=session_pb2.PreSharedKeyCredential(key=self.psk),
                ),
                requested_capabilities=self.capabilities,
                initial_subscriptions=self.initial_subscriptions,
                agent_timestamp_wall_us=_now_us(),
                min_protocol_version=1000,
                max_protocol_version=1000,
            ),
        )
        await self._send_queue.put(init_msg)
        self._reader_task = asyncio.create_task(self._read_loop())
        resp = await self._wait_for("session_established", timeout=5.0)
        est = resp.session_established
        self.session_id = est.session_id
        self.namespace = est.namespace
        self.heartbeat_interval_ms = est.heartbeat_interval_ms
        self.granted_capabilities = list(est.granted_capabilities)
        self.resume_token = bytes(est.resume_token)
        if est.HasField("portal_part_tokens"):
            self.resolved_portal_tokens = dict(est.portal_part_tokens.tokens)
        snap = await self._wait_for("scene_snapshot", timeout=5.0)
        self.scene_snapshot_json = snap.scene_snapshot.snapshot_json
        print(
            f"  [grpc] init: ns={self.namespace} caps={self.granted_capabilities} "
            f"resume_token={self.resume_token.hex()[:16]}…",
            flush=True,
        )

    async def resume(self, resume_token: bytes) -> session_pb2.SessionResumeResult:
        """Reconnect to the orphaned session via ``SessionResume`` on a fresh
        stream. Returns the ``SessionResumeResult`` and captures the follow-on
        SceneSnapshot into ``scene_snapshot_json``."""
        self._transport_closed = False
        self._session_close_sent = False
        self._seq = 0
        self._response_queue = asyncio.Queue()
        self._deferred_responses = []
        import grpc
        import session_pb2_grpc

        self._channel = grpc.aio.insecure_channel(self.target)
        stub = session_pb2_grpc.HudSessionStub(self._channel)
        self._send_queue = asyncio.Queue()
        self._stream = stub.Session(self._request_iterator())

        resume_msg = session_pb2.ClientMessage(
            sequence=self._next_seq(),
            timestamp_wall_us=_now_us(),
            session_resume=session_pb2.SessionResume(
                agent_id=self.agent_id,
                resume_token=resume_token,
                last_seen_server_sequence=self._server_seq,
                auth_credential=session_pb2.AuthCredential(
                    pre_shared_key=session_pb2.PreSharedKeyCredential(key=self.psk),
                ),
            ),
        )
        await self._send_queue.put(resume_msg)
        self._reader_task = asyncio.create_task(self._read_loop())
        resp = await self._wait_for("session_resume_result", timeout=8.0)
        result = resp.session_resume_result
        print(
            f"  [grpc] resume: accepted={result.accepted} "
            f"caps={list(result.granted_capabilities)} err={result.error!r}",
            flush=True,
        )
        if result.accepted:
            self.granted_capabilities = list(result.granted_capabilities)
            self.resume_token = bytes(result.new_session_token)
            try:
                snap = await self._wait_for("scene_snapshot", timeout=5.0)
                self.scene_snapshot_json = snap.scene_snapshot.snapshot_json
            except Exception as exc:  # noqa: BLE001
                print(f"  [grpc] resume: no follow-on snapshot ({exc})", flush=True)
        return result


def _now_us() -> int:
    return int(time.time() * 1_000_000)


async def observe_snapshot(target: str, psk: str, label: str) -> Optional[dict]:
    """Connect a throwaway observer session, grab the current scene snapshot
    (which carries ``portal_surfaces`` overlay state), then leave gracefully."""
    obs = HudClient(target, psk, agent_id=f"liveverify-observer-{label}")
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


class suppress_all:
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return True

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return True


def portal_surface_summary(snapshot: Optional[dict]) -> dict:
    """Extract the portal-surface descriptors + transcript-body text nodes for
    parity checks. Returns {surfaces: {scene_id: {session_id, lifecycle,
    display_state}}, transcript_units: [..], has_disconnect_marker: bool}."""
    out: dict[str, Any] = {"surfaces": {}, "transcript_units": [], "raw_present": False}
    if not snapshot:
        return out
    surfaces = snapshot.get("portal_surfaces") or snapshot.get("portalSurfaces") or {}
    for sid, surf in surfaces.items():
        identity = surf.get("identity", {})
        out["surfaces"][sid] = {
            "session_id": identity.get("session_id") or identity.get("sessionId"),
            "lifecycle": surf.get("lifecycle"),
            "display_state": surf.get("display_state") or surf.get("displayState"),
        }
    out["raw_present"] = bool(out["surfaces"])
    # Transcript units: scan the serialized snapshot for our stable unit markers.
    blob = json.dumps(snapshot)
    for tag, unit in (("A", UNIT_A), ("B", UNIT_B), ("C", UNIT_C), ("D", UNIT_D)):
        if unit.split("**")[1] in blob or f"[unit {tag}]" in blob:
            out["transcript_units"].append(tag)
    out["has_disconnect_marker"] = "disconnected — stream stale" in blob or "⊘" in blob
    return out


async def capture_vm_screenshot(
    host: str,
    admin_user: str,
    ssh_key: str,
    remote_png: str,
    scene_w: float,
    scene_h: float,
) -> dict:
    """Full-screen VM capture via the exemplar's scheduled-task path. Writes the
    PNG on the VM at ``remote_png`` (cropped locally after SCP)."""
    actions = [{"kind": "screenshot", "label": "phase", "path": remote_png}]
    return await ex.run_windows_diagnostic_input(
        host,
        user=admin_user,
        ssh_key=ssh_key,
        actions=actions,
        timeout_s=45.0,
        connect_timeout_s=6.0,
        scene_width=scene_w,
        scene_height=scene_h,
    )


def emit(timeline: list, phase: str, detail: dict) -> None:
    entry = {"t_wall_us": _now_us(), "phase": phase, **detail}
    timeline.append(entry)
    print(f"[timeline] {phase}: {json.dumps(detail)}", flush=True)


async def run(args: argparse.Namespace) -> int:
    psk = os.environ.get(args.psk_env, "")
    if not psk:
        print(f"FATAL: {args.psk_env} not set in environment", file=sys.stderr)
        return 2
    target = args.target
    timeline: list = []
    outdir = args.outdir
    os.makedirs(os.path.join(outdir, "snapshots"), exist_ok=True)
    os.makedirs(os.path.join(outdir, "logs"), exist_ok=True)

    # Portal placement: centered on the reported display area.
    scene_w, scene_h = args.tab_width, args.tab_height
    portal_x = max(0.0, (scene_w - ex.PORTAL_W) / 2.0)
    portal_y = max(0.0, (scene_h - ex.PORTAL_H) / 2.0)
    crop_box = {
        "x": int(portal_x),
        "y": int(portal_y),
        "w": int(ex.PORTAL_W),
        "h": int(ex.PORTAL_H),
    }
    with open(os.path.join(outdir, "logs", "crop_box.json"), "w") as f:
        json.dump({"crop_box": crop_box, "scene": [scene_w, scene_h]}, f, indent=2)

    async def snapshot_phase(label: str, remote_png: str) -> None:
        snap = await observe_snapshot(target, psk, label)
        path = os.path.join(outdir, "snapshots", f"{label}.json")
        with open(path, "w") as f:
            json.dump(snap or {}, f, indent=2)
        summary = portal_surface_summary(snap)
        emit(timeline, f"snapshot:{label}", {"summary": summary, "file": f"snapshots/{label}.json"})
        if args.screenshots:
            shot = await capture_vm_screenshot(
                args.win_host, args.admin_user, args.ssh_key, remote_png, scene_w, scene_h
            )
            emit(timeline, f"screenshot:{label}", {"remote_png": remote_png, "ok": shot.get("ok"), "result": shot})

    # Capabilities MUST be a subset of the config-registered agent's grants
    # (deployed tze_hud.toml registers agent-alpha with exactly these three);
    # requesting more (e.g. upload_resource) trips PERMISSION_DENIED at lease time.
    client = ResumableHudClient(
        target, psk, agent_id=args.agent_id,
        capabilities=["create_tiles", "modify_own_tiles", "access_input_events"],
    )
    lease_id: Optional[bytes] = None
    hb_task: Optional[asyncio.Task] = None
    try:
        # ── Phase 1: baseline ────────────────────────────────────────────────
        await client.connect_init()
        emit(timeline, "connect", {"session_id": client.session_id.hex(), "resume_token": client.resume_token.hex()})
        lease_id = await client.request_lease(ttl_ms=args.lease_ttl_ms, priority=2)
        emit(timeline, "lease", {"lease_id": lease_id.hex(), "ttl_ms": client.last_granted_lease_ttl_ms})

        # Keepalive so the session stays live through the baseline/stale phases.
        hb_interval = (client.heartbeat_interval_ms or 5000) / 1000.0

        async def _hb() -> None:
            while True:
                await asyncio.sleep(hb_interval)
                with suppress_all():
                    await client.send_heartbeat()

        hb_task = asyncio.create_task(_hb())

        tiles = await ex.create_portal_tiles(
            client, lease_id, portal_x, portal_y, scene_w, scene_h
        )
        await ex.publish_portal(
            client, lease_id, tiles, TITLE, SUBTITLE, BASELINE_BODY, FOOTER,
            include_tile_setup=True,
        )
        emit(timeline, "baseline-published", {"units": ["A", "B", "C"], "lifecycle": "ACTIVE"})
        await asyncio.sleep(args.settle_s)
        await snapshot_phase("01-baseline", args.remote_dir + "\\lv-01-baseline.png")

        # ── Phase 2: stale (cooperative lifecycle -> DEGRADED) ────────────────
        await client.submit_mutation_batch(
            lease_id,
            [
                ex.update_portal_surface_state_mutation(
                    tiles.frame,
                    lifecycle=types_pb2.PORTAL_LIFECYCLE_STATE_DEGRADED,
                )
            ],
        )
        emit(timeline, "stale-marked", {"lifecycle": "DEGRADED", "transcript": "preserved (A,B,C)"})
        await asyncio.sleep(args.settle_s)
        await snapshot_phase("02-stale", args.remote_dir + "\\lv-02-stale.png")

        # ── Phase 3: ungraceful transport drop (no detach, no lease release) ──
        if hb_task:
            hb_task.cancel()
            with suppress_all():
                await hb_task
            hb_task = None
        await client.drop_connection()
        emit(timeline, "transport-dropped", {
            "how": "drop_connection() — closed resident gRPC channel WITHOUT SessionClose/detach and WITHOUT lease release",
        })
        # Snapshot immediately: within grace the surface + tiles persist.
        await asyncio.sleep(args.settle_s)
        await snapshot_phase("03-dropped", args.remote_dir + "\\lv-03-dropped.png")

        # ── Phase 4: wait for the server to detect the drop & orphan the lease ─
        emit(timeline, "await-detection", {"wait_s": args.detect_wait_s,
             "why": "missed-heartbeat threshold detects the ungraceful drop, orphans the lease under resume grace, stages resume-token entry"})
        await asyncio.sleep(args.detect_wait_s)

        # ── Phase 5: reconnect via SessionResume (within grace) ───────────────
        result = await client.resume(client.resume_token)
        emit(timeline, "resume-attempt", {"accepted": result.accepted, "error": result.error})
        if not result.accepted:
            emit(timeline, "resume-FAILED", {"error": result.error,
                 "note": "resume token rejected — see VERDICTS.md for interpretation"})
        else:
            # Re-establish keepalive and continue the transcript in place.
            async def _hb2() -> None:
                while True:
                    await asyncio.sleep(hb_interval)
                    with suppress_all():
                        await client.send_heartbeat()
            hb_task = asyncio.create_task(_hb2())
            await ex.publish_portal(
                client, lease_id, tiles, TITLE, SUBTITLE, RESUME_BODY, FOOTER,
                include_tile_setup=False,
            )
            emit(timeline, "resume-published", {"units": ["A", "B", "C", "D"], "lifecycle": "ACTIVE"})
        await asyncio.sleep(args.settle_s)
        await snapshot_phase("04-resumed", args.remote_dir + "\\lv-04-resumed.png")

    finally:
        # ── Phase 6: cleanup ─────────────────────────────────────────────────
        if hb_task:
            hb_task.cancel()
            with suppress_all():
                await hb_task
        if lease_id is not None:
            with suppress_all():
                await client.release_lease(lease_id)
                emit(timeline, "cleanup-lease-released", {"lease_id": lease_id.hex()})
        with suppress_all():
            await client.disconnect(graceful=True, reason="liveverify done")
        # Verify overlay clean via a final observer snapshot.
        clean = await observe_snapshot(target, psk, "99-clean")
        with open(os.path.join(outdir, "snapshots", "99-clean.json"), "w") as f:
            json.dump(clean or {}, f, indent=2)
        emit(timeline, "cleanup-verify", {"summary": portal_surface_summary(clean)})

    with open(os.path.join(outdir, "logs", "timeline.json"), "w") as f:
        json.dump(timeline, f, indent=2)
    print(f"\nTimeline written to {outdir}/logs/timeline.json", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--target", default=os.environ.get("TZE_HUD_GRPC_TARGET", "127.0.0.1:50051"))
    p.add_argument("--psk-env", default="TZE_HUD_PSK")
    # Must match a config-registered agent to be granted tile capabilities
    # (deployed tze_hud.toml registers agent-alpha). Resume is agent-bound, so
    # the same id is replayed on SessionResume.
    p.add_argument("--agent-id", default="agent-alpha")
    p.add_argument("--tab-width", type=float, default=1280.0)
    p.add_argument("--tab-height", type=float, default=800.0)
    p.add_argument("--lease-ttl-ms", type=int, default=120000)
    p.add_argument("--settle-s", type=float, default=3.0)
    p.add_argument("--detect-wait-s", type=float, default=20.0,
                   help="Wait after drop for missed-heartbeat detection (grace window is ~30s)")
    p.add_argument("--outdir", default=os.path.dirname(os.path.abspath(__file__)))
    p.add_argument("--screenshots", action="store_true")
    p.add_argument("--win-host", default=os.environ.get("TZE_HUD_TEST_HOST", ""))
    p.add_argument("--admin-user", default="admin-user")
    p.add_argument("--ssh-key", default=os.path.expanduser("~/.ssh/hud-ssh-key"))
    p.add_argument("--remote-dir", default="C:\\tze_hud")
    return p.parse_args()


if __name__ == "__main__":
    sys.exit(asyncio.run(run(parse_args())))
