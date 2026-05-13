#!/usr/bin/env python3
"""
Windows media ingress exemplar producer and YouTube source-evidence sidecar.

The baseline HUD lane uses a self-owned/local synthetic source and opens the
runtime's video-only MediaIngressOpen path. The approved YouTube bridge lane
keeps the official player/control surface operator-visible, names the Windows
raw-frame bridge path, and enters the HUD runtime only through MediaIngressOpen.
It does not download, extract, cache, route audio, or host a browser/WebView
inside the compositor.
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import os
import re
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
YOUTUBE_VIDEO_ID_RE = re.compile(r"^[A-Za-z0-9_-]{11}$")
APPROVED_MEDIA_ZONE = "media-pip"
LOCAL_PRODUCER_AGENT_ID = "windows-local-media-producer"
YOUTUBE_BRIDGE_AGENT_ID = "windows-youtube-frame-bridge"
DEFAULT_SOURCE_LABEL = "synthetic-color-bars"
YOUTUBE_BRIDGE_SOURCE_LABEL = "youtube-official-player-frame-bridge"
YOUTUBE_BRIDGE_PATH = "operator-visible-official-player-window-capture-to-media-ingress-open"
RAW_YOUTUBE_BRIDGE_DECISION = "approved_operator_visible_player_frame_bridge"
BANNED_SOURCE_MARKERS = (
    "yt-dlp",
    "youtube-dl",
    "googlevideo.com",
    "videoplayback",
    "download",
    "cache",
    "direct media url",
    "audio route",
)


def validate_youtube_video_id(video_id: str) -> str:
    """Return a valid YouTube video id or raise before shell/browser use."""
    if not YOUTUBE_VIDEO_ID_RE.fullmatch(video_id):
        raise ValueError("video_id must match the 11-character YouTube id format")
    return video_id


def validate_ssh_arg(name: str, value: str) -> str:
    """Reject values that OpenSSH could parse as options."""
    if not value:
        raise ValueError(f"{name} is required")
    if value.startswith("-"):
        raise ValueError(f"{name} must not start with '-'")
    return value


def validate_approved_media_zone(zone_name: str) -> str:
    """Return the only currently approved media ingress zone."""
    if zone_name != APPROVED_MEDIA_ZONE:
        raise ValueError(
            f"media ingress exemplar only supports approved zone {APPROVED_MEDIA_ZONE!r}"
        )
    return zone_name


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
    video_id = validate_youtube_video_id(video_id)
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
        "bridge_path_name": YOUTUBE_BRIDGE_PATH,
        "hud_ingress_source": (
            "official-player raw-frame bridge or self-owned/local fallback; "
            "both enter through MediaIngressOpen"
        ),
        "operator_visible_player_controls": "required",
        "audio_route_to_hud": "none",
        "prohibited_paths": list(BANNED_SOURCE_MARKERS),
        "rationale": (
            "Operator/maintainer approval recorded on 2026-05-12 permits a "
            "narrow Windows-only raw-frame bridge from an operator-visible "
            "official YouTube player sidecar into the HUD media ingress path. "
            "The bridge remains video-only and does not download, extract, "
            "cache, route audio, or embed a browser in the compositor."
        ),
    }


async def run_local_producer(args: argparse.Namespace) -> dict[str, Any]:
    psk = args.psk or os.getenv(args.psk_env)
    if not psk:
        raise RuntimeError(f"set {args.psk_env} or pass --psk")
    zone_name = validate_approved_media_zone(args.zone_name)

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
            zone_name=zone_name,
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
        "media_ingress_entrypoint": "MediaIngressOpen",
        "video_only": True,
        "audio_route_to_hud": "none",
        "zone_name": zone_name,
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
    video_id = validate_youtube_video_id(args.video_id)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    html_path = output_dir / "youtube_source_evidence.html"
    html = build_source_evidence_html(video_id)
    html_path.write_text(html, encoding="utf-8")
    official_url = f"https://www.youtube.com/embed/{video_id}"

    launched_by = "dry-run"
    if not args.dry_run:
        if args.windows_host:
            windows_user = validate_ssh_arg("windows_user", args.windows_user)
            windows_host = validate_ssh_arg("windows_host", args.windows_host)
            cmd = [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                f"ConnectTimeout={args.connect_timeout_s}",
                "-l",
                windows_user,
            ]
            if args.ssh_key:
                cmd.extend(["-i", args.ssh_key])
            cmd.append(windows_host)
            remote_html_b64 = base64.b64encode(html.encode("utf-8")).decode("ascii")
            remote_script = (
                f"$htmlBytes=[Convert]::FromBase64String('{remote_html_b64}');"
                "$html=[Text.Encoding]::UTF8.GetString($htmlBytes);"
                "$path=Join-Path $env:TEMP 'tze_hud_youtube_source_evidence.html';"
                "Set-Content -LiteralPath $path -Value $html -Encoding UTF8;"
                "Start-Process -FilePath $path"
            )
            encoded_script = base64.b64encode(remote_script.encode("utf-16le")).decode(
                "ascii"
            )
            cmd.extend(["powershell", "-NoProfile", "-EncodedCommand", encoded_script])
            subprocess.run(cmd, check=True)
            launched_by = f"ssh:{windows_user}@{windows_host}"
        else:
            webbrowser.open(html_path.resolve().as_uri(), new=1, autoraise=True)
            launched_by = "local-browser"

    return {
        "lane": "youtube-source-evidence",
        "video_id": video_id,
        "official_player_url": official_url,
        "html_evidence_path": str(html_path),
        "launched_by": launched_by,
        "raw_youtube_frame_bridge": RAW_YOUTUBE_BRIDGE_DECISION,
        "download_or_extraction": "not_used",
        "cache_or_offline_copy": "not_used",
        "audio_route_to_hud": "none",
        "operator_visible_player_controls": True,
        "hud_runtime_receives_youtube_frames": False,
    }


def build_youtube_bridge_dry_run_evidence(
    *,
    sidecar_evidence: dict[str, Any],
    target: str,
    agent_id: str,
    zone_name: str,
) -> dict[str, Any]:
    """Return evidence for the approved bridge path without opening gRPC."""
    return {
        "lane": "youtube-official-player-frame-bridge",
        "video_id": sidecar_evidence["video_id"],
        "official_player_url": sidecar_evidence["official_player_url"],
        "bridge_path_name": YOUTUBE_BRIDGE_PATH,
        "bridge_source": "operator-visible official YouTube player sidecar",
        "bridge_sink": "HUD media ingress approved zone",
        "media_ingress_entrypoint": "MediaIngressOpen",
        "target": target,
        "agent_id": agent_id,
        "zone_name": zone_name,
        "video_only": True,
        "operator_visible_player_controls": True,
        "download_or_extraction": "not_used",
        "cache_or_offline_copy": "not_used",
        "audio_route_to_hud": "none",
        "media_ingress_open_attempted": False,
        "media_ingress_open_admitted": False,
        "hud_runtime_receives_youtube_frames": False,
        "blocked_reason": (
            "dry-run; live proof requires the Windows frame-capture adapter and "
            "exclusive validation access"
        ),
        "sidecar": sidecar_evidence,
    }


async def run_youtube_bridge(args: argparse.Namespace) -> dict[str, Any]:
    """Launch the approved sidecar and open the bridge's MediaIngressOpen lane."""
    zone_name = validate_approved_media_zone(args.zone_name)
    if args.media_ingress_dry_run:
        sidecar_evidence = launch_youtube_sidecar(args)
        return build_youtube_bridge_dry_run_evidence(
            sidecar_evidence=sidecar_evidence,
            target=args.target,
            agent_id=args.agent_id,
            zone_name=zone_name,
        )

    raise RuntimeError(
        "live YouTube bridge proof requires a Windows frame-capture adapter from "
        "the operator-visible official player sidecar into MediaIngressOpen; "
        "do not substitute the local/synthetic producer for YouTube frame proof"
    )


