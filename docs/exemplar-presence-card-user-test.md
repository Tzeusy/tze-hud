# Exemplar Presence Card — User-Test Script

**Issue**: hud-apoe.5
**Date**: 2026-03-31
**Spec**: `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md`
**Tasks ref**: `openspec/changes/exemplar-presence-card/tasks.md` §7

---

## Overview

This script defines the manual visual validation procedure for the
exemplar-presence-card lifecycle. A tester executes 7 steps in sequence and
records a PASS or FAIL verdict for each. All 7 steps must pass for the
scenario to be considered green.

**Prerequisites:**

- Runtime is running with a default tab open (1920x1080 or equivalent).
- Three agent processes (alpha, beta, gamma) can be launched independently.
- Each agent uses the canonical gRPC test sequence defined in the spec
  (SessionInit → LeaseRequest → UploadResource → MutationBatch → periodic
  SetTileRoot).
- Agent alpha uses the blue avatar (RGB 66, 133, 244).
- Agent beta uses the green avatar (RGB 52, 168, 83).
- Agent gamma uses the orange avatar (RGB 251, 188, 4).

---

## Step 1 — Launch 3 agent sessions

**Action:**
Launch agents alpha, beta, and gamma concurrently. Each agent must:

1. Complete `SessionInit` and receive `SessionEstablished`.
2. Submit `LeaseRequest` with `ttl_ms=120000` and capabilities
   `[create_tiles, modify_own_tiles]`. Confirm `LeaseResponse.granted = true`.
3. Upload its 32x32 PNG avatar via `UploadResource`. Confirm a `ResourceId`
   is returned.
4. Submit a `MutationBatch` containing:
   - `CreateTile` with bounds `{x: 16, y: tab_height − 96/184/272, width: 200, height: 80}`
     (agents alpha/beta/gamma use y-offsets −96, −184, −272 respectively),
     `z_order` = 100/101/102, and `lease_id` = the granted lease.
   - `UpdateTileOpacity(1.0)`.
   - `UpdateTileInputMode(Passthrough)`.
   - `SetTileRoot` (or 3x `AddNode`) with the 3-node flat tree:
     `SolidColorNode` (dark bg) → `StaticImageNode` (avatar) +
     `TextMarkdownNode` ("**AgentName**\nLast active: now").

**Pass criteria:**
- All three `MutationBatch` calls are accepted (no validation error returned).
- Three presence card tiles are visible in the bottom-left corner of the
  active tab.
- Cards are stacked vertically with approximately 8px gaps between them.
- Agent alpha's card appears at the bottom (y = tab_height − 96).
- Agent beta's card appears in the middle (y = tab_height − 184).
- Agent gamma's card appears at the top (y = tab_height − 272).
- Each card shows a colored 32x32 avatar square: blue (alpha), green (beta),
  orange (gamma).
- Each card shows bold agent name text and "Last active: now" beneath it.
- No card overlaps any other card.

**Fail criteria:**
- Any `MutationBatch` is rejected.
- Fewer than 3 cards visible.
- Cards overlap or appear at incorrect positions.
- Avatar colors are wrong or missing.
- Text is garbled, absent, or shows wrong agent names.

---

## Step 2 — Verify initial visual state

**Action:**
With all 3 agents connected and cards visible, perform a visual inspection.

**Pass criteria:**
- Each card is 200px wide and 80px tall.
- All three cards are anchored to the bottom-left, 16px from left edge, 16px
  from bottom edge.
- Background of each card is a semi-transparent dark rectangle (approx
  `rgba(0.08, 0.08, 0.08, 0.78)` — dark, not fully opaque).
- Avatar icon is rendered at position (8, 24) within each card (near the
  left, vertically centered).
- Agent name is rendered in bold.
- Status line reads "Last active: now".
- Text does not overflow card bounds (ellipsis applied if too wide).
- Cards do not interfere with other UI elements (Passthrough input mode:
  clicking on the cards does NOT capture pointer events; input passes through
  to underlying content).

**Fail criteria:**
- Card background is fully opaque or wrong color.
- Avatar position is incorrect.
- Agent name is not bold.
- "Last active" text is missing.
- Pointer events on the card area do not pass through.

---

## Step 3 — Wait 30s and verify content updates

**Action:**
Wait 30 seconds without disconnecting any agent. Each agent's periodic update
loop should fire and submit a `MutationBatch` containing one `SetTileRoot`
mutation with the updated `TextMarkdownNode` content.

**Pass criteria:**
- The "Last active" line on all 3 cards updates to "Last active: 30s ago"
  (human-readable format).
- The update occurs on all 3 cards within a few seconds of the 30s mark.
- The avatar icon and background color of each card are unchanged.
- The `SetTileRoot` batch contains exactly 1 mutation (the full rebuilt node
  tree), not a partial node update.
- Card geometry (position, size, z-order) is unchanged after the update.

**Fail criteria:**
- "Last active" text does not update on any card after 30s.
- Text updates only on some cards (partial failure).
- Avatar or background is removed or changed after the update.
- Content updates cause a visible flicker or repositioning of the cards.
- Text shows a raw number (e.g., "30000ms ago") instead of "30s ago".

---

## Step 4 — Disconnect agent gamma (agent 2)

**Action:**
Kill agent gamma's session. This can be done by:
- Closing the gRPC stream (sending `SessionClose` with
  `expect_resume = false`), **or**
- Hard-killing the agent process (connection drop without graceful close).

Either method is valid for this test. The runtime must detect the disconnect
via heartbeat timeout if no graceful close is sent (3 missed heartbeats at 5s
interval = 15s detection latency maximum).

