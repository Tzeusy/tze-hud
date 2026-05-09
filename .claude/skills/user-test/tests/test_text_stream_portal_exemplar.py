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

    def test_diagnostic_input_plan_targets_portal_focus_drag_and_scroll(self) -> None:
        plan = portal.build_diagnostic_input_plan(1000.0, 120.0)

        labels = [step["label"] for step in plan]
        self.assertEqual(
            labels,
            [
                "focus-composer",
                "type-composer-text",
                "drag-portal-header",
                "scroll-output-pane",
            ],
        )
        self.assertEqual(plan[0]["kind"], "click")
        self.assertEqual(plan[2]["kind"], "drag")
        self.assertEqual(plan[3]["kind"], "wheel")
        self.assertNotEqual(plan[2]["end_x"], plan[2]["start_x"])
        self.assertGreater(plan[2]["end_y"], plan[2]["start_y"])

    def test_windows_diagnostic_input_script_uses_os_input_not_event_transcript(self) -> None:
        script = portal.windows_diagnostic_input_script(
            [
                {"kind": "click", "label": "focus", "x": 10.0, "y": 20.0},
                {
                    "kind": "drag",
                    "label": "drag",
                    "start_x": 10.0,
                    "start_y": 20.0,
                    "end_x": 30.0,
                    "end_y": 50.0,
                    "steps": 2,
                },
                {
                    "kind": "wheel",
                    "label": "scroll",
                    "x": 30.0,
                    "y": 50.0,
                    "delta": -240,
                    "count": 2,
                },
                {"kind": "text", "label": "text", "text": "ok"},
            ]
        )

        self.assertIn("SetCursorPos", script)
        self.assertIn("mouse_event", script)
        self.assertIn("SendInput", script)
        self.assertNotIn("EventBatch", script)
        self.assertNotIn("input_event_tx", script)


if __name__ == "__main__":
    unittest.main()
