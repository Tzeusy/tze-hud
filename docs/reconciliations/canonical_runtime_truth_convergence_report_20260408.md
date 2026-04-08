# Canonical Runtime Truth Convergence Report (2026-04-08)

Issue: `hud-7yaf.5`  
Epic: `hud-7yaf` (Converge canonical runtime truth and harden release startup)

## Scope

This report summarizes the convergence work already landed for:
- canonical app config/schema alignment
- CI enforcement of the canonical app config boot path
- fail-closed startup behavior in the canonical runtime app
- README/operator-doc convergence with current runtime truth

It also calls out residual risks and follow-up items.

## What Changed

### 1) Canonical app production-config CI gate was added

- Commit: `50ebb71` (`hud-7yaf.2`)
- Added CI job for canonical app config boot:
  - `.github/workflows/ci.yml` (`canonical-app-production-boot`)
- Added app-level production boot integration test:
  - `app/tze_hud_app/tests/production_boot.rs`
- Added test target wiring:
  - `app/tze_hud_app/Cargo.toml`

Behavioral effect:
- CI now boots runtime using `app/tze_hud_app/config/production.toml` and fails if startup silently degrades to default/headless behavior.

### 2) Canonical app startup was hardened to fail closed

- Commit: `47ac3e1` (`hud-7yaf.3`)
- Runtime/app startup guard updates in:
  - `app/tze_hud_app/src/main.rs`

Behavioral effect in strict startup mode:
- Startup exits non-zero if no readable config is found.
- Startup exits non-zero if PSK is still trivial default (`tze-hud-key`).
- Startup exits non-zero if loaded config fails validation.
- Dev insecure fallback is explicitly gated via `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1` and only honored for debug builds.

### 3) Runtime/operator docs were converged to current schema and path

- Commit: `f7af324` (`hud-7yaf.4`)
- Updated docs:
  - `README.md`
  - `about/lay-and-land/operations/DEPLOYMENT.md`
  - `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`
  - `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`

Behavioral effect:
- Public/operator docs now point to canonical runtime binary path and current `TzeHudConfig` schema (`[runtime]` + `[[tabs]]`) instead of stale legacy config shape.

## Enforced Contracts (Runtime + CI)

### Runtime-enforced contracts

Source: `app/tze_hud_app/src/main.rs`

- Canonical startup requires readable config in strict mode.
- Canonical startup rejects default PSK in strict mode.
- Canonical startup validates config and exits on validation failure.
- Dev insecure startup is explicit and constrained (debug + env override).

### CI-enforced contracts

Source: `.github/workflows/ci.yml`, `app/tze_hud_app/tests/production_boot.rs`

- Canonical app production config must boot under CI (`canonical-app-production-boot`).
- Boot gate asserts not only runtime construction but config-declared state:
  - widget instances exist
  - widget types exist
  - profile-driven zone policy override is present
- Dev-mode leakage guard remains active in CI (`dev-mode-guard`).

## Canonical Operator Path

Canonical operator path for deployment/automation:

1. Build canonical binary (`tze_hud`) from `tze_hud_app`.
2. Deploy canonical config from `app/tze_hud_app/config/production.toml` as `tze_hud.toml` beside the binary.
3. Launch canonical runtime with explicit non-default PSK and endpoint/window controls (CLI/env).
4. Validate MCP endpoint reachability before publish assertions.
5. Publish zone messages only after reachability gate passes.

Primary operator references:
- `README.md`
- `about/lay-and-land/operations/DEPLOYMENT.md`
- `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`
- `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`

## Fail-Closed Semantics (Current)

For canonical startup (`app/tze_hud_app/src/main.rs`):

- Missing/unreadable config in strict mode: hard fail.
- Invalid config in strict mode: hard fail.
- Trivial default PSK in strict mode: hard fail.
- Insecure fallback path exists only as explicit debug/development escape hatch:
  - `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1`
  - ignored in release builds.

## Residual Risks

1. Dual production-boot CI jobs increase maintenance surface.
- CI currently runs both `production-boot-vertical-slice` and `canonical-app-production-boot`.
- This is acceptable now, but can drift if one gate is updated and the other is not.

2. Operator docs describe canonical path but do not prominently explain the debug-only insecure override.
- `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1` is documented in runtime source/help text.
- It is not yet surfaced consistently across operator docs as a non-production escape hatch.

3. Existing unrelated unstable tests remain in repo baseline.
- Known pre-existing failures (tracked separately in notes) can still produce CI instability unrelated to this convergence set.

## Follow-Up Items

1. Decide whether to keep both production-boot jobs long-term or designate one as informational to reduce drift risk.
2. Add a short operator-doc note explicitly stating that insecure startup override is debug-only and non-canonical.
3. Keep reconciliation issue `hud-7yaf.6` focused on confirming no spec/runtime/doc drift remains after these merges.

## Changed/Referenced Paths

Runtime/app/config:
- `app/tze_hud_app/src/main.rs`
- `app/tze_hud_app/config/production.toml`
- `app/tze_hud_app/tests/production_boot.rs`
- `app/tze_hud_app/tests/canonical_config_schema.rs`

CI:
- `.github/workflows/ci.yml`

Docs:
- `README.md`
- `about/lay-and-land/operations/DEPLOYMENT.md`
- `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`
- `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`
