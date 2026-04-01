# Exemplar Dashboard Tile — User-Test Script

**Issue**: hud-i6yd.8
**Spec**: `openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md`
**Tasks ref**: `openspec/changes/exemplar-dashboard-tile/tasks.md` §12

---

## Overview

This script defines the manual visual validation procedure for the exemplar
dashboard tile lifecycle. A tester executes 9 steps in sequence and records a
PASS or FAIL verdict for each. All 9 steps must pass for the scenario to be
considered green.

This document covers three scenarios from the spec:

1. **End-to-end lifecycle** (§12.1) — happy path from session connect through
   tile removal.
2. **Disconnect-during-lifecycle** (§12.2) — orphan badge and grace-period
   cleanup.
3. **Namespace isolation** (§12 isolation clause) — a second agent cannot
   mutate the tile.

**Prerequisites:**

- Runtime is running with a default tab open (1920×1080 or equivalent).
- The `dashboard_tile_agent` binary (or equivalent test harness) can be
  launched from the command line.
- A second agent process (intruder) can be launched independently.
- The runtime's gRPC port is accessible at `[::1]:50051` (or the configured
  port).

---

## Scenario 1 — End-to-End Lifecycle

### Step 1 — Launch dashboard agent and verify session establishment

**Action:**
Launch the dashboard tile agent process. The agent must:

1. Complete `SessionInit` and receive `SessionEstablished` with a valid
   `session_id` and namespace assignment.
2. Submit `LeaseRequest { ttl_ms: 60000, capabilities: [create_tiles,
   modify_own_tiles] }`. Confirm `LeaseResponse.granted = true`.

**Pass criteria:**
- Agent log shows `SessionEstablished` with a non-empty `session_id`.
- Agent log shows `LeaseResponse { granted: true }` with a 16-byte `lease_id`.
- Lease priority is within the agent-owned band (not chrome priority 0).

**Fail criteria:**
- `SessionInit` fails or `SessionEstablished` is not received.
- `LeaseResponse.granted = false`.
- Any authentication error.

---

### Step 2 — Resource upload and tile creation

**Action:**
The agent must:

1. Upload the 48×48 PNG icon via the resource upload path. Confirm a BLAKE3
   `ResourceId` (32 bytes) is returned.
2. Submit **Batch A** — `CreateTile` only:
   - `CreateTile` with bounds `(x=50, y=50, width=400, height=300)`,
     `z_order=100`, the namespace assigned at handshake, and `lease_id`.
   Confirm `MutationResult { accepted: true, created_ids: [<tile_id>] }`.
   (Note: `CreateTile` does not carry a client-specified `tile_id`; the
   runtime-assigned `tile_id` is returned in `created_ids[0]` and must be
   captured before the node batch can reference it.)
3. Submit **Batch B** — atomic 6-node composition using the returned `tile_id`:
   - `AddNode(None, SolidColorNode)` — background root,
     `rgba(0.07, 0.07, 0.07, 0.90)`, bounds `(0, 0, 400, 300)`.
   - `AddNode(bg, StaticImageNode)` — 48×48 icon at `(16, 16)`, fit=Contain.
   - `AddNode(bg, TextMarkdownNode)` — header `**Dashboard Agent**` at
     `(76, 20, 308, 32)`, font_size=18, white.
   - `AddNode(bg, TextMarkdownNode)` — body (status/uptime) at
     `(16, 72, 368, 180)`, font_size=14, gray.
   - `AddNode(bg, HitRegionNode)` — Refresh at `(16, 256, 176, 36)`,
     `interaction_id="refresh-button"`.
   - `AddNode(bg, HitRegionNode)` — Dismiss at `(208, 256, 176, 36)`,
     `interaction_id="dismiss-button"`.
   - `UpdateTileOpacity(1.0)`.
   - `UpdateTileInputMode(Passthrough)`.
4. Confirm `MutationResult { accepted: true }` is received for Batch B.

