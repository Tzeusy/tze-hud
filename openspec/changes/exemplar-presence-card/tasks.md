## 1. Avatar Resources

- [ ] 1.1 Create 3 placeholder 32x32 PNG avatar images (solid blue, solid green, solid orange) in test fixtures directory
- [ ] 1.2 Write resource upload helper that uploads a PNG and returns the ResourceId (BLAKE3 hash)

## 2. Presence Card Tile Creation

- [ ] 2.1 Implement presence card tile builder: constructs CreateTile mutation with 200x80 bounds, computed y-offset for agent index (0/1/2), z_order (100+index), plus separate UpdateTileOpacity (1.0) and UpdateTileInputMode (Passthrough) mutations. Note: CreateTile only carries tab_id, namespace, lease_id, bounds, z_order.
- [ ] 2.2 Implement node tree builder: constructs SetTileRoot with 3-node tree — SolidColorNode (Rgba { r: 0.08, g: 0.08, b: 0.08, a: 0.78 }, full-tile bounds), StaticImageNode (32x32 at (8,24), ResourceId reference), TextMarkdownNode ("**AgentName**\nLast active: now", 14px, Rgba { r: 0.94, g: 0.94, b: 0.94, a: 1.0 }, at (48,8), 144x64, Ellipsis overflow). Alternatively, use 3x AddNode mutations.
- [ ] 2.3 Submit MutationBatch containing: CreateTile + SetTileRoot (with full node tree) + UpdateTileOpacity + UpdateTileInputMode — and verify batch is accepted

## 3. Lease Lifecycle

- [ ] 3.1 Implement lease request with ttl_ms 120000, capabilities [create_tiles, modify_own_tiles]. Note: `AutoRenew` renewal policy is a server-side concern, not a LeaseRequest proto field.
- [ ] 3.2 Verify auto-renewal fires at 75% TTL (90s) — confirm LeaseResponse with granted = true is received
- [ ] 3.3 Verify tile creation is rejected when no lease is active (LeaseExpired / LeaseNotFound error)

## 4. Periodic Content Updates

- [ ] 4.1 Implement 30-second content update loop: constructs SetTileRoot mutation with updated node tree (only TextMarkdownNode content changes; full tree is rebuilt). Note: there is no ReplaceNode variant — use SetTileRoot.
- [ ] 4.2 Verify content updates produce valid MutationBatch (1 mutation, SetTileRoot) and are accepted by the runtime
- [ ] 4.3 Verify human-friendly time formatting: "now" at 0s, "30s ago" at 30s, "1m ago" at 60s, "2m ago" at 120s

## 5. Multi-Agent Coexistence Test

- [ ] 5.1 Write gRPC integration test: 3 concurrent agent sessions each create a presence card tile with unique namespace, avatar color, y-offset, and z_order
- [ ] 5.2 Verify all 3 tiles are present in the scene graph after creation (query SceneSnapshot)
- [ ] 5.3 Verify no tile bounds overlap (distinct y-offsets with 8px gaps)
- [ ] 5.4 Verify all 3 agents can submit concurrent content updates without interference

## 6. Disconnect and Orphan Handling Test

- [ ] 6.1 Write disconnect test: agent 2 drops connection (close gRPC stream)
- [ ] 6.2 Verify agent 2's lease transitions to ORPHANED after heartbeat timeout (15s)
- [ ] 6.3 Verify disconnection badge is signaled on agent 2's tile (LeaseStateChange event)
- [ ] 6.4 Verify agent 2's tile remains in scene graph during grace period (frozen at last state)
- [ ] 6.5 Verify agent 2's tile is removed after grace period expiry (30s) — lease transitions to EXPIRED
- [ ] 6.6 Verify agents 0 and 1 continue operating normally throughout disconnect/cleanup sequence

## 7. User-Test Scenario

- [ ] 7.1 Write user-test script that launches 3 agent sessions, waits for visual confirmation of stacked cards
- [ ] 7.2 Add content update verification step (wait 30s, confirm text updates)
- [ ] 7.3 Add disconnect step (kill agent 2 session, verify staleness badge appears)
- [ ] 7.4 Add cleanup verification step (wait 30s, verify agent 2's card is removed, agents 0 and 1 remain)
- [ ] 7.5 Document pass/fail criteria for manual visual inspection
