#!/usr/bin/env python3
"""
Presence Card exemplar user-test scenario.

Runs three resident gRPC sessions against a live HUD and walks the manual
Presence Card lifecycle the operator needs to observe:

1. Create 3 stacked cards in the bottom-left corner
2. Wait for the first periodic status update
3. Disconnect agent-gamma and observe orphan/badge state
4. Wait through the grace period and confirm only 2 cards remain

The script emits structured JSON step events to stdout and can also write a
machine-readable transcript file.

Current branch note: the resident session stream still does not expose the
RFC 0011 resource-upload messages, so this scenario renders the per-agent
avatar as a 32x32 solid-color square instead of a StaticImageNode upload.
That is sufficient for the operator-visible stacking/update/disconnect checks
that `hud-sx7q.3` covers.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

from hud_grpc_client import HudClient, _make_node
from proto_gen import types_pb2


CARD_W = 320.0
CARD_H = 112.0
CARD_GAP = 12.0
LEFT_MARGIN = 24.0
BOTTOM_MARGIN = 24.0

BG_RGBA = (0.10, 0.14, 0.19, 0.72)
SHEEN_RGBA = (0.92, 0.96, 1.0, 0.16)
EYEBROW_RGBA = (0.72, 0.80, 0.90, 0.82)
NAME_RGBA = (0.97, 0.99, 1.0, 1.0)
STATUS_RGBA = (0.82, 0.88, 0.94, 0.92)
CHIP_BG_RGBA = (0.86, 0.92, 1.0, 0.12)
CHIP_TEXT_RGBA = (0.96, 0.98, 1.0, 0.96)

SHEEN_X = 0.0
SHEEN_Y = 0.0
SHEEN_W = CARD_W
SHEEN_H = 2.0

ACCENT_X = 0.0
ACCENT_Y = 18.0
ACCENT_W = 4.0
ACCENT_H = 76.0

AVATAR_PLATE_X = 24.0
AVATAR_PLATE_Y = 28.0
AVATAR_PLATE_W = 56.0
AVATAR_PLATE_H = 56.0

AVATAR_X = 34.0
AVATAR_Y = 38.0
AVATAR_W = 36.0
AVATAR_H = 36.0

EYEBROW_X = 96.0
EYEBROW_Y = 18.0
EYEBROW_W = 152.0
EYEBROW_H = 12.0
EYEBROW_FONT_SIZE_PX = 11.0

NAME_X = 96.0
NAME_Y = 34.0
NAME_W = 152.0
NAME_H = 26.0
NAME_FONT_SIZE_PX = 20.0

STATUS_X = 96.0
STATUS_Y = 68.0
STATUS_W = 148.0
STATUS_H = 18.0
STATUS_FONT_SIZE_PX = 13.0

CHIP_BG_X = 224.0
CHIP_BG_Y = 20.0
CHIP_BG_W = 44.0
CHIP_BG_H = 22.0

CHIP_TEXT_X = CHIP_BG_X
CHIP_TEXT_Y = CHIP_BG_Y + 1.0
CHIP_TEXT_W = CHIP_BG_W
CHIP_TEXT_H = CHIP_BG_H
CHIP_FONT_SIZE_PX = 10.0

DISMISS_BG_X = 280.0
DISMISS_BG_Y = 18.0
DISMISS_BG_W = 24.0
DISMISS_BG_H = 24.0

DISMISS_TEXT_X = DISMISS_BG_X
DISMISS_TEXT_Y = DISMISS_BG_Y
DISMISS_TEXT_W = DISMISS_BG_W
DISMISS_TEXT_H = DISMISS_BG_H
DISMISS_FONT_SIZE_PX = 12.0
DISMISS_INTERACTION_ID = "dismiss-card"

DEFAULT_PSK_ENV = "TZE_HUD_PSK"
DEFAULT_TARGET = "tzehouse-windows.parrot-hen.ts.net:50051"
DEFAULT_TRANSCRIPT_PATH = "test_results/presence-card-latest.json"


@dataclass(frozen=True)
class AgentSpec:
    index: int
    name: str
    rgba: tuple[float, float, float, float]
    z_order: int


AGENTS: list[AgentSpec] = [
    AgentSpec(0, "agent-alpha", (66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0), 100),
    AgentSpec(1, "agent-beta", (52 / 255.0, 168 / 255.0, 83 / 255.0, 1.0), 101),
    AgentSpec(2, "agent-gamma", (251 / 255.0, 188 / 255.0, 4 / 255.0, 1.0), 102),
]


@dataclass
class AgentRuntime:
    spec: AgentSpec
    client: HudClient
    lease_id: bytes
    tile_id: bytes
    heartbeat_task: Optional[asyncio.Task] = None
    dismiss_task: Optional[asyncio.Task] = None
    dismissed_by_user: bool = False


def format_last_active(elapsed_seconds: int) -> str:
    if elapsed_seconds <= 0:
        return "now"
    if elapsed_seconds < 60:
        return f"{elapsed_seconds}s ago"
    minutes = elapsed_seconds // 60
    return f"{minutes}m ago"


def card_y_offset(agent_index: int, tab_height: float) -> float:
    return (
        tab_height
        - CARD_H * (agent_index + 1)
        - CARD_GAP * agent_index
        - BOTTOM_MARGIN
    )


def build_text_content(agent_name: str, elapsed_seconds: int) -> str:
    return f"Connected • last active {format_last_active(elapsed_seconds)}"


def build_time_chip_content(elapsed_seconds: int) -> str:
    label = format_last_active(elapsed_seconds)
    if label == "now":
        return "NOW"
    if label.endswith("s ago"):
        return f"{label[:-5]}S"
    if label.endswith("m ago"):
        return f"{label[:-5]}M"
    return label.upper()


def build_presence_card_mutations(
    tile_id: bytes,
    agent_name: str,
    avatar_rgba: tuple[float, float, float, float],
    elapsed_seconds: int,
    include_tile_setup: bool,
    root_uuid: Optional[uuid.UUID] = None,
) -> list[types_pb2.MutationProto]:
    root_uuid = root_uuid or uuid.uuid4()

    root_node = _make_node(
        {
            "id": root_uuid.bytes_le,
            "solid_color": {
                "r": BG_RGBA[0],
                "g": BG_RGBA[1],
                "b": BG_RGBA[2],
                "a": BG_RGBA[3],
            },
            "bounds": [0.0, 0.0, CARD_W, CARD_H],
        }
    )
    sheen_node = _make_node(
        {
            "solid_color": {
                "r": SHEEN_RGBA[0],
                "g": SHEEN_RGBA[1],
                "b": SHEEN_RGBA[2],
                "a": SHEEN_RGBA[3],
            },
            "bounds": [SHEEN_X, SHEEN_Y, SHEEN_W, SHEEN_H],
        }
    )
    accent_node = _make_node(
        {
            "solid_color": {
                "r": avatar_rgba[0],
                "g": avatar_rgba[1],
                "b": avatar_rgba[2],
                "a": 0.78,
            },
            "bounds": [ACCENT_X, ACCENT_Y, ACCENT_W, ACCENT_H],
        }
    )
    avatar_plate_node = _make_node(
        {
            "solid_color": {
                "r": avatar_rgba[0],
                "g": avatar_rgba[1],
                "b": avatar_rgba[2],
                "a": 0.22,
            },
            "bounds": [AVATAR_PLATE_X, AVATAR_PLATE_Y, AVATAR_PLATE_W, AVATAR_PLATE_H],
        }
    )
    avatar_node = _make_node(
        {
            "solid_color": {
                "r": avatar_rgba[0],
                "g": avatar_rgba[1],
                "b": avatar_rgba[2],
                "a": avatar_rgba[3],
            },
            "bounds": [AVATAR_X, AVATAR_Y, AVATAR_W, AVATAR_H],
        }
    )
    eyebrow_node = _make_node(
        {
            "text_markdown": {
                "content": "RESIDENT AGENT",
                "font_size_px": EYEBROW_FONT_SIZE_PX,
                "color": list(EYEBROW_RGBA),
            },
            "bounds": [EYEBROW_X, EYEBROW_Y, EYEBROW_W, EYEBROW_H],
        }
    )
    name_node = _make_node(
        {
            "text_markdown": {
                "content": f"**{agent_name}**",
                "font_size_px": NAME_FONT_SIZE_PX,
                "color": list(NAME_RGBA),
            },
            "bounds": [NAME_X, NAME_Y, NAME_W, NAME_H],
        }
    )
    status_node = _make_node(
        {
            "text_markdown": {
                "content": build_text_content(agent_name, elapsed_seconds),
                "font_size_px": STATUS_FONT_SIZE_PX,
                "color": list(STATUS_RGBA),
            },
            "bounds": [STATUS_X, STATUS_Y, STATUS_W, STATUS_H],
        }
    )
    chip_bg_node = _make_node(
        {
            "solid_color": {
                "r": CHIP_BG_RGBA[0],
                "g": CHIP_BG_RGBA[1],
                "b": CHIP_BG_RGBA[2],
                "a": CHIP_BG_RGBA[3],
            },
            "bounds": [CHIP_BG_X, CHIP_BG_Y, CHIP_BG_W, CHIP_BG_H],
        }
    )
    chip_text_node = _make_node(
        {
            "text_markdown": {
                "content": build_time_chip_content(elapsed_seconds),
                "font_size_px": CHIP_FONT_SIZE_PX,
                "color": list(CHIP_TEXT_RGBA),
            },
            "bounds": [CHIP_TEXT_X, CHIP_TEXT_Y, CHIP_TEXT_W, CHIP_TEXT_H],
        }
    )
    dismiss_bg_node = _make_node(
        {
            "solid_color": {
                "r": 0.94,
                "g": 0.97,
                "b": 1.0,
                "a": 0.14,
            },
            "bounds": [DISMISS_BG_X, DISMISS_BG_Y, DISMISS_BG_W, DISMISS_BG_H],
        }
    )
    dismiss_text_node = _make_node(
        {
            "text_markdown": {
                "content": "X",
                "font_size_px": DISMISS_FONT_SIZE_PX,
                "color": list(CHIP_TEXT_RGBA),
            },
            "bounds": [DISMISS_TEXT_X, DISMISS_TEXT_Y, DISMISS_TEXT_W, DISMISS_TEXT_H],
        }
    )
    dismiss_hit_region_node = _make_node(
        {
            "hit_region": {
                "interaction_id": DISMISS_INTERACTION_ID,
                "accepts_focus": True,
                "accepts_pointer": True,
            },
            "bounds": [DISMISS_BG_X, DISMISS_BG_Y, DISMISS_BG_W, DISMISS_BG_H],
        }
    )

    mutations: list[types_pb2.MutationProto] = []
    if include_tile_setup:
        mutations.extend(
            [
                types_pb2.MutationProto(
                    update_tile_opacity=types_pb2.UpdateTileOpacityMutation(
                        tile_id=tile_id,
                        opacity=1.0,
                    )
                ),
                types_pb2.MutationProto(
                    update_tile_input_mode=types_pb2.UpdateTileInputModeMutation(
                        tile_id=tile_id,
                        input_mode=types_pb2.TILE_INPUT_MODE_CAPTURE,
                    )
                ),
            ]
        )

    mutations.extend(
        [
            types_pb2.MutationProto(
                set_tile_root=types_pb2.SetTileRootMutation(
                    tile_id=tile_id,
                    node=root_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=sheen_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=accent_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=avatar_plate_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=avatar_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=eyebrow_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=name_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=status_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=chip_bg_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=chip_text_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=dismiss_bg_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=dismiss_text_node,
                )
            ),
            types_pb2.MutationProto(
                add_node=types_pb2.AddNodeMutation(
                    tile_id=tile_id,
                    parent_id=root_uuid.bytes,
                    node=dismiss_hit_region_node,
                )
            ),
        ]
    )
    return mutations


def build_step_plan(
    update_wait_s: int,
    heartbeat_timeout_s: int,
    orphan_grace_s: int,
    disconnect_agent_name: str,
) -> list[dict[str, Any]]:
    return [
        {
            "code": "create",
            "title": "Create stacked cards",
            "action": "launch 3 resident sessions and create tiles",
            "expected_visual": "3 stacked cards visible in the bottom-left corner",
        },
        {
            "code": "update_wait",
            "title": "Wait for periodic update",
            "action": f"wait {update_wait_s}s, then rebuild all 3 cards",
            "expected_visual": f"all cards show Last active: {update_wait_s}s ago",
        },
        {
            "code": "disconnect",
            "title": "Disconnect gamma",
            "action": f"disconnect {disconnect_agent_name}",
            "expected_visual": "alpha and beta remain connected",
        },
        {
            "code": "orphan_observe",
            "title": "Observe orphan badge",
            "action": f"observe disconnect/orphan state within ~1s after the session closes or within {heartbeat_timeout_s}s after heartbeat timeout",
            "expected_visual": f"disconnection badge appears on {disconnect_agent_name} only",
        },
        {
            "code": "cleanup_wait",
            "title": "Wait for grace expiry",
            "action": f"wait {orphan_grace_s}s for orphan grace expiry",
            "expected_visual": f"{disconnect_agent_name} is removed while alpha and beta stay at original positions",
        },
        {
            "code": "final_state",
            "title": "Final state",
            "action": "verify remaining cards",
            "expected_visual": "2 remaining cards continue updating with no reflow",
        },
    ]


def emit_step_event(
    transcript: list[dict[str, Any]],
    step_index: int,
    status: str,
    step: dict[str, Any],
    **extra: Any,
) -> None:
    event = {
        "ts_wall": int(time.time()),
        "step_index": step_index,
        "status": status,
        **step,
        **extra,
    }
    transcript.append(event)
    print(json.dumps(event, sort_keys=True), flush=True)


def write_transcript(path: str, payload: dict[str, Any]) -> None:
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(payload, indent=2), encoding="utf-8")


async def heartbeat_loop(client: HudClient, interval_ms: int) -> None:
    send_interval_s = max(1.0, interval_ms / 2000.0)
    while True:
        await asyncio.sleep(send_interval_s)
        await client.send_heartbeat()


async def start_agent(
    target: str,
    psk: str,
    spec: AgentSpec,
    tab_height: float,
) -> AgentRuntime:
    client = HudClient(
        target,
        psk=psk,
        agent_id=spec.name,
        initial_subscriptions=["SCENE_TOPOLOGY", "INPUT_EVENTS"],
    )
    await client.connect()
    lease_id = await client.request_lease(ttl_ms=120_000)
    tile_id = await client.create_tile(
        lease_id=lease_id,
        x=LEFT_MARGIN,
        y=card_y_offset(spec.index, tab_height),
        w=CARD_W,
        h=CARD_H,
        z_order=spec.z_order,
    )
    await client.apply_mutations(
        lease_id,
        build_presence_card_mutations(
            tile_id=tile_id,
            agent_name=spec.name,
            avatar_rgba=spec.rgba,
            elapsed_seconds=0,
            include_tile_setup=True,
        ),
    )
    heartbeat_interval_ms = client.heartbeat_interval_ms or 5_000
    task = asyncio.create_task(heartbeat_loop(client, heartbeat_interval_ms))
    runtime = AgentRuntime(
        spec=spec,
        client=client,
        lease_id=lease_id,
        tile_id=tile_id,
        heartbeat_task=task,
    )
    runtime.dismiss_task = asyncio.create_task(watch_for_dismiss(runtime))
    return runtime


async def rebuild_agent_card(agent: AgentRuntime, elapsed_seconds: int) -> None:
    await agent.client.apply_mutations(
        agent.lease_id,
        build_presence_card_mutations(
            tile_id=agent.tile_id,
            agent_name=agent.spec.name,
            avatar_rgba=agent.spec.rgba,
            elapsed_seconds=elapsed_seconds,
            include_tile_setup=False,
        ),
    )


async def watch_for_dismiss(agent: AgentRuntime) -> None:
    try:
        await agent.client.wait_for_click(DISMISS_INTERACTION_ID)
    except asyncio.CancelledError:
        raise
    except Exception:
        return
    agent.dismissed_by_user = True
    await stop_agent(agent, "presence-card dismissed by user")


async def stop_agent(agent: AgentRuntime, reason: str) -> None:
    if agent.heartbeat_task is not None:
        agent.heartbeat_task.cancel()
        try:
            await agent.heartbeat_task
        except asyncio.CancelledError:
            pass
        agent.heartbeat_task = None
    if (
        agent.dismiss_task is not None
        and agent.dismiss_task is not asyncio.current_task()
    ):
        agent.dismiss_task.cancel()
        try:
            await agent.dismiss_task
        except asyncio.CancelledError:
            pass
        agent.dismiss_task = None
    await agent.client.close(reason=reason, expect_resume=False)


async def cleanup_agents(agents: list[AgentRuntime]) -> None:
    for agent in agents:
        try:
            await stop_agent(agent, "presence-card cleanup")
        except Exception:
            pass


async def run_scenario(args: argparse.Namespace) -> int:
    psk = os.getenv(args.psk_env, "")
    if not psk:
        print(
            json.dumps(
                {"error": "missing_psk", "psk_env": args.psk_env},
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 2

    transcript: list[dict[str, Any]] = []
    plan = build_step_plan(
        update_wait_s=args.update_wait_s,
        heartbeat_timeout_s=args.heartbeat_timeout_s,
        orphan_grace_s=args.orphan_grace_s,
        disconnect_agent_name="agent-gamma",
    )
    agents: list[AgentRuntime] = []

    try:
        emit_step_event(
            transcript,
            0,
            "started",
            {
                "code": "scenario",
                "title": "Presence Card live scenario",
                "action": "connect target and start agents",
                "expected_visual": "operator follows JSON step transcript",
            },
            target=args.target,
            tab_height=args.tab_height,
        )

        create_step = plan[0]
        emit_step_event(transcript, 1, "started", create_step)
        for spec in AGENTS:
            agents.append(await start_agent(args.target, psk, spec, args.tab_height))
        emit_step_event(
            transcript,
            1,
            "completed",
            create_step,
            agents=[agent.spec.name for agent in agents],
        )

        update_step = plan[1]
        emit_step_event(transcript, 2, "started", update_step)
        await asyncio.sleep(args.update_wait_s)
        await asyncio.gather(*(rebuild_agent_card(agent, args.update_wait_s) for agent in agents))
        emit_step_event(transcript, 2, "completed", update_step)

        gamma = next(agent for agent in agents if agent.spec.name == "agent-gamma")
        disconnect_step = plan[2]
        emit_step_event(transcript, 3, "started", disconnect_step)
        await stop_agent(gamma, "presence-card disconnect")
        agents = [agent for agent in agents if agent.spec.name != "agent-gamma"]
        emit_step_event(transcript, 3, "completed", disconnect_step)

        orphan_step = plan[3]
        emit_step_event(transcript, 4, "started", orphan_step)
        await asyncio.sleep(args.observe_badge_s)
        emit_step_event(transcript, 4, "completed", orphan_step)

        cleanup_step = plan[4]
        emit_step_event(transcript, 5, "started", cleanup_step)
        survivor_update_s = min(args.update_wait_s, args.orphan_grace_s)
        await asyncio.sleep(survivor_update_s)
        total_elapsed = args.update_wait_s + survivor_update_s
        await asyncio.gather(*(rebuild_agent_card(agent, total_elapsed) for agent in agents))
        remaining_wait_s = max(0, args.orphan_grace_s - survivor_update_s)
        if remaining_wait_s:
            await asyncio.sleep(remaining_wait_s)
        emit_step_event(transcript, 5, "completed", cleanup_step)

        final_step = plan[5]
        emit_step_event(transcript, 6, "completed", final_step, remaining_agents=[agent.spec.name for agent in agents])
        emit_step_event(
            transcript,
            7,
            "completed",
            {
                "code": "scenario_complete",
                "title": "Presence Card scenario complete",
                "action": "review transcript and perform human visual checks",
                "expected_visual": "3 cards -> updates -> gamma badge -> gamma removed -> 2 cards",
            },
        )
    finally:
        await cleanup_agents(agents)
        if args.transcript_out:
            write_transcript(
                args.transcript_out,
                {
                    "target": args.target,
                    "tab_height": args.tab_height,
                    "steps": transcript,
                },
            )

    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the Presence Card live resident gRPC scenario.",
    )
    parser.add_argument("--target", default=DEFAULT_TARGET, help="gRPC host:port for the HUD session stream")
    parser.add_argument("--psk-env", default=DEFAULT_PSK_ENV, help="Environment variable containing the HUD PSK")
    parser.add_argument("--tab-height", type=float, default=1080.0, help="Logical tab height used to compute bottom-left card stacking")
    parser.add_argument("--update-wait-s", type=int, default=30, help="Seconds to wait before the first periodic update")
    parser.add_argument("--heartbeat-timeout-s", type=int, default=15, help="Human reference for heartbeat-timeout orphan detection")
    parser.add_argument("--orphan-grace-s", type=int, default=30, help="Seconds to wait for orphan grace expiry after disconnect")
    parser.add_argument("--observe-badge-s", type=float, default=1.0, help="Seconds to pause for the disconnection badge visual check")
    parser.add_argument(
        "--transcript-out",
        default=DEFAULT_TRANSCRIPT_PATH,
        help="Path to write a JSON transcript artifact",
    )
    return parser.parse_args()


def main() -> int:
    try:
        return asyncio.run(run_scenario(parse_args()))
    except KeyboardInterrupt:
        print(json.dumps({"error": "interrupted"}), file=sys.stderr)
        return 130
    except Exception as exc:
        print(json.dumps({"error": "exception", "detail": str(exc)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
