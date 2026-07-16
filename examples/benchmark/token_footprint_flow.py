#!/usr/bin/env python3
"""Drive canonical MCP flows through the production portal client transport."""

import importlib.util
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request


OWNER_TOKEN_SENTINEL = "<OWNER_TOKEN>"
CLIENT_TIMESTAMP = 1_700_000_000_000_000
transactions = []


def load_portal_client():
    path = pathlib.Path(os.environ["PORTAL_CLIENT_PATH"])
    spec = importlib.util.spec_from_file_location("portal_client", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


portal_client = load_portal_client()


def replace_owner_token(node):
    if isinstance(node, dict):
        return {
            key: OWNER_TOKEN_SENTINEL if key == "owner_token" else replace_owner_token(value)
            for key, value in node.items()
        }
    if isinstance(node, list):
        return [replace_owner_token(value) for value in node]
    return node


def recording_rpc(method, params):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    ).encode()
    request = urllib.request.Request(
        portal_client.mcp_url(),
        data=body,
        headers={
            "Authorization": f"Bearer {portal_client.psk()}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            raw = response.read()
    except (urllib.error.HTTPError, urllib.error.URLError) as error:
        raise RuntimeError(f"MCP transport failed: {error}") from error
    parsed = json.loads(raw)
    canonical_request = json.dumps(replace_owner_token(json.loads(body)))
    canonical_response = json.dumps(
        replace_owner_token(parsed), separators=(",", ":")
    )
    transactions.append(
        {
            "method": method,
            "request_body": canonical_request,
            "response_body": canonical_response,
        }
    )
    return parsed


portal_client.rpc = recording_rpc


def invoke(method, params):
    params["client_timestamp_wall_us"] = CLIENT_TIMESTAMP
    params["request_id"] = f"token-calibration-{method}"
    response = portal_client.call_tool(method, params)
    if response.get("error"):
        raise RuntimeError(f"{method} rejected: {response['error']}")
    return response["result"]


def main():
    invoke(
        "publish_to_zone",
        {
            "zone_name": "notification-area",
            "content": {
                "type": "notification",
                "title": "Calibration",
                "text": "Canonical zone calibration payload.",
                "icon": "info",
                "urgency": 1,
            },
            "namespace": "token-calibration-zone",
            "ttl_us": 60_000_000,
            "merge_key": "token-calibration-zone-0001",
        },
    )

    attach = invoke(
        "portal_projection_attach",
        {
            "projection_id": "token-calibration-portal",
            "display_name": "Token Calibration Portal",
            "idempotency_key": "token-calibration-attach-0001",
            "provider_kind": "codex",
            "content_classification": "private",
            "workspace_hint": "/workspace/tze_hud",
            "repository_hint": "tze_hud",
            "icon_profile_hint": "codex",
            "hud_target": "default",
        },
    )
    owner_token = attach["owner_token"]
    invoke(
        "portal_projection_publish",
        {
            "projection_id": "token-calibration-portal",
            "owner_token": owner_token,
            "output_text": "Canonical append-only portal payload.",
            "logical_unit_id": "token-calibration-output-0001",
            "output_kind": "assistant",
            "content_classification": "private",
            "expects_reply": True,
        },
    )
    pending = invoke(
        "portal_projection_get_pending_input",
        {
            "projection_id": "token-calibration-portal",
            "owner_token": owner_token,
            "max_items": 1,
            "max_bytes": 4096,
            "wait_ms": 1_000,
        },
    )
    input_id = pending["items"][0]["input_id"]
    invoke(
        "portal_projection_acknowledge_input",
        {
            "projection_id": "token-calibration-portal",
            "owner_token": owner_token,
            "input_id": input_id,
            "ack_state": "handled",
            "ack_message": "Canonical input handled.",
        },
    )

    invoke(
        "publish_to_widget",
        {
            "widget_name": "token-calibration-gauge",
            "params": {"level": 0.625},
            "transition_ms": 0,
            "namespace": "token-calibration-widget",
            "ttl_us": 60_000_000,
        },
    )
    json.dump({"transactions": transactions}, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