**Pass criteria:**
- `MutationResult.accepted = true`.
- Dashboard tile is now visible on screen at position (50, 50).
- Tile displays:
  - Dark semi-transparent background panel (`rgba(0.07, 0.07, 0.07, 0.90)`).
  - Steel-blue 48×48 icon in the top-left area of the tile.
  - Bold white header text "**Dashboard Agent**".
  - Gray body text showing status/uptime (e.g., "Status: OK\nUptime: 0s").
  - Two buttons visible at the bottom: "Refresh" (left) and "Dismiss" (right).
- Tile renders at `z_order=100` (below zone tiles, below chrome).

**Fail criteria:**
- `MutationResult.accepted = false`.
- Tile is not visible after batch submission.
- Any node is missing (fewer than 6 visible elements in the tile).
- Buttons are not visible or are positioned incorrectly.

---

### Step 3 — Verify tile visual state

**Action:**
With the tile visible, perform a detailed visual inspection.

**Pass criteria:**
- Tile is exactly 400×300 visible area (bounds 400×300).
- Tile origin is at approximately (50, 50) from the display top-left.
- Background is a dark, semi-transparent rectangle (not fully opaque).
- Icon is rendered at approximately (16, 16) within the tile, 48×48 px.
- Header text is bold, white, rendered at approximately (76, 20) within tile.
- Body text is gray, readable, rendered at approximately (16, 72).
- Refresh button bounds: approximately (16, 256) tile-local, 176×36 px.
- Dismiss button bounds: approximately (208, 256) tile-local, 176×36 px.
- No elements overflow the tile bounds.
- The tile does not overlap any chrome elements (tab bar, system indicators).

**Fail criteria:**
- Tile dimensions or position deviate significantly from spec.
- Text or icon elements overflow tile bounds.
- Background is fully opaque (no transparency).
- Chrome elements are obscured by the tile (z_order violation).

---

### Step 4 — Periodic content update

**Action:**
Wait at least 5 seconds. The agent must automatically submit a `SetTileRoot`
mutation that rebuilds the full 6-node tree with an updated body
`TextMarkdownNode` (e.g., incremented uptime counter).

Alternatively, manually trigger a content update if the agent supports it.

**Pass criteria:**
- Body text in the tile changes visibly (e.g., "Uptime: 0s" → "Uptime: 5s").
- All other node elements remain unchanged (header, icon, buttons).
- `MutationResult.accepted = true` for the update batch.
- Node count in scene graph remains 6 after the update.

**Fail criteria:**
- Body text does not update.
- Any node disappears after the update.
- `MutationResult` is rejected for the update.

---

### Step 5 — Hover visual feedback on Refresh button

**Action:**
Move the pointer over the "Refresh" button (bounds approximately display-space
`x=66, y=306, w=176, h=36`).

**Pass criteria:**
- A subtle hover tint (white overlay ~0.1 alpha) appears on the Refresh button
  within 1 frame.
- Cursor changes to a pointer cursor (`CursorStyle::Pointer`).
- No visual change on the Dismiss button while hovering Refresh.

**Fail criteria:**
- No hover feedback visible.
- Hover feedback appears on wrong button.
- Cursor does not change.

---

### Step 6 — Refresh button click and callback

**Action:**
Click (primary button down + up) on the "Refresh" button.

**Pass criteria:**
- Press darkening (multiply ~0.85) appears on Refresh button within 4ms of
  click (local feedback — no agent roundtrip required).
- Agent receives a `ClickEvent` with `interaction_id = "refresh-button"`.
- Agent immediately submits a new `SetTileRoot` content update in response.
- Body text in the tile updates visibly (timestamp or counter advances).
- Press visual clears on pointer up.

**Fail criteria:**
- No press visual feedback on click.
- Agent does not receive a `ClickEvent`.
- Body text does not update after the click.
- Press state is not cleared after pointer up.

---

### Step 7 — Dismiss button click and tile removal

**Action:**
Click on the "Dismiss" button (bounds approximately display-space
`x=258, y=306, w=176, h=36`).

