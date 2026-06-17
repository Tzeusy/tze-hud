# TZE HUD Project Review — Improvement Cycles

**Repository:** `Tzeusy/tze-hud`  
**Review date:** 2026-06-17  
**Suggested repo path:** `docs/audits/20260617_project_review_improvement_cycles.md`  
**Verdict:** **Healthy but fragile**  
**Confidence:** Medium-high for repository health; medium for runtime correctness because this review inspected repository evidence but did not clone/build/run the project.

---

## 1. Executive summary

`tze-hud` is a serious, unusually well-documented pre-release Rust system. It has clear doctrine, OpenSpec coverage, crate boundaries, CI gates, security posture, tests, deployment/runbook material, and a visible audit/remediation loop. This is substantially healthier than typical `0.1.0` systems software.

The project’s v1 shape is coherent: a Windows-first, local, high-performance HUD runtime for LLM-agent presence, with MCP/gRPC active and media/WebRTC deferred or default-off. The repo mostly backs that up through `about/`, `openspec/`, `Cargo.toml`, `tests/integration/`, `.github/workflows/`, and runtime/protocol/config crates.

The fragility is concentrated in a few areas: very large hot-path files, policy telemetry/arbitration seams, public host/operator-specific runbook details, pre-release release/rollback maturity, and suspended/stubbed media validation. The improvement strategy should be: close truth gaps first, reduce hotspots second, then harden release/ops.

---

## 2. Review baseline

This review uses the repository’s own source-of-truth order:

1. Project-shape artifacts: `about/heart-and-soul/`, `about/lay-and-land/`, `about/craft-and-care/`
2. OpenSpec artifacts: `openspec/`
3. README, docs, examples, package metadata, issues/comments
4. Implementation, tests, config, scripts, CI, infra, git history
5. Inference from code shape

Major claims below are labeled:

- `[Observed]` directly verified from repo artifacts.
- `[Inferred]` reasoned from repo evidence.
- `[Unknown]` not verified from available artifacts.

---

## 3. Normative project shape

[Observed] `about/heart-and-soul/vision.md` defines `tze_hud` as a local presence/runtime layer for LLM agents, not a generic dashboard, browser shell, notification center, remote desktop, or chatbot UI.

[Observed] `about/heart-and-soul/v1.md` narrows v1 to a Windows-first, local native Rust HUD runtime. Multi-device, mobile, glasses, embodied presence, and broader media directions are deferred/default-off.

[Observed] `README.md` says the active v1 transport surface is MCP + gRPC and that WebRTC/media is inactive/default-off for v1.

[Observed] `about/heart-and-soul/security.md` sets high security expectations: authenticated connections before capabilities, additive/revocable/auditable capabilities, agent isolation, resource governance, human override/safe mode, deterministic tests, and bounded channels.

[Observed] `about/craft-and-care/engineering-bar.md` requires strict engineering behavior: Rust 2024/MSRV 1.88, p99/frame-budget attention, CI gates, no casual `unsafe`/unchecked shortcuts, and adversarial review posture.

[Inferred] The correct grading calibration is **ambitious pre-release local systems software**, not production SaaS and not a toy prototype.

---

## 4. Scorecard

