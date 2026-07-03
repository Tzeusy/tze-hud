from __future__ import annotations

import asyncio
import contextlib
import math
import sys
import unittest
from pathlib import Path
from unittest import mock


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

import text_stream_portal_exemplar as portal  # noqa: E402


class TextStreamPortalExemplarTests(unittest.TestCase):
    def test_large_display_size_clamp_uses_scene_bounds_not_demo_cap(self) -> None:
        w, h = portal.clamp_portal_size(2200.0, 1400.0, 3840.0, 2160.0)

        self.assertEqual((w, h), (2200.0, 1400.0))
        self.assertGreater(w, 1280.0)
        self.assertGreater(h, 960.0)

    def test_large_display_default_size_regression_exceeds_old_cap(self) -> None:
        w, h = portal.default_portal_size(3840.0, 2160.0)

        self.assertEqual((w, h), (1720.0, 1360.0))
        self.assertGreater(w, 1280.0)
        self.assertGreater(h, 960.0)

    def test_profile_swap_expanded_stays_larger_than_large_display_standard(self) -> None:
        standard_w, standard_h = portal.default_portal_size(3840.0, 2160.0)
        profiles = portal.profile_swap_dimensions(standard_w, standard_h)
        expanded = next(profile for profile in profiles if profile[0] == "expanded")

        expanded_w, expanded_h = portal.clamp_portal_size(
            expanded[1],
            expanded[2],
            3840.0,
            2160.0,
        )
        self.assertGreater(expanded_w, standard_w)
        self.assertGreater(expanded_h, standard_h)

    def test_portal_size_clamp_keeps_geometry_on_screen(self) -> None:
        self.assertEqual(
            portal.clamp_portal_size(5000.0, 3000.0, 3840.0, 2160.0),
            (3840.0, 2160.0),
        )
        self.assertEqual(
            portal.clamp_portal_size(100.0, 100.0, 3840.0, 2160.0),
            (portal.PORTAL_MIN_W, portal.PORTAL_MIN_H),
        )

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


