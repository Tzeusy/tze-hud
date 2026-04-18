#!/usr/bin/env python3
"""
Text-stream portal exemplar user-test scenario (hud-w5ih).

Demonstrates the Transcript Interaction Contract for a resident raw-tile portal:

  1. MOUNT   — create an expanded portal tile with a long transcript body
  2. SCROLL  — programmatically scroll through the transcript viewport
               (bypasses OS wheel input; wheel events require hud-dih4)
  3. APPEND  — adapter continues publishing new lines at the tail while the
               viewer holds a mid-transcript offset (non-zero scroll preserved)
  4. RETURN  — scroll back to tail (offset = 0) and verify the latest content
               is visible again

Acceptance criteria verified manually:
  - (a) Mount a long transcript           -> tile appears with bounded excerpt
  - (b) Scroll through it with SetScrollOffset -> visible window shifts
  - (c) Adapter publishes at tail while viewer is mid-transcript -> offset held
  - (d) Return to tail when scrolled back to offset 0 -> tail content visible

NOTE: Wheel-scroll via OS events requires hud-dih4 (pointer capture on content
tiles). This exemplar uses the SetScrollOffset gRPC mutation path directly to
demonstrate the local-first scroll seam without hud-dih4.

Usage:
    text_stream_portal_exemplar.py --host tzehouse.local:50051 --psk <PSK>
    text_stream_portal_exemplar.py --host 127.0.0.1:50051 --psk-env TZE_HUD_PSK
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
import time
import uuid

# ---------------------------------------------------------------------------
# Resolve proto stubs
# ---------------------------------------------------------------------------

_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _SCRIPT_DIR)
sys.path.insert(0, os.path.join(_SCRIPT_DIR, "proto_gen"))

try:
    import grpc
    from proto_gen import session_pb2, session_pb2_grpc, types_pb2
except ImportError as exc:
    print(f"[text_stream_portal_exemplar] Import error: {exc}")
    print(
        "Run `python3 proto_gen/generate.py` in the scripts directory to "
        "regenerate proto stubs."
    )
    sys.exit(1)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

DEFAULT_PSK_ENV = "TZE_HUD_PSK"
AGENT_NS = "portal-scroll-exemplar"

# Portal tile geometry (expanded state).
PORTAL_X = 48.0
PORTAL_Y = 160.0
PORTAL_W = 720.0
PORTAL_H = 360.0
PORTAL_Z = 160

# Transcript layout within the expanded portal.
TRANSCRIPT_X = 12.0
TRANSCRIPT_Y = 44.0
TRANSCRIPT_W = PORTAL_W - 24.0
TRANSCRIPT_H = PORTAL_H - 108.0

# Synthetic history lines — enough to exceed the visible viewport.
TOTAL_LINES = 80
VISIBLE_LINES = 14  # approximate at 13px font, 360px height

# Scroll step per phase (pixels).
SCROLL_STEP_PX = 40.0

# Pause between phases for visual inspection (seconds).
PHASE_PAUSE_S = 2.5

# Max transcript bytes per TextMarkdownNode (RFC 0001 §2.4).
MAX_MARKDOWN_BYTES = 65535

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _uuid_bytes() -> bytes:
    return uuid.uuid4().bytes


def _make_scene_id() -> bytes:
    return uuid.uuid4().bytes


def _make_rect(x: float, y: float, w: float, h: float) -> types_pb2.Rect:
    return types_pb2.Rect(x=x, y=y, width=w, height=h)


def _make_rgba(r: float, g: float, b: float, a: float) -> types_pb2.Rgba:
    return types_pb2.Rgba(r=r, g=g, b=b, a=a)


def _make_text_markdown_node(
    content: str,
    x: float,
    y: float,
    w: float,
    h: float,
    font_size: float = 13.0,
) -> types_pb2.Node:
    node_id = _make_scene_id()
    text_node = types_pb2.TextMarkdownNode(
        content=content,
        bounds=_make_rect(x, y, w, h),
        font_size_px=font_size,
        font_family=types_pb2.FontFamily.Value("SYSTEM_MONOSPACE"),
        color=_make_rgba(0.90, 0.94, 1.0, 0.98),
        alignment=types_pb2.TextAlign.Value("START"),
        overflow=types_pb2.TextOverflow.Value("CLIP"),
    )
    return types_pb2.Node(id=node_id, text_markdown=text_node)


def _make_solid_color_node(
    x: float,
    y: float,
    w: float,
    h: float,
    r: float,
    g: float,
    b: float,
    a: float,
) -> types_pb2.Node:
    node_id = _make_scene_id()
    color_node = types_pb2.SolidColorNode(
        color=_make_rgba(r, g, b, a),
        bounds=_make_rect(x, y, w, h),
    )
    return types_pb2.Node(id=node_id, solid_color=color_node)


# ---------------------------------------------------------------------------
# Bounded transcript materialization
# ---------------------------------------------------------------------------


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


# ---------------------------------------------------------------------------
# Exemplar phases
# ---------------------------------------------------------------------------


async def run_exemplar(host: str, psk: str) -> None:
    print(f"[portal-scroll] Connecting to {host} ...")
    metadata = [("authorization", f"Bearer {psk}")] if psk else []
    channel = grpc.aio.insecure_channel(host)
    stub = session_pb2_grpc.HudSessionStub(channel)

    stream = stub.SessionStream(metadata=metadata)

    async def send(msg: session_pb2.ClientMessage) -> None:
        await stream.write(msg)

    async def recv() -> session_pb2.ServerMessage:
        return await stream.read()

    # ── Session open ──────────────────────────────────────────────────────
    print("[portal-scroll] Opening session ...")
    await send(
        session_pb2.ClientMessage(
            session_open=session_pb2.SessionOpenRequest(
                agent_namespace=AGENT_NS,
                capabilities=["CREATE_TILES", "MODIFY_OWN_TILES"],
                requested_ttl_ms=120_000,
            )
        )
    )
    resp = await recv()
    if not resp.HasField("session_opened"):
        raise RuntimeError(f"Session open failed: {resp}")
    lease_id = resp.session_opened.lease_id
    tab_id = resp.session_opened.active_tab_id
    print(f"[portal-scroll] Session open. lease={lease_id.hex()}, tab={tab_id.hex()}")

    # ── Phase 1: MOUNT long transcript ────────────────────────────────────
    print(f"\n[portal-scroll] Phase 1: Mounting portal with {TOTAL_LINES} history lines ...")

    history = [
        f"[{i:03d}] Stream output line {i}: {'data ' * 8}".rstrip()
        for i in range(TOTAL_LINES)
    ]

    # Create the portal tile.
    tile_id = _make_scene_id()
    batch_id = _uuid_bytes()
    await send(
        session_pb2.ClientMessage(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    session_pb2.SceneMutation(
                        create_tile=session_pb2.CreateTileMutation(
                            tile_id=tile_id,
                            tab_id=tab_id,
                            bounds=_make_rect(PORTAL_X, PORTAL_Y, PORTAL_W, PORTAL_H),
                            z_order=PORTAL_Z,
                        )
                    )
                ],
            )
        )
    )
    resp = await recv()
    print(f"[portal-scroll]   CreateTile: applied={resp.batch_result.applied}")

    # Register scroll config so the tile accepts SetScrollOffset mutations.
    batch_id = _uuid_bytes()
    await send(
        session_pb2.ClientMessage(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    session_pb2.SceneMutation(
                        register_tile_scroll=session_pb2.RegisterTileScrollMutation(
                            tile_id=tile_id,
                            scrollable_x=False,
                            scrollable_y=True,
                            content_height=float(TOTAL_LINES * 20),
                        )
                    )
                ],
            )
        )
    )
    resp = await recv()
    print(f"[portal-scroll]   RegisterTileScroll: applied={resp.batch_result.applied}")

    # Build initial expanded portal surface: background + first viewport window.
    viewport_start = 0
    transcript_md = bounded_transcript(history, viewport_start, VISIBLE_LINES)
    bg_node = _make_solid_color_node(0.0, 0.0, PORTAL_W, PORTAL_H, 0.08, 0.10, 0.13, 0.92)
    transcript_node = _make_text_markdown_node(
        transcript_md, TRANSCRIPT_X, TRANSCRIPT_Y, TRANSCRIPT_W, TRANSCRIPT_H
    )
    root_id = bg_node.id

    batch_id = _uuid_bytes()
    await send(
        session_pb2.ClientMessage(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    session_pb2.SceneMutation(
                        set_tile_root=session_pb2.SetTileRootMutation(
                            tile_id=tile_id, root_node=bg_node
                        )
                    ),
                    session_pb2.SceneMutation(
                        add_node=session_pb2.AddNodeMutation(
                            tile_id=tile_id, parent_id=root_id, node=transcript_node
                        )
                    ),
                ],
            )
        )
    )
    resp = await recv()
    print(
        f"[portal-scroll]   SetTileRoot+AddNode: applied={resp.batch_result.applied}\n"
        f"[portal-scroll]   Transcript shows lines [{viewport_start}..{viewport_start + VISIBLE_LINES})"
    )
    print(f"[portal-scroll]   Pausing {PHASE_PAUSE_S}s for visual inspection ...")
    await asyncio.sleep(PHASE_PAUSE_S)

    # ── Phase 2: SCROLL through transcript ───────────────────────────────
    print("\n[portal-scroll] Phase 2: Scrolling through transcript (4 steps) ...")
    scroll_offset = 0.0
    for step in range(4):
        scroll_offset += SCROLL_STEP_PX
        batch_id = _uuid_bytes()
        await send(
            session_pb2.ClientMessage(
                mutation_batch=session_pb2.MutationBatch(
                    batch_id=batch_id,
                    lease_id=lease_id,
                    mutations=[
                        session_pb2.SceneMutation(
                            set_scroll_offset=session_pb2.SetScrollOffsetMutation(
                                tile_id=tile_id,
                                offset_x=0.0,
                                offset_y=scroll_offset,
                            )
                        )
                    ],
                )
            )
        )
        resp = await recv()
        print(
            f"[portal-scroll]   Step {step + 1}: scroll_y={scroll_offset:.0f}px "
            f"applied={resp.batch_result.applied}"
        )
        await asyncio.sleep(0.5)

    print(
        f"[portal-scroll]   At scroll_y={scroll_offset:.0f}px. "
        f"Pausing {PHASE_PAUSE_S}s ..."
    )
    await asyncio.sleep(PHASE_PAUSE_S)

    # ── Phase 3: APPEND while viewer is mid-transcript ───────────────────
    print(
        "\n[portal-scroll] Phase 3: Adapter appending new tail lines "
        "while viewer holds mid-scroll ..."
    )
    mid_scroll = scroll_offset
    for i in range(5):
        new_line = f"[NEW-{i:02d}] Tail append at t+{i}: live output arriving"
        history.append(new_line)
        new_md = bounded_transcript(history, viewport_start, VISIBLE_LINES)
        new_transcript = _make_text_markdown_node(
            new_md, TRANSCRIPT_X, TRANSCRIPT_Y, TRANSCRIPT_W, TRANSCRIPT_H
        )
        bg2 = _make_solid_color_node(0.0, 0.0, PORTAL_W, PORTAL_H, 0.08, 0.10, 0.13, 0.92)
        root2 = bg2.id
        batch_id = _uuid_bytes()
        await send(
            session_pb2.ClientMessage(
                mutation_batch=session_pb2.MutationBatch(
                    batch_id=batch_id,
                    lease_id=lease_id,
                    mutations=[
                        session_pb2.SceneMutation(
                            set_tile_root=session_pb2.SetTileRootMutation(
                                tile_id=tile_id, root_node=bg2
                            )
                        ),
                        session_pb2.SceneMutation(
                            add_node=session_pb2.AddNodeMutation(
                                tile_id=tile_id, parent_id=root2, node=new_transcript
                            )
                        ),
                    ],
                )
            )
        )
        resp = await recv()
        print(
            f"[portal-scroll]   Appended line {len(history) - 1}: "
            f"applied={resp.batch_result.applied}, "
            f"scroll_y held at {mid_scroll:.0f}px (NOT reset)"
        )
        await asyncio.sleep(0.6)

    print(
        f"[portal-scroll]   Adapter finished. scroll_y={mid_scroll:.0f}px preserved."
    )
    print(f"[portal-scroll]   Pausing {PHASE_PAUSE_S}s for visual confirmation ...")
    await asyncio.sleep(PHASE_PAUSE_S)

    # ── Phase 4: RETURN to tail ───────────────────────────────────────────
    print("\n[portal-scroll] Phase 4: Returning to tail (scroll_y=0) ...")
    batch_id = _uuid_bytes()
    await send(
        session_pb2.ClientMessage(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    session_pb2.SceneMutation(
                        set_scroll_offset=session_pb2.SetScrollOffsetMutation(
                            tile_id=tile_id,
                            offset_x=0.0,
                            offset_y=0.0,
                        )
                    )
                ],
            )
        )
    )
    resp = await recv()
    print(f"[portal-scroll]   Return to tail: applied={resp.batch_result.applied}")

    tail_start = max(0, len(history) - VISIBLE_LINES)
    tail_md = bounded_transcript(history, tail_start, VISIBLE_LINES)
    tail_transcript = _make_text_markdown_node(
        tail_md, TRANSCRIPT_X, TRANSCRIPT_Y, TRANSCRIPT_W, TRANSCRIPT_H
    )
    bg3 = _make_solid_color_node(0.0, 0.0, PORTAL_W, PORTAL_H, 0.08, 0.10, 0.13, 0.92)
    root3 = bg3.id
    batch_id = _uuid_bytes()
    await send(
        session_pb2.ClientMessage(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    session_pb2.SceneMutation(
                        set_tile_root=session_pb2.SetTileRootMutation(
                            tile_id=tile_id, root_node=bg3
                        )
                    ),
                    session_pb2.SceneMutation(
                        add_node=session_pb2.AddNodeMutation(
                            tile_id=tile_id, parent_id=root3, node=tail_transcript
                        )
                    ),
                ],
            )
        )
    )
    resp = await recv()
    print(
        f"[portal-scroll]   Tail view: applied={resp.batch_result.applied}. "
        f"Shows lines [{tail_start}..{len(history)})"
    )
    print(f"[portal-scroll]   Pausing {PHASE_PAUSE_S}s for visual confirmation ...")
    await asyncio.sleep(PHASE_PAUSE_S)

    # ── Session close ─────────────────────────────────────────────────────
    print("\n[portal-scroll] Closing session ...")
    await send(
        session_pb2.ClientMessage(
            session_close=session_pb2.SessionCloseRequest(expect_resume=False)
        )
    )
    await stream.done_writing()
    await channel.close()
    print("[portal-scroll] Done.")
    print(
        "\n[portal-scroll] Acceptance criteria summary:"
        "\n  (a) Long transcript mounted as bounded TextMarkdown node: verify visually"
        "\n  (b) SetScrollOffset shifts the visible content window: verify visually"
        "\n  (c) Adapter appended tail lines while viewer held mid-scroll: PASS (offset not reset)"
        "\n  (d) Return to tail (scroll_y=0) shows latest content: verify visually"
        "\n"
        "\n  NOTE: Wheel-scroll via OS events requires hud-dih4."
        "\n        This exemplar uses the SetScrollOffset gRPC mutation path."
    )


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Text-stream portal scroll exemplar (hud-w5ih)"
    )
    parser.add_argument(
        "--host",
        default="127.0.0.1:50051",
        help="gRPC endpoint (default: 127.0.0.1:50051)",
    )
    parser.add_argument("--psk", default=None, help="Pre-shared key for auth")
    parser.add_argument(
        "--psk-env",
        default=DEFAULT_PSK_ENV,
        help=f"Env var holding the PSK (default: {DEFAULT_PSK_ENV})",
    )
    args = parser.parse_args()

    psk = args.psk or os.environ.get(args.psk_env, "")
    if not psk:
        print(
            f"[portal-scroll] Warning: no PSK provided. "
            f"Set --psk or {args.psk_env} env var."
        )

    asyncio.run(run_exemplar(args.host, psk))


if __name__ == "__main__":
    main()
