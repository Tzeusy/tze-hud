# Project Review: tze_hud (mayor/rig)

**Date**: 2026-06-12
**Project type**: Native Rust systems runtime (agent-native presence engine) | **Maturity**: Active pre-v1-closeout | **Users**: Single owner + LLM agent fleet
**Method**: `/th-projects` project-review, full mode — shape scan + structural scan + 6 parallel investigation agents (A: mapping/baseline, B: code/architecture, C: reliability/testing, D: security/perf/data, E: docs/ops, F: gaps/scale). Every major claim labeled [Observed]/[Inferred]/[Unknown] in the underlying agent evidence.

---

## 1. Normative Baseline

- **Doctrine maturity**: mature (14 authored files in `about/heart-and-soul/`)
- **Design-contract maturity**: structured (14 RFCs + 15 review docs; scaffold remnants: RFC 0004:258 `<TBD>` field number, RFC 0014 §4.2 TBD never patched after 0018 resolved it)
- **Spec maturity**: structured (37 main spec families, 106/106 with scenarios; only 56/106 carry source references; 2 promoted specs with literal `Purpose: TBD` headers)
- **Topology maturity**: structured but **stale** (omits `tze_hud_projection` entirely; documents deferred v2 media subsystems as extant)
- **Engineering-standards maturity**: structured (`about/craft-and-care/engineering-bar.md`, 175 lines, specific and quantitative)
- **Repo assessment (scanner)**: SHAPED — full structure present, authored content incomplete in 2 pillars
- **Source-of-truth order used**: heart-and-soul + legends-and-lore → openspec → lay-and-land → README/docs/beads → code inference
- **Known contradictions before scoring**: RFC count disagrees across four authoritative places (README says 11, components.md says 13-but-lists-12, CLAUDE.md says 13, filesystem has 14); crate count (README 15, actual 16+app); v1.md ships-list predates the text-stream-portal pivot; deferral stamping inconsistent (openspec media specs bannered, RFCs 0014/0018 not); failure.md defers its E25 degradation ladder to "RFC 0014 (forthcoming)" but RFC 0014 is the media-plane wire protocol — a stale/colliding pointer.

### What this means for confidence
Shape artifacts are strong enough to act as the primary normative baseline for every domain — alignment judgments are high-confidence. The exception is *topology and v1 scope freshness*: lay-and-land and v1.md lag the two newest workstreams (portals, projection authority), so "is this in scope?" judgments required cross-checking beads and git history rather than trusting the pillar.

---

## 2. Executive Summary

tze_hud is a **healthy, unusually well-governed codebase whose safety nets lag its velocity**. The doctrine is not decorative: leases/TTL/revocation, the three-plane protocol split, local-first input ack, timing semantics, and hardware-calibrated perf budgets are all verifiably implemented where the doctrine says they must be, frequently with spec-section citations at the implementation site. Performance and wire-contract discipline (categories 11, 12) are exemplary — CI-enforced calibrated budgets and reserved-field protobuf hygiene that would be rare in a team ten times this size.

The fragility is concentrated in the gap between the project's own stated bar and its enforcement machinery: a LAN-reachable `LocalSocketCredential` auth bypass on default-`0.0.0.0` binds contradicts security.md's mandatory-auth doctrine (`auth.rs:91-95`); the nightly D18 real-decode lane has been dead for 10+ days with no alerting while the engineering bar still claims it gates media changes; a doctrine-forbidden flaky wall-clock test sits inside the blocking `test-unit` gate and broke main twice on 2026-06-12; there are zero release tags across 1,023 commits with no rollback story; and branch protection requires no reviews and is non-strict. Separately, a systemic delivery pattern — "landed-but-not-live" (merged components with zero production callers awaiting the in-process portal host) — is the top planning risk.

Biggest strengths: spec-to-code traceability culture, the five-pillar knowledge architecture with a 96%-closure beads backlog, and incident-driven CI hardening with embedded postmortems. Biggest risks: the auth bypass, the unmonitored dead perf lane, v1-closeout drift (windows-first change stalled a month while portal work surged), and god-file churn concentration (`session_server.rs` 16.9k lines, `renderer.rs` 16.2k, both top churn hotspots in a 1,235-branch fleet).

---

## 3. Scorecard

