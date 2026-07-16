## ADDED Requirements

### Requirement: Shared Physical Resident-Memory Ledger
The resource store and compositor cache classes MUST debit a shared physical resident-memory ledger derived from the frozen operational envelope. Each actual CPU or GPU allocation MUST be charged once to its named class, even when multiple handles or agents reference it. Distinct resident copies of the same content, including a decoded CPU image and its GPU texture, MUST each be charged because each consumes memory. Logical per-agent resource accounting MUST remain separate and MUST continue to double-count shared resources as required by the per-agent budget contract.

Source: RFC 0011 §4.3, §7.5, §8, §11; RFC 0006 §3.1
Scope: v1-mandatory

#### Scenario: Duplicate handle does not duplicate physical charge
- **WHEN** a second cache handle refers to an already-accounted resident allocation
- **THEN** physical resident-memory usage MUST remain unchanged while logical ownership/reference accounting MAY change

#### Scenario: CPU and GPU copies are separate allocations
- **WHEN** one resource has both decoded CPU bytes and a GPU texture copy resident
- **THEN** the ledger MUST charge both actual allocations to their named classes

### Requirement: Cache Admission Under Aggregate Pressure
Before admitting a resource-store or compositor-cache allocation, the runtime MUST enforce both the class sub-ceiling and aggregate runtime-resident-memory ceiling. A cache class MUST attempt eviction using its existing safe eviction policy before denying cache admission. If no safe eviction can create headroom, optional cache work MUST proceed uncached or at the already-specified lower-quality path; mandatory resource admission MUST fail with a structured budget error. No eviction MAY free an allocation referenced by the frame currently being rendered.

Source: RFC 0011 §6.6, §7.5, §8.4; heart-and-soul/failure.md §Degradation axes
Scope: v1-mandatory

#### Scenario: Optional widget raster cache is full
- **WHEN** a widget raster result would exceed its class or aggregate resident-memory ceiling and safe LRU eviction cannot create headroom
- **THEN** rendering MUST continue without retaining the optional cache entry and accounting MUST remain within both ceilings

#### Scenario: Mandatory resource cannot fit
- **WHEN** a mandatory decoded resource would exceed its class or aggregate ceiling after safe eviction
- **THEN** resource admission MUST fail with a stable structured budget error and MUST NOT partially debit the ledger

#### Scenario: Current-frame allocation is eviction candidate
- **WHEN** the least-recently-used allocation is referenced by the frame currently being rendered
- **THEN** eviction MUST be deferred and a different safe candidate or the no-cache/denial path MUST be used
