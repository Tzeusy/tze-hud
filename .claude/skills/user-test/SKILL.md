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

After the gauge cycle, confirm that a `status-indicator` widget instance is registered before proceeding. Use the `--list-widgets` output from Step 4 (or re-run it) to verify that a widget named `status-indicator` appears in the list. If no such instance is present, skip the sub-steps below and report that the status-indicator widget is not deployed.

Run the status-indicator enum cycle to verify discrete color binding and re-rasterization on param change.

Use `scripts/status-indicator-enum-cycle-test.json` from this skill with `--delay-ms 1000`:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/status-indicator-enum-cycle-test.json \
  --delay-ms 1000
```

The status indicator should visually cycle through:
- `online` → green dot (#00CC66)
- `away` → yellow dot (#FFB800)
- `busy` → red dot (#FF4444)
- `offline` → gray dot (#666666)

Each transition is a discrete snap (no interpolation). Require human visual confirmation that the dot color changes at each step.

Next, run the label-update sequence to verify text-content binding:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/status-indicator-label-update-test.json \
  --delay-ms 1000
```

Expected label progression: "Butler" → "Codex" → (empty, no visible text). The dot remains green (online) throughout. Require human confirmation that the label text updates below the dot on each step, and clears on the final step.

Finally, run the validation fixture to confirm invalid enum rejection at the MCP surface:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/status-indicator-validation-test.json
```

Expected result: MCP returns an error response (`WIDGET_PARAMETER_INVALID_VALUE`) for `status=do-not-disturb`. The widget display must not change. Report whether the error response matches expectation.

### Step 7: Widget Reactivity Test (Progress Bar)

After the status-indicator tests, confirm that a `progress-bar` widget instance is registered before proceeding. Use the `--list-widgets` output from Step 4 (or re-run it) to verify that a widget named `progress-bar` appears in the list. If no such instance is present, skip the sub-steps below and report that the progress-bar widget is not deployed.

This is the **progress-bar-widget** user-test scenario. It animates a thin horizontal bar from 0 to 100% and confirms visual quality at each step.

#### 7a: 7-Step Sequence

Run `progress-bar-step.json` with `--delay-ms 1000` so the tester has ~1 second to observe each visual transition:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/progress-bar-step.json \
  --delay-ms 1000
```

At each step, prompt the tester to confirm the expected visual state:

| Step | Published params | What to confirm |
|------|-----------------|-----------------|
| 1 | `progress=0.0, label=""` | Bar is empty (zero width fill); no label text visible |
| 2 | `progress=0.25, label="25%"` | Fill animates smoothly to 25%; label reads "25%" centered on bar |
| 3 | `progress=0.5, label="50%"` | Fill animates smoothly to 50%; label reads "50%" |
| 4 | `progress=0.75, label="75%"` | Fill animates smoothly to 75%; label reads "75%" |
| 5 | `progress=1.0, label="100%"` | Fill animates smoothly to full width; label reads "100%" |
| 6 | `fill_color={r:0.0, g:0.784, b:0.325, a:1.0}` | Fill color transitions from blue to green (equivalent to RGBA `[0,200,83,255]`) over 300ms; progress/label unchanged |
| 7 | clear | Bar resets to empty with no visual artifacts |

**Human acceptance criteria at each step:**

- **(a) Pill/capsule shape** — The bar has visually rounded end-caps on both the track and the fill. No sharp corners.
- **(b) Smooth fill animation** — Each step 2-5 fills with a visible 200ms animation. No jumps or jank.
- **(c) Centered label** — Label text is horizontally and vertically centered on the bar at all non-empty steps.
- **(d) Correct fill color** — Steps 1-5 use the accent blue (`#4A9EFF` or token override). Step 6 transitions to green.
- **(e) Clean reset** — After the clear action, the bar is completely empty with no residual fill or label artifacts.

#### 7b: Color Sweep (Optional)

Optionally, run the color-sweep fixture to validate color interpolation across the full spectrum with `--delay-ms 1000`:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/progress-bar-color-sweep.json \
  --delay-ms 1000