class ComposerInputModeRoutingTests(unittest.TestCase):
    """Headless coverage for composer keyboard-routing wiring (hud-dwcr7).

    The runtime only routes keystrokes into the ComposerDraftManager when the
    focused hit-region carries `accepts_composer_input=True` (types.proto:103,
    consumed by tze_hud_input::node_accepts_composer_input). Without it, a
    pointer-down acquires focus but typed characters are dropped — the operator
    "can't type anything in". These tests pin the wire-level flag so the
    regression cannot silently return.
    """

    @staticmethod
    def _collect_hit_regions(children):
        regions = {}
        for node in children:
            if node.HasField("hit_region"):
                regions[node.hit_region.interaction_id] = node.hit_region
        return regions

    def test_composer_hit_region_accepts_composer_input(self) -> None:
        _root, children = portal.build_input_scroll_nodes("")
        regions = self._collect_hit_regions(children)
        self.assertIn(
            portal.COMPOSER_INTERACTION_ID,
            regions,
            "composer tile must expose the composer focus hit-region",
        )
        composer = regions[portal.COMPOSER_INTERACTION_ID]
        self.assertTrue(
            composer.accepts_focus,
            "composer hit-region must acquire focus on pointer-down",
        )
        self.assertTrue(
            composer.accepts_composer_input,
            "composer hit-region must route keystrokes into the runtime draft "
            "manager (accepts_composer_input=True) or typing is dropped",
        )

    def test_make_node_forwards_accepts_composer_input(self) -> None:
        # The client node builder must serialize accepts_composer_input onto the
        # wire; a stale builder/binding silently drops it (the original bug).
        node = portal.make_hit_region(
            "probe", 0.0, 0.0, 10.0, 10.0, accepts_composer_input=True,
        )
        self.assertTrue(node.hit_region.accepts_composer_input)

    def test_header_drag_region_does_not_accept_composer_input(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        self.assertIn(
            portal.PORTAL_DRAG_INTERACTION_ID,
            regions,
            "frame chrome must expose the header drag hit-region",
        )
        drag = regions[portal.PORTAL_DRAG_INTERACTION_ID]
        self.assertFalse(
            drag.accepts_composer_input,
            "header drag region must not capture composer keystrokes",
        )


class HeaderDragBandTests(unittest.TestCase):
    """Full-width titlebar drag band (hud-643dv).

    Owner direction: the whole top header band should drag the portal like a
    Windows titlebar, minus the minimize control. Whole-portal MOVE is owned by
    the runtime (screen-sovereignty): the runtime generates a geometry-driven
    header-BAND drag handle for the portal frame tile, and — because a runtime
    drag handle is hit-tested before node hit-regions — that band yields to any
    interactive (accepts_pointer) node under the point so the minimize button
    still wins inside its own bounds.

    These pin the EXEMPLAR-side invariants the runtime band relies on:
      * the header drag node is a full-width semantic marker of the band, and
      * it is inert (accepts_pointer=False) so it neither drives movement nor
        shadows the minimize control (which would be caught by the band's
        yield-to-interactive-node rule and break the drag), while
      * the minimize control stays pointer-interactive so the band yields to it.
    """

    @staticmethod
    def _collect_hit_regions(children):
        regions = {}
        for node in children:
            if node.HasField("hit_region"):
                regions.setdefault(node.hit_region.interaction_id, []).append(
                    node.hit_region
                )
        return regions

    def test_header_drag_marker_spans_full_header_width(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        self.assertIn(portal.PORTAL_DRAG_INTERACTION_ID, regions)
        drag = regions[portal.PORTAL_DRAG_INTERACTION_ID][0]
        # Full top band: origin at the top-left corner, spanning the whole portal
        # width and the header height — not the old x=MINIMIZE_HIT_W carve-out.
        self.assertEqual(drag.bounds.x, 0.0, "drag band must start at the left edge")
        self.assertEqual(drag.bounds.y, 0.0, "drag band must start at the top edge")
        self.assertEqual(
            drag.bounds.width, portal.PORTAL_W,
            "drag band must span the FULL portal width (Windows-titlebar band)",
        )
        self.assertEqual(
            drag.bounds.height, portal.HEADER_H,
            "drag band height must be the header layout constant, not a new magic value",
        )

    def test_header_drag_marker_is_inert_not_a_pointer_target(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        for hit in regions[portal.PORTAL_DRAG_INTERACTION_ID]:
            # Movement is owned by the runtime header-band handle, so this node
            # must NOT claim the pointer: an accepts_pointer node here would
            # shadow the minimize control and be swallowed by the runtime band's
            # yield-to-interactive rule, breaking both drag and minimize.
            self.assertFalse(
                hit.accepts_pointer,
                "the header drag marker must be inert (accepts_pointer=False); the "
                "runtime band owns movement (hud-643dv)",
            )
            # Still pointer-only chrome — never a keyboard Tab stop.
            self.assertFalse(
                hit.accepts_focus,
                "the header drag marker must not be a Tab stop",
            )

    def test_minimize_control_beats_the_band(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        # The minimize control must stay pointer-interactive AND focusable so the
        # runtime band yields to it (pointer) and a keyboard viewer can reach it.
        self.assertIn(portal.PORTAL_MINIMIZE_INTERACTION_ID, regions)
        for hit in regions[portal.PORTAL_MINIMIZE_INTERACTION_ID]:
            self.assertTrue(
                hit.accepts_pointer,
                "minimize must remain a pointer target so the drag band yields to it",
            )
            self.assertTrue(
                hit.accepts_focus,
                "minimize stays a Tab stop (unchanged by the band)",
            )
        # And the minimize hit-rect lies within the full-width band, so the two
        # genuinely overlap and precedence (not geometry) is what protects it.
        minimize = regions[portal.PORTAL_MINIMIZE_INTERACTION_ID][0]
        drag = regions[portal.PORTAL_DRAG_INTERACTION_ID][0]
        self.assertGreaterEqual(minimize.bounds.x, drag.bounds.x)
        self.assertLessEqual(
            minimize.bounds.x + minimize.bounds.width,
            drag.bounds.x + drag.bounds.width,
            "minimize must sit inside the full-width band (real overlap)",
        )


class PortalFocusTraversalRingTests(unittest.TestCase):
    """Tab/Shift+Tab focus-traversal stops for the portal (hud-02sp5).

    The runtime focus ring (tze_hud_input::FocusManager::build_focus_cycle)
    collects only HitRegionNodes with accepts_focus=True (input-model spec
    "Focus Cycling"). The text-stream-portals spec names the focusable
    "portal controls" as the composer plus the expand, collapse, and reply
    affordances; drag/resize are pointer-only. Before this fix every frame
    affordance was accepts_focus=False, so a keyboard-only viewer (Mobile
    Presence Node / glasses) had no reachable stops. These tests pin the
    deliberate tab-ring set so it cannot silently regress in either
    direction (interactive control dropped, or a pointer-only drag/resize
    handle blindly promoted into the ring).
    """

    @staticmethod
    def _collect_hit_regions(children):
        regions = {}
        for node in children:
            if node.HasField("hit_region"):
                # A portal can carry two hit regions with the same
                # interaction_id (e.g. the L-shaped restore target); keep a
                # list so callers can assert every fragment agrees.
                regions.setdefault(node.hit_region.interaction_id, []).append(
                    node.hit_region
                )
        return regions

    # ── Expanded frame: interactive controls ARE focus stops ──────────────

    def test_minimize_collapse_control_is_focusable(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        self.assertIn(
            portal.PORTAL_MINIMIZE_INTERACTION_ID,
            regions,
            "frame chrome must expose the minimize/collapse control",
        )
        for hit in regions[portal.PORTAL_MINIMIZE_INTERACTION_ID]:
            self.assertTrue(
                hit.accepts_focus,
                "minimize is a collapse control and MUST be a Tab stop so a "
                "keyboard-only viewer can collapse the portal without a pointer",
            )

    def test_submit_reply_control_is_focusable(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        self.assertIn(portal.SUBMIT_INTERACTION_ID, regions)
        for hit in regions[portal.SUBMIT_INTERACTION_ID]:
            self.assertTrue(
                hit.accepts_focus,
                "submit is the reply affordance and MUST be a Tab stop",
            )

    # ── Minimized icon: restore/expand IS a focus stop ────────────────────

    def test_restore_expand_control_is_focusable(self) -> None:
        _root, children = portal.build_minimized_icon_nodes(
            attention=False, pulse=False,
        )
        regions = self._collect_hit_regions(children)
        self.assertIn(
            portal.PORTAL_RESTORE_INTERACTION_ID,
            regions,
            "minimized icon must expose the restore/expand control",
        )
        for hit in regions[portal.PORTAL_RESTORE_INTERACTION_ID]:
            self.assertTrue(
                hit.accepts_focus,
                "restore is the expand control and MUST be a Tab stop so a "
                "minimized portal can be re-expanded without a pointer",
            )

    # ── Pointer-only affordances are NOT focus stops (deliberate) ─────────

    def test_pointer_only_frame_affordances_are_not_focusable(self) -> None:
        _root, children = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        regions = self._collect_hit_regions(children)
        for interaction_id in (
            portal.PORTAL_DRAG_INTERACTION_ID,
            portal.PANE_RESIZE_INTERACTION_ID,
            portal.PORTAL_RESIZE_INTERACTION_ID,
        ):
            self.assertIn(interaction_id, regions)
            for hit in regions[interaction_id]:
                self.assertFalse(
                    hit.accepts_focus,
                    f"{interaction_id} is a pointer-only drag/resize handle and "
                    "MUST NOT be a Tab stop (matches Telegram-style chrome: the "
                    "title bar and resize grips are not tabbable)",
                )

    def test_minimized_icon_drag_is_not_focusable(self) -> None:
        _root, children = portal.build_minimized_icon_nodes(
            attention=False, pulse=False,
        )
        regions = self._collect_hit_regions(children)
        self.assertIn(portal.PORTAL_ICON_DRAG_INTERACTION_ID, regions)
        for hit in regions[portal.PORTAL_ICON_DRAG_INTERACTION_ID]:
            self.assertFalse(
                hit.accepts_focus,
                "the minimized-icon drag handle is pointer-only and MUST NOT be "
                "a Tab stop",
            )

    def test_capture_backstop_is_not_focusable(self) -> None:
        _root, children = portal.build_capture_backstop_nodes(1920.0, 1080.0)
        regions = self._collect_hit_regions(children)
        for hits in regions.values():
            for hit in hits:
                self.assertFalse(
                    hit.accepts_focus,
                    "the capture backstop is an invisible pointer-capture surface "
                    "and MUST NOT be a Tab stop",
                )

    # ── Whole-ring shape assertion ────────────────────────────────────────

    def test_expanded_tab_ring_is_the_deliberate_control_set(self) -> None:
        """The union of focusable frame + composer stops is exactly the
        interactive control set — no pointer-only handle leaks in."""
        _root, frame = portal.build_portal_nodes(
            "title", "subtitle", "body", "footer",
        )
        _croot, composer = portal.build_input_scroll_nodes("")
        focusable = set()
        pointer_only = set()
        for children in (frame, composer):
            for iid, hits in self._collect_hit_regions(children).items():
                for hit in hits:
                    (focusable if hit.accepts_focus else pointer_only).add(iid)
        # Interactive controls that a keyboard-only viewer must be able to reach.
        self.assertIn(portal.PORTAL_MINIMIZE_INTERACTION_ID, focusable)
        self.assertIn(portal.SUBMIT_INTERACTION_ID, focusable)
        self.assertIn(portal.COMPOSER_INTERACTION_ID, focusable)
        # Pointer-only chrome must stay out of the ring.
        self.assertNotIn(portal.PORTAL_DRAG_INTERACTION_ID, focusable)
        self.assertNotIn(portal.PORTAL_RESIZE_INTERACTION_ID, focusable)
        self.assertNotIn(portal.PANE_RESIZE_INTERACTION_ID, focusable)


class PromotionGateEvidenceSchemaTests(unittest.TestCase):
    """Headless coverage for the RFC 0013 §7.2 promotion-gate evidence schema.

    These exercise the artifact-shaping logic WITHOUT a live HUD, so the harness
    is provably emitting the gate-conformant artifact shape before any live run.
    """

    # ── Reference-hardware tag (engineering-bar §2) ───────────────────────

    def test_reference_hardware_tag_has_required_gate_fields(self) -> None:
        tag = portal.reference_hardware_tag()
        for field in ("tag", "hostname", "gpu", "gpu_driver", "os", "is_reference"):
            self.assertIn(field, tag, f"missing reference-hardware field {field!r}")
        # Defaults fall back to the canonical reference host constants.
        self.assertEqual(tag["tag"], portal.REFERENCE_HARDWARE_TAG)
        self.assertEqual(tag["gpu"], portal.REFERENCE_GPU)
        self.assertEqual(tag["gpu_driver"], portal.REFERENCE_GPU_DRIVER)

    def test_reference_hardware_tag_honours_cli_overrides(self) -> None:
        tag = portal.reference_hardware_tag(
            tag="OtherHost",
            hostname="some-laptop",
            gpu="Intel UHD",
            gpu_driver="1.2.3",
        )
        self.assertEqual(tag["tag"], "OtherHost")
        self.assertEqual(tag["hostname"], "some-laptop")
        self.assertEqual(tag["gpu"], "Intel UHD")
        self.assertEqual(tag["gpu_driver"], "1.2.3")
        self.assertFalse(tag["is_reference"])

    def test_reference_hardware_tag_marks_reference_host_from_target(self) -> None:
        tag = portal.reference_hardware_tag(
            target=portal.REFERENCE_HOSTNAME + ":50051",
        )
        self.assertTrue(tag["is_reference"])
        self.assertEqual(tag["target_host"], portal.REFERENCE_HOSTNAME)

    def test_reference_run_marks_reference_even_from_nonreference_collector(self) -> None:
        # The orchestration box is a non-reference Linux collector, but the run
        # drives the Windows reference target. is_reference must follow the
        # target, so the legitimate reference run is NOT misread as off-reference.
        with mock.patch.object(portal.socket, "gethostname", return_value="linux-collector"):
            tag = portal.reference_hardware_tag(
                target=portal.REFERENCE_HOSTNAME + ":50051",
            )
        self.assertEqual(tag["collected_hostname"], "linux-collector")
        self.assertEqual(tag["target_host"], portal.REFERENCE_HOSTNAME)
        self.assertTrue(tag["is_reference"])

    def test_reference_hardware_tag_ignores_collector_host_for_reference(self) -> None:
        # Even if the collector itself happens to be the reference host, a run
        # that drives a NON-reference target is informational-only. is_reference
        # is a property of the target, never the collection host.
        with mock.patch.object(portal.socket, "gethostname", return_value=portal.REFERENCE_HOSTNAME):
            tag = portal.reference_hardware_tag(
                target="some-other-host:50051",
            )
        self.assertEqual(tag["collected_hostname"], portal.REFERENCE_HOSTNAME)
        self.assertEqual(tag["target_host"], "some-other-host")
        self.assertFalse(tag["is_reference"])

    def test_reference_hardware_tag_without_target_is_not_reference(self) -> None:
        # No target means nothing to attribute reference status to, even when the
        # collector is the reference host. Fail-fast: do not silently infer
        # reference status from the local box.
        with mock.patch.object(portal.socket, "gethostname", return_value=portal.REFERENCE_HOSTNAME):
            tag = portal.reference_hardware_tag()
        self.assertEqual(tag["target_host"], "")
        self.assertFalse(tag["is_reference"])

    def test_reference_hostname_override_matches_target(self) -> None:
        # A run may declare the reference host identity explicitly (e.g. the real
        # TzeHouse tailnet name). is_reference is true only when the target host
        # matches that declared identity...
        tag = portal.reference_hardware_tag(
            hostname="tzehouse.tailnet",
            target="tzehouse.tailnet:50051",
        )
        self.assertTrue(tag["is_reference"])
        # ...and false when the declared reference host is not the target driven.
        other = portal.reference_hardware_tag(
            hostname="tzehouse.tailnet",
            target="some-other-host:50051",
        )
        self.assertFalse(other["is_reference"])

    # ── Cadence axis: RTT baseline vs runtime overhead ────────────────────

    def test_cadence_evidence_separates_runtime_overhead_from_rtt(self) -> None:
        # Two presented appends: present latency 30ms over each cycle's own 20ms
        # transport RTT ⇒ runtime overhead 10ms each (within the present budget).
        appends = [
            {"cycle": 1, "publish_ms": 0.0, "present_ms": 30.0, "rtt_ms": 20.0},
            {"cycle": 2, "publish_ms": 100.0, "present_ms": 130.0, "rtt_ms": 20.0},
        ]
        ev = portal.build_cadence_rtt_evidence(
            20.0, appends, cadence_cycles=2, cadence_interval_ms=100,
        )
        self.assertEqual(ev["transport_rtt_baseline_ms"], 20.0)
        self.assertEqual(ev["presented_count"], 2)
        self.assertEqual(ev["coalesced_count"], 0)
        for entry in ev["appends"]:
            self.assertTrue(entry["presented"])
            self.assertAlmostEqual(entry["present_latency_ms"], 30.0)
            # Overhead is isolated against THIS cycle's RTT, not the baseline.
            self.assertAlmostEqual(entry["overhead_baseline_ms"], 20.0)
            self.assertAlmostEqual(entry["overhead_ms"], 10.0)
            self.assertTrue(entry["within_present_budget"])
        self.assertEqual(ev["runtime_overhead_ms"]["over_budget_count"], 0)
        self.assertTrue(ev["within_present_budget"])

    def test_cadence_overhead_uses_per_cycle_rtt_not_fixed_baseline(self) -> None:
        # Regression for hud-lod76 (root-cause hud-ans49 / live evidence
        # hud-ofe76): a cycle whose transport RTT spiked to 56ms — with the
        # publish→present latency tracking that same spike — must score ~0
        # RUNTIME overhead and PASS the budget. The discredited fixed-baseline
        # calc (present_latency - 22.008ms) would instead report ~34ms overhead
        # and falsely FAIL the 16.6ms runtime budget, charging transport jitter
        # to the runtime.
        fixed_baseline = 22.008
        appends = [
            {"cycle": 1, "publish_ms": 0.0, "present_ms": 56.04, "rtt_ms": 56.0},
        ]
        ev = portal.build_cadence_rtt_evidence(
            fixed_baseline, appends, cadence_cycles=1, cadence_interval_ms=100,
        )
        entry = ev["appends"][0]
        # Per-cycle isolation: overhead = present_latency(56.04) - this RTT(56.0).
        self.assertAlmostEqual(entry["overhead_baseline_ms"], 56.0)
        self.assertAlmostEqual(entry["overhead_ms"], 0.04, places=3)
        self.assertTrue(entry["within_present_budget"])
        self.assertTrue(ev["within_present_budget"])
        self.assertEqual(ev["runtime_overhead_ms"]["over_budget_count"], 0)
        # The transport baseline is still recorded for context...
        self.assertEqual(ev["transport_rtt_baseline_ms"], fixed_baseline)
        # ...but the old fixed-baseline calc would have falsely failed the gate.
        old_overhead = entry["present_latency_ms"] - fixed_baseline
        self.assertGreater(old_overhead, portal.HIGH_MUTATION_PRESENT_BUDGET_MS)

    def test_cadence_evidence_records_per_append_publish_present_timestamps(self) -> None:
        appends = [{"cycle": 1, "publish_ms": 5.0, "present_ms": 12.0}]
        ev = portal.build_cadence_rtt_evidence(
            2.0, appends, cadence_cycles=1, cadence_interval_ms=100,
        )
        entry = ev["appends"][0]
        # The per-append entry preserves the publish→present timestamps the gate
        # requires (tasks 5.7 / spec "runtime overhead beyond transport RTT").
        self.assertEqual(entry["publish_ms"], 5.0)
        self.assertEqual(entry["present_ms"], 12.0)
        self.assertAlmostEqual(entry["present_latency_ms"], 7.0)

    def test_cadence_evidence_excludes_coalesced_appends_from_budget(self) -> None:
        # A coalesced-away append (no present_ms) must not count toward the
        # presented-append budget per the spec scenario.
        appends = [
            {"cycle": 1, "publish_ms": 0.0, "present_ms": 25.0},
            {"cycle": 2, "publish_ms": 50.0},  # coalesced away — no present
        ]
        ev = portal.build_cadence_rtt_evidence(
            20.0, appends, cadence_cycles=2, cadence_interval_ms=100,
        )
        self.assertEqual(ev["presented_count"], 1)
        self.assertEqual(ev["coalesced_count"], 1)
        coalesced = ev["appends"][1]
        self.assertFalse(coalesced["presented"])
        self.assertIsNone(coalesced["overhead_ms"])
        self.assertNotIn("within_present_budget", coalesced)

    def test_cadence_evidence_flags_over_budget_runtime_overhead(self) -> None:
        # Genuine runtime overhead: present latency 40ms while THIS cycle's
        # transport RTT was only 5ms ⇒ 35ms runtime overhead > 16.6ms budget.
        appends = [{"cycle": 1, "publish_ms": 0.0, "present_ms": 40.0, "rtt_ms": 5.0}]
        ev = portal.build_cadence_rtt_evidence(
            5.0, appends, cadence_cycles=1, cadence_interval_ms=100,
        )
        self.assertEqual(ev["runtime_overhead_ms"]["over_budget_count"], 1)
        self.assertFalse(ev["within_present_budget"])
        self.assertFalse(ev["appends"][0]["within_present_budget"])

    # ── Operator-confirmable evidence (window-mgmt / profile-swap) ────────

    def test_operator_evidence_entry_is_structured(self) -> None:
        entry = portal.operator_evidence_entry(
            "window-mgmt:move",
            "portal moved cleanly to centre",
            {"portal_x": 100.0, "portal_y": 50.0},
        )
        self.assertEqual(entry["code"], "window-mgmt:move")
        self.assertEqual(entry["operator_confirm"], "portal moved cleanly to centre")
        self.assertEqual(entry["observed"]["portal_x"], 100.0)
        # Operator confirmation defaults to unfilled until the live run.
        self.assertIsNone(entry["confirmed"])

    # ── Full artifact envelope ────────────────────────────────────────────

    def test_evidence_artifact_carries_reference_tag_and_schema_version(self) -> None:
        tag = portal.reference_hardware_tag(target=portal.REFERENCE_HOSTNAME + ":50051")
        artifact = portal.build_evidence_artifact(
            target="host:50051",
            doc="docs/x.md",
            phases="cadence,window-mgmt,profile-swap",
            scene_width=1920.0,
            scene_height=1080.0,
            portal_w=860.0,
            portal_h=680.0,
            lease_release_on_exit=True,
            cleanup_errors=[],
            steps=[{"code": "cadence", "status": "completed"}],
            hardware_tag=tag,
        )
        self.assertEqual(artifact["schema_version"], portal.EVIDENCE_SCHEMA_VERSION)
        self.assertIn("reference_hardware", artifact)
        self.assertTrue(artifact["reference_hardware"]["is_reference"])
        # The artifact must round-trip through JSON (it is written to disk).
        import json as _json
        _json.dumps(artifact)

    def test_evidence_artifact_is_backward_compatible(self) -> None:
        # The fields the prior schema exposed must still be present so existing
        # consumers/readers do not break.
        artifact = portal.build_evidence_artifact(
            target="host:50051",
            doc="docs/x.md",
            phases="baseline",
            scene_width=1920.0,
            scene_height=1080.0,
            portal_w=860.0,
            portal_h=680.0,
            lease_release_on_exit=False,
            cleanup_errors=["boom"],
            steps=[],
            hardware_tag=portal.reference_hardware_tag(),
        )
        for legacy_field in (
            "target", "doc", "scene_width", "scene_height",
            "portal_w", "portal_h", "lease_release_on_exit",
            "cleanup_errors", "steps",
        ):
            self.assertIn(legacy_field, artifact)
        self.assertEqual(artifact["cleanup_errors"], ["boom"])
        self.assertFalse(artifact["lease_release_on_exit"])


# ── Sustained soak phase (hud-pnofj) ──────────────────────────────────────────


class SoakPhaseTests(unittest.TestCase):
    def test_soak_tail_history_stays_bounded_to_window_lines(self) -> None:
        seed = [f"seed {i}" for i in range(5)]
        lines = list(seed[-3:])

        for cycle in range(1, 101):
            portal.append_soak_tail_line(lines, seed, cycle, elapsed_s=float(cycle), window_lines=3)
            self.assertLessEqual(len(lines), 3)

        self.assertEqual(len(lines), 3)
        self.assertTrue(all(line.startswith("[soak] line") for line in lines))
        self.assertIn("line 000098", lines[0])
        self.assertIn("line 000100", lines[-1])

    def test_soak_pacing_subtracts_publish_duration(self) -> None:
        captured_bodies: list[list[str]] = []
        sleeps: list[float] = []
        now = 0.0

        original_publish = portal.publish_portal
        original_sleep = portal.asyncio.sleep
        original_monotonic = portal.time.monotonic

        def fake_monotonic() -> float:
            return now

        async def fake_publish_portal(*args, body: str, **kwargs) -> None:
            nonlocal now
            captured_bodies.append(body.splitlines())
            now += 0.07

        async def fake_sleep(delay_s: float) -> None:
            nonlocal now
            sleeps.append(delay_s)
            now += delay_s

        async def run() -> list[dict]:
            transcript: list[dict] = []
            await portal.run_soak(
                client=object(),
                lease_id=b"lease",
                tiles=_portal_tiles(),
                body_full="\n".join(f"seed {i}" for i in range(5)),
                transcript=transcript,
                duration_s=0.8,
                interval_ms=250,
                window_lines=3,
                mutation_lock=asyncio.Lock(),
            )
            return transcript

        portal.publish_portal = fake_publish_portal
        portal.asyncio.sleep = fake_sleep
        portal.time.monotonic = fake_monotonic
        try:
            transcript = asyncio.run(run())
        finally:
            portal.publish_portal = original_publish
            portal.asyncio.sleep = original_sleep
            portal.time.monotonic = original_monotonic

        self.assertEqual(len(captured_bodies), 4)
        self.assertTrue(all(len(body) <= 3 for body in captured_bodies))
        self.assertAlmostEqual(sleeps[0], 0.18, places=6)
        self.assertTrue(all(sleep_s >= 0.0 for sleep_s in sleeps))
        completed = [step for step in transcript if step["status"] == "completed"][-1]
        self.assertEqual(completed["cycles"], 4)

    def test_soak_phase_extends_initial_lease_ttl(self) -> None:
        self.assertEqual(
            portal.scenario_lease_ttl_ms("baseline,soak", baseline_hold_s=20.0, soak_duration_s=3600.0),
            3_720_000,
        )
        self.assertEqual(
            portal.scenario_lease_ttl_ms("baseline,scroll", baseline_hold_s=20.0, soak_duration_s=3600.0),
            600_000,
        )


class _InteractionCleanupClient:
    def __init__(self) -> None:
        self._event_queue: asyncio.Queue = asyncio.Queue()

    async def submit_mutation_batch(self, lease_id, mutations, timeout=5.0):
        class _Result:
            accepted = True
            error_code = 0
            error_message = ""
            batch_id = b""
            created_ids: list[bytes] = []

        return _Result()


class _InteractionCleanupErrorClient(_InteractionCleanupClient):
    def __init__(self) -> None:
        super().__init__()
        self.render_started = asyncio.Event()

    async def submit_mutation_batch(self, lease_id, mutations, timeout=5.0):
        self.render_started.set()
        try:
            await asyncio.Event().wait()
        except asyncio.CancelledError as exc:
            raise RuntimeError("composer render cleanup failed") from exc


def _pending_portal_background_tasks() -> list[asyncio.Task]:
    names = {
        "portal_interaction_loop.<locals>.composer_blink_worker",
        "portal_interaction_loop.<locals>.composer_render_worker",
    }
    return [
        task for task in asyncio.all_tasks()
        if not task.done() and task.get_coro().__qualname__ in names
    ]


class InteractionCleanupTests(unittest.TestCase):
    def test_cancelled_interaction_loop_cancels_composer_background_tasks(self) -> None:
        async def run() -> None:
            client = _InteractionCleanupClient()
            tiles = _portal_tiles()
            old_blink_seconds = portal.COMPOSER_CARET_BLINK_SECONDS
            portal.COMPOSER_CARET_BLINK_SECONDS = 60.0
            loop_task = asyncio.create_task(
                portal.portal_interaction_loop(
                    client=client,
                    lease_id=b"lease",
                    tiles=tiles,
                    transcript=[],
                    body_full="body",
                    initial_portal_x=0.0,
                    initial_portal_y=0.0,
                    tab_width=portal.PORTAL_W,
                    tab_height=portal.PORTAL_H,
                    mutation_lock=asyncio.Lock(),
                    clipboard_host="windows-host.example",
                    clipboard_user="hud-user",
                    clipboard_ssh_key="/tmp/no-key",
                    clipboard_timeout_s=0.1,
                )
            )
            try:
                await client._event_queue.put(
                    portal.events_pb2.EventBatch(
                        events=[
                            portal.events_pb2.InputEnvelope(
                                focus_gained=portal.events_pb2.FocusGainedEvent(
                                    tile_id=tiles.input_scroll,
                                )
                            )
                        ]
                    )
                )
                for _ in range(20):
                    if _pending_portal_background_tasks():
                        break
                    await asyncio.sleep(0)
                self.assertTrue(
                    _pending_portal_background_tasks(),
                    "focus should arm a composer background task before cancellation",
                )

                loop_task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await loop_task
                await asyncio.sleep(0)

                self.assertEqual(
                    _pending_portal_background_tasks(),
                    [],
                    "interaction-loop cancellation must cancel detached composer tasks",
                )
            finally:
                portal.COMPOSER_CARET_BLINK_SECONDS = old_blink_seconds
                loop_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await loop_task
                for task in _pending_portal_background_tasks():
                    task.cancel()
                await asyncio.sleep(0)

        asyncio.run(run())

    def test_cancelled_interaction_loop_surfaces_composer_cleanup_errors(self) -> None:
        async def run() -> None:
            client = _InteractionCleanupErrorClient()
            tiles = _portal_tiles()
            old_blink_seconds = portal.COMPOSER_CARET_BLINK_SECONDS
            old_debounce_seconds = portal.COMPOSER_RENDER_DEBOUNCE_SECONDS
            old_runtime_node_ids = dict(portal.COMPOSER_RUNTIME_NODE_IDS)
            portal.COMPOSER_CARET_BLINK_SECONDS = 60.0
            portal.COMPOSER_RENDER_DEBOUNCE_SECONDS = 0.0
            portal.COMPOSER_RUNTIME_NODE_IDS.clear()
            portal.COMPOSER_RUNTIME_NODE_IDS.update(
                {
                    **{
                        key: f"{key}-node".encode("utf-8")
                        for key in portal.COMPOSER_LINE_KEYS
                    },
                    "caret": b"caret-node",
                }
            )
            loop_task = asyncio.create_task(
                portal.portal_interaction_loop(
                    client=client,
                    lease_id=b"lease",
                    tiles=tiles,
                    transcript=[],
                    body_full="body",
                    initial_portal_x=0.0,
                    initial_portal_y=0.0,
                    tab_width=portal.PORTAL_W,
                    tab_height=portal.PORTAL_H,
                    mutation_lock=asyncio.Lock(),
                    clipboard_host="windows-host.example",
                    clipboard_user="hud-user",
                    clipboard_ssh_key="/tmp/no-key",
                    clipboard_timeout_s=0.1,
                )
            )
            try:
                await client._event_queue.put(
                    portal.events_pb2.EventBatch(
                        events=[
                            portal.events_pb2.InputEnvelope(
                                focus_gained=portal.events_pb2.FocusGainedEvent(
                                    tile_id=tiles.input_scroll,
                                )
                            )
                        ]
                    )
                )
                await asyncio.wait_for(client.render_started.wait(), timeout=1.0)

                loop_task.cancel()
                with self.assertRaisesRegex(RuntimeError, "composer render cleanup failed"):
                    await loop_task

                self.assertEqual(
                    _pending_portal_background_tasks(),
                    [],
                    "failed composer cleanup must not leak detached tasks",
                )
            finally:
                portal.COMPOSER_CARET_BLINK_SECONDS = old_blink_seconds
                portal.COMPOSER_RENDER_DEBOUNCE_SECONDS = old_debounce_seconds
                portal.COMPOSER_RUNTIME_NODE_IDS.clear()
                portal.COMPOSER_RUNTIME_NODE_IDS.update(old_runtime_node_ids)
                loop_task.cancel()
                with contextlib.suppress(asyncio.CancelledError, Exception):
                    await loop_task
                for task in _pending_portal_background_tasks():
                    task.cancel()
                await asyncio.sleep(0)

        asyncio.run(run())


# ── Steady-state publish atomicity (hud-ooeam flicker fix) ────────────────────


class _RecordingClient:
    """Minimal fake HudClient that records every mutation batch.

    All higher-level helpers funnel through ``submit_mutation_batch`` exactly as
    the real client does, so recording that one entry point captures the full
    mutation stream (set_tile_root / add_node / update_node_content / …) plus
    which tile each touched.
    """

    def __init__(self) -> None:
        self.batches: list[dict] = []
        self._next_id = 0

    def _alloc_id(self) -> bytes:
        self._next_id += 1
        return self._next_id.to_bytes(16, "big")

    async def submit_mutation_batch(self, lease_id, mutations, timeout=5.0):
        created: list[bytes] = []
        for m in mutations:
            kind = m.WhichOneof("mutation")
            tile_id = b""
            field = getattr(m, kind)
            if hasattr(field, "tile_id"):
                tile_id = field.tile_id
            elif kind == "publish_to_tile":
                tile_id = field.element_id
            self.batches.append({"kind": kind, "tile_id": tile_id})
            # set_tile_root / add_node / create_tile produce a server id.
            if kind in ("set_tile_root", "add_node", "create_tile"):
                created.append(self._alloc_id())

        class _Result:
            accepted = True
            error_code = 0
            error_message = ""

        result = _Result()
        result.batch_id = b""
        result.created_ids = created
        return result

    async def add_node(self, lease_id, tile_id, node, parent_id=None):
        mr = await self.submit_mutation_batch(
            lease_id,
            [portal.types_pb2.MutationProto(
                add_node=portal.types_pb2.AddNodeMutation(
                    tile_id=tile_id, parent_id=parent_id or b"", node=node,
                ),
            )],
        )
        return mr.created_ids[0]

    async def update_node_content(self, lease_id, tile_id, node_id, node):
        mut = portal.update_node_content_mutation(tile_id, node_id, node)
        await self.submit_mutation_batch(lease_id, [mut])

    async def update_tile_opacity(self, lease_id, tile_id, opacity):
        await self.submit_mutation_batch(
            lease_id,
            [portal.types_pb2.MutationProto(
                update_tile_opacity=portal.types_pb2.UpdateTileOpacityMutation(
                    tile_id=tile_id, opacity=opacity,
                ),
            )],
        )

    async def update_tile_input_mode(self, lease_id, tile_id, input_mode):
        await self.submit_mutation_batch(
            lease_id,
            [portal.types_pb2.MutationProto(
                update_tile_input_mode=portal.types_pb2.UpdateTileInputModeMutation(
                    tile_id=tile_id, input_mode=input_mode,
                ),
            )],
        )


def _portal_tiles() -> "portal.PortalTiles":
    ids = [i.to_bytes(16, "big") for i in range(900, 906)]
    return portal.PortalTiles(
        capture_backstop=ids[0],
        frame=ids[1],
        input_scroll=ids[2],
        output_scroll=ids[3],
        drag_shield=ids[4],
        minimized_icon=ids[5],
        tab_width=portal.PORTAL_W,
        tab_height=portal.PORTAL_H,
    )


class SteadyStatePublishAtomicityTests(unittest.TestCase):
    """Lock in the hud-ooeam flicker fix at the mutation-stream contract level.

    The live flicker came from the steady-state publish tearing the output-body
    tile down (set_tile_root → empties the tile) and re-adding children via
    separate add_node RPCs/commits, so the render thread sampled an empty body
    between commits. The fix updates the body text node in place via a single
    atomic update_node_content batch. These tests assert that contract without a
    live HUD.
    """

    def _setup_then_steady(self):
        tiles = _portal_tiles()
        client = _RecordingClient()
        lease = b"lease"
        body0 = "line 0\nline 1\nline 2"
        body1 = "line 0\nline 1\nline 2\nline 3"

        async def run():
            # Reset module-level runtime-id registries for isolation.
            portal.FRAME_RUNTIME_NODE_IDS.clear()
            portal.COMPOSER_RUNTIME_NODE_IDS.clear()
            portal.OUTPUT_RUNTIME_NODE_IDS.clear()
            # First publish: full tile setup (mounts + captures runtime ids).
            await portal.publish_portal(
                client, lease, tiles,
                title="T", subtitle="S", body=body0,
                footer_meta="f0", include_tile_setup=True,
            )
            setup_batches = list(client.batches)
            client.batches.clear()
            # Steady-state publish: append one line, republish window.
            await portal.publish_portal(
                client, lease, tiles,
                title="T", subtitle="S", body=body1,
                footer_meta="f1", include_tile_setup=False,
            )
            return setup_batches, list(client.batches)

        return asyncio.run(run()), tiles

    def test_steady_state_output_body_updates_in_place_not_torn_down(self) -> None:
        (setup_batches, steady_batches), tiles = self._setup_then_steady()

        out = tiles.output_scroll
        # Setup must have mounted the output tile via a tile-root + add_node(s).
        self.assertTrue(
            any(b["kind"] == "set_tile_root" and b["tile_id"] == out for b in setup_batches),
            "setup should mount the output tile root",
        )
        # The captured body id must exist after setup.
        self.assertIn("body", portal.OUTPUT_RUNTIME_NODE_IDS)

        # Steady state: NO teardown/rebuild of the output tile.
        out_set_root = [b for b in steady_batches if b["kind"] == "set_tile_root" and b["tile_id"] == out]
        out_add_node = [b for b in steady_batches if b["kind"] == "add_node" and b["tile_id"] == out]
        self.assertEqual(out_set_root, [], "steady-state must not reset the output tile root")
        self.assertEqual(out_add_node, [], "steady-state must not re-add output children")

        # Steady state MUST update the body content in place.
        out_updates = [b for b in steady_batches if b["kind"] == "update_node_content" and b["tile_id"] == out]
        self.assertTrue(out_updates, "steady-state must update the output body in place")

    def test_steady_state_frame_chrome_updates_in_place_not_torn_down(self) -> None:
        (_setup, steady_batches), tiles = self._setup_then_steady()
        frame = tiles.frame
        frame_set_root = [b for b in steady_batches if b["kind"] == "set_tile_root" and b["tile_id"] == frame]
        frame_add_node = [b for b in steady_batches if b["kind"] == "add_node" and b["tile_id"] == frame]
        self.assertEqual(frame_set_root, [], "steady-state must not reset the frame tile root")
        self.assertEqual(frame_add_node, [], "steady-state must not re-add frame children")
        frame_updates = [b for b in steady_batches if b["kind"] == "update_node_content" and b["tile_id"] == frame]
        self.assertTrue(frame_updates, "steady-state must update frame chrome in place")

    def test_steady_state_remounts_when_output_ids_invalidated(self) -> None:
        # If the captured output ids were invalidated (e.g. the interactive
        # scroll path tore the tile down), a steady-state publish must fall back
        # to a full re-mount (set_tile_root) rather than issuing an in-place
        # update against a node id that no longer exists.
        tiles = _portal_tiles()
        client = _RecordingClient()
        lease = b"lease"

        async def run():
            portal.FRAME_RUNTIME_NODE_IDS.clear()
            portal.COMPOSER_RUNTIME_NODE_IDS.clear()
            portal.OUTPUT_RUNTIME_NODE_IDS.clear()
            await portal.publish_portal(
                client, lease, tiles,
                title="T", subtitle="S", body="a\nb",
                footer_meta="f0", include_tile_setup=True,
            )
            # Simulate the scroll-path teardown invalidation.
            portal.OUTPUT_RUNTIME_NODE_IDS.clear()
            client.batches.clear()
            await portal.publish_portal(
                client, lease, tiles,
                title="T", subtitle="S", body="a\nb\nc",
                footer_meta="f1", include_tile_setup=False,
            )
            return list(client.batches)

        steady_batches = asyncio.run(run())
        out = tiles.output_scroll
        out_set_root = [b for b in steady_batches if b["kind"] == "set_tile_root" and b["tile_id"] == out]
        self.assertTrue(out_set_root, "must re-mount the output tile when ids are invalidated")
        # And the re-mount must re-capture the body id for subsequent cycles.
        self.assertIn("body", portal.OUTPUT_RUNTIME_NODE_IDS)


if __name__ == "__main__":
    unittest.main()
