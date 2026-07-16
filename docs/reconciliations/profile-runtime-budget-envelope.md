# Profile-Driven Operational Runtime Budget Reconciliation

Issue: `hud-k3lfx`

Authority follow-up: `hud-1utwb`

OpenSpec change: `profile-runtime-budget-envelope`

Scope: configuration, runtime admission, scene resources, resource store, and compositor caches. Cadence/quiescence and device-specific implementation are excluded.

## Finding

The selected display profile is a frozen configuration object, not yet one operational authority. Production consumes `max_truncation_input_bytes` and validates configured agent ceilings, but session/lease defaults and memory-owning stores recreate independent defaults. This confirms `hud-48s45` F5/F6 and identifies the exact handoff seams below.

## Production Consumer Inventory

| Contract value or resource | Source today | Production consumer(s) | Current effect | Required owner after approval |
|---|---|---|---|---|
| Resolved profile identity and values | [`DisplayProfile`](../../crates/tze_hud_scene/src/config/mod.rs) and [`ConfigLoader::freeze`](../../crates/tze_hud_config/src/loader.rs) | [`RuntimeContext`](../../crates/tze_hud_runtime/src/runtime_context.rs) | Frozen and shared | `OperationalRuntimeEnvelope` derived once at freeze |
| Registered-agent `max_tiles`, `max_texture_mb`, `max_update_hz` | [`RawAgentConfig`](../../crates/tze_hud_config/src/raw.rs) | [`loader::validate_agent_profile_ceilings`](../../crates/tze_hud_config/src/loader.rs) / [`agents::validate_agents`](../../crates/tze_hud_config/src/agents.rs) | Validated, then discarded; `ResolvedConfig` retains capabilities only | Retained effective override map in resolved config/envelope |
| `max_truncation_input_bytes` | Selected profile | [`WindowedRuntime` compositor initialization](../../crates/tze_hud_runtime/src/windowed/mod.rs); [`HeadlessRuntime::new`](../../crates/tze_hud_runtime/src/headless.rs) | Governs truncation shaping in both production runtimes | Preserve as direct profile-derived render bound |
| `max_agents` | Selected profile | No production admission consumer found | Informational/config-only | Resident-session limit in admission/session server |
| `max_tiles` | Selected profile | Config validation only | Does not cap aggregate leased tiles or default leases | Aggregate scene counter plus per-agent default ceiling |
| `max_texture_mb` | Selected profile | Config validation only | Does not configure resource store, lease defaults, or aggregate residency | Aggregate agent-leased texture authority; physical runtime memory remains separate |
| `max_agent_update_hz` | Selected profile | Config validation only | Does not configure mutation intake/session defaults | Effective per-session update budget and aggregate admission input |
| Per-session resource defaults | [`ResourceBudget::default()` and runtime session constants](../../crates/tze_hud_runtime/src/session.rs) | Production [`handle_lease_request`](../../crates/tze_hud_protocol/src/session_server/leases.rs) calls [`SceneGraph::grant_lease_with_priority`](../../crates/tze_hud_scene/src/graph/leases.rs), which stores `ResourceBudget::default()` | Every lease receives 8 tiles, 256 MiB, 30 Hz regardless of selected profile/agent override | Effective profile/override-derived budget passed into lease grant |
| Session admission limits | [`SessionLimits`](../../crates/tze_hud_runtime/src/admission.rs) and protocol [`SessionConfig`](../../crates/tze_hud_protocol/src/session_server/config.rs) constants | `AdmissionController` has no production caller; protocol stream manages sessions independently | Profile `max_agents` is not enforced; runtime admission implementation is effectively test-only | One production admission path sourced from envelope |
| Mutation budget enforcer | [`MutationIntakeStage`](../../crates/tze_hud_runtime/src/pipeline.rs) owns [`BudgetEnforcer`](../../crates/tze_hud_runtime/src/budget.rs) | Windowed/headless create `FramePipeline`, but production session establishment does not register an effective profile-derived session budget | Component behavior exists without end-to-end admission wiring | Register/unregister from production session lifecycle |
| Scene resource store | [`ResourceStoreConfig::default()`](../../crates/tze_hud_resource/src/types.rs) (512 MiB decoded textures plus a declarative 64 MiB font setting) | Constructed independently by protocol [`HudSessionImpl`](../../crates/tze_hud_protocol/src/session_server/service.rs), [`WindowedRuntime`](../../crates/tze_hud_runtime/src/windowed/mod.rs), and [`HeadlessRuntime`](../../crates/tze_hud_runtime/src/headless.rs) | Selected profile cannot lower or account this store | Class-scoped envelope limits and shared resident-allocation ledger |
| Protocol in-memory widget asset store | [`WidgetAssetStore::default()`](../../crates/tze_hud_protocol/src/session.rs) (64 MiB total, 16 MiB per namespace) | Windowed/headless `SharedState`; populated by the no-durable-store fallback path | Independent fixed resident ceiling; the normal durable-store path bypasses this map, but runtime registration still creates resident SVG source/renderer copies | Explicit widget-asset-residency class covering every retained source copy, or elimination of duplicate residency |
| MCP widget asset registry | [`WidgetAssetRegistry::default()`](../../crates/tze_hud_mcp/src/tools.rs) (4096 entries, 16 MiB per request, no aggregate byte cap) | Production [`McpServer`](../../crates/tze_hud_mcp/src/server.rs) owns a separate registry | Retains payload bytes for dedup but does not use the durable store or runtime widget-registration/render path | Inject the same widget-asset-residency handle and durable/runtime registration path, or remove the payload-retaining duplicate |
| Durable runtime widget store | [`[widget_runtime_assets]` resolution](../../crates/tze_hud_config/src/runtime_widget_assets.rs) → [`RuntimeWidgetStoreConfig`](../../crates/tze_hud_resource/src/runtime_widget_store.rs) | Windowed/headless startup | Correctly governed by separate total/per-agent disk ceilings | Remain separate; loaded/rasterized copies debit resident ledger |
| Widget raster caches | Five `OnceLock<Mutex<...>>` caches in [`tze_hud_compositor::widget`](../../crates/tze_hud_compositor/src/widget.rs) | Widget rasterization hot path | Fixed 32/48 MiB class caps, 208 MiB total; no profile input or common accounting | Profile-owned class ceiling plus aggregate ledger; retain safe LRU/no-cache behavior |
| Font residency/cache | [`ResourceStoreConfig::max_font_cache_bytes`](../../crates/tze_hud_resource/src/types.rs) and renderer/font internals | No production read of `max_font_cache_bytes` found; font bytes and renderer glyph/font state are retained independently | The configured 64 MiB value is declarative only, not an enforced production ceiling | Profile-owned font-residency class with enforced, ledger-visible usage |
| Image/GPU texture cache | [`renderer::image_cache`](../../crates/tze_hud_compositor/src/renderer/image_cache.rs) plus scene references | Windowed/headless render paths | Evicts unused scene images, but has no profile-visible accounted-byte ledger | Charge each owned CPU/GPU allocation identity once while retaining logical per-agent charges |
| Frame cadence/degradation thresholds | CLI/env `opts.fps`; independent runtime constants | Windowed loop / degradation module | Outside this issue | `hud-le1e0` / `hud-0jfqd`; deliberately untouched |
| Headless `max_agent_update_hz` value | RFC 0006 §3.4 and `DisplayProfile::headless()` define 60; canonical configuration OpenSpec is corrected from 30 to 60 | Config validation today; future envelope admission after approval | Per-agent state-stream admission ceiling, distinct from compositor cadence | Preserve 60 as the profile authority and keep cadence/quiescence separate |