| Area | Score | Confidence | Summary | Key evidence |
|---|---:|---|---|---|
| Goal alignment / product coherence | 4 | High | Strong v1 thesis and Windows-first scope; product claims need sharper wording around policy/media maturity. | `about/heart-and-soul/vision.md`, `about/heart-and-soul/v1.md`, `README.md` |
| Architecture / modularity | 4 | Medium | Good crate topology and documented active/parked boundaries; still has large hub files. | `Cargo.toml`, `about/lay-and-land/components.md`, `crates/tze_hud_runtime/src/windowed.rs`, `crates/tze_hud_protocol/src/session_server.rs` |
| Code clarity / craftsmanship | 3 | Medium | Strict lints and extraction work are good; hot files are too large for safe review. | `Cargo.toml`, `crates/tze_hud_runtime/src/windowed.rs`, `crates/tze_hud_protocol/src/session_server.rs` |
| Correctness / reliability | 4 | Medium | Broad test surface and capstone v1 tests exist; pass status not locally verified. | `tests/integration/`, `.github/workflows/ci.yml` |
| Error handling / failure behavior | 4 | Medium | Fail-closed config, safe-mode doctrine, bounded channels, and auth rejection tests are strong. | `README.md`, `about/heart-and-soul/security.md`, `crates/tze_hud_protocol/src/auth.rs` |
| Observability / debuggability | 4 | Medium | Telemetry validation is strong; policy-specific telemetry remains an active seam. | `crates/tze_hud_telemetry/src/validation.rs`, `openspec/specs/policy-arbitration/spec.md`, `AGENTS.md` |
| Testing strategy / quality | 4 | High | Strong layered CI/test strategy; GPU/pixel and real-decode lanes remain partial/informational. | `.github/workflows/ci.yml`, `.github/workflows/real-decode-windows.yml`, `tests/integration/` |
| Tooling / engineering hygiene | 4 | High | Strong Rust/tooling posture with workspace lints, cargo-deny, justfile, and local CI mirrors. | `Cargo.toml`, `deny.toml`, `justfile`, `.github/workflows/ci.yml` |
| Dependency / ecosystem health | 4 | Medium | Dependencies are centralized and justified; GPU/text stack has an acknowledged upgrade cliff. | `Cargo.toml`, `deny.toml` |
| Security posture | 4 | Medium | Better than typical pre-release projects; LocalSocket auth was remediated; public ops details remain risk. | `crates/tze_hud_protocol/src/auth.rs`, `docs/audits/20260612_project_review.md`, `README.md`, `AGENTS.md` |
| Performance / scalability | 4 | Medium | Performance is treated as product behavior; claims depend on CI/reference lanes not locally run. | `about/craft-and-care/engineering-bar.md`, `.github/workflows/ci.yml` |
| Data model / API design | 4 | Medium | Protocol/lease/policy semantics are strong; some upload/policy seams remain unresolved. | `openspec/specs/`, `crates/tze_hud_protocol/src/`, `AGENTS.md` |
| Documentation / DX | 4 | High | Excellent depth and traceability; docs are sprawling and contain minor rot/public-runbook concerns. | `about/`, `openspec/`, `docs/`, `README.md`, `AGENTS.md` |
| Release / operations / production readiness | 3 | Medium | Good owner/operator runbooks; public release/package/rollback maturity is still pre-release. | `Cargo.toml`, `CHANGELOG.md`, `README.md` |
| Maintainability / change safety | 3 | Medium | Tests/specs help, but hot files, docs breadth, deferred material, and review enforcement gaps raise risk. | `crates/tze_hud_runtime/src/windowed.rs`, `crates/tze_hud_protocol/src/session_server.rs`, `about/craft-and-care/engineering-bar.md` |

---

## 5. Highest-value improvement cycles

### Cycle 0 — Truth, hygiene, and public-surface cleanup

**Goal:** Make repo claims match implementation truth and reduce unnecessary public exposure.

| Item | Priority | Evidence | Why it matters | Done when |
|---|---:|---|---|---|
| Fix README Windows command typo | P0 | `README.md` contains malformed `tze_hud.exe` command text | Small runnable-doc rot undermines otherwise strong DX | Command snippets are corrected and grep-checked |
| Add CI/gate status section | P1 | `.github/workflows/ci.yml`, `justfile` | Reviewers need fast visibility into required gates | README has badges or a short required-gates table |
| Scrub public host/user/operator details | P0 | `README.md`, `AGENTS.md` include TzeHouse/Tailscale/SSH/VNC/scheduled-task/PSK workflows | Public docs should not reveal live operational shape unless required | Public docs use placeholders; live runbooks move private/internal |
| Add implementation-status labels to specs/docs | P1 | Broad `openspec/` and `about/` surface includes active/deferred/parked material | Prevents doctrine/specs from outrunning implementation | Each v1-relevant doc section is labeled implemented/reserved/deferred/parked |
| Reconcile policy telemetry claim | P0 | `openspec/specs/policy-arbitration/spec.md`, `AGENTS.md` | Trust/privacy/security claims need observable proof | Minimal telemetry is implemented and tested, or v1 spec is downgraded honestly |

**Recommended validation:**

```bash
just ci
cargo test -p tze_hud_protocol
cargo test -p tze_hud_runtime
```

---

### Cycle 1 — Critical-path hotspot decomposition

**Goal:** Make runtime/protocol critical paths reviewable without changing behavior.

