#!/usr/bin/env python3
"""
Publish a batch of MCP `publish_to_widget` messages to a running HUD endpoint.

Message file format (JSON array):
[
  {
    "widget_name": "gauge",
    "params": {"level": 0.75, "label": "CPU Usage"},
    "transition_ms": 500,
    "ttl_us": 60000000,
    "namespace": "user-test"
  }
]

Also supports clear operations:
[
  {
    "action": "clear",
    "widget_name": "gauge",
    "namespace": "user-test"
  }
]
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from typing import Any


def rpc_call(url: str, token: str, method: str, params: dict[str, Any], request_id: int) -> dict[str, Any]:
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        url=url,
        data=body,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {token}",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=20) as resp:
        payload = resp.read().decode("utf-8")
    return json.loads(payload)


def load_messages(path: str) -> list[dict[str, Any]]:
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)
    if not isinstance(data, list):
        raise ValueError("messages file must be a JSON array")
    out: list[dict[str, Any]] = []
    for idx, item in enumerate(data):
        if not isinstance(item, dict):
            raise ValueError(f"message[{idx}] must be an object")
        action = item.get("action", "publish")
        widget_name = item.get("widget_name")
        if not isinstance(widget_name, str) or not widget_name.strip():
            raise ValueError(f"message[{idx}].widget_name must be a non-empty string")
        if action == "publish":
            params = item.get("params")
            if not isinstance(params, dict) or not params:
                raise ValueError(f"message[{idx}].params must be a non-empty object")
        elif action != "clear":
            raise ValueError(f"message[{idx}].action must be 'publish' or 'clear'")
        out.append(item)
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description="Publish MCP widget message batch")
    parser.add_argument("--url", required=True, help="MCP HTTP URL, e.g. http://host:9090")
    parser.add_argument("--psk-env", default="MCP_TEST_PSK", help="Environment variable containing PSK")
    parser.add_argument("--messages-file", required=True, help="Path to JSON array of widget message objects")
    parser.add_argument("--namespace", default="user-test", help="Default namespace if message namespace missing")
    parser.add_argument("--ttl-us", type=int, default=60_000_000, help="Default TTL in microseconds")
    parser.add_argument("--delay-ms", type=int, default=0, help="Delay between publishes")
    parser.add_argument("--list-widgets", action="store_true", help="Call list_widgets before publishing")
    args = parser.parse_args()

    token = os.getenv(args.psk_env, "")
    if not token:
        print(f"ERROR: env var {args.psk_env} is empty or unset", file=sys.stderr)
        return 2

    try:
        if args.list_widgets:
            widgets = rpc_call(args.url, token, "list_widgets", {}, 1)
            print(json.dumps({"list_widgets": widgets}, ensure_ascii=True))

        messages = load_messages(args.messages_file)
        results: list[dict[str, Any]] = []
        req_id = 10
        for msg in messages:
            action = msg.get("action", "publish")

            if action == "clear":
                params: dict[str, Any] = {
                    "widget_name": msg["widget_name"],
                    "namespace": msg.get("namespace", args.namespace),
                }
                if msg.get("instance_id") is not None:
                    params["instance_id"] = msg["instance_id"]
                response = rpc_call(args.url, token, "clear_widget", params, req_id)
            else:
                params = {
                    "widget_name": msg["widget_name"],
                    "params": msg["params"],
                    "namespace": msg.get("namespace", args.namespace),
                    "ttl_us": int(msg.get("ttl_us", args.ttl_us)),
                }
                if msg.get("transition_ms") is not None:
                    params["transition_ms"] = int(msg["transition_ms"])
                if msg.get("instance_id") is not None:
                    params["instance_id"] = msg["instance_id"]
                response = rpc_call(args.url, token, "publish_to_widget", params, req_id)

            results.append(
                {
                    "request_id": req_id,
                    "action": action,
                    "widget_name": msg["widget_name"],
                    "response": response,
                }
            )
            req_id += 1
            if args.delay_ms > 0:
                time.sleep(args.delay_ms / 1000.0)

        print(json.dumps({"published": results}, ensure_ascii=True))
        return 0
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        print(
            json.dumps(
                {
                    "error": "http_error",
                    "status": e.code,
                    "body": body,
                },
                ensure_ascii=True,
            ),
            file=sys.stderr,
        )
        return 3
    except urllib.error.URLError as e:
        print(json.dumps({"error": "url_error", "detail": str(e)}, ensure_ascii=True), file=sys.stderr)
        return 4
    except Exception as e:
        print(json.dumps({"error": "exception", "detail": str(e)}, ensure_ascii=True), file=sys.stderr)
        return 5


if __name__ == "__main__":
    raise SystemExit(main())
