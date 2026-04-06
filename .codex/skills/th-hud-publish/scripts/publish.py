#!/usr/bin/env python3
"""
Publish MCP zone messages to a running tze_hud instance.

Usage:
  # List zones
  publish.py --url http://host:9090 --psk-env HUD_MCP_PSK --list-zones

  # Single inline publish (string content)
  publish.py --url http://host:9090 --zone alert-banner --content "Hello"

  # Single inline publish (typed content)
  publish.py --url http://host:9090 --zone status-bar \
    --content '{"type":"status_bar","entries":{"build":"passing"}}' \
    --merge-key build-status

  # Batch publish from file
  publish.py --url http://host:9090 --messages-file msgs.json

Content formats:
  - Plain string: "Hello world" → StreamText
  - Typed object (JSON): {"type":"notification","text":"Done!","urgency":1}
  - Types: stream_text, notification, status_bar, solid_color

Only zone_name and content are required per message.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from typing import Any


def rpc_call(
    url: str, token: str, method: str, params: dict[str, Any], request_id: int
) -> dict[str, Any]:
    """Send a single JSON-RPC 2.0 request and return the parsed response."""
    body = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
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
        return json.loads(resp.read().decode("utf-8"))


def parse_content(raw: str) -> Any:
    """Parse content: try JSON object first, fall back to plain string."""
    stripped = raw.strip()
    if stripped.startswith("{"):
        try:
            obj = json.loads(stripped)
            if isinstance(obj, dict):
                return obj
        except json.JSONDecodeError:
            pass
    return raw


def load_messages(path: str) -> list[dict[str, Any]]:
    """Load and validate a JSON array of publish messages."""
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)
    if not isinstance(data, list):
        raise ValueError("messages file must be a JSON array")
    for idx, item in enumerate(data):
        if not isinstance(item, dict):
            raise ValueError(f"message[{idx}] must be an object")
        if not isinstance(item.get("zone_name"), str) or not item["zone_name"].strip():
            raise ValueError(f"message[{idx}].zone_name must be a non-empty string")
        content = item.get("content")
        if content is None:
            raise ValueError(f"message[{idx}].content is required")
        if isinstance(content, str) and not content:
            raise ValueError(f"message[{idx}].content must be non-empty")
        if isinstance(content, dict) and "type" not in content:
            raise ValueError(f"message[{idx}].content object must have a \"type\" field")
    return data


def publish_messages(
    url: str, token: str, messages: list[dict[str, Any]]
) -> tuple[list[dict[str, Any]], bool]:
    """Publish a list of messages and return (results, any_failed)."""
    results: list[dict[str, Any]] = []
    any_failed = False
    req_id = 10

    for msg in messages:
        params: dict[str, Any] = {
            "zone_name": msg["zone_name"],
            "content": msg["content"],
        }
        if "ttl_us" in msg:
            params["ttl_us"] = int(msg["ttl_us"])
        if "merge_key" in msg:
            params["merge_key"] = msg["merge_key"]
        if "namespace" in msg:
            params["namespace"] = msg["namespace"]

        response = rpc_call(url, token, "publish_to_zone", params, req_id)
        ok = "error" not in response
        if not ok:
            any_failed = True
        results.append(
            {
                "request_id": req_id,
                "zone_name": params["zone_name"],
                "ok": ok,
                "response": response,
            }
        )
        req_id += 1

    return results, any_failed


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Publish zone messages to a tze_hud MCP endpoint"
    )
    parser.add_argument(
        "--url", required=True, help="MCP HTTP URL (e.g. http://host:9090)"
    )
    parser.add_argument(
        "--psk-env",
        default="HUD_MCP_PSK",
        help="Environment variable containing the pre-shared key (default: HUD_MCP_PSK)",
    )
    parser.add_argument(
        "--list-zones",
        action="store_true",
        help="Call list_zones and print results",
    )

    # Batch mode
    parser.add_argument(
        "--messages-file", help="Path to JSON array of message objects"
    )

    # Inline single-publish mode
    parser.add_argument(
        "--zone", help="Zone name for inline single publish"
    )
    parser.add_argument(
        "--content",
        help="Content for inline publish: plain string or JSON object string",
    )
    parser.add_argument(
        "--merge-key", help="Merge key for inline publish (MergeByKey zones)"
    )
    parser.add_argument(
        "--ttl-us", type=int, help="TTL in microseconds for inline publish"
    )
    parser.add_argument(
        "--namespace", help="Namespace for inline publish"
    )

    args = parser.parse_args()

    has_inline = args.zone or args.content
    if not args.list_zones and not args.messages_file and not has_inline:
        parser.error(
            "provide --list-zones, --messages-file, or --zone/--content"
        )
    if has_inline and not (args.zone and args.content):
        parser.error("--zone and --content must both be provided for inline publish")
    if has_inline and args.messages_file:
        parser.error("cannot combine --zone/--content with --messages-file")

    token = os.getenv(args.psk_env, "")
    if not token:
        print(
            f"ERROR: environment variable {args.psk_env} is empty or unset",
            file=sys.stderr,
        )
        return 2

    try:
        if args.list_zones:
            zones = rpc_call(args.url, token, "list_zones", {}, 1)
            print(json.dumps(zones, indent=2))

        if has_inline:
            msg: dict[str, Any] = {
                "zone_name": args.zone,
                "content": parse_content(args.content),
            }
            if args.ttl_us is not None:
                msg["ttl_us"] = args.ttl_us
            if args.merge_key is not None:
                msg["merge_key"] = args.merge_key
            if args.namespace is not None:
                msg["namespace"] = args.namespace

            results, any_failed = publish_messages(args.url, token, [msg])
            print(json.dumps({"published": results}, indent=2))
            if any_failed:
                return 1

        elif args.messages_file:
            messages = load_messages(args.messages_file)
            results, any_failed = publish_messages(args.url, token, messages)
            print(json.dumps({"published": results}, indent=2))
            if any_failed:
                return 1

        return 0

    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        print(
            json.dumps({"error": "http_error", "status": e.code, "body": body}),
            file=sys.stderr,
        )
        return 3
    except urllib.error.URLError as e:
        print(
            json.dumps({"error": "url_error", "detail": str(e)}),
            file=sys.stderr,
        )
        return 4
    except ValueError as e:
        print(
            json.dumps({"error": "validation_error", "detail": str(e)}),
            file=sys.stderr,
        )
        return 5
    except Exception as e:
        print(
            json.dumps({"error": "exception", "detail": str(e)}),
            file=sys.stderr,
        )
        return 6


if __name__ == "__main__":
    raise SystemExit(main())
