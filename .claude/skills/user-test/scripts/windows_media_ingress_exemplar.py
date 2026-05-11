#!/usr/bin/env python3
"""
Windows media ingress exemplar producer and YouTube source-evidence sidecar.

The HUD lane uses a self-owned/local synthetic source and opens the runtime's
video-only MediaIngressOpen path. The YouTube lane launches the requested video
through the official embedded player URL as source evidence only; it does not
download, extract, capture, or bridge YouTube frames into the HUD runtime.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import subprocess
import sys
import time
import uuid
import webbrowser
from pathlib import Path
from typing import Any

_SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPT_DIR))
sys.path.insert(0, str(_SCRIPT_DIR / "proto_gen"))

from hud_grpc_client import HudClient  # noqa: E402
from proto_gen import session_pb2  # noqa: E402


YOUTUBE_VIDEO_ID = "O0FGCxkHM-U"
YOUTUBE_EMBED_URL = f"https://www.youtube.com/embed/{YOUTUBE_VIDEO_ID}"
APPROVED_MEDIA_ZONE = "media-pip"
LOCAL_PRODUCER_AGENT_ID = "windows-local-media-producer"
DEFAULT_SOURCE_LABEL = "synthetic-color-bars"
RAW_YOUTUBE_BRIDGE_DECISION = "blocked_pending_policy_approval"
BANNED_SOURCE_MARKERS = (
    "yt-dlp",
    "youtube-dl",
    "googlevideo.com",
    "videoplayback",
    "download",
    "direct media url",
)


def build_video_only_sdp_offer(
    *,
    stream_id: uuid.UUID,
    source_label: str = DEFAULT_SOURCE_LABEL,
    width: int = 640,
    height: int = 360,
    fps: int = 30,
) -> bytes:
    """Build a minimal video-only WebRTC SDP offer for admission proof."""
    if width <= 0 or height <= 0 or fps <= 0:
        raise ValueError("width, height, and fps must be positive")
    safe_label = "".join(ch if ch.isalnum() or ch in "-_." else "-" for ch in source_label)
    ice_ufrag = stream_id.hex[:16]
    ice_pwd = stream_id.hex + stream_id.hex[:8]
    ssrc = int.from_bytes(stream_id.bytes[:4], "big") or 1
    lines = [
        "v=0",
        f"o=tze-hud-local-producer 0 0 IN IP4 127.0.0.1",
        f"s=tze_hud {safe_label}",
        "t=0 0",
        "a=group:BUNDLE 0",
        "a=msid-semantic: WMS tze-hud-local-source",
        "m=video 9 UDP/TLS/RTP/SAVPF 96",
        "c=IN IP4 0.0.0.0",
        "a=mid:0",
        "a=sendonly",
        "a=rtcp-mux",
        "a=rtcp-rsize",
        f"a=ice-ufrag:{ice_ufrag}",
        f"a=ice-pwd:{ice_pwd}",
        "a=fingerprint:sha-256 "
        "00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:"
        "00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00",
        "a=setup:actpass",
        "a=rtpmap:96 H264/90000",
        "a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
        f"a=framerate:{fps}",
        f"a=framesize:96 {width}-{height}",
        f"a=msid:tze-hud-local-source {safe_label}",
        f"a=ssrc:{ssrc} cname:tze-hud-local-producer",
        "",
    ]
    return "\r\n".join(lines).encode("utf-8")


def build_source_evidence_html(video_id: str = YOUTUBE_VIDEO_ID) -> str:
    """Return a small external-player evidence page using the official embed URL."""
    embed_url = f"https://www.youtube.com/embed/{video_id}"
    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="referrer" content="strict-origin-when-cross-origin">
  <title>tze_hud YouTube source evidence</title>
  <style>
    html, body {{ margin: 0; height: 100%; background: #111; }}
    iframe {{ width: 100vw; height: 100vh; border: 0; }}
  </style>
</head>
<body>
  <iframe
    id="youtube-source-evidence"
    width="960"
    height="540"
    src="{embed_url}"
    allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
    allowfullscreen>
  </iframe>
</body>
</html>
"""


def policy_review() -> dict[str, Any]:
    """Machine-readable review for the YouTube bridge boundary."""
    return {
        "youtube_video_id": YOUTUBE_VIDEO_ID,
        "official_player_url": YOUTUBE_EMBED_URL,
        "raw_youtube_frame_bridge": RAW_YOUTUBE_BRIDGE_DECISION,
        "hud_ingress_source": "self-owned/local synthetic video-only source",
        "audio_route_to_hud": "none",
        "prohibited_paths": list(BANNED_SOURCE_MARKERS),
        "rationale": (
            "First acceptance separates HUD media-ingress proof from YouTube "
            "source evidence. YouTube launches only through the official embedded "
            "player URL; raw-frame bridging remains blocked until a separate "
            "policy review approves a compliant bridge."
        ),
    }


