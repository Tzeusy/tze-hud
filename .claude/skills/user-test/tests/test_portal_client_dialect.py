"""Regression tests for the preferred portal client's MCP wire dialect."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from unittest import mock


CLIENT_PATH = (
    Path(__file__).parents[2] / "hud-projection" / "scripts" / "portal_client.py"
)
SPEC = importlib.util.spec_from_file_location("portal_client_dialect", CLIENT_PATH)
assert SPEC is not None and SPEC.loader is not None
portal_client = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(portal_client)


def tools_call_result(result: dict, *, is_error: bool = False) -> dict:
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "content": [{"type": "text", "text": json.dumps(result)}],
            "isError": is_error,
        },
    }


def test_standard_tools_call_is_primary_and_unwrapped_for_callers() -> None:
    calls: list[tuple[str, dict]] = []

    def fake_rpc(method: str, params: dict) -> dict:
        calls.append((method, params))
        if method == "tools/call":
            return tools_call_result({"accepted": True, "status_summary": "attached"})
        return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True}}

    with mock.patch.object(portal_client, "rpc", side_effect=fake_rpc):
        response = portal_client.call_tool(
            "portal_projection_attach",
            {"operation": "attach", "projection_id": "dialect-test"},
        )

    assert [method for method, _ in calls] == ["tools/call"]
    assert calls[0][1]["name"] == "portal_projection_attach"
    assert response["result"]["accepted"] is True


def test_method_not_found_falls_back_to_legacy_bare_method() -> None:
    calls: list[str] = []

    def fake_rpc(method: str, params: dict) -> dict:
        calls.append(method)
        if method == "tools/call":
            return {
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32601, "message": "Method not found: tools/call"},
            }
        return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True}}

    with mock.patch.object(portal_client, "rpc", side_effect=fake_rpc):
        response = portal_client.call_tool(
            "portal_projection_publish",
            {"operation": "publish_output", "projection_id": "dialect-test"},
        )

    assert calls == ["tools/call", "portal_projection_publish"]
    assert response["result"]["accepted"] is True


def test_standard_tool_execution_error_does_not_trigger_legacy_retry() -> None:
    calls: list[str] = []

    def fake_rpc(method: str, params: dict) -> dict:
        calls.append(method)
        return {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [{"type": "text", "text": "projection is detached"}],
                "isError": True,
            },
        }

    with mock.patch.object(portal_client, "rpc", side_effect=fake_rpc):
        response = portal_client.call_tool(
            "portal_projection_publish",
            {"operation": "publish_output", "projection_id": "dialect-test"},
        )

    assert calls == ["tools/call"]
    assert response["error"]["message"] == "projection is detached"
