---
name: user-test
description: Use when validating a cross-machine HUD flow where Butler deploys/runs the full Windows app over SSH+SCP (tailnet default host), then publishes configurable test messages to HUD zones via MCP `publish_to_zone`.
metadata:
  owner: tze
  authors:
    - tze
    - OpenAI Codex
    - Claude
  status: active
  last_reviewed: "2026-06-20"
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
- `win_user` (default: `hud-user`)
- `win_host` (default: `windows-host.example`)
- `ssh_key_path` (default in local environment: `~/.ssh/hud-ssh-key`)
- `task_name` (default: `TzeHudOverlay`)
- `mcp_http_url` (default: `http://windows-host.example:9090`)
- `mcp_psk_env` (default: `MCP_TEST_PSK`)
- `messages`: array of zone publishes
- `widget_messages`: array of widget publishes (optional)

For the exact `messages` / `widget_messages` payload shapes, content types by
zone, widget parameter types, and `widget_name` instance-discovery semantics
(`list_widgets`), see
[references/message-payloads.md](references/message-payloads.md).

## Support File Index

Use these files when the matching workflow section below calls for them:

- Deployment and setup: [scripts/deploy_windows_hud.sh](scripts/deploy_windows_hud.sh), [scripts/windows_setup_hud_automation.ps1](scripts/windows_setup_hud_automation.ps1), [scripts/d18_validation.sh](scripts/d18_validation.sh)
- Batch publishers and broad zone fixtures: [scripts/publish_zone_batch.py](scripts/publish_zone_batch.py), [scripts/publish_widget_batch.py](scripts/publish_widget_batch.py), [scripts/all-zones-test.json](scripts/all-zones-test.json), [scripts/widget-cleanup.json](scripts/widget-cleanup.json)
- Widget fixtures: [scripts/gauge_cycle_test.json](scripts/gauge_cycle_test.json), [scripts/progress-bar-step.json](scripts/progress-bar-step.json), [scripts/progress-bar-color-sweep.json](scripts/progress-bar-color-sweep.json), [scripts/progress-bar-rapidfire-100-5s.json](scripts/progress-bar-rapidfire-100-5s.json), [scripts/status-indicator-enum-cycle-test.json](scripts/status-indicator-enum-cycle-test.json), [scripts/status-indicator-theme-cycle-test.json](scripts/status-indicator-theme-cycle-test.json), [scripts/status-indicator-label-update-test.json](scripts/status-indicator-label-update-test.json), [scripts/status-indicator-validation-test.json](scripts/status-indicator-validation-test.json), [scripts/status-indicator-contention-test.json](scripts/status-indicator-contention-test.json), [scripts/status-indicator-theme-status-matrix-test.json](scripts/status-indicator-theme-status-matrix-test.json)
- Zone exemplars and fixtures: [scripts/subtitle_exemplar.py](scripts/subtitle_exemplar.py), [scripts/subtitle-full-sequence.json](scripts/subtitle-full-sequence.json), [scripts/subtitle-single-line.json](scripts/subtitle-single-line.json), [scripts/subtitle-multiline.json](scripts/subtitle-multiline.json), [scripts/subtitle-rapid-replace.json](scripts/subtitle-rapid-replace.json), [scripts/subtitle-streaming.json](scripts/subtitle-streaming.json), [scripts/subtitle-ttl-expiry.json](scripts/subtitle-ttl-expiry.json), [scripts/notification_exemplar.py](scripts/notification_exemplar.py), [scripts/notification-full-gamut.json](scripts/notification-full-gamut.json), [scripts/alert_banner_exemplar.py](scripts/alert_banner_exemplar.py), [scripts/status_bar_exemplar.py](scripts/status_bar_exemplar.py), [scripts/ambient_background_exemplar.py](scripts/ambient_background_exemplar.py)
- Resident gRPC, portal, media, and stress helpers: [scripts/hud_grpc_client.py](scripts/hud_grpc_client.py), [scripts/test_hud_grpc_client.py](scripts/test_hud_grpc_client.py), [scripts/presence_card_exemplar.py](scripts/presence_card_exemplar.py), [scripts/text_stream_portal_exemplar.py](scripts/text_stream_portal_exemplar.py), [scripts/windows_media_ingress_exemplar.py](scripts/windows_media_ingress_exemplar.py), [scripts/windows_media_resource_sampler.py](scripts/windows_media_resource_sampler.py), [scripts/stress_test_zones.py](scripts/stress_test_zones.py)

## Reference Files

Detailed payload shapes, extended workflow steps, and per-surface exemplar
scenarios live in `references/`. Load the one matching the task:

- [references/message-payloads.md](references/message-payloads.md) — zone message
  and widget payload shapes, content types by zone, widget parameter types, and
  `widget_name` instance discovery (`list_widgets`).
- [references/widget-reactivity-tests.md](references/widget-reactivity-tests.md) —
  Workflow Steps 5–7: gauge cycling, status-indicator (enum/theme/label/validation),
  and progress-bar (7-step, color sweep, rapid-fire) reactivity tests.
- [references/zone-exemplars.md](references/zone-exemplars.md) — MCP `publish_to_zone`
  exemplar scenarios: subtitle, notification stack, alert-banner, status-bar, and
  ambient-background.
- [references/resident-exemplars.md](references/resident-exemplars.md) — resident
  gRPC session scenarios: Presence Card and Text Stream Portals.
- [references/media-and-validation-lanes.md](references/media-and-validation-lanes.md)
  — Windows media-ingress exemplar lane and the D18 SSH validation lane.

