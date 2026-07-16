#!/usr/bin/env python3
"""Production-framing contract tests for the canonical token-footprint driver."""

import importlib.util
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

    def test_portal_requests_include_production_operation_discriminator(self):
        captured = []

        def fake_call_tool(method, params):
            captured.append((method, params.copy()))
            return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True}}

        with mock.patch.object(flow.portal_client, "call_tool", fake_call_tool):
            for method in (
                "portal_projection_attach",
                "portal_projection_publish",
                "portal_projection_get_pending_input",
                "portal_projection_acknowledge_input",
            ):
                flow.invoke_portal_tool(method, {"projection_id": "fixture"})

        self.assertEqual(
            [params["operation"] for _, params in captured],
            ["attach", "publish_output", "get_pending_input", "acknowledge_input"],
        )


if __name__ == "__main__":
    unittest.main()