| Hotspot | Priority | Evidence | Risk | Decomposition target |
|---|---:|---|---|---|
| `crates/tze_hud_runtime/src/windowed.rs` | P0 | ~10k lines / ~421 KB | Runtime/window/input/render logic is too large for safe local reasoning | Split window lifecycle, overlay mode, input routing, GPU lock/presentation, widgets/local feedback, safe mode/freeze shell, MCP/gRPC startup |
| `crates/tze_hud_protocol/src/session_server.rs` | P0 | ~9.3k lines / ~390 KB | Session/security/mutation code is too large for safe review | Continue existing `session_server/` module split: auth/handshake, admission, mutations, uploads, widgets, zone publish, subscriptions, tests |

**Rules for this cycle:**

- Keep extraction PRs semantic-preserving.
- Do not mix extraction with feature work.
- Add or move tests with the code being extracted.
- Preserve public interfaces unless a spec-backed change requires otherwise.
- Add a hotspot ledger to `about/craft-and-care/engineering-bar.md`.

**Done when:**

- Each hotspot has a façade plus named modules with focused responsibilities.
- Reviewers can understand a behavior path without scanning a 9k–10k line file.
- CI and v1 thesis tests remain green.
- New module tests cover at least the extracted failure/admission paths.

---

### Cycle 2 — Policy, capability, and telemetry closure

**Goal:** Make security/privacy/attention claims executable and observable.

| Item | Priority | Evidence | Why it matters | Done when |
|---|---:|---|---|---|
| Minimal `PolicyTelemetry` event schema | P0 | `openspec/specs/policy-arbitration/spec.md` | Policy decisions must be auditable | Reject/allow/shed/redact decisions emit structured telemetry |
| Capability grant/revocation audit | P0 | `about/heart-and-soul/security.md`, policy spec | Authenticated connections and additive/revocable capabilities are core guarantees | Tests assert grant/reject/revoke events |
| Attention-budget arbitration test | P1 | vision/security docs and policy spec | Attention budgets are product-defining | Integration test proves a budgeted request is constrained |
| Privacy/redaction path test | P1 | README and policy docs claim viewer-aware privacy | Privacy must be demonstrable, not aspirational | A viewer/privacy scenario has runtime outcome + telemetry assertion |
| Resource shedding telemetry | P1 | Resource governance docs/specs | Overload behavior needs operator visibility | Saturation/shedding test emits expected counters/events |

**Suggested acceptance shape:**

- One mutation is capability-rejected.
- One mutation is attention-budgeted.
- One output is privacy-redacted or withheld.
- One resource path is shed under configured pressure.
- Each produces expected runtime behavior and telemetry.

---

### Cycle 3 — Release and operations hardening

**Goal:** Move from owner-machine runbook maturity toward repeatable product release maturity.

| Item | Priority | Evidence | Why it matters | Done when |
|---|---:|---|---|---|
| Release artifact manifest | P1 | `Cargo.toml` version `0.1.0`, `CHANGELOG.md` pre-release status | Operators need provenance and repeatability | Release notes define artifact names, target triples, config expectations |
| Checksums/signing policy | P1 | Release/package story not yet productized | Local runtimes should be tamper-evident | Each release artifact has checksum and optional signing/provenance note |
| Config schema/version policy | P1 | Fail-closed config behavior in `README.md` | Config migration failure can break startup | Config schema version and migration/compatibility policy are documented |
| Rollback procedure | P1 | README has deployment commands but not release rollback | Screen-owning runtime needs quick recovery | Rollback is documented and smoke-tested |
| Released-binary smoke test | P2 | CI validates workspace, but product release needs artifact validation | Build/test is not the same as release/install validation | Smoke test runs against release artifact and production config |

**Done when:**

- A new operator can install, verify, start, stop, and roll back a release without TzeHouse-specific knowledge.
- The canonical app is treated as a release artifact, not just a workspace binary.
- Production config boot remains fail-closed.

---

### Cycle 4 — Scope pruning and dependency runway

**Goal:** Prevent v1 from being dragged by deferred media/platform ambitions or dependency cliffs.

