# Runtime Widget Asset Topology

This map documents where runtime widget SVG asset registration/upload lives in the codebase, where durable bytes are stored, how startup re-index runs, and where budget/capability checks are enforced.

## Control-Plane Entry Points

### MCP plane (`register_widget_asset`)

- Public tool declaration: `crates/tze_hud_mcp/src/lib.rs`
- Request dispatch: `crates/tze_hud_mcp/src/server.rs`
- Tool handler: `crates/tze_hud_mcp/src/tools.rs` (`handle_register_widget_asset`)

Behavior at this boundary:
- Capability gate: requires `register_widget_asset`
- Metadata-first dedup preflight by BLAKE3 hash
- Optional CRC32C transport integrity check
- Stable error codes (`WIDGET_ASSET_*`)

### Session stream plane (`WidgetAssetRegister`)

- Message handling entrypoint: `crates/tze_hud_protocol/src/session_server.rs` (`handle_widget_asset_register`)
- In-memory protocol store: `crates/tze_hud_protocol/src/session.rs` (`WidgetAssetStore`)

Behavior at this boundary:
- Capability gate: requires `register_widget_asset`
- Hash/size/checksum/SVG validation
- Transactional response via `WidgetAssetRegisterResult`

## Runtime Registration Wiring

After payload validation, runtime registration into widget lifecycle is wired through:

- `crates/tze_hud_runtime/src/widget_runtime_registration.rs`
  - `register_runtime_widget_svg_asset`
  - validates widget type/layer compatibility
  - records runtime SVG handle in widget registry
  - enqueues SVG bytes for renderer registration

This keeps stage-1 asset registration separate from stage-2 widget parameter publish.

## Durable Storage Topology

Durable store implementation:

- `crates/tze_hud_resource/src/runtime_widget_store.rs`
  - `RuntimeWidgetStore::open`
  - `RuntimeWidgetStore::put_svg`
  - on-disk content-addressed blobs + sidecars

On-disk layout under configured store root:

- `blobs/<64-hex-blake3>`: SVG payload bytes
- `meta/<64-hex-blake3>.json`: sidecar metadata (`agent_namespace`, `size_bytes`, `resource_id_hex`)

Atomicity/durability hooks:

- temp file + rename (`write_atomic`)
- best-effort parent directory sync (`sync_parent_dir`)

## Store Path Resolution (Linux/macOS/Windows)

Configuration source:

- `crates/tze_hud_config/src/raw.rs` (`[widget_runtime_assets]`)
- `crates/tze_hud_config/src/runtime_widget_assets.rs`

Resolution behavior:

- Explicit `store_path`:
  - absolute path used as-is
  - relative path resolved against provided `config_parent`
  - in config-file-driven startup, `config_parent` is the config file parent directory
  - when `config_parent` is `None`, resolution falls back to `.` (current working directory)
  - headless startup passes `None`, so explicit relative `store_path` resolves from the process working directory
- No `store_path`: platform default path

Platform defaults (`platform_default_store_path`):

- Linux: `${XDG_CACHE_HOME:-$HOME/.cache}/tze_hud/resources/runtime_widget_assets`
- macOS: `$HOME/Library/Caches/tze_hud/resources/runtime_widget_assets`
- Windows: `%LOCALAPPDATA%\tze_hud\resources\runtime_widget_assets`

## Startup Reconciliation / Re-index Path

Startup call chain:

1. Runtime startup resolves widget asset store config
   - windowed: `crates/tze_hud_runtime/src/windowed.rs`
   - headless: `crates/tze_hud_runtime/src/headless.rs`
2. Runtime opens durable store
   - `RuntimeWidgetStore::open(...)`
3. Open path runs startup re-index
   - `RuntimeWidgetStore::reindex_from_disk()`

Re-index behavior:

- scans `blobs/`
- ignores temp files (`.tmp-*`)
- verifies BLAKE3(blob content) matches id (filename)
- validates sidecar (`meta/*.json`) id + size
- re-applies current budget ceilings before admitting entry
- rebuilds in-memory hash index and byte-accounting maps

## Budget Enforcement Hooks

### Config-time budget validation

- `validate_runtime_widget_asset_budgets(...)`
- `resolve_runtime_widget_asset_store(...)`

Invariant:

- `max_agent_bytes <= max_total_bytes`, unless one side is `0` (unbounded)

### Durable-store budget enforcement

- `RuntimeWidgetStore::enforce_budgets(...)`
- errors:
  - `TotalBudgetExceeded`
  - `AgentBudgetExceeded`

### Protocol/MCP ingress budget checks

- Session stream path:
  - `handle_widget_asset_register` checks `WidgetAssetStore` `max_total_bytes` and `max_namespace_bytes`
  - returns `WIDGET_ASSET_BUDGET_EXCEEDED`
- MCP path:
  - `handle_register_widget_asset` checks per-request max bytes and registry capacity
  - returns `WIDGET_ASSET_BUDGET_EXCEEDED`

## Observability Touchpoints

Operator-observable signals:

- Stable registration error codes on MCP/session responses:
  - `WIDGET_ASSET_CAPABILITY_MISSING`
  - `WIDGET_ASSET_HASH_MISMATCH`
  - `WIDGET_ASSET_CHECKSUM_MISMATCH`
  - `WIDGET_ASSET_INVALID_SVG`
  - `WIDGET_ASSET_BUDGET_EXCEEDED`
  - `WIDGET_ASSET_STORE_IO_ERROR`
  - `WIDGET_ASSET_TYPE_INVALID`
- Startup misconfiguration surfaced from runtime bootstrap:
  - `runtime widget asset store config invalid` (windowed/headless startup path)
- Re-index behavior regression tests:
  - `survives_restart_and_rehydrates_hash_index`
  - `corrupt_blob_not_admitted_on_reindex`
  - `partial_temp_files_ignored`
  - `enforces_total_budget`
  - `enforces_per_agent_budget`

## Operations Pointers

For runbook-level validation commands and deployment context, see:

- `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`
- `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`
