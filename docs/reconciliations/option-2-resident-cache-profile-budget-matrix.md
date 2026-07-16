# Option 2 Resident-Cache Profile Budget Matrix

**Status:** Proposed owner decision for `hud-eqh2x`; prepared by `hud-8lkzw`

**Scope:** Planning and reconciliation only. This artifact does not change OpenSpec, configuration, or runtime behavior.

**Recommendation:** Approve option 2 with a **1,024 MiB full-display envelope** and a **512 MiB headless envelope**, strict disjoint class ceilings, no runtime borrowing between classes, and the conformance conditions in this artifact—including deterministic widget font loading—before enforcement.

## Decision requested

Approve, revise, or reject the exact built-in profile values below. Approval authorizes the existing `profile-runtime-budget-envelope` change to encode the values in its delta specs and implementation tasks. It does not itself authorize implementation or canonical OpenSpec sync.

| Profile | Aggregate resident | Resource resident | Widget source resident | Widget raster resident | Font resident | Unallocated slack |
|---|---:|---:|---:|---:|---:|---:|
| `full-display` | **1,024 MiB** | **512 MiB** | **192 MiB** | **256 MiB** | **64 MiB** | **0 MiB** |
| `headless` | **512 MiB** | **256 MiB** | **64 MiB** | **128 MiB** | **64 MiB** | **0 MiB** |

