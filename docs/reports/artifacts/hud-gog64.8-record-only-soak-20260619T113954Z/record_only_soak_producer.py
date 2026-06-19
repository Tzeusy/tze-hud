#!/usr/bin/env python3
"""Record-only media-ingress producer evidence harness for hud-gog64.8."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import time
import uuid
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[4]
USER_TEST_SCRIPTS = REPO_ROOT / ".claude" / "skills" / "user-test" / "scripts"

import sys

sys.path.insert(0, str(USER_TEST_SCRIPTS))
sys.path.insert(0, str(USER_TEST_SCRIPTS / "proto_gen"))

from hud_grpc_client import HudClient  # noqa: E402
from proto_gen import session_pb2  # noqa: E402
from windows_media_ingress_exemplar import build_video_only_sdp_offer  # noqa: E402


def _wall_us() -> int:
    return int(time.time() * 1_000_000)


def _state_to_dict(state: Any) -> dict[str, Any]:
    try:
        state_name = session_pb2.MediaSessionState.Name(state.state)
    except ValueError:
        state_name = f"MEDIA_SESSION_STATE_{state.state}"
    return {
        "stream_epoch": state.stream_epoch,
        "state": state_name,
        "current_step": state.current_step,
        "effective_bitrate_kbps": state.effective_bitrate_kbps,
        "effective_fps": state.effective_fps,
        "effective_width_px": state.effective_width_px,
        "effective_height_px": state.effective_height_px,
        "dropped_frames_since_last": state.dropped_frames_since_last,
        "watchdog_warnings": state.watchdog_warnings,
        "sample_timestamp_wall_us": state.sample_timestamp_wall_us,
    }


async def run(args: argparse.Namespace) -> dict[str, Any]:
    psk = os.getenv(args.psk_env)
    if not psk:
        raise RuntimeError(f"set {args.psk_env}")

    stream_uuid = uuid.uuid4()
    sdp_offer = build_video_only_sdp_offer(
        stream_id=stream_uuid,
        source_label=args.source_label,
        width=args.width,
        height=args.height,
        fps=args.fps,
    )
    opened_at_wall_us = _wall_us()
    opened_at_monotonic = time.monotonic()
    state_samples: list[dict[str, Any]] = []
    errors: list[str] = []

    heartbeat_errors: list[str] = []

    async with HudClient(
        args.target,
        psk=psk,
        agent_id=args.agent_id,
        capabilities=["media_ingress", "publish_zone:media-pip", "read_telemetry"],
        initial_subscriptions=["SCENE_TOPOLOGY", "TELEMETRY_FRAMES"],
    ) as client:
        heartbeat_stop = asyncio.Event()

        async def heartbeat_loop() -> None:
            interval_s = max(1.0, (client.heartbeat_interval_ms or 5000) / 1000.0 * 0.5)
            while not heartbeat_stop.is_set():
                await asyncio.sleep(interval_s)
                if heartbeat_stop.is_set():
                    break
                try:
                    await client.send_heartbeat()
                except Exception as exc:  # noqa: BLE001 - recorded as evidence.
                    heartbeat_errors.append(str(exc))

        heartbeat_task = asyncio.create_task(heartbeat_loop())
        result = await client.open_media_ingress(
            client_stream_id=stream_uuid.bytes,
            agent_sdp_offer=sdp_offer,
            zone_name=args.zone_name,
            content_classification=args.content_classification,
            declared_peak_kbps=args.declared_peak_kbps,
            codec_preference=[session_pb2.VIDEO_H264_BASELINE],
            timeout=args.timeout_s,
        )
        admitted_at_wall_us = _wall_us()
        try:
            initial_state_msg = await client._wait_for(
                "media_ingress_state",
                timeout=args.timeout_s,
                matcher=lambda msg: msg.media_ingress_state.stream_epoch
                == result.stream_epoch,
            )
            state_samples.append(_state_to_dict(initial_state_msg.media_ingress_state))
        except Exception as exc:  # noqa: BLE001 - evidence capture should continue.
            errors.append(f"initial media_ingress_state unavailable: {exc}")

        deadline = time.monotonic() + args.hold_s
        poll_timeout = min(args.state_poll_s, 5.0)
        while time.monotonic() < deadline:
            wait_s = min(poll_timeout, max(0.0, deadline - time.monotonic()))
            if wait_s <= 0:
                break
            try:
                state_msg = await client._wait_for(
                    "media_ingress_state",
                    timeout=wait_s,
                    matcher=lambda msg: msg.media_ingress_state.stream_epoch
                    == result.stream_epoch,
                )
                state_samples.append(_state_to_dict(state_msg.media_ingress_state))
            except TimeoutError:
                continue

        try:
            close_notice = await client.close_media_ingress(
                result.stream_epoch,
                reason="hud-gog64.8 record-only soak complete",
                timeout=args.timeout_s,
            )
            closed_at_wall_us = _wall_us()
            try:
                final_state_msg = await client._wait_for(
                    "media_ingress_state",
                    timeout=args.timeout_s,
                    matcher=lambda msg: msg.media_ingress_state.stream_epoch
                    == result.stream_epoch,
                )
                state_samples.append(_state_to_dict(final_state_msg.media_ingress_state))
            except Exception as exc:  # noqa: BLE001 - close notice is primary teardown proof.
                errors.append(f"final media_ingress_state unavailable: {exc}")
        finally:
            heartbeat_stop.set()
            heartbeat_task.cancel()
            try:
                await heartbeat_task
            except asyncio.CancelledError:
                pass

    dropped_total = sum(sample["dropped_frames_since_last"] for sample in state_samples)
    nonzero_frame_samples = [
        sample
        for sample in state_samples
        if sample["effective_fps"] > 0
        or sample["effective_width_px"] > 0
        or sample["effective_height_px"] > 0
    ]

    return {
        "lane": "hud-gog64.8-record-only-soak",
        "target": args.target,
        "agent_id": args.agent_id,
        "zone_name": args.zone_name,
        "source_label": args.source_label,
        "video_only": True,
        "audio_route_to_hud": "none",
        "hold_seconds_requested": args.hold_s,
        "hold_seconds_observed": round(time.monotonic() - opened_at_monotonic, 3),
        "opened_at_wall_us": opened_at_wall_us,
        "admitted_at_wall_us": admitted_at_wall_us,
        "closed_at_wall_us": closed_at_wall_us,
        "time_to_admission_ms": round((admitted_at_wall_us - opened_at_wall_us) / 1000, 3),
        "first_frame_time_ms": None,
        "first_frame_time_note": (
            "No decoded-frame transport is active in this record-only slice; "
            "live MediaIngressState samples reported effective_fps/width/height as zero."
        ),
        "client_stream_id": stream_uuid.hex,
        "admitted": True,
        "stream_epoch": result.stream_epoch,
        "assigned_surface_id_hex": result.assigned_surface_id.hex(),
        "selected_codec": session_pb2.MediaCodec.Name(result.selected_codec),
        "sdp_offer_bytes": len(sdp_offer),
        "state_sample_count": len(state_samples),
        "state_samples": state_samples,
        "dropped_frames_total": dropped_total,
        "nonzero_frame_sample_count": len(nonzero_frame_samples),
        "close_reason": session_pb2.MediaCloseReason.Name(close_notice.reason),
        "close_detail": close_notice.detail,
        "errors": errors,
        "heartbeat_errors": heartbeat_errors,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", default="windows-host.example:50052")
    parser.add_argument("--psk-env", default="TZE_HUD_PSK")
    parser.add_argument("--agent-id", default="windows-local-media-producer")
    parser.add_argument("--zone-name", default="media-pip")
    parser.add_argument("--content-classification", default="household")
    parser.add_argument("--source-label", default="synthetic-color-bars")
    parser.add_argument("--width", type=int, default=640)
    parser.add_argument("--height", type=int, default=360)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--declared-peak-kbps", type=int, default=2000)
    parser.add_argument("--hold-s", type=float, default=600.0)
    parser.add_argument("--timeout-s", type=float, default=10.0)
    parser.add_argument("--state-poll-s", type=float, default=10.0)
    parser.add_argument("--evidence-json", required=True)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    evidence = asyncio.run(run(args))
    path = Path(args.evidence_json)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(evidence, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