| Item | Priority | Evidence | Why it matters | Done when |
|---|---:|---|---|---|
| Decide media v1 status | P0 | `about/heart-and-soul/v1.md`, `.github/workflows/real-decode-windows.yml` | Stubbed validation must not look like product coverage | Media is formally parked, or narrow v1 exemplar has real acceptance tests |
| Mark suspended workflows clearly | P1 | `real-decode-windows.yml` placeholder exits 0 | Placeholder success can create false confidence | Workflow name/docs make suspended status impossible to miss |
| Dependency upgrade issue/ledger | P1 | `Cargo.toml`, `deny.toml` mention wgpu/glyphon/MSRV constraints | GPU ecosystem pinning will become expensive | Upgrade ledger tracks blockers, MSRV, expected perf/test impact |
| Park v2/mobile/glasses scope outside v1 docs | P2 | `openspec/changes/`, `about/heart-and-soul/v1.md` | Reduces review/onboarding burden | Deferred material is clearly separated from v1 acceptance |
| Review stale evidence artifacts | P2 | `test_results/`, `docs/evidence/`, AGENTS force-add guidance | Prevents accidental artifact sprawl | Artifact retention rules are documented and linted |

---

## 6. Blockers vs enhancements

### Blockers

| Gap | Impact | Evidence | Remedy | Effort |
|---|---|---|---|---|
| Policy telemetry/arbitration closeout | Security/privacy/attention claims cannot be fully proven | `openspec/specs/policy-arbitration/spec.md`, `AGENTS.md` | Implement minimal telemetry or downgrade spec/README claims | L |
| Hotspot decomposition | Critical runtime/protocol paths are review-hostile | `crates/tze_hud_runtime/src/windowed.rs`, `crates/tze_hud_protocol/src/session_server.rs` | Extract semantic-preserving modules with tests | L/XL |
| Public ops detail scrubbing | Public attack surface and social-engineering risk | `README.md`, `AGENTS.md` | Move live host/operator notes private; keep generic deployment docs public | S/M |
| Release/rollback productization | Broader use lacks repeatable install/provenance/recovery path | `Cargo.toml`, `CHANGELOG.md`, `README.md` | Add release manifest, checksums, rollback, artifact smoke tests | M |
| Media scope decision | Stubbed/suspended media validation can create false confidence | `.github/workflows/real-decode-windows.yml`, `about/heart-and-soul/v1.md` | Park media or add real acceptance tests for the narrow exemplar | M/L |

### Enhancements

| Gap | Impact | Evidence | Remedy | Effort |
|---|---|---|---|---|
| README command typo | Minor DX rot | `README.md` | Patch and grep-check commands | S |
| CI status discoverability | Reviewers cannot quickly see required gates | `.github/workflows/ci.yml`, `justfile` | Add badges/gate table | S |
| Evidence artifact policy clarity | Risk of accidental artifact sprawl | `test_results/`, `docs/evidence/`, `AGENTS.md` | Define retention/force-add rules and add lint | S/M |
| Dependency upgrade runway | Future MSRV/wgpu/glyphon migration cliff | `Cargo.toml`, `deny.toml` | Track upgrade criteria and blocked crates | M |
| Review exception logging | Admin bypasses can normalize | `AGENTS.md`, `about/craft-and-care/engineering-bar.md` | Require machine-readable exception records | S/M |

---

## 7. Risk register

| Risk | Severity | Likelihood | Confidence | Evidence | Why it matters | Fix |
|---|---|---:|---|---|---|---|
| Policy telemetry/arbitration seam remains unresolved | High | Medium | High | `openspec/specs/policy-arbitration/spec.md`, `AGENTS.md` | Trust claims need observable proof | Implement minimal telemetry or re-scope |
| Hot files block safe change | High | High | High | `windowed.rs`, `session_server.rs` | Large critical files hide bugs and slow review | Continue focused extraction |
| Public host/operator details leak operational shape | Medium | High | Medium | `README.md`, `AGENTS.md` | Gives attackers/social engineers unnecessary context | Scrub public docs |
| Release story remains pre-release/operator-local | Medium | Medium | Medium | `Cargo.toml`, `CHANGELOG.md`, `README.md` | Hard to support broader installs/rollbacks | Add release/rollback policy |
| Media lane is suspended/stubbed | Medium | Medium | High | `.github/workflows/real-decode-windows.yml` | Can create false confidence | Park or implement real validation |
| PR review enforcement gap | Medium | Medium | High | `about/craft-and-care/engineering-bar.md`, `AGENTS.md` | Quality relies on discipline instead of controls | Enforce review or log exceptions |
| Dependency/MSRV upgrade cliff | Medium | High | Medium | `Cargo.toml`, `deny.toml` | GPU/text dependencies will age quickly | Maintain upgrade runway |
| Documentation outpaces implementation truth | Medium | High | Medium | `about/`, `openspec/`, `docs/` | Strong docs become harmful when stale | Add status labels and periodic pruning |

