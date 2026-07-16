#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Deterministic CLI for the tze_hud `portal_projection_*` MCP facade.

One subcommand per projection operation, with owner-token custody handled for
you: the token returned by `attach` is written to a 0600 file OUTSIDE the repo
(default `~/.local/state/tze_hud/portal-tokens/<projection_id>.token`, override
with PORTAL_TOKEN_DIR) and is never printed — every response is recursively
redacted before it reaches stdout.

Environment:
  HUD_MCP_URL   MCP endpoint, with or without the /mcp suffix (required).
  HUD_PSK       bearer PSK; falls back to MCP_TEST_PSK, HUD_MCP_PSK,
                TZE_HUD_MCP_RESIDENT_PRINCIPAL (required via one of them).
  Resolve both with:  eval "$(.claude/skills/user-test/scripts/tzehouse_env.sh)"
  (or hud_vm_env.sh for the autonomous VM testhost).

Dialect: the runtime's MCP server dispatches tool names as bare JSON-RPC
methods and does not implement standard `tools/call` (bug hud-09emd). This
client tries the bare method first and transparently falls back to
`tools/call` so it keeps working when hud-09emd is fixed.

Subcommands:
  attach   --projection-id ID [--display-name S] [--provider-kind claude]
           [--workspace-hint S] [--repository-hint S] [--icon-profile S]
           [--classification private] [--idempotency-key S]
  publish  --projection-id ID (--text S | --text-file F | -)  [--kind assistant]
           [--coalesce-key S] [--logical-unit-id S]
  status   --projection-id ID --state active [--text S]
  poll     --projection-id ID [--wait-ms 30000] [--rounds 1] [--max-items 4]
           [--max-bytes 4096] [--ack handled|deferred|none] [--ack-message S]
           Prints one JSON object per received input item (NDJSON).
           Exit 0 = items received, 3 = no items (deterministic signal).
  ack      --projection-id ID --input-id I --state handled|deferred|rejected
           [--message S] [--not-before-us N]
  detach   --projection-id ID [--reason S]     (removes the token file)
  cleanup  --projection-id ID [--reason S]     (removes the token file)
  token-path --projection-id ID                (prints the token file path)

