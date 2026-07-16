## Context

RFC 0006 §3.1 defines a display profile as a budget envelope that shapes what the runtime grants and enforces. The selected `DisplayProfile` is frozen in `RuntimeContext`, but the production graph currently stops short of that contract:

| Surface | Current source | Production state |
|---|---|---|
| Registered-agent override validation | `DisplayProfile::{max_tiles,max_texture_mb,max_agent_update_hz}` | Enforced while loading configuration |
| Truncation shaping bound | `DisplayProfile::max_truncation_input_bytes` | Consumed by windowed and headless compositor setup |
| Resident-session admission | `SessionConfig` / `SessionLimits` constants | Independent of `profile.max_agents`; `AdmissionController` has no production caller |
| Lease resource defaults | `ResourceBudget::default()` | Assigned by the production lease path, independent of profile and registered-agent overrides |
| Aggregate leased tiles/textures | Per-lease checks only | No process-wide profile counter consumes `max_tiles` or `max_texture_mb` |
| Scene resource store | `ResourceStoreConfig::default()` | Independent 512 MiB texture and 64 MiB font ceilings in protocol, headless, and windowed startup |
| Widget raster caches | Five process-global fixed caches | Independent 32/48 MiB caps totaling 208 MiB |
| Durable widget assets | `[widget_runtime_assets]` | Separately governed on-disk footprint; intentionally not resident memory |

The ownership ambiguity is real. RFC 0006 calls `max_texture_mb` the total across agent-leased surfaces, while RFC 0011 calls startup widget assets runtime-owned overhead. Reinterpreting `max_texture_mb` to include every runtime cache would change that contract and the effective built-in budgets.

## Goals / Non-Goals

**Goals:**

- Derive one immutable operational envelope at config freeze and pass it to every admission/resource/cache consumer.
- Preserve separate logical per-agent accounting and physical process-residency accounting.
- Make aggregate admission enforceable and observable, including exact source/profile identity.
- Keep durable disk footprint separate from resident CPU/GPU memory.
- Preserve current v1 Windows/headless behavior unless a tighter custom profile is selected.

**Non-Goals:**

- Cadence, quiescence, frame thresholds, or `DegradationController` wiring (owned by `hud-le1e0` / `hud-0jfqd`).
- Mobile, glasses, VR, stereo, device detection, or device-specific defaults.
- Protocol schema changes or agent-controlled budget negotiation.
- A new cache implementation strategy; this change governs existing stores and caches.

## Decisions

### 1. Freeze a typed operational envelope once

`ResolvedConfig` will retain registered-agent budget overrides and a resolved profile-owned resident-memory budget. `RuntimeContext` will expose an immutable `OperationalRuntimeEnvelope`; consumers receive that value or a shared accounting handle instead of rereading TOML or recreating defaults.

Alternatives rejected:

- Passing `DisplayProfile` directly to every crate leaks configuration representation across subsystem boundaries.
- Re-deriving limits at each call site preserves the current drift and makes arithmetic/precedence inconsistent.

### 2. Apply deterministic admission precedence

For each agent, the effective per-session default is `min(canonical runtime default, selected-profile ceiling)`. A validated registered-agent override replaces the canonical default, then remains capped by the selected profile and absolute runtime hard maximum. Aggregate profile counters are checked independently, so valid per-agent limits cannot collectively exceed the display envelope.

`profile.max_agents` governs resident sessions. Guest protocol capacity remains a separate control-plane safety limit because guests do not hold resident leases or consume the profile's persistent-presence pool.

### 3. Keep logical and physical accounting distinct

Per-agent texture accounting remains logical: shared resources are charged in full to each referencing agent, as RFC 0011 requires. The process-wide resident-memory ledger charges each actual allocation once, including separate CPU decoded bytes and GPU texture copies when both exist. A cache handle or repeated reference does not create another physical charge.

This prevents a coordinated agent bypass without falsely claiming that logical per-agent sums equal process memory.

### 4. Keep durable storage outside resident-memory accounting

`[widget_runtime_assets]` continues to govern the durable on-disk SVG footprint. Loading/rasterizing a durable asset creates resident allocations that debit the operational resident-memory envelope; merely storing its bytes on disk does not.

### 5. Hard gate: choose the profile schema for runtime-owned resident caches

Three contract shapes were evaluated:

1. **Overload `max_texture_mb`.** Treat it as all CPU/GPU/runtime cache residency. Small schema change, but contradicts its current agent-leased-texture wording and conflates fonts/pixmaps with textures.
2. **Add an explicit aggregate resident-memory field with class sub-ceilings (recommended).** Preserve `max_texture_mb` for agent-leased texture authority; add `max_runtime_resident_mb` plus `max_resource_store_mb`, `max_widget_cache_mb`, and `max_font_cache_mb`, requiring sub-ceilings to fit within the aggregate. This is explicit, observable, and extensible without hidden ratios.
3. **Keep independent fixed cache caps.** Wire admission only. Lowest code churn, but fails the requested aggregate envelope and keeps desktop-headroom assumptions.

Recommendation: option 2. Consequence: the profile schema and built-in defaults grow, and operators can lower each cache class coherently. Option 1 is harder to reason about and is semantically breaking; option 3 leaves the identified seam open. Because selecting among these changes a design contract and built-in profile semantics, implementation remains gated on owner approval. If unanswered, the default is status quo: do not merge/sync the proposed delta and do not reinterpret `max_texture_mb`.

## Risks / Trade-offs

- **[Risk] Physical allocation accounting spans resource, compositor, and runtime crates.** → Define a small project-owned ledger interface in runtime and pass class-scoped handles; do not introduce a global allocator or new dependency.
- **[Risk] Double-counting CPU decoded bytes and GPU textures can be confused with duplicate logical references.** → Give ledger entries allocation identities and document the logical-versus-physical distinction in tests and telemetry.
- **[Risk] A tighter profile can reject sessions/configurations that previously started.** → Emit startup envelope values and stable structured admission errors naming both requested and profile ceilings.
- **[Risk] Cache eviction during rendering can violate resource lifetime.** → Cache classes evict only at their existing safe boundaries; no mid-frame deallocation.
- **[Risk] Built-in default values could accidentally increase memory.** → Preserve or lower current independent ceilings when choosing built-in values; add a test that the computed aggregate equals the documented profile value.

## Migration Plan

1. Obtain owner approval for option 2 and its built-in full-display/headless values.
2. Add profile fields, validation, and config-freeze tests without changing runtime consumers.
3. Introduce `OperationalRuntimeEnvelope` and production-consumer construction tests.
4. Wire session/lease admission and aggregate leased-resource enforcement.
5. Wire resource-store and compositor cache class handles one class at a time, preserving current per-class ceilings as initial values.
6. Add startup/accounting telemetry and full integration coverage, then remove independent production defaults.

Rollback is a normal commit revert before canonical spec sync. No data migration is involved; durable widget blobs remain readable because only in-memory admission changes.

## Open Questions

- Owner approval of option 2 versus options 1/3.
- Exact built-in `max_runtime_resident_mb` and per-class values for `full-display` and `headless`; status quo independent ceilings provide the upper-bound compatibility baseline, not an automatic approval of their sum.
