#!/usr/bin/env python3
"""
Text Stream Portal exemplar user-test scenario.

Renders a two-pane bounded portal surface (INPUT left · OUTPUT right) on the
live HUD via the resident `HudSession` gRPC stream, using only existing scene
primitives (SolidColor, TextMarkdown, HitRegion). No terminal-emulator node,
no new node type, no chrome-hosted affordances.

Layout:
  - Equal 50/50 split between INPUT and OUTPUT panes
  - Header drag surface moves the whole portal group.
  - Fat 6px divider (with centred grip bar + hit region) between panes.
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
import base64
import contextlib
import json
import os
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

# Resolve HudClient + proto stubs (co-located).
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _SCRIPT_DIR)
sys.path.insert(0, os.path.join(_SCRIPT_DIR, "proto_gen"))

from hud_grpc_client import HudClient, _make_node  # noqa: E402
from proto_gen import events_pb2, types_pb2  # noqa: E402


# ─── Portal chrome tokens (iterate here) ──────────────────────────────────────

PORTAL_W = 860.0
PORTAL_H = 680.0
PORTAL_MIN_W = 640.0
PORTAL_MIN_H = 480.0
PORTAL_MAX_W = 1280.0
PORTAL_MAX_H = 960.0
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
TEXT_WINDOW_BG_RGBA = (0.0, 0.0, 0.0, 0.95)
COMPOSER_BORDER_RGBA = (1.0, 1.0, 1.0, 0.12)
SUBMIT_HINT_RGBA = (0.54, 0.60, 0.68, 0.90)
EYEBROW_RGBA = (0.70, 0.76, 0.84, 0.90)
CARET_RGBA = (0.48, 0.86, 0.56, 0.95)
STATIC_CARET_RGBA = (0.48, 0.86, 0.56, 0.0)

TITLE_RGBA = (0.98, 0.99, 1.0, 1.0)
SUBTITLE_RGBA = (0.78, 0.82, 0.88, 0.88)
BODY_RGBA = (0.92, 0.94, 0.97, 0.98)
META_RGBA = (0.66, 0.70, 0.76, 0.82)
ACTIVITY_DOT_RGBA = (0.48, 0.86, 0.56, 0.92)
INPUT_TEXT_RGBA = (0.88, 0.92, 0.98, 0.96)
INPUT_PLACEHOLDER_RGBA = (0.50, 0.55, 0.64, 0.78)
HEADER_GRIP_RGBA = (1.0, 1.0, 1.0, 0.66)

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
MIN_PANE_W = 240.0
INPUT_PANE_W = (PORTAL_W - PANE_DIVIDER_W) / 2.0
PANE_DIVIDER_RGBA = (1.0, 1.0, 1.0, 0.14)
PANE_DIVIDER_GRIP_RGBA = (1.0, 1.0, 1.0, 0.40)
PANE_DIVIDER_GRIP_H = 44.0
PANE_DIVIDER_GRIP_W = 2.0
PORTAL_RESIZE_HANDLE = 22.0
MINIMIZED_ICON_SIZE = 58.0
MINIMIZED_ICON_RADIUS = 17.0
MINIMIZE_BUTTON_SIZE = 22.0
MINIMIZE_BUTTON_X = 10.0
MINIMIZE_BUTTON_Y = 14.0
MINIMIZE_HIT_W = 44.0
MINIMIZE_HIT_H = HEADER_H

SCROLL_INTERACTION_ID = "portal-scroll"
SUBMIT_INTERACTION_ID = "portal-submit"
COMPOSER_INTERACTION_ID = "portal-composer-focus"
PANE_RESIZE_INTERACTION_ID = "portal-pane-resize"
PORTAL_DRAG_INTERACTION_ID = "portal-drag-header"
PORTAL_RESIZE_INTERACTION_ID = "portal-resize-bottom-right"
PORTAL_MINIMIZE_INTERACTION_ID = "portal-minimize"
PORTAL_RESTORE_INTERACTION_ID = "portal-restore"
PORTAL_ICON_DRAG_INTERACTION_ID = "portal-icon-drag"


@dataclass(frozen=True)
class PaneRect:
    x: float
    y: float
    w: float
    h: float


@dataclass(frozen=True)
class PortalTiles:
    capture_backstop: bytes
    frame: bytes
    input_scroll: bytes
    output_scroll: bytes
    drag_shield: bytes
    minimized_icon: bytes
    tab_width: float
    tab_height: float


@dataclass(frozen=True)
class ComposerVisualLine:
    text: str
    positions: tuple[tuple[int, float], ...]


# ─── CLI defaults ─────────────────────────────────────────────────────────────

DEFAULT_PSK_ENV = "TZE_HUD_PSK"
DEFAULT_TARGET = "tzehouse-windows.parrot-hen.ts.net:50051"
DEFAULT_DOC = "docs/exemplar-manual-review-checklist.md"
DEFAULT_TRANSCRIPT_PATH = "test_results/text-stream-portal-latest.json"
DEFAULT_SSH_KEY = os.path.expanduser("~/.ssh/ecdsa_home")
MAX_MARKDOWN_BYTES = 65535
DRAG_MAX_SECONDS = 12.0
DRAG_IDLE_RELEASE_SECONDS = 1.0
DRAG_APPLY_MIN_INTERVAL_SECONDS = 0.025
ICON_DRAG_APPLY_MIN_INTERVAL_SECONDS = 0.008
ICON_DRAG_START_THRESHOLD_PX = 20.0
KEY_ECHO_TIMEOUT_SECONDS = 1.0
COMPOSER_RENDER_DEBOUNCE_SECONDS = 0.02
COMPOSER_CARET_BLINK_SECONDS = 0.45
COMPOSER_CARET_W = 2.0
COMPOSER_CARET_H = INPUT_FONT + 5.0
COMPOSER_WRAP_CHAR_W = INPUT_FONT * 0.57
COMPOSER_CARET_CHAR_W = COMPOSER_WRAP_CHAR_W
COMPOSER_TEXT_RENDER_MARGIN_X = 6.0
COMPOSER_TEXT_RENDER_MARGIN_Y = 6.0
COMPOSER_WRAP_SAFETY_PX = INPUT_FONT * 2.0
COMPOSER_HIT_PAD = 18.0
COMPOSER_NODE_IDS = {
    "root": uuid.uuid4().bytes,
    "hit": uuid.uuid4().bytes,
    "text": uuid.uuid4().bytes,
    "caret": uuid.uuid4().bytes,
}
COMPOSER_RUNTIME_NODE_IDS: dict[str, bytes] = {}
FRAME_RUNTIME_NODE_IDS: dict[str, bytes] = {}
PORTAL_STATUS_STATE: dict[str, bool] = {
    "minimized": False,
    "attention": False,
    "pulse": False,
}
FRAME_CHILD_KEYS = [
    # header
    "header_bg", "header_divider", "minimize_bg", "minimize_glyph",
    "minimize_hit", "activity_dot", "title", "subtitle",
    "header_grip", "portal_drag_hit",
    # input pane
    "input_pane_bg", "input_eyebrow",
    "composer_bg", "border_t", "border_b", "border_l", "border_r",
    "submit_hint", "submit_hit",
    # divider
    "pane_divider", "pane_divider_grip", "pane_resize_hit",
    # output pane
    "output_pane_bg", "output_eyebrow", "output_text_window_bg",
    # footer
    "footer_divider", "footer_bg", "footer_node", "resize_handle", "resize_hit",
]


def fresh_composer_node_ids() -> dict[str, bytes]:
    return {
        "root": uuid.uuid4().bytes,
        "hit": uuid.uuid4().bytes,
        "text": uuid.uuid4().bytes,
        "caret": uuid.uuid4().bytes,
    }

# ─── Scroll contract tokens ──────────────────────────────────────────────────

SCROLL_TOTAL_LINES = 80
SCROLL_VISIBLE_LINES = 14
SCROLL_STEP_PX = 40.0
SCROLL_LINE_PX = 20.0
SCROLL_PHASE_PAUSE_S = 2.5
COMPOSER_LINE_PX = INPUT_FONT * 1.4

# ─── Content helpers ──────────────────────────────────────────────────────────


def load_transcript_slice(doc_path: str, max_lines: int) -> str:
    """Load the markdown file and trim to a bounded viewport."""
    raw = Path(doc_path).read_text(encoding="utf-8")
    lines = raw.splitlines()
    return "\n".join(lines[:max_lines])


def normalize_composer_input(text: str) -> str:
    return text.replace("\r\n", "\n").replace("\r", "\n")


def composer_key_fallback_text(key: str) -> Optional[str]:
    """Return printable text for key-downs that do not emit Character events."""
    if key == "Space":
        return " "
    return None


def composer_wrap_char_width_px(ch: str) -> float:
    """Approximate SystemMonospace advance for visual caret placement."""
    if ch == "\t":
        return COMPOSER_WRAP_CHAR_W * 4.0
    return COMPOSER_WRAP_CHAR_W


def composer_caret_char_width_px(ch: str) -> float:
    if ch == "\t":
        return COMPOSER_CARET_CHAR_W * 4.0
    return COMPOSER_CARET_CHAR_W


def composer_wrap_text_width_px(text: str) -> float:
    return sum(composer_wrap_char_width_px(ch) for ch in text)


def composer_visual_lines(
    text: str,
    max_width_px: float,
) -> list[ComposerVisualLine]:
    lines: list[ComposerVisualLine] = []
    line_chars: list[str] = []
    positions: list[tuple[int, float]] = [(0, 0.0)]
    wrap_line_width = 0.0
    caret_line_width = 0.0

    def add_position(index: int) -> None:
        if positions and positions[-1][0] == index:
            return
        positions.append((index, caret_line_width))

    def finish_line() -> None:
        lines.append(ComposerVisualLine("".join(line_chars), tuple(positions)))

    def start_line(index: int, zero_positions: tuple[int, ...] = ()) -> None:
        nonlocal line_chars, positions, wrap_line_width, caret_line_width
        finish_line()
        line_chars = []
        wrap_line_width = 0.0
        caret_line_width = 0.0
        positions = [(index, 0.0)]
        for position in zero_positions:
            add_position(position)

    def append_char(index: int, ch: str) -> None:
        nonlocal wrap_line_width, caret_line_width
        line_chars.append(ch)
        wrap_line_width += composer_wrap_char_width_px(ch)
        caret_line_width += composer_caret_char_width_px(ch)
        add_position(index + 1)

    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "\n":
            add_position(i)
            start_line(i + 1)
            i += 1
            continue

        if ch.isspace():
            run_end = i + 1
            while run_end < len(text) and text[run_end].isspace() and text[run_end] != "\n":
                run_end += 1
            next_word_end = run_end
            while next_word_end < len(text) and not text[next_word_end].isspace():
                next_word_end += 1

            whitespace_width = composer_wrap_text_width_px(text[i:run_end])
            next_word_width = composer_wrap_text_width_px(text[run_end:next_word_end])
            if (
                wrap_line_width > 0.0
                and run_end < len(text)
                and wrap_line_width + whitespace_width + next_word_width > max_width_px
            ):
                add_position(i)
                start_line(i + 1, tuple(range(i + 2, run_end + 1)))
                i = run_end
                continue

            while i < run_end:
                ch = text[i]
                wrap_ch_width = composer_wrap_char_width_px(ch)
                if wrap_line_width > 0.0 and wrap_line_width + wrap_ch_width > max_width_px:
                    start_line(i + 1)
                    i += 1
                    continue
                append_char(i, ch)
                i += 1
            continue

        word_end = i + 1
        while word_end < len(text) and not text[word_end].isspace():
            word_end += 1
        word = text[i:word_end]
        word_width = composer_wrap_text_width_px(word)
        if wrap_line_width > 0.0 and wrap_line_width + word_width > max_width_px:
            start_line(i)

        while i < word_end:
            ch = text[i]
            wrap_ch_width = composer_wrap_char_width_px(ch)
            if wrap_line_width > 0.0 and wrap_line_width + wrap_ch_width > max_width_px:
                start_line(i)
            append_char(i, ch)
            i += 1

    add_position(len(text))
    finish_line()
    return lines


def composer_wrapped_layout(
    text: str,
    cursor: int,
    max_width_px: float,
) -> tuple[str, float, int]:
    """Return explicit visual wrapping plus caret x/row for the raw cursor."""
    cursor = max(0, min(cursor, len(text)))
    lines = composer_visual_lines(text, max_width_px)
    cursor_x = 0.0
    cursor_row = 0
    for row, line in enumerate(lines):
        for index, x in line.positions:
            if index == cursor:
                cursor_x = x
                cursor_row = row
                break
        else:
            continue
        break
    else:
        cursor_row = len(lines) - 1
        cursor_x = lines[-1].positions[-1][1]

    return "\n".join(line.text for line in lines), cursor_x, cursor_row


def composer_text_area_width_px() -> float:
    composer_rect = input_composer_local_rect()
    text_inset = 12.0
    return max(
        0.0,
        composer_rect.w
        - text_inset * 2.0
        - COMPOSER_TEXT_RENDER_MARGIN_X * 2.0
        - COMPOSER_CARET_W,
    )


def composer_wrap_area_width_px() -> float:
    # The compositor still applies glyphon word wrap to the pre-wrapped string.
    # Keep our explicit lines narrower so glyphon does not add an extra row.
    return max(0.0, composer_text_area_width_px() - COMPOSER_WRAP_SAFETY_PX)


def composer_display_text(
    text: str,
    cursor: int,
    *,
    focused: bool,
) -> tuple[str, bool]:
    """Render composer text and report whether it should use placeholder styling."""
    cursor = max(0, min(cursor, len(text)))
    if not text and not focused:
        return "", True
    display_text, _, _ = composer_wrapped_layout(
        text,
        cursor,
        composer_wrap_area_width_px(),
    )
    return display_text, False


def composer_caret_layout(text: str, cursor: int) -> tuple[float, int]:
    _, cursor_x, cursor_row = composer_wrapped_layout(
        text,
        cursor,
        composer_wrap_area_width_px(),
    )
    return cursor_x, cursor_row


def composer_cursor_for_vertical_move(
    text: str,
    cursor: int,
    delta_rows: int,
    preferred_x: Optional[float],
) -> tuple[int, float]:
    """Move the raw cursor vertically through the explicit visual line model."""
    lines = composer_visual_lines(text, composer_wrap_area_width_px())
    _, current_x, current_row = composer_wrapped_layout(
        text,
        cursor,
        composer_wrap_area_width_px(),
    )
    target_x = current_x if preferred_x is None else preferred_x
    target_row = max(0, min(current_row + delta_rows, len(lines) - 1))
    if target_row == current_row:
        return cursor, target_x
    target_positions = lines[target_row].positions or ((0, 0.0),)
    target_cursor, _ = min(
        target_positions,
        key=lambda position: (abs(position[1] - target_x), position[0]),
    )
    return target_cursor, target_x


def composer_word_delete_start(text: str, cursor: int) -> int:
    """Return the cursor position after deleting the previous word cluster."""
    cursor = max(0, min(cursor, len(text)))
    if cursor == 0:
        return 0
    i = cursor
    while i > 0 and not (text[i - 1].isalnum() or text[i - 1] == "_"):
        i -= 1
    while i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_"):
        i -= 1
    return i


def composer_word_forward_end(text: str, cursor: int) -> int:
    """Return the cursor position after advancing over the next word cluster."""
    cursor = max(0, min(cursor, len(text)))
    i = cursor
    while i < len(text) and not (text[i].isalnum() or text[i] == "_"):
        i += 1
    while i < len(text) and (text[i].isalnum() or text[i] == "_"):
        i += 1
    return i


def clamp_portal_size(w: float, h: float, tab_width: float, tab_height: float) -> tuple[float, float]:
    return (
        max(PORTAL_MIN_W, min(w, min(PORTAL_MAX_W, tab_width))),
        max(PORTAL_MIN_H, min(h, min(PORTAL_MAX_H, tab_height))),
    )


def clamp_input_pane_width(width: float) -> float:
    max_input_w = max(MIN_PANE_W, PORTAL_W - PANE_DIVIDER_W - MIN_PANE_W)
    return max(MIN_PANE_W, min(width, max_input_w))


def set_input_pane_width(width: float) -> None:
    global INPUT_PANE_W
    INPUT_PANE_W = clamp_input_pane_width(width)


def set_portal_size(w: float, h: float, tab_width: float, tab_height: float) -> None:
    global PORTAL_W, PORTAL_H
    old_w = PORTAL_W
    PORTAL_W, PORTAL_H = clamp_portal_size(w, h, tab_width, tab_height)
    if old_w > 0:
        set_input_pane_width(INPUT_PANE_W * (PORTAL_W / old_w))
    else:
        set_input_pane_width((PORTAL_W - PANE_DIVIDER_W) / 2.0)


def target_host(target: str) -> str:
    return target.rsplit(":", 1)[0] if ":" in target else target


async def read_windows_clipboard(
    host: str,
    *,
    user: str,
    ssh_key: str,
    timeout_s: float,
) -> str:
    if not host:
        return ""
    cmd = [
        "ssh",
        "-i", ssh_key,
        "-o", "BatchMode=yes",
        "-o", "IdentitiesOnly=yes",
        "-o", "StrictHostKeyChecking=no",
        f"{user}@{host}",
        "powershell -NoProfile -Command \"Get-Clipboard -Raw\"",
    ]
    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )
        stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout_s)
    except (OSError, asyncio.TimeoutError, subprocess.SubprocessError):
        return ""
    if proc.returncode != 0:
        return ""
    return stdout.decode("utf-8", errors="replace").replace("\r\n", "\n").rstrip("\n")


async def read_windows_left_button_down(
    host: str,
    *,
    user: str,
    ssh_key: str,
    timeout_s: float,
) -> Optional[bool]:
    if not host:
        return None
    ps = (
        "Add-Type -Namespace Win32 -Name User32 -MemberDefinition "
        "'[DllImport(\"user32.dll\")] public static extern short GetAsyncKeyState(int vKey);'; "
        "$s=[Win32.User32]::GetAsyncKeyState(1); "
        "if (($s -band -32768) -ne 0) { 'down' } else { 'up' }"
    )
    cmd = [
        "ssh",
        "-i", ssh_key,
        "-o", "BatchMode=yes",
        "-o", "IdentitiesOnly=yes",
        "-o", "StrictHostKeyChecking=no",
        f"{user}@{host}",
        f"powershell -NoProfile -Command \"{ps}\"",
    ]
    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )
        stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout_s)
    except (OSError, asyncio.TimeoutError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    value = stdout.decode("utf-8", errors="replace").strip().lower()
    if value == "down":
        return True
    if value == "up":
        return False
    return None


def ps_single_quoted(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def build_diagnostic_input_plan(portal_x: float, portal_y: float) -> list[dict[str, Any]]:
    """Build an OS-input plan covering composer focus, drag, and output scroll."""
    input_rect, output_rect = portal_pane_rects()
    composer_x = portal_x + input_rect.x + input_rect.w / 2.0
    composer_y = portal_y + input_rect.y + min(input_rect.h - 10.0, 72.0)
    header_x = portal_x + PORTAL_W / 2.0
    header_y = portal_y + HEADER_H / 2.0
    drag_dx = -120.0
    drag_dy = 72.0
    output_x = portal_x + drag_dx + output_rect.x + output_rect.w / 2.0
    output_y = portal_y + drag_dy + output_rect.y + min(output_rect.h - 10.0, 96.0)
    return [
        {
            "kind": "click",
            "label": "focus-composer",
            "x": composer_x,
            "y": composer_y,
        },
        {
            "kind": "text",
            "label": "type-composer-text",
            "text": "diagnostic input",
        },
        {
            "kind": "drag",
            "label": "drag-portal-header",
            "start_x": header_x,
            "start_y": header_y,
            "end_x": header_x + drag_dx,
            "end_y": header_y + drag_dy,
            "steps": 8,
        },
        {
            "kind": "wheel",
            "label": "scroll-output-pane",
            "x": output_x,
            "y": output_y,
            "delta": -360,
            "count": 3,
        },
    ]


def windows_diagnostic_input_script(actions: list[dict[str, Any]]) -> str:
    """Return a PowerShell script that injects real Windows OS input events."""
    lines = [
        "$ErrorActionPreference = 'Stop'",
        "Add-Type -TypeDefinition @\"",
        "using System;",
        "using System.Runtime.InteropServices;",
        "public static class HudDiagnosticInput {",
        "  [DllImport(\"user32.dll\")] public static extern bool SetCursorPos(int X, int Y);",
        "  [DllImport(\"user32.dll\")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, UIntPtr extra);",
        "  [DllImport(\"user32.dll\", SetLastError=true)] public static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);",
        "  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION U; }",
        "  [StructLayout(LayoutKind.Explicit)] public struct INPUTUNION { [FieldOffset(0)] public KEYBDINPUT ki; }",
        "  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public UIntPtr dwExtraInfo; }",
        "}",
        "\"@",
        "$MOUSEEVENTF_LEFTDOWN = 0x0002",
        "$MOUSEEVENTF_LEFTUP = 0x0004",
        "$MOUSEEVENTF_WHEEL = 0x0800",
        "$INPUT_KEYBOARD = 1",
        "$KEYEVENTF_UNICODE = 0x0004",
        "$KEYEVENTF_KEYUP = 0x0002",
        "$InputSize = [System.Runtime.InteropServices.Marshal]::SizeOf([type][HudDiagnosticInput+INPUT])",
        "function Move-To([double]$x, [double]$y) {",
        "  [HudDiagnosticInput]::SetCursorPos([int][Math]::Round($x), [int][Math]::Round($y)) | Out-Null",
        "  Start-Sleep -Milliseconds 80",
        "}",
        "function Left-Down() { [HudDiagnosticInput]::mouse_event($MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80 }",
        "function Left-Up() { [HudDiagnosticInput]::mouse_event($MOUSEEVENTF_LEFTUP, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 120 }",
        "function Wheel-At([double]$x, [double]$y, [int]$delta, [int]$count) {",
        "  Move-To $x $y",
        "  for ($i = 0; $i -lt $count; $i++) {",
        "    [HudDiagnosticInput]::mouse_event($MOUSEEVENTF_WHEEL, 0, 0, $delta, [UIntPtr]::Zero)",
        "    Start-Sleep -Milliseconds 140",
        "  }",
        "}",
        "function Send-Text([string]$text) {",
        "  foreach ($ch in $text.ToCharArray()) {",
        "    $inputs = [HudDiagnosticInput+INPUT[]]::new(2)",
        "    $scan = [uint16][char]$ch",
        "    $inputs[0].type = $INPUT_KEYBOARD",
        "    $inputs[0].U.ki.wVk = 0",
        "    $inputs[0].U.ki.wScan = $scan",
        "    $inputs[0].U.ki.dwFlags = $KEYEVENTF_UNICODE",
        "    $inputs[1].type = $INPUT_KEYBOARD",
        "    $inputs[1].U.ki.wVk = 0",
        "    $inputs[1].U.ki.wScan = $scan",
        "    $inputs[1].U.ki.dwFlags = $KEYEVENTF_UNICODE -bor $KEYEVENTF_KEYUP",
        "    [HudDiagnosticInput]::SendInput(2, $inputs, $InputSize) | Out-Null",
        "    Start-Sleep -Milliseconds 25",
        "  }",
        "}",
    ]
    for action in actions:
        kind = str(action.get("kind", ""))
        if kind == "click":
            lines.extend([
                f"Write-Output {ps_single_quoted('diagnostic:' + str(action.get('label', 'click')))}",
                f"Move-To {float(action['x']):.1f} {float(action['y']):.1f}",
                "Left-Down",
                "Left-Up",
            ])
        elif kind == "drag":
            steps = max(1, int(action.get("steps", 8)))
            lines.extend([
                f"Write-Output {ps_single_quoted('diagnostic:' + str(action.get('label', 'drag')))}",
                f"$sx = {float(action['start_x']):.1f}; $sy = {float(action['start_y']):.1f}",
                f"$ex = {float(action['end_x']):.1f}; $ey = {float(action['end_y']):.1f}",
                f"$steps = {steps}",
                "Move-To $sx $sy",
                "Left-Down",
                "for ($i = 1; $i -le $steps; $i++) {",
                "  $t = [double]$i / [double]$steps",
                "  Move-To ($sx + (($ex - $sx) * $t)) ($sy + (($ey - $sy) * $t))",
                "}",
                "Left-Up",
            ])
        elif kind == "wheel":
            lines.extend([
                f"Write-Output {ps_single_quoted('diagnostic:' + str(action.get('label', 'wheel')))}",
                (
                    f"Wheel-At {float(action['x']):.1f} {float(action['y']):.1f} "
                    f"{int(action.get('delta', -120))} {max(1, int(action.get('count', 1)))}"
                ),
            ])
        elif kind == "text":
            lines.extend([
                f"Write-Output {ps_single_quoted('diagnostic:' + str(action.get('label', 'text')))}",
                f"Send-Text {ps_single_quoted(str(action.get('text', '')))}",
                "Start-Sleep -Milliseconds 120",
            ])
        else:
            raise ValueError(f"unsupported diagnostic action kind: {kind!r}")
    return "\n".join(lines) + "\n"


async def run_windows_diagnostic_input(
    host: str,
    *,
    user: str,
    ssh_key: str,
    actions: list[dict[str, Any]],
    timeout_s: float,
) -> dict[str, Any]:
    script = windows_diagnostic_input_script(actions)
    encoded = base64.b64encode(script.encode("utf-16le")).decode("ascii")
    cmd = [
        "ssh",
        "-i", ssh_key,
        "-o", "BatchMode=yes",
        "-o", "IdentitiesOnly=yes",
        "-o", "StrictHostKeyChecking=no",
        f"{user}@{host}",
        "powershell",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        encoded,
    ]
    started = time.time()
    proc: Optional[asyncio.subprocess.Process] = None
    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=timeout_s)
    except asyncio.TimeoutError:
        if proc is not None:
            with contextlib.suppress(Exception):
                proc.kill()
        return {
            "ok": False,
            "returncode": None,
            "error": "timeout",
            "duration_s": round(time.time() - started, 3),
        }
    except (OSError, subprocess.SubprocessError) as exc:
        return {
            "ok": False,
            "returncode": None,
            "error": f"{type(exc).__name__}: {exc}",
            "duration_s": round(time.time() - started, 3),
        }
    return {
        "ok": proc.returncode == 0,
        "returncode": proc.returncode,
        "stdout": stdout.decode("utf-8", errors="replace").strip(),
        "stderr": stderr.decode("utf-8", errors="replace").strip(),
        "duration_s": round(time.time() - started, 3),
    }


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
    preserve_markdown: bool = False,
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
    if preserve_markdown and content:
        data["text_markdown"]["color_runs"] = [{
            "start_byte": 0,
            "end_byte": len(content.encode("utf-8")),
            "color": list(rgba),
        }]
    if node_id is not None:
        data["id"] = node_id
    return _make_node(data)


def make_hit_region(
    interaction_id: str, x: float, y: float, w: float, h: float,
    *,
    accepts_focus: bool = True,
    auto_capture: bool = False,
    release_on_up: bool = False,
    node_id: Optional[bytes] = None,
) -> types_pb2.NodeProto:
    data: dict[str, Any] = {
        "hit_region": {
            "interaction_id": interaction_id,
            "accepts_focus": accepts_focus,
            "accepts_pointer": True,
            "auto_capture": auto_capture,
            "release_on_up": release_on_up,
        },
        "bounds": [x, y, w, h],
    }
    if node_id is not None:
        data["id"] = node_id
    return _make_node(data)


def portal_pane_rects() -> tuple[PaneRect, PaneRect]:
    """Return tile-local scroll-capture rects for input composer and output body."""
    pane_y = HEADER_H + DIVIDER_H
    pane_h = PORTAL_H - pane_y - FOOTER_H - DIVIDER_H
    input_pane_x = 0.0
    input_pane_w = INPUT_PANE_W
    divider_x = input_pane_w
    output_pane_x = divider_x + PANE_DIVIDER_W
    output_pane_w = PORTAL_W - output_pane_x

    composer_inset = 14.0
    composer_x = composer_inset
    composer_y = pane_y + 40.0
    composer_w = input_pane_w - composer_inset * 2.0
    composer_h = pane_h - 40.0 - 44.0
    tile_x = max(input_pane_x, composer_x - COMPOSER_HIT_PAD)
    tile_y = max(pane_y, composer_y - COMPOSER_HIT_PAD)
    tile_right = min(input_pane_x + input_pane_w, composer_x + composer_w + COMPOSER_HIT_PAD)
    tile_bottom = min(pane_y + pane_h, composer_y + composer_h + COMPOSER_HIT_PAD)

    body_y = pane_y + 40.0
    body_h = pane_h - 40.0 - 8.0
    output_body = PaneRect(
        output_pane_x + PADDING_X,
        body_y,
        output_pane_w - PADDING_X * 2.0,
        body_h,
    )
    input_composer = PaneRect(tile_x, tile_y, tile_right - tile_x, tile_bottom - tile_y)
    return input_composer, output_body


def input_composer_local_rect() -> PaneRect:
    """Return the visible composer box relative to the enlarged input tile."""
    input_tile, _ = portal_pane_rects()
    pane_y = HEADER_H + DIVIDER_H
    pane_h = PORTAL_H - pane_y - FOOTER_H - DIVIDER_H
    composer_inset = 14.0
    composer_x = composer_inset
    composer_y = pane_y + 40.0
    composer_w = INPUT_PANE_W - composer_inset * 2.0
    composer_h = pane_h - 40.0 - 44.0
    return PaneRect(
        composer_x - input_tile.x,
        composer_y - input_tile.y,
        composer_w,
        composer_h,
    )


def scroll_max_y_for_text(content: str, viewport_h: float, line_px: float) -> float:
    """Approximate max scroll offset for bounded text in a pane viewport."""
    line_count = max(1, len(content.splitlines()))
    return max(0.0, line_count * line_px - viewport_h)


def scroll_content_height_for_text(content: str, viewport_h: float, line_px: float) -> float:
    """Approximate full scroll content height for text in a pane viewport."""
    return viewport_h + scroll_max_y_for_text(content, viewport_h, line_px)


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
    minimize_bg = make_solid_color_node(
        0.10, 0.14, 0.20, 0.88,
        MINIMIZE_BUTTON_X,
        MINIMIZE_BUTTON_Y,
        MINIMIZE_BUTTON_SIZE,
        MINIMIZE_BUTTON_SIZE,
        radius=7.0,
    )
    minimize_glyph = make_solid_color_node(
        0.76, 0.82, 0.90, 0.96,
        MINIMIZE_BUTTON_X + 6.0,
        MINIMIZE_BUTTON_Y + 10.0,
        MINIMIZE_BUTTON_SIZE - 12.0,
        2.0,
        radius=1.0,
    )
    minimize_hit = make_hit_region(
        PORTAL_MINIMIZE_INTERACTION_ID,
        0.0,
        0.0,
        MINIMIZE_HIT_W,
        MINIMIZE_HIT_H,
        accepts_focus=False,
    )
    activity_dot = make_solid_color_node(
        *ACTIVITY_DOT_RGBA,
        PORTAL_W - PADDING_X - ACTIVITY_DOT_SIZE,
        (HEADER_H - ACTIVITY_DOT_SIZE) / 2.0,
        ACTIVITY_DOT_SIZE, ACTIVITY_DOT_SIZE,
        radius=ACTIVITY_DOT_SIZE / 2.0,
    )
    title_x = PADDING_X + MINIMIZE_BUTTON_SIZE + 18.0
    title_node = make_text_node(
        title,
        title_x, 10.0,
        PORTAL_W - title_x - PADDING_X - ACTIVITY_DOT_SIZE - 12.0,
        22.0, TITLE_FONT, TITLE_RGBA,
    )
    subtitle_node = make_text_node(
        subtitle,
        title_x, 31.0,
        PORTAL_W - title_x - PADDING_X,
        16.0, SUBTITLE_FONT, SUBTITLE_RGBA,
    )
    grip_w = 92.0
    grip_h = 6.0
    header_grip = make_solid_color_node(
        *HEADER_GRIP_RGBA,
        (PORTAL_W - grip_w) / 2.0,
        8.0,
        grip_w,
        grip_h,
        radius=grip_h / 2.0,
    )
    portal_drag_hit = make_hit_region(
        PORTAL_DRAG_INTERACTION_ID,
        MINIMIZE_HIT_W, 0.0,
        PORTAL_W - MINIMIZE_HIT_W, HEADER_H,
        accepts_focus=False,
        auto_capture=True,
        release_on_up=True,
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
        *TEXT_WINDOW_BG_RGBA,
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
        accepts_focus=False,
        auto_capture=True,
        release_on_up=True,
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
    _, output_rect = portal_pane_rects()
    output_text_window_bg = make_solid_color_node(
        *TEXT_WINDOW_BG_RGBA,
        output_rect.x,
        output_rect.y,
        output_rect.w,
        output_rect.h,
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
    resize_handle = make_solid_color_node(
        *PANE_DIVIDER_GRIP_RGBA,
        PORTAL_W - PORTAL_RESIZE_HANDLE,
        PORTAL_H - PORTAL_RESIZE_HANDLE,
        PORTAL_RESIZE_HANDLE,
        PORTAL_RESIZE_HANDLE,
        radius=6.0,
    )
    resize_hit = make_hit_region(
        PORTAL_RESIZE_INTERACTION_ID,
        PORTAL_W - PORTAL_RESIZE_HANDLE - 8.0,
        PORTAL_H - PORTAL_RESIZE_HANDLE - 8.0,
        PORTAL_RESIZE_HANDLE + 12.0,
        PORTAL_RESIZE_HANDLE + 12.0,
        accepts_focus=False,
        auto_capture=True,
        release_on_up=True,
    )

    children = [
        # header
        header_bg, header_divider, minimize_bg, minimize_glyph, minimize_hit,
        activity_dot, title_node, subtitle_node, header_grip, portal_drag_hit,
        # input pane
        input_pane_bg, input_eyebrow,
        composer_bg, border_t, border_b, border_l, border_r,
        submit_hint, submit_hit,
        # divider (fat drag handle)
        pane_divider, pane_divider_grip, pane_resize_hit,
        # output pane
        output_pane_bg, output_eyebrow, output_text_window_bg,
        # footer
        footer_divider, footer_bg, footer_node, resize_handle, resize_hit,
    ]
    return root_node, children


def build_input_scroll_nodes(
    composer_text: str = "",
    composer_placeholder: str = "type a reply — Enter to submit",
    *,
    node_ids: Optional[dict[str, bytes]] = None,
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    node_ids = node_ids or fresh_composer_node_ids()
    input_rect, _ = portal_pane_rects()
    composer_rect = input_composer_local_rect()
    text_inset = 12.0
    hit_h = input_rect.h + scroll_max_y_for_text(composer_text, composer_rect.h, SCROLL_LINE_PX)
    root = make_solid_color_node(
        0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, input_rect.w, input_rect.h,
        node_id=node_ids.get("root"),
    )
    hit = make_hit_region(
        COMPOSER_INTERACTION_ID,
        0.0, 0.0, input_rect.w, hit_h,
        node_id=node_ids.get("hit"),
    )
    text_node = make_text_node(
        composer_text or composer_placeholder,
        composer_rect.x + text_inset,
        composer_rect.y + text_inset,
        composer_rect.w - text_inset * 2.0,
        composer_rect.h - text_inset * 2.0,
        INPUT_FONT,
        INPUT_TEXT_RGBA if composer_text else INPUT_PLACEHOLDER_RGBA,
        node_id=node_ids.get("text"),
    )
    caret = build_composer_caret_node(
        "",
        0,
        focused=False,
        caret_visible=False,
        node_id=node_ids.get("caret"),
    )
    return root, [hit, text_node, caret]


def build_composer_text_node(
    composer_text: str = "",
    composer_placeholder: str = "type a reply — Enter to submit",
    *,
    placeholder_style: bool = False,
    node_id: Optional[bytes] = None,
) -> types_pb2.NodeProto:
    composer_rect = input_composer_local_rect()
    text_inset = 12.0
    content = composer_placeholder if placeholder_style else composer_text
    return make_text_node(
        content,
        composer_rect.x + text_inset,
        composer_rect.y + text_inset,
        composer_rect.w - text_inset * 2.0,
        composer_rect.h - text_inset * 2.0,
        INPUT_FONT,
        INPUT_PLACEHOLDER_RGBA if placeholder_style else INPUT_TEXT_RGBA,
        node_id=node_id,
        preserve_markdown=not placeholder_style,
    )


def build_composer_caret_node(
    composer_text: str,
    cursor: int,
    *,
    focused: bool,
    caret_visible: bool,
    node_id: Optional[bytes] = None,
) -> types_pb2.NodeProto:
    composer_rect = input_composer_local_rect()
    text_inset = 12.0
    cursor_x, line_index = composer_caret_layout(composer_text, cursor)
    caret_x = composer_rect.x + text_inset + min(
        COMPOSER_TEXT_RENDER_MARGIN_X + cursor_x,
        max(0.0, composer_rect.w - text_inset * 2.0 - COMPOSER_CARET_W),
    )
    caret_y = (
        composer_rect.y
        + text_inset
        + COMPOSER_TEXT_RENDER_MARGIN_Y
        + line_index * COMPOSER_LINE_PX
    )
    rgba = CARET_RGBA if focused and caret_visible else STATIC_CARET_RGBA
    return make_solid_color_node(
        *rgba,
        caret_x,
        caret_y,
        COMPOSER_CARET_W,
        COMPOSER_CARET_H,
        radius=COMPOSER_CARET_W / 2.0,
        node_id=node_id,
    )


def build_output_scroll_nodes(body: str) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    _, output_rect = portal_pane_rects()
    content_h = scroll_content_height_for_text(body, output_rect.h, SCROLL_LINE_PX)
    hit_h = content_h
    root = make_solid_color_node(*TEXT_WINDOW_BG_RGBA, 0.0, 0.0, output_rect.w, output_rect.h)
    hit = make_hit_region(SCROLL_INTERACTION_ID, 0.0, 0.0, output_rect.w, hit_h)
    body_node = make_text_node(
        body,
        0.0,
        0.0,
        output_rect.w,
        content_h,
        BODY_FONT,
        BODY_RGBA,
    )
    return root, [hit, body_node]


def visible_output_text(body: str, offset_y: float, viewport_h: float) -> tuple[str, int]:
    lines = body.splitlines()
    if not lines:
        return "", 0
    start = max(0, min(int(offset_y // SCROLL_LINE_PX), len(lines) - 1))
    visible_count = max(1, int(viewport_h // SCROLL_LINE_PX) + 2)
    end = min(len(lines), start + visible_count)
    return "\n".join(lines[start:end]), start


async def set_root_with_children(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    root: types_pb2.NodeProto,
    children: list[types_pb2.NodeProto],
    mutation_lock: Optional[asyncio.Lock] = None,
) -> tuple[bytes, list[bytes]]:
    if mutation_lock is not None:
        async with mutation_lock:
            return await set_root_with_children(client, lease_id, tile_id, root, children)

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
    child_ids: list[bytes] = []
    for child in children:
        child_ids.append(await client.add_node(lease_id, tile_id, child, parent_id=root_id))
    return root_id, child_ids


def update_node_content_mutation(
    tile_id: bytes,
    node_id: bytes,
    node: types_pb2.NodeProto,
) -> types_pb2.MutationProto:
    mutation = types_pb2.UpdateNodeContentMutation(tile_id=tile_id, node_id=node_id)
    if node.HasField("solid_color"):
        mutation.solid_color.CopyFrom(node.solid_color)
    elif node.HasField("text_markdown"):
        mutation.text_markdown.CopyFrom(node.text_markdown)
    elif node.HasField("hit_region"):
        mutation.hit_region.CopyFrom(node.hit_region)
    elif node.HasField("static_image"):
        mutation.static_image.CopyFrom(node.static_image)
    else:
        raise ValueError("node has no updateable content")
    return types_pb2.MutationProto(update_node_content=mutation)


async def set_frame_root_with_runtime_ids(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    title: str,
    subtitle: str,
    body: str,
    footer_meta: str,
    mutation_lock: Optional[asyncio.Lock] = None,
) -> None:
    frame_root, frame_children = build_portal_nodes(title, subtitle, body, footer_meta)

    async def mount() -> None:
        FRAME_RUNTIME_NODE_IDS.clear()
        frame_root_id, frame_child_ids = await set_root_with_children(
            client, lease_id, tile_id, frame_root, frame_children,
        )
        FRAME_RUNTIME_NODE_IDS["root"] = frame_root_id
        for key, node_id in zip(FRAME_CHILD_KEYS, frame_child_ids):
            FRAME_RUNTIME_NODE_IDS[key] = node_id

    if mutation_lock is not None:
        async with mutation_lock:
            await mount()
    else:
        await mount()


async def update_frame_chrome_live(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    title: str,
    subtitle: str,
    body: str,
    footer_meta: str,
    *,
    live_only: bool = True,
) -> None:
    if not FRAME_RUNTIME_NODE_IDS:
        return
    frame_root, frame_children = build_portal_nodes(title, subtitle, body, footer_meta)
    keyed_nodes = {"root": frame_root}
    keyed_nodes.update(zip(FRAME_CHILD_KEYS, frame_children))
    live_keys = {
        "root",
        "header_bg",
        "header_divider",
        "input_pane_bg",
        "composer_bg",
        "border_t",
        "border_b",
        "border_l",
        "border_r",
        "pane_divider",
        "pane_divider_grip",
        "pane_resize_hit",
        "output_pane_bg",
        "output_text_window_bg",
        "footer_divider",
        "footer_bg",
        "resize_handle",
        "resize_hit",
    }
    allowed_keys = live_keys if live_only else set(keyed_nodes.keys())
    mutations = [
        update_node_content_mutation(tile_id, node_id, keyed_nodes[key])
        for key, node_id in FRAME_RUNTIME_NODE_IDS.items()
        if key in keyed_nodes and key in allowed_keys
    ]
    if mutations:
        await client.submit_mutation_batch(lease_id, mutations, timeout=2.0)


async def set_input_root_with_runtime_ids(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    composer_text: str,
    mutation_lock: Optional[asyncio.Lock] = None,
) -> None:
    node_ids = fresh_composer_node_ids()
    input_root, input_children = build_input_scroll_nodes(
        composer_text,
        node_ids=node_ids,
    )

    async def mount() -> None:
        COMPOSER_RUNTIME_NODE_IDS.clear()
        _, input_child_ids = await set_root_with_children(
            client, lease_id, tile_id, input_root, input_children,
        )
        if len(input_child_ids) >= 3:
            COMPOSER_RUNTIME_NODE_IDS["hit"] = input_child_ids[0]
            COMPOSER_RUNTIME_NODE_IDS["text"] = input_child_ids[1]
            COMPOSER_RUNTIME_NODE_IDS["caret"] = input_child_ids[2]

    if mutation_lock is not None:
        async with mutation_lock:
            await mount()
    else:
        await mount()


async def render_composer_static(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    composer_text: str,
    cursor: int,
    *,
    focused: bool,
    caret_visible: bool,
    mutation_lock: Optional[asyncio.Lock] = None,
) -> tuple[str, float, int]:
    text_node_id = COMPOSER_RUNTIME_NODE_IDS.get("text")
    caret_node_id = COMPOSER_RUNTIME_NODE_IDS.get("caret")
    if text_node_id is None or caret_node_id is None:
        raise RuntimeError("composer nodes are not mounted")

    display_text, placeholder_style = composer_display_text(
        composer_text,
        cursor,
        focused=focused,
    )
    cursor_x, cursor_row = composer_caret_layout(composer_text, cursor)
    text_node = build_composer_text_node(
        display_text,
        placeholder_style=placeholder_style,
        node_id=text_node_id,
    )
    caret_node = build_composer_caret_node(
        composer_text,
        cursor,
        focused=focused,
        caret_visible=caret_visible,
        node_id=caret_node_id,
    )

    async def update() -> None:
        await client.update_node_content(lease_id, tile_id, text_node_id, text_node)
        await client.update_node_content(lease_id, tile_id, caret_node_id, caret_node)

    if mutation_lock is not None:
        async with mutation_lock:
            await update()
    else:
        await update()

    return display_text, cursor_x, cursor_row


async def update_input_scroll_geometry_live(
    client: HudClient,
    lease_id: bytes,
    tile_id: bytes,
    composer_text: str,
) -> None:
    if not COMPOSER_RUNTIME_NODE_IDS:
        return
    _, input_children = build_input_scroll_nodes(composer_text)
    child_keys = ["hit", "text", "caret"]
    mutations = [
        update_node_content_mutation(tile_id, node_id, node)
        for key, node in zip(child_keys, input_children)
        if (node_id := COMPOSER_RUNTIME_NODE_IDS.get(key)) is not None
    ]
    if mutations:
        await client.submit_mutation_batch(lease_id, mutations, timeout=2.0)


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
    output_scroll_content: Optional[str] = None,
    mutation_lock: Optional[asyncio.Lock] = None,
) -> None:
    """Publish the portal scene.

    Server rewrites the root id on set_tile_root, so set_tile_root is
    submitted alone first and the server-assigned id is used as parent_id
    for subsequent add_node calls. Batching set_tile_root + add_node fails
    under atomic-batch semantics.
    """
    if include_tile_setup:
        await client.update_tile_opacity(lease_id, tiles.capture_backstop, 0.0)
        await client.update_tile_input_mode(
            lease_id, tiles.capture_backstop, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
        )
        for tile_id in (tiles.frame, tiles.input_scroll, tiles.output_scroll):
            await client.update_tile_opacity(lease_id, tile_id, 1.0)
            await client.update_tile_input_mode(
                lease_id, tile_id, types_pb2.TILE_INPUT_MODE_CAPTURE,
            )
        await client.update_tile_input_mode(
            lease_id, tiles.drag_shield, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
        )
        await client.update_tile_opacity(lease_id, tiles.drag_shield, 0.0)
        await client.update_tile_opacity(lease_id, tiles.minimized_icon, 0.0)
        await client.update_tile_input_mode(
            lease_id, tiles.minimized_icon, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
        )
        input_rect, output_rect = portal_pane_rects()
        output_scroll_body = output_scroll_content if output_scroll_content is not None else body
        await client.submit_mutation_batch(
            lease_id,
            [
                register_tile_scroll_mutation(
                    tiles.input_scroll,
                    scrollable_y=True,
                    content_height=scroll_max_y_for_text(
                        composer_text, input_composer_local_rect().h, SCROLL_LINE_PX,
                    ),
                ),
                register_tile_scroll_mutation(
                    tiles.output_scroll,
                    scrollable_y=True,
                    content_height=scroll_max_y_for_text(
                        output_scroll_body, output_rect.h, SCROLL_LINE_PX,
                    ),
                ),
            ],
        )

    backstop_root, backstop_children = build_capture_backstop_nodes(
        PORTAL_W,
        PORTAL_H,
    )
    await set_root_with_children(
        client, lease_id, tiles.capture_backstop, backstop_root, backstop_children,
        mutation_lock,
    )

    await set_frame_root_with_runtime_ids(
        client, lease_id, tiles.frame, title, subtitle, body, footer_meta,
        mutation_lock,
    )

    should_mount_input = (
        include_tile_setup
        or bool(composer_text)
        or not COMPOSER_RUNTIME_NODE_IDS
    )
    if should_mount_input:
        await set_input_root_with_runtime_ids(
            client, lease_id, tiles.input_scroll, composer_text, mutation_lock,
        )

    output_root, output_children = build_output_scroll_nodes(body)
    await set_root_with_children(
        client, lease_id, tiles.output_scroll, output_root, output_children, mutation_lock,
    )

    if include_tile_setup:
        shield_root, shield_children = build_drag_shield_nodes(
            PORTAL_W,
            PORTAL_H,
            None,
        )
        await set_root_with_children(
            client, lease_id, tiles.drag_shield, shield_root, shield_children, mutation_lock,
        )
        icon_root, icon_children = build_minimized_icon_nodes(attention=False, pulse=False)
        await set_root_with_children(
            client, lease_id, tiles.minimized_icon, icon_root, icon_children, mutation_lock,
        )
    elif PORTAL_STATUS_STATE.get("minimized"):
        PORTAL_STATUS_STATE["attention"] = True
        PORTAL_STATUS_STATE["pulse"] = not PORTAL_STATUS_STATE.get("pulse", False)
        icon_root, icon_children = build_minimized_icon_nodes(
            attention=True,
            pulse=PORTAL_STATUS_STATE["pulse"],
        )
        await set_root_with_children(
            client, lease_id, tiles.minimized_icon, icon_root, icon_children, mutation_lock,
        )


async def create_portal_tiles(
    client: HudClient,
    lease_id: bytes,
    portal_x: float,
    portal_y: float,
    tab_width: float,
    tab_height: float,
) -> PortalTiles:
    input_rect, output_rect = portal_pane_rects()
    capture_backstop = await client.create_tile(
        lease_id,
        x=portal_x,
        y=portal_y,
        w=PORTAL_W,
        h=PORTAL_H,
        z_order=PORTAL_Z - 100,
    )
    frame = await client.create_tile(
        lease_id,
        x=portal_x,
        y=portal_y,
        w=PORTAL_W,
        h=PORTAL_H,
        z_order=PORTAL_Z,
    )
    input_scroll = await client.create_tile(
        lease_id,
        x=portal_x + input_rect.x,
        y=portal_y + input_rect.y,
        w=input_rect.w,
        h=input_rect.h,
        z_order=PORTAL_Z + 2,
    )
    output_scroll = await client.create_tile(
        lease_id,
        x=portal_x + output_rect.x,
        y=portal_y + output_rect.y,
        w=output_rect.w,
        h=output_rect.h,
        z_order=PORTAL_Z + 3,
    )
    drag_shield = await client.create_tile(
        lease_id,
        x=max(0.0, tab_width - 1.0),
        y=max(0.0, tab_height - 1.0),
        w=1.0,
        h=1.0,
        z_order=PORTAL_Z + 20,
    )
    minimized_icon = await client.create_tile(
        lease_id,
        x=portal_x,
        y=portal_y,
        w=MINIMIZED_ICON_SIZE,
        h=MINIMIZED_ICON_SIZE,
        z_order=PORTAL_Z + 10,
    )
    return PortalTiles(
        capture_backstop=capture_backstop,
        frame=frame,
        input_scroll=input_scroll,
        output_scroll=output_scroll,
        drag_shield=drag_shield,
        minimized_icon=minimized_icon,
        tab_width=tab_width,
        tab_height=tab_height,
    )


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


def publish_to_tile_bounds_mutation(
    tile_id: bytes,
    x: float,
    y: float,
    w: float,
    h: float,
) -> types_pb2.MutationProto:
    return types_pb2.MutationProto(
        publish_to_tile=types_pb2.PublishToTileMutation(
            element_id=tile_id,
            bounds=types_pb2.Rect(x=x, y=y, width=w, height=h),
        )
    )


def portal_bounds_mutations(tiles: PortalTiles, portal_x: float, portal_y: float) -> list[types_pb2.MutationProto]:
    input_rect, output_rect = portal_pane_rects()
    return [
        publish_to_tile_bounds_mutation(
            tiles.capture_backstop, portal_x, portal_y, PORTAL_W, PORTAL_H,
        ),
        publish_to_tile_bounds_mutation(
            tiles.frame, portal_x, portal_y, PORTAL_W, PORTAL_H,
        ),
        publish_to_tile_bounds_mutation(
            tiles.input_scroll,
            portal_x + input_rect.x,
            portal_y + input_rect.y,
            input_rect.w,
            input_rect.h,
        ),
        publish_to_tile_bounds_mutation(
            tiles.output_scroll,
            portal_x + output_rect.x,
            portal_y + output_rect.y,
            output_rect.w,
            output_rect.h,
        ),
        publish_to_tile_bounds_mutation(
            tiles.minimized_icon,
            portal_x,
            portal_y,
            MINIMIZED_ICON_SIZE,
            MINIMIZED_ICON_SIZE,
        ),
    ]


def portal_hidden_bounds_mutations(tiles: PortalTiles, portal_x: float, portal_y: float) -> list[types_pb2.MutationProto]:
    hidden_x = max(0.0, tiles.tab_width - 1.0)
    hidden_y = max(0.0, tiles.tab_height - 1.0)
    return [
        publish_to_tile_bounds_mutation(tiles.capture_backstop, hidden_x, hidden_y, 1.0, 1.0),
        publish_to_tile_bounds_mutation(tiles.frame, hidden_x, hidden_y, 1.0, 1.0),
        publish_to_tile_bounds_mutation(tiles.input_scroll, hidden_x, hidden_y, 1.0, 1.0),
        publish_to_tile_bounds_mutation(tiles.output_scroll, hidden_x, hidden_y, 1.0, 1.0),
        publish_to_tile_bounds_mutation(tiles.drag_shield, hidden_x, hidden_y, 1.0, 1.0),
        publish_to_tile_bounds_mutation(
            tiles.minimized_icon,
            portal_x,
            portal_y,
            MINIMIZED_ICON_SIZE,
            MINIMIZED_ICON_SIZE,
        ),
    ]


def build_capture_backstop_nodes(tab_width: float, tab_height: float) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    root = make_solid_color_node(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, tab_width, tab_height)
    hit = make_hit_region(
        "portal-capture-backstop",
        0.0, 0.0,
        tab_width,
        tab_height,
        accepts_focus=False,
    )
    return root, [hit]


def build_drag_shield_nodes(
    tab_width: float,
    tab_height: float,
    interaction_id: Optional[str],
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    root = make_solid_color_node(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, tab_width, tab_height)
    if not interaction_id:
        return root, []
    hit = make_hit_region(
        interaction_id,
        0.0, 0.0,
        tab_width,
        tab_height,
        accepts_focus=False,
    )
    return root, [hit]


def build_minimized_icon_nodes(
    *,
    attention: bool,
    pulse: bool,
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto]]:
    accent = (1.0, 0.82, 0.22, 0.98) if attention else (0.48, 0.86, 0.56, 0.94)
    halo_alpha = 0.28 if pulse else 0.12
    root = make_solid_color_node(
        0.02, 0.03, 0.05, 0.72,
        0.0, 0.0,
        MINIMIZED_ICON_SIZE,
        MINIMIZED_ICON_SIZE,
        radius=MINIMIZED_ICON_RADIUS,
    )
    halo = make_solid_color_node(
        accent[0], accent[1], accent[2], halo_alpha,
        4.0, 4.0,
        MINIMIZED_ICON_SIZE - 8.0,
        MINIMIZED_ICON_SIZE - 8.0,
        radius=MINIMIZED_ICON_RADIUS - 3.0,
    )
    core = make_solid_color_node(
        0.06, 0.08, 0.12, 0.94,
        10.0, 10.0,
        MINIMIZED_ICON_SIZE - 20.0,
        MINIMIZED_ICON_SIZE - 20.0,
        radius=12.0,
    )
    signal = make_solid_color_node(
        accent[0], accent[1], accent[2], accent[3],
        36.0, 13.0,
        8.0, 8.0,
        radius=4.0,
    )
    line_a = make_solid_color_node(
        0.82, 0.88, 0.96, 0.95,
        18.0, 23.0, 17.0, 2.0,
        radius=1.0,
    )
    line_b = make_solid_color_node(
        0.82, 0.88, 0.96, 0.70,
        18.0, 30.0, 22.0, 2.0,
        radius=1.0,
    )
    line_c = make_solid_color_node(
        accent[0], accent[1], accent[2], 0.90,
        18.0, 37.0, 13.0, 2.0,
        radius=1.0,
    )
    grip_a = make_solid_color_node(
        0.70, 0.78, 0.90, 0.66,
        43.0, 43.0, 7.0, 2.0,
        radius=1.0,
    )
    grip_b = make_solid_color_node(
        0.70, 0.78, 0.90, 0.66,
        43.0, 49.0, 7.0, 2.0,
        radius=1.0,
    )
    restore_hit_top = make_hit_region(
        PORTAL_RESTORE_INTERACTION_ID,
        0.0, 0.0,
        MINIMIZED_ICON_SIZE, 36.0,
        accepts_focus=False,
    )
    restore_hit_left = make_hit_region(
        PORTAL_RESTORE_INTERACTION_ID,
        0.0, 36.0,
        36.0, MINIMIZED_ICON_SIZE - 36.0,
        accepts_focus=False,
    )
    drag_hit = make_hit_region(
        PORTAL_ICON_DRAG_INTERACTION_ID,
        36.0, 36.0,
        MINIMIZED_ICON_SIZE - 36.0,
        MINIMIZED_ICON_SIZE - 36.0,
        accepts_focus=False,
    )
    return root, [
        halo, core, signal, line_a, line_b, line_c, grip_a, grip_b,
        restore_hit_top, restore_hit_left, drag_hit,
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


async def portal_interaction_loop(
    client: HudClient,
    lease_id: bytes,
    tiles: PortalTiles,
    transcript: list[dict[str, Any]],
    body_full: str,
    initial_portal_x: float,
    initial_portal_y: float,
    tab_width: float,
    tab_height: float,
    mutation_lock: asyncio.Lock,
    clipboard_host: str,
    clipboard_user: str,
    clipboard_ssh_key: str,
    clipboard_timeout_s: float,
) -> None:
    """Handle live pointer/keyboard input for manual exemplar review."""
    portal_x = initial_portal_x
    portal_y = initial_portal_y
    composer_text = ""
    composer_cursor = 0
    composer_cursor_goal_x: Optional[float] = None
    composer_focused = False
    _, output_rect = portal_pane_rects()
    output_view_start = 0
    drag: Optional[dict[str, float | str]] = None
    last_output_scroll_y: Optional[float] = None
    pending_key_echoes: list[dict[str, float | str]] = []
    suppressed_shortcut_chars: list[dict[str, float | str]] = []
    pending_paste_requests: list[dict[str, float | int]] = []
    next_paste_request_id = 0
    composer_render_task: Optional[asyncio.Task[None]] = None
    composer_render_dirty = False
    composer_last_dirty_at = 0.0
    composer_caret_visible = True
    composer_blink_task: Optional[asyncio.Task[None]] = None
    portal_minimized = False
    minimized_attention = False
    minimized_pulse = False
    last_drag_apply_at = 0.0

    async def render_minimized_icon() -> None:
        root, children = build_minimized_icon_nodes(
            attention=minimized_attention,
            pulse=minimized_pulse,
        )
        await set_root_with_children(
            client, lease_id, tiles.minimized_icon, root, children, mutation_lock,
        )

    async def set_portal_minimized(minimized: bool) -> None:
        nonlocal minimized_attention, minimized_pulse, portal_minimized, portal_x, portal_y
        if portal_minimized == minimized:
            return
        portal_minimized = minimized
        if minimized:
            minimized_pulse = minimized_attention
            set_composer_focus(False)
        else:
            minimized_attention = False
            minimized_pulse = False
            portal_x = max(0.0, min(portal_x, max(0.0, tab_width - PORTAL_W)))
            portal_y = max(0.0, min(portal_y, max(0.0, tab_height - PORTAL_H)))
        PORTAL_STATUS_STATE["minimized"] = minimized
        PORTAL_STATUS_STATE["attention"] = minimized_attention
        PORTAL_STATUS_STATE["pulse"] = minimized_pulse
        portal_opacity = 0.0 if minimized else 1.0
        portal_input_mode = (
            types_pb2.TILE_INPUT_MODE_PASSTHROUGH
            if minimized
            else types_pb2.TILE_INPUT_MODE_CAPTURE
        )
        icon_opacity = 1.0 if minimized else 0.0
        icon_input_mode = (
            types_pb2.TILE_INPUT_MODE_CAPTURE
            if minimized
            else types_pb2.TILE_INPUT_MODE_PASSTHROUGH
        )
        if minimized:
            async with mutation_lock:
                hidden_x = max(0.0, tiles.tab_width - 1.0)
                hidden_y = max(0.0, tiles.tab_height - 1.0)
                await client.submit_mutation_batch(
                    lease_id,
                    [
                        publish_to_tile_bounds_mutation(tiles.capture_backstop, hidden_x, hidden_y, 1.0, 1.0),
                        publish_to_tile_bounds_mutation(tiles.input_scroll, hidden_x, hidden_y, 1.0, 1.0),
                        publish_to_tile_bounds_mutation(tiles.output_scroll, hidden_x, hidden_y, 1.0, 1.0),
                        publish_to_tile_bounds_mutation(tiles.drag_shield, hidden_x, hidden_y, 1.0, 1.0),
                        publish_to_tile_bounds_mutation(tiles.minimized_icon, hidden_x, hidden_y, 1.0, 1.0),
                        publish_to_tile_bounds_mutation(
                            tiles.frame,
                            portal_x,
                            portal_y,
                            MINIMIZED_ICON_SIZE,
                            MINIMIZED_ICON_SIZE,
                        ),
                    ],
                    timeout=2.0,
                )
                await client.update_tile_opacity(lease_id, tiles.capture_backstop, 0.0)
                await client.update_tile_input_mode(
                    lease_id, tiles.capture_backstop, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
                )
                await client.update_tile_opacity(lease_id, tiles.input_scroll, 0.0)
                await client.update_tile_input_mode(
                    lease_id, tiles.input_scroll, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
                )
                await client.update_tile_opacity(lease_id, tiles.output_scroll, 0.0)
                await client.update_tile_input_mode(
                    lease_id, tiles.output_scroll, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
                )
                await client.update_tile_opacity(lease_id, tiles.frame, 1.0)
                await client.update_tile_input_mode(
                    lease_id, tiles.frame, types_pb2.TILE_INPUT_MODE_CAPTURE,
                )
                await client.update_tile_opacity(lease_id, tiles.minimized_icon, 0.0)
                await client.update_tile_input_mode(
                    lease_id, tiles.minimized_icon, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
                )
                icon_root, icon_children = build_minimized_icon_nodes(
                    attention=minimized_attention,
                    pulse=minimized_pulse,
                )
                await set_root_with_children(
                    client, lease_id, tiles.frame, icon_root, icon_children,
                )
            return

        visible_body, _ = visible_output_text(body_full, 0.0, output_rect.h)
        async with mutation_lock:
            await client.submit_mutation_batch(
                lease_id,
                portal_bounds_mutations(tiles, portal_x, portal_y),
                timeout=2.0,
            )
            await client.update_tile_opacity(lease_id, tiles.capture_backstop, 0.0)
            await client.update_tile_input_mode(
                lease_id, tiles.capture_backstop, types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
            )
            for tile_id in (tiles.frame, tiles.input_scroll, tiles.output_scroll):
                await client.update_tile_opacity(lease_id, tile_id, portal_opacity)
                await client.update_tile_input_mode(lease_id, tile_id, portal_input_mode)
            await client.update_tile_opacity(lease_id, tiles.minimized_icon, icon_opacity)
            await client.update_tile_input_mode(lease_id, tiles.minimized_icon, icon_input_mode)
            await set_frame_root_with_runtime_ids(
                client,
                lease_id,
                tiles.frame,
                "Exemplar Review Portal",
                "docs/exemplar-manual-review-checklist.md",
                visible_body,
                f"restored  •  input {INPUT_PANE_W:.0f}px / output {output_rect.w:.0f}px",
            )

    async def move_minimized_icon(new_x: float, new_y: float) -> None:
        nonlocal portal_x, portal_y
        portal_x = max(0.0, min(new_x, max(0.0, tab_width - MINIMIZED_ICON_SIZE)))
        portal_y = max(0.0, min(new_y, max(0.0, tab_height - MINIMIZED_ICON_SIZE)))
        async with mutation_lock:
            await client.submit_mutation_batch(
                lease_id,
                [
                    publish_to_tile_bounds_mutation(
                        tiles.frame,
                        portal_x,
                        portal_y,
                        MINIMIZED_ICON_SIZE,
                        MINIMIZED_ICON_SIZE,
                    )
                ],
                timeout=0.5,
            )

    async def move_portal(new_x: float, new_y: float) -> None:
        nonlocal portal_x, portal_y
        portal_x = max(0.0, min(new_x, max(0.0, tab_width - PORTAL_W)))
        portal_y = max(0.0, min(new_y, max(0.0, tab_height - PORTAL_H)))
        mutations = portal_bounds_mutations(tiles, portal_x, portal_y)
        if drag is not None:
            mutations.append(
                publish_to_tile_bounds_mutation(
                    tiles.drag_shield, portal_x, portal_y, PORTAL_W, PORTAL_H,
                )
            )
        async with mutation_lock:
            await client.submit_mutation_batch(
                lease_id,
                mutations,
                timeout=2.0,
            )

    async def set_drag_shield(interaction_id: Optional[str]) -> None:
        await client.update_tile_opacity(lease_id, tiles.drag_shield, 0.0)
        await client.submit_mutation_batch(
            lease_id,
            [
                publish_to_tile_bounds_mutation(
                    tiles.drag_shield,
                    portal_x if interaction_id else max(0.0, tab_width - 1.0),
                    portal_y if interaction_id else max(0.0, tab_height - 1.0),
                    PORTAL_W if interaction_id else 1.0,
                    PORTAL_H if interaction_id else 1.0,
                )
            ],
            timeout=2.0,
        )
        await client.update_tile_input_mode(
            lease_id,
            tiles.drag_shield,
            types_pb2.TILE_INPUT_MODE_CAPTURE if interaction_id else types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
        )
        root, children = build_drag_shield_nodes(PORTAL_W, PORTAL_H, interaction_id)
        await set_root_with_children(
            client, lease_id, tiles.drag_shield, root, children, mutation_lock,
        )

    async def clear_drag_shield() -> None:
        await set_drag_shield(None)

    async def apply_current_bounds() -> None:
        nonlocal portal_x, portal_y, output_rect
        portal_x = max(0.0, min(portal_x, max(0.0, tab_width - PORTAL_W)))
        portal_y = max(0.0, min(portal_y, max(0.0, tab_height - PORTAL_H)))
        _, output_rect = portal_pane_rects()
        mutations = portal_bounds_mutations(tiles, portal_x, portal_y)
        if drag is not None:
            mutations.append(
                publish_to_tile_bounds_mutation(
                    tiles.drag_shield, portal_x, portal_y, PORTAL_W, PORTAL_H,
                )
            )
        async with mutation_lock:
            await client.submit_mutation_batch(
                lease_id,
                mutations,
                timeout=2.0,
            )

    async def apply_drag_delta(dx: float, dy: float, *, rebuild: bool) -> None:
        nonlocal last_drag_apply_at, portal_x, portal_y
        if drag is None:
            return
        now = time.monotonic()
        drag_kind = str(drag.get("kind", "portal"))
        min_interval = (
            ICON_DRAG_APPLY_MIN_INTERVAL_SECONDS
            if drag_kind == "icon"
            else DRAG_APPLY_MIN_INTERVAL_SECONDS
        )
        if not rebuild and now - last_drag_apply_at < min_interval:
            drag["pending_dx"] = dx
            drag["pending_dy"] = dy
            return
        last_drag_apply_at = now
        drag.pop("pending_dx", None)
        drag.pop("pending_dy", None)
        if drag_kind == "portal":
            await move_portal(float(drag["portal_x"]) + dx, float(drag["portal_y"]) + dy)
            return
        if drag_kind == "icon":
            raw_moved = max(abs(dx), abs(dy))
            if not bool(drag.get("icon_dragging", False)):
                if raw_moved < ICON_DRAG_START_THRESHOLD_PX:
                    return
                drag["icon_dragging"] = True
            await move_minimized_icon(float(drag["portal_x"]) + dx, float(drag["portal_y"]) + dy)
            return
        if drag_kind == "pane":
            set_input_pane_width(float(drag["input_pane_w"]) + dx)
        elif drag_kind == "resize":
            set_portal_size(
                float(drag["portal_w"]) + dx,
                float(drag["portal_h"]) + dy,
                tab_width,
                tab_height,
            )
        if rebuild:
            await rebuild_resized_portal()
        else:
            await apply_current_bounds()

    async def rebuild_resized_portal() -> None:
        nonlocal output_rect, portal_x, portal_y
        portal_x = max(0.0, min(portal_x, max(0.0, tab_width - PORTAL_W)))
        portal_y = max(0.0, min(portal_y, max(0.0, tab_height - PORTAL_H)))
        _, output_rect = portal_pane_rects()
        visible_body, _ = visible_output_text(body_full, 0.0, output_rect.h)
        async with mutation_lock:
            await client.submit_mutation_batch(
                lease_id,
                [
                    *portal_bounds_mutations(tiles, portal_x, portal_y),
                    register_tile_scroll_mutation(
                        tiles.input_scroll,
                        scrollable_y=True,
                        content_height=scroll_max_y_for_text(
                            composer_text,
                            input_composer_local_rect().h,
                            SCROLL_LINE_PX,
                        ),
                    ),
                    register_tile_scroll_mutation(
                        tiles.output_scroll,
                        scrollable_y=True,
                        content_height=scroll_max_y_for_text(
                            body_full,
                            output_rect.h,
                            SCROLL_LINE_PX,
                        ),
                    ),
                ],
                timeout=2.0,
            )
            await update_frame_chrome_live(
                client,
                lease_id,
                tiles.frame,
                "Exemplar Review Portal",
                "docs/exemplar-manual-review-checklist.md",
                visible_body,
                f"resized  •  input {INPUT_PANE_W:.0f}px / output {output_rect.w:.0f}px",
                live_only=False,
            )
            await update_input_scroll_geometry_live(
                client, lease_id, tiles.input_scroll, composer_text,
            )
        request_composer_render()

    async def render_composer_once() -> None:
        text_node_id = COMPOSER_RUNTIME_NODE_IDS.get("text")
        caret_node_id = COMPOSER_RUNTIME_NODE_IDS.get("caret")
        if text_node_id is None or caret_node_id is None:
            print("  [grpc] Composer render skipped; input nodes not mounted yet.", flush=True)
            return
        display_text, placeholder_style = composer_display_text(
            composer_text,
            composer_cursor,
            focused=composer_focused,
        )
        text_node = build_composer_text_node(
            display_text,
            placeholder_style=placeholder_style,
            node_id=text_node_id,
        )
        caret_node = build_composer_caret_node(
            composer_text,
            composer_cursor,
            focused=composer_focused,
            caret_visible=composer_caret_visible,
            node_id=caret_node_id,
        )
        try:
            if mutation_lock is not None:
                async with mutation_lock:
                    await client.update_node_content(
                        lease_id, tiles.input_scroll, text_node_id, text_node,
                    )
                    await client.update_node_content(
                        lease_id, tiles.input_scroll, caret_node_id, caret_node,
                    )
            else:
                await client.update_node_content(
                    lease_id, tiles.input_scroll, text_node_id, text_node,
                )
                await client.update_node_content(
                    lease_id, tiles.input_scroll, caret_node_id, caret_node,
                )
        except RuntimeError as exc:
            if "node not found" in str(exc).lower():
                print("  [grpc] Composer render skipped; stale input nodes during resize.", flush=True)
                return
            raise
        print(
            "  [grpc] Composer text/caret updated: "
            f"{text_node_id.hex()[:16]}.../{caret_node_id.hex()[:16]}...",
            flush=True,
        )

    async def composer_render_worker() -> None:
        nonlocal composer_render_dirty, composer_render_task
        try:
            while True:
                composer_render_dirty = False
                while True:
                    quiet_for = time.monotonic() - composer_last_dirty_at
                    remaining = COMPOSER_RENDER_DEBOUNCE_SECONDS - quiet_for
                    if remaining <= 0.0:
                        break
                    await asyncio.sleep(remaining)
                await render_composer_once()
                if not composer_render_dirty:
                    break
        finally:
            composer_render_task = None

    def request_composer_render() -> None:
        nonlocal composer_render_dirty, composer_render_task, composer_last_dirty_at
        composer_render_dirty = True
        composer_last_dirty_at = time.monotonic()
        if composer_render_task is None or composer_render_task.done():
            composer_render_task = asyncio.create_task(composer_render_worker())

    async def composer_blink_worker() -> None:
        nonlocal composer_blink_task, composer_caret_visible
        try:
            while composer_focused:
                await asyncio.sleep(COMPOSER_CARET_BLINK_SECONDS)
                if not composer_focused:
                    break
                composer_caret_visible = not composer_caret_visible
                request_composer_render()
        finally:
            composer_blink_task = None

    def set_composer_focus(focused: bool) -> None:
        nonlocal composer_blink_task, composer_caret_visible, composer_focused
        if composer_focused == focused:
            return
        composer_focused = focused
        composer_caret_visible = True
        request_composer_render()
        if focused and (composer_blink_task is None or composer_blink_task.done()):
            composer_blink_task = asyncio.create_task(composer_blink_worker())

    async def render_output_scroll(offset_y: float) -> None:
        nonlocal output_view_start
        visible_body, output_view_start = visible_output_text(body_full, offset_y, output_rect.h)
        output_root, output_children = build_output_scroll_nodes(visible_body)
        await set_root_with_children(
            client, lease_id, tiles.output_scroll, output_root, output_children,
            mutation_lock,
        )

    async def finish_drag(reason: str, display_x: Optional[float] = None, display_y: Optional[float] = None) -> None:
        nonlocal drag
        if drag is None:
            return
        drag_kind = str(drag.get("kind", "portal"))
        if display_x is None or display_y is None:
            pending_dx = drag.get("pending_dx")
            pending_dy = drag.get("pending_dy")
            if pending_dx is not None and pending_dy is not None:
                await apply_drag_delta(float(pending_dx), float(pending_dy), rebuild=False)
        if display_x is not None and display_y is not None:
            dx = display_x - float(drag["start_x"])
            dy = display_y - float(drag["start_y"])
            await apply_drag_delta(dx, dy, rebuild=False)
        if drag_kind == "icon":
            icon_dragging = bool(drag.get("icon_dragging", False))
            moved = max(
                abs(portal_x - float(drag["portal_x"])),
                abs(portal_y - float(drag["portal_y"])),
            )
            if display_x is not None and display_y is not None:
                moved = max(moved,
                    abs(display_x - float(drag["start_x"])),
                    abs(display_y - float(drag["start_y"])),
                )
            drag = None
            if not icon_dragging and moved < ICON_DRAG_START_THRESHOLD_PX:
                emit_step_event(transcript, 9, "checkpoint", {
                    "code": "portal-icon:drag-cancel",
                    "title": "Minimized icon drag cancelled",
                    "action": "icon drag grip was pressed without crossing the drag threshold",
                    "expected_visual": "icon remains minimized; main icon body still restores immediately",
                }, portal_x=portal_x, portal_y=portal_y,
                   portal_w=PORTAL_W, portal_h=PORTAL_H)
            else:
                emit_step_event(transcript, 9, "checkpoint", {
                    "code": "portal-icon:drag-end",
                    "title": "Minimized icon drag ended",
                    "action": "floating text-stream icon stayed minimized at its new anchor",
                    "expected_visual": "icon remains clickable at the dragged position",
                }, portal_x=portal_x, portal_y=portal_y, reason=reason)
            return
        drag = None
        await clear_drag_shield()
        if drag_kind in {"pane", "resize"}:
            await rebuild_resized_portal()
        code = {
            "portal": "drag:end",
            "pane": "pane-resize:end",
            "resize": "portal-resize:end",
        }.get(drag_kind, "drag:end")
        title = {
            "portal": "Portal drag ended",
            "pane": "Pane resize ended",
            "resize": "Portal resize ended",
        }.get(drag_kind, "Portal drag ended")
        emit_step_event(transcript, 9, "checkpoint", {
            "code": code,
            "title": title,
            "action": "all portal tiles committed to grouped position",
            "expected_visual": "input/output panes remain aligned with portal frame",
        }, portal_x=portal_x, portal_y=portal_y,
           portal_w=PORTAL_W, portal_h=PORTAL_H,
           input_pane_w=INPUT_PANE_W, output_pane_w=portal_pane_rects()[1].w,
           reason=reason)

    async def submit_composer() -> None:
        nonlocal composer_cursor, composer_cursor_goal_x, composer_text
        if composer_text.strip():
            emit_step_event(transcript, 10, "checkpoint", {
                "code": "input:submit",
                "title": "Composer submitted",
                "action": "operator submitted text from portal composer",
                "expected_visual": "composer clears after submit",
        }, submitted=composer_text)
        composer_text = ""
        composer_cursor = 0
        composer_cursor_goal_x = None
        request_composer_render()

    def prune_pending_key_echoes() -> None:
        now = time.monotonic()
        pending_key_echoes[:] = [
            pending for pending in pending_key_echoes
            if now - float(pending["at"]) < KEY_ECHO_TIMEOUT_SECONDS
        ]
        suppressed_shortcut_chars[:] = [
            pending for pending in suppressed_shortcut_chars
            if now - float(pending["at"]) < KEY_ECHO_TIMEOUT_SECONDS
        ]
        pending_paste_requests[:] = [
            pending for pending in pending_paste_requests
            if now - float(pending["at"]) < KEY_ECHO_TIMEOUT_SECONDS
        ]

    def consume_key_echo(character: str) -> bool:
        prune_pending_key_echoes()
        for index, pending in enumerate(pending_key_echoes):
            if character == pending["text"]:
                del pending_key_echoes[index]
                return True
        return False

    def suppress_shortcut_character(character: str) -> None:
        if character:
            suppressed_shortcut_chars.append({
                "text": character,
                "at": time.monotonic(),
            })

    def consume_suppressed_shortcut_character(character: str) -> bool:
        prune_pending_key_echoes()
        for index, pending in enumerate(suppressed_shortcut_chars):
            if character == pending["text"]:
                del suppressed_shortcut_chars[index]
                return True
        return False

    def consume_pending_paste_request() -> bool:
        prune_pending_key_echoes()
        if pending_paste_requests:
            pending_paste_requests.pop(0)
            return True
        return False

    async def fallback_paste_request(request_id: int) -> None:
        await asyncio.sleep(0.18)
        for index, pending in enumerate(pending_paste_requests):
            if int(pending["id"]) == request_id:
                del pending_paste_requests[index]
                pasted = await paste_windows_clipboard()
                if pasted:
                    pending_key_echoes.append({
                        "text": pasted,
                        "at": time.monotonic(),
                    })
                return

    async def paste_windows_clipboard() -> str:
        nonlocal composer_cursor, composer_cursor_goal_x, composer_text
        pasted = await read_windows_clipboard(
            clipboard_host,
            user=clipboard_user,
            ssh_key=clipboard_ssh_key,
            timeout_s=clipboard_timeout_s,
        )
        pasted = normalize_composer_input(pasted)
        if not pasted:
            emit_step_event(transcript, 10, "checkpoint", {
                "code": "input:paste-empty",
                "title": "Composer paste empty",
                "action": "clipboard read returned no text",
                "expected_visual": "composer text remains unchanged",
            })
            return ""
        composer_text = (
            composer_text[:composer_cursor]
            + pasted
            + composer_text[composer_cursor:]
        )
        composer_cursor += len(pasted)
        composer_cursor_goal_x = None
        emit_step_event(transcript, 10, "checkpoint", {
            "code": "input:paste",
            "title": "Composer pasted clipboard",
            "action": "inserted Windows clipboard text at cursor",
            "expected_visual": "clipboard text appears once in composer",
        }, chars=len(pasted), lines=len(pasted.splitlines()))
        request_composer_render()
        return pasted

    while True:
        try:
            timeouts: list[float] = []
            if drag is not None:
                last_activity_at = float(drag.get("last_activity_at", drag["started_at"]))
                timeouts.append(
                    max(0.0, DRAG_IDLE_RELEASE_SECONDS - (time.monotonic() - last_activity_at))
                )
            timeout = min(timeouts) if timeouts else None
            batch = await asyncio.wait_for(client._event_queue.get(), timeout=timeout)
        except asyncio.TimeoutError:
            prune_pending_key_echoes()
            if drag is not None:
                last_activity_at = float(drag.get("last_activity_at", drag["started_at"]))
                if time.monotonic() - last_activity_at >= DRAG_IDLE_RELEASE_SECONDS:
                    await finish_drag("idle_release")
            continue
        pending_output_scroll_y: Optional[float] = None
        for envelope in batch.events:
            kind = envelope.WhichOneof("event")
            prune_pending_key_echoes()

            if kind == "pointer_down":
                ev = envelope.pointer_down
                if ev.interaction_id == PORTAL_MINIMIZE_INTERACTION_ID:
                    if drag is not None:
                        await finish_drag("superseded:minimize")
                    await set_portal_minimized(True)
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "portal:minimize",
                        "title": "Portal minimized",
                        "action": "top-left minimize control collapsed the portal to a floating status icon",
                        "expected_visual": "full portal hides; compact text-stream icon remains clickable",
                    }, portal_x=portal_x, portal_y=portal_y)
                elif ev.interaction_id == PORTAL_RESTORE_INTERACTION_ID:
                    if not portal_minimized:
                        emit_step_event(transcript, 9, "checkpoint", {
                            "code": "portal:restore-ignored",
                            "title": "Restore ignored while portal is expanded",
                            "action": "stale minimized-icon event arrived after restore",
                            "expected_visual": "expanded portal remains visible",
                        }, portal_x=portal_x, portal_y=portal_y)
                        continue
                    if drag is not None and str(drag.get("kind", "")) == "icon":
                        await finish_drag("superseded:restore-click")
                    await set_portal_minimized(False)
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "portal:restore",
                        "title": "Portal restored",
                        "action": "floating text-stream icon body restored the full portal immediately",
                        "expected_visual": "portal reappears at the icon anchor",
                    }, portal_x=portal_x, portal_y=portal_y,
                       portal_w=PORTAL_W, portal_h=PORTAL_H)
                elif ev.interaction_id == PORTAL_ICON_DRAG_INTERACTION_ID:
                    if not portal_minimized:
                        continue
                    if drag is not None:
                        if str(drag.get("kind", "")) == "icon":
                            emit_step_event(transcript, 9, "checkpoint", {
                                "code": "portal-icon:pointer-down-ignored",
                                "title": "Duplicate icon drag pointer down ignored",
                                "action": "minimized icon drag grip already has an armed gesture",
                                "expected_visual": "icon remains ready to move while dragging",
                            }, portal_x=portal_x, portal_y=portal_y)
                            continue
                        await finish_drag("superseded:restore")
                    drag = {
                        "kind": "icon",
                        "device_id": ev.device_id,
                        "start_x": ev.display_x,
                        "start_y": ev.display_y,
                        "portal_x": portal_x,
                        "portal_y": portal_y,
                        "icon_dragging": False,
                        "started_at": time.monotonic(),
                        "last_activity_at": time.monotonic(),
                    }
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "portal-icon:pointer-down",
                        "title": "Minimized icon drag grip pointer down",
                        "action": "floating text-stream icon drag grip is ready to move the icon",
                        "expected_visual": "icon moves only after a deliberate drag from the grip",
                    }, portal_x=portal_x, portal_y=portal_y,
                       portal_w=PORTAL_W, portal_h=PORTAL_H)
                elif ev.interaction_id == PORTAL_DRAG_INTERACTION_ID:
                    if drag is not None:
                        await finish_drag("superseded:pointer_down")
                    drag = {
                        "kind": "portal",
                        "device_id": ev.device_id,
                        "start_x": ev.display_x,
                        "start_y": ev.display_y,
                        "portal_x": portal_x,
                        "portal_y": portal_y,
                        "started_at": time.monotonic(),
                        "last_activity_at": time.monotonic(),
                    }
                    await set_drag_shield(PORTAL_DRAG_INTERACTION_ID)
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "drag:start",
                        "title": "Portal drag started",
                        "action": "header drag surface received pointer down",
                        "expected_visual": "portal follows pointer while dragging",
                    }, display_x=ev.display_x, display_y=ev.display_y)
                elif ev.interaction_id == PANE_RESIZE_INTERACTION_ID:
                    if drag is not None:
                        await finish_drag("superseded:pane_resize")
                    drag = {
                        "kind": "pane",
                        "device_id": ev.device_id,
                        "start_x": ev.display_x,
                        "start_y": ev.display_y,
                        "input_pane_w": INPUT_PANE_W,
                        "started_at": time.monotonic(),
                        "last_activity_at": time.monotonic(),
                    }
                    await set_drag_shield(PANE_RESIZE_INTERACTION_ID)
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "pane-resize:start",
                        "title": "Pane resize started",
                        "action": "middle divider received pointer down",
                        "expected_visual": "dragging changes input/output width ratio",
                    }, display_x=ev.display_x, display_y=ev.display_y,
                       input_pane_w=INPUT_PANE_W)
                elif ev.interaction_id == PORTAL_RESIZE_INTERACTION_ID:
                    if drag is not None:
                        await finish_drag("superseded:portal_resize")
                    drag = {
                        "kind": "resize",
                        "device_id": ev.device_id,
                        "start_x": ev.display_x,
                        "start_y": ev.display_y,
                        "portal_w": PORTAL_W,
                        "portal_h": PORTAL_H,
                        "started_at": time.monotonic(),
                        "last_activity_at": time.monotonic(),
                    }
                    await set_drag_shield(PORTAL_RESIZE_INTERACTION_ID)
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "portal-resize:start",
                        "title": "Portal resize started",
                        "action": "bottom-right resize handle received pointer down",
                        "expected_visual": "dragging resizes the whole portal surface",
                    }, display_x=ev.display_x, display_y=ev.display_y,
                       portal_w=PORTAL_W, portal_h=PORTAL_H)
                elif ev.interaction_id == COMPOSER_INTERACTION_ID:
                    if drag is not None:
                        await finish_drag("superseded:composer_focus")
                    emit_step_event(transcript, 10, "checkpoint", {
                        "code": "input:focus-attempt",
                        "title": "Composer pointer down",
                        "action": "input composer received pointer down",
                        "expected_visual": "keyboard focus should move to composer",
                    }, display_x=ev.display_x, display_y=ev.display_y)
                else:
                    emit_step_event(transcript, 9, "checkpoint", {
                        "code": "input:pointer-down-unhandled",
                        "title": "Unhandled portal pointer down",
                        "action": "runtime delivered pointer down for an interaction id without exemplar handling",
                        "expected_visual": "operator click may appear to do nothing",
                    }, interaction_id=ev.interaction_id,
                       display_x=ev.display_x, display_y=ev.display_y)

            elif kind == "pointer_move" and drag is not None:
                ev = envelope.pointer_move
                if ev.device_id != drag["device_id"]:
                    continue
                now = time.monotonic()
                if now - float(drag["started_at"]) > DRAG_MAX_SECONDS:
                    await finish_drag("watchdog")
                    continue
                drag["last_activity_at"] = now
                dx = ev.display_x - float(drag["start_x"])
                dy = ev.display_y - float(drag["start_y"])
                await apply_drag_delta(dx, dy, rebuild=False)

            elif kind == "pointer_up" and drag is not None:
                ev = envelope.pointer_up
                if ev.device_id == drag["device_id"]:
                    await finish_drag("pointer_up", ev.display_x, ev.display_y)

            elif kind == "pointer_cancel" and drag is not None:
                ev = envelope.pointer_cancel
                if ev.device_id == drag["device_id"]:
                    await finish_drag("pointer_cancel")

            elif kind == "capture_released" and drag is not None:
                ev = envelope.capture_released
                if ev.device_id == drag["device_id"]:
                    reason_name = events_pb2.CaptureReleasedReason.Name(ev.reason)
                    await finish_drag(f"capture_released:{reason_name}")

            elif kind == "character":
                ev = envelope.character
                if ev.tile_id != tiles.input_scroll:
                    continue
                character = normalize_composer_input(ev.character)
                suppressed_shortcut = consume_suppressed_shortcut_character(character)
                consumed_echo = consume_key_echo(character)
                from_paste_request = consume_pending_paste_request()
                emit_step_event(transcript, 10, "checkpoint", {
                    "code": "input:character",
                    "title": "Composer character received",
                    "action": "runtime delivered character input to composer",
                    "expected_visual": "typed character appears in composer text window",
                }, character=character, consumed_echo=consumed_echo,
                   suppressed_shortcut=suppressed_shortcut,
                   from_paste_request=from_paste_request)
                if character in {"\r", "\n"}:
                    continue
                if suppressed_shortcut:
                    continue
                if consumed_echo:
                    continue
                composer_cursor_goal_x = None
                composer_text = (
                    composer_text[:composer_cursor]
                    + character
                    + composer_text[composer_cursor:]
                )
                composer_cursor += len(character)
                request_composer_render()

            elif kind == "key_down":
                ev = envelope.key_down
                if ev.tile_id != tiles.input_scroll:
                    continue
                emit_step_event(transcript, 10, "checkpoint", {
                    "code": "input:key-down",
                    "title": "Composer key down received",
                    "action": "runtime delivered key input to composer",
                    "expected_visual": "editing commands affect composer when applicable",
                }, key=ev.key, key_code=ev.key_code, repeat=ev.repeat,
                   ctrl=ev.ctrl, shift=ev.shift, alt=ev.alt, meta=ev.meta)
                if ev.key == "Backspace" and (ev.ctrl or ev.alt):
                    composer_cursor_goal_x = None
                    pending_key_echoes.clear()
                    word_start = composer_word_delete_start(composer_text, composer_cursor)
                    if word_start < composer_cursor:
                        composer_text = (
                            composer_text[:word_start]
                            + composer_text[composer_cursor:]
                        )
                        composer_cursor = word_start
                    request_composer_render()
                elif ev.key == "Backspace":
                    composer_cursor_goal_x = None
                    pending_key_echoes.clear()
                    if composer_cursor > 0:
                        composer_text = (
                            composer_text[:composer_cursor - 1]
                            + composer_text[composer_cursor:]
                        )
                        composer_cursor -= 1
                    request_composer_render()
                elif ev.key == "Delete":
                    composer_cursor_goal_x = None
                    pending_key_echoes.clear()
                    if composer_cursor < len(composer_text):
                        composer_text = (
                            composer_text[:composer_cursor]
                            + composer_text[composer_cursor + 1:]
                        )
                    request_composer_render()
                elif ev.key == "Home":
                    composer_cursor_goal_x = None
                    composer_cursor = 0
                    request_composer_render()
                elif ev.key == "End":
                    composer_cursor_goal_x = None
                    composer_cursor = len(composer_text)
                    request_composer_render()
                elif ev.key == "ArrowLeft" and (ev.ctrl or ev.alt):
                    composer_cursor_goal_x = None
                    composer_cursor = composer_word_delete_start(composer_text, composer_cursor)
                    request_composer_render()
                elif ev.key == "ArrowLeft":
                    composer_cursor_goal_x = None
                    composer_cursor = max(0, composer_cursor - 1)
                    request_composer_render()
                elif ev.key == "ArrowRight" and (ev.ctrl or ev.alt):
                    composer_cursor_goal_x = None
                    composer_cursor = composer_word_forward_end(composer_text, composer_cursor)
                    request_composer_render()
                elif ev.key == "ArrowRight":
                    composer_cursor_goal_x = None
                    composer_cursor = min(len(composer_text), composer_cursor + 1)
                    request_composer_render()
                elif ev.key == "ArrowUp":
                    composer_cursor, composer_cursor_goal_x = composer_cursor_for_vertical_move(
                        composer_text,
                        composer_cursor,
                        -1,
                        composer_cursor_goal_x,
                    )
                    request_composer_render()
                elif ev.key == "ArrowDown":
                    composer_cursor, composer_cursor_goal_x = composer_cursor_for_vertical_move(
                        composer_text,
                        composer_cursor,
                        1,
                        composer_cursor_goal_x,
                    )
                    request_composer_render()
                elif ev.key == "Enter":
                    composer_cursor_goal_x = None
                    pending_key_echoes.clear()
                    await submit_composer()
                elif ev.key == "Escape":
                    composer_cursor_goal_x = None
                    pending_key_echoes.clear()
                    composer_text = ""
                    composer_cursor = 0
                    request_composer_render()
                elif ev.ctrl or ev.meta:
                    composer_cursor_goal_x = None
                    pending_key_echoes.clear()
                    if ev.key.lower() == "v":
                        next_paste_request_id += 1
                        paste_request_id = next_paste_request_id
                        pending_paste_requests.append({
                            "id": paste_request_id,
                            "at": time.monotonic(),
                        })
                        asyncio.create_task(fallback_paste_request(paste_request_id))
                        emit_step_event(transcript, 10, "checkpoint", {
                            "code": "input:paste-requested",
                            "title": "Composer paste requested",
                            "action": "waiting for runtime clipboard character payload; SSH clipboard fallback is armed",
                            "expected_visual": "clipboard text appears once in composer",
                        })
                    elif len(ev.key) == 1:
                        suppress_shortcut_character(ev.key)
                    emit_step_event(transcript, 10, "checkpoint", {
                        "code": "input:shortcut-ignored",
                        "title": "Composer shortcut ignored",
                        "action": "runtime delivered a modified key that this exemplar does not implement",
                        "expected_visual": "shortcut key does not insert literal text",
                    }, key=ev.key, key_code=ev.key_code, ctrl=ev.ctrl, meta=ev.meta)
                else:
                    fallback_text = composer_key_fallback_text(ev.key)
                    if fallback_text is not None:
                        composer_cursor_goal_x = None
                        composer_text = (
                            composer_text[:composer_cursor]
                            + fallback_text
                            + composer_text[composer_cursor:]
                        )
                        composer_cursor += len(fallback_text)
                        pending_key_echoes.append({
                            "text": fallback_text,
                            "at": time.monotonic(),
                        })
                        emit_step_event(transcript, 10, "checkpoint", {
                            "code": "input:key-text",
                            "title": "Composer key text applied",
                            "action": "printable key-down updated composer in-order",
                            "expected_visual": "typed character appears once in composer text window",
                        }, text=fallback_text)
                        request_composer_render()

            elif kind == "scroll_offset_changed":
                ev = envelope.scroll_offset_changed
                if ev.tile_id != tiles.output_scroll:
                    continue
                if abs(ev.offset_y) < 0.5:
                    continue
                pending_output_scroll_y = ev.offset_y

            elif kind == "focus_gained":
                ev = envelope.focus_gained
                if ev.tile_id != tiles.input_scroll:
                    continue
                emit_step_event(transcript, 10, "checkpoint", {
                    "code": "input:focus-gained",
                    "title": "Composer focus gained",
                    "action": "runtime focus manager focused the composer hit region",
                    "expected_visual": "subsequent keyboard events route to composer",
                })
                set_composer_focus(True)

            elif kind == "focus_lost":
                ev = envelope.focus_lost
                if ev.tile_id != tiles.input_scroll:
                    continue
                emit_step_event(transcript, 10, "checkpoint", {
                    "code": "input:focus-lost",
                    "title": "Composer focus lost",
                    "action": "runtime focus manager moved focus away from composer",
                    "expected_visual": "composer stops receiving keyboard events",
                })
                set_composer_focus(False)

        prune_pending_key_echoes()

        if pending_output_scroll_y is not None:
            if last_output_scroll_y is not None and abs(pending_output_scroll_y - last_output_scroll_y) < 0.5:
                continue
            last_output_scroll_y = pending_output_scroll_y
            await render_output_scroll(pending_output_scroll_y)
            emit_step_event(transcript, 8, "checkpoint", {
                "code": "scroll:output",
                "title": "Output transcript scrolled",
                "action": "portal received local-first scroll offset",
                "expected_visual": "output text stays clipped inside transcript box",
            }, scroll_y=pending_output_scroll_y, viewport_start=output_view_start)


async def run_baseline(
    client: HudClient, lease_id: bytes, tiles: PortalTiles,
    body_full: str, transcript: list[dict[str, Any]], hold_s: float,
    mutation_lock: asyncio.Lock,
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
        mutation_lock=mutation_lock,
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
    mutation_lock: asyncio.Lock,
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
        output_scroll_content="\n".join(history),
        mutation_lock=mutation_lock,
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
            mutation_lock=mutation_lock,
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
            mutation_lock=mutation_lock,
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
        mutation_lock=mutation_lock,
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
    mutation_lock: asyncio.Lock,
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
            mutation_lock=mutation_lock,
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
    mutation_lock: asyncio.Lock,
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
            mutation_lock=mutation_lock,
        )
        await asyncio.sleep(interval_ms / 1000.0)
    await publish_portal(
        client, lease_id, tiles,
        title="Exemplar Review Portal",
        subtitle="docs/exemplar-manual-review-checklist.md",
        body=body_full,
        footer_meta=f"rapid-settled  •  lines 1-{len(lines)}",
        include_tile_setup=False,
        mutation_lock=mutation_lock,
    )
    emit_step_event(transcript, 3, "completed", {
        "code": "rapid",
        "title": "Rapid replace (coalescing smoke)",
        "action": "settled on full body",
        "expected_visual": "full body visible, no tearing",
    })


async def run_composer_smoke(
    client: HudClient, lease_id: bytes, tiles: PortalTiles,
    transcript: list[dict[str, Any]],
    mutation_lock: asyncio.Lock,
    hold_s: float,
) -> None:
    """Render deterministic composer states for live caret/input review."""
    emit_step_event(transcript, 5, "started", {
        "code": "composer-smoke",
        "title": "Composer caret smoke",
        "action": "render hello-world and long-paste composer states with visible caret",
        "expected_visual": "caret aligns with text after Space and wrapped markdown paste",
    })
    await publish_portal(
        client, lease_id, tiles,
        title="Exemplar Review Portal",
        subtitle="Composer caret and Space smoke",
        body="INPUT pane is under review. OUTPUT pane remains bounded.",
        footer_meta="composer-smoke  •  deterministic live render",
        include_tile_setup=True,
        mutation_lock=mutation_lock,
    )

    hello = "hello world"
    display_text, cursor_x, cursor_row = await render_composer_static(
        client,
        lease_id,
        tiles.input_scroll,
        hello,
        len(hello),
        focused=True,
        caret_visible=True,
        mutation_lock=mutation_lock,
    )
    emit_step_event(transcript, 5, "checkpoint", {
        "code": "composer:hello-world",
        "title": "Composer hello world rendered",
        "action": "rendered normal typing target with Space between words",
        "expected_visual": "hello world appears once, caret after the final d",
    }, cursor_x=cursor_x, cursor_row=cursor_row, visual_lines=len(display_text.splitlines()))
    await asyncio.sleep(min(hold_s, 3.0))

    paste = (
        "**Long markdown paste** near the right edge of the composer line with "
        "words, punctuation, and enough text to wrap several visual rows without "
        "leaving a blank-looking trailing-space caret offset."
    )
    display_text, cursor_x, cursor_row = await render_composer_static(
        client,
        lease_id,
        tiles.input_scroll,
        paste,
        len(paste),
        focused=True,
        caret_visible=True,
        mutation_lock=mutation_lock,
    )
    visual_lines = display_text.splitlines()
    trailing_ws_lines = [
        index for index, line in enumerate(visual_lines[:-1])
        if line.endswith((" ", "\t"))
    ]
    emit_step_event(transcript, 5, "completed", {
        "code": "composer:long-paste",
        "title": "Composer long paste rendered",
        "action": "rendered wrapped markdown-like paste with caret at end",
        "expected_visual": "caret remains on the final wrapped row without a one-character lag at line ends",
    }, cursor_x=cursor_x, cursor_row=cursor_row,
       visual_lines=len(visual_lines), trailing_ws_lines=trailing_ws_lines)
    await asyncio.sleep(hold_s)


async def run_diagnostic_input_phase(
    transcript: list[dict[str, Any]],
    *,
    host: str,
    user: str,
    ssh_key: str,
    portal_x: float,
    portal_y: float,
    timeout_s: float,
) -> None:
    """Drive focus, drag, and scroll through Windows OS input injection."""
    actions = build_diagnostic_input_plan(portal_x, portal_y)
    emit_step_event(transcript, 6, "started", {
        "code": "diagnostic-input",
        "title": "Compositor-path diagnostic input",
        "action": "inject OS pointer, wheel, and Unicode input into the live overlay",
        "expected_visual": "composer focuses, portal drags, and OUTPUT pane scrolls via the normal runtime input path",
    }, host=host, user=user, action_labels=[a["label"] for a in actions])
    result = await run_windows_diagnostic_input(
        host,
        user=user,
        ssh_key=ssh_key,
        actions=actions,
        timeout_s=timeout_s,
    )
    status = "completed" if result.get("ok") else "failed"
    emit_step_event(transcript, 6, status, {
        "code": "diagnostic-input",
        "title": "Compositor-path diagnostic input",
        "action": "Windows OS input injector finished",
        "expected_visual": "transcript should include input:focus-gained, drag:start/drag:end, and scroll:output checkpoints",
    }, **result)


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
        initial_subscriptions=["SCENE_TOPOLOGY", "INPUT_EVENTS", "FOCUS_EVENTS"],
    )
    heartbeat_task: Optional[asyncio.Task] = None
    interaction_task: Optional[asyncio.Task] = None
    lease_id: Optional[bytes] = None
    scene_width = args.tab_width
    scene_height = args.tab_height
    cleanup_errors: list[str] = []
    mutation_lock = asyncio.Lock()

    try:
        emit_step_event(transcript, 0, "started", {
            "code": "scenario",
            "title": "Text Stream Portal live scenario",
            "action": "connect and open resident session",
            "expected_visual": "operator follows JSON step transcript",
        }, target=args.target, doc=args.doc, phases=args.phases)

        await client.connect()
        scene_width, scene_height = client.scene_display_area or (scene_width, scene_height)
        emit_step_event(transcript, 0, "checkpoint", {
            "code": "scene:display-area",
            "title": "Scene display area resolved",
            "action": "use live scene dimensions for portal placement and drag bounds",
            "expected_visual": "portal can be dragged across the full HUD surface",
        }, scene_width=scene_width, scene_height=scene_height)
        lease_ttl_ms = max(600_000, int(args.baseline_hold_s * 1000) + 120_000)
        lease_id = await client.request_lease(ttl_ms=lease_ttl_ms)
        set_portal_size(PORTAL_W, PORTAL_H, scene_width, scene_height)
        portal_x = (
            args.portal_x
            if args.portal_x is not None
            else scene_width - PORTAL_W - PORTAL_X_FROM_RIGHT
        )
        portal_x = max(0.0, min(portal_x, max(0.0, scene_width - PORTAL_W)))
        portal_y = max(0.0, min(PORTAL_Y, max(0.0, scene_height - PORTAL_H)))
        tiles = await create_portal_tiles(
            client=client,
            lease_id=lease_id,
            portal_x=portal_x,
            portal_y=portal_y,
            tab_width=scene_width,
            tab_height=scene_height,
        )
        heartbeat_interval_ms = client.heartbeat_interval_ms or 5_000
        heartbeat_task = asyncio.create_task(
            heartbeat_loop(client, heartbeat_interval_ms)
        )
        interaction_task = asyncio.create_task(
            portal_interaction_loop(
                client=client,
                lease_id=lease_id,
                tiles=tiles,
                transcript=transcript,
                body_full=body,
                initial_portal_x=portal_x,
                initial_portal_y=portal_y,
                tab_width=scene_width,
                tab_height=scene_height,
                mutation_lock=mutation_lock,
                clipboard_host=target_host(args.target),
                clipboard_user=args.clipboard_user,
                clipboard_ssh_key=args.clipboard_ssh_key,
                clipboard_timeout_s=args.clipboard_timeout_s,
            )
        )

        phases = [p.strip() for p in (args.phases or "baseline").split(",")]
        for phase in phases:
            if phase == "baseline":
                await run_baseline(
                    client, lease_id, tiles, body, transcript,
                    args.baseline_hold_s, mutation_lock,
                )
            elif phase == "scroll":
                await run_scroll(client, lease_id, tiles, transcript, mutation_lock)
            elif phase == "streaming":
                await run_streaming(
                    client, lease_id, tiles, body, transcript,
                    args.stream_chunks, args.stream_interval_s, mutation_lock,
                )
            elif phase == "rapid":
                await run_rapid(
                    client, lease_id, tiles, body, transcript,
                    args.rapid_cycles, args.rapid_interval_ms, mutation_lock,
                )
            elif phase == "composer-smoke":
                await run_composer_smoke(
                    client, lease_id, tiles, transcript, mutation_lock,
                    args.composer_smoke_hold_s,
                )
            elif phase == "diagnostic-input":
                await publish_portal(
                    client, lease_id, tiles,
                    title="Exemplar Review Portal",
                    subtitle="Compositor-path diagnostic input",
                    body=body,
                    footer_meta="diagnostic-input  •  OS input injector armed",
                    include_tile_setup=True,
                    mutation_lock=mutation_lock,
                )
                await run_diagnostic_input_phase(
                    transcript,
                    host=target_host(args.target),
                    user=args.diagnostic_input_user,
                    ssh_key=args.diagnostic_input_ssh_key,
                    portal_x=portal_x,
                    portal_y=portal_y,
                    timeout_s=args.diagnostic_input_timeout_s,
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
            "expected_visual": "portal visible until cleanup releases the lease",
        })
    finally:
        if heartbeat_task is not None:
            heartbeat_task.cancel()
            try:
                await heartbeat_task
            except asyncio.CancelledError:
                pass
        if interaction_task is not None:
            interaction_task.cancel()
            try:
                await interaction_task
            except asyncio.CancelledError:
                pass
            except Exception as exc:
                detail = f"{type(exc).__name__}: {exc}"
                cleanup_errors.append(f"interaction_task: {detail}")
                emit_step_event(transcript, 98, "failed", {
                    "code": "interaction:task-error",
                    "title": "Portal interaction loop failed",
                    "action": "continue cleanup despite interaction task failure",
                    "expected_visual": "portal lease cleanup still runs",
                }, error=detail)
        if lease_id is not None and not args.leave_lease_on_exit:
            try:
                await asyncio.wait_for(
                    client.release_lease(lease_id),
                    timeout=args.cleanup_timeout_s,
                )
                emit_step_event(transcript, 100, "completed", {
                    "code": "cleanup:lease-release",
                    "title": "Portal lease released",
                    "action": "release the lease before closing the session",
                    "expected_visual": "all portal tiles are removed from the HUD",
                })
            except Exception as exc:
                detail = f"{type(exc).__name__}: {exc}"
                cleanup_errors.append(detail)
                emit_step_event(transcript, 100, "failed", {
                    "code": "cleanup:lease-release",
                    "title": "Portal lease release failed",
                    "action": "attempted to release the portal lease before session close",
                    "expected_visual": "portal may remain until runtime orphan cleanup or HUD restart",
                }, error=detail)
        try:
            await client.close(reason="portal-exemplar done", expect_resume=False)
        except Exception:
            pass
        if args.transcript_out:
            write_transcript(args.transcript_out, {
                "target": args.target,
                "doc": args.doc,
                "scene_width": scene_width,
                "scene_height": scene_height,
                "portal_w": PORTAL_W,
                "portal_h": PORTAL_H,
                "lease_release_on_exit": not args.leave_lease_on_exit,
                "cleanup_errors": cleanup_errors,
                "steps": transcript,
            })

    return 0


def run_composer_self_test() -> int:
    width = composer_wrap_area_width_px()
    failures: list[str] = []

    fallback = composer_key_fallback_text("Space")
    if fallback != " ":
        failures.append(f"Space fallback returned {fallback!r}")
    if composer_key_fallback_text("A") is not None:
        failures.append("non-Space printable key-down produced fallback text")

    hello_display, hello_x, hello_row = composer_wrapped_layout(
        "hello world",
        len("hello world"),
        width,
    )
    if hello_display != "hello world" or hello_row != 0:
        failures.append(
            f"hello world wrapped unexpectedly: row={hello_row}, text={hello_display!r}"
        )
    expected_hello_x = len("hello world") * COMPOSER_CARET_CHAR_W
    if abs(hello_x - expected_hello_x) > 0.01:
        failures.append(f"hello world caret x={hello_x:.2f}, expected {expected_hello_x:.2f}")

    paste = (
        "**Long markdown paste** near the right edge of the composer line with "
        "words, punctuation, and enough text to wrap several visual rows without "
        "leaving a blank-looking trailing-space caret offset."
    )
    display, cursor_x, cursor_row = composer_wrapped_layout(paste, len(paste), width)
    lines = display.splitlines()
    for row, line in enumerate(lines[:-1]):
        if line.endswith((" ", "\t")):
            failures.append(f"wrapped line {row} ends with whitespace: {line!r}")
    if not (0 <= cursor_row < len(lines)):
        failures.append(f"caret row {cursor_row} outside {len(lines)} visual lines")
    if cursor_x > width + 0.01:
        failures.append(f"caret x={cursor_x:.2f} exceeds wrap width {width:.2f}")

    long_word = "x" * int(width // COMPOSER_WRAP_CHAR_W + 8)
    _, long_x, long_row = composer_wrapped_layout(long_word, len(long_word), width)
    if long_row == 0:
        failures.append("long unbroken word did not wrap")
    if long_x > width + 0.01:
        failures.append(f"long-word caret x={long_x:.2f} exceeds wrap width {width:.2f}")

    result = {
        "status": "failed" if failures else "passed",
        "wrap_width_px": width,
        "char_width_px": COMPOSER_WRAP_CHAR_W,
        "failures": failures,
        "cases": {
            "hello_world": {
                "display": hello_display,
                "cursor_x": hello_x,
                "cursor_row": hello_row,
            },
            "long_markdown_paste": {
                "visual_lines": len(lines),
                "cursor_x": cursor_x,
                "cursor_row": cursor_row,
            },
            "long_word": {
                "cursor_x": long_x,
                "cursor_row": long_row,
            },
        },
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 1 if failures else 0


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Run the Text Stream Portal live resident gRPC scenario."
    )
    p.add_argument(
        "--self-test",
        action="store_true",
        help="Run local composer wrap/caret/input smoke checks without opening gRPC",
    )
    p.add_argument("--target", default=DEFAULT_TARGET)
    p.add_argument("--psk-env", default=DEFAULT_PSK_ENV)
    p.add_argument("--agent-id", default="agent-alpha")
    p.add_argument("--doc", default=DEFAULT_DOC)
    p.add_argument("--max-lines", type=int, default=120)
    p.add_argument("--tab-width", type=float, default=1920.0)
    p.add_argument("--tab-height", type=float, default=1080.0)
    p.add_argument("--portal-x", type=float, default=None)
    p.add_argument("--phases", default="baseline,scroll",
                   help="Comma list: baseline,scroll,streaming,rapid,composer-smoke,diagnostic-input")
    p.add_argument("--baseline-hold-s", type=float, default=20.0)
    p.add_argument("--composer-smoke-hold-s", type=float, default=8.0)
    p.add_argument("--stream-chunks", type=int, default=6)
    p.add_argument("--stream-interval-s", type=float, default=1.5)
    p.add_argument("--rapid-cycles", type=int, default=12)
    p.add_argument("--rapid-interval-ms", type=int, default=80)
    p.add_argument("--cleanup-timeout-s", type=float, default=5.0)
    p.add_argument("--clipboard-user", default="tzeus")
    p.add_argument("--clipboard-ssh-key", default=DEFAULT_SSH_KEY)
    p.add_argument("--clipboard-timeout-s", type=float, default=0.7)
    p.add_argument("--diagnostic-input-user", default="tzeus")
    p.add_argument("--diagnostic-input-ssh-key", default=DEFAULT_SSH_KEY)
    p.add_argument("--diagnostic-input-timeout-s", type=float, default=12.0)
    p.add_argument(
        "--leave-lease-on-exit",
        action="store_true",
        help="Skip explicit lease release on exit; only use when testing orphan/grace behavior",
    )
    p.add_argument("--transcript-out", default=DEFAULT_TRANSCRIPT_PATH)
    return p.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        return run_composer_self_test()
    try:
        return asyncio.run(run_scenario(args))
    except KeyboardInterrupt:
        print(json.dumps({"error": "interrupted"}), file=sys.stderr)
        return 130
    except Exception as exc:
        print(json.dumps({"error": "exception", "detail": str(exc)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