**Pass criteria:**
- Agent gamma's connection is terminated.
- Agents alpha and beta remain connected and continue their periodic update
  loops unaffected.

**Fail criteria:**
- Agent gamma cannot be killed or disconnected.
- Killing agent gamma causes agents alpha or beta to disconnect.

---

## Step 5 — Verify disconnection badge within 1s

**Action:**
Immediately after agent gamma's disconnect (or after the 15s heartbeat
timeout if using a hard kill), observe the visual state of agent gamma's card.

**Timing note:** The spec requires the badge to appear within 1 frame
(16.6ms) of the `ORPHANED` lease transition. For a manual tester, the
observable tolerance is 1 second. If using a graceful close (stream drop),
the badge must appear almost instantly. If using a hard kill, allow up to
15s for heartbeat detection then watch for badge appearance within ~1s.

**Pass criteria:**
- Agent gamma's card (top card, orange avatar) shows a disconnection badge.
  The badge is a dark semi-transparent overlay with a dim link-break icon in
  the top-left corner of the card, plus a content-dimming scrim (card content
  appears at ~70% opacity).
- The badge appears promptly: within 1 second of the `ORPHANED` event being
  observable (use runtime logs to determine the exact `ORPHANED` transition
  time if needed).
- Agents alpha's and beta's cards show NO badge — they are unaffected.
- Agent gamma's card still shows its last-committed content (avatar, name,
  "Last active: Xs ago") beneath the badge overlay. The card is frozen, not
  blanked.
- Agent gamma's card remains at its original position (y = tab_height − 272).

**Fail criteria:**
- No badge appears on agent gamma's card.
- Badge appears on alpha's or beta's card (isolation failure).
- Agent gamma's card goes blank instead of showing the badge overlay.
- The badge appears but takes more than 1 second after the observable
  `ORPHANED` transition.
- Agent gamma's card repositions or disappears immediately on disconnect.

---

## Step 6 — Wait 30s for grace period expiry and tile removal

**Action:**
Wait 30 seconds from the moment agent gamma's lease entered the `ORPHANED`
state. After the grace period expires, the runtime must transition agent
gamma's lease from `ORPHANED` to `EXPIRED` and remove the tile from the
scene graph.

**Pass criteria:**
- Agent gamma's card (the top card, orange avatar, with disconnection badge)
  disappears from the display within a few seconds of the 30s grace period
  expiry.
- After removal, only 2 cards remain visible: alpha (bottom) and beta
  (middle).
- Alpha's card remains at y = tab_height − 96 (original position).
- Beta's card remains at y = tab_height − 184 (original position).
- Neither alpha nor beta's card repositions — the runtime does NOT
  automatically reflow remaining cards to fill the gap left by gamma's removal.
- Alpha's and beta's "Last active" text continues to update every 30s
  throughout this step.

**Fail criteria:**
- Agent gamma's card is not removed after 30s grace period.
- Alpha's or beta's card repositions after gamma's card is removed.
- Alpha's or beta's card disappears alongside gamma's.
- Alpha's or beta's periodic updates stop after gamma's removal.

---

## Step 7 — Final state verification

**Action:**
After agent gamma's tile has been removed, perform a final visual inspection
of the 2 remaining cards.

**Pass criteria:**
- Exactly 2 presence cards are visible, in the bottom-left corner.
- Alpha's card: bottom position (y = tab_height − 96), blue avatar, bold
  "agent-alpha" name, updated "Last active: Xs ago" text.
- Beta's card: middle position (y = tab_height − 184), green avatar, bold
  "agent-beta" name, updated "Last active: Xs ago" text.
- Both cards remain at full opacity (no disconnection badge).
- "Last active" timestamps on both cards continue to increment every 30s.

**Fail criteria:**
- Fewer or more than 2 cards visible.
- Either remaining card is in the wrong position.
- Either remaining card has a disconnection badge.
- "Last active" text has frozen on either card.

---

## Scenario Verdict

| Step | Description | Expected |
|------|-------------|----------|
| 1 | Launch 3 agent sessions and create presence cards | 3 stacked cards visible |
| 2 | Verify initial visual state | Correct colors, text, geometry, Passthrough |
| 3 | Wait 30s, verify content updates | "Last active: 30s ago" on all 3 cards |
| 4 | Disconnect agent gamma | Gamma connection terminated |
| 5 | Badge appears within 1s | Disconnection badge on gamma only |
| 6 | Wait 30s, gamma tile removed | 2 remaining cards unchanged |
| 7 | Final state: alpha + beta remain | Both cards at original positions, updating |

**PASS:** All 7 steps meet pass criteria.
**FAIL:** Any step meets a fail criterion. Record which step(s) failed and the
observed behavior.

---

## Notes for Automated Testing

This user-test scenario has automated equivalents in:

- `tests/integration/presence_card_tile.rs` — tile geometry, node tree,
  Passthrough input mode, snapshot correctness (Steps 1–2).
- `crates/tze_hud_scene/tests/lease_lifecycle_presence_card.rs` — lease
  lifecycle, ORPHANED/EXPIRED transitions, tile removal, namespace isolation
  (Steps 4–6).
- `crates/tze_hud_runtime/src/shell/badges.rs` — disconnection badge
  rendering (Step 5).
- `crates/tze_hud_protocol/tests/heartbeat.rs` — heartbeat timeout detection
  at 15s (Step 4).

The manual test is required in addition to automated tests because it
validates:
- The visual appearance on a real display (correct colors, layout, readability).
- The Passthrough hit-test behavior under real pointer events.
- The timing of badge appearance from a human observer's perspective.
- That no visual artifacts appear during content updates (Step 3).
- That card removal is clean with no visual artifacts (Step 6).