```

The bar cycles through: blue -> green -> yellow -> red -> blue (reset) -> clear (empty). Each transition should produce a visible smooth color animation over 300ms. Confirm that the fill color matches expectations at each step before the next publish fires, and that after the final clear action the bar is fully empty with no residual fill or label.

Report pass/fail per step. A step fails if the tester observes: missing animation, wrong color, misaligned label, missing rounded end-caps, or visible artifacts after the reset/clear-to-empty step.

## Notification Stack Exemplar Scenario

Use `scripts/notification_exemplar.py` to exercise the notification-area zone
on a live HUD. The script simulates 3 agents (alpha, beta, gamma) publishing
notifications with mixed urgency levels across 4 phases.

### CLI

```bash
python3 .claude/skills/user-test/scripts/notification_exemplar.py \
  --url http://tzehouse-windows.parrot-hen.ts.net:9090 \
  --psk-env TZE_HUD_PSK \
  --ttl 8000
```

Required: `--url`. Optional: `--psk-env` (default `TZE_HUD_PSK`), `--ttl` (ms, default 8000).

### Phases

| Phase | What happens | Pause |
|-------|-------------|-------|
| 1 — Initial burst | alpha (urgency 0), beta (urgency 1), gamma (urgency 2) published in order | 2s |
| 2 — Stack growth | alpha (urgency 3), beta (urgency 1) — stack reaches max_depth=5 | 2s |
| 3 — TTL expiry | waits remaining phase-1 TTL plus ~650ms (150ms fade-out + 500ms margin) for phase-1 batch to auto-dismiss | 1s |
| 4 — Max depth eviction | 6 rapid notifications; 1st is evicted instantly (no fade) when 6th arrives | 3s |

### Visual Checklist (per phase)

**Phase 1:** Three notifications stacked newest-at-top. gamma (amber), beta
(dark blue), alpha (dark gray) backdrops with 1px border and body-font text.

**Phase 2:** Five notifications. Top two are phase-2 publishes; bottom three
are phase-1. All urgency-tinted correctly.

**Phase 3:** Phase-1 batch (urgency 0/1/2) has faded out; only 2 phase-2
notifications remain (urgency 3 and 1).

**Phase 4:** Exactly 5 notifications visible. "Burst A1" (oldest, urgency 0)
is gone with no fade — evicted instantly. "Burst C6" is at top.

### Notification payload shape

```json
{"type": "notification", "text": "...", "icon": "...", "urgency": 0}
```

Published via MCP `publish_to_zone` to `notification-area` zone with `ttl_us`
derived from `--ttl` and `namespace` set to the simulated agent namespace
(`alpha`, `beta`, or `gamma`).

## Alert-Banner Exemplar Scenario

Use `scripts/alert_banner_exemplar.py` to exercise the alert-banner zone on a
live HUD. The script publishes 3 alerts at increasing urgency levels with 3-second
delays between each, validating urgency-driven visual differentiation and simultaneous
multi-alert display.

### CLI

```bash
python3 .claude/skills/user-test/scripts/alert_banner_exemplar.py \
  --url http://tzehouse-windows.parrot-hen.ts.net:9090 \
  --psk-env TZE_HUD_PSK \
  --ttl 15000