---

## 8. Scale horizon

### 10x

[Inferred] At 10x current usage, the architecture should mostly hold. Bounded channels, telemetry counters, leases, capabilities, integration tests, and perf budgets are the right primitives. Main risk: change safety around hot files and policy seams.

### 100x

[Inferred] At 100x event/mutation volume, the project needs harder evidence for queue behavior, coalescing semantics, backpressure, policy latency, and rejection telemetry. Runtime-visible overload counters and replayable traces become more important than static benchmark confidence.

### 1 year

[Inferred] The biggest risk is scope entropy. V1 is Windows-first/local/no-default-media, but the repo carries deferred media/mobile/glasses/embodied traces. Without clearer status labels, docs/specs can outrun implementation.

### 3 years

[Inferred] The biggest risks are dependency migration and maintainer knowledge transfer. The Rust/wgpu/glyphon/MSRV constraints and host-specific runbook knowledge need durable, generic maintenance paths.

### 5 years

[Inferred] The biggest risk is calcification: doctrine/specs/workflows becoming expensive to change while runtime assumptions age. Every v1-mandatory claim should become executable evidence or be demoted.

---

## 9. Strengths to preserve

[Observed] Preserve the doctrine/spec/RFC/OpenSpec culture. It is the project’s biggest differentiator and gives reviewers a real intent hierarchy.

[Observed] Preserve fail-closed config and dev-mode isolation. This is the right posture for an agent-facing local runtime.

[Observed] Preserve bounded channel semantics and overflow accounting. This is a strong foundation for UI/runtime backpressure.

[Observed] Preserve v1 thesis and production boot gates. They connect product intent to executable validation better than ordinary unit-only testing.

[Observed] Preserve the audit/remediation loop. The LocalSocket remediation pattern is the model: concrete risk, code change, regression test, doc update.

---

## 10. Suggested next-review checklist

Use this checklist after each improvement cycle:

- [ ] README claims match implemented behavior.
- [ ] v1-mandatory spec claims have tests or telemetry.
- [ ] Deferred/parked features are labeled as such.
- [ ] No public docs expose live host/user/PSK/operator details.
- [ ] `windowed.rs` and `session_server.rs` are smaller or have clear extraction progress.
- [ ] Policy/capability failures produce observable telemetry.
- [ ] Release artifacts can be installed, verified, started, stopped, and rolled back.
- [ ] CI gates are visible from README and reproducible through `just`.
- [ ] Dependency waivers have ownerless-but-actionable upgrade criteria.
- [ ] Manual/admin bypasses are logged and post-merge CI is checked.

---

## 11. Repository evidence index

| Evidence area | Paths |
|---|---|
| Product doctrine | `about/heart-and-soul/vision.md`, `about/heart-and-soul/v1.md`, `about/heart-and-soul/security.md` |
| Architecture map | `about/lay-and-land/components.md`, `Cargo.toml`, `crates/` |
| Engineering bar | `about/craft-and-care/engineering-bar.md`, `rust-toolchain.toml`, `justfile` |
| Specs | `openspec/specs/`, especially `openspec/specs/policy-arbitration/spec.md` |
| Tests | `tests/integration/`, crate-level tests under `crates/*/src/` |
| CI | `.github/workflows/ci.yml`, `.github/workflows/performance-assert.yml`, `.github/workflows/real-decode-windows.yml` |
| Security/auth | `crates/tze_hud_protocol/src/auth.rs`, `docs/audits/20260612_project_review.md` |
| Hotspots | `crates/tze_hud_runtime/src/windowed.rs`, `crates/tze_hud_protocol/src/session_server.rs` |
| Dependencies | `Cargo.toml`, `Cargo.lock`, `deny.toml` |
| Operations | `README.md`, `AGENTS.md`, `docs/` |
| Release status | `Cargo.toml`, `CHANGELOG.md`, README deployment sections |

---

## 12. Final verdict

**Healthy but fragile**

The repository is healthy because its architecture, docs, specs, tests, CI gates, security posture, and self-audit culture are materially stronger than typical pre-release systems code. It is fragile because critical runtime/protocol behavior remains concentrated in very large files, policy telemetry is still a live seam, media validation is intentionally suspended/stubbed, and release/ops maturity is still owner-machine/pre-release shaped. The improvement path is clear: close claim/telemetry/doc gaps, decompose hotspots, then harden release and dependency operations.