def write_evidence(path: str | None, evidence: dict[str, Any]) -> None:
    payload = dict(evidence)
    review = policy_review()
    if payload != review:
        payload["policy_review"] = review
    text = json.dumps(payload, indent=2, sort_keys=True) + "\n"
    if path:
        output_path = Path(path)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text, encoding="utf-8")
    print(text, end="")


def add_common_evidence_arg(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--evidence-json", help="optional JSON evidence output path")


def add_media_ingress_args(
    parser: argparse.ArgumentParser,
    *,
    default_agent_id: str = LOCAL_PRODUCER_AGENT_ID,
    default_source_label: str = DEFAULT_SOURCE_LABEL,
) -> None:
    parser.add_argument("--target", default="tzehouse-windows.parrot-hen.ts.net:50051")
    parser.add_argument("--psk-env", default="TZE_HUD_PSK")
    parser.add_argument("--psk", help="PSK value; prefer --psk-env for normal runs")
    parser.add_argument("--agent-id", default=default_agent_id)
    parser.add_argument("--zone-name", default=APPROVED_MEDIA_ZONE)
    parser.add_argument("--content-classification", default="household")
    parser.add_argument("--source-label", default=default_source_label)
    parser.add_argument("--width", type=int, default=640)
    parser.add_argument("--height", type=int, default=360)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--declared-peak-kbps", type=int, default=2_000)
    parser.add_argument("--hold-s", type=float, default=10.0)
    parser.add_argument("--timeout-s", type=float, default=10.0)


def add_youtube_sidecar_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--video-id", default=YOUTUBE_VIDEO_ID)
    parser.add_argument("--output-dir", default="build/windows-media-ingress")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--windows-host")
    parser.add_argument("--windows-user", default="tzeus")
    parser.add_argument("--ssh-key")
    parser.add_argument("--connect-timeout-s", type=int, default=5)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    local = sub.add_parser("local-producer", help="open a video-only HUD media ingress stream")
    add_media_ingress_args(local)
    add_common_evidence_arg(local)

    youtube = sub.add_parser("youtube-sidecar", help="launch official YouTube embed evidence")
    add_youtube_sidecar_args(youtube)
    add_common_evidence_arg(youtube)

    bridge = sub.add_parser(
        "youtube-bridge",
        help="launch the official player sidecar and open the approved MediaIngressOpen lane",
    )
    add_youtube_sidecar_args(bridge)
    add_media_ingress_args(
        bridge,
        default_agent_id=YOUTUBE_BRIDGE_AGENT_ID,
        default_source_label=YOUTUBE_BRIDGE_SOURCE_LABEL,
    )
    bridge.add_argument(
        "--media-ingress-dry-run",
        action="store_true",
        help="name the approved bridge path without authenticating to the HUD",
    )
    add_common_evidence_arg(bridge)

    review = sub.add_parser("policy-review", help="print the YouTube bridge policy decision")
    add_common_evidence_arg(review)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "local-producer":
        evidence = asyncio.run(run_local_producer(args))
    elif args.command == "youtube-sidecar":
        evidence = launch_youtube_sidecar(args)
    elif args.command == "youtube-bridge":
        evidence = asyncio.run(run_youtube_bridge(args))
    elif args.command == "policy-review":
        evidence = policy_review()
    else:
        raise AssertionError(args.command)
    write_evidence(getattr(args, "evidence_json", None), evidence)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