**Pass criteria:**
- Press darkening appears on Dismiss button within 4ms (local feedback).
- Agent receives a `ClickEvent` with `interaction_id = "dismiss-button"`.
- Agent sends `LeaseRelease`.
- Runtime responds with `LeaseResponse { granted: true }` followed by
  `LeaseStateChange { previous_state: "ACTIVE", new_state: "RELEASED" }`.
- Dashboard tile disappears from the screen immediately.
- Scene graph contains 0 tiles and 0 nodes after removal.

**Fail criteria:**
- No press visual feedback on click.
- Agent does not receive a `ClickEvent` with `dismiss-button`.
- Tile remains visible after LeaseRelease.
- `LeaseRelease` returns `granted: false`.

---

## Scenario 2 — Disconnect During Lifecycle

### Step 8 — Disconnect after tile creation (orphan badge)

**Action:**
Repeat Steps 1–3 to create a new dashboard tile (with a fresh agent
process). Once the tile is visible, abruptly terminate the agent process
(do not send `SessionClose` or `LeaseRelease` — simulate ungraceful
disconnect by killing the process).

Wait up to 15 seconds (3 missed heartbeats at 5s each) for the runtime to
detect the disconnect.

**Pass criteria:**
- Within approximately 15 seconds of disconnect, a disconnection badge
  appears overlaid on the tile (per spec: "disconnection badge within 1 frame
  of detection").
- The tile itself is frozen at its last known state (body text does not
  change).
- The tile is NOT removed immediately — it persists during the 30-second
  grace period.
- Scene graph still contains the tile.

After 30 additional seconds (grace period expiry):

- The tile is automatically removed from the screen.
- Scene graph contains 0 tiles after grace period expiry.

**Fail criteria:**
- Disconnection badge does not appear within ~15 seconds.
- Tile is removed immediately without the badge/grace period.
- Tile persists after 30-second grace period.
- Badge appears on the wrong tile.

---

## Scenario 3 — Namespace Isolation

### Step 9 — Second agent cannot mutate dashboard tile

**Action:**
While the dashboard tile is active (connected agent, tile visible), launch a
second agent process (intruder) with different credentials:

1. Intruder completes `SessionInit` and acquires a lease with
   `[create_tiles, modify_own_tiles]`.
2. Intruder attempts to send a `MutationBatch` targeting the dashboard tile's
   `tile_id` (obtained by observing the `MutationResult.created_ids` from Step
   2 above) using the intruder's `lease_id`.

**Pass criteria:**
- `MutationResult.accepted = false` is returned to the intruder.
- `error_code` indicates `NAMESPACE_MISMATCH` or `LEASE_NOT_FOUND`.
- The dashboard tile is visually unchanged.
- The intruder cannot delete or modify the tile.

**Fail criteria:**
- `MutationResult.accepted = true` for the intruder's mutation.
- Dashboard tile changes visually after the intrusion attempt.

---

## Automated Test Equivalents

This user-test scenario has automated equivalents in:

- `tests/integration/dashboard_tile_lifecycle.rs` — all 8 headless test
  functions covering lifecycle, disconnect, namespace isolation, and the three
  headless spec scenarios.
- `crates/tze_hud_protocol/tests/dashboard_tile_agent.rs` — gRPC session and
  lease acquisition.
- `crates/tze_hud_protocol/tests/dashboard_tile_agent_callbacks.rs` — button
  activation events and dismiss/refresh callbacks.
- `tests/integration/dashboard_tile_creation.rs` — resource upload, atomic
  tile creation, node tree verification, periodic content update.
- `tests/integration/dashboard_tile_input.rs` — HitRegionNode local feedback,
  focus cycling, NAVIGATE_NEXT + ACTIVATE.
- `tests/integration/disconnect_orphan.rs` — heartbeat timeout, orphan state,
  reconnect within grace period.

Run all headless tests without a display server or GPU:

```bash
cargo test --test dashboard_tile_lifecycle
cargo test --test dashboard_tile_creation
cargo test --test dashboard_tile_input
cargo test --test disconnect_orphan
cargo test -p tze_hud_protocol
```
