## 1. Contract Gate

- [x] 1.1 Obtain owner approval for the resident-cache profile schema (design option 2) and record exact full-display/headless aggregate and per-class values; approved 2026-07-17 with strict disjoint ceilings/no borrowing and separate durable-disk/logical-agent texture domains.
- [x] 1.2 Reconcile the existing headless `max_agent_update_hz` value across RFC 0006 §3.4, the canonical configuration spec, and `DisplayProfile::headless()` before admission wiring. Resolved at 60 Hz; exact boundary tests cover acceptance at 60 and rejection above 60.
- [x] 1.3 Re-run `openspec validate profile-runtime-budget-envelope --strict` after the headless authority correction.

## 2. Configuration Envelope

- [x] 2.1 Add the approved runtime-resident aggregate and class fields to raw/resolved display profiles with overflow-safe validation and built-in defaults.
- [x] 2.2 Retain registered-agent budget overrides in `ResolvedConfig` and derive immutable `OperationalRuntimeEnvelope` in `RuntimeContext`.
- [x] 2.3 Add configuration tests for freeze, precedence, sub-ceiling totals, custom-profile lowering, and restart-only semantics.

## 3. Admission and Lease Wiring

- [x] 3.1 Construct production session limits and per-agent effective `ResourceBudget` values from the operational envelope and retained overrides.
- [x] 3.2 Pass effective budgets into every production lease grant and register sessions with the mutation-intake budget enforcer.
- [x] 3.3 Enforce aggregate resident-session, leased-tile, and agent-leased texture ceilings atomically with behavior-executing multi-agent tests.

## 4. Resource and Cache Wiring

- [x] 4.1 Introduce a project-owned resident-allocation ledger with deterministic accounted-byte rules, allocation identities, disjoint class totals, and atomic reserve/release semantics; place the neutral contract below runtime in the dependency graph while runtime owns construction and policy.
- [x] 4.2 Construct scene resource stores and both gRPC/MCP widget-source stores from distinct resource-residency and widget-asset-residency envelope limits instead of independent defaults; converge MCP registration on the durable/runtime registration path or remove its payload-retaining duplicate.
- [ ] 4.3 Wire image/GPU resource residency, widget raster caches, and font residency to class/aggregate admission at their existing safe eviction boundaries, including no-cache fallbacks where the work is optional.
- [ ] 4.4 Add tests proving logical shared-resource double-charging, physical allocation single-charging, separate CPU/GPU copies, and current-frame eviction safety.

## 5. Observability and Verification

- [ ] 5.1 Emit a machine-readable startup/accounting snapshot from the enforcement objects and add exact production-consumer coverage tests, including the gRPC widget fallback store, MCP widget registry, and retained widget source copies.
- [ ] 5.2 Update configuration, runtime, resource, topology, and operator documentation with the approved ownership model.
- [ ] 5.3 Run focused crate tests, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and the relevant integration/headless gates.