async def run_local_producer(args: argparse.Namespace) -> dict[str, Any]:
    psk = args.psk or os.getenv(args.psk_env)
    if not psk:
        raise RuntimeError(f"set {args.psk_env} or pass --psk")

    stream_uuid = uuid.uuid4()
    sdp_offer = build_video_only_sdp_offer(
        stream_id=stream_uuid,
        source_label=args.source_label,
        width=args.width,
        height=args.height,
        fps=args.fps,
    )
    async with HudClient(
        args.target,
        psk=psk,
        agent_id=args.agent_id,
        capabilities=["media_ingress", "publish_zone:media-pip", "read_telemetry"],
        initial_subscriptions=["SCENE_TOPOLOGY"],
    ) as client:
        result = await client.open_media_ingress(
            client_stream_id=stream_uuid.bytes,
            agent_sdp_offer=sdp_offer,
            zone_name=args.zone_name,
            content_classification=args.content_classification,
            declared_peak_kbps=args.declared_peak_kbps,
            codec_preference=[session_pb2.VIDEO_H264_BASELINE],
            timeout=args.timeout_s,
        )
        started_at = int(time.time())
        if args.hold_s > 0:
            await asyncio.sleep(args.hold_s)
        close_notice = await client.close_media_ingress(
            result.stream_epoch,
            reason="local producer complete",
            timeout=args.timeout_s,
        )

    return {
        "lane": "hud-media-ingress-local-producer",
        "target": args.target,
        "agent_id": args.agent_id,
        "source_label": args.source_label,
        "video_only": True,
        "audio_route_to_hud": "none",
        "zone_name": args.zone_name,
        "content_classification": args.content_classification,
        "declared_peak_kbps": args.declared_peak_kbps,
        "client_stream_id": stream_uuid.hex,
        "admitted": True,
        "stream_epoch": result.stream_epoch,
        "assigned_surface_id_hex": result.assigned_surface_id.hex(),
        "selected_codec": session_pb2.MediaCodec.Name(result.selected_codec),
        "sdp_offer_bytes": len(sdp_offer),
        "started_at_unix": started_at,
        "held_seconds": args.hold_s,
        "close_reason": session_pb2.MediaCloseReason.Name(close_notice.reason),
    }


def launch_youtube_sidecar(args: argparse.Namespace) -> dict[str, Any]:
    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    html_path = output_dir / "youtube_source_evidence.html"
    html_path.write_text(build_source_evidence_html(args.video_id), encoding="utf-8")
    official_url = f"https://www.youtube.com/embed/{args.video_id}"

    launched_by = "dry-run"
    if not args.dry_run:
        if args.windows_host:
            cmd = [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                f"ConnectTimeout={args.connect_timeout_s}",
            ]
            if args.ssh_key:
                cmd.extend(["-i", args.ssh_key])
            cmd.append(f"{args.windows_user}@{args.windows_host}")
            cmd.append(
                "powershell -NoProfile -Command "
                f"\"Start-Process '{official_url}'\""
            )
            subprocess.run(cmd, check=True)
            launched_by = f"ssh:{args.windows_user}@{args.windows_host}"
        else:
            webbrowser.open(official_url, new=1, autoraise=True)
            launched_by = "local-browser"

    return {
        "lane": "youtube-source-evidence",
        "video_id": args.video_id,
        "official_player_url": official_url,
        "html_evidence_path": str(html_path),
        "launched_by": launched_by,
        "raw_youtube_frame_bridge": RAW_YOUTUBE_BRIDGE_DECISION,
        "download_or_extraction": "not_used",
        "hud_runtime_receives_youtube_frames": False,
    }


def write_evidence(path: str | None, evidence: dict[str, Any]) -> None:
    payload = dict(evidence)
    review = policy_review()
    if payload != review:
        payload["policy_review"] = review
    text = json.dumps(payload, indent=2, sort_keys=True) + "\n"
    if path:
        Path(path).write_text(text, encoding="utf-8")
    print(text, end="")


def add_common_evidence_arg(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--evidence-json", help="optional JSON evidence output path")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    local = sub.add_parser("local-producer", help="open a video-only HUD media ingress stream")
    local.add_argument("--target", default="tzehouse-windows.parrot-hen.ts.net:50051")
    local.add_argument("--psk-env", default="TZE_HUD_PSK")
    local.add_argument("--psk", help="PSK value; prefer --psk-env for normal runs")
    local.add_argument("--agent-id", default=LOCAL_PRODUCER_AGENT_ID)
    local.add_argument("--zone-name", default=APPROVED_MEDIA_ZONE)
    local.add_argument("--content-classification", default="household")
    local.add_argument("--source-label", default=DEFAULT_SOURCE_LABEL)
    local.add_argument("--width", type=int, default=640)
    local.add_argument("--height", type=int, default=360)
    local.add_argument("--fps", type=int, default=30)
    local.add_argument("--declared-peak-kbps", type=int, default=2_000)
    local.add_argument("--hold-s", type=float, default=10.0)
    local.add_argument("--timeout-s", type=float, default=10.0)
    add_common_evidence_arg(local)

    youtube = sub.add_parser("youtube-sidecar", help="launch official YouTube embed evidence")
    youtube.add_argument("--video-id", default=YOUTUBE_VIDEO_ID)
    youtube.add_argument("--output-dir", default="build/windows-media-ingress")
    youtube.add_argument("--dry-run", action="store_true")
    youtube.add_argument("--windows-host")
    youtube.add_argument("--windows-user", default="tzeus")
    youtube.add_argument("--ssh-key")
    youtube.add_argument("--connect-timeout-s", type=int, default=5)
    add_common_evidence_arg(youtube)

    review = sub.add_parser("policy-review", help="print the YouTube bridge policy decision")
    add_common_evidence_arg(review)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "local-producer":
        evidence = asyncio.run(run_local_producer(args))
    elif args.command == "youtube-sidecar":
        evidence = launch_youtube_sidecar(args)
    elif args.command == "policy-review":
        evidence = policy_review()
    else:
        raise AssertionError(args.command)
    write_evidence(getattr(args, "evidence_json", None), evidence)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
