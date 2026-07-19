#!/usr/bin/env python3
"""Production-framing contract tests for the canonical token-footprint driver."""

import importlib.util
import io
import json
import os
import pathlib
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[2]
os.environ.setdefault(
    "PORTAL_CLIENT_PATH",
    str(ROOT / ".claude/skills/hud-projection/scripts/portal_client.py"),
)
SCRIPT = ROOT / "examples/benchmark/token_footprint_flow.py"
SPEC = importlib.util.spec_from_file_location("token_footprint_flow", SCRIPT)
flow = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(flow)


class CanonicalFlowFramingTests(unittest.TestCase):
    def test_zone_and_widget_use_standard_mcp_tools_call_framing(self):
        calls = []

        def fake_recording_rpc(method, params, transaction_method=None):
            calls.append((method, params, transaction_method))
            result = {"accepted": True}
            return {
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [{"type": "text", "text": json.dumps(result)}],
                    "isError": False,
                },
            }

        with mock.patch.object(flow, "recording_rpc", fake_recording_rpc):
            self.assertEqual(
                flow.invoke_mcp_tool("publish_to_zone", {"zone_name": "notification-area"}),
                {"accepted": True},
            )
            self.assertEqual(
                flow.invoke_mcp_tool("publish_to_widget", {"widget_name": "gauge"}),
                {"accepted": True},
            )

        self.assertEqual([call[0] for call in calls], ["tools/call", "tools/call"])
        self.assertEqual(
            [call[1]["name"] for call in calls],
            ["publish_to_zone", "publish_to_widget"],
        )
        self.assertEqual(
            [call[2] for call in calls],
            ["publish_to_zone", "publish_to_widget"],
        )

    def test_portal_requests_pin_bare_method_compatibility_frame(self):
        captured = []

        def unexpected_call_tool(method, params):
            raise AssertionError(
                "canonical portal calibration must not follow call_tool dialect policy"
            )

        def fake_rpc(method, params):
            captured.append((method, params.copy()))
            return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True}}

        with (
            mock.patch.object(flow.portal_client, "call_tool", unexpected_call_tool),
            mock.patch.object(flow.portal_client, "rpc", fake_rpc),
        ):
            for method in (
                "portal_projection_attach",
                "portal_projection_publish",
                "portal_projection_get_pending_input",
                "portal_projection_acknowledge_input",
            ):
                flow.invoke_portal_tool(method, {"projection_id": "fixture"})

        self.assertEqual(
            [method for method, _ in captured],
            [
                "portal_projection_attach",
                "portal_projection_publish",
                "portal_projection_get_pending_input",
                "portal_projection_acknowledge_input",
            ],
        )
        self.assertEqual(
            [params["operation"] for _, params in captured],
            ["attach", "publish_output", "get_pending_input", "acknowledge_input"],
        )

    def test_piggyback_candidate_turn_does_not_issue_an_explicit_poll(self):
        portal_calls = []

        def fake_portal_tool(method, params):
            portal_calls.append((method, params.copy()))
            if method == "portal_projection_attach":
                return {"owner_token": "fixture-owner-token"}
            if method == "portal_projection_publish":
                return {
                    "accepted": True,
                    "pending_input": {
                        "items": [
                            {
                                "input_id": "fixture-input-0001",
                                "content": "Canonical HUD-originated input.",
                            }
                        ],
                        "remaining_count": 0,
                    },
                }
            if method == "portal_projection_get_pending_input":
                raise AssertionError("candidate v2 must not issue an explicit poll")
            if method == "portal_projection_acknowledge_input":
                return {"accepted": True}
            raise AssertionError(f"unexpected portal operation: {method}")

        with (
            mock.patch.object(flow, "MODE", flow.MODE_PIGGYBACK_CANDIDATE_V2),
            mock.patch.object(flow, "transactions", []),
            mock.patch.object(flow, "invoke_portal_tool", fake_portal_tool),
            mock.patch.object(flow, "invoke_mcp_tool", return_value={"accepted": True}),
            mock.patch.object(flow.sys, "stdout", io.StringIO()),
        ):
            flow.main()

        self.assertEqual(
            [method for method, _ in portal_calls],
            [
                "portal_projection_attach",
                "portal_projection_publish",
                "portal_projection_acknowledge_input",
            ],
        )
        self.assertTrue(portal_calls[1][1]["expects_reply"])


if __name__ == "__main__":
    unittest.main()