Repository-wide absence checks found no production caller of `AdmissionController::{new,with_limits,admit}` outside its own tests, no production consumer of the selected profile's `max_agents`, `max_tiles`, `max_texture_mb`, or `max_agent_update_hz` beyond configuration validation, no production read of `ResourceStoreConfig::max_font_cache_bytes`, and no production bridge from the MCP widget registry to durable/runtime widget registration. These are reachability gaps, not missing unit tests.

## Headless Update-Rate Authority

The authoritative headless `max_agent_update_hz` is **60 Hz**. This is a contract correction, not a new performance-budget choice:

- RFC 0006 §3.4 has defined 60 Hz for the headless profile since the profile contract was introduced. Its separate mobile profile defines 30 Hz for aggressive coalescing.
- `DisplayProfile::headless()` gained the field as 60 Hz in `edd5fee1`; that commit explicitly records full-display=60, headless=60, mobile=30 "per RFC 0006."
- The canonical configuration spec originally omitted this field. Commit `1cb0e39b` later filled omitted profile fields and inserted headless=30 without a rationale or corresponding implementation change. The value conflicts with its cited RFC source and matches the distinct mobile value.
- Headless is the CI/test parity profile. A 30 Hz admission ceiling would prevent headless validation of the 60 Hz state-stream ceiling accepted by the production full-display profile without evidence of a separate CI constraint.

