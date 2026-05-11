# Windows Media Ingress Validation Report — hud-gog64.5

Date: 2026-05-11T16:17:24Z  
Issue: `hud-gog64.5`  
Change: `openspec/changes/windows-media-ingress-exemplar/`  
Runtime config path: `app/tze_hud_app/config/windows-media-ingress.toml`  
Windows target: `tzehouse-windows.parrot-hen.ts.net` / `100.87.181.125`  
Approved media zone: `media-pip`  
Approved local producer identity: `windows-local-media-producer`  
HUD source identity: `synthetic-color-bars` self-owned/local video-only source  
YouTube source identity: `O0FGCxkHM-U` through `https://www.youtube.com/embed/O0FGCxkHM-U`

## Verdict

Synthetic/local deterministic validation is complete and passing. It covers media enablement/config shape, approved zone registration, admission, frame presentation, placeholder-before-first-frame, clipping, teardown, second-stream rejection, policy denial, lease/capability revoke, reconnect/disconnect slot release, disabled-gate responses, and operator-disabled admission behavior.

Live Windows reachability is currently good, but authenticated live media ingress did not run because this worker shell has neither `TZE_HUD_PSK` nor `MCP_TEST_PSK` set. The 10-minute record-only soak is blocked for the same reason. No live HUD frame-ingress proof is claimed.

## Evidence Artifacts

Artifact directory: `docs/reports/artifacts/windows_media_ingress_hud_gog64_5/`

| Artifact | Meaning |
|---|---|
| `reachability-20260511T161724Z.txt` | Tailscale, SSH, and TCP probe evidence for the Windows host. |
| `policy-review.json` | Machine-readable YouTube/HUD source boundary. |
| `youtube-source-evidence-live.json` | Live SSH launch evidence for the official YouTube embed sidecar. |
| `youtube-source-evidence.json` | Dry-run source-evidence artifact for the same official-player lane. |
| `youtube_source_evidence.html` | Generated official-player sidecar HTML. |
| `local-producer-blocked-missing-psk.txt` | Auth preflight failure for the live HUD local-producer lane. |

## Commands Run

```bash
openspec validate windows-media-ingress-exemplar --strict
```

Result: pass.

```bash
cargo test -p tze_hud_config media_ingress --lib
cargo test -p tze_hud_app --test benchmark_config_schema windows_media_config -- --nocapture
cargo test -p tze_hud_protocol media_ingress_ --lib -- --nocapture
cargo test -p tze_hud_protocol --test media_signaling media_ingress -- --nocapture
cargo test -p tze_hud_runtime media_ingress --lib -- --nocapture
cargo test -p tze_hud_runtime --features v2_preview synthetic_media_surface --lib -- --nocapture
cargo test -p tze_hud_compositor video_surface --lib -- --nocapture
cargo test -p tze_hud_compositor --features v2_preview synthetic_video_surface --lib -- --nocapture
cargo test -p tze_hud_compositor --features v2_preview invalid_video_surface --lib -- --nocapture
cargo test -p tze_hud_compositor --features v2_preview scoped_to_media_pip --lib -- --nocapture
python3 -m py_compile .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py
```

Results:

- `tze_hud_config`: 5 media ingress tests passed.
- `tze_hud_app`: 2 Windows media config tests passed.
- `tze_hud_protocol` lib: 6 session-server media ingress tests passed.
- `tze_hud_protocol` media signaling: 10 wire/proto tests passed.
- `tze_hud_runtime` media ingress: 63 state/admission tests passed.
- `tze_hud_runtime --features v2_preview`: 2 synthetic surface tests passed.
- `tze_hud_compositor`: 2 placeholder/video-surface tests passed.
- `tze_hud_compositor --features v2_preview`: frame render/clip/teardown, invalid-frame, and media-pip scoping tests passed.
- `windows_media_ingress_exemplar.py` compiled successfully.

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  policy-review \
  --evidence-json docs/reports/artifacts/windows_media_ingress_hud_gog64_5/policy-review.json

python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  youtube-sidecar \
  --windows-host tzehouse-windows.parrot-hen.ts.net \
  --windows-user tzeus \
  --ssh-key ~/.ssh/ecdsa_home \
  --connect-timeout-s 8 \
  --output-dir docs/reports/artifacts/windows_media_ingress_hud_gog64_5 \
  --evidence-json docs/reports/artifacts/windows_media_ingress_hud_gog64_5/youtube-source-evidence-live.json
```

Result: pass. The sidecar launched over SSH as `tzeus` and records `download_or_extraction: not_used`, `hud_runtime_receives_youtube_frames: false`, and `raw_youtube_frame_bridge: blocked_pending_policy_approval`.

```bash
timeout 8 tailscale status --json | jq '.Peer[]? | select(.DNSName=="tzehouse-windows.parrot-hen.ts.net.") | {HostName,DNSName,Online,LastSeen,TailscaleIPs}'
timeout 12 tailscale ping -c 1 tzehouse-windows.parrot-hen.ts.net
timeout 12 ssh -i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=8 tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
timeout 12 ssh -i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=8 hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
for port in 22 50051 9090; do
  timeout 6 bash -lc "cat < /dev/null > /dev/tcp/tzehouse-windows.parrot-hen.ts.net/$port" \
    >/dev/null 2>&1 && echo "$port open" || echo "$port closed_or_timeout"