Exit codes: 0 success · 1 transport/config error · 2 operation rejected ·
3 poll returned no items.
"""

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request

TOKEN_DIR = os.environ.get("PORTAL_TOKEN_DIR") or os.path.join(
    os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state"),
    "tze_hud", "portal-tokens",
)

# Projection IDs become token filenames; reject anything not filename-safe
# BEFORE any RPC so a successful attach can never lose its one-time token to
# a failed save (and `..`/`/` can never escape the token directory).
PROJECTION_ID_SAFE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")


def die(msg, code=1):
    print(f"portal_client: ERROR — {msg}", file=sys.stderr)
    sys.exit(code)


def mcp_url():
    url = os.environ.get("HUD_MCP_URL") or die(
        "HUD_MCP_URL not set (eval tzehouse_env.sh or hud_vm_env.sh first)")
    return url if url.rstrip("/").endswith("/mcp") else url.rstrip("/") + "/mcp"


def psk():
    for var in ("HUD_PSK", "MCP_TEST_PSK", "HUD_MCP_PSK", "TZE_HUD_MCP_RESIDENT_PRINCIPAL"):
        if os.environ.get(var):
            return os.environ[var]
    die("no PSK in env (HUD_PSK / MCP_TEST_PSK / HUD_MCP_PSK / TZE_HUD_MCP_RESIDENT_PRINCIPAL)")


def token_path(projection_id):
    return os.path.join(TOKEN_DIR, f"{projection_id}.token")


def save_token(projection_id, token):
    os.makedirs(TOKEN_DIR, mode=0o700, exist_ok=True)
    path = token_path(projection_id)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(token)


def load_token(projection_id):
    path = token_path(projection_id)
    try:
        with open(path, encoding="utf-8") as f:
            return f.read().strip()
    except FileNotFoundError:
        die(f"no owner token on file at {path} — attach first; for a live projection, "
            "repeat attach with the original idempotency key to rotate ownership")
    except OSError as e:
        die(f"cannot read owner token at {path}: {e}")


def redact(node):
    """Strip owner_token from any response structure before it is printed."""
    if isinstance(node, dict):
        if "owner_token" in node:
            node["owner_token"] = "<REDACTED>"
        for v in node.values():
            redact(v)
    elif isinstance(node, list):
        for v in node:
            redact(v)
    return node


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(mcp_url(), data=body, headers={
        "Authorization": f"Bearer {psk()}",
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
    })
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            ctype = resp.headers.get("Content-Type", "")
    except urllib.error.HTTPError as e:
        die(f"HTTP {e.code} from {mcp_url()}: {e.read().decode('utf-8', errors='replace')[:400]}")
    except urllib.error.URLError as e:
        die(f"cannot reach {mcp_url()}: {e.reason}")
    if "text/event-stream" in ctype:
        lines = [l[5:].strip() for l in raw.splitlines() if l.startswith("data:")]
        raw = lines[-1] if lines else "{}"
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        die(f"malformed JSON from {mcp_url()}: {raw[:400]!r}")


def call_tool(tool, args):
    """Bare-method dialect first; tools/call fallback (hud-09emd)."""
    args.setdefault("client_timestamp_wall_us", int(time.time() * 1_000_000))
    args.setdefault("request_id", f"req-{tool}-{int(time.time() * 1000)}")
    resp = rpc(tool, args)
    if (resp.get("error") or {}).get("code") == -32601:
        resp = rpc("tools/call", {"name": tool, "arguments": args})
        content = resp.get("result", {}).get("content")
        if isinstance(content, list) and content and content[0].get("type") == "text":
            resp = {"jsonrpc": "2.0", "result": json.loads(content[0]["text"]), "id": resp.get("id")}
    return resp


def result_or_die(resp):
    if resp.get("error") is not None:
        print(json.dumps(redact(resp), indent=2))
        sys.exit(2)
    result = resp.get("result") or {}
    if result.get("accepted") is False:
        print(json.dumps(redact(result), indent=2))
        sys.exit(2)
    return result


def emit(result):
    print(json.dumps(redact(result), indent=2))


def cmd_attach(a):
    args = {
        "operation": "attach",
        "projection_id": a.projection_id,
        "provider_kind": a.provider_kind,
        "display_name": a.display_name or a.projection_id,
        "content_classification": a.classification,
        "hud_target": "default",
        "idempotency_key": a.idempotency_key
        or f"{a.projection_id}-{time.strftime('%Y%m%d')}",
    }
    if a.workspace_hint:
        args["workspace_hint"] = a.workspace_hint
    if a.repository_hint:
        args["repository_hint"] = a.repository_hint
    if a.icon_profile:
        args["icon_profile_hint"] = a.icon_profile
    resp = call_tool("portal_projection_attach", args)
    result = result_or_die(resp)
    token = result.get("owner_token")
    if token:
        save_token(a.projection_id, token)
    else:
        die("attach accepted without owner_token — protocol violation; the prior token "
            "must not be assumed valid after a replay", 2)
    result["token_file"] = token_path(a.projection_id)
    emit(result)


def cmd_publish(a):
    if a.text is not None:
        text = a.text
    elif a.text_file == "-":
        text = sys.stdin.read()
    elif a.text_file:
        with open(a.text_file, encoding="utf-8", errors="replace") as f:
            text = f.read()
    else:
        die("provide --text or --text-file (use '-' for stdin)")
    args = {
        "operation": "publish_output",
        "projection_id": a.projection_id,
        "owner_token": load_token(a.projection_id),
        "output_text": text,
        "output_kind": a.kind,
        "content_classification": a.classification,
    }
    if a.coalesce_key:
        args["coalesce_key"] = a.coalesce_key
    if a.logical_unit_id:
        args["logical_unit_id"] = a.logical_unit_id
    emit(result_or_die(call_tool("portal_projection_publish", args)))


def cmd_status(a):
    args = {
        "operation": "publish_status",
        "projection_id": a.projection_id,
        "owner_token": load_token(a.projection_id),
        "lifecycle_state": a.state,
    }
    if a.text:
        args["status_text"] = a.text
    emit(result_or_die(call_tool("portal_projection_publish_status", args)))


def do_ack(projection_id, input_id, state, message, not_before_us=None):
    args = {
        "operation": "acknowledge_input",
        "projection_id": projection_id,
        "owner_token": load_token(projection_id),
        "input_id": input_id,
        "ack_state": state,
    }
    if message:
        args["ack_message"] = message
    if not_before_us:
        args["not_before_wall_us"] = not_before_us
    return result_or_die(call_tool("portal_projection_acknowledge_input", args))


def cmd_poll(a):
    got = []
    for _ in range(a.rounds):
        result = result_or_die(call_tool("portal_projection_get_pending_input", {
            "operation": "get_pending_input",
            "projection_id": a.projection_id,
            "owner_token": load_token(a.projection_id),
            "max_items": a.max_items,
            "max_bytes": a.max_bytes,
            "wait_ms": min(a.wait_ms, 30000),
        }))
        items = result.get("items") or []
        for item in items:
            if a.ack != "none":
                item["ack"] = redact(do_ack(a.projection_id, item["input_id"], a.ack,
                                            a.ack_message))
            print(json.dumps(redact(item)))
        got.extend(items)
        if items:
            break
    if not got:
        print("portal_client: no pending input", file=sys.stderr)
        sys.exit(3)


def cmd_ack(a):
    emit(do_ack(a.projection_id, a.input_id, a.state, a.message, a.not_before_us))


def _terminal(a, op, tool, extra=None):
    args = {
        "operation": op,
        "projection_id": a.projection_id,
        "owner_token": load_token(a.projection_id),
        "reason": a.reason,
    }
    args.update(extra or {})
    result = result_or_die(call_tool(tool, args))
    try:
        os.remove(token_path(a.projection_id))
        result["token_file_removed"] = True
    except FileNotFoundError:
        pass
    emit(result)


def main():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    def base(sp):
        sp.add_argument("--projection-id", required=True)
        return sp

    sp = base(sub.add_parser("attach"))
    sp.add_argument("--display-name")
    sp.add_argument("--provider-kind", default="claude",
                    choices=["claude", "codex", "opencode", "other"])
    sp.add_argument("--workspace-hint")
    sp.add_argument("--repository-hint")
    sp.add_argument("--icon-profile")
    sp.add_argument("--classification", default="private")
    sp.add_argument("--idempotency-key")
    sp.set_defaults(fn=cmd_attach)

    sp = base(sub.add_parser("publish"))
    sp.add_argument("--text")
    sp.add_argument("--text-file")
    sp.add_argument("--kind", default="assistant",
                    choices=["assistant", "tool", "status", "error", "other"])
    sp.add_argument("--classification", default="private")
    sp.add_argument("--coalesce-key")
    sp.add_argument("--logical-unit-id")
    sp.set_defaults(fn=cmd_publish)

    sp = base(sub.add_parser("status"))
    sp.add_argument("--state", required=True,
                    choices=["attached", "active", "degraded", "hud_unavailable",
                             "detached", "cleanup_pending", "expired"])
    sp.add_argument("--text")
    sp.set_defaults(fn=cmd_status)

    sp = base(sub.add_parser("poll"))
    sp.add_argument("--wait-ms", type=int, default=30000)
    sp.add_argument("--rounds", type=int, default=1)
    sp.add_argument("--max-items", type=int, default=4)
    sp.add_argument("--max-bytes", type=int, default=4096)
    sp.add_argument("--ack", default="none", choices=["none", "handled", "deferred"])
    sp.add_argument("--ack-message", default="received")
    sp.set_defaults(fn=cmd_poll)

    sp = base(sub.add_parser("ack"))
    sp.add_argument("--input-id", required=True)
    sp.add_argument("--state", required=True, choices=["handled", "deferred", "rejected"])
    sp.add_argument("--message")
    sp.add_argument("--not-before-us", type=int)
    sp.set_defaults(fn=cmd_ack)

    sp = base(sub.add_parser("detach"))
    sp.add_argument("--reason", default="session complete")
    sp.set_defaults(fn=lambda a: _terminal(a, "detach", "portal_projection_detach"))

    sp = base(sub.add_parser("cleanup"))
    sp.add_argument("--reason", default="remove stale portal")
    # The MCP cleanup handler requires cleanup_authority; this client only
    # holds the owner token, so it always acts with owner authority
    # (operator cleanup uses separate daemon authority, out of scope here).
    sp.set_defaults(fn=lambda a: _terminal(a, "cleanup", "portal_projection_cleanup",
                                           {"cleanup_authority": "owner"}))

    sp = base(sub.add_parser("token-path"))
    sp.set_defaults(fn=lambda a: print(token_path(a.projection_id)))

    args = p.parse_args()
    if getattr(args, "projection_id", None) and not PROJECTION_ID_SAFE.match(args.projection_id):
        die(f"unsafe projection id {args.projection_id!r} — must match {PROJECTION_ID_SAFE.pattern} "
            "(it becomes the token filename)")
    args.fn(args)


if __name__ == "__main__":
    main()
