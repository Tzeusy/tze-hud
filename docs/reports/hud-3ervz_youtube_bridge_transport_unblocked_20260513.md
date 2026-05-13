# hud-3ervz YouTube Bridge Transport Unblocked

Date: 2026-05-13

## Scope

This retry fixed the live `youtube-bridge` failure discovered by `hud-i1nrq`.
The failure happened before frame capture because the generated Windows
CopyFromScreen PowerShell was sent over SSH as one `powershell -EncodedCommand`
payload and exceeded the Windows/OpenSSH command-line limit.

The implementation now copies the generated script and a small runner to a
temporary directory under `C:\tze_hud\tmp`, runs the runner through an
interactive scheduled task under the configured Windows user, polls for a
return-code file, fetches stdout/stderr/rc with SCP, and removes both the task
and temp directory in cleanup. Fetched text is decoded with UTF-8 BOM and
UTF-16 fallback because Windows PowerShell output can arrive as UTF-16.

The remote YouTube sidecar launch uses the same interactive scheduled-task
transport. Direct SSH `Start-Process` can succeed without creating a visible
desktop player window, which leaves frame capture blocked on "no visible
official YouTube player window".

## Live Proof

Artifact directory:

```text
docs/reports/artifacts/hud-3ervz-youtube-bridge-file-transport-20260513T131423Z/
```

The live official-player frame capture succeeded after launching the sidecar
through an interactive desktop task:

```text
video_id=O0FGCxkHM-U
window="tze_hud YouTube source evidence O0FGCxkHM-U - Google Chrome"
selected_window_visible_area=2956800
captured_frame_count=5
distinct_frame_hashes=4
mean_rgb samples:
  [46.30, 32.45, 22.28]
  [46.30, 32.45, 22.28]
  [23.95, 16.38, 15.03]
  [7.22, 7.69, 9.73]
  [6.53, 6.86, 8.69]
```

The policy boundary stayed intact:

- official operator-visible YouTube embedded player for `O0FGCxkHM-U`
- video-only frame path
- `MediaIngressOpen` entrypoint
- no `yt-dlp` / `youtube-dl`
- no direct media URL extraction
- no download/cache/offline copy
- no audio route to HUD
- no browser/WebView compositor surface

The `youtube-bridge` retry using the captured live fixture then reached and
completed `MediaIngressOpen` against the isolated runtime:

```text
media_ingress_open_attempted=true
media_ingress_open_admitted=true
frame_capture.captured_frame_count=5
frame_capture.distinct_frame_hashes=4
close_reason=AGENT_CLOSED
```

This closes the command-length/transport blocker. It does not close
`hud-s0pit`: HUD pixel/readback proof for the rendered media surface remains a
separate acceptance requirement.

## Host Restore

Cleanup restored the benchmark HUD:

```text
stopped_isolated_pids=[27312]
deleted_temp_tasks=[
  TzeHudI1nrqRetry012749,
  TzeHud3ervzCapture,
  TzeHud3ervzSidecar
]
benchmark_ports_ready=true
benchmark_pid=31444
listeners=0.0.0.0:50051, 0.0.0.0:9090
```

Fresh smoke after cleanup:

```text
tailscale ping tzehouse-windows.parrot-hen.ts.net -> pong
ssh tzeus -> tzehouse\tzeus
ssh hudbot -> tzehouse\hudbot
tcp 22/50051/9090 -> open
MCP /mcp -> HTTP 200 JSON-RPC parse error for empty body
publish_widget_batch.py --list-widgets -> main-gauge, main-progress, main-status
```

## Local Validation

```text
python3 -m py_compile .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py
python3 .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py
python3 .claude/skills/user-test/tests/test_windows_media_ingress_exemplar.py
git diff --check
```

Results:

```text
17 script tests passed
6 user-test tests passed
git diff --check passed
```

No PSK or secret material was written to tracked files.
