## 1. Contract Gate

- [ ] 1.1 Obtain owner approval for the resident-cache profile schema (design option 2) and record exact full-display/headless aggregate and per-class values; status quo is the default if unanswered.
- [ ] 1.2 Re-run `openspec validate profile-runtime-budget-envelope --strict` after any approved contract edits.

## 2. Configuration Envelope

- [ ] 2.1 Add the approved runtime-resident aggregate and class fields to raw/resolved display profiles with overflow-safe validation and built-in defaults.
- [ ] 2.2 Retain registered-agent budget overrides in `ResolvedConfig` and derive immutable `OperationalRuntimeEnvelope` in `RuntimeContext`.
- [ ] 2.3 Add configuration tests for freeze, precedence, sub-ceiling totals, custom-profile lowering, and restart-only semantics.

## 3. Admission and Lease Wiring

- [ ] 3.1 Construct production session limits and per-agent effective `ResourceBudget` values from the operational envelope and retained overrides.
- [ ] 3.2 Pass effective budgets into every production lease grant and register sessions with the mutation-intake budget enforcer.
- [ ] 3.3 Enforce aggregate resident-session, leased-tile, and agent-leased texture ceilings atomically with behavior-executing multi-agent tests.

## 4. Resource and Cache Wiring

- [ ] 4.1 Introduce a project-owned physical resident-memory ledger with allocation identities, class totals, and atomic reserve/release semantics.
- [ ] 4.2 Construct scene resource stores and protocol widget stores from class-scoped envelope limits instead of independent defaults.
- [ ] 4.3 Wire widget raster and font caches to class/aggregate admission at their existing safe eviction boundaries, including no-cache fallbacks.
- [ ] 4.4 Add tests proving logical shared-resource double-charging, physical allocation single-charging, separate CPU/GPU copies, and current-frame eviction safety.

## 5. Observability and Verification

- [ ] 5.1 Emit a machine-readable startup/accounting snapshot from the enforcement objects and add exact production-consumer coverage tests.
- [ ] 5.2 Update configuration, runtime, resource, topology, and operator documentation with the approved ownership model.
- [ ] 5.3 Run focused crate tests, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and the relevant integration/headless gates.
