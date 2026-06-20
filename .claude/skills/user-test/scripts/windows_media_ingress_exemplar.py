#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = [
#   "grpcio>=1.80.0",
#   "protobuf>=6.31.1",
# ]
# ///
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
import base64
import http.server
import json
import math
import os
import re
import socket
import subprocess
import sys
import threading
import time
import uuid
import webbrowser
from pathlib import Path
from typing import Any

_SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPT_DIR))
sys.path.insert(0, str(_SCRIPT_DIR / "proto_gen"))

from hud_grpc_client import HudClient, MediaIngressRejected  # noqa: E402
from proto_gen import session_pb2  # noqa: E402


YOUTUBE_VIDEO_ID = "O0FGCxkHM-U"
YOUTUBE_EMBED_URL = f"https://www.youtube.com/embed/{YOUTUBE_VIDEO_ID}"
YOUTUBE_VIDEO_ID_RE = re.compile(r"^[A-Za-z0-9_-]{11}$")
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


# ---------------------------------------------------------------------------
# Frame-capture validator
# ---------------------------------------------------------------------------
# Minimum cross-frame luminance spread required to accept captured frames as
# real playback rather than a static page (error screen, loading screen, etc.).
# Chosen to pass real video (large inter-frame swings observed, e.g.
# [46,32,22]→[7,7,9]) while rejecting a static YouTube Error-153 page
# (mean_rgb ~[39,41,44] with zero or near-zero frame-to-frame variation).
_FRAME_LUMINANCE_SPREAD_THRESHOLD = 1.0  # minimum stddev of per-frame luma (0-255 scale)
# Minimum sum(mean_rgb) for a frame to count as non-blank.
_FRAME_NONBLANK_SUM_THRESHOLD = 3.0


def _frame_luminance(mean_rgb: list[float]) -> float:
    """Return BT.601 luminance for an [R, G, B] mean_rgb sample (0-255 scale)."""
    r, g, b = mean_rgb[0], mean_rgb[1], mean_rgb[2]
    return 0.299 * r + 0.587 * g + 0.114 * b


def _validate_official_player_frames(
    frames: list[dict[str, Any]],
) -> dict[str, Any]:
    """Validate a list of captured frame descriptors as genuine playback frames.

    Each frame dict must contain:
      - ``mean_rgb``: list of three floats (R, G, B channel means, 0-255 scale)
      - ``sha256``:   hex-string hash of the raw frame data

    Returns a dict with:
      - ``capture_validated``: bool — True only when all three checks pass
      - ``frame_count``: int
      - ``distinct_hashes``: int
      - ``mean_luma``: float (mean luminance across frames)
      - ``luma_stddev``: float (stddev of per-frame luminance)
      - ``rejection_reason``: str | None — populated when capture_validated is False

    Validation requires ALL of:
    1. Non-blank: at least one frame has sum(mean_rgb) > 3.0 (not pure-black).
    2. Distinct hashes: >= 2 distinct sha256 hashes (not a single frozen frame).
    3. Content variance: per-frame luminance stddev >= threshold (not a static
       page — e.g. a YouTube Error-153 dark screen whose frames have near-zero
       cross-frame luminance variation even though their hashes may differ due
       to a blinking cursor or spinner).
    """
    if not frames:
        return {
            "capture_validated": False,
            "frame_count": 0,
            "distinct_hashes": 0,
            "mean_luma": 0.0,
            "luma_stddev": 0.0,
            "rejection_reason": "no frames captured",
        }

    frame_count = len(frames)
    hashes = {f["sha256"] for f in frames}
    distinct_hashes = len(hashes)
    luma_values = [_frame_luminance(f["mean_rgb"]) for f in frames]
    mean_luma = sum(luma_values) / frame_count
    variance = sum((luma - mean_luma) ** 2 for luma in luma_values) / frame_count
    luma_stddev = math.sqrt(variance)

    # Check 1: not pure-black
    nonblank = any(sum(f["mean_rgb"]) > _FRAME_NONBLANK_SUM_THRESHOLD for f in frames)
    if not nonblank:
        return {
            "capture_validated": False,
            "frame_count": frame_count,
            "distinct_hashes": distinct_hashes,
            "mean_luma": mean_luma,
            "luma_stddev": luma_stddev,
            "rejection_reason": (
                f"all frames appear blank (sum(mean_rgb) <= {_FRAME_NONBLANK_SUM_THRESHOLD})"
            ),
        }

    # Check 2: distinct hashes (not a single frozen frame repeated)
    if distinct_hashes < 2:
        return {
            "capture_validated": False,
            "frame_count": frame_count,
            "distinct_hashes": distinct_hashes,
            "mean_luma": mean_luma,
            "luma_stddev": luma_stddev,
            "rejection_reason": "fewer than 2 distinct frame hashes — capture appears frozen",
        }

    # Check 3: meaningful cross-frame luminance variance
    if luma_stddev < _FRAME_LUMINANCE_SPREAD_THRESHOLD:
        return {
            "capture_validated": False,
            "frame_count": frame_count,
            "distinct_hashes": distinct_hashes,
            "mean_luma": mean_luma,
            "luma_stddev": luma_stddev,
            "rejection_reason": (
                f"near-static frame content: luminance stddev {luma_stddev:.4f} "
                f"< threshold {_FRAME_LUMINANCE_SPREAD_THRESHOLD} — "
                "likely a static error or loading page rather than real playback"
            ),
        }

    return {
        "capture_validated": True,
        "frame_count": frame_count,
        "distinct_hashes": distinct_hashes,
        "mean_luma": mean_luma,
        "luma_stddev": luma_stddev,
        "rejection_reason": None,
    }


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
        "o=tze-hud-local-producer 0 0 IN IP4 127.0.0.1",
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