The exact production consumer is configuration validation: `profile_ceiling_for_validation()` selects `DisplayProfile::headless()`, and `validate_agent_profile_ceilings()` rejects a registered agent only when `max_update_hz` exceeds that ceiling. `ConfigLoader::freeze()` retains the resolved value in `ResolvedConfig` / `RuntimeContext`, but no headless or windowed scheduling path reads `max_agent_update_hz`. Fixtures and application configs do not override the built-in value. Consequently, 60 Hz does not require or authorize a 60 fps idle loop; compositor cadence/quiescence remains outside this change.

## Ownership Boundary

The reconciliation keeps three budgets distinct:

1. **Logical per-agent/lease budget.** Shared resources are double-charged to each referencing agent, preventing coordinated bypass.
2. **Physical resident-allocation budget.** Each owned CPU/GPU allocation identity is charged once using a deterministic accounted-byte size. Distinct decoded and GPU copies are separate charges. This is an enforcement ledger, not a claim to measure allocator metadata, driver padding, shared GPU heaps, or process RSS exactly.
3. **Durable disk budget.** Content-addressed widget blobs on disk are separately governed and create a resident charge only when loaded/rasterized.

This boundary preserves RFC 0011 semantics while giving the runtime an honest, enforceable view of governed residency.

## Contract Decision Gate

The existing RFCs do not authorize silently folding runtime-owned font/widget caches into `max_texture_mb`: RFC 0006 describes that value as agent-leased texture memory, and RFC 0011 explicitly calls startup widget resources runtime-owned overhead.

Options:

1. Overload `max_texture_mb` to cover all resident CPU/GPU caches. Consequence: smallest schema, most semantic ambiguity, breaking reinterpretation.
2. Add an explicit aggregate runtime-resident budget with disjoint resource/image, widget-asset source, widget-raster, and font-residency sub-ceilings. Consequence: larger but explicit profile schema and clean accounting. **Recommended.**
3. Leave fixed independent cache caps and wire only admission. Consequence: lower churn but no aggregate envelope; F5 remains unresolved.

Default if unanswered: retain status quo and do not sync the delta into canonical specs or implement the proposed contract. The OpenSpec change artifact and design remain mergeable as reviewable seam artifacts; no runtime behavior changes in this issue.

## Verification Contract

After approval, completion requires behavior-executing tests that prove:

- a tighter custom profile changes production session and lease limits;
- valid per-agent limits cannot collectively exceed aggregate profile ceilings;
- logical shared-resource charges and physical allocation charges remain distinct;
- every governed memory-owning store/cache, including both protocol-plane widget registries and retained widget SVG source bytes, reports its disjoint class and aggregate usage from the enforcement ledger;
- durable-only bytes do not count as resident;
- cache pressure evicts or proceeds uncached without freeing current-frame resources; and
- full workspace `check`/`clippy` plus relevant integration/headless suites remain green.
