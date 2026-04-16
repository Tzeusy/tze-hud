# Presence Card Live User-Test Flow Reconciliation (gen-1)

Date: 2026-04-16
Issue: `hud-sx7q.5`
Epic: `hud-sx7q` (Presence Card live user-test flow)

## Inputs Audited

- `bd show hud-sx7q.5 --json` (epic + sibling bead contract)
- `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md`
- `docs/exemplar-presence-card-coverage.md`
- `docs/exemplar-presence-card-user-test.md`
- `docs/exemplar-manual-review-checklist.md`
- `.claude/skills/user-test/SKILL.md`
- `.claude/skills/user-test/scripts/presence_card_exemplar.py`
- `.claude/skills/user-test/scripts/hud_grpc_client.py`
- `.claude/skills/user-test/tests/test_presence_card_exemplar.py`
- `tests/integration/presence_card_tile.rs`
- `tests/integration/presence_card_coexistence.rs`
- `tests/integration/disconnect_orphan.rs`
- `crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs`

## Requirement-to-Bead Coverage Matrix

| Requirement (epic contract) | Primary implementing bead(s) | Coverage status | Evidence |
|---|---|---|---|
| Presence Card Tile Geometry | `hud-sx7q.2`, `hud-sx7q.3` | Covered (automation) | `tests/integration/presence_card_tile.rs`; `presence_card_exemplar.py` constants (`CARD_W=200`, `CARD_H=80`, margins, z-order per agent) |
| Multi-Agent Vertical Stacking | `hud-sx7q.3` | Covered (automation + scenario implementation) | `tests/integration/presence_card_coexistence.rs`; `presence_card_exemplar.py` (`card_y_offset`, 3-agent launch plan) |
| Presence Card Node Tree | `hud-sx7q.2`, `hud-sx7q.3` | Partially covered | Helper exposes Presence Card node builders (including `StaticImageNode`) in `hud_grpc_client.py`; live scenario currently uses solid-color avatar squares pending resident upload seam |
| Periodic Content Update | `hud-sx7q.3` | Covered (automation + scenario implementation) | `tests/integration/presence_card_coexistence.rs`; `presence_card_exemplar.py` (`rebuild_agent_card`, 30s update) |
| Agent Disconnect and Orphan Handling | `hud-sx7q.3`, `hud-sx7q.4` | Covered in automation, live proof blocked | `tests/integration/disconnect_orphan.rs`; live scenario disconnect/orphan steps implemented but 2026-04-16 run blocked on missing `TZE_HUD_PSK` |
| Resource Upload for Avatar Icons | `hud-sx7q.2` | Partially covered | Automated upload/reference semantics covered in `tests/integration/presence_card_tile.rs`; resident session flow still lacks true upload message path (tracked by epic `hud-ooj1`) |
| gRPC Test Sequence | `hud-sx7q.2`, `hud-sx7q.3`, `hud-sx7q.4` | Partially covered | Helper + scenario implement resident sequence; live run attempt recorded in `docs/exemplar-presence-card-user-test.md` is blocked by missing `TZE_HUD_PSK` |
| User-Test Scenario | `hud-sx7q.3`, `hud-sx7q.4` | Partially covered | Scenario script and skill entry exist; manual visual PASS/FAIL remains blocked by auth preflight failure (`missing_psk`) |

## Coverage Verdict

1. Sibling beads `hud-sx7q.1` through `hud-sx7q.4` materially closed the original planning/tooling gaps.
2. Full requirement closure for this epic is not yet achieved because two residual gaps remain:
- Live resident run cannot complete until `TZE_HUD_PSK` is available in the operator environment.
- Resident session-stream avatar upload is still a known contract seam; the live scenario currently uses solid-color avatar placeholders rather than true uploaded `StaticImageNode` assets.

## Gap Routing

Existing tracked seam:
- Resident upload contract/implementation work is already active under epic `hud-ooj1` (including schema/runtime/consumer follow-through).

Coordinator action still needed under `hud-sx7q`:
1. Reopen `hud-sx7q.4` or create a new child bead for authenticated rerun + evidence capture.
2. Make that bead gate `hud-sx7q.6` epic report closeout, since manual visual verdict is currently `BLOCKED`.

## Gen-2 Determination

Gen-2 reconciliation is required only if the authenticated rerun or resident upload seam lands additional behavior changes that alter this matrix.

At this time, no new code-path implementation bead was created in this worker session because required follow-up seams are already tracked (`hud-ooj1`) and lifecycle mutation is coordinator-owned.
