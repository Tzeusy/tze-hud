"""Two-pane chrome geometry resolution for the text-stream-portal exemplar.

Companion to :mod:`portal_part_tokens`. Where that module mirrors the *canonical*
Rust portal token surface (``crates/tze_hud_config/src/portal_tokens.rs``) that
governs colors/fonts/spacing for the single-node Phase-1 pilot, this module owns
the **two-pane chrome GEOMETRY** — header height, pane split, content inset,
corner radius — that is specific to the exemplar's split INPUT/OUTPUT layout.

Why this lives here and NOT in the canonical Rust token surface (hud-q1qzw):
the two-pane split is the exemplar's own chrome; the runtime has no two-pane
concept (there is no ``two_pane``/``pane_split`` token in ``tze_hud_config``).
Adding ``portal.two_pane.*`` keys to the product token surface would ship
product tokens the runtime never consumes — dead surface that pollutes the
canonical vocabulary. The canonical ``portal.spacing.*`` tokens deliberately
carry the single-node pilot's values (content inset 6px, header 28px); the
two-pane chrome needs its own, different values (18px / 52px). So this is an
**exemplar-local geometry profile**, resolved through the SAME three-layer
semantics the visual tokens use:

    exemplar default  →  (geometry-profile override)  →  resolved value

Swapping the geometry profile (a different ``{key: value}`` override map) reskins
the chrome geometry — proving geometry is token/profile-driven, not literal,
exactly the way :func:`portal_part_tokens.resolve_portal_tokens` proves it for
colors/fonts.

Unlike the color/font tokens, these geometry values are NOT delivered over the
runtime handshake: the runtime does not model the two-pane split, so there is no
runtime-active geometry profile to adopt. The exemplar owns them end-to-end.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

# Reuse the exact numeric-parse semantics the visual-token resolver uses, so a
# geometry override string is parsed identically to a color/font token value.
from portal_part_tokens import parse_numeric

# ── Two-pane geometry token keys ───────────────────────────────────────────────

PORTAL_TWO_PANE_HEADER_HEIGHT_PX = "portal.two_pane.header_height_px"
PORTAL_TWO_PANE_FOOTER_HEIGHT_PX = "portal.two_pane.footer_height_px"
PORTAL_TWO_PANE_DIVIDER_HEIGHT_PX = "portal.two_pane.divider_height_px"
PORTAL_TWO_PANE_CONTENT_INSET_PX = "portal.two_pane.content_inset_px"
PORTAL_TWO_PANE_CORNER_RADIUS_PX = "portal.two_pane.corner_radius_px"
PORTAL_TWO_PANE_SPLIT_RATIO = "portal.two_pane.pane_split_ratio"
PORTAL_TWO_PANE_DIVIDER_WIDTH_PX = "portal.two_pane.pane_divider_width_px"
PORTAL_TWO_PANE_MIN_PANE_WIDTH_PX = "portal.two_pane.min_pane_width_px"


# ── Exemplar defaults ──────────────────────────────────────────────────────────
#
# These match the exemplar's reviewed two-pane chrome literals byte-for-byte, so
# resolving with no overrides reproduces the current appearance exactly (no
# visual regression). ``tests/test_text_stream_portal_exemplar.py`` asserts the
# exemplar's resolved geometry equals these defaults.

EXEMPLAR_GEOMETRY_DEFAULTS: dict[str, str] = {
    PORTAL_TWO_PANE_HEADER_HEIGHT_PX: "52",
    PORTAL_TWO_PANE_FOOTER_HEIGHT_PX: "30",
    PORTAL_TWO_PANE_DIVIDER_HEIGHT_PX: "1",
    PORTAL_TWO_PANE_CONTENT_INSET_PX: "18",
    PORTAL_TWO_PANE_CORNER_RADIUS_PX: "14",
    PORTAL_TWO_PANE_SPLIT_RATIO: "0.5",
    PORTAL_TWO_PANE_DIVIDER_WIDTH_PX: "6",
    PORTAL_TWO_PANE_MIN_PANE_WIDTH_PX: "240",
}


# ── Resolved geometry bundle ───────────────────────────────────────────────────


@dataclass(frozen=True)
class PortalTwoPaneGeometry:
    """Resolved two-pane chrome geometry.

    Built by :func:`resolve_two_pane_geometry`; every field is already parsed
    from its token string. The exemplar reads these directly when laying out the
    two-pane chrome — no geometry literals in the layout path.

    ``pane_split_ratio`` is the INPUT pane's fraction of the pane-bearing width
    ``(portal_w - pane_divider_width_px)``; ``0.5`` is the equal 50/50 split.
    """

    header_height_px: float
    footer_height_px: float
    divider_height_px: float
    content_inset_px: float
    corner_radius_px: float
    pane_split_ratio: float
    pane_divider_width_px: float
    min_pane_width_px: float


def _px(token_map: dict[str, str], key: str) -> float:
    raw = token_map.get(key, EXEMPLAR_GEOMETRY_DEFAULTS[key])
    parsed = parse_numeric(raw)
    if parsed is None:
        # Unparseable override → fall back to the exemplar default (mirrors the
        # visual resolver's warn-and-default behavior).
        parsed = parse_numeric(EXEMPLAR_GEOMETRY_DEFAULTS[key])
    assert parsed is not None  # exemplar defaults are always valid numerics
    return parsed


def resolve_two_pane_geometry(
    overrides: Optional[dict[str, str]] = None,
) -> PortalTwoPaneGeometry:
    """Resolve :class:`PortalTwoPaneGeometry` from an optional geometry-profile map.

    ``overrides`` is a ``{token_key: value_string}`` map (a geometry profile).
    Absent or unparseable keys fall back to the exemplar default — the same
    resolve semantics :func:`portal_part_tokens.resolve_portal_tokens` uses for
    visual tokens.
    """
    tm = dict(overrides) if overrides else {}
    return PortalTwoPaneGeometry(
        header_height_px=_px(tm, PORTAL_TWO_PANE_HEADER_HEIGHT_PX),
        footer_height_px=_px(tm, PORTAL_TWO_PANE_FOOTER_HEIGHT_PX),
        divider_height_px=_px(tm, PORTAL_TWO_PANE_DIVIDER_HEIGHT_PX),
        content_inset_px=_px(tm, PORTAL_TWO_PANE_CONTENT_INSET_PX),
        corner_radius_px=_px(tm, PORTAL_TWO_PANE_CORNER_RADIUS_PX),
        pane_split_ratio=_px(tm, PORTAL_TWO_PANE_SPLIT_RATIO),
        pane_divider_width_px=_px(tm, PORTAL_TWO_PANE_DIVIDER_WIDTH_PX),
        min_pane_width_px=_px(tm, PORTAL_TWO_PANE_MIN_PANE_WIDTH_PX),
    )
