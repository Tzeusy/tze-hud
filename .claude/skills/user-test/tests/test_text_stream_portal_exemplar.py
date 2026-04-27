from __future__ import annotations

import math
import sys
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

import text_stream_portal_exemplar as portal  # noqa: E402


class TextStreamPortalExemplarTests(unittest.TestCase):
    def test_caret_advance_matches_explicit_wrap_advance_at_line_end(self) -> None:
        max_width = portal.composer_wrap_area_width_px()
        chars_on_line = math.floor(max_width / portal.COMPOSER_WRAP_CHAR_W)
        text = "x" * chars_on_line

        display_text, cursor_x, cursor_row = portal.composer_wrapped_layout(
            text,
            len(text),
            max_width,
        )

        self.assertEqual(display_text, text)
        self.assertEqual(cursor_row, 0)
        self.assertAlmostEqual(
            cursor_x,
            portal.composer_wrap_text_width_px(text),
            places=5,
        )

    def test_space_is_the_only_printable_key_down_fallback(self) -> None:
        self.assertEqual(portal.composer_key_fallback_text("Space"), " ")
        self.assertIsNone(portal.composer_key_fallback_text("a"))
        self.assertIsNone(portal.composer_key_fallback_text("Tab"))


if __name__ == "__main__":
    unittest.main()
