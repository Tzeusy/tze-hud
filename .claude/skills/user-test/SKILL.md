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
5. Publish configurable widget test messages.

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

- `widget_messages`: array of widget publishes (optional)

Widget message shape:

```json
[
  {
    "widget_name": "gauge",
    "params": {"level": 0.75, "label": "CPU Usage"},
    "transition_ms": 500,
    "ttl_us": 60000000,
    "namespace": "user-test"
  },
  {
    "action": "clear",
    "widget_name": "gauge",
    "namespace": "user-test"
  }
]
```

**Widget parameter types:**
- `f32`: JSON number (e.g. `0.75`) — often with min/max range
- `string`: JSON string (e.g. `"CPU Usage"`)
- `color`: JSON object `{"r": 0-1, "g": 0-1, "b": 0-1, "a": 0-1}`
- `enum`: JSON string from allowed values (e.g. `"warning"`)

`transition_ms`, `ttl_us`, `namespace`, and `instance_id` are optional per message.

## Workflow

### Step 0: SSH Connectivity Gate

Verify key auth for **both** users (Linux):

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home \
  tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Both must succeed. `hudbot` is used for file deployment (SCP). `tzeus` is used for process control (kill, scheduled task trigger) because `tzeus` owns the interactive desktop session.

### Step 1: Deploy (SCP via hudbot)

Copy the prebuilt `.exe` to the Windows host:

```bash
# Kill any running instance first (must use tzeus — hudbot can't kill it)
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o StrictHostKeyChecking=no \
  tzeus@tzehouse-windows.parrot-hen.ts.net "taskkill /F /IM tze_hud.exe"
sleep 2

# SCP the exe (via hudbot)
scp -i ~/.ssh/ecdsa_home -o BatchMode=yes -o StrictHostKeyChecking=no \
  /path/to/tze_hud.exe \
  hudbot@tzehouse-windows.parrot-hen.ts.net:C:/tze_hud/tze_hud.exe
```

Report: file size, checksum (`sha256sum`), remote path.

### Step 2: Register + Launch (via tzeus)

The HUD **must** be launched via a scheduled task as `tzeus` with `--window-mode overlay`. This is critical for transparency — SSH-launched processes cannot access the desktop GPU, and `run_hud.ps1` wrappers interfere with window creation.

```bash
# Register the overlay task (idempotent — safe to re-run)
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o StrictHostKeyChecking=no \
  tzeus@tzehouse-windows.parrot-hen.ts.net \
  "powershell -NoProfile -Command \"Register-ScheduledTask -TaskName 'TzeHudOverlay' \
    -Action (New-ScheduledTaskAction \
      -Execute 'C:\\tze_hud\\tze_hud.exe' \
      -Argument '--window-mode overlay' \
      -WorkingDirectory 'C:\\tze_hud') \
    -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries) \
    -Force\""

# Launch it
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o StrictHostKeyChecking=no \
  tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Run /TN TzeHudOverlay"
```

**Transparency requirements** (if the window is grey/opaque, one of these is wrong):
- `--window-mode overlay` — fullscreen mode is intentionally opaque
- Task runs as `tzeus` (the user logged into the console desktop)
- Exe runs directly (no PowerShell/bat wrapper — wrapper windows break transparency)
- NVIDIA driver 595.97+ on the Windows host
- Commit must include `with_no_redirection_bitmap(true)`, Vulkan forcing, PreMultiplied alpha

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

### Step 4: Publish Configurable Widget Messages

Use `scripts/publish_widget_batch.py` from this skill.

Recommended sequence:

1. Generate a temporary JSON file with user-provided widget messages.
2. List widgets first (`--list-widgets`) to discover available widget types and instances.
3. Publish the full widget message batch.
4. Return per-message results (success/failure, applied params, and errors).

Example:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file /tmp/hud-widget-messages.json \
  --list-widgets
```

If `list_widgets` returns no instances, skip widget publishing and report that no widgets are registered (the HUD binary may predate widget support).

### Step 5: Widget Reactivity Test (Gauge Cycling)

After the initial widget publish, cycle the gauge through a sequence of values with 3-second delays to verify widget reactivity (re-rasterization on param change).

Use `scripts/gauge_cycle_test.json` from this skill with `--delay-ms 3000`:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/gauge_cycle_test.json \
  --delay-ms 3000
```

The gauge should visually cycle through: blue 25% "Low" → yellow 50% "Medium" → red 95% "Critical!" → green 42% "Normal". Report per-step success and whether the user confirmed visual updates.

### Step 6: Widget Reactivity Test (Status Indicator)

After the gauge cycle, run the status-indicator enum cycle to verify discrete color binding and re-rasterization on param change.

Use `scripts/status-indicator-enum-cycle-test.json` from this skill with `--delay-ms 1000`:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py   --url "$MCP_HTTP_URL"   --psk-env MCP_TEST_PSK   --messages-file .claude/skills/user-test/scripts/status-indicator-enum-cycle-test.json   --delay-ms 1000
```

The status indicator should visually cycle through:
- `online` → green dot (#00CC66)
- `away` → yellow dot (#FFB800)
- `busy` → red dot (#FF4444)
- `offline` → gray dot (#666666)

Each transition is a discrete snap (no interpolation). Require human visual confirmation that the dot color changes at each step.

Next, run the label-update sequence to verify text-content binding:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py   --url "$MCP_HTTP_URL"   --psk-env MCP_TEST_PSK   --messages-file .claude/skills/user-test/scripts/status-indicator-label-update-test.json   --delay-ms 1000
```

Expected label progression: "Butler" → "Codex" → (empty, no visible text). The dot remains green (online) throughout. Require human confirmation that the label text updates below the dot on each step, and clears on the final step.

Finally, run the validation fixture to confirm invalid enum rejection at the MCP surface:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py   --url "$MCP_HTTP_URL"   --psk-env MCP_TEST_PSK   --messages-file .claude/skills/user-test/scripts/status-indicator-validation-test.json
```

Expected result: MCP returns an error response (`WIDGET_PARAMETER_INVALID_VALUE`) for `status=do-not-disturb`. The widget display must not change. Report whether the error response matches expectation.

## Behavior Rules

- Use automation-first deploy/launch by default.
- Default to full app deployment via SCP + scheduled task.
- Use `--package` / cargo build only when explicitly requested.
- Always pass non-interactive SSH/SCP flags (`BatchMode`, `IdentitiesOnly`) for automation safety.
- Use `hudbot` for file operations (SCP, mkdir). Use `tzeus` for process control (taskkill, schtasks).
- **Always launch via `TzeHudOverlay` scheduled task with `--window-mode overlay`.** Never launch via `run_hud.ps1` or direct SSH exec — both produce grey opaque windows.
- **Never use `Start-Process -RedirectStandardOutput`** — it sets `CREATE_NO_WINDOW` which breaks `WS_EX_NOREDIRECTIONBITMAP` transparency.
- Require real MCP HTTP reachability before claiming publish-path success.
- Never invent zone names or endpoint values; require user-provided values.
- Treat any publish error as actionable; include exact response payload.
- Keep messages configurable from the user prompt; do not hardcode content.
- Assume Windows host defaults to `tzehouse-windows.parrot-hen.ts.net` unless user overrides.
