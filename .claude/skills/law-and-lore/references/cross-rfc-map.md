# Cross-RFC Integration Map

Load this file when working across RFC boundaries — protocol integration, field number allocation, or resolving cross-cutting concerns.

## Dependency Graph

```
RFC 0001 (Scene Contract) ← foundation for all others
  ├── RFC 0002 (Runtime Kernel) ← depends on 0001
  ├── RFC 0003 (Timing) ← depends on 0001
  ├── RFC 0004 (Input) ← depends on 0001, 0002, 0003
  └── RFC 0005 (Session Protocol) ← depends on 0001, 0002, 0003, 0004
        ├── RFC 0006 (Configuration) ← depends on 0001–0005
        ├── RFC 0007 (System Shell) ← depends on 0001–0005
        ├── RFC 0008 (Lease Governance) ← depends on 0001–0005
        └── RFC 0009 (Policy Arbitration) ← depends on 0001–0008
              └── RFC 0010 (Scene Events) ← depends on 0001–0009
  RFC 0011 (Resource Store) ← orthogonal; depends on 0001, 0002, 0005, 0008
```

## Key Integration Points

| From → To | What crosses the boundary | Where to look |
|-----------|--------------------------|---------------|
| 0005 → 0001 | `MutationBatch` messages carry `SceneId` object references | 0005 §2.2, §9 proto |
| 0005 → 0003 | `TimingHints` with `present_at_wall_us`, `expires_at_wall_us` | 0005 §2.4, §7.1 |
| 0005 → 0004 | Fields 26–29: focus/capture requests; field 34: `InputEvent`; §7.1: subscription filtering | 0005 §3.1, §3.8 |
| 0005 → 0006 | Handshake carries `requested_capabilities` from 0006 §6.3; config defines `reconnect_grace_secs` | 0005 §1.2, §10 |
| 0005 → 0007 | Fields 45–46: `SessionSuspended`/`SessionResumed` for safe mode; `DegradationNotice` | 0005 §3.7, §7.5 |
| 0005 → 0008 | `LeaseRequest`/`LeaseResponse` messages; `LeaseStateChange` subscription events | 0005 §3.3, §7.1 |
| 0008 → 0002 | Lease priority guides degradation ladder shedding order | 0002 §6, 0008 §2.2 |
| 0007 → 0009 | Safe mode = Level 1 (Safety); freeze = Level 0 (Human Override) | 0009 §4.3, §5 |
| 0009 → 0006 | Policy references `redaction_style` (0006 §2.8), `quiet_hours` (0006 §6.1), budgets (0006 §4.2) | 0009 §1.1, §5 |
| 0010 → 0005 | Scene events via subscription categories: `scene_topology`, `lease_changes`, `degradation_notices`, `zone_events` | 0010 §1.2 |
| 0011 → 0001 | `StaticImageNode.resource_id` references content-addressed `ResourceId` (BLAKE3 hash) | 0011 §1.1 |

## Resolved Cross-RFC Inconsistencies

These were caught during review rounds. Know them to avoid re-introducing drift:

1. **Clock naming** — `_wall_us` vs `_mono_us` suffixes unified in RFC 0005 Round 6
2. **Field number conflict** — `EventBatch` fields 39–40 (RFC 0004) reassigned to 43–44 (RFC 0005 Round 3)
3. **Safe mode vs revocation** — Safe mode *suspends* leases; only explicit revocation *revokes* (RFC 0008)
4. **GPU failure response** — Two-phase: safe mode entry first, then shutdown if unrecoverable (RFC 0009 §5)
5. **Redaction ownership** — Privacy owns all redaction, not chrome (RFC 0009)
6. **Grace period naming** — `reconnect_grace_period_ms` vs `reconnect_grace_secs` drift documented with cross-ref

## Quantitative Budgets (Cross-RFC)

| Metric | Target | Source |
|--------|--------|--------|
| Input-to-local-ack p99 | < 4ms | RFC 0004 DR-I1 |
| Hit-test latency | < 100us (50 tiles) | RFC 0004 DR-I2 |
| Input-to-scene-commit p99 | < 50ms (local) | RFC 0004 DR-I3 |
| Sync drift budget | < 500us | RFC 0003 §4.2 |
| Frame-time budget | 16.6ms @ 60fps | RFC 0002 |
| Compositor frame time p99 | < 8ms | RFC 0002 §3.2 |
| Event classification latency | < 5us/event | RFC 0010 DR-SE8 |
| Event delivery latency | < 100us from emission | RFC 0010 DR-SE9 |
| Policy evaluation latency | < 100us/mutation | RFC 0009 §9 |
| Heartbeat interval | 5000ms default | RFC 0005 §1.3 |
| Reconnect grace period | 30000ms default | RFC 0005 §10 |
