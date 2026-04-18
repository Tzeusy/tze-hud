## 1. Exemplar Agent Scaffold

- [ ] 1.1 Create the exemplar agent binary/test harness (e.g., `examples/dashboard_tile_agent.rs` or equivalent integration test module) with gRPC client setup connecting to the HudSession bidirectional stream
- [ ] 1.2 Implement session establishment: send SessionInit, receive SessionEstablished, verify session_id and namespace assignment

## 2. Lease Acquisition

- [ ] 2.1 Implement LeaseRequest with ttl_ms = 60000, capabilities = [create_tiles, modify_own_tiles], and lease_priority. Note: renewal policy (`AutoRenew`) and resource budgets are server-side / Rust-layer concerns, not LeaseRequest proto fields.
- [ ] 2.2 Verify LeaseResponse granted = true and store the returned LeaseId for subsequent mutations
- [ ] 2.3 Add test: lease request without required capabilities is denied

## 3. Resource Upload

- [ ] 3.1 Upload a 48x48 PNG icon image via the resource upload path and capture the returned ResourceId (BLAKE3 content hash)
- [ ] 3.2 Add test: referencing an un-uploaded ResourceId in StaticImageNode is rejected with ResourceNotFound

## 4. Atomic Tile Creation Batch

- [ ] 4.1 Build the MutationBatch containing: CreateTile (400x300 at (50,50), z_order=100), SetTileRoot (with full 6-node tree: SolidColorNode, StaticImageNode, 2x TextMarkdownNode, 2x HitRegionNode), followed by separate UpdateTileOpacity and UpdateTileInputMode mutations. Note: CreateTile only carries tab_id, namespace, lease_id, bounds, z_order — opacity and input_mode must be set via separate mutations.
- [ ] 4.2 Submit the batch and verify MutationResult success (fields: batch_id, accepted, created_ids, error_code, error_message)
- [ ] 4.3 Add test: verify the scene graph contains the tile with all 6 nodes in correct tree order after commit
- [ ] 4.4 Add test: partial failure (e.g., one AddNode with width=0) rejects the entire batch atomically — no tile appears

## 5. Intra-Tile Compositing Verification

- [ ] 5.1 Add test: verify painter's model ordering — SolidColorNode renders first (background), then StaticImageNode (icon), then TextMarkdownNodes (header, body), then HitRegionNodes (buttons on top)
- [ ] 5.2 Add test: verify z_order = 100 places the tile in the agent-owned band (below ZONE_TILE_Z_MIN = 0x8000_0000)
- [ ] 5.3 Add test: verify chrome layer elements (tab bar, disconnection badges) render above the dashboard tile

## 6. Periodic Content Update

- [ ] 6.1 Implement a periodic task (every 5 seconds) that submits a SetTileRoot mutation with an updated node tree (only the body TextMarkdownNode content changes; the full tree is rebuilt). Note: there is no SetTileRoot variant — use SetTileRoot to swap the entire node tree atomically.
- [ ] 6.2 Add test: content update succeeds when lease is ACTIVE and the TextMarkdownNode reflects the new content
- [ ] 6.3 Add test: content update is rejected when lease has expired (LeaseExpired error)

## 7. Input Capture and Local Feedback

- [ ] 7.1 Add test: injected PointerDownEvent at coordinates within "Refresh" HitRegionNode bounds produces a NodeHit with interaction_id = "refresh-button"
- [ ] 7.2 Add test: HitRegionLocalState.pressed is set to true within p99 < 4ms of PointerDownEvent arrival (headless, synthetic injection)
- [ ] 7.3 Add test: HitRegionLocalState.hovered is set on PointerEnterEvent and cleared on PointerLeaveEvent for both buttons
- [ ] 7.4 Add test: PointerUpEvent with release_on_up = true clears pressed state and releases pointer capture
- [ ] 7.5 Add test: focus ring is rendered when focus transfers to a HitRegionNode via Tab key or click

## 8. Agent Callbacks on Button Activation

- [ ] 8.1 Implement agent-side event handler: receive EventBatch from gRPC stream, extract ClickEvent or CommandInputEvent(ACTIVATE), match on interaction_id
- [ ] 8.2 On interaction_id = "refresh-button": trigger an immediate content update (MutationBatch with SetTileRoot)
- [ ] 8.3 On interaction_id = "dismiss-button": send LeaseRelease and verify the tile is removed from the scene
- [ ] 8.4 Add test: click on Refresh dispatches ClickEvent with correct interaction_id, tile_id, node_id to agent
- [ ] 8.5 Add test: ACTIVATE command on focused Dismiss button dispatches CommandInputEvent with action = ACTIVATE and interaction_id = "dismiss-button"
- [ ] 8.6 Add test: all buttons are reachable and activatable without a pointer (NAVIGATE_NEXT + ACTIVATE)

## 9. Focus Cycling

- [ ] 9.1 Add test: Tab key (NAVIGATE_NEXT) cycles focus from Refresh to Dismiss to next tile (or wraps to Refresh if only tile)
- [ ] 9.2 Add test: Shift+Tab (NAVIGATE_PREV) cycles focus in reverse order
- [ ] 9.3 Add test: FocusGainedEvent and FocusLostEvent are dispatched to the agent on focus transitions between the two buttons

## 10. Lease Governance Lifecycle

- [ ] 10.1 Add test: auto-renewal fires at 75% TTL (45 seconds) — agent receives LeaseResponse with granted = true and updated expiry
- [ ] 10.2 Add test: agent disconnect transitions lease to ORPHANED, tile is frozen, disconnection badge appears within 1 frame
- [ ] 10.3 Add test: agent reconnect within 30-second grace period restores ACTIVE lease and clears badge
- [ ] 10.4 Add test: grace period expiry (no reconnect within 30 seconds) transitions lease to EXPIRED and removes tile
- [ ] 10.5 Add test: explicit LeaseRelease transitions lease to RELEASED and removes tile cleanly

## 11. Namespace Isolation

- [ ] 11.1 Add test: a second agent session cannot mutate or delete the dashboard tile (rejected with CapabilityMissing or LeaseNotFound)
- [ ] 11.2 Add test: the dashboard agent cannot mutate tiles owned by another namespace

## 12. Full Lifecycle User-Test

- [ ] 12.1 Implement end-to-end user-test scenario: session connect -> lease request -> resource upload -> atomic tile creation -> content update -> Refresh click -> Dismiss click -> tile removal
- [ ] 12.2 Add test: disconnect-during-lifecycle triggers orphan path with badge, cleaned up after grace period
- [ ] 12.3 Verify all headless tests pass without display server or GPU