| # | Area | Score | Conf. |
|---|------|-------|-------|
| 1 | Goal alignment | 4 | H |
| 2 | Architecture | 4 | H |
| 3 | Code clarity | 4 | H |
| 4 | Correctness | 4 | M |
| 5 | Error handling | 4 | H |
| 6 | Observability | 4 | M |
| 7 | Testing | 4 | H |
| 8 | Tooling/hygiene | 4 | H |
| 9 | Dependencies | 3 | M |
| 10 | Security | 4 | H |
| 11 | Performance | 5 | H |
| 12 | Data/API design | 5 | H |
| 13 | Documentation/DX | 4 | H |
| 14 | Release/ops | 3 | H |
| 15 | Maintainability | 4 | M |

**Average**: 4.0/5

Notes on conservative merging: Category 9 was assessed at 3.5 and rounded down (no CVE/license automation blocks a 4). Category 15 carries a recorded caveat: a strict craft-and-care reading of the engineering-bar §4 "approval present" requirement vs `required_pull_request_reviews: null` would cap it at 2; scored 4 because the divergence is acknowledged in writing (AGENTS.md:310) and substantively mitigated by 10 required status checks — but the contradiction must be resolved in writing.

---

## 4. Shape and Goal Fulfillment

### Explicit v1 goals (v1.md ships-list)
- Zones/widgets LLM-first surface, hit_regions, leases/budgets/attention, design tokens, Windows click-through, property tests, headless CI + Windows perf gate: **Achieved** (verified at file level by Agents B/D/F)
- Token-driven zone rendering "replacing hardcoded per-content-type colors": **Partially achieved** — `Compositor::tile_background_color` still hardcodes per-content-type colors (`crates/tze_hud_compositor/src/renderer.rs:7370-7384`), a live normative violation of a ships-claim
- Touch input (v1.md:89): **Unmet** — zero touch event paths in runtime (doc comment only, `windowed.rs:318`)
- CLI/gRPC introspection surface (v1.md:161): **Partially achieved** — pieces via MCP/session server, no dedicated CLI
- macOS headless CI lane (v1.md:185 success criterion): **Unmet as written** — CI runs ubuntu+windows only; refocus implies the criterion should be amended, not built
- 3-agent 60-min soak + release tag + spec closeout (windows-first 5.1-5.4): **Open**; 0 git tags ever

### Implicit goals (from git/beads, not yet in doctrine)
Text-stream portal Phase-1 is the de-facto current product (~40 of last 50 commits, 25+ of 39 open beads) — legitimized by RFC 0013 and the phase-1 change but absent from v1.md's ships-list. Also implicit: live-on-real-Windows-rig evidence capture as a de-facto gate, and render-thread perf hardening (doctrine-consistent).

### Documentation gaps (shape findings)
1. `about/lay-and-land/components.md` omits `tze_hud_projection` (major active crate) and presents deferred v2 media subsystems as extant (components.md:7, :25-43)
2. Deferral stamping inconsistent: RFCs 0014/0018 read as active drafts; `tze_hud_media_apple`/`_android` (2,473 LOC) remain workspace members despite "deferred indefinitely"
3. Counts drift: RFC count wrong in 3 of 4 places; README "15 crates / ~100k LOC" vs 16 crates + app / ~285k LOC
4. 50/106 specs lack source references — sampling shows ~70% are *implemented-but-unlinked* (convention gap), ~20% correctly aspirational, ~10% realized as scripts/skills
5. `text-stream-portal-phase1/tasks.md` shows 2/53 checkboxes vs many landed PRs — tracking truth lives in beads; checkboxes are stale
6. AGENTS.md accreted (two contradictory tracker blocks, dead QUICKSTART link, ~100 flat notes); docs/ (129+ files) has no index separating normative from episodic
7. failure.md's E25 ladder pointer ("RFC 0014 forthcoming") collides with the existing media RFC 0014

---

## 5. Detailed Findings (condensed; full evidence in agent reports)

