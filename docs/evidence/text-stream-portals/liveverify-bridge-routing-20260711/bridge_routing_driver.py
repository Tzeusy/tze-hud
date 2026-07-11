#!/usr/bin/env python3
"""Live bridge-routing verification driver (hud-rw8eo).

Proves that portal projections created via the MCP ``portal_projection_*`` facade
are materialised over the **resident-gRPC bridge** (``PortalTransport::ResidentGrpcBridge``,
wired by hud-hfuxy / PR #1046) instead of the in-process direct-scene arm, when
the runtime is launched with ``--resident-grpc-portal`` (== ``TZE_HUD_RESIDENT_GRPC_PORTAL=1``).

Why the tile ``namespace`` is the proof
---------------------------------------
There is deliberately **no** transport field on the wire (WM-S2b snapshot exclusion,
``types.proto``), and the runtime writes tracing to stdout only (discarded by the
Scheduled-Task deployment). But routing is directly observable through *who owns
the materialised tile*:

  * bridged   -> the bridge opens its OWN loopback ``HudSession`` as agent
                 ``resident-grpc-portal`` (DEFAULT_RESIDENT_GRPC_AGENT_ID) and
                 creates the portal tile from that session, so the tile's
                 ``namespace == "resident-grpc-portal"`` (namespace isolation,
                 RFC 0001 §1.2, is enforced at tile creation).
  * in-process -> the driver paints the tile directly under
                 ``PORTAL_DRIVER_NAMESPACE == "tze_hud_portal_driver"``.

The in-process path can NEVER produce a ``resident-grpc-portal`` tile, so observing
that namespace on a projection attached through the ordinary MCP facade is an
unambiguous proof the projection was routed over the bridge.

Observation is via a throwaway gRPC observer ``HudSession`` reading
``ServerMessage.scene_snapshot`` (SCENE_TOPOLOGY subscription).

This is an EVIDENCE harness, not product code.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
import urllib.request
from typing import Any, Optional

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO_ROOT = os.path.abspath(os.path.join(_HERE, "..", "..", "..", ".."))
_SCRIPTS = os.path.join(_REPO_ROOT, ".claude", "skills", "user-test", "scripts")
sys.path.insert(0, _SCRIPTS)
sys.path.insert(0, os.path.join(_SCRIPTS, "proto_gen"))
from hud_grpc_client import HudClient  # noqa: E402

BRIDGE_NAMESPACE = "resident-grpc-portal"
INPROCESS_NAMESPACE = "tze_hud_portal_driver"


def now_us() -> int:
    return int(time.time() * 1_000_000)


class Driver:
    def __init__(self, host: str, mcp_url: str, psk: str, agent_id: str, outdir: str):
        self.host = host
        self.mcp = mcp_url.rstrip("/") + "/mcp"
        self.psk = psk
        self.agent_id = agent_id
        self.outdir = outdir
        self.timeline: list[dict[str, Any]] = []

    # ── MCP JSON-RPC (methods are the JSON-RPC `method`, not tools/call) ──────
    def mcp_call(self, method: str, params: dict) -> dict:
        body = json.dumps({"jsonrpc": "2.0", "id": method, "method": method, "params": params}).encode()
        req = urllib.request.Request(
            self.mcp, data=body,
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + self.psk},
        )
        with urllib.request.urlopen(req, timeout=30) as r:
            resp = json.loads(r.read())
        if "error" in resp:
            raise RuntimeError(f"MCP {method} error: {resp['error']}")
        return resp["result"]

    def log(self, code: str, **kw) -> None:
        entry = {"ts_wall_us": now_us(), "code": code, **kw}
        self.timeline.append(entry)
        printable = {k: v for k, v in kw.items() if k != "snapshot"}
        print(f"[{code}] {json.dumps(printable)[:300]}", flush=True)

    # ── gRPC observer snapshot ───────────────────────────────────────────────
    async def _snapshot(self) -> dict:
        c = HudClient(f"{self.host}:50051", self.psk, agent_id=self.agent_id,
                      initial_subscriptions=["SCENE_TOPOLOGY"])
        await c.connect()
        try:
            return json.loads(c.scene_snapshot_json)
        finally:
            await c.close()

    def snapshot(self) -> dict:
        return asyncio.new_event_loop().run_until_complete(self._snapshot())

    @staticmethod
    def portal_tiles(snap: dict) -> list[dict]:
        tiles = snap.get("tiles") or {}
        surfaces = snap.get("portal_surfaces") or {}
        out = []
        for sid in surfaces:
            t = tiles.get(sid)
            if t is not None:
                out.append({"tile_id": sid, "namespace": t.get("namespace"),
                            "lease_id": t.get("lease_id")})
        # also include any tile whose namespace is a portal materialiser even if
        # portal_surfaces is empty (defensive)
        for tid, t in tiles.items():
            ns = t.get("namespace")
            if ns in (BRIDGE_NAMESPACE, INPROCESS_NAMESPACE) and not any(o["tile_id"] == tid for o in out):
                out.append({"tile_id": tid, "namespace": ns, "lease_id": t.get("lease_id")})
        return out

    @staticmethod
    def surface_summary(snap: dict) -> list[dict]:
        # The SCENE_TOPOLOGY snapshot's PortalSurface descriptor carries the
        # part TOPOLOGY (kinds: Frame/Header/Transcript/Composer) + lifecycle +
        # display_state, NOT the rendered transcript text (text is materialised
        # into tile scene nodes / pixels; pixel capture is unreliable on this
        # software-GPU VM). So the streaming proof is: the bridged surface stays
        # Active with a Transcript part across sequential publishes.
        surfaces = snap.get("portal_surfaces") or {}
        out = []
        for sid, s in surfaces.items():
            parts = s.get("parts") or []
            kinds = [p.get("kind") for p in parts if isinstance(p, dict)]
            out.append({"surface_id": sid,
                        "lifecycle": s.get("lifecycle"),
                        "display_state": s.get("display_state"),
                        "part_kinds": kinds})
        return out

    def write_snapshot(self, name: str, snap: dict) -> None:
        path = os.path.join(self.outdir, "snapshots", name)
        with open(path, "w") as f:
            json.dump(snap, f, indent=1, sort_keys=True)
        print(f"  wrote {path}", flush=True)


def run(args: argparse.Namespace) -> int:
    d = Driver(args.host, args.mcp_url, args.psk, args.agent_id, args.outdir)
    pid = args.projection_id
    verdicts: dict[str, Any] = {}

    d.log("scenario:start", projection_id=pid, host="<vm-ip>", note="bridge-enabled runtime (--resident-grpc-portal)")

    # ── Phase 1: attach (routes onto the bridge at dispatch_portal_op Attach) ─
    r = d.mcp_call("portal_projection_attach", {"projection_id": pid, "display_name": "bridge routing live-verify"})
    token = r["owner_token"]
    d.log("attach", accepted=r["accepted"], status=r["status_summary"])

    # ── Phase 2: publish baseline transcript unit + settle for materialisation ─
    d.mcp_call("portal_projection_publish", {"projection_id": pid, "owner_token": token,
               "output_text": "**[unit A]** bridge routing live-verify — baseline transcript line one",
               "output_kind": "assistant"})
    d.log("publish", unit="A")
    time.sleep(args.settle_s)

    snap = d.snapshot()
    d.write_snapshot("01-bridged-baseline.json", snap)
    ptiles = d.portal_tiles(snap)
    surfaces = d.surface_summary(snap)
    d.log("snapshot:baseline", portal_tiles=ptiles, surfaces=surfaces)
    bridged = bool(ptiles) and all(t["namespace"] == BRIDGE_NAMESPACE for t in ptiles)
    verdicts["1_attach_publish_routes_via_bridge"] = {
        "pass": bridged,
        "portal_tile_namespace": ptiles[0]["namespace"] if ptiles else None,
        "expected": BRIDGE_NAMESPACE,
        "in_process_would_be": INPROCESS_NAMESPACE,
    }

    # ── Phase 3: transcript streaming (append units B, C, D) ─────────────────
    for unit, text in [("B", "**[unit B]** streaming continues — second line over the bridge"),
                       ("C", "**[unit C]** third streamed line, coalesced through the authority drain"),
                       ("D", "**[unit D]** fourth line — confirms sustained bridged streaming")]:
        d.mcp_call("portal_projection_publish", {"projection_id": pid, "owner_token": token,
                   "output_text": text, "output_kind": "assistant"})
        d.log("publish", unit=unit)
        time.sleep(args.stream_gap_s)
    time.sleep(args.settle_s)
    snap2 = d.snapshot()
    d.write_snapshot("02-bridged-streamed.json", snap2)
    ptiles2 = d.portal_tiles(snap2)
    surfaces2 = d.surface_summary(snap2)
    d.log("snapshot:streamed", portal_tiles=ptiles2, surfaces=surfaces2)
    still_bridged = bool(ptiles2) and all(t["namespace"] == BRIDGE_NAMESPACE for t in ptiles2)
    has_transcript = bool(surfaces2 and "Transcript" in (surfaces2[0].get("part_kinds") or []))
    active = bool(surfaces2 and str(surfaces2[0].get("lifecycle", "")).lower().find("active") >= 0)
    verdicts["2_transcript_streaming"] = {
        "pass": still_bridged and has_transcript,
        "still_bridged": still_bridged,
        "surface_lifecycle": surfaces2[0].get("lifecycle") if surfaces2 else None,
        "part_kinds": surfaces2[0].get("part_kinds") if surfaces2 else None,
        "publishes_accepted": 4,
        "note": ("4 sequential publishes (A-D) accepted by the authority; the bridged "
                 "PortalSurface stays materialised with a Transcript part across the stream. "
                 "Transcript pixel text is not in the topology snapshot (pixel capture "
                 "unreliable on this software-GPU VM)."),
    }

    # ── Phase 4 (#3): unread-count / jump-to-latest plumbing (#1107) ─────────
    # Each non-viewer publish increments unread_output_count; the bridge adapter
    # emits SetTileUnreadCountMutation over the wire (#1107). The numeric value
    # lands in RuntimeOverlayState.tile_unread_counts (#[serde(skip)]) and is NOT
    # present in the SceneGraphSnapshot, so it is not externally observable over
    # gRPC snapshot polling — recorded as a plumbing-exercised / value-not-observable
    # result rather than a hard PASS.
    for i in range(args.unread_bursts):
        d.mcp_call("portal_projection_publish", {"projection_id": pid, "owner_token": token,
                   "output_text": f"**[burst {i}]** rapid output to accumulate unread count",
                   "output_kind": "assistant"})
    d.log("publish:unread-bursts", count=args.unread_bursts)
    time.sleep(args.settle_s)
    snap3 = d.snapshot()
    d.write_snapshot("03-bridged-unread-bursts.json", snap3)
    ptiles3 = d.portal_tiles(snap3)
    d.log("snapshot:unread", portal_tiles=ptiles3,
          note="unread count is #[serde(skip)] overlay state; not in snapshot")
    verdicts["3_unread_jump_to_latest_parity"] = {
        "pass": None,  # not externally observable
        "bridged_path_active": bool(ptiles3) and all(t["namespace"] == BRIDGE_NAMESPACE for t in ptiles3),
        "note": ("SetTileUnreadCountMutation (#1107) is emitted on the bridged path by "
                 "construction (resident_grpc.rs render_batch), but the numeric unread value "
                 "lands in RuntimeOverlayState.tile_unread_counts (#[serde(skip)]) and is not "
                 "carried in SceneGraphSnapshot/portal_surfaces — not externally observable over "
                 "gRPC; pixel capture unreliable on this software-GPU VM."),
    }

    # ── Phase 5 (#4): composer draft over the bridge (hud-omfqi) ─────────────
    # inject_composer_paste feeds the runtime ComposerDraftManager -> a
    # ComposerDraftState event. Over the bridge that is classified as
    # ResidentBridgeInputKind::DraftState. Only a real ComposerDraftSubmit (OS
    # Enter) is forwarded to the authority pending-input inbox; draft-state and
    # cancel are dropped by drain_resident_grpc_input. So get_pending_input will
    # NOT return a paste. We drive the paste (no OS key injection) and record the
    # get_pending_input result to document the boundary precisely.
    try:
        paste = d.mcp_call("inject_composer_paste",
                           {"text": "draft over the bridge — composer smoke (no OS key injection)"})
        d.log("inject_composer_paste", result=paste)
    except Exception as e:  # tool may report no active composer
        paste = {"error": str(e)}
        d.log("inject_composer_paste:error", error=str(e))
    time.sleep(2)
    pend = d.mcp_call("portal_projection_get_pending_input",
                      {"projection_id": pid, "owner_token": token, "wait_ms": 1500})
    d.log("get_pending_input", result=pend)
    verdicts["4_composer_input_over_bridge"] = {
        "pass": None,  # submit path needs OS keyboard
        "paste_injected": paste,
        "pending_input": pend,
        "note": ("inject_composer_paste produces DRAFT state only; over the bridge it is "
                 "classified as DraftState and dropped by drain_resident_grpc_input (only "
                 "ComposerDraftSubmit reaches pending-input). A submit reaching pending-input "
                 "requires a real OS Enter keypress on the focused bridged composer — no "
                 "keyboard-free MCP/gRPC substitute exists in this build. Draft-state ingress "
                 "over the bridge is exercised; pending-input submit is not keyboard-free."),
    }

    # ── Phase 6 (#5): clean detach / teardown ────────────────────────────────
    r = d.mcp_call("portal_projection_detach", {"projection_id": pid, "owner_token": token,
                   "reason": "live-verify complete"})
    d.log("detach", result=r)
    time.sleep(args.settle_s)
    snap9 = d.snapshot()
    d.write_snapshot("99-detached-clean.json", snap9)
    ptiles9 = d.portal_tiles(snap9)
    surfaces9 = snap9.get("portal_surfaces") or {}
    d.log("snapshot:detached", portal_tiles=ptiles9, n_portal_surfaces=len(surfaces9))
    clean = (len(ptiles9) == 0) and (len(surfaces9) == 0)
    verdicts["5_clean_detach_teardown"] = {
        "pass": clean,
        "portal_tiles_after_detach": ptiles9,
        "n_portal_surfaces_after_detach": len(surfaces9),
    }

    d.log("scenario:end", verdicts=verdicts)

    with open(os.path.join(args.outdir, "logs", "timeline.json"), "w") as f:
        json.dump(d.timeline, f, indent=1)
    with open(os.path.join(args.outdir, "logs", "verdicts.json"), "w") as f:
        json.dump(verdicts, f, indent=1)
    print("\n=== VERDICTS ===")
    print(json.dumps(verdicts, indent=1))
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", required=True, help="VM ip (gRPC 50051)")
    ap.add_argument("--mcp-url", required=True, help="http://<host>:9090")
    ap.add_argument("--psk", required=True)
    ap.add_argument("--agent-id", default="agent-alpha", help="observer/registered agent id")
    ap.add_argument("--projection-id", default="bridge-routing-live")
    ap.add_argument("--outdir", default=_HERE)
    ap.add_argument("--settle-s", type=float, default=6.0)
    ap.add_argument("--stream-gap-s", type=float, default=1.5)
    ap.add_argument("--unread-bursts", type=int, default=4)
    return run(ap.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