## Workflow

### Step 0: SSH Connectivity Gate

Verify key auth for **both** users (Linux):

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/hud-ssh-key \
  hud-user@windows-host.example "whoami"
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/hud-ssh-key \
  admin-user@windows-host.example "whoami"
```

Both must succeed. `hud-user` is used for file deployment (SCP). `admin-user` is used for process control (kill, scheduled task trigger) because `admin-user` owns the interactive desktop session.

### Step 1: Deploy (SCP via hud-user)

Copy the prebuilt `.exe` to the Windows host:

```bash
# Kill any running instance first (must use admin-user — hud-user can't kill it)
ssh -i ~/.ssh/hud-ssh-key -o BatchMode=yes -o StrictHostKeyChecking=no \
  admin-user@windows-host.example "taskkill /F /IM tze_hud.exe"
sleep 2

# SCP the exe (via hud-user)
scp -i ~/.ssh/hud-ssh-key -o BatchMode=yes -o StrictHostKeyChecking=no \
  /path/to/tze_hud.exe \
  hud-user@windows-host.example:C:/tze_hud/tze_hud.exe
```

Report: file size, checksum (`sha256sum`), remote path.

### Step 2: Register + Launch (via admin-user)

The HUD **must** be launched via a scheduled task as `admin-user` with `--window-mode overlay`. This is critical for transparency — SSH-launched processes cannot access the desktop GPU, and `run_hud.ps1` wrappers interfere with window creation.

```bash
# Register the overlay task (idempotent — safe to re-run)
ssh -i ~/.ssh/hud-ssh-key -o BatchMode=yes -o StrictHostKeyChecking=no \
  admin-user@windows-host.example \
  "powershell -NoProfile -Command \"Register-ScheduledTask -TaskName 'TzeHudOverlay' \
    -Action (New-ScheduledTaskAction \
      -Execute 'C:\\tze_hud\\tze_hud.exe' \
      -Argument '--window-mode overlay' \
      -WorkingDirectory 'C:\\tze_hud') \
    -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries) \
    -Force\""

# Launch it
ssh -i ~/.ssh/hud-ssh-key -o BatchMode=yes -o StrictHostKeyChecking=no \
  admin-user@windows-host.example \
  "schtasks /Run /TN TzeHudOverlay"
```

**Transparency requirements** (if the window is grey/opaque, one of these is wrong):
- `--window-mode overlay` — fullscreen mode is intentionally opaque
- Task runs as `admin-user` (the user logged into the console desktop)
- Exe runs directly (no PowerShell/bat wrapper — wrapper windows break transparency)
- NVIDIA driver 595.97+ on the Windows host
- Commit must include `with_no_redirection_bitmap(true)`, Vulkan forcing, PreMultiplied alpha

### Step 2: MCP Reachability Gate

Require live MCP HTTP reachability before publish.

- Default URL: `http://windows-host.example:9090`
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
5. For manual user-test runs that touch durable widget instances, clear them at the end with `--cleanup-on-exit` or the cleanup fixture below. This prevents stale widget state from remaining on the HUD after interrupted or partial tests.

Example:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file /tmp/hud-widget-messages.json \
  --list-widgets \
  --cleanup-on-exit
```

If `list_widgets` returns no instances, skip widget publishing and report that no widgets are registered (the HUD binary may predate widget support).

To clear stale widget state explicitly, run:

```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file .claude/skills/user-test/scripts/widget-cleanup.json
```

### Steps 5–7: Widget Reactivity Tests

After Step 4, run the gauge, status-indicator, and progress-bar reactivity
tests. Full step-by-step instructions, fixtures, expected visuals, and human
acceptance criteria are in
[references/widget-reactivity-tests.md](references/widget-reactivity-tests.md).

## Exemplar Scenarios

Beyond the core deploy/publish loop, this skill bundles per-surface exemplar
scenarios. Each lives in a reference file with its own CLI, phases/sequence,
visual checklist, and payload shape:

- **Zone exemplars** (MCP `publish_to_zone`): subtitle, notification stack,
  alert-banner, status-bar, ambient-background —
  [references/zone-exemplars.md](references/zone-exemplars.md).
- **Resident gRPC exemplars**: Presence Card and Text Stream Portals —
  [references/resident-exemplars.md](references/resident-exemplars.md).
- **Media ingress & validation lanes**: Windows media-ingress exemplar and the
  D18 SSH validation lane —
  [references/media-and-validation-lanes.md](references/media-and-validation-lanes.md).

## Behavior Rules

- Use automation-first deploy/launch by default.
- Default to full app deployment via SCP + scheduled task.
- Use `--package` / cargo build only when explicitly requested.
- Always pass non-interactive SSH/SCP flags (`BatchMode`, `IdentitiesOnly`) for automation safety.
- Use `hud-user` for file operations (SCP, mkdir). Use `admin-user` for process control (taskkill, schtasks).
- **Always launch via `TzeHudOverlay` scheduled task with `--window-mode overlay`.** Never launch via `run_hud.ps1` or direct SSH exec — both produce grey opaque windows.
- **Never use `Start-Process -RedirectStandardOutput`** — it sets `CREATE_NO_WINDOW` which breaks `WS_EX_NOREDIRECTIONBITMAP` transparency.
- Require real MCP HTTP reachability before claiming publish-path success.
- Never invent zone names or endpoint values; require user-provided values.
- Treat any publish error as actionable; include exact response payload.
- Keep messages configurable from the user prompt; do not hardcode content.
- Assume Windows host defaults to `windows-host.example` unless user overrides.
```