### 5.1 Goal alignment — 4/H
Doctrine→code traceability at unusual granularity (frame stages per RFC 0003 at `windowed.rs:1551-1641`; session lifecycle per RFC 0005 at `session_server.rs:8-23`; timing fields at `scene/src/types.rs:391-393`). All doctrine anti-patterns checked and cleared: no LLM in frame loop, no plane collapse, no JSON on hot paths (MCP crate explicitly cold, `tze_hud_mcp/src/lib.rs:6-7`), no remote-roundtrip touch ack, no forked APIs. Violations: hardcoded tile background colors (remedy: `color.tile.background.*` tokens, S); vestigial mobile crates (S); `tze_hud_a11y` has zero consumers (S — document or park).

### 5.2 Architecture — 4/H
Clean layered acyclic crate graph; workspace dependency discipline matches the bar exactly (GPU chain co-pinning documented in-manifest). Weaknesses: god files on the highest-churn paths (production line counts: session_server 7.4k, renderer 7.7k, graph 4.1k, windowed 4.9k — also the top churn hotspots; remedy: section-banner-aligned submodule extraction, L); sync-by-comment constants between runtime and the unwired 8.4k-LOC `tze_hud_policy` crate (`attention_budget/mod.rs:48-49`; remedy M).

### 5.3 Code clarity — 4/H
Best-in-class constant/comment discipline (token-override path + fallback + spec citation, `renderer.rs:114-127`; bead IDs in comments for archaeology). fmt/clippy `-D warnings` CI-enforced. Drift: unjustified `#[allow(clippy::too_many_arguments)]` (5 sites) against the bar's justification rule (S); magic colors in `tile_background_color` (S).