done
```

Result: pass. Tailscale reported the peer online, `tailscale ping` returned `pong`, SSH succeeded for `tzeus` and `hudbot`, and TCP ports `22`, `50051`, and `9090` were open.

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  local-producer \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --agent-id windows-local-media-producer \
  --zone-name media-pip \
  --source-label synthetic-color-bars \
  --hold-s 30 \
  --evidence-json docs/reports/artifacts/windows_media_ingress_hud_gog64_5/local-producer-evidence.json
```

Result: blocked before network I/O with `RuntimeError: set TZE_HUD_PSK or pass --psk`. This worker did not have `TZE_HUD_PSK` or `MCP_TEST_PSK` set.

## Functional Gate Matrix

| Gate | Status | Evidence |
|---|---:|---|
| OpenSpec strict validation | Pass | `openspec validate windows-media-ingress-exemplar --strict` |
| Explicit config enablement/default-off behavior | Pass | `tze_hud_config media_ingress` |
| Approved zone identity and producer capability grants | Pass | `tze_hud_app --test benchmark_config_schema windows_media_config` |
| Synthetic admission | Pass | `tze_hud_protocol media_ingress_open_admits_one_configured_video_stream` |
| Frame presentation | Pass | `test_synthetic_video_surface_frames_render_clip_and_teardown` |
| Placeholder before first frame | Pass | `test_synthetic_video_surface_frames_render_clip_and_teardown`, `test_video_surface_ref_zone_emits_dark_placeholder` |
| Clipping to `media-pip` geometry | Pass | `test_synthetic_video_surface_frames_render_clip_and_teardown` |
| Teardown returns to placeholder | Pass | `test_synthetic_video_surface_frames_render_clip_and_teardown` |
| Second-stream rejection | Pass | `media_ingress_rejects_second_stream_wrong_zone_missing_classification_and_audio`, `media_ingress_limit_is_global_and_disconnect_releases_slot` |
| Policy denial / missing classification / audio rejection | Pass | `media_ingress_rejects_second_stream_wrong_zone_missing_classification_and_audio`, `media_ingress` admission tests |
| Lease/capability revoke | Pass | `media_ingress_close_and_capability_revoke_emit_state_and_notice`, `test_capability_revoked_transitions_to_revoked` |
| Reconnect/disconnect gating | Pass | `media_ingress_limit_is_global_and_disconnect_releases_slot` |
| Disabled-gate response | Pass | `media_ingress_disabled_gate_rejects_without_admission`, `synthetic_media_surface_rejects_default_off_config` |
| Operator-disable behavior | Pass, deterministic only | `test_dialog_gate_rejects_operator_disabled_media_ingress` |
| Live Windows local producer rendered in `media-pip` | Blocked | Missing local non-default PSK env; see `local-producer-blocked-missing-psk.txt` |
| YouTube official-player source evidence | Pass | `youtube-source-evidence-live.json` |
| 10-minute record-only soak | Blocked | Same PSK blocker as live local-producer lane |

## Record-Only Metrics

| Metric | Value |
|---|---|
| First-frame time | Not collected; live authenticated producer did not start. |
| Dropped frames | Not collected; live authenticated producer did not start. |
| CPU/GPU/memory | Not collected for media soak; authenticated live/soak lane blocked before launch. |
| Soak duration | 0s; 10-minute record-only soak blocked by missing local PSK. |
| Teardown behavior | Deterministic pass in compositor frame test; live teardown not collected. |
| Operator-disable behavior | Deterministic admission pass; live operator-disable proof not collected. |

## Blocker And Unblock Condition

The Windows host is reachable. The active blocker is authentication material in this worker environment: set a non-default HUD PSK in `TZE_HUD_PSK` or `MCP_TEST_PSK`, or provide an approved no-output retrieval path, then rerun the local producer and 10-minute soak against a HUD launched with `app/tze_hud_app/config/windows-media-ingress.toml`.

Do not count YouTube sidecar evidence as HUD frame-ingress proof. The sidecar only proves official-player source handling for `O0FGCxkHM-U`; raw YouTube frame bridging remains blocked pending separate policy approval.

## Follow-Up Beads To Create

The worker did not mutate Beads lifecycle state. The coordinator should create or wire follow-ups for:

```json
[
  {
    "title": "Provision no-secret PSK path for Windows media ingress validation",
    "type": "task",
    "priority": 1,
    "depends_on": "hud-gog64.5",
    "rationale": "TzeHouse is reachable, but this worker cannot run authenticated gRPC/MCP media validation because TZE_HUD_PSK and MCP_TEST_PSK are unset. Provide a no-secret retrieval/provisioning path or operator-set env var, then rerun local-producer and soak."
  },
  {
    "title": "Run authenticated 10-minute Windows media ingress record-only soak",
    "type": "task",
    "priority": 1,
    "depends_on": "hud-gog64.5",
    "rationale": "The deterministic gates and YouTube sidecar evidence passed, but the live local-producer and soak metrics were blocked before authentication. The soak must record first-frame time, dropped frames, CPU/GPU/memory, operator-disable behavior, and teardown behavior."
  }
]
```
