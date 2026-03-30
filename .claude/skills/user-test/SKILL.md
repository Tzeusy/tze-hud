---
name: user-test
description: Use when validating a cross-machine HUD flow where Butler deploys/runs the full Windows app over SSH+SCP (tailnet default host), then publishes configurable test messages to HUD zones via MCP `publish_to_zone`.
---

# User Test

Run an automation-first cross-machine validation loop:

1. Deploy a full Windows application `.exe` from Linux.
2. Launch it on Windows over SSH/SCP.
3. Verify MCP HTTP is reachable.
4. Publish configurable zone test messages.

## Required Inputs

Collect these before executing:

- `full_app_exe` (required): absolute/relative Linux path to the full application `.exe` to deploy
- `package` (optional fallback): cargo package/bin crate to build only if explicitly requested
- `target` (default: `x86_64-pc-windows-gnu`)
- `profile` (`release` or `debug`, default: `release`)
- `win_user` (default: `hudbot`)
- `win_host` (default: `tzehouse-windows.parrot-hen.ts.net`)
- `ssh_key_path` (default in local environment: `~/.ssh/ecdsa_home`)
- `task_name` (default: `TzeHudOverlay`)
- `mcp_http_url` (default: `http://tzehouse-windows.parrot-hen.ts.net:9090`)
- `mcp_psk_env` (default: `MCP_TEST_PSK`)
- `messages`: array of zone publishes

Message shape — `content` is either a plain string (StreamText) or a typed JSON object:

```json
[
  {
    "zone_name": "alert-banner",
    "content": "Deploy v2.1.0 started",
    "ttl_us": 30000000,
    "namespace": "butler-test"
  },
  {
    "zone_name": "subtitle",
    "content": "Running integration tests...",
    "ttl_us": 60000000
  },
  {
    "zone_name": "status-bar",
    "content": {"type": "status_bar", "entries": {"build": "passing", "agent": "butler", "target": "windows"}},
    "merge_key": "build-status",
    "ttl_us": 120000000,
    "namespace": "butler-test"
  },
  {
    "zone_name": "notification-area",
    "content": {"type": "notification", "text": "Build complete", "icon": "", "urgency": 1},
    "ttl_us": 10000000
  },
  {
    "zone_name": "ambient-background",
    "content": {"type": "solid_color", "r": 0.05, "g": 0.1, "b": 0.2, "a": 1.0},
    "ttl_us": 300000000
  },
  {
    "zone_name": "pip",
    "content": {"type": "solid_color", "r": 0.2, "g": 0.8, "b": 0.2, "a": 0.9},
    "ttl_us": 60000000
  }
]
```

**Content types by zone:**
- `alert-banner`, `subtitle`: plain string (StreamText)
- `status-bar`: `{"type":"status_bar","entries":{"key":"value",...}}` with `merge_key`
- `notification-area`: `{"type":"notification","text":"...","icon":"","urgency":0-3}`
- `ambient-background`, `pip`: `{"type":"solid_color","r":0-1,"g":0-1,"b":0-1,"a":0-1}`

`merge_key`, `ttl_us`, and `namespace` are optional per message.

## Workflow

### Step 0: SSH Connectivity Gate

Verify key auth first (Linux):

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Do not continue until this exact check succeeds for `hudbot`.

### Step 1: Deploy + Launch Full App (Linux)

Preferred: deploy a prebuilt full app `.exe`:

```bash
WIN_USER=hudbot \
SSH_OPTS='-i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null' \
.claude/skills/user-test/scripts/deploy_windows_hud.sh \
  --win-host tzehouse-windows.parrot-hen.ts.net \
  --full-app-exe /path/to/full-app.exe \
  --launch-mode auto \
  --tail
```

Fallback only when explicitly requested:

```bash
.claude/skills/user-test/scripts/deploy_windows_hud.sh --package <pkg> ...
```

`deploy_windows_hud.sh` defaults to `WIN_USER=hudbot` and `--launch-mode auto`.
In `auto`, it tries scheduled-task launch first, then falls back to direct `run_hud.ps1` launch if task APIs are unavailable in the `hudbot` SSH context.

If needed, force/skip task bootstrap:

```bash
# force bootstrap behavior (default)
.claude/skills/user-test/scripts/deploy_windows_hud.sh --bootstrap-task ...

# fail if task trigger fails
.claude/skills/user-test/scripts/deploy_windows_hud.sh --no-bootstrap-task ...

# launch mode override
.claude/skills/user-test/scripts/deploy_windows_hud.sh --launch-mode direct ...
```

Then report:

- artifact path
- file type (`file ...`)
- checksum (`sha256sum ...`)
- remote path (`C:\tze_hud\<exe-name>.exe`)
- latest launcher/stdout/stderr log lines

### Step 2: MCP Reachability Gate

Require live MCP HTTP reachability before publish.

- Default URL: `http://tzehouse-windows.parrot-hen.ts.net:9090`
- If MCP HTTP is unreachable, stop and report launch/runtime mismatch.
- Do not treat startup subtitle simulation as a substitute for MCP publish validation.

### Step 3: Publish Configurable Zone Messages

Use `scripts/publish_zone_batch.py` from this skill.

Recommended sequence:

1. Generate a temporary JSON file with user-provided messages.
2. List zones first (`--list-zones`) for visibility.
3. Publish the full message batch.
4. Return per-message results (success/failure, ids, and errors).

Example:

```bash
python3 .claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file /tmp/hud-zone-messages.json \
  --list-zones
```

## Behavior Rules

- Use automation-first deploy/launch by default.
- Default to full app deployment via `--full-app-exe`.
- Use `--package` only when explicitly requested.
- Always pass non-interactive SSH/SCP flags (`BatchMode`, `IdentitiesOnly`) for automation safety.
- Default the automation account to `hudbot`; do not route through `tzeus` unless explicitly requested.
- Prefer `--launch-mode auto` so `hudbot` can continue via direct launch when scheduled-task APIs are denied.
- Require real MCP HTTP reachability before claiming publish-path success.
- Never invent zone names or endpoint values; require user-provided values.
- Treat any publish error as actionable; include exact response payload.
- Keep messages configurable from the user prompt; do not hardcode content.
- Assume Windows host defaults to `tzehouse-windows.parrot-hen.ts.net` unless user overrides.
