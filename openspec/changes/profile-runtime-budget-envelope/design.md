## Context

RFC 0006 §3.1 defines a display profile as a budget envelope that shapes what the runtime grants and enforces. The selected `DisplayProfile` is frozen in `RuntimeContext`, but the production graph currently stops short of that contract:

| Surface | Current source | Production state |
|---|---|---|
| Registered-agent override validation | `DisplayProfile::{max_tiles,max_texture_mb,max_agent_update_hz}` | Enforced while loading configuration |
| Truncation shaping bound | `DisplayProfile::max_truncation_input_bytes` | Consumed by windowed and headless compositor setup |
| Resident-session admission | `SessionConfig` / `SessionLimits` constants | Independent of `profile.max_agents`; `AdmissionController` has no production caller |
| Lease resource defaults | `ResourceBudget::default()` | Assigned by the production lease path, independent of profile and registered-agent overrides |
| Aggregate leased tiles/textures | Per-lease checks only | No process-wide profile counter consumes `max_tiles` or `max_texture_mb` |
| Scene resource store | `ResourceStoreConfig::default()` | Independent 512 MiB texture ceiling in protocol, headless, and windowed startup; the adjacent 64 MiB font setting has no production reader |
| Protocol widget asset store | `WidgetAssetStore::default()` | Independent 64 MiB total / 16 MiB per-namespace ceiling on the in-memory fallback path; normal durable registration bypasses the map but still creates resident SVG source/renderer copies |
| MCP widget asset registry | `WidgetAssetRegistry::default()` | Separate production registry retains as many as 4096 payloads at up to 16 MiB each, with no aggregate byte cap and no durable/runtime-registration bridge |
| Widget raster caches | Five process-global fixed caches | Independent 32/48 MiB caps totaling 208 MiB |
| Font residency/cache | `ResourceStoreConfig::max_font_cache_bytes` plus renderer internals | The 64 MiB config field has no production reader; retained font/glyph state has no common enforced or reported ceiling |
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

`profile.max_agents` governs resident/embodied sessions. Guest protocol capacity remains a separate control-plane safety limit because guests do not hold resident leases or consume the profile's persistent-presence pool.

The headless per-agent update-rate ceiling is 60 Hz. RFC 0006 §3.4 established that value before `DisplayProfile::headless()` implemented it in `edd5fee1`; the implementation commit explicitly cites RFC 0006. The canonical configuration OpenSpec's later 30 Hz value was introduced in `1cb0e39b` while previously omitted profile fields were being filled, without a contract rationale, and matches the RFC's separate mobile ceiling. Headless is the active CI/test parity profile, so lowering only its state-stream admission ceiling would make production-rate validation unrepresentative. The canonical configuration requirement is therefore corrected to 60 Hz.

This value governs registered-agent admission validation today and the future profile-derived session default. It is not compositor cadence: neither windowed nor headless runtime reads `max_agent_update_hz` to schedule frames, and the cadence/quiescence work remains owned by `hud-le1e0` / `hud-0jfqd`.

### 3. Keep logical and physical accounting distinct

Per-agent texture accounting remains logical: shared resources are charged in full to each referencing agent, as RFC 0011 requires. The process-wide resident-allocation ledger charges each owned allocation identity once, including separate CPU decoded bytes and GPU texture copies when both exist. A cache handle or repeated reference does not create another physical charge.

Ledger bytes are deterministic enforcement quantities, not an unverifiable promise of exact process RSS or driver heap usage. CPU owners report the retained allocation bytes they reserve; GPU owners report a documented requested-allocation size derived from texture extent/format/sample/mip data. Allocator metadata, driver padding, and shared heap overhead remain observability/calibration data rather than admission arithmetic. Every governed allocation belongs to exactly one resident class so class totals sum to the aggregate without hidden overlap.

This prevents a coordinated agent bypass without falsely claiming that logical per-agent sums equal process memory.

### 4. Keep durable storage outside resident-memory accounting

