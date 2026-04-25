#!/usr/bin/env python3
"""
Text Stream Portal exemplar user-test scenario.

Renders a two-pane bounded portal surface (INPUT left · OUTPUT right) on the
live HUD via the resident `HudSession` gRPC stream, using only existing scene
primitives (SolidColor, TextMarkdown, HitRegion). No terminal-emulator node,
no new node type, no chrome-hosted affordances.

Layout:
  - Equal 50/50 split between INPUT and OUTPUT panes
  - Fat 6px drag divider (with centred grip bar + hit region) between them.
  - The static frame is one tile; the input composer and transcript body are
    separate transparent scroll-capture tiles so wheel input cannot move the
    whole portal.
  - Panes render at 95% black opacity (operator preference).

Phases:
  - baseline  : render full chrome + transcript viewport, then hold
  - scroll    : register output scroll, step through transcript, append tail,
                then return to latest output
  - streaming : clear transcript, reveal content in ordered chunks
  - rapid     : rapid-replace stress (coalescing-coherence smoke)

Emits per-step JSON transcript to stdout and writes an artifact (default:
`test_results/text-stream-portal-latest.json`).
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

# Resolve HudClient + proto stubs (co-located).
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _SCRIPT_DIR)
sys.path.insert(0, os.path.join(_SCRIPT_DIR, "proto_gen"))

from hud_grpc_client import HudClient, _make_node  # noqa: E402
from proto_gen import types_pb2  # noqa: E402


# ─── Portal chrome tokens (iterate here) ──────────────────────────────────────

PORTAL_W = 860.0
PORTAL_H = 680.0
PORTAL_RADIUS = 14.0
PORTAL_X_FROM_RIGHT = 28.0
PORTAL_Y = 120.0
PORTAL_Z = 220

BG_RGBA = (0.0, 0.0, 0.0, 0.30)              # light portal frame only
HEADER_BG_RGBA = (0.0, 0.0, 0.0, 0.50)       # header slightly denser than frame
DIVIDER_RGBA = (1.0, 1.0, 1.0, 0.10)
FOOTER_BG_RGBA = (0.0, 0.0, 0.0, 0.50)
# Input + output panes: black at 95% opacity.
INPUT_PANE_BG_RGBA = (0.0, 0.0, 0.0, 0.95)
OUTPUT_PANE_BG_RGBA = (0.0, 0.0, 0.0, 0.95)
COMPOSER_BG_RGBA = (1.0, 1.0, 1.0, 0.05)
COMPOSER_BORDER_RGBA = (1.0, 1.0, 1.0, 0.12)
SUBMIT_HINT_RGBA = (0.54, 0.60, 0.68, 0.90)
EYEBROW_RGBA = (0.70, 0.76, 0.84, 0.90)
CARET_RGBA = (0.48, 0.86, 0.56, 0.95)

TITLE_RGBA = (0.98, 0.99, 1.0, 1.0)
SUBTITLE_RGBA = (0.78, 0.82, 0.88, 0.88)
BODY_RGBA = (0.92, 0.94, 0.97, 0.98)
META_RGBA = (0.66, 0.70, 0.76, 0.82)
ACTIVITY_DOT_RGBA = (0.48, 0.86, 0.56, 0.92)
INPUT_TEXT_RGBA = (0.88, 0.92, 0.98, 0.96)
INPUT_PLACEHOLDER_RGBA = (0.50, 0.55, 0.64, 0.78)

TITLE_FONT = 17.0
SUBTITLE_FONT = 11.0
BODY_FONT = 13.0
META_FONT = 11.0
EYEBROW_FONT = 10.0
INPUT_FONT = 13.0
SUBMIT_HINT_FONT = 10.0

PADDING_X = 18.0
HEADER_H = 52.0
FOOTER_H = 30.0
DIVIDER_H = 1.0
ACTIVITY_DOT_SIZE = 8.0

# Equal 50/50 split with a fat divider between panes. Runtime pointer capture
# exists, but resize-on-drag still needs portal-side geometry mutation logic.
PANE_DIVIDER_W = 6.0
INPUT_PANE_W = (PORTAL_W - PANE_DIVIDER_W) / 2.0
PANE_DIVIDER_RGBA = (1.0, 1.0, 1.0, 0.14)
PANE_DIVIDER_GRIP_RGBA = (1.0, 1.0, 1.0, 0.40)
PANE_DIVIDER_GRIP_H = 44.0
PANE_DIVIDER_GRIP_W = 2.0

SCROLL_INTERACTION_ID = "portal-scroll"
SUBMIT_INTERACTION_ID = "portal-submit"
COMPOSER_INTERACTION_ID = "portal-composer-focus"
PANE_RESIZE_INTERACTION_ID = "portal-pane-resize"


@dataclass(frozen=True)
class PaneRect:
    x: float
    y: float
    w: float
    h: float


@dataclass(frozen=True)
class PortalTiles:
    frame: bytes
    input_scroll: bytes
    output_scroll: bytes


# ─── CLI defaults ─────────────────────────────────────────────────────────────

DEFAULT_PSK_ENV = "TZE_HUD_PSK"
DEFAULT_TARGET = "tzehouse-windows.parrot-hen.ts.net:50051"
DEFAULT_DOC = "docs/exemplar-manual-review-checklist.md"
DEFAULT_TRANSCRIPT_PATH = "test_results/text-stream-portal-latest.json"
MAX_MARKDOWN_BYTES = 65535

# ─── Scroll contract tokens ──────────────────────────────────────────────────

SCROLL_TOTAL_LINES = 80
SCROLL_VISIBLE_LINES = 14
SCROLL_STEP_PX = 40.0
SCROLL_LINE_PX = 20.0
SCROLL_PHASE_PAUSE_S = 2.5

# ─── Content helpers ──────────────────────────────────────────────────────────


def load_transcript_slice(doc_path: str, max_lines: int) -> str:
    """Load the markdown file and trim to a bounded viewport."""
    raw = Path(doc_path).read_text(encoding="utf-8")
    lines = raw.splitlines()
    return "\n".join(lines[:max_lines])


def bounded_transcript(lines: list[str], start: int, max_lines: int) -> str:
    """Return a viewport-sized markdown window within the protocol byte budget."""
    end = min(start + max_lines, len(lines))
    start = min(start, end)
    while start < end:
        joined = "\n".join(lines[start:end])
        if len(joined.encode("utf-8")) <= MAX_MARKDOWN_BYTES:
            return joined
        start += 1
    return ""


def make_solid_color_node(
    r: float, g: float, b: float, a: float,
    x: float, y: float, w: float, h: float,
    radius: float = -1.0,
    node_id: Optional[bytes] = None,
) -> types_pb2.NodeProto:
    data: dict[str, Any] = {
        "solid_color": {"r": r, "g": g, "b": b, "a": a},
        "bounds": [x, y, w, h],
    }
    if radius >= 0.0:
        data["solid_color"]["radius"] = radius
    if node_id is not None:
        data["id"] = node_id
    return _make_node(data)


def make_text_node(
    content: str, x: float, y: float, w: float, h: float,
    font_px: float, rgba: tuple[float, float, float, float],
    node_id: Optional[bytes] = None,
) -> types_pb2.NodeProto:
    # Explicit transparent background overrides any default RenderingPolicy
    # backdrop for TextMarkdown nodes.
    data: dict[str, Any] = {
        "text_markdown": {
            "content": content,
            "font_size_px": font_px,
            "color": list(rgba),
            "background": [0.0, 0.0, 0.0, 0.0],
        },
        "bounds": [x, y, w, h],
    }
    if node_id is not None:
        data["id"] = node_id
    return _make_node(data)


def make_hit_region(
    interaction_id: str, x: float, y: float, w: float, h: float,
) -> types_pb2.NodeProto:
    return _make_node(
        {
            "hit_region": {
                "interaction_id": interaction_id,
                "accepts_focus": True,
                "accepts_pointer": True,
            },
            "bounds": [x, y, w, h],
        }
    )


def portal_pane_rects() -> tuple[PaneRect, PaneRect]:
    """Return tile-local scroll-capture rects for input composer and output body."""
    pane_y = HEADER_H + DIVIDER_H
    pane_h = PORTAL_H - pane_y - FOOTER_H - DIVIDER_H
    input_pane_w = INPUT_PANE_W
    divider_x = input_pane_w
    output_pane_x = divider_x + PANE_DIVIDER_W
    output_pane_w = PORTAL_W - output_pane_x

    composer_inset = 14.0
    composer_x = composer_inset
    composer_y = pane_y + 40.0
    composer_w = input_pane_w - composer_inset * 2.0
    composer_h = pane_h - 40.0 - 44.0

    body_y = pane_y + 40.0
    body_h = pane_h - 40.0 - 8.0
    output_body = PaneRect(
        output_pane_x + PADDING_X,
        body_y,
        output_pane_w - PADDING_X * 2.0,
        body_h,
    )
    input_composer = PaneRect(composer_x, composer_y, composer_w, composer_h)
    return input_composer, output_body


def scroll_max_y_for_text(content: str, viewport_h: float, line_px: float) -> float:
    """Approximate max scroll offset for bounded text in a pane viewport."""
    line_count = max(1, len(content.splitlines()))
    return max(0.0, line_count * line_px - viewport_h)


def build_portal_nodes(
    title: str,
    subtitle: str,
    body: str,
    footer_meta: str,
    composer_text: str = "",
    composer_placeholder: str = "type a reply — Enter to submit",
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    """Return static frame/chrome nodes for a two-pane portal."""
    root_node = make_solid_color_node(
        *BG_RGBA, 0.0, 0.0, PORTAL_W, PORTAL_H, radius=PORTAL_RADIUS,
    )

    # ── Header chrome (full width) ────────────────────────────────────────
    header_bg = make_solid_color_node(
        *HEADER_BG_RGBA, 0.0, 0.0, PORTAL_W, HEADER_H,
    )
    header_divider = make_solid_color_node(
        *DIVIDER_RGBA, 0.0, HEADER_H, PORTAL_W, DIVIDER_H,
    )
    activity_dot = make_solid_color_node(
        *ACTIVITY_DOT_RGBA,
        PORTAL_W - PADDING_X - ACTIVITY_DOT_SIZE,
        (HEADER_H - ACTIVITY_DOT_SIZE) / 2.0,
        ACTIVITY_DOT_SIZE, ACTIVITY_DOT_SIZE,
        radius=ACTIVITY_DOT_SIZE / 2.0,
    )
    title_node = make_text_node(
        title,
        PADDING_X, 10.0,
        PORTAL_W - PADDING_X * 2.0 - ACTIVITY_DOT_SIZE - 12.0,
        22.0, TITLE_FONT, TITLE_RGBA,
    )
    subtitle_node = make_text_node(
        subtitle,
        PADDING_X, 31.0,
        PORTAL_W - PADDING_X * 2.0,
        16.0, SUBTITLE_FONT, SUBTITLE_RGBA,
    )

    # ── Pane geometry ─────────────────────────────────────────────────────
    pane_y = HEADER_H + DIVIDER_H
    pane_h = PORTAL_H - pane_y - FOOTER_H - DIVIDER_H
    input_pane_x = 0.0
    input_pane_w = INPUT_PANE_W
    divider_x = input_pane_x + input_pane_w
    output_pane_x = divider_x + PANE_DIVIDER_W
    output_pane_w = PORTAL_W - output_pane_x

    # ── Input pane (left): eyebrow → composer box → submit hint ──────────
    input_pane_bg = make_solid_color_node(
        *INPUT_PANE_BG_RGBA,
        input_pane_x, pane_y, input_pane_w, pane_h,
    )
    input_eyebrow = make_text_node(
        "INPUT",
        input_pane_x + PADDING_X, pane_y + 14.0,
        input_pane_w - PADDING_X * 2.0, 14.0,
        EYEBROW_FONT, EYEBROW_RGBA,
    )

    composer_inset = 14.0
    composer_x = input_pane_x + composer_inset
    composer_y = pane_y + 40.0
    composer_w = input_pane_w - composer_inset * 2.0
    composer_h = pane_h - 40.0 - 44.0  # leave room for submit-hint strip
    composer_bg = make_solid_color_node(
        *COMPOSER_BG_RGBA,
        composer_x, composer_y, composer_w, composer_h,
        radius=10.0,
    )
    # 1px inset border drawn as four thin rects — cheap highlight.
    border_t = make_solid_color_node(
        *COMPOSER_BORDER_RGBA, composer_x, composer_y, composer_w, 1.0,
    )
    border_b = make_solid_color_node(
        *COMPOSER_BORDER_RGBA,
        composer_x, composer_y + composer_h - 1.0, composer_w, 1.0,
    )
    border_l = make_solid_color_node(
        *COMPOSER_BORDER_RGBA, composer_x, composer_y, 1.0, composer_h,
    )
    border_r = make_solid_color_node(
        *COMPOSER_BORDER_RGBA,
        composer_x + composer_w - 1.0, composer_y, 1.0, composer_h,
    )

    # Submit hint strip at the bottom of the input pane.
    submit_hint_y = composer_y + composer_h + 10.0
    submit_hint = make_text_node(
        "Enter submit  ·  Shift+Enter newline  ·  Esc cancel",
        input_pane_x + PADDING_X,
        submit_hint_y,
        input_pane_w - PADDING_X * 2.0, 16.0,
        SUBMIT_HINT_FONT, SUBMIT_HINT_RGBA,
    )
    submit_hit = make_hit_region(
        SUBMIT_INTERACTION_ID,
        input_pane_x + PADDING_X,
        submit_hint_y - 6.0,
        input_pane_w - PADDING_X * 2.0, 24.0,
    )

    # ── Vertical divider between panes (drag-resize handle) ───────────────
    pane_divider = make_solid_color_node(
        *PANE_DIVIDER_RGBA,
        divider_x, pane_y, PANE_DIVIDER_W, pane_h,
    )
    grip_x = divider_x + (PANE_DIVIDER_W - PANE_DIVIDER_GRIP_W) / 2.0
    grip_y = pane_y + (pane_h - PANE_DIVIDER_GRIP_H) / 2.0
    pane_divider_grip = make_solid_color_node(
        *PANE_DIVIDER_GRIP_RGBA,
        grip_x, grip_y, PANE_DIVIDER_GRIP_W, PANE_DIVIDER_GRIP_H,
        radius=PANE_DIVIDER_GRIP_W / 2.0,
    )
    pane_resize_hit = make_hit_region(
        PANE_RESIZE_INTERACTION_ID,
        divider_x - 4.0, pane_y, PANE_DIVIDER_W + 8.0, pane_h,
    )

    # ── Output pane (right) ───────────────────────────────────────────────
    output_pane_bg = make_solid_color_node(
        *OUTPUT_PANE_BG_RGBA,
        output_pane_x, pane_y, output_pane_w, pane_h,
    )
    output_eyebrow = make_text_node(
        "TRANSCRIPT",
        output_pane_x + PADDING_X, pane_y + 14.0,
        output_pane_w - PADDING_X * 2.0, 14.0,
        EYEBROW_FONT, EYEBROW_RGBA,
    )
    # ── Footer ────────────────────────────────────────────────────────────
    footer_divider = make_solid_color_node(
        *DIVIDER_RGBA, 0.0, PORTAL_H - FOOTER_H - DIVIDER_H,
        PORTAL_W, DIVIDER_H,
    )
    footer_bg = make_solid_color_node(
        *FOOTER_BG_RGBA, 0.0, PORTAL_H - FOOTER_H,
        PORTAL_W, FOOTER_H,
    )
    footer_node = make_text_node(
        footer_meta,
        PADDING_X, PORTAL_H - FOOTER_H + 8.0,
        PORTAL_W - PADDING_X * 2.0, 16.0,
        META_FONT, META_RGBA,
    )

    children = [
        # header
        header_bg, header_divider, activity_dot, title_node, subtitle_node,
        # input pane
        input_pane_bg, input_eyebrow,
        composer_bg, border_t, border_b, border_l, border_r,
        submit_hint, submit_hit,
        # divider (fat drag handle)
        pane_divider, pane_divider_grip, pane_resize_hit,
        # output pane
        output_pane_bg, output_eyebrow,
        # footer
        footer_divider, footer_bg, footer_node,
    ]
    return root_node, children


def build_input_scroll_nodes(
    composer_text: str = "",
    composer_placeholder: str = "type a reply — Enter to submit",
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    input_rect, _ = portal_pane_rects()
    text_inset = 12.0
    root = make_hit_region(COMPOSER_INTERACTION_ID, 0.0, 0.0, input_rect.w, input_rect.h)
    text_node = make_text_node(
        composer_text or composer_placeholder,
        text_inset,
        text_inset,
        input_rect.w - text_inset * 2.0,
        input_rect.h - text_inset * 2.0,
        INPUT_FONT,
        INPUT_TEXT_RGBA if composer_text else INPUT_PLACEHOLDER_RGBA,
    )
    caret = make_solid_color_node(
        *CARET_RGBA,
        text_inset,
        text_inset + INPUT_FONT + 2.0,
        8.0,
        2.0,
    )
    return root, [text_node, caret]


def build_output_scroll_nodes(body: str) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    _, output_rect = portal_pane_rects()
    root = make_hit_region(SCROLL_INTERACTION_ID, 0.0, 0.0, output_rect.w, output_rect.h)
    body_node = make_text_node(
        body,
        0.0,
        0.0,
        output_rect.w,
        output_rect.h,
        BODY_FONT,
        BODY_RGBA,
    )
    return root, [body_node]


async def set_root_with_children(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    root: types_pb2.NodeProto,
    children: list[types_pb2.NodeProto],
) -> None:
    mr = await client.submit_mutation_batch(
        lease_id,
        [types_pb2.MutationProto(
            set_tile_root=types_pb2.SetTileRootMutation(
                tile_id=tile_id, node=root,
            ),
        )],
    )
    root_id = mr.created_ids[0] if mr.created_ids else root.id
    print(f"  [grpc] Tile root set; server root_id={root_id.hex()[:16]}...", flush=True)
    for child in children:
        await client.add_node(lease_id, tile_id, child, parent_id=root_id)


async def publish_portal(
    client: HudClient,
    lease_id: bytes,
    tiles: PortalTiles,
    title: str,
    subtitle: str,
    body: str,
    footer_meta: str,
    include_tile_setup: bool,
    composer_text: str = "",
) -> None:
    """Publish the portal scene.

    Server rewrites the root id on set_tile_root, so set_tile_root is
    submitted alone first and the server-assigned id is used as parent_id
    for subsequent add_node calls. Batching set_tile_root + add_node fails
    under atomic-batch semantics.
    """
    if include_tile_setup:
        for tile_id in (tiles.frame, tiles.input_scroll, tiles.output_scroll):
            await client.update_tile_opacity(lease_id, tile_id, 1.0)
            await client.update_tile_input_mode(
                lease_id, tile_id, types_pb2.TILE_INPUT_MODE_CAPTURE,
            )
        input_rect, output_rect = portal_pane_rects()
        await client.submit_mutation_batch(
            lease_id,
            [
                register_tile_scroll_mutation(
                    tiles.input_scroll,
                    scrollable_y=True,
                    content_height=scroll_max_y_for_text(
                        composer_text, input_rect.h, SCROLL_LINE_PX,
                    ),
                ),
                register_tile_scroll_mutation(
                    tiles.output_scroll,
                    scrollable_y=True,
                    content_height=scroll_max_y_for_text(
                        body, output_rect.h, SCROLL_LINE_PX,
                    ),
                ),
            ],
        )

    frame_root, frame_children = build_portal_nodes(title, subtitle, body, footer_meta)
    await set_root_with_children(client, lease_id, tiles.frame, frame_root, frame_children)

    input_root, input_children = build_input_scroll_nodes(composer_text)
    await set_root_with_children(client, lease_id, tiles.input_scroll, input_root, input_children)

    output_root, output_children = build_output_scroll_nodes(body)
    await set_root_with_children(client, lease_id, tiles.output_scroll, output_root, output_children)


async def create_portal_tiles(
    client: HudClient,
    lease_id: bytes,
    portal_x: float,
) -> PortalTiles:
    input_rect, output_rect = portal_pane_rects()
    frame = await client.create_tile(
        lease_id,
        x=portal_x,
        y=PORTAL_Y,
        w=PORTAL_W,
        h=PORTAL_H,
        z_order=PORTAL_Z,
    )
    input_scroll = await client.create_tile(
        lease_id,
        x=portal_x + input_rect.x,
        y=PORTAL_Y + input_rect.y,
        w=input_rect.w,
        h=input_rect.h,
        z_order=PORTAL_Z + 1,
    )
    output_scroll = await client.create_tile(
        lease_id,
        x=portal_x + output_rect.x,
        y=PORTAL_Y + output_rect.y,
        w=output_rect.w,
        h=output_rect.h,
        z_order=PORTAL_Z + 1,
    )
    return PortalTiles(frame=frame, input_scroll=input_scroll, output_scroll=output_scroll)


def register_tile_scroll_mutation(
    tile_id: bytes,
    *,
    scrollable_y: bool = True,
    content_height: float = -1.0,
) -> types_pb2.MutationProto:
    return types_pb2.MutationProto(
        register_tile_scroll=types_pb2.RegisterTileScrollMutation(
            tile_id=tile_id,
            scrollable_x=False,
            scrollable_y=scrollable_y,
            content_width=-1.0,
            content_height=content_height,
        )
    )


def set_scroll_offset_mutation(
    tile_id: bytes,
    offset_x: float,
    offset_y: float,
) -> types_pb2.MutationProto:
    return types_pb2.MutationProto(
        set_scroll_offset=types_pb2.SetScrollOffsetMutation(
            tile_id=tile_id,
            offset_x=offset_x,
            offset_y=offset_y,
        )
    )


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


async def run_baseline(
    client: HudClient, lease_id: bytes, tiles: PortalTiles,
    body_full: str, transcript: list[dict[str, Any]], hold_s: float,
) -> None:
    emit_step_event(transcript, 1, "started", {
        "code": "baseline",
        "title": "Baseline render",
        "action": "publish full portal chrome + transcript viewport",
        "expected_visual": "portal surface appears at right edge with header, body, footer",
    })
    total_lines = len(body_full.splitlines())
    await publish_portal(
        client, lease_id, tiles,
        title="Exemplar Review Portal",
        subtitle="docs/exemplar-manual-review-checklist.md",
        body=body_full,
        footer_meta=f"lines 1-{total_lines}  •  content-layer  •  live",
        include_tile_setup=True,
    )
    emit_step_event(transcript, 1, "completed", {
        "code": "baseline",
        "title": "Baseline render",
        "action": "hold for operator observation",
        "expected_visual": "portal surface visible; body text readable",
    }, hold_s=hold_s, lines=total_lines)
    await asyncio.sleep(hold_s)


async def run_scroll(
    client: HudClient, lease_id: bytes, tiles: PortalTiles,
    transcript: list[dict[str, Any]],
) -> None:
    """Exercise the transcript interaction contract inside the portal output pane."""
    emit_step_event(transcript, 4, "started", {
        "code": "scroll",
        "title": "Output scroll contract",
        "action": "mount long output, register scroll, step offset, append tail, return",
        "expected_visual": "OUTPUT pane scrolls through bounded transcript data, then returns to latest lines",
    })

    history = [
        f"[{i:03d}] Stream output line {i}: {'data ' * 8}".rstrip()
        for i in range(SCROLL_TOTAL_LINES)
    ]
    await client.submit_mutation_batch(
        lease_id,
        [register_tile_scroll_mutation(
            tiles.output_scroll,
            scrollable_y=True,
            content_height=max(
                0.0,
                len(history) * SCROLL_LINE_PX - portal_pane_rects()[1].h,
            ),
        )],
    )
    viewport_start = 0
    await publish_portal(
        client, lease_id, tiles,
        title="Exemplar Review Portal",
        subtitle="Transcript Interaction Contract",
        body=bounded_transcript(history, viewport_start, SCROLL_VISIBLE_LINES),
        footer_meta=(
            f"scroll  •  lines {viewport_start + 1}-"
            f"{viewport_start + SCROLL_VISIBLE_LINES} / {len(history)}"
        ),
        include_tile_setup=True,
    )
    emit_step_event(transcript, 4, "checkpoint", {
        "code": "scroll:mount",
        "title": "Mount long output",
        "action": "registered vertical scroll config and mounted bounded output",
        "expected_visual": "OUTPUT pane shows first transcript window within portal bounds",
    }, visible_lines=SCROLL_VISIBLE_LINES, total_lines=len(history))
    await asyncio.sleep(SCROLL_PHASE_PAUSE_S)

    scroll_offset = 0.0
    for step in range(4):
        scroll_offset += SCROLL_STEP_PX
        viewport_start = min(
            len(history) - SCROLL_VISIBLE_LINES,
            int(scroll_offset // SCROLL_LINE_PX),
        )
        await client.submit_mutation_batch(
            lease_id,
            [set_scroll_offset_mutation(tiles.output_scroll, 0.0, scroll_offset)],
        )
        await publish_portal(
            client, lease_id, tiles,
            title="Exemplar Review Portal",
            subtitle="Transcript Interaction Contract",
            body=bounded_transcript(history, viewport_start, SCROLL_VISIBLE_LINES),
            footer_meta=(
                f"scroll_y={scroll_offset:.0f}px  •  lines "
                f"{viewport_start + 1}-{viewport_start + SCROLL_VISIBLE_LINES}"
            ),
            include_tile_setup=False,
        )
        emit_step_event(transcript, 4, "checkpoint", {
            "code": "scroll:offset",
            "title": "Scroll output window",
            "action": f"set scroll_y={scroll_offset:.0f}px",
            "expected_visual": "OUTPUT pane advances through transcript while chrome remains readable",
        }, scroll_y=scroll_offset, viewport_start=viewport_start, scroll_step=step + 1)
        await asyncio.sleep(0.5)

    await asyncio.sleep(SCROLL_PHASE_PAUSE_S)
    mid_scroll = scroll_offset
    for i in range(5):
        history.append(f"[NEW-{i:02d}] Tail append at t+{i}: live output arriving")
        await publish_portal(
            client, lease_id, tiles,
            title="Exemplar Review Portal",
            subtitle="Transcript Interaction Contract",
            body=bounded_transcript(history, viewport_start, SCROLL_VISIBLE_LINES),
            footer_meta=(
                f"mid-scroll append  •  held scroll_y={mid_scroll:.0f}px  •  "
                f"tail={len(history)} lines"
            ),
            include_tile_setup=False,
        )
        emit_step_event(transcript, 4, "checkpoint", {
            "code": "scroll:append",
            "title": "Append while mid-scroll",
            "action": f"append line {len(history) - 1} while preserving scroll_y",
            "expected_visual": "visible output window does not jump to the tail",
        }, scroll_y=mid_scroll, total_lines=len(history))
        await asyncio.sleep(0.6)

    await client.submit_mutation_batch(
        lease_id,
        [set_scroll_offset_mutation(tiles.output_scroll, 0.0, 0.0)],
    )
    tail_start = max(0, len(history) - SCROLL_VISIBLE_LINES)
    await publish_portal(
        client, lease_id, tiles,
        title="Exemplar Review Portal",
        subtitle="Transcript Interaction Contract",
        body=bounded_transcript(history, tail_start, SCROLL_VISIBLE_LINES),
        footer_meta=f"tail  •  lines {tail_start + 1}-{len(history)} / {len(history)}",
        include_tile_setup=False,
    )
    emit_step_event(transcript, 4, "completed", {
        "code": "scroll",
        "title": "Output scroll contract",
        "action": "returned scroll offset to tail",
        "expected_visual": "latest appended lines are visible in OUTPUT pane",
    }, tail_start=tail_start, total_lines=len(history))
    await asyncio.sleep(SCROLL_PHASE_PAUSE_S)


async def run_streaming(
    client: HudClient, lease_id: bytes, tiles: PortalTiles,
    body_full: str, transcript: list[dict[str, Any]],
    chunks: int, chunk_interval_s: float,
) -> None:
    emit_step_event(transcript, 2, "started", {
        "code": "streaming",
        "title": "Incremental streaming reveal",
        "action": f"reveal body in {chunks} ordered chunks",
        "expected_visual": "body grows over time; header/footer unchanged",
    })
    lines = body_full.splitlines()
    per_chunk = max(1, len(lines) // chunks)
    for i in range(1, chunks + 1):
        end = min(len(lines), per_chunk * i) if i < chunks else len(lines)
        partial = "\n".join(lines[:end])
        await publish_portal(
            client, lease_id, tiles,
            title="Exemplar Review Portal",
            subtitle="docs/exemplar-manual-review-checklist.md",
            body=partial,
            footer_meta=f"streaming  •  lines 1-{end} / {len(lines)}",
            include_tile_setup=False,
        )
        if i < chunks:
            await asyncio.sleep(chunk_interval_s)
    emit_step_event(transcript, 2, "completed", {
        "code": "streaming",
        "title": "Incremental streaming reveal",
        "action": "final chunk published",
        "expected_visual": "full body visible, matches baseline",
    })


async def run_rapid(
    client: HudClient, lease_id: bytes, tiles: PortalTiles,
    body_full: str, transcript: list[dict[str, Any]],
    cycles: int, interval_ms: int,
) -> None:
    emit_step_event(transcript, 3, "started", {
        "code": "rapid",
        "title": "Rapid replace (coalescing smoke)",
        "action": f"publish {cycles} alternating bodies, ~{interval_ms}ms apart",
        "expected_visual": "portal remains coherent; no collapse to latest-only line",
    })
    lines = body_full.splitlines()
    alt_bodies = [
        "\n".join(lines[: max(8, len(lines) // 2)]),
        "\n".join(lines),
    ]
    for i in range(cycles):
        body = alt_bodies[i % 2]
        await publish_portal(
            client, lease_id, tiles,
            title="Exemplar Review Portal",
            subtitle="docs/exemplar-manual-review-checklist.md",
            body=body,
            footer_meta=f"rapid  •  cycle {i+1}/{cycles}",
            include_tile_setup=False,
        )
        await asyncio.sleep(interval_ms / 1000.0)
    await publish_portal(
        client, lease_id, tiles,
        title="Exemplar Review Portal",
        subtitle="docs/exemplar-manual-review-checklist.md",
        body=body_full,
        footer_meta=f"rapid-settled  •  lines 1-{len(lines)}",
        include_tile_setup=False,
    )
    emit_step_event(transcript, 3, "completed", {
        "code": "rapid",
        "title": "Rapid replace (coalescing smoke)",
        "action": "settled on full body",
        "expected_visual": "full body visible, no tearing",
    })


async def run_scenario(args: argparse.Namespace) -> int:
    psk = os.getenv(args.psk_env, "")
    if not psk:
        print(json.dumps({"error": "missing_psk", "psk_env": args.psk_env},
                         sort_keys=True), file=sys.stderr)
        return 2

    body = load_transcript_slice(args.doc, args.max_lines)
    transcript: list[dict[str, Any]] = []
    client = HudClient(
        args.target,
        psk=psk,
        agent_id=args.agent_id,
        capabilities=["create_tiles", "modify_own_tiles", "access_input_events"],
        initial_subscriptions=["SCENE_TOPOLOGY", "INPUT_EVENTS"],
    )
    heartbeat_task: Optional[asyncio.Task] = None

    try:
        emit_step_event(transcript, 0, "started", {
            "code": "scenario",
            "title": "Text Stream Portal live scenario",
            "action": "connect and open resident session",
            "expected_visual": "operator follows JSON step transcript",
        }, target=args.target, doc=args.doc, phases=args.phases)

        await client.connect()
        lease_id = await client.request_lease(ttl_ms=180_000)
        portal_x = args.tab_width - PORTAL_W - PORTAL_X_FROM_RIGHT
        tiles = await create_portal_tiles(
            client=client,
            lease_id=lease_id,
            portal_x=portal_x,
        )
        heartbeat_interval_ms = client.heartbeat_interval_ms or 5_000
        heartbeat_task = asyncio.create_task(
            heartbeat_loop(client, heartbeat_interval_ms)
        )

        phases = [p.strip() for p in (args.phases or "baseline").split(",")]
        for phase in phases:
            if phase == "baseline":
                await run_baseline(
                    client, lease_id, tiles, body, transcript,
                    args.baseline_hold_s,
                )
            elif phase == "scroll":
                await run_scroll(client, lease_id, tiles, transcript)
            elif phase == "streaming":
                await run_streaming(
                    client, lease_id, tiles, body, transcript,
                    args.stream_chunks, args.stream_interval_s,
                )
            elif phase == "rapid":
                await run_rapid(
                    client, lease_id, tiles, body, transcript,
                    args.rapid_cycles, args.rapid_interval_ms,
                )
            else:
                emit_step_event(transcript, -1, "skipped", {
                    "code": f"unknown:{phase}",
                    "title": f"unknown phase {phase!r}",
                    "action": "no-op",
                    "expected_visual": "—",
                })

        emit_step_event(transcript, 99, "completed", {
            "code": "scenario_complete",
            "title": "Text Stream Portal scenario complete",
            "action": "review transcript and capture UX notes",
            "expected_visual": "portal visible until session closes",
        })
    finally:
        if heartbeat_task is not None:
            heartbeat_task.cancel()
            try:
                await heartbeat_task
            except asyncio.CancelledError:
                pass
        try:
            await client.close(reason="portal-exemplar done", expect_resume=False)
        except Exception:
            pass
        if args.transcript_out:
            write_transcript(args.transcript_out, {
                "target": args.target,
                "doc": args.doc,
                "portal_w": PORTAL_W,
                "portal_h": PORTAL_H,
                "steps": transcript,
            })

    return 0


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Run the Text Stream Portal live resident gRPC scenario."
    )
    p.add_argument("--target", default=DEFAULT_TARGET)
    p.add_argument("--psk-env", default=DEFAULT_PSK_ENV)
    p.add_argument("--agent-id", default="agent-alpha")
    p.add_argument("--doc", default=DEFAULT_DOC)
    p.add_argument("--max-lines", type=int, default=120)
    p.add_argument("--tab-width", type=float, default=1920.0)
    p.add_argument("--phases", default="baseline,scroll",
                   help="Comma list: baseline,scroll,streaming,rapid")
    p.add_argument("--baseline-hold-s", type=float, default=20.0)
    p.add_argument("--stream-chunks", type=int, default=6)
    p.add_argument("--stream-interval-s", type=float, default=1.5)
    p.add_argument("--rapid-cycles", type=int, default=12)
    p.add_argument("--rapid-interval-ms", type=int, default=80)
    p.add_argument("--transcript-out", default=DEFAULT_TRANSCRIPT_PATH)
    return p.parse_args()


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
