# Exemplar Presence Card — User-Test Script and Live-Proof Gap

**Issue**: hud-sx7q.3
**Date**: 2026-04-10
**Spec**: `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md`
**Tasks ref**: `openspec/changes/exemplar-presence-card/tasks.md` §7

---

## Reconciled Status

This document now serves two purposes:
- Defines the manual 7-step visual validation scenario for Presence Card.
- Records the remaining gap: the repo-native `/user-test` scenario exists, but live resident proof execution and manual closeout are still open.

Implemented coverage already exists in automated tests (tile/node/lease/disconnect/coexistence). The unresolved work is live resident proof execution and manual closeout.

## Executable Scenario

The repo-native `/user-test` automation for this flow lives at
`.claude/skills/user-test/scripts/presence_card_exemplar.py`.

Run it with:

```bash
python3 .claude/skills/user-test/scripts/presence_card_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --tab-height 1080 \
  --transcript-out test_results/presence-card-latest.json
```

The script emits structured JSON step events to stdout and writes a transcript
artifact by default. The operator should follow the `expected_visual` field for
each emitted step while performing the manual checks below.

Current branch note:
the executable `/user-test` scenario validates the live operator-visible
sequence for stacking, content refresh, disconnect/orphan observation, and
cleanup. Because the resident session stream in this branch does not yet expose
the RFC 0011 resource-upload messages, the executable scenario uses 32x32
solid-color avatar squares rather than uploaded `StaticImageNode` avatars.

**Prerequisites:**

- Runtime is running with a default tab open (1920x1080 or equivalent).
- Three agent processes (alpha, beta, gamma) can be launched independently.
- Each agent uses the canonical gRPC test sequence defined in the spec
  (SessionInit → LeaseRequest → UploadResource → MutationBatch → periodic
  SetTileRoot).
- Agent alpha uses the blue avatar (RGB 66, 133, 244).
- Agent beta uses the green avatar (RGB 52, 168, 83).
- Agent gamma uses the orange avatar (RGB 251, 188, 4).

Exact spec sections still awaiting live proof:
1. `Requirement: gRPC Test Sequence`
- `Scenario: Full single-agent lifecycle`
- `Scenario: Three-agent concurrent lifecycle`
2. `Requirement: User-Test Scenario`
- `Scenario: User-test visual verification sequence`

---

## Manual Visual Validation Script

### Step 1 — Launch 3 agent sessions

Action:
- Launch `agent-alpha`, `agent-beta`, `agent-gamma` concurrently.
- Each agent executes SessionInit -> LeaseRequest -> UploadResource -> MutationBatch(CreateTile + UpdateTileOpacity + UpdateTileInputMode + SetTileRoot).

Pass criteria:
- All three batches accepted.
- Three cards visible in bottom-left stack at y offsets `-96`, `-184`, `-272` from tab height.
- Avatars and text render correctly with no overlap.

### Step 2 — Verify initial visual state

Pass criteria:
- Each card is 200x80, 16px left/bottom margin.
- Background, avatar placement, and markdown text formatting are correct.
- Passthrough behavior is observable (card does not capture pointer input).

### Step 3 — Wait 30s and verify updates

Pass criteria:
- "Last active" updates to human-friendly text (`30s ago`).
- Geometry and visuals remain stable (no reflow/flicker side effects).

### Step 4 — Disconnect agent gamma

Pass criteria:
- Gamma disconnects.
- Alpha and beta remain active and continue updates.

### Step 5 — Verify disconnection badge

Pass criteria:
- Gamma card shows disconnection badge promptly after orphan transition.
- Alpha/beta show no badge.

### Step 6 — Wait for grace expiry

Pass criteria:
- Gamma card is removed after grace period.
- Alpha/beta remain at original y-positions (no auto reposition).

### Step 7 — Final state verification

Pass criteria:
- Exactly two cards remain (alpha/beta), still updating.
- No erroneous badge or geometry drift.

---

## Scenario Verdict Template

| Step | Result | Notes |
|---|---|---|
| 1 | PASS/FAIL | |
| 2 | PASS/FAIL | |
| 3 | PASS/FAIL | |
| 4 | PASS/FAIL | |
| 5 | PASS/FAIL | |
| 6 | PASS/FAIL | |
| 7 | PASS/FAIL | |

Overall: PASS only if all steps pass.

---

## Automation Reality Check

Automated coverage exists for most behavior:
- `tests/integration/presence_card_tile.rs`
- `tests/integration/presence_card_coexistence.rs`
- `tests/integration/disconnect_orphan.rs`
- `crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs`

But the `/user-test` skill currently has no Presence Card resident scenario entry. That is the primary tooling gap before this can be considered live-proven in the same way as other exemplars.