### 5.4 Correctness — 4/M
~4,897 test functions; property/fuzz suites exactly where the bar demands (scene graph, protocol boundary, batch atomicity, media ingress); near-zero production unwraps on hot paths (renderer 0, windowed 0); defensive protocol design (dedup TTL windows, heartbeat thresholds, reconnect grace). Confidence M because tests could not be executed in this environment (headless GPU deadlock constraint) — judged from CI evidence + reading. Concurrency hotspot: compositor-thread `try_lock` choreography has produced repeated bugs (#698 lineage); remedy: try_lock-miss telemetry counter (S), then evaluate double-buffered scene snapshot (M/L). 22 production unwraps vs the bar's "never unwrap in library code" — mechanical `expect("invariant: …")` chore wave (S/M).

### 5.5 Error handling — 4/H
15 thiserror enums with stable error codes + tests asserting code strings; failure.md doctrine implemented (1,033-line 6-level degradation state machine with hysteresis; orphan/grace lease lifecycle integration-tested with deterministic clocks; safe mode). Gaps: 5 bare unwraps in session_server (S); E25 10-step doctrine ladder vs implemented 6-level ladder — explicit deferral but pointed at a colliding RFC number (M: author the RFC or annotate failure.md).

### 5.6 Observability — 4/M
Real telemetry crate (3.6k lines: frame, budget-violation, degradation, soak/leak, media audit) wired into the pipeline; frame-loop-safe drop-on-full sender (`collector.rs:24`); JSON tracing per the bar. Gaps: projection authority **binary** has zero tracing — a diagnostic hole for an externally-facing daemon (S); no automated trend gate across CI runs despite validation.md demanding trend surfacing (M).

### 5.7 Testing — 4/H
Layered suites + 10 required branch-protection contexts (verified via GitHub API), trace-regression and v1-thesis capstone gates. Three confirmed problems: (1) `test_texture_upload_p99_within_budget` (`examples/vertical_slice/tests/budget_assertions.rs:946`) is a recurring flake **inside the blocking unit gate** — failed main twice on 2026-06-12 after a prior "fix" (PR #602), violating validation.md's determinism doctrine (S-M: relocate wall-clock budget asserts to the informational/Windows lane); (2) `cargo test (integration)` — the 14 doctrine-critical behavioral suites — is **not** a required check (S); (3) `strict: false` lets merge-skew breakage land, which is exactly how the two main-push failures happened (S: strict checks or merge queue).

### 5.8 Tooling/hygiene — 4/H
CI enforces the stated bar plus two unusually thoughtful custom gates (vocabulary lint, dev-mode release-leak guard); incident postmortems embedded in ci.yml; toolchain pinned everywhere. Gaps: **the nightly Windows D18 real-decode lane is dead** — 10+ consecutive scheduled runs queued 24h and cancelled since ≥2026-06-02 (self-hosted GPU runner offline), no alerting, while engineering-bar.md:119 claims it gates media changes (M: restore runner + lane-failure notification + annotate the bar); thin local dev harness — no rust-toolchain.toml/justfile/[workspace.lints] (S).

### 5.9 Dependencies — 3/M
Lean lock (509 packages) and doctrine-conformant workspace centralization, but **no audit automation whatsoever** (no cargo-audit/deny, no dependabot/renovate, no SBOM) on a GPL-3.0 project with codec deps (S: cargo-deny CI job). unsafe confined to FFI/platform; SAFETY-comment backfill needed in media crates (~10 sites, S).

### 5.10 Security — 4/H
security.md is substantially implemented: fail-closed PSK auth on both planes, constant-time compare, additive/revocable/audited capabilities, live mid-session revocation broadcast, lease TTL cascade, abuse controls (rate limiters, payload caps, single-use agent-bound resume tokens). No secrets in repo. **Normative violation**: `Credential::LocalSocket(_)` accepted unconditionally (`crates/tze_hud_protocol/src/auth.rs:91-95`) while both servers default-bind 0.0.0.0 (`windowed.rs:3753,4031`) — any LAN host can authenticate without the PSK, contradicting security.md:19. Remedy (S): reject LocalSocket unless peer is loopback, or default-bind 127.0.0.1. Plaintext transport acceptable for tailnet-local v1 but must precede any remote-agent work (M).

### 5.11 Performance — 5/H
The project's strongest domain. Quantitative per-stage budgets with hardware-normalization calibration, CI-enforced on every PR via a *tested* gate script; "closer to ceiling = regression, track trends" doctrine; living hot-path hygiene (parse-on-commit caches, Arc<str>, lock-free atomics in the last five commits); load harnesses present. Caveat: gates cover the Windows lane only; soak/idle budgets need manual reference-host runs.

### 5.12 Data/API design — 5/H
Real contract surface is 4 proto files / 2,522 lines (the scan's "48" was worktree noise). Exemplary discipline: versioned package, reserved fields+names with RFC citations, legacy quarantined in `events_legacy.proto`, in-band version negotiation, four message classes realized in wire semantics (dedup-by-batch_id, ack'd durable vs fire-and-forget ephemeral, coalesced state-streams). Flag: the contract's host module (session_server.rs, 16.9k lines) is the evolution hazard, not the contract.

### 5.13 Documentation/DX — 4/H
README is operational and CI-validated (config schema claims backed by a required test); 16/16 crates have architecture-grade rustdoc headers; curriculum/ is a deliberate onboarding ramp. Weaknesses: README count drift; AGENTS.md self-contradicts on sync mechanics and references a nonexistent QUICKSTART; docs/ mixes normative reference with episodic artifacts (25 bead-stamped reports, 30 one-shot reconciliations, 11 MB committed evidence) with no index (all S remedies).

### 5.14 Release/ops — 3/H
Strong CI gates and a documented end-to-end deploy story, but **no release story at all**: 0 tags / 1,023 commits, version frozen at 0.1.0, no CHANGELOG, no artifact provenance on the deployed Windows binary, rollback undefined — while the engineering bar's own perf-comparability rules presuppose reference tags. Single reference host (`tzehouse-windows`) is a SPOF with no remote wake path; host config drift hand-managed. Remedies: tags+changelog (S), embed git SHA in `--version` + record on deploy (M), declarative host config (M), second host or accepted-risk note (L).

### 5.15 Maintainability — 4/M (caveat recorded in §3)
Change safety is predominantly mechanical (types + 10 required checks + behavior gates unusual at this maturity); spec drift actively managed via reconciliation reports; failures become durable knowledge. Weaknesses: review approval is convention-only with two documented same-day leak incidents (AGENTS.md:315); 312 merged-but-undeleted branches; rustdoc bar unenforced (no `missing_docs` lint anywhere). Remedies: amend engineering-bar §4 to match achievable mechanics or add a second reviewing identity (S); automated merged-branch deletion (S); `#![warn(missing_docs)]` on protocol+scene first (M).

---

## 6. Feature Gap Analysis

### Blockers (for v1 closeout as written)
| Gap | Why it matters | Evidence | Effort |
|-----|----------------|----------|--------|
| In-process portal host chain not landed | 5+ merged components have zero production callers; promotion gate downstream | hud-2iup7 → hud-be6ee → hud-ttq97/endkj; `resident_grpc.rs:301`, `projection/src/lib.rs:2243` | M |
| windows-first 4.x perf + 5.1 soak + 5.2-5.4 closeout stalled since 05-11 | This *is* v1; v1.md names it "active source of truth" | `openspec/changes/windows-first-performant-runtime/tasks.md` 19/28 | L |
| validation-operations-standalone 0/13 since 04-25 | Carries the evidence machinery v1 success criteria require | change tasks.md | M |
| v1.md amendments (touch, macOS lane, introspection, portal scope) | Closeout report fails against doctrine as written | v1.md:89,161,185 | S |

### Enhancements
| Gap | Why it matters | Evidence | Effort |
|-----|----------------|----------|--------|
| Spec source-reference convention | 50/106 specs unreconcilable without archaeology | shape scan; Agent A sampling | S/M |
| CLI introspection surface | v1.md:161 promise; fleet debuggability | app/main.rs | M |
| Zero-production-caller lint/gate for spec-claimed APIs | Prevents recurrence of landed-but-not-live | Agent F pattern analysis | M |

---

## 7. Scale & Long-Horizon

**At 10x** (agents/zones/portals): first technical ceiling is the single `Arc<Mutex<SceneGraph>>` shared by gRPC, MCP, and the frame loop with silent try_lock-skip semantics (~15 sites in windowed.rs) — degradation is silent staleness, and this class already bit twice (commits 3cc8692c, b6fd414d). First organizational ceiling is god-file merge contention across the agent fleet. Budget caps themselves hold (240 tiles at 30 agents); serial resvg rasterization could pressure the frame budget.

**1-year risks**: doc-estate accretion outpacing curation (AGENTS.md, docs/) degrading agent instruction fidelity; unmonitored CVE exposure; dual ResourceBudget struct drift (`scene/src/types.rs:330` vs `scene/src/lease/mod.rs:118`); policy-crate constant drift.

**3-year risks**: session_server.rs/renderer.rs calcify if not split before phase-4 media-egress fields (already wire-reserved) land; GPU chain co-pin (wgpu 24/glyphon 0.8/Rust 1.88) becomes an upgrade cliff if not exercised periodically.

**100x / 5-year**: not meaningfully applicable to a local single-user runtime; deliberately not enterprise-framed.

---

## 8. Risk Register

| # | Risk | Sev. | Likelih. | Conf. | Fix | Effort |
|---|------|------|----------|-------|-----|--------|
| 1 | LocalSocket auth bypass on 0.0.0.0 default binds (normative violation of security.md) | H | M | H | Loopback peer check or 127.0.0.1 default bind | S |
| 2 | D18 real-decode lane dead 10+ days, unmonitored; media bar unenforceable | H | certain (ongoing) | H | Restore runner + lane-failure alerting + annotate bar | M |
| 3 | Landed-but-not-live integration debt (portal host chain) | H | H | H | Land in-process host; add production-caller gate to done-definition | M |
| 4 | v1 closeout drift: windows-first stalled, 0 tags, no rollback | H | M | H | Re-sequence after portal host; tags+changelog+provenance | L |
| 5 | Flaky perf test in blocking gate + strict:false branch protection | M-H | H | H | Relocate wall-clock asserts; strict checks/merge queue; require test-integration | S-M |
| 6 | Silent try_lock-skip on shared scene mutex under load | M | M | M | Miss-counter telemetry now; snapshot/double-buffer later | M |
| 7 | God-file churn concentration (fleet merge funnel) | M | M | H | Section-aligned submodule splits before phase-4 fields | L |
| 8 | Doc estate accretion: stale topology/v1.md, AGENTS.md self-contradiction, no docs index | M | H | H | Doc-refresh tranche (see §9 quick wins) | S-M |
| 9 | No dependency CVE/license automation (GPL-3.0 + codec deps) | M | M | M | cargo-deny CI job | S |
| 10 | Single reference host SPOF for all live evidence | M | M | H | Wake path/second host or accepted-risk note | S-M |
| 11 | Review approval convention-only; two documented leak incidents | M | M | H | Amend bar §4 or second reviewing identity; encode adversarial checklist in reviewer prompt | S |
| 12 | Dual ResourceBudget structs with conflicting defaults | L | M | H | Unify in scene crate | S |

---

## 9. Recommendations & Roadmap (advisory — `/project-direction` owns sequencing)

### Quick wins (S)
1. **Close the LocalSocket hole** — loopback verification or 127.0.0.1 default bind (`auth.rs:91-95`, `windowed.rs:3753,4031`)
2. **CI gate hardening trio** — require `cargo test (integration)`, enable strict checks/merge queue, quarantine the texture-upload flake out of the blocking lane
3. **Doc-refresh tranche** — fix README/components/CLAUDE.md counts; add `tze_hud_projection` to lay-and-land; deferral banners on RFCs 0014/0018 + mobile crates; fix two `Purpose: TBD` specs; fix failure.md's RFC-0014 pointer collision; amend v1.md (touch/macOS/introspection/portal scope)
4. **cargo-deny CI job** (advisories + GPL license compliance)
5. **Hygiene batch** — archive the completed external-agent-projection-authority change; reconcile portal tasks.md to landed PRs; unify ResourceBudget structs; tile-background tokens (closes the last v1 ships-claim violation); automated merged-branch deletion (312 outstanding); try_lock-miss telemetry counter; unwrap→expect + clippy-allow justification chore wave

### Medium (M)
1. **Land the in-process portal host chain** (hud-2iup7 → hud-be6ee → hud-ttq97/endkj) — the critical path; everything portal-promotion-related queues behind it
2. **Restore the self-hosted GPU runner** + scheduled-lane failure alerting
3. **Versioning**: first tags (retro-tag the perf-baseline commit), CHANGELOG, git-SHA in `--version`, deploy provenance record
4. **Execute validation-operations-standalone** (0/13) alongside windows-first 5.2
5. **AGENTS.md regeneration + docs/ index** with normative/episodic split
6. **Policy-crate decision**: wire it or extract shared constants and freeze it

### Strategic (L+)
1. **v1 closeout**: windows-first 4.1-4.6 perf deep-dives, 5.1 three-agent soak (now unblocked in principle — projection authority supplies the 3-resident capability), first release tag, spec-to-code closeout report
2. **God-file splits** (session_server, renderer first) — before the reserved phase-4 media-egress fields land
3. **Scene-lock evolution**: evaluate double-buffered scene snapshot for the commit path, guided by the new miss-counter data

**Dependencies**: security fix (Q1) before any remote-agent/cloud-relay work; runner restoration (M2) before any media-pipeline changes; portal host (M1) before portal evidence/promotion beads; v1.md amendments (Q3) before closeout (S1); versioning (M3) before more deploy automation; splits (S2) before phase-4 protocol work.

---

## 10. Planning Handoff for `/project-direction`

**Boundary**: this review audits and classifies; `/project-direction` decides sequencing, specs, and beads. No beads were created by this review.

### Required shape work before implementation planning
- Amend v1.md (touch posture, macOS lane criterion, introspection scope, portal Phase-1 acknowledgment)
- Refresh lay-and-land/components.md (add projection crate; re-label deferred media subsystems; fix RFC count)
- Stamp deferral banners on RFC 0014/0018 and mobile-crate roots; fix failure.md's E25→"RFC 0014" pointer
- Resolve engineering-bar §4 review-approval contradiction in writing

### Required spec work before implementation planning
- Fix `Purpose: TBD` in drag-to-reposition and element-identity-store specs
- Adopt a minimal `Implementation:` source-reference convention; backfill the 7 sampled implemented-but-unlinked families first
- Sync validation-operations-standalone into `openspec/specs/validation-framework` before windows-first task 5.2

### Candidate workstreams (with normative-violation vs health-risk classification)
1. **Security hardening** (normative violation) — LocalSocket/bind fix; later TLS before remote agents
2. **CI trust restoration** (normative violation: determinism doctrine + dead-lane vs bar) — gate trio + runner + alerting
3. **Portal host completion** (health/delivery risk) — the active critical path, beads already exist
4. **v1 closeout** (doctrine obligation) — windows-first 4.x/5.x + validation-ops + first tag
5. **Doc/shape refresh tranche** (shape gap) — items above, mostly S, parallelizable as a chore wave
6. **Structural splits** (health risk) — god files, sequenced before phase-4 protocol fields

### Sequencing constraints
- Portal promotion gate (7.x) strictly downstream of in-process host chain; do not schedule section 6/6b/7 evidence work ahead of it
- windows-media-ingress-exemplar stays hard-gated on windows-first release criteria (its own tasks.md header)
- Inherit blocked-cluster truth from beads, not tasks.md checkboxes
- Chore waves (unwrap→expect, clippy justifications, branch cleanup) are low-conflict fill-in work for the fleet

### Explicit deprioritizations
- mTLS/OAuth full hardening — premature until remote agents are scheduled (loopback fix suffices for v1)
- Mobile/media crates, RFCs 0014/0018 implementation, `_deferred/v2-embodied-media-presence` — parked by refocus
- macOS CI lane build-out — amend the doctrine criterion instead
- Multi-tenant/enterprise governance, 100x scaling — wrong frame for a local single-user runtime
- a11y wiring — keep parked; document status only

---

## 11. Strengths Worth Preserving

1. **Spec-citation-at-implementation-site culture** — RFC sections, spec line numbers, and bead IDs in code comments make the doctrine mechanically navigable for agents
2. **CI-enforced, hardware-calibrated perf budgets with a tested gate script** — and the "closer-to-ceiling is a regression" doctrine behind it
3. **Sovereignty machinery implemented clause-for-clause** — lease TTL state machine, live revocation broadcast, audit events; security.md is code, not aspiration
4. **Incident-to-knowledge pipeline** — postmortems embedded in ci.yml, dated audits/decisions/reconciliations, AGENTS.md countermeasure notes
5. **Backlog hygiene** — 1,745 beads at 96% closure, dependency-linked, root-caused to file:line; spec-gated sequencing already practiced (media exemplar correctly dormant)

---

## 12. Appendix

### A. Repository map (condensed)
16 crates + `app/tze_hud_app` (canonical binary) + 5 example packages + integration-test package. Core: scene (Layer 0, pure), compositor (wgpu/glyphon/resvg), runtime (8-stage pipeline hub), protocol (tonic gRPC + auth), input/telemetry/config/resource/widget/mcp/projection/validation. Peripheral/parked: policy (built, unwired), a11y (no consumers), media_apple/media_android (v2 relics). ~285k LOC Rust including tests. Specs: 37 families + 5 active changes. CI: 10 required contexts + Windows perf gate + 5 specialized workflows (D18 lane currently dead).

### B. Evidence index (primary citations)
Auth/security: `crates/tze_hud_protocol/src/auth.rs:25-114`, `token.rs:107-179`, `crates/tze_hud_runtime/src/windowed.rs:3753,4031`, `crates/tze_hud_mcp/src/server.rs:8-125`. Doctrine-in-code: `windowed.rs:1551-1641`, `scene/src/types.rs:391-393`, `scene/src/lease/mod.rs:90-233`, `runtime/src/degradation.rs`. Violations: `compositor/src/renderer.rs:7370-7384` (colors), `examples/vertical_slice/tests/budget_assertions.rs:946` (flake; CI runs 27395804782/27396029825/27340218711), dead lane runs 26842584949→27370762495. Shape drift: `about/lay-and-land/components.md:7,25-43`, `about/heart-and-soul/v1.md:89,161,185`, README.md:26, AGENTS.md:122,235,310,315, `openspec/specs/{drag-to-reposition,element-identity-store}/spec.md:4`. Scale: `windowed.rs:1362-3107` (try_lock sites), `scene/src/types.rs:330` vs `scene/src/lease/mod.rs:118`. Full per-domain evidence in the six agent reports (session transcript, 2026-06-12).

---

**Verdict**: **Healthy but fragile**

**Justification**: No category scores below 3 and the average is 4.0 with two exemplary domains (performance, wire contracts) — comfortably above the "Healthy" bar numerically. But the verdict steps down one level because the safety nets the project's own doctrine demands are partially absent or broken: one acknowledged-severity normative security violation (LAN-reachable auth bypass), a dead and unmonitored perf-enforcement lane the engineering bar still cites as a gate, a doctrine-forbidden flake inside the blocking CI gate that broke main twice on review day, zero release tags/rollback story across 1,023 commits, and convention-only review with documented leak incidents. Each fix is small (mostly S/M effort); the fragility is in enforcement machinery, not in the code's substance.
