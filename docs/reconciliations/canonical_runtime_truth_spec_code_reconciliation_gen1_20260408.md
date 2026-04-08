# Canonical Runtime Truth Spec-to-Code Reconciliation (gen-1, 2026-04-08)

Issue: `hud-7yaf.6`  
Epic: `hud-7yaf`  
Scope: Reconcile runtime/config/docs/CI convergence against epic acceptance criteria and cited specs.

## Inputs Reviewed

- Epic and child issue metadata from `bd show hud-7yaf --json`
- Prior convergence artifact:
  - `docs/reconciliations/canonical_runtime_truth_convergence_report_20260408.md`
- Runtime/config/CI/docs implementation paths:
  - `app/tze_hud_app/src/main.rs`
  - `app/tze_hud_app/config/production.toml`
  - `app/tze_hud_app/tests/canonical_config_schema.rs`
  - `app/tze_hud_app/tests/production_boot.rs`
  - `.github/workflows/ci.yml`
  - `README.md`
  - `about/lay-and-land/operations/DEPLOYMENT.md`
  - `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`
  - `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`

## Epic Acceptance Criteria Reconciliation

### 1) Canonical app production config conforms and boots as blessed operator path

Status: satisfied.

Evidence:
- Canonical config exists and is schema-current:
  - `app/tze_hud_app/config/production.toml` uses `[runtime]` + `[[tabs]]`.
- Loader/schema validation gate exists:
  - `app/tze_hud_app/tests/canonical_config_schema.rs`.
- Runtime boot gate against canonical app config exists:
  - `app/tze_hud_app/tests/production_boot.rs`.

### 2) CI fails if canonical app config silently falls back

Status: satisfied.

Evidence:
- Dedicated CI job exists and runs:
  - `.github/workflows/ci.yml` job `canonical-app-production-boot`.
- The test asserts config-declared widget instances/types and profile-derived
  zone policy values (not just runtime construction):
  - `app/tze_hud_app/tests/production_boot.rs`.

### 3) Windowed release startup fails closed on missing/invalid config and trivial default PSK unless explicit dev override

Status: satisfied.

Evidence:
- Strict startup mode in canonical path:
  - requires readable config and exits non-zero when missing/unreadable.
  - validates config TOML and exits non-zero on schema/validation errors.
  - rejects default PSK `tze-hud-key`.
  - source: `app/tze_hud_app/src/main.rs`.
- Dev insecure override is explicit and isolated:
  - `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1` only enables fallback for debug builds.
  - release mode ignores the override.
  - source and unit tests: `app/tze_hud_app/src/main.rs`.

### 4) README/operator docs converge to current schema and annotate unstable lanes

Status: satisfied (including final residual gap closure in this change).

Evidence:
- Canonical docs already converged to current loader schema and operator path.
- Residual gap from prior report (explicit dev-insecure override guidance in
  operator docs) is now closed in:
  - `README.md`
  - `about/lay-and-land/operations/DEPLOYMENT.md`
  - `about/lay-and-land/operations/OPERATOR_CHECKLIST.md`
  - `about/lay-and-land/operations/RUNTIME_APP_BINARY.md`
- New guidance explicitly states fail-closed startup behavior and that
  `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1` is debug-only and non-canonical.

### 5) Final reconciliation confirms docs/config/runtime/CI convergence

Status: satisfied.

Evidence:
- Requirement-to-implementation mapping captured in this report.
- Targeted verification commands run and passing (see section below).
- No additional implementation gap requiring a mandatory follow-up bead was
  found in runtime/config/CI/docs coverage for this epic.

## Spec Mapping (Cited in Epic)

### `openspec/changes/ship-runtime-app-binary/specs/runtime-app-binary/spec.md`

- Requirement: Canonical Runtime Application Executable
  - Covered by canonical `tze_hud_app` binary/operator docs and CI references.
- Requirement: Configuration-Driven Runtime Startup
  - Covered by strict config resolution/validation in `app/tze_hud_app/src/main.rs`
    and canonical boot test gate in `app/tze_hud_app/tests/production_boot.rs`.
- Requirement: Windows Artifact Identity for Automation
  - Covered in operator docs pathing and deployment references.

### `openspec/changes/v1-mvp-standards/specs/configuration/spec.md`

- Requirement: Configuration File Resolution Order
  - Implemented in canonical runtime startup config resolution path.
- Requirement: TOML Configuration Format
  - Enforced by runtime config parse/validate behavior.
- Requirement: Minimal Valid Configuration
  - Enforced and validated via canonical config schema/boot tests.

### `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`

- Requirement: Window Modes
  - Canonical runtime startup preserves explicit window mode controls (`fullscreen`/`overlay`).
- Requirement: Headless Mode
  - CI/test boot behavior is intentionally explicit and not treated as silent
    production fallback in canonical operator path.

## Verification Executed

Command:

```bash
cargo test -p tze_hud_app --all-targets
```

Result: pass.

- Unit tests: 30 passed, 0 failed
- `canonical_config_schema`: 1 passed, 0 failed
- `production_boot`: 2 passed, 0 failed

## Conclusion

`hud-7yaf` acceptance criteria are now fully reconciled against current
implementation and docs. This epic is closeable from a spec-to-code perspective,
with no additional mandatory child issue identified by this reconciliation pass.