def build_source_evidence_html(
    video_id: str = YOUTUBE_VIDEO_ID,
    origin_port: int | None = None,
) -> str:
    """Return a small external-player evidence page using the official embed URL.

    When *origin_port* is provided the embed URL includes ``?origin=http://127.0.0.1:<port>``
    so that the YouTube IFrame player receives a valid HTTP origin instead of a
    null/file:// origin.  Pass the port chosen by the local HTTP server (see
    :func:`_start_local_http_server`).
    """
    video_id = validate_youtube_video_id(video_id)
    if origin_port is not None:
        origin = f"http://127.0.0.1:{origin_port}"
        embed_url = f"https://www.youtube.com/embed/{video_id}?origin={origin}&autoplay=1"
    else:
        origin = ""
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
    zone_name = validate_approved_media_zone(args.zone_name)

    stream_uuid = uuid.uuid4()
    sdp_offer = build_video_only_sdp_offer(
        stream_id=stream_uuid,
        source_label=args.source_label,
        width=args.width,
        height=args.height,
        fps=args.fps,
    )
    expect_reject = getattr(args, "expect_reject_code", None)
    async with HudClient(
        args.target,
        psk=psk,
        agent_id=args.agent_id,
        capabilities=["media_ingress", "publish_zone:media-pip", "read_telemetry"],
        initial_subscriptions=["SCENE_TOPOLOGY"],
    ) as client:
        if expect_reject:
            # Authenticated admission-rejection proof: the session must establish
            # (PSK accepted) and MediaIngressOpen must be rejected with exactly the
            # expected reject_code. A clean admission is a failure for this lane.
            try:
                result = await client.open_media_ingress(
                    client_stream_id=stream_uuid.bytes,
                    agent_sdp_offer=sdp_offer,
                    zone_name=zone_name,
                    content_classification=args.content_classification,
                    declared_peak_kbps=args.declared_peak_kbps,
                    codec_preference=[session_pb2.VIDEO_H264_BASELINE],
                    timeout=args.timeout_s,
                )
            except MediaIngressRejected as rejected:
                if rejected.reject_code != expect_reject:
                    raise RuntimeError(
                        "media ingress rejected with unexpected reject_code "
                        f"{rejected.reject_code!r} (expected {expect_reject!r}): "
                        f"{rejected.reject_reason}"
                    ) from rejected
                return {
                    "lane": "hud-media-ingress-local-producer",
                    "target": args.target,
                    "agent_id": args.agent_id,
                    "source_label": args.source_label,
                    "video_only": True,
                    "zone_name": zone_name,
                    "content_classification": args.content_classification,
                    "client_stream_id": stream_uuid.hex,
                    "authenticated": True,
                    "admitted": False,
                    "reject_code": rejected.reject_code,
                    "reject_reason": rejected.reject_reason,
                    "expected_reject_code": expect_reject,
                    "sdp_offer_bytes": len(sdp_offer),
                    "rejected_at_unix": int(time.time()),
                }
            raise RuntimeError(
                f"expected admission rejection {expect_reject!r} but stream was "
                f"admitted (epoch={result.stream_epoch})"
            )
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