`[widget_runtime_assets]` continues to govern the durable on-disk SVG footprint. Loading/rasterizing a durable asset creates resident allocations that debit the operational resident-memory envelope; merely storing its bytes on disk does not.

### 5. Hard gate: choose the profile schema for runtime-owned resident caches

Three contract shapes were evaluated:

1. **Overload `max_texture_mb`.** Treat it as all CPU/GPU/runtime cache residency. Small schema change, but contradicts its current agent-leased-texture wording and conflates fonts/pixmaps with textures.
2. **Add an explicit aggregate resident-memory field with disjoint class sub-ceilings (recommended).** Preserve `max_texture_mb` for logical agent-leased texture authority; add `max_runtime_resident_mb` plus `max_resource_resident_mb`, `max_widget_asset_resident_mb`, `max_widget_raster_cache_mb`, and `max_font_resident_mb`, requiring sub-ceilings to fit within the aggregate. The resource class covers scene-resource retained/decoded bytes and their GPU image copies; the widget-asset class covers retained runtime SVG source copies in both gRPC and MCP ingress plus renderer registration; raster and font residency remain separate. This is explicit, observable, and extensible without hidden ratios.
3. **Keep independent fixed cache caps.** Wire admission only. Lowest code churn, but fails the requested aggregate envelope and keeps desktop-headroom assumptions.

Recommendation: option 2. Consequence: the profile schema and built-in defaults grow, and operators can lower each resident class coherently. Option 1 is harder to reason about and is semantically breaking; option 3 leaves the identified seam open. Because selecting among these changes a design contract and built-in profile semantics, canonical spec sync and implementation remain gated on owner approval. If unanswered, the default is status quo: this reviewable change artifact may merge, but do not sync its delta into canonical specs, implement it, or reinterpret `max_texture_mb`.

## Risks / Trade-offs

- **[Risk] Physical allocation accounting spans protocol, resource, compositor, and runtime crates.** → Put the neutral ledger value types/interface in an existing lower-level shared crate so dependency leaves can report reserve/release without depending back on `tze_hud_runtime`; runtime owns construction, aggregate policy, and handle injection. Do not introduce a global allocator or a dependency cycle.
- **[Risk] Double-counting CPU decoded bytes and GPU textures can be confused with duplicate logical references.** → Give ledger entries allocation identities and document the logical-versus-physical distinction in tests and telemetry.
- **[Risk] A tighter profile can reject sessions/configurations that previously started.** → Emit startup envelope values and stable structured admission errors naming both requested and profile ceilings.
- **[Risk] Cache eviction during rendering can violate resource lifetime.** → Cache classes evict only at their existing safe boundaries; no mid-frame deallocation.
- **[Risk] Built-in default values could accidentally increase memory.** → Preserve or lower current independent ceilings when choosing built-in values; add a test that the computed aggregate equals the documented profile value.

## Migration Plan

1. Obtain owner approval for option 2 and its built-in full-display/headless values.
2. Preserve the reconciled 60 Hz headless `max_agent_update_hz` authority while wiring admission; do not reuse it as compositor cadence.
3. Add profile fields, validation, and config-freeze tests without changing runtime consumers.
4. Introduce `OperationalRuntimeEnvelope`, the dependency-safe accounting contract, and production-consumer construction tests.
5. Wire session/lease admission and aggregate leased-resource enforcement.
6. Wire resource, gRPC/MCP widget-source, compositor raster/image, and font class handles one class at a time, preserving current enforced ceilings as initial values where an enforced ceiling actually exists; converge MCP registration on the durable/runtime path instead of retaining an unbounded parallel payload registry.
7. Add startup/accounting telemetry and full integration coverage, then remove independent production defaults.

Rollback is a normal commit revert before canonical spec sync. No data migration is involved; durable widget blobs remain readable because only in-memory admission changes.

## Open Questions

- Owner approval of option 2 versus options 1/3.
- Exact built-in `max_runtime_resident_mb` and per-class values for `full-display` and `headless`; current independent caps are inputs, not an upper-bound compatibility baseline, because font residency and the image/GPU path are not completely capped today.