```

Required: `--url`. Optional: `--psk-env` (default `TZE_HUD_PSK`), `--ttl` (ms, default 15000).

### Sequence

| Step | Alert | Urgency | Text | Pause |
|------|-------|---------|------|-------|
| 1 | Info | 1 | "Info: system nominal" | 3s |
| 2 | Warning | 2 | "Warning: disk space low" | 3s |
| 3 | Critical | 3 | "CRITICAL: security breach detected" | — |

### Visual Checklist

After all 3 publishes, the alert-banner zone should show all three alerts
simultaneously:

- **Critical (red)** at top — "CRITICAL: security breach detected"
- **Warning (amber)** in middle — "Warning: disk space low"
- **Info (blue)** at bottom — "Info: system nominal"

All three remain visible until their TTL elapses. Confirm urgency-derived
color tinting is applied correctly at each level: blue for info (urgency=1),
amber for warning (urgency=2), red for critical (urgency=3).

### Alert payload shape

```json
{"type": "notification", "text": "...", "icon": "", "urgency": 1}
```

Published via MCP `publish_to_zone` to `alert-banner` zone with `ttl_us`
set to `--ttl` (ms) × 1000 (e.g. `--ttl 15000` → `ttl_us = 15000000`), and `namespace` set to `alert-<level>` (e.g. `alert-critical`).

## Status-Bar Exemplar Scenario

Use `scripts/status_bar_exemplar.py` to exercise the status-bar zone on a live
HUD. The script simulates three independent agents (`agent-weather`, `agent-power`,
`agent-clock`) publishing merge-keyed entries, validating multi-agent coexistence,
key replacement, empty-value removal, and TTL-driven sweep.

### CLI

```bash
python3 .claude/skills/user-test/scripts/status_bar_exemplar.py \
  --url http://tzehouse-windows.parrot-hen.ts.net:9090 \
  --psk-env TZE_HUD_PSK \
  --battery-ttl 5000
```

Required: `--url`. Optional: `--psk-env` (default `TZE_HUD_PSK`),
`--ttl` (ms, default 60000 — long TTL for weather/time entries),
`--battery-ttl` (ms, default 5000 — short TTL used to demonstrate step-9 expiry).

### 10-Step Sequence

| Step | Agent | Action | Pause |
|------|-------|--------|-------|
| 1 | agent-weather | publish `weather` → `"72F Sunny"` | — |
| 2 | agent-power | publish `battery` → `"85%"` (short TTL) | — |
| 3 | agent-clock | publish `time` → `"3:42 PM"` | — |
| 4 | — | VISUAL CHECK: all 3 visible | 3s |
| 5 | agent-weather | update `weather` → `"75F Cloudy"` (key replacement) | — |
| 6 | — | VISUAL CHECK: weather updated; battery/time unchanged | 3s |
| 7 | agent-weather | publish empty value for `weather` (key removal) | — |
| 8 | — | VISUAL CHECK: weather gone; battery/time remain | 3s |
| 9 | — | wait for battery TTL to expire (~5s + 500ms margin) | — |
| 10 | — | VISUAL CHECK: battery gone; time remains | 3s |

### Visual Checklist

**Step 4:** Status bar shows all three key-value pairs in a horizontal row at the
bottom edge of the display with a dark opaque backdrop. Entries display in
monospace font using secondary text color. Order may vary by insertion time.

**Step 6:** Status bar still shows three entries. The `weather` value reads
`75F Cloudy` (replaced). `battery: 85%` and `time: 3:42 PM` are unchanged.

**Step 8:** Status bar shows two entries. The `weather` key is no longer visible
(empty-value convention suppresses rendering). `battery` and `time` remain.

**Step 10:** Status bar shows one entry. The `battery` key has been swept by
`sweep_expired_zone_publications` after its TTL elapsed. Only `time: 3:42 PM`
remains visible.

### Human Acceptance Criteria

| # | Criterion | Step |
|---|-----------|------|
| AC1 | Three distinct keys coexist — no key overwrites another | 4 |
| AC2 | Key replacement updates only the target key's value | 6 |
| AC3 | Empty-value publish removes that key from the visible display | 8 |
| AC4 | TTL expiry removes the key without explicit publish | 10 |
| AC5 | Chrome-layer bar is always visible above content tiles | all visual steps |
| AC6 | Monospace font and dark opaque backdrop visible throughout | all visual steps |

All six criteria must pass for the status-bar exemplar to be accepted.

### Status-bar payload shape

```json
{
  "zone_name": "status-bar",
  "content": {"type": "status_bar", "entries": {"weather": "72F Sunny"}},
  "merge_key": "weather",
  "ttl_us": 60000000,
  "namespace": "agent-weather"
}
```

Each agent uses a distinct `namespace` (`agent-weather`, `agent-power`,
`agent-clock`). The `merge_key` matches the single entry key in `entries`.
Published via MCP `publish_to_zone`.

---

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
