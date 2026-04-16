## ADDED Requirements

### Requirement: Publish-To-Tile Contract Is Additive And Lease-Gated
`PublishToTileMutation` MUST coexist with `SetTileRootMutation` and MUST NOT replace it. `PublishToTileMutation` addresses tiles through persistent element identity resolution and MUST enforce active-lease validation on the resolved tile namespace.
Scope: v1-mandatory

#### Scenario: SetTileRoot and PublishToTile coexist
- **WHEN** a client mutates a tile by raw `tile_id`
- **THEN** `SetTileRootMutation` MUST remain valid and supported
- **AND** `PublishToTileMutation` MUST be available as the element-identity-addressed path

#### Scenario: PublishToTile validates lease
- **WHEN** `PublishToTileMutation` targets an element whose resolved tile namespace is not actively leased by the caller
- **THEN** the runtime MUST reject the mutation with `LeaseNotFound` when no lease exists for the resolved tile namespace
- **AND** the runtime MUST reject the mutation with `LeaseExpired` when a lease exists but is not active
- **AND** the runtime MUST reject the mutation with `CapabilityMissing` when an active lease exists but is not held by the caller

#### Scenario: Runtime override stage is applied before commit
- **WHEN** a valid mutation batch includes `PublishToTileMutation`
- **THEN** user geometry overrides from element identity store MUST be applied in runtime override stage
- **AND** resulting state MUST be atomically committed after that stage

---

### Requirement: Element Store Deletion Is Post-v1
v1 element identity store behavior MUST be monotonic growth. Explicit user deletion is deferred to a post-v1 layout management surface.
Scope: v1-mandatory

#### Scenario: No explicit deletion surface in v1
- **WHEN** v1 runtime is operating with element identity store enabled
- **THEN** store entries MUST persist and MUST NOT require an in-v1 explicit deletion workflow

#### Scenario: Future deletion path is acknowledged
- **WHEN** deletion semantics are documented
- **THEN** documentation MUST state that explicit deletion is delivered by a post-v1 layout-management UI