def _pick_free_port() -> int:
    """Return an ephemeral TCP port that is free on 127.0.0.1 at the time of call."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _start_local_http_server(
    serve_dir: Path,
    port: int,
) -> http.server.HTTPServer:
    """Start a daemon HTTP server bound to 127.0.0.1:<port> serving *serve_dir*.

    Returns the running :class:`http.server.HTTPServer` instance.  The server
    runs in a daemon thread so it is automatically stopped when the process
    exits.  Callers that need explicit teardown can call ``server.shutdown()``.
    """

    class _Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a: Any, **kw: Any) -> None:
            super().__init__(*a, directory=str(serve_dir), **kw)

        def log_message(self, fmt: str, *args: Any) -> None:  # silence request log
            pass

    server = http.server.HTTPServer(("127.0.0.1", port), _Handler)
    t = threading.Thread(target=server.serve_forever, daemon=True)
    t.start()
    return server


def launch_youtube_sidecar(args: argparse.Namespace) -> dict[str, Any]:
    video_id = validate_youtube_video_id(args.video_id)
    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    html_filename = "youtube_source_evidence.html"
    html_path = output_dir / html_filename
    official_url = f"https://www.youtube.com/embed/{video_id}"

    launched_by = "dry-run"
    http_origin: str | None = None

    if args.dry_run:
        # Dry-run: generate HTML without an HTTP origin (no server is started).
        html = build_source_evidence_html(video_id)
        html_path.write_text(html, encoding="utf-8")
    elif args.windows_host:
        # Windows SSH path: spin up a PowerShell HttpListener on the remote host
        # so the YouTube IFrame player receives a valid HTTP origin.
        windows_user = validate_ssh_arg("windows_user", args.windows_user)
        windows_host = validate_ssh_arg("windows_host", args.windows_host)

        # The PowerShell script:
        # 1. Picks a free port on the Windows host.
        # 2. Generates the HTML with the correct ?origin= param baked in.
        # 3. Writes it under a temp sidecar directory.
        # 4. Starts an HttpListener on a .NET Thread (same process — HttpListener
        #    is not serializable across Start-Job process boundaries).
        # 5. Opens Chrome (or the default browser) at the http:// URL.
        # 6. Sleeps 5 s to serve the initial request, then stops the listener.
        ps_script = r"""
$port = (Get-NetTCPConnection -State Listen | Where-Object { $_.LocalAddress -eq '0.0.0.0' } | Measure-Object).Count
# Pick a free ephemeral port via TcpListener
$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()

