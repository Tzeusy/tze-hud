# Cooperative HUD Projection Live Governance Attempt

Date: 2026-05-09
Bead: `hud-ggntn.10`
Worker branch: `agent/hud-ggntn.10`

## Bootstrap

Worker context was valid before any validation work:

- `pwd -P`: `/home/tze/gt/tze_hud/mayor/rig/.worktrees/parallel-agents/hud-ggntn.10`
- `git branch --show-current`: `agent/hud-ggntn.10`
- `assert_worker_context.py`: `status=ok`

## Intended Scope

Run live Windows governance validation for cooperative HUD projection GAP-3:

`attach -> publish_output -> submit HUD input -> poll/acknowledge -> collapse/restore -> detach cleanup`, plus redaction, safe mode, freeze, dismiss, orphan cleanup, backlog non-escalation, and local-ack/input-to-scene evidence.

## Commands and Evidence

Connectivity and runtime checks:

- `ssh -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"`: passed.
- `ssh -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"`: passed.
- Remote `tze_hud` process was present.
- Remote `Test-NetConnection` showed `127.0.0.1:50051` and `127.0.0.1:9090` reachable.
- PSK was extracted from `TzeHudOverlay` into a local temp file with CR stripping; it is not stored in these artifacts.

Live gRPC validation artifacts:

- `logs/text-stream-portal-live-transcript.json`: stock text-stream portal exemplar successfully opened a resident session, acquired a lease, created six portal tiles, rendered baseline/composer/streaming phases, and released the lease.
- `logs/text-stream-portal-interactive-transcript.json`: stock portal baseline run rendered and released cleanup; scheduled SendKeys did not produce input events in the transcript.
- `logs/live-governance-storyboard-transcript.json`: deterministic live resident-gRPC storyboard accepted lease/tile mutations for the full governance state sequence and released the lease.

Screenshot blocker artifact:

- `screenshots/lock-screen-capture-blocker.png`: interactive scheduled screenshot captured the Windows lock screen, not the HUD overlay. Repeated captures during live HUD mutations produced the same lock-screen image, so duplicate screenshots were removed to avoid misleading evidence.

## Result

Blocked for the required visible Windows HUD proof.

The resident gRPC surface is reachable and accepts live HUD scene mutations, but the capture path available to this worker records the lock screen instead of the visible HUD overlay. Because GAP-3 explicitly requires visible Windows HUD evidence, this pass cannot honestly close the gap.

## Gates

- Worker bootstrap/context gate: passed.
- SSH connectivity gate for `hudbot` and `tzeus`: passed.
- Windows runtime process and port reachability gate: passed.
- Live resident gRPC mutation/lease cleanup smoke: passed.
- Visible HUD screenshot gate: blocked by lock-screen capture.

## Blocker

The Windows console session reports active, but scheduled screenshot capture sees the lock screen. The next pass needs an unlocked/observable interactive desktop or a runtime-native frame/readback artifact path that captures the HUD surface directly.

## Follow-ups

- Re-run the live governance sequence with the Windows desktop unlocked and operator-visible.
- Prefer a runtime-native screenshot/readback command for HUD evidence so future workers do not depend on Windows desktop screenshot behavior.
- If scheduled keyboard/mouse input remains required, verify the input injection path against a simple visible Notepad or HUD hit-region target before using it for portal composer evidence.
