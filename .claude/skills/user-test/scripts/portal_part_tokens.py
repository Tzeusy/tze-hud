"""Portal design-token resolution for the text-stream-portal exemplar.

This is a **faithful Python mirror** of the canonical portal token surface that
lives in Rust at ``crates/tze_hud_config/src/portal_tokens.rs``
(``PortalPartTokens`` + ``resolve_portal_tokens`` + the ``defaults`` module).

Why this exists (hud-7jrj3 / finding ``vd-exemplar-hardcodes-all-visual-values``):
the exemplar previously hardcoded every color/font/spacing as a Python literal in
its publish path, so a profile/token swap could not reskin the live portal
end-to-end. The promotion evidence gate (spec §"no literal styling in the
exemplar publish path") requires every published visual value to resolve from a
design token. This module gives the exemplar the same three-layer resolve
semantics the runtime uses:

    canonical default  →  (profile-scoped override)  →  resolved value

The exemplar seeds the resolver with an *exemplar profile* (a set of
``portal.*`` overrides that reproduces its reviewed look) exactly the way a real
component profile is a swappable set of token overrides — the values in the
publish path are then read from the resolved :class:`PortalPartTokens`, never
from bare literals. Swapping the profile (a different override map) reskins every
published value.

## Single source of truth

``CANONICAL_DEFAULTS`` mirrors the Rust ``mod defaults`` block byte-for-byte.
``tests/test_text_stream_portal_exemplar.py`` parses the Rust source and asserts
this mirror matches, so the two cannot silently drift. Colors are hex (``#RRGGBB``
or ``#RRGGBBAA``); numerics are plain decimal strings — identical to Rust.

## Runtime handshake exposure (hud-16um0)

The runtime now exposes its *resolved* ``PortalPartTokens`` over the session
handshake: ``SessionEstablished.portal_part_tokens`` carries the runtime's
ACTIVE profile's fully-resolved portal tokens as a ``{key: value_string}`` map
(see ``crates/tze_hud_config::resolve_portal_token_strings``). The exemplar
PREFERS that map — the runtime's active profile drives the live look — and uses
this module only as a typed FALLBACK when the runtime predates the field (empty
map). The runtime-delivered map is parsed by the same :func:`resolve_portal_tokens`
here, so this drift-guarded mirror still governs the parse path either way.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional


# ── Canonical token keys (mirror of PORTAL_TOKEN_* in portal_tokens.rs) ────────

PORTAL_TOKEN_FRAME_BACKGROUND = "portal.frame.background"
PORTAL_TOKEN_FRAME_OPACITY = "portal.frame.opacity"
PORTAL_TOKEN_FRAME_BORDER_COLOR = "portal.frame.border_color"

PORTAL_TOKEN_HEADER_TEXT_COLOR = "portal.header.text_color"
PORTAL_TOKEN_HEADER_FONT_SIZE = "portal.header.font_size"

PORTAL_TOKEN_COMPOSER_BACKGROUND = "portal.composer.background"
PORTAL_TOKEN_COMPOSER_TEXT_COLOR = "portal.composer.text_color"
PORTAL_TOKEN_COMPOSER_FONT_SIZE = "portal.composer.font_size"
PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR = "portal.composer.at_capacity_color"
PORTAL_TOKEN_COMPOSER_CARET_COLOR = "portal.composer.caret_color"
PORTAL_TOKEN_COMPOSER_SELECTION_COLOR = "portal.composer.selection_color"
PORTAL_TOKEN_COMPOSER_PLACEHOLDER_COLOR = "portal.composer.placeholder_color"

PORTAL_TOKEN_TRANSCRIPT_BACKGROUND = "portal.transcript.background"
PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR = "portal.transcript.text_color"
PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE = "portal.transcript.font_size"
PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR = "portal.transcript.dim_text_color"
PORTAL_TOKEN_TRANSCRIPT_DIM_BACKGROUND = "portal.transcript.dim_background"
PORTAL_TOKEN_TRANSCRIPT_MAX_MEASURE_PX = "portal.transcript.max_measure_px"

PORTAL_TOKEN_STALE_MARKER_COLOR = "portal.stale_marker.color"

PORTAL_TOKEN_UNREAD_INDICATOR_COLOR = "portal.unread_indicator.color"
PORTAL_TOKEN_AWAITING_REPLY_COLOR = "portal.awaiting_reply.color"
PORTAL_TOKEN_EMPTY_STATE_COLOR = "portal.empty_state.color"
PORTAL_TOKEN_CONNECTING_MARKER_COLOR = "portal.connecting_marker.color"

PORTAL_TOKEN_ACTIVITY_CUE_COLOR = "portal.activity_cue.color"
PORTAL_TOKEN_STREAMING_CURSOR_COLOR = "portal.streaming_cursor.color"

PORTAL_TOKEN_LIFECYCLE_ACTIVE_COLOR = "portal.lifecycle.active_color"
PORTAL_TOKEN_LIFECYCLE_ATTACHED_COLOR = "portal.lifecycle.attached_color"
PORTAL_TOKEN_LIFECYCLE_ATTENTION_COLOR = "portal.lifecycle.attention_color"
PORTAL_TOKEN_LIFECYCLE_INACTIVE_COLOR = "portal.lifecycle.inactive_color"
PORTAL_TOKEN_LIFECYCLE_ACCENT_WIDTH_PX = "portal.lifecycle.accent_width_px"

PORTAL_TOKEN_DIVIDER_COLOR = "portal.divider.color"
PORTAL_TOKEN_UNREAD_DIVIDER_COLOR = "portal.unread_divider.color"

PORTAL_TOKEN_COLLAPSED_BACKGROUND = "portal.collapsed_card.background"
PORTAL_TOKEN_COLLAPSED_TEXT_COLOR = "portal.collapsed_card.text_color"
PORTAL_TOKEN_COLLAPSED_FONT_SIZE = "portal.collapsed_card.font_size"

PORTAL_TOKEN_TRANSITION_IN_MS = "portal.transition.in_ms"
PORTAL_TOKEN_TRANSITION_OUT_MS = "portal.transition.out_ms"

PORTAL_TOKEN_WINDOW_MIN_WIDTH_PX = "portal.window.min_width_px"
PORTAL_TOKEN_WINDOW_MIN_HEIGHT_PX = "portal.window.min_height_px"
PORTAL_TOKEN_WINDOW_RESIZE_STEP_PX = "portal.window.resize_step_px"
PORTAL_TOKEN_WINDOW_RESIZE_AFFORDANCE_PX = "portal.window.resize_affordance_px"

PORTAL_TOKEN_SCROLL_INDICATOR_COLOR = "portal.scroll_indicator.color"
PORTAL_TOKEN_SCROLL_INDICATOR_WIDTH_PX = "portal.scroll_indicator.width_px"
PORTAL_TOKEN_SCROLL_INDICATOR_MIN_HEIGHT_PX = "portal.scroll_indicator.min_height_px"

PORTAL_TOKEN_FOCUS_RING_COLOR = "portal.focus_ring.color"
PORTAL_TOKEN_FOCUS_RING_WIDTH_PX = "portal.focus_ring.width_px"

PORTAL_TOKEN_WINDOW_RESIZE_GRIP_COLOR = "portal.window.resize_grip.color"
PORTAL_TOKEN_WINDOW_RESIZE_GRIP_HOVER_COLOR = "portal.window.resize_grip.hover_color"
PORTAL_TOKEN_WINDOW_RESIZE_GRIP_SIZE_PX = "portal.window.resize_grip.size_px"

PORTAL_TOKEN_SPACING_CONTENT_INSET_PX = "portal.spacing.content_inset_px"
PORTAL_TOKEN_SPACING_HEADER_HEIGHT_PX = "portal.spacing.header_height_px"
PORTAL_TOKEN_SPACING_SECTION_GAP_PX = "portal.spacing.section_gap_px"


# ── Canonical defaults ─────────────────────────────────────────────────────────
#
# Mirror of the Rust ``mod defaults`` block, keyed by the Rust const name. This
# is the drift-guarded table: the exemplar test parses portal_tokens.rs and
# asserts equality, so any change to the Rust defaults must be reflected here.
# Keeping the Rust const name as the key makes that structural comparison exact.

_RUST_DEFAULTS: dict[str, str] = {
    "FRAME_BACKGROUND": "#0A0D11",
    "FRAME_OPACITY": "0.98",
    "FRAME_BORDER_COLOR": "#2A3344",
    "HEADER_TEXT_COLOR": "#F5F8FF",
    "HEADER_FONT_SIZE": "16",
    "COMPOSER_BACKGROUND": "#0F1418",
    "COMPOSER_TEXT_COLOR": "#E0E8F4",
    "COMPOSER_FONT_SIZE": "16",
    "COMPOSER_AT_CAPACITY_COLOR": "#B87333",
    "TRANSCRIPT_BACKGROUND": "#0A0D11",
    "TRANSCRIPT_TEXT_COLOR": "#E6EFFA",
    "TRANSCRIPT_FONT_SIZE": "16",
    "TRANSCRIPT_DIM_TEXT_COLOR": "#6B7689",
    "TRANSCRIPT_DIM_BACKGROUND": "#07090C",
    "STALE_MARKER_COLOR": "#B87333",
    "UNREAD_INDICATOR_COLOR": "#6B7689",
    "AWAITING_REPLY_COLOR": "#7B85C4",
    "EMPTY_STATE_COLOR": "#5F8A78",
    "CONNECTING_MARKER_COLOR": "#4C93A6",
    "ACTIVITY_CUE_COLOR": "#8A8FB0",
    "STREAMING_CURSOR_COLOR": "#A6ABC8",
    "LIFECYCLE_ACTIVE_COLOR": "#4FA88A",
    "LIFECYCLE_ATTACHED_COLOR": "#5A8FC0",
    "LIFECYCLE_ATTENTION_COLOR": "#C28A3D",
    "LIFECYCLE_INACTIVE_COLOR": "#5A6373",
    "LIFECYCLE_ACCENT_WIDTH_PX": "4",
    "DIVIDER_COLOR": "#46536E",
    "UNREAD_DIVIDER_COLOR": "#586A8C",
    "COLLAPSED_BACKGROUND": "#12161C",
    "COLLAPSED_TEXT_COLOR": "#C8D6E8",
    "COLLAPSED_FONT_SIZE": "14",
    "TRANSITION_IN_MS": "120",
    "TRANSITION_OUT_MS": "80",
    "WINDOW_MIN_WIDTH_PX": "240",
    "WINDOW_MIN_HEIGHT_PX": "160",
    "WINDOW_RESIZE_STEP_PX": "32",
    "WINDOW_RESIZE_AFFORDANCE_PX": "8",
    "SCROLL_INDICATOR_COLOR": "#4A5568",
    "SCROLL_INDICATOR_WIDTH_PX": "4",
    "SCROLL_INDICATOR_MIN_HEIGHT_PX": "24",
    "COMPOSER_CARET_COLOR": "#E0E8F4",
    "COMPOSER_SELECTION_COLOR": "#3A7BD573",
    "COMPOSER_PLACEHOLDER_COLOR": "#6B7689",
    "FOCUS_RING_COLOR": "#3380FF",
    "FOCUS_RING_WIDTH_PX": "2",
    "WINDOW_RESIZE_GRIP_COLOR": "#5A6373",
    "WINDOW_RESIZE_GRIP_HOVER_COLOR": "#8A93A6",
    "WINDOW_RESIZE_GRIP_SIZE_PX": "14",
    "SPACING_CONTENT_INSET_PX": "6",
    "SPACING_HEADER_HEIGHT_PX": "28",
    "SPACING_SECTION_GAP_PX": "8",
    "TRANSCRIPT_MAX_MEASURE_PX": "0",
}

# Canonical defaults keyed by the wire token key (the form a profile overrides).
CANONICAL_DEFAULTS: dict[str, str] = {
    PORTAL_TOKEN_FRAME_BACKGROUND: _RUST_DEFAULTS["FRAME_BACKGROUND"],
    PORTAL_TOKEN_FRAME_OPACITY: _RUST_DEFAULTS["FRAME_OPACITY"],
    PORTAL_TOKEN_FRAME_BORDER_COLOR: _RUST_DEFAULTS["FRAME_BORDER_COLOR"],
    PORTAL_TOKEN_HEADER_TEXT_COLOR: _RUST_DEFAULTS["HEADER_TEXT_COLOR"],
    PORTAL_TOKEN_HEADER_FONT_SIZE: _RUST_DEFAULTS["HEADER_FONT_SIZE"],
    PORTAL_TOKEN_COMPOSER_BACKGROUND: _RUST_DEFAULTS["COMPOSER_BACKGROUND"],
    PORTAL_TOKEN_COMPOSER_TEXT_COLOR: _RUST_DEFAULTS["COMPOSER_TEXT_COLOR"],
    PORTAL_TOKEN_COMPOSER_FONT_SIZE: _RUST_DEFAULTS["COMPOSER_FONT_SIZE"],
    PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR: _RUST_DEFAULTS["COMPOSER_AT_CAPACITY_COLOR"],
    PORTAL_TOKEN_COMPOSER_CARET_COLOR: _RUST_DEFAULTS["COMPOSER_CARET_COLOR"],
    PORTAL_TOKEN_COMPOSER_SELECTION_COLOR: _RUST_DEFAULTS["COMPOSER_SELECTION_COLOR"],
    PORTAL_TOKEN_COMPOSER_PLACEHOLDER_COLOR: _RUST_DEFAULTS["COMPOSER_PLACEHOLDER_COLOR"],
    PORTAL_TOKEN_TRANSCRIPT_BACKGROUND: _RUST_DEFAULTS["TRANSCRIPT_BACKGROUND"],
    PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR: _RUST_DEFAULTS["TRANSCRIPT_TEXT_COLOR"],
    PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE: _RUST_DEFAULTS["TRANSCRIPT_FONT_SIZE"],
    PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR: _RUST_DEFAULTS["TRANSCRIPT_DIM_TEXT_COLOR"],
    PORTAL_TOKEN_TRANSCRIPT_DIM_BACKGROUND: _RUST_DEFAULTS["TRANSCRIPT_DIM_BACKGROUND"],
    PORTAL_TOKEN_TRANSCRIPT_MAX_MEASURE_PX: _RUST_DEFAULTS["TRANSCRIPT_MAX_MEASURE_PX"],
    PORTAL_TOKEN_STALE_MARKER_COLOR: _RUST_DEFAULTS["STALE_MARKER_COLOR"],
    PORTAL_TOKEN_UNREAD_INDICATOR_COLOR: _RUST_DEFAULTS["UNREAD_INDICATOR_COLOR"],
    PORTAL_TOKEN_AWAITING_REPLY_COLOR: _RUST_DEFAULTS["AWAITING_REPLY_COLOR"],
    PORTAL_TOKEN_EMPTY_STATE_COLOR: _RUST_DEFAULTS["EMPTY_STATE_COLOR"],
    PORTAL_TOKEN_CONNECTING_MARKER_COLOR: _RUST_DEFAULTS["CONNECTING_MARKER_COLOR"],
    PORTAL_TOKEN_ACTIVITY_CUE_COLOR: _RUST_DEFAULTS["ACTIVITY_CUE_COLOR"],
    PORTAL_TOKEN_STREAMING_CURSOR_COLOR: _RUST_DEFAULTS["STREAMING_CURSOR_COLOR"],
    PORTAL_TOKEN_LIFECYCLE_ACTIVE_COLOR: _RUST_DEFAULTS["LIFECYCLE_ACTIVE_COLOR"],
    PORTAL_TOKEN_LIFECYCLE_ATTACHED_COLOR: _RUST_DEFAULTS["LIFECYCLE_ATTACHED_COLOR"],
    PORTAL_TOKEN_LIFECYCLE_ATTENTION_COLOR: _RUST_DEFAULTS["LIFECYCLE_ATTENTION_COLOR"],
    PORTAL_TOKEN_LIFECYCLE_INACTIVE_COLOR: _RUST_DEFAULTS["LIFECYCLE_INACTIVE_COLOR"],
    PORTAL_TOKEN_LIFECYCLE_ACCENT_WIDTH_PX: _RUST_DEFAULTS["LIFECYCLE_ACCENT_WIDTH_PX"],
    PORTAL_TOKEN_DIVIDER_COLOR: _RUST_DEFAULTS["DIVIDER_COLOR"],
    PORTAL_TOKEN_UNREAD_DIVIDER_COLOR: _RUST_DEFAULTS["UNREAD_DIVIDER_COLOR"],
    PORTAL_TOKEN_COLLAPSED_BACKGROUND: _RUST_DEFAULTS["COLLAPSED_BACKGROUND"],
    PORTAL_TOKEN_COLLAPSED_TEXT_COLOR: _RUST_DEFAULTS["COLLAPSED_TEXT_COLOR"],
    PORTAL_TOKEN_COLLAPSED_FONT_SIZE: _RUST_DEFAULTS["COLLAPSED_FONT_SIZE"],
    PORTAL_TOKEN_TRANSITION_IN_MS: _RUST_DEFAULTS["TRANSITION_IN_MS"],
    PORTAL_TOKEN_TRANSITION_OUT_MS: _RUST_DEFAULTS["TRANSITION_OUT_MS"],
    PORTAL_TOKEN_WINDOW_MIN_WIDTH_PX: _RUST_DEFAULTS["WINDOW_MIN_WIDTH_PX"],
    PORTAL_TOKEN_WINDOW_MIN_HEIGHT_PX: _RUST_DEFAULTS["WINDOW_MIN_HEIGHT_PX"],
    PORTAL_TOKEN_WINDOW_RESIZE_STEP_PX: _RUST_DEFAULTS["WINDOW_RESIZE_STEP_PX"],
    PORTAL_TOKEN_WINDOW_RESIZE_AFFORDANCE_PX: _RUST_DEFAULTS["WINDOW_RESIZE_AFFORDANCE_PX"],
    PORTAL_TOKEN_SCROLL_INDICATOR_COLOR: _RUST_DEFAULTS["SCROLL_INDICATOR_COLOR"],
    PORTAL_TOKEN_SCROLL_INDICATOR_WIDTH_PX: _RUST_DEFAULTS["SCROLL_INDICATOR_WIDTH_PX"],
    PORTAL_TOKEN_SCROLL_INDICATOR_MIN_HEIGHT_PX: _RUST_DEFAULTS["SCROLL_INDICATOR_MIN_HEIGHT_PX"],
    PORTAL_TOKEN_FOCUS_RING_COLOR: _RUST_DEFAULTS["FOCUS_RING_COLOR"],
    PORTAL_TOKEN_FOCUS_RING_WIDTH_PX: _RUST_DEFAULTS["FOCUS_RING_WIDTH_PX"],
    PORTAL_TOKEN_WINDOW_RESIZE_GRIP_COLOR: _RUST_DEFAULTS["WINDOW_RESIZE_GRIP_COLOR"],
    PORTAL_TOKEN_WINDOW_RESIZE_GRIP_HOVER_COLOR: _RUST_DEFAULTS["WINDOW_RESIZE_GRIP_HOVER_COLOR"],
    PORTAL_TOKEN_WINDOW_RESIZE_GRIP_SIZE_PX: _RUST_DEFAULTS["WINDOW_RESIZE_GRIP_SIZE_PX"],
    PORTAL_TOKEN_SPACING_CONTENT_INSET_PX: _RUST_DEFAULTS["SPACING_CONTENT_INSET_PX"],
    PORTAL_TOKEN_SPACING_HEADER_HEIGHT_PX: _RUST_DEFAULTS["SPACING_HEADER_HEIGHT_PX"],
    PORTAL_TOKEN_SPACING_SECTION_GAP_PX: _RUST_DEFAULTS["SPACING_SECTION_GAP_PX"],
}


# ── Parsing (mirror of tokens.rs parse_color_hex / parse_numeric) ──────────────

Rgba = tuple[float, float, float, float]


def parse_color_hex(value: str) -> Optional[Rgba]:
    """Parse ``#RRGGBB`` or ``#RRGGBBAA`` into an RGBA tuple of 0..1 floats.

    Mirrors ``tze_hud_config::tokens::parse_color_hex``: 6-digit hex resolves to
    opaque (alpha 1.0); 8-digit hex carries an explicit alpha byte. Any other
    shape returns ``None`` (caller falls back to the canonical default).
    """
    s = value.strip()
    if not s.startswith("#") or not s.isascii():
        return None
    hexpart = s[1:]
    try:
        if len(hexpart) == 6:
            r = int(hexpart[0:2], 16)
            g = int(hexpart[2:4], 16)
            b = int(hexpart[4:6], 16)
            return (r / 255.0, g / 255.0, b / 255.0, 1.0)
        if len(hexpart) == 8:
            r = int(hexpart[0:2], 16)
            g = int(hexpart[2:4], 16)
            b = int(hexpart[4:6], 16)
            a = int(hexpart[6:8], 16)
            return (r / 255.0, g / 255.0, b / 255.0, a / 255.0)
    except ValueError:
        return None
    return None


def parse_numeric(value: str) -> Optional[float]:
    """Parse a finite decimal string. Mirrors ``parse_numeric`` in Rust."""
    try:
        n = float(value.strip())
    except (ValueError, AttributeError):
        return None
    if n != n or n in (float("inf"), float("-inf")):  # NaN / inf guard
        return None
    return n


# ── Resolved token bundle ──────────────────────────────────────────────────────


@dataclass(frozen=True)
class PortalPartTokens:
    """Resolved portal visual values.

    Python analogue of the Rust ``PortalPartTokens`` struct. Built by
    :func:`resolve_portal_tokens`; every field is already parsed from its token
    string. The exemplar reads these directly when building scene nodes — no
    literal colors/sizes in the publish path.
    """

    # Frame
    frame_background: Rgba
    frame_opacity: float
    frame_border_color: Rgba
    # Header
    header_text_color: Rgba
    header_font_size_px: float
    # Composer
    composer_background: Rgba
    composer_text_color: Rgba
    composer_font_size_px: float
    composer_at_capacity_color: Rgba
    composer_caret_color: Rgba
    composer_selection_color: Rgba
    composer_placeholder_color: Rgba
    # Transcript
    transcript_background: Rgba
    transcript_text_color: Rgba
    transcript_font_size_px: float
    transcript_dim_text_color: Rgba
    transcript_dim_background: Rgba
    transcript_max_measure_px: float
    # Ancillary
    stale_marker_color: Rgba
    unread_indicator_color: Rgba
    awaiting_reply_color: Rgba
    empty_state_color: Rgba
    connecting_marker_color: Rgba
    activity_cue_color: Rgba
    streaming_cursor_color: Rgba
    lifecycle_active_color: Rgba
    lifecycle_attached_color: Rgba
    lifecycle_attention_color: Rgba
    lifecycle_inactive_color: Rgba
    lifecycle_accent_width_px: float
    divider_color: Rgba
    unread_divider_color: Rgba
    collapsed_background: Rgba
    collapsed_text_color: Rgba
    collapsed_font_size_px: float
    scroll_indicator_color: Rgba
    scroll_indicator_width_px: float
    scroll_indicator_min_height_px: float
    focus_ring_color: Rgba
    focus_ring_width_px: float
    resize_grip_color: Rgba
    resize_grip_hover_color: Rgba
    resize_grip_size_px: float
    content_inset_px: float
    header_height_px: float
    section_gap_px: float


def _color(token_map: dict[str, str], key: str) -> Rgba:
    raw = token_map.get(key, CANONICAL_DEFAULTS[key])
    parsed = parse_color_hex(raw)
    if parsed is None:
        # Unparseable override → fall back to the canonical default (mirrors the
        # Rust resolver's warn-and-default behavior).
        parsed = parse_color_hex(CANONICAL_DEFAULTS[key])
    assert parsed is not None  # canonical defaults are always valid hex
    return parsed


def _px(token_map: dict[str, str], key: str) -> float:
    raw = token_map.get(key, CANONICAL_DEFAULTS[key])
    parsed = parse_numeric(raw)
    if parsed is None:
        parsed = parse_numeric(CANONICAL_DEFAULTS[key])
    assert parsed is not None
    return parsed


def resolve_portal_tokens(overrides: Optional[dict[str, str]] = None) -> PortalPartTokens:
    """Resolve :class:`PortalPartTokens` from an optional profile override map.

    ``overrides`` is a ``{token_key: value_string}`` map (the same shape a
    component profile contributes). Absent or unparseable keys fall back to the
    canonical default — exactly the resolve semantics of
    ``tze_hud_config::resolve_portal_tokens``.
    """
    tm = dict(overrides) if overrides else {}
    return PortalPartTokens(
        frame_background=_color(tm, PORTAL_TOKEN_FRAME_BACKGROUND),
        frame_opacity=_px(tm, PORTAL_TOKEN_FRAME_OPACITY),
        frame_border_color=_color(tm, PORTAL_TOKEN_FRAME_BORDER_COLOR),
        header_text_color=_color(tm, PORTAL_TOKEN_HEADER_TEXT_COLOR),
        header_font_size_px=_px(tm, PORTAL_TOKEN_HEADER_FONT_SIZE),
        composer_background=_color(tm, PORTAL_TOKEN_COMPOSER_BACKGROUND),
        composer_text_color=_color(tm, PORTAL_TOKEN_COMPOSER_TEXT_COLOR),
        composer_font_size_px=_px(tm, PORTAL_TOKEN_COMPOSER_FONT_SIZE),
        composer_at_capacity_color=_color(tm, PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR),
        composer_caret_color=_color(tm, PORTAL_TOKEN_COMPOSER_CARET_COLOR),
        composer_selection_color=_color(tm, PORTAL_TOKEN_COMPOSER_SELECTION_COLOR),
        composer_placeholder_color=_color(tm, PORTAL_TOKEN_COMPOSER_PLACEHOLDER_COLOR),
        transcript_background=_color(tm, PORTAL_TOKEN_TRANSCRIPT_BACKGROUND),
        transcript_text_color=_color(tm, PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR),
        transcript_font_size_px=_px(tm, PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE),
        transcript_dim_text_color=_color(tm, PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR),
        transcript_dim_background=_color(tm, PORTAL_TOKEN_TRANSCRIPT_DIM_BACKGROUND),
        transcript_max_measure_px=_px(tm, PORTAL_TOKEN_TRANSCRIPT_MAX_MEASURE_PX),
        stale_marker_color=_color(tm, PORTAL_TOKEN_STALE_MARKER_COLOR),
        unread_indicator_color=_color(tm, PORTAL_TOKEN_UNREAD_INDICATOR_COLOR),
        awaiting_reply_color=_color(tm, PORTAL_TOKEN_AWAITING_REPLY_COLOR),
        empty_state_color=_color(tm, PORTAL_TOKEN_EMPTY_STATE_COLOR),
        connecting_marker_color=_color(tm, PORTAL_TOKEN_CONNECTING_MARKER_COLOR),
        activity_cue_color=_color(tm, PORTAL_TOKEN_ACTIVITY_CUE_COLOR),
        streaming_cursor_color=_color(tm, PORTAL_TOKEN_STREAMING_CURSOR_COLOR),
        lifecycle_active_color=_color(tm, PORTAL_TOKEN_LIFECYCLE_ACTIVE_COLOR),
        lifecycle_attached_color=_color(tm, PORTAL_TOKEN_LIFECYCLE_ATTACHED_COLOR),
        lifecycle_attention_color=_color(tm, PORTAL_TOKEN_LIFECYCLE_ATTENTION_COLOR),
        lifecycle_inactive_color=_color(tm, PORTAL_TOKEN_LIFECYCLE_INACTIVE_COLOR),
        lifecycle_accent_width_px=_px(tm, PORTAL_TOKEN_LIFECYCLE_ACCENT_WIDTH_PX),
        divider_color=_color(tm, PORTAL_TOKEN_DIVIDER_COLOR),
        unread_divider_color=_color(tm, PORTAL_TOKEN_UNREAD_DIVIDER_COLOR),
        collapsed_background=_color(tm, PORTAL_TOKEN_COLLAPSED_BACKGROUND),
        collapsed_text_color=_color(tm, PORTAL_TOKEN_COLLAPSED_TEXT_COLOR),
        collapsed_font_size_px=_px(tm, PORTAL_TOKEN_COLLAPSED_FONT_SIZE),
        scroll_indicator_color=_color(tm, PORTAL_TOKEN_SCROLL_INDICATOR_COLOR),
        scroll_indicator_width_px=_px(tm, PORTAL_TOKEN_SCROLL_INDICATOR_WIDTH_PX),
        scroll_indicator_min_height_px=_px(tm, PORTAL_TOKEN_SCROLL_INDICATOR_MIN_HEIGHT_PX),
        focus_ring_color=_color(tm, PORTAL_TOKEN_FOCUS_RING_COLOR),
        focus_ring_width_px=_px(tm, PORTAL_TOKEN_FOCUS_RING_WIDTH_PX),
        resize_grip_color=_color(tm, PORTAL_TOKEN_WINDOW_RESIZE_GRIP_COLOR),
        resize_grip_hover_color=_color(tm, PORTAL_TOKEN_WINDOW_RESIZE_GRIP_HOVER_COLOR),
        resize_grip_size_px=_px(tm, PORTAL_TOKEN_WINDOW_RESIZE_GRIP_SIZE_PX),
        content_inset_px=_px(tm, PORTAL_TOKEN_SPACING_CONTENT_INSET_PX),
        header_height_px=_px(tm, PORTAL_TOKEN_SPACING_HEADER_HEIGHT_PX),
        section_gap_px=_px(tm, PORTAL_TOKEN_SPACING_SECTION_GAP_PX),
    )