$origin = "http://127.0.0.1:$port"
$embedUrl = "https://www.youtube.com/embed/""" + video_id + r"""?origin=$origin&autoplay=1"
$html = @"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="referrer" content="strict-origin-when-cross-origin">
  <title>tze_hud YouTube source evidence</title>
  <style>
    html, body { margin: 0; height: 100%; background: #111; }
    iframe { width: 100vw; height: 100vh; border: 0; }
  </style>
</head>
<body>
  <iframe
    id="youtube-source-evidence"
    width="960"
    height="540"
    src="$embedUrl"
    allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
    allowfullscreen>
  </iframe>
</body>
</html>
"@
$dir = Join-Path $env:TEMP "tze_hud_youtube_sidecar_$port"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$htmlPath = Join-Path $dir "youtube_source_evidence.html"
Set-Content -LiteralPath $htmlPath -Value $html -Encoding UTF8

$http = [System.Net.HttpListener]::new()
$http.Prefixes.Add("http://127.0.0.1:$port/")
$http.Start()
# Use a .NET Thread (same process) rather than Start-Job (separate process):
# HttpListener is not serializable across job process boundaries, so Start-Job
# receives a dead deserialized copy and cannot serve requests.
$serveDir = $dir
$t = [System.Threading.Thread]::new({
    while ($http.IsListening) {
        try {
            $ctx = $http.GetContext()
            $resp = $ctx.Response
            $rel = $ctx.Request.Url.AbsolutePath.TrimStart('/')
            if ($rel -eq '') { $rel = 'youtube_source_evidence.html' }
            $file = Join-Path $serveDir $rel
            if (Test-Path $file) {
                $bytes = [System.IO.File]::ReadAllBytes($file)
                $resp.ContentLength64 = $bytes.Length
                $resp.OutputStream.Write($bytes, 0, $bytes.Length)
            } else {
                $resp.StatusCode = 404
            }
            $resp.OutputStream.Close()
        } catch { }
    }
})
$t.IsBackground = $true
$t.Start()

Start-Process "http://127.0.0.1:$port/youtube_source_evidence.html"
Start-Sleep -Seconds 5
$http.Stop()
"""
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
        encoded_script = base64.b64encode(ps_script.encode("utf-16le")).decode("ascii")
        cmd.extend(["powershell", "-NoProfile", "-EncodedCommand", encoded_script])
        subprocess.run(cmd, check=True)
        launched_by = f"ssh:{windows_user}@{windows_host}"
        # HTML was generated dynamically on the Windows host; write a local copy
        # without the port (port is not known on the Linux side for Windows path).
        html = build_source_evidence_html(video_id)
        html_path.write_text(html, encoding="utf-8")
    else:
        # Local browser path: spin up a Python HTTP server on a free port so the
        # IFrame player receives a valid HTTP origin.
        port = _pick_free_port()
        http_origin = f"http://127.0.0.1:{port}"
        html = build_source_evidence_html(video_id, origin_port=port)
        html_path.write_text(html, encoding="utf-8")
        _start_local_http_server(output_dir, port)
        url = f"{http_origin}/{html_filename}"
        webbrowser.open(url, new=1, autoraise=True)
        launched_by = "local-browser-http"

    return {
        "lane": "youtube-source-evidence",
        "video_id": video_id,
        "official_player_url": official_url,
        "html_evidence_path": str(html_path),
        "launched_by": launched_by,
        "http_origin": http_origin,
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
    local.add_argument("--target", default="windows-host.example:50051")
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
    local.add_argument(
        "--expect-reject-code",
        help=(
            "Assert MediaIngressOpen is rejected with this reject_code "
            "(e.g. MEDIA_DISABLED). Exit 0 only if the authenticated session is "
            "established and admission is rejected with exactly this code; a clean "
            "admission or a different/absent rejection is a failure."
        ),
    )
    add_common_evidence_arg(local)

    youtube = sub.add_parser("youtube-sidecar", help="launch official YouTube embed evidence")
    youtube.add_argument("--video-id", default=YOUTUBE_VIDEO_ID)
    youtube.add_argument("--output-dir", default="build/windows-media-ingress")
    youtube.add_argument("--dry-run", action="store_true")
    youtube.add_argument("--windows-host")
    youtube.add_argument("--windows-user", default="admin-user")
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
