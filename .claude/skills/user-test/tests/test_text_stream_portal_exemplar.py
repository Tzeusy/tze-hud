from __future__ import annotations

import asyncio
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
                "drag-portal-header",
                "scroll-output-pane",
                "type-composer-text",
            ],
        )
        self.assertEqual(plan[0]["kind"], "click")
        self.assertEqual(plan[1]["kind"], "drag")
        self.assertEqual(plan[2]["kind"], "wheel")
        self.assertNotEqual(plan[1]["end_x"], plan[1]["start_x"])
        self.assertGreater(plan[1]["end_y"], plan[1]["start_y"])

    def test_diagnostic_input_plan_uses_clamped_drag_for_wheel_target(self) -> None:
        plan = portal.build_diagnostic_input_plan(
            0.0,
            0.0,
            tab_width=portal.PORTAL_W + 20.0,
            tab_height=portal.PORTAL_H + 20.0,
        )
        _, output_rect = portal.portal_pane_rects()

        drag = plan[1]
        wheel = plan[2]
        self.assertEqual(drag["end_x"], drag["start_x"])
        self.assertEqual(wheel["x"], output_rect.x + output_rect.w / 2.0)
        self.assertEqual(
            wheel["y"],
            20.0 + output_rect.y + min(output_rect.h - 10.0, 96.0),
        )

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
        self.assertIn("if (-not [HudDiagnosticInput]::SetCursorPos", script)
        self.assertIn("$sent = [HudDiagnosticInput]::SendInput", script)
        self.assertIn("public MOUSEINPUT mi", script)
        self.assertIn("public KEYBDINPUT ki", script)
        self.assertIn("public HARDWAREINPUT hi", script)
        self.assertIn("Marshal]::GetLastWin32Error()", script)
        self.assertIn("diagnostic-warning:SendInput failed sent=", script)
        self.assertIn("' input_size=' + $InputSize", script)
        self.assertNotIn("throw 'SendInput failed'", script)
        self.assertLess(
            script.index("$inputs = [HudDiagnosticInput+INPUT[]]::new(2)"),
            script.index("foreach ($ch in $text.ToCharArray())"),
        )
        self.assertEqual(script.count("$inputs = [HudDiagnosticInput+INPUT[]]::new(2)"), 1)
        self.assertNotIn("EventBatch", script)
        self.assertNotIn("input_event_tx", script)

    def test_windows_diagnostic_input_script_scales_scene_to_desktop_coordinates(self) -> None:
        script = portal.windows_diagnostic_input_script(
            [{"kind": "click", "label": "focus", "x": 3000.0, "y": 1200.0}],
            scene_width=3840.0,
            scene_height=2160.0,
        )

        self.assertIn("[System.Windows.Forms.Screen]::PrimaryScreen.Bounds", script)
        self.assertIn("$HudDiagnosticScaleX", script)
        self.assertIn("$targetX = $x * $HudDiagnosticScaleX", script)
        self.assertIn("$targetY = $y * $HudDiagnosticScaleY", script)

    def test_windows_diagnostic_input_uses_ssh_connect_timeout(self) -> None:
        captured: dict[str, tuple[str, ...]] = {}
        original = portal.asyncio.create_subprocess_exec

        class FakeProcess:
            returncode = 0

            async def communicate(self, input: bytes | None = None) -> tuple[bytes, bytes]:
                return b"ok", b""

        async def fake_create_subprocess_exec(
            *cmd: str,
            stdin: object,
            stdout: object,
            stderr: object,
        ) -> FakeProcess:
            captured["cmd"] = cmd
            return FakeProcess()

        async def run() -> None:
            portal.asyncio.create_subprocess_exec = fake_create_subprocess_exec
            try:
                result = await portal.run_windows_diagnostic_input(
                    "example.invalid",
                    user="tester",
                    ssh_key="/tmp/key",
                    actions=[],
                    timeout_s=1.0,
                    connect_timeout_s=2.0,
                )
            finally:
                portal.asyncio.create_subprocess_exec = original
            self.assertTrue(result["ok"])

        asyncio.run(run())
        self.assertIn("ConnectTimeout=2", captured["cmd"])

    def test_windows_diagnostic_input_runs_as_interactive_scheduled_task(self) -> None:
        captured: dict[str, tuple[str, ...]] = {}
        captured_input: dict[str, bytes] = {}
        original = portal.asyncio.create_subprocess_exec

        class FakeProcess:
            returncode = 0

            async def communicate(self, input: bytes | None = None) -> tuple[bytes, bytes]:
                captured_input["input"] = input or b""
                return (
                    b'{"ok":true,"returncode":0,"stdout":"diagnostic:focus","stderr":""}',
                    b"",
                )

        async def fake_create_subprocess_exec(
            *cmd: str,
            stdin: object,
            stdout: object,
            stderr: object,
        ) -> FakeProcess:
            captured["cmd"] = cmd
            return FakeProcess()

        async def run() -> None:
            portal.asyncio.create_subprocess_exec = fake_create_subprocess_exec
            try:
                result = await portal.run_windows_diagnostic_input(
                    "example.invalid",
                    user="tester",
                    ssh_key="/tmp/key",
                    actions=[
                        {
                            "kind": "click",
                            "label": "focus",
                            "x": 10.0,
                            "y": 20.0,
                        }
                    ],
                    timeout_s=1.0,
                    connect_timeout_s=2.0,
                )
            finally:
                portal.asyncio.create_subprocess_exec = original
            self.assertTrue(result["ok"])
            self.assertEqual(result["stdout"], "diagnostic:focus")

        asyncio.run(run())
        self.assertNotIn("-EncodedCommand", captured["cmd"])
        self.assertIn("-", captured["cmd"])
        remote_script = captured_input["input"].decode("utf-8")
        self.assertIn("Register-ScheduledTask", remote_script)
        self.assertIn("-LogonType Interactive", remote_script)
        self.assertIn("Start-ScheduledTask", remote_script)
        self.assertIn("TzeHudDiagnosticInput", remote_script)
        self.assertIn("text_stream_portal_diagnostic_input_result_", remote_script)

    def test_windows_diagnostic_task_script_always_cleans_up_task_and_files(self) -> None:
        script = portal.windows_diagnostic_task_script(
            "Write-Output 'ok'",
            user="tester",
            timeout_s=1.0,
            run_id="abc123",
        )

        self.assertIn("try {", script)
        self.assertIn("finally {", script)
        self.assertIn("Stop-ScheduledTask -TaskName $taskName", script)
        self.assertIn("Unregister-ScheduledTask -TaskName $taskName", script)
        self.assertIn("Remove-Item -Force $scriptPath,$resultPath", script)
        self.assertNotIn("exit 0", script)

    def test_windows_diagnostic_input_reaps_timed_out_process(self) -> None:
        original = portal.asyncio.create_subprocess_exec

        class SlowProcess:
            returncode = None

            def __init__(self) -> None:
                self.killed = False
                self.waited = False

            async def communicate(self, input: bytes | None = None) -> tuple[bytes, bytes]:
                await asyncio.sleep(1.0)
                return b"", b""

            def kill(self) -> None:
                self.killed = True

            async def wait(self) -> int:
                self.waited = True
                return -9

        proc = SlowProcess()

        async def fake_create_subprocess_exec(
            *cmd: str,
            stdin: object,
            stdout: object,
            stderr: object,
        ) -> SlowProcess:
            return proc

        async def run() -> None:
            portal.asyncio.create_subprocess_exec = fake_create_subprocess_exec
            try:
                result = await portal.run_windows_diagnostic_input(
                    "example.invalid",
                    user="tester",
                    ssh_key="/tmp/key",
                    actions=[],
                    timeout_s=0.01,
                )
            finally:
                portal.asyncio.create_subprocess_exec = original
            self.assertFalse(result["ok"])
            self.assertEqual(result["error"], "timeout")

        asyncio.run(run())
        self.assertTrue(proc.killed)
        self.assertTrue(proc.waited)


if __name__ == "__main__":
    unittest.main()
