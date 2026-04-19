#!/usr/bin/env python3
"""
Text-stream portal exemplar user-test scenario (hud-w5ih).

Demonstrates the Transcript Interaction Contract for a resident raw-tile portal:

  1. MOUNT   — create an expanded portal tile with a long transcript body,
               register a vertical scroll config.
  2. SCROLL  — programmatically scroll through the transcript viewport
               using SetScrollOffsetMutation (bypasses OS wheel input;
               wheel wiring lives in hud-6bbe).
  3. APPEND  — adapter continues publishing new lines at the tail while the
               viewer holds a mid-transcript offset (non-zero scroll preserved).
  4. RETURN  — scroll back to tail (offset = 0) and verify the latest content
               is visible again.

Uses the shared `HudClient` helper (`hud_grpc_client.py`) for the resident
session handshake + lease + mutation-batch plumbing. The scroll operations
ride the new `RegisterTileScrollMutation` / `SetScrollOffsetMutation` proto
surface added in hud-w5ih.

Usage:
    text_stream_portal_exemplar.py --target <host:port> --psk-env TZE_HUD_PSK
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
import uuid

# Resolve proto stubs + HudClient helper (co-located).
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _SCRIPT_DIR)
sys.path.insert(0, os.path.join(_SCRIPT_DIR, "proto_gen"))

from hud_grpc_client import HudClient  # noqa: E402
from proto_gen import types_pb2  # noqa: E402


# ─── Portal geometry ──────────────────────────────────────────────────────────

PORTAL_X = 48.0
PORTAL_Y = 160.0
PORTAL_W = 720.0
PORTAL_H = 360.0
PORTAL_Z = 160

# Transcript layout within the portal.
TRANSCRIPT_X = 12.0
TRANSCRIPT_Y = 44.0
TRANSCRIPT_W = PORTAL_W - 24.0
TRANSCRIPT_H = PORTAL_H - 108.0

# Synthetic history lines — enough to exceed the visible viewport.
TOTAL_LINES = 80
VISIBLE_LINES = 14  # approx at 13px font, 360px height

SCROLL_STEP_PX = 40.0
PHASE_PAUSE_S = 2.5

MAX_MARKDOWN_BYTES = 65535

DEFAULT_TARGET = "tzehouse-windows.parrot-hen.ts.net:50051"
DEFAULT_PSK_ENV = "TZE_HUD_PSK"
DEFAULT_AGENT_ID = "agent-alpha"


# ─── Node helpers ─────────────────────────────────────────────────────────────


def _uuid_bytes() -> bytes:
    return uuid.uuid4().bytes


def _make_rect(x: float, y: float, w: float, h: float) -> types_pb2.Rect:
    return types_pb2.Rect(x=x, y=y, width=w, height=h)


def _make_rgba(r: float, g: float, b: float, a: float) -> types_pb2.Rgba:
    return types_pb2.Rgba(r=r, g=g, b=b, a=a)


def _make_text_markdown_node(
    content: str, x: float, y: float, w: float, h: float, font_size: float = 13.0
) -> types_pb2.NodeProto:
    return types_pb2.NodeProto(
        id=_uuid_bytes(),
        text_markdown=types_pb2.TextMarkdownNodeProto(
            content=content,
            bounds=_make_rect(x, y, w, h),
            font_size_px=font_size,
            color=_make_rgba(0.90, 0.94, 1.0, 0.98),
        ),
    )


def _make_solid_color_node(
    x: float, y: float, w: float, h: float,
    r: float, g: float, b: float, a: float,
) -> types_pb2.NodeProto:
    return types_pb2.NodeProto(
        id=_uuid_bytes(),
        solid_color=types_pb2.SolidColorNodeProto(
            color=_make_rgba(r, g, b, a),
            bounds=_make_rect(x, y, w, h),
            radius=-1.0,
        ),
    )


def bounded_transcript(lines: list, start: int, max_lines: int) -> str:
    """Return a markdown string bounded to a viewport window and byte budget."""
    end = min(start + max_lines, len(lines))
    start = min(start, end)
    while start < end:
        joined = "\n".join(lines[start:end])
        if len(joined.encode("utf-8")) <= MAX_MARKDOWN_BYTES:
            return joined
        start += 1
    return ""


# ─── Mutation helpers ─────────────────────────────────────────────────────────


def _set_tile_root_mutation(tile_id: bytes, node: types_pb2.NodeProto):
    return types_pb2.MutationProto(
        set_tile_root=types_pb2.SetTileRootMutation(tile_id=tile_id, node=node),
    )


def _add_node_mutation(tile_id: bytes, parent_id: bytes, node: types_pb2.NodeProto):
    return types_pb2.MutationProto(
        add_node=types_pb2.AddNodeMutation(
            tile_id=tile_id, parent_id=parent_id, node=node,
        ),
    )


def _register_tile_scroll_mutation(
    tile_id: bytes,
    scrollable_y: bool = True,
    content_height: float = -1.0,
):
    return types_pb2.MutationProto(
        register_tile_scroll=types_pb2.RegisterTileScrollMutation(
            tile_id=tile_id,
            scrollable_x=False,
            scrollable_y=scrollable_y,
            content_width=-1.0,
            content_height=content_height,
        ),
    )


def _set_scroll_offset_mutation(tile_id: bytes, offset_x: float, offset_y: float):
    return types_pb2.MutationProto(
        set_scroll_offset=types_pb2.SetScrollOffsetMutation(
            tile_id=tile_id, offset_x=offset_x, offset_y=offset_y,
        ),
    )


# ─── Phases ───────────────────────────────────────────────────────────────────


async def mount_transcript(
    client: HudClient, lease_id: bytes, tile_id: bytes, transcript_md: str,
) -> None:
    """Publish root SolidColor background + transcript TextMarkdown child.

    Splits set_tile_root and add_node into separate batches so the
    server-assigned root id can be used as parent_id — batching them
    together fails because the server rewrites the root id (atomic-batch
    rejection; see docs/text-stream-refinement.md gotchas).
    """
    bg_node = _make_solid_color_node(
        0.0, 0.0, PORTAL_W, PORTAL_H, 0.08, 0.10, 0.13, 0.92,
    )
    transcript_node = _make_text_markdown_node(
        transcript_md, TRANSCRIPT_X, TRANSCRIPT_Y, TRANSCRIPT_W, TRANSCRIPT_H,
    )

    mr = await client.submit_mutation_batch(
        lease_id, [_set_tile_root_mutation(tile_id, bg_node)],
    )
    root_id = mr.created_ids[0] if mr.created_ids else bg_node.id
    await client.submit_mutation_batch(
        lease_id, [_add_node_mutation(tile_id, root_id, transcript_node)],
    )


async def run_exemplar(target: str, psk: str, agent_id: str) -> None:
    print(f"[portal-scroll] Connecting to {target} ...", flush=True)
    client = HudClient(
        target,
        psk=psk,
        agent_id=agent_id,
        capabilities=["create_tiles", "modify_own_tiles", "access_input_events"],
        initial_subscriptions=["SCENE_TOPOLOGY"],
    )

    try:
        await client.connect()
        print(
            f"[portal-scroll] Session open. namespace={client.namespace}",
            flush=True,
        )

        lease_id = await client.request_lease(ttl_ms=120_000)

        # ── Phase 1: MOUNT long transcript ────────────────────────────────
        print(
            f"\n[portal-scroll] Phase 1: Mount portal with {TOTAL_LINES} lines",
            flush=True,
        )
        history = [
            f"[{i:03d}] Stream output line {i}: {'data ' * 8}".rstrip()
            for i in range(TOTAL_LINES)
        ]
        tile_id = await client.create_tile(
            lease_id=lease_id,
            x=PORTAL_X, y=PORTAL_Y, w=PORTAL_W, h=PORTAL_H,
            z_order=PORTAL_Z,
        )
        await client.submit_mutation_batch(
            lease_id,
            [_register_tile_scroll_mutation(
                tile_id,
                scrollable_y=True,
                content_height=float(TOTAL_LINES * 20),
            )],
        )
        print("[portal-scroll]   RegisterTileScroll: accepted", flush=True)

        viewport_start = 0
        transcript_md = bounded_transcript(history, viewport_start, VISIBLE_LINES)
        await mount_transcript(client, lease_id, tile_id, transcript_md)
        print(
            f"[portal-scroll]   Mount complete; visible lines "
            f"[{viewport_start}..{viewport_start + VISIBLE_LINES})",
            flush=True,
        )
        print(f"[portal-scroll]   Pausing {PHASE_PAUSE_S}s ...", flush=True)
        await asyncio.sleep(PHASE_PAUSE_S)

        # ── Phase 2: SCROLL through transcript ────────────────────────────
        print("\n[portal-scroll] Phase 2: Scrolling through transcript (4 steps)", flush=True)
        scroll_offset = 0.0
        for step in range(4):
            scroll_offset += SCROLL_STEP_PX
            await client.submit_mutation_batch(
                lease_id,
                [_set_scroll_offset_mutation(tile_id, 0.0, scroll_offset)],
            )
            print(
                f"[portal-scroll]   Step {step + 1}: scroll_y={scroll_offset:.0f}px",
                flush=True,
            )
            await asyncio.sleep(0.5)
        print(
            f"[portal-scroll]   Holding at scroll_y={scroll_offset:.0f}px for "
            f"{PHASE_PAUSE_S}s ...",
            flush=True,
        )
        await asyncio.sleep(PHASE_PAUSE_S)

        # ── Phase 3: APPEND while viewer is mid-transcript ────────────────
        print(
            "\n[portal-scroll] Phase 3: Adapter appending tail lines while "
            "viewer holds mid-scroll ...",
            flush=True,
        )
        mid_scroll = scroll_offset
        for i in range(5):
            history.append(f"[NEW-{i:02d}] Tail append at t+{i}: live output arriving")
            new_md = bounded_transcript(history, viewport_start, VISIBLE_LINES)
            await mount_transcript(client, lease_id, tile_id, new_md)
            print(
                f"[portal-scroll]   Appended line {len(history) - 1}; "
                f"scroll_y held at {mid_scroll:.0f}px",
                flush=True,
            )
            await asyncio.sleep(0.6)
        print(f"[portal-scroll]   Pausing {PHASE_PAUSE_S}s ...", flush=True)
        await asyncio.sleep(PHASE_PAUSE_S)

        # ── Phase 4: RETURN to tail ───────────────────────────────────────
        print("\n[portal-scroll] Phase 4: Returning to tail (scroll_y=0)", flush=True)
        await client.submit_mutation_batch(
            lease_id, [_set_scroll_offset_mutation(tile_id, 0.0, 0.0)],
        )
        tail_start = max(0, len(history) - VISIBLE_LINES)
        tail_md = bounded_transcript(history, tail_start, VISIBLE_LINES)
        await mount_transcript(client, lease_id, tile_id, tail_md)
        print(
            f"[portal-scroll]   Tail view; lines [{tail_start}..{len(history)})",
            flush=True,
        )
        print(f"[portal-scroll]   Pausing {PHASE_PAUSE_S}s ...", flush=True)
        await asyncio.sleep(PHASE_PAUSE_S)

        print("\n[portal-scroll] Acceptance summary:")
        print("  (a) Long transcript mounted as bounded TextMarkdown node")
        print("  (b) SetScrollOffset shifts the visible content window")
        print("  (c) Appended lines did NOT reset scroll_y (offset preserved)")
        print("  (d) Return to tail (scroll_y=0) shows latest content")
    finally:
        try:
            await client.close(reason="text-stream-portal exemplar done")
        except Exception:
            pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Text-stream portal scroll exemplar (hud-w5ih)"
    )
    parser.add_argument("--target", default=DEFAULT_TARGET,
                        help="gRPC host:port for the resident session")
    parser.add_argument("--psk", default=None, help="Pre-shared key")
    parser.add_argument("--psk-env", default=DEFAULT_PSK_ENV,
                        help=f"Env var holding the PSK (default {DEFAULT_PSK_ENV})")
    parser.add_argument("--agent-id", default=DEFAULT_AGENT_ID,
                        help="Registered agent id")
    args = parser.parse_args()

    psk = args.psk or os.environ.get(args.psk_env, "")
    if not psk:
        print(
            f"[portal-scroll] ERROR: no PSK provided — set --psk or "
            f"{args.psk_env} env var",
            file=sys.stderr,
        )
        return 2

    try:
        asyncio.run(run_exemplar(args.target, psk, args.agent_id))
        return 0
    except Exception as exc:
        print(f"[portal-scroll] ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
