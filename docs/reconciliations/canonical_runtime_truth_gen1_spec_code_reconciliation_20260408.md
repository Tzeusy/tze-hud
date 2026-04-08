# Canonical Runtime Truth Spec-to-Code Reconciliation (Gen-1, 2026-04-08)

Issue: `hud-7yaf.6`  
Epic: `hud-7yaf`  
Scope: Reconcile epic acceptance criteria and cited spec requirements against merged sibling beads (`hud-7yaf.1` ... `hud-7yaf.5`) and current `main` implementation.

## Inputs Reviewed

- Epic + child beads: `bd show hud-7yaf.6 --json` (including dependency metadata)
- Prior report artifact: `docs/reconciliations/canonical_runtime_truth_convergence_report_20260408.md`
- Specs:
  - `openspec/changes/ship-runtime-app-binary/specs/runtime-app-binary/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/configuration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
- Runtime/app/config/docs/CI paths listed in the sibling report and epic.

## Epic Acceptance Criteria Coverage

| Epic AC | Coverage Status | Evidence |
|---|---|---|
| 1. Canonical app production config conforms to loader and boots as blessed path | PASS | `app/tze_hud_app/config/production.toml`; `app/tze_hud_app/tests/canonical_config_schema.rs`; `app/tze_hud_app/tests/production_boot.rs`; sibling beads `hud-7yaf.1`, `hud-7yaf.2` |
| 2. CI fails if canonical app config falls back to default/headless behavior | PASS | `.github/workflows/ci.yml` job `canonical-app-production-boot`; `app/tze_hud_app/tests/production_boot.rs` asserts widget instances/types + profile-resolved zone policy (fallback-sensitive assertions); sibling bead `hud-7yaf.2` |
| 3. Windowed release startup fails closed on missing/invalid config + trivial default PSK, with explicit dev override separation | PASS | `app/tze_hud_app/src/main.rs` strict-mode gates for config presence/readability, config validation, default PSK rejection; debug-only override gate via `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP`; unit tests in same file (startup security mode + validation helpers); sibling bead `hud-7yaf.3` |
| 4. README/operator docs converge to current runtime truth and avoid stale canonical schema claims | PASS | `README.md`; `about/lay-and-land/operations/DEPLOYMENT.md`; `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`; `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`; sibling bead `hud-7yaf.4` |
| 5. Final reconciliation confirms docs/config/runtime/CI convergence against cited specs | PASS | This report (`hud-7yaf.6`) plus prior convergence report from `hud-7yaf.5` |

## Cited Spec Requirement Coverage

### `runtime-app-binary/spec.md`

1. `Requirement: Canonical Runtime Application Executable`  
Status: PASS
- Canonical non-demo binary target is explicit: `app/tze_hud_app/Cargo.toml` (`[[bin]] name = "tze_hud"`).
- Operator docs designate canonical path and distinguish demo binaries:
  - `README.md`
  - `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`
  - `about/lay-and-land/operations/DEPLOYMENT.md`

2. `Requirement: Configuration-Driven Runtime Startup`  
Status: PASS
- Runtime startup consumes config + CLI/env controls:
  - `app/tze_hud_app/src/main.rs`
  - `crates/tze_hud_runtime/src/windowed.rs`
- Deterministic endpoint enable/disable behavior by configured ports:
  - gRPC `grpc_port == 0` disables server (`start_network_services`)
  - MCP `mcp_port == 0` disables server
  - tests in `crates/tze_hud_runtime/src/windowed.rs` cover disable/enable/idempotent behavior.

3. `Requirement: Windows Artifact Identity for Automation`  
Status: PASS
- Deterministic Windows artifact naming and output paths are documented for automation:
  - `about/lay-and-land/operations/RUNTIME_APP_BINARY.md` (`## Artifact Identity`)
  - `about/lay-and-land/operations/DEPLOYMENT.md` (`## Canonical App Binary Identity`)
- Canonical artifact references are explicit and stable:
  - `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
  - `target/x86_64-pc-windows-msvc/release/tze_hud.exe`
  - `C:\\tze_hud\\tze_hud.exe`

### `configuration/spec.md` (epic-relevant subset)

- `Requirement: TOML Configuration Format`  
Status: PASS (startup validates TOML via `reload_config`, fails on invalid TOML in strict mode).
- `Requirement: Configuration File Resolution Order`  
Status: PASS (`crates/tze_hud_config/src/resolver.rs` + startup path handling in `app/tze_hud_app/src/main.rs`).
- `Requirement: Minimal Valid Configuration`  
Status: PASS (`[runtime]` + `[[tabs]]` enforced by startup validation and tested in `canonical_config_schema.rs` / startup unit tests).

### `runtime-kernel/spec.md` (epic-relevant subset)

- `Requirement: Window Modes`  
Status: PASS (fullscreen/overlay runtime controls and docs converge on CLI/env-driven mode selection; runtime implementation in `windowed.rs`, operator docs updated).

## Sibling Bead Reconciliation

| Child bead | Merged outcome | Reconciliation note |
|---|---|---|
| `hud-7yaf.1` | closed (`gh-pr:376`) | Canonical app config/schema alignment is present in current config + schema tests |
| `hud-7yaf.2` | closed (`gh-pr:379`) | Canonical-app CI boot gate exists and is fallback-sensitive |
| `hud-7yaf.3` | closed (`gh-pr:381`) | Strict fail-closed startup behavior is present and test-covered |
| `hud-7yaf.4` | closed (`gh-pr:380`) | README/operator docs converged to current schema/path |
| `hud-7yaf.5` | closed | Convergence report artifact exists under `docs/reconciliations/` |

## Gap Decision

Mandatory spec/epic gaps found: **none**.

- No missing child bead required to satisfy `hud-7yaf` acceptance criteria.
- No additional runtime/config/doc/CI divergence that blocks epic closure was identified in this pass.

## Non-Blocking Residual Risks

1. CI maintenance surface remains slightly elevated because both `production-boot-vertical-slice` and `canonical-app-production-boot` jobs are retained.
2. Debug-only insecure startup override visibility is strongest in runtime source/help text; operator docs mention canonical fail-closed path but do not yet emphasize this override in a dedicated warning block.

These are not blockers for `hud-7yaf` acceptance, but can be tracked as optional hardening follow-ups if desired.