The configuration field names remain those proposed by the existing design: `max_runtime_resident_mb`, `max_resource_resident_mb`, `max_widget_asset_resident_mb`, `max_widget_raster_cache_mb`, and `max_font_resident_mb` ([design decision](../../openspec/changes/profile-runtime-budget-envelope/design.md#L71-L79)). “Widget source resident” in this artifact maps to `max_widget_asset_resident_mb`; the more explicit label avoids confusing retained runtime source copies with the durable widget-asset store.

## Keep the three budget domains separate

| Domain | What it governs | Unit and lifetime | Proposed change |
|---|---|---|---|
| Logical agent-leased texture budget | Aggregate decoded-resource charge across all agent-leased surfaces, including deliberate charging of shared content to every referencing agent; effective per-session/lease limits remain a separate lower-level gate | MiB charged to agent/lease identity while referenced, then summed against the profile aggregate | None. Preserve profile `max_texture_mb`: 2,048 MiB full-display and 512 MiB headless ([profile defaults](../../about/legends-and-lore/rfcs/0006-configuration.md#L921-L966), [headless defaults](../../about/legends-and-lore/rfcs/0006-configuration.md#L1030-L1053)). Current production leases still receive the independent 256 MiB `ResourceBudget::default()` until the existing envelope change wires effective profile/registered-agent limits ([lease default](../../crates/tze_hud_scene/src/types.rs#L612-L660)). |
| Durable widget store | Content-addressed widget source persisted on disk | Stored blob bytes across restarts | None. Preserve 256 MiB total and 64 MiB per agent ([durable-store contract](../../about/legends-and-lore/rfcs/0006-configuration.md#L704-L729)). Disk bytes are never charged to the resident envelope merely because the blob exists. |
| Runtime resident envelope | Retained CPU and GPU allocations owned by runtime caches and stores | Accounted resident bytes from allocation until release | Add the aggregate and four class ceilings in the matrix above. |

`max_texture_mb` is the profile's aggregate authorization/fairness budget for agent-leased surfaces, not a per-agent memory grant and not a promise that its full logical quantity can be made physically resident. Effective per-session/lease logical limits remain independently enforced below that aggregate. A publication can therefore be within both logical gates and still be rejected or rendered through a lower-residency path when a resident class is full. The existing delta already requires this separation ([configuration delta](../../openspec/changes/profile-runtime-budget-envelope/specs/configuration/spec.md#L21-L37)).

## Current baseline and compatibility consequence

Current values are independent limits, not a coherent resident envelope:

| Surface | Current/default value | What it actually retains or accounts | Compatibility consequence |
|---|---:|---|---|
| Profile wiring | No resident-cache values differ by profile | Headless and windowed startup both construct the default resource store and default gRPC widget store ([headless startup](../../crates/tze_hud_runtime/src/headless.rs#L364-L383), [windowed startup](../../crates/tze_hud_runtime/src/windowed/mod.rs#L1887-L1914)). The compositor cache constants are likewise profile-independent. | The proposed matrix creates the first exact full-display/headless resident-cache distinction. |
| Resource store | 512 MiB total decoded limit; 16 MiB raw and 64 MiB decoded per resource | `ResourceRecord.decoded_bytes` metadata is summed; the dedup store does not own the decoded buffer ([defaults](../../crates/tze_hud_resource/src/types.rs#L165-L195), [record](../../crates/tze_hud_resource/src/dedup.rs#L29-L64), [sum](../../crates/tze_hud_resource/src/dedup.rs#L199-L207)). | The proposed resource ceiling will govern actual retained CPU/GPU resource allocations, so “512 MiB” becomes stricter and more meaningful than today’s metadata reservation. Headless deliberately tightens it to 256 MiB. |
| gRPC fallback widget source | 64 MiB total, 16 MiB per namespace | Raw payload `Vec<u8>` retained by `WidgetAssetStore` ([store](../../crates/tze_hud_protocol/src/session.rs#L66-L108)). | Full-display preserves this raw-source capacity plus room for known downstream copies. Headless retains one 64 MiB class ceiling across all source owners, rather than promising the old fallback limit independently. |
| MCP widget source | Up to 4,096 entries, each up to 16 MiB | Raw payload `Vec<u8>` retained in a separate registry, with no aggregate byte ceiling ([registry](../../crates/tze_hud_mcp/src/tools.rs#L1311-L1358), [per-payload check](../../crates/tze_hud_mcp/src/tools.rs#L1440-L1452)). | The theoretical entry limits equal 64 GiB of payload, plus registry and allocator overhead. Existing workloads relying on that effectively unbounded aggregate must be rejected or migrated; it is not a compatibility baseline. MCP and gRPC join one source ledger. |
| Widget CPU raster caches | 208 MiB configured aggregate ceiling across five caches | Static 32 MiB, bound 48 MiB, text 32 MiB, primitive 48 MiB, composed 48 MiB; bytes are populated on demand, not preallocated ([cache declarations](../../crates/tze_hud_compositor/src/widget.rs#L1776-L1778), [remaining cache declarations](../../crates/tze_hud_compositor/src/widget.rs#L1957-L1964)). | Full-display raises the class to 256 MiB to include retained GPU widget textures. Headless lowers the combined class to 128 MiB, requiring smaller CPU cache shares and/or earlier no-cache rendering. |
| Font stores and caches | Nominal 64 MiB font-cache default | Raw agent font bytes are retained; the text compositor uses bundled fonts, but the widget renderer separately scans host system fonts; font-system heap and glyph atlases are uncapped ([font default](../../crates/tze_hud_resource/src/types.rs#L190-L195), [raw store](../../crates/tze_hud_resource/src/font_bytes_store.rs#L1-L17), [text owners](../../crates/tze_hud_compositor/src/text.rs#L373-L428), [widget system-font scan](../../crates/tze_hud_compositor/src/widget.rs#L1733-L1766)). | Both profiles preserve 64 MiB only on the implementation condition that widget font loading becomes deterministic or is measured and preflighted before enforcement. The current host-dependent scan is not a defensible fixed baseline. |
| Aggregate resident memory | None | MCP source, renderer source/plan copies, widget GPU textures, image GPU allocations, and glyph atlases have no common ledger ([baseline reconciliation](profile-runtime-budget-envelope.md#L27-L39)). | New admission can deny allocations that succeed today. Restart-time migration and clear telemetry are required. |

The current nominal independent sum of the resource default, gRPC fallback, five CPU raster caches, and font default is 848 MiB. That number is neither current RSS nor a safe maximum: some entries are metadata-only, while several real retained copies are uncapped.

### Derivation and confidence

| Proposed value | Derivation type | Confidence and required evidence |
|---|---|---|
| Full-display aggregate 1,024 MiB | Product-policy envelope; exact sum of proposed strict class ceilings | **Provisional.** It is a round, operator-readable ceiling, not a measured RSS or workload peak. Class-saturation and representative full-display evidence must pass before enforcement. |
| Full-display resource 512 MiB | Inherited RFC/default value, redefined from logical decoded metadata to physical residency | **Medium.** Preserves the documented number but changes its meaning; image CPU/GPU high-water and denial tests are required. |
| Full-display widget source 192 MiB | Policy reserve equal to the current 64 MiB gRPC aggregate multiplied by the three source-buffer generations reachable on the fallback path | **Medium-low.** The fallback store, renderer source, and renderer-plan source copies are code-grounded, but MCP is not yet converged and the compiled plan also owns cloned bindings and parsed primitive-plan allocations. Copy-lifetime and render-plan amplification telemetry must validate the reserve; 192 MiB does not guarantee that 64 MiB of raw source will always fit. |
| Full-display widget raster 256 MiB | Current configured CPU cache ceiling of 208 MiB plus a 48 MiB GPU allowance | **Medium-low.** CPU caps are exact, but the GPU allowance is a policy reserve, not a measured peak. Representative active-widget evidence may revise it. |
| Font 64 MiB in both profiles | Inherited RFC/default value; bundled TTF source bytes measured at 4.44 MiB | **Low until font loading is deterministic.** Approval requires bundled-only widget fonts or measured supported-host font residency with startup preflight. Glyph-atlas and shaping peaks remain to be measured. |
| Headless aggregate 512 MiB, resource 256 MiB, raster 128 MiB | Constrained-profile policy choices at one-half the corresponding full-display values | **Low-medium.** These are deliberate CI bounds, not measured suite peaks. Enforce only after the complete headless suite passes class telemetry and saturation tests. |
| Headless widget source 64 MiB | One bounded shared source class, compared with current default SVG sources of 9,703 bytes | **Medium.** Deliberately drops independent fallback-plus-copy capacity; stress tests must assert deterministic denial. |

“Provisional” does not make the matrix non-normative: it is the exact implementation target if approved. It means enforcement cannot be declared complete until the named conformance evidence confirms the target or sends an explicit revision back to the owner.

## Sizing rationale

### Full-display: 1,024 MiB

- **512 MiB resource resident** carries forward RFC 0011’s resource-store total as an actual physical class ceiling ([size defaults](../../about/legends-and-lore/rfcs/0011-resource-store.md#L524-L548)). This is intentionally independent of the 2,048 MiB full-display profile aggregate for logical agent-leased textures and of each session/lease's effective logical limit.
- **192 MiB widget source resident** is a three-source-equivalent policy reserve based on the current 64 MiB gRPC aggregate. The fallback path can retain the fallback-store payload, renderer source, and renderer-plan source together; the scene pending queue transfers rather than copies its ownership when drained ([registration queue](../../crates/tze_hud_scene/src/graph/overlay.rs#L277-L295), [runtime drain](../../crates/tze_hud_runtime/src/widget_runtime_registration.rs#L10-L22), [render-plan clone](../../crates/tze_hud_compositor/src/widget.rs#L2712-L2743)). The compiled plan also owns cloned bindings and parsed primitive-plan allocations, however, and MCP does not yet reach this path. Therefore the ceiling governs measured owned bytes rather than promising 64 MiB of raw-source capacity; conformance must measure plan amplification and temporary replacement overlap before enforcement.
- **256 MiB widget raster resident** preserves the present 208 MiB configured CPU-cache ceiling and leaves 48 MiB for retained per-instance GPU textures. The existing performance-contract scene uses 512 × 512 RGBA8 widget textures, each 1 MiB; 48 MiB would cover 16 such active textures plus 32 MiB of format/extent margin. This is a sizing example, not a maximum-widget invariant ([performance-contract size](../../crates/tze_hud_compositor/src/widget.rs#L2680-L2684)). GPU byte cost must be calculated from the requested format, extent, mip count, and sample count, not from process RSS ([texture upload](../../crates/tze_hud_compositor/src/widget.rs#L2797-L2883)).
- **64 MiB font resident** preserves the RFC 0011 default, now shared by bundled fonts, agent font copies, shaping caches, and glyph atlases. The ten checked-in TTF files enumerated by the compositor currently total 4,655,896 bytes (4.44 MiB), measured as the sum of their file lengths ([bundled-face list](../../crates/tze_hud_compositor/src/fonts.rs#L46-L118)). This leaves substantial nominal room, but the number is conditional: implementation should replace the widget renderer’s host system-font scan with the same bundled-only set. If host fonts remain supported, the owner must revise 64 MiB using measured supported-host peaks and startup preflight before enforcement.

### Headless: 512 MiB

- **256 MiB resource resident** makes the test/CI profile deliberately bounded at half the full-display resource class. It is a compatibility tightening, surfaced at startup and through admission telemetry.
- **64 MiB widget source resident** is enough for the default bundled widget SVG sources (currently 9,703 bytes, or 0.0093 MiB, measured as the sum of checked-in SVG file lengths under `assets/widget_bundles/` and `assets/widgets/`) and bounded test workloads, but it no longer preserves a separate 64 MiB gRPC store on top of renderer copies. Large-source stress tests must use smaller concurrency or expect deterministic denial.
- **128 MiB widget raster resident** provides deterministic rendering coverage while forcing smaller CPU cache shares or no-cache paths. Headless is a semantic-parity profile, not a promise to reproduce the full-display cache hit rate.
- **64 MiB font resident** stays equal to full-display because both profiles must converge on the same deterministic bundled-font floor and exercise the same shaping behavior. That is a required implementation condition, not a description of the current widget font loader.

These values are envelope defaults, not measured steady-state RSS targets. They intentionally leave OS, executable, driver, allocator fragmentation, command buffers, frame-local scratch, and uninstrumented subsystem memory outside the ledger.

## Exact accounting rules

Every retained allocation is charged to exactly one class and to the aggregate. No allocation may be charged to two classes, and no retained allocation in these classes may be omitted from the aggregate.

| Class | Included owners | Accounted bytes |
|---|---|---|
| Resource resident | Decoded image CPU buffers, uploaded image/resource GPU allocations, and other non-font resource-store retained payloads; raw/parsed fonts always belong to the font class even when a resource-store component owns them | Owned immutable slice length; owned `Vec`/`String` capacity; GPU requested allocation footprint as the sum across mip levels of `ceil(width_mip / block_width) × ceil(height_mip / block_height) × block_bytes × depth_or_layers_mip × samples`. `ResourceRecord.decoded_bytes` remains a logical charge only and does not itself consume resident bytes. |
| Widget source resident | gRPC fallback payload, MCP ingress registry payload, scene pending registration payload, renderer source, and every source-derived compiled render-plan allocation (including cloned bindings and parsed primitive-plan state) | Each simultaneously owned heap buffer by capacity or an explicitly measured allocation size. Moving an owner transfers its charge; copying creates a second charge until the old owner releases. Durable disk blobs are excluded until read into a retained runtime owner. |
| Widget raster resident | Five CPU pixmap caches and retained per-instance widget GPU textures | CPU pixmap data length plus GPU requested allocation footprint. A cache lookup or shared handle adds no charge; a distinct backing allocation does. |
| Font resident | Bundled source bytes, raw agent font store, compositor/widget font-database copies, shaping and swash caches, and glyph-atlas GPU allocations | Bundled static source length once as an explicit protected-floor policy charge; owned heap buffer capacity; explicitly wrapped cache allocation size; GPU requested allocation footprint by the same per-mip rule above. Shared backing bytes charge once. |

Allocator metadata, hash-table buckets not attributable to a retained payload, executable mappings other than the explicit bundled-font protected-floor charge, driver-private memory, and transient current-frame scratch are excluded, so `accounted_bytes` is not RSS. Transient scratch must remain bounded by existing entry dimensions and frame-working-set guards; this exclusion must not become an unbounded retention escape hatch.

The implementation should publish an accounting-rule version. Any allocation that cannot be measured by one of these rules must fail closed for caching: use a bounded no-cache/fallback path or deny it until an explicit measurement rule exists.

## Admission, eviction, and oversubscription

1. **No class borrowing or oversubscription.** A class with free space cannot lend it to another class. This can strand capacity, but it prevents an MCP/source burst from starving fonts or render-critical resources and makes the same input fail at the same class boundary across runs. V1 chooses starvation isolation and deterministic admission over opportunistic utilization. Built-in class ceilings sum exactly to the aggregate, so the aggregate is intentionally non-binding for them today; it remains the top-level configuration, reporting, and future-class invariant. Custom profiles may sum to less than the aggregate; unused slack remains inert, and startup should warn about it.
2. **Atomic reservation.** Before retaining bytes, reserve against both the class ceiling and aggregate ceiling. Commit only after allocation succeeds; roll back on failure. Release exactly once when the owning allocation is dropped or transferred.
3. **Safe eviction first.** Resource owners may evict zero-reference entries. Widget raster caches may evict least-recently-used entries and render without caching; current-frame inputs are not evictable. Font owners may evict eligible shaping/glyph caches and agent fonts with no live references, while the bundled/permanent floor remains protected.
4. **Stable fallback or denial.** If safe eviction cannot satisfy a reservation, optional caches take their existing no-cache or lower-residency path. Mandatory source/resource admission fails atomically with a stable resource-exhausted result; no partial registry or publication state survives.
5. **Logical authorization remains independent.** Effective per-session/lease logical texture limits remain enforced, and the existing envelope change adds the profile-wide aggregate `max_texture_mb` gate across agent-leased surfaces. Resident admission is an additional physical gate, never a substitution for either logical check.

The existing resource-store delta’s reserve/evict/no-cache/deny ordering remains the normative behavior to carry forward ([resource-store delta](../../openspec/changes/profile-runtime-budget-envelope/specs/resource-store/spec.md#L3-L37)).

## Required observability

Expose one snapshot schema from both headless and windowed runtimes. The current delta already requires a profile/accounting snapshot ([runtime-kernel delta](../../openspec/changes/profile-runtime-budget-envelope/specs/runtime-kernel/spec.md#L35-L43)); implementation must make it operational with:

- `profile_name`, `profile_source`, and `accounting_rule_version`;
- aggregate and per-class `ceiling_bytes`, `used_bytes`, `reserved_bytes`, `peak_used_bytes`, and `allocation_count`;
- per-owner `used_bytes` for:
  - resource CPU decode and resource/image GPU;
  - widget source gRPC fallback, MCP ingress, scene pending, renderer source, and renderer plan;
  - widget raster static, bound, text, primitive, composed, and instance GPU;
  - font bundled source, agent raw store, text font database, widget font database, shaping/swash caches, and glyph atlas;
- per-class and per-owner eviction count/bytes, admission-denial count/bytes, no-cache count, and fallback count;
- current and configured maximum durable-disk bytes, including per-agent usage, under a separate `durable_widget_store` section;
- profile aggregate `max_texture_mb`, aggregate current charge, and effective per-session/lease logical texture limits and charges under a separate `logical_texture_budget` section; and
- an explicit `accounted_bytes_are_not_process_rss: true` marker.

Startup must emit the resolved profile, all five ceilings, their sum, and whether values were built in or explicitly configured. Admission failures must identify the class, requested bytes, current use, class ceiling, aggregate use, aggregate ceiling, attempted eviction, and selected fallback or denial.

## Migration and compatibility

- Add all five fields as optional profile configuration. Omitted fields resolve to the exact built-in matrix above. Custom-profile values may lower, but must not escalate above, the selected base profile's aggregate or matching class ceiling; they must also pass `sum(class ceilings) <= aggregate` validation.
- Apply changed ceilings on restart. A hot reload that changes any resident ceiling must report `restart_required` and keep the active ledger unchanged.
- Initialize ledgers before accepting sessions or restoring durable widget registrations. Startup fails closed if permanent/bundled allocations cannot fit the selected font or source class.
- Before enabling the font ceiling, replace the widget renderer’s host system-font scan with the deterministic bundled set. If that behavior is intentionally retained, pause enforcement and return measured supported-host high-water data plus a revised/preflighted font ceiling for owner approval.
- Re-indexing the durable store does not charge blob bytes. Reading and retaining a blob must reserve widget source bytes before publication registration.
- No protobuf, wire, or durable-data migration is required. Existing disk blobs remain readable. Existing configurations that relied on unbounded MCP aggregate storage, independent duplicate limits, or current cache sizes may see eviction, no-cache rendering, or deterministic admission denial.
- Rollout order: accounting and snapshot plumbing; class owners behind shadow accounting; startup validation; enforcement in headless; enforcement in full-display; removal of shadow-only mode after conformance evidence. Shadow accounting must never be described as enforcement.

## Owner decision record

Select one outcome and record it on `hud-eqh2x`:

| Outcome | Owner entry |
|---|---|
| **Approve recommendation** | “Approve option 2: full-display `1024 = 512 resource + 192 widget source + 256 widget raster + 64 font` MiB; headless `512 = 256 + 64 + 128 + 64` MiB; strict disjoint ceilings, no borrowing; complete the stated conformance conditions, including deterministic widget font loading, before enforcement.” |
| **Revise** | Supply all ten profile/class values and whether class borrowing is allowed. Any borrowing requires a revised accounting and determinism review before implementation. |
| **Reject option 2** | Select option 1 (single aggregate only) or option 3 (retain independent ceilings) and state how unbounded MCP/source/GPU/font ownership will be controlled. |

Until an explicit owner choice is recorded, `hud-eqh2x` remains blocked and no canonical spec sync or runtime/config implementation should begin.

## Evidence boundary

This matrix reconciles the current implementation and RFC defaults; it does not claim that the proposed values have already been load-tested. Acceptance evidence for implementation must include class-saturating tests, exact reserve/release invariants, MCP and gRPC convergence, headless/full-display parity, and proof that mandatory admission is atomic. RFC 0006 defines a selected profile as an enforced budget envelope ([profile contract](../../about/legends-and-lore/rfcs/0006-configuration.md#L903-L907)); RFC 0011 distinguishes durable storage from runtime memory and logical charging ([durable/runtime separation](../../about/legends-and-lore/rfcs/0011-resource-store.md#L558-L585), [logical double charging](../../about/legends-and-lore/rfcs/0011-resource-store.md#L733-L772)).
