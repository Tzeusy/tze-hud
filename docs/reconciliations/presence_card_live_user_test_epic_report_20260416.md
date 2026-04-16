# Presence Card Live User-Test Flow Epic Report

Date: 2026-04-16
Issue: `hud-sx7q.6`
Epic: `hud-sx7q` (Presence Card live user-test flow)
Source direction artifacts:
- `docs/reconciliations/presence_card_direction_report_20260410.md`
- `docs/reconciliations/presence_card_backlog_materialization_20260410.md`
Reconciliation anchor:
- `docs/reconciliations/presence_card_live_user_test_reconciliation_gen1_20260416.md` (`hud-sx7q.5`)

## Epic Outcome Summary

The epic delivered the planned Presence Card live-proof tooling and artifact
reconciliation, then ended with an honest blocked state for final manual visual
sign-off.

Delivered:
1. Presence Card planning and review docs were reconciled to code reality.
2. Resident gRPC helper and scenario surfaces were extended for Presence Card
   lifecycle execution.
3. `/user-test` now includes a dedicated Presence Card resident scenario with
   structured transcript output.
4. Live Windows run was attempted and recorded in docs, including the blocking
   authentication preflight result.
5. Gen-1 spec-to-code reconciliation was completed and captured.

Not yet fully closed:
1. Live scenario pass/fail visual verification remains blocked pending operator
   environment `TZE_HUD_PSK`.

## What Shipped (By Child Bead)

## `hud-sx7q.1` — doc reconciliation
- Updated `docs/exemplar-presence-card-coverage.md` to reflect implemented
  automation and isolate live-proof gaps.
- Updated `docs/exemplar-presence-card-user-test.md` and
  `docs/exemplar-manual-review-checklist.md` to align checklist language with
  actual remaining work.

## `hud-sx7q.2` — resident helper extension
- Extended `.claude/skills/user-test/scripts/hud_grpc_client.py` for Presence
  Card resident flow support.
- Landed helper work via PR #426.

## `hud-sx7q.3` — live scenario + skill integration
- Added `.claude/skills/user-test/scripts/presence_card_exemplar.py`.
- Added/updated Presence Card scenario guidance in
  `.claude/skills/user-test/SKILL.md`.
- Added scenario coverage in
  `.claude/skills/user-test/tests/test_presence_card_exemplar.py`.

## `hud-sx7q.4` — live validation attempt
- Recorded live run attempt and blocker in:
  - `docs/exemplar-presence-card-coverage.md`
  - `docs/exemplar-presence-card-user-test.md`
  - `docs/exemplar-manual-review-checklist.md`

## `hud-sx7q.5` — reconciliation
- Added reconciliation artifact:
  `docs/reconciliations/presence_card_live_user_test_reconciliation_gen1_20260416.md`
- Confirmed most requirements are covered and explicitly called out remaining
  seams as blocked/partial rather than overstating completion.

## Windows Verification Record

Observed in documented 2026-04-16 run:
1. Target reachability succeeded (`50051` gRPC, `9090` MCP).
2. Scenario preflight failed with `{"error":"missing_psk","psk_env":"TZE_HUD_PSK"}`.
3. Manual visual step verdict remains `BLOCKED` for all seven Presence Card
   review steps until `TZE_HUD_PSK` is provisioned in the operator shell.

This epic therefore closes implementation/documentation work while preserving an
explicit live-proof blocker.

## Residual Risk / Blockers

1. **Operator auth dependency**: live resident scenario cannot run without
   `TZE_HUD_PSK` in environment.
2. **Resident avatar upload seam**: full in-scenario uploaded `StaticImageNode`
   avatars remain coupled to the separate resident upload contract work (`hud-ooj1`).
3. **Manual checklist closure risk**: checklist item 7 must not be marked done
   until authenticated rerun evidence exists.

## Updated Artifacts Index

Planning/reconciliation:
- `docs/reconciliations/presence_card_direction_report_20260410.md`
- `docs/reconciliations/presence_card_backlog_materialization_20260410.md`
- `docs/reconciliations/presence_card_live_user_test_reconciliation_gen1_20260416.md`
- `docs/reconciliations/presence_card_live_user_test_epic_report_20260416.md`

Scenario/tooling:
- `.claude/skills/user-test/SKILL.md`
- `.claude/skills/user-test/scripts/hud_grpc_client.py`
- `.claude/skills/user-test/scripts/presence_card_exemplar.py`
- `.claude/skills/user-test/tests/test_presence_card_exemplar.py`

Coverage/manual review:
- `docs/exemplar-presence-card-coverage.md`
- `docs/exemplar-presence-card-user-test.md`
- `docs/exemplar-manual-review-checklist.md`

## Coordinator Follow-Through

To fully close Presence Card live proof, coordinator should route one final
auth-enabled rerun/verification bead (or reopen `hud-sx7q.4`) that:
1. Executes `presence_card_exemplar.py` with valid `TZE_HUD_PSK`.
2. Captures transcript evidence.
3. Records PASS/FAIL for all seven manual visual steps.
