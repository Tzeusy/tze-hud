# Epic Report: Enforcement-Machinery Remediation (hud-1aswu)

**Epic**: hud-1aswu — "Audit 2026-06-12 remediation: enforcement machinery (security + CI trust)"
**Report bead**: hud-1aswu.7
**Date**: 2026-06-13
**Source audit**: `docs/audits/20260612_project_review.md` §5.7–5.10, §8 risks 1/2/5/9, §9 Q1/Q2/Q4/M2
**Reconciliation**: `docs/audits/20260612_reconciliation_gen1.md` (hud-1aswu.6, merged 5b80e758)
**Status**: CLOSED — 4 of 5 findings FULL; 1 finding OWNER-DEFERRED with named residual-risk window

---

## 1. Executive Summary

The 2026-06-12 project review identified five enforcement-machinery gaps as the primary structural fragility
of tze_hud: a normative security violation in LocalSocket authentication, branch-protection weaknesses
allowing merge-skew regressions, a doctrine-violating flaky test in the blocking CI gate, an unmonitored
dead GPU validation lane, and absence of dependency CVE/license automation.

This epic addressed all five findings through five child beads. Four findings are fully remediated and
closed. One finding (F2 — branch-protection strict mode and required integration-suite context) was
explicitly deferred to 2026-09-12 by owner decision, with the existing branch-protection configuration
left unchanged (no regressions introduced). A gen-1 reconciliation bead (hud-1aswu.6) verified
delivered state against every finding and confirmed no coverage gaps remain.

**The enforcement-machinery epic is reconciled.** The single residual risk is an explicitly named,
time-bounded deferral tracked in hud-1aswu.2, due 2026-09-12.

---

## 2. Finding-by-Finding Outcomes

### F1 — LocalSocket auth bypass on `0.0.0.0` default binds

**Source**: §5.10, §8 risk 1, §9 Q1
**Implementing bead**: hud-1aswu.1 (closed)
**Outcome**: FULL
**Evidence**: PR #710, merged commit 42006006

**Finding**: `Credential::LocalSocket(_)` was accepted unconditionally in
`crates/tze_hud_protocol/src/auth.rs:91-95` while both servers defaulted to binding `0.0.0.0`
(MCP HTTP at `windowed.rs:4024`, gRPC at `windowed.rs:4302`). Any LAN host could therefore
authenticate without the PSK, contradicting `security.md:19` ("every agent connection must be
authenticated; no anonymous connections at any presence level").

**Remediation delivered**:
- `evaluate_auth_credential` now takes `peer_addr: Option<IpAddr>`; `Credential::LocalSocket`
  is accepted **only** when `addr.is_loopback()`, else returns `AuthResult::Failed (AUTH_FAILED)`;
  `None` peer is rejected conservatively.
- Default binds for both gRPC and MCP HTTP flipped to `127.0.0.1`. External binding is an
  explicit opt-in via `TZE_HUD_BIND_ALL_INTERFACES=1`, enforced in `select_grpc_bind_host` and
  `mcp_bind_host` with regression tests (`select_grpc_bind_host_default_is_loopback`,
  `..._all_interfaces_is_not_loopback`, `start_network_services_loopback_default_binds_successfully`).
- Both legs of the audit remedy ("reject non-loopback OR bind loopback") are delivered.

**Residual risk**: None. HARD CONSTRAINT satisfied — this fix was required to precede any
remote-agent, cloud-relay, or LAN-exposure work.

---

### F2 — `cargo test (integration)` not a required check; `strict:false` branch protection

**Source**: §5.7 weaknesses 2/3, §8 risk 5 (partial), §9 Q2
**Implementing bead**: hud-1aswu.2 (DEFERRED — defer_until 2026-09-12)
**Outcome**: OWNER-DEFERRED

**Finding**: The 14 headless behavioral integration suites (`tests/integration/`, `cargo test (integration)`
CI job) were not among the 10 required status checks on `main`, and branch protection had `strict:false`.
This allowed a merged PR green on a stale base to land while `main` was red — this exact failure mode
occurred twice on 2026-06-12 (CI runs 27395804782, 27396029825). Anchor: `about/craft-and-care/engineering-bar.md` §4 merge conditions.

**Owner decision (2026-06-13)**: Branch-protection hardening — requiring `cargo test (integration)` as a
required context and enabling `strict:true` (or a merge queue) — is not necessary for the next few months.
Deferred to 2026-09-12. No existing protection was removed; this bead only proposed adding required checks.

**Verified unremediated state** (confirmed by `gh api repos/Tzeusy/tze-hud/branches/main/protection`,
2026-06-13): `required_status_checks.strict = false`; the 10 required contexts do NOT include
`cargo test (integration)`. The `test-integration` job exists in `ci.yml` and runs on every PR
but is not a required merge gate.

**Advisory follow-up** (noted by hud-1aswu.5 gen-1 recon): The `cargo deny (advisories + licenses)` job
(delivered by F5/hud-1aswu.5) is also CI-present but absent from the 10 required contexts. Promoting
cargo-deny to a required check should be folded into hud-1aswu.2's scope at 2026-09-12.

---

#### RESIDUAL RISK WINDOW — F2 deferral (expires 2026-09-12)

**Risk**: While hud-1aswu.2 is deferred, the merge-skew failure mode that broke `main` twice on
2026-06-12 (CI runs 27395804782, 27396029825) **remains possible**. Specifically:

- `strict:false` still allows a PR green on a stale base to be merged.
- The integration suite (`cargo test (integration)`) is still non-blocking — a regression in the 14
  behavioral suites does not prevent merge.
- Cargo-deny (F5) is also non-blocking — a failing advisory or license check does not prevent merge.

This is an accepted-risk owner decision, not an overlooked gap. The risk is bounded by the deferral
expiry: **2026-09-12**.

**Action required at expiry**: Land hud-1aswu.2, which should include:
  1. Add `cargo test (integration)` as a required context on `main` branch protection.
  2. Enable `strict:true` (or configure a merge queue).
  3. Add `cargo deny (advisories + licenses)` as a required context (folded from the advisory
     surfaced during hud-1aswu.5 reconciliation).
  4. Verify via `gh api repos/Tzeusy/tze-hud/branches/main/protection` that all three are present.
  5. Amend AGENTS.md merge-recipe if mechanics change.

---

### F3 — Flaky calibrated wall-clock perf assertion in blocking CI gate

**Source**: §5.7 weakness 1, §8 risk 5 (partial), §9 Q2
**Implementing bead**: hud-1aswu.3 (closed)
**Outcome**: FULL
**Evidence**: PR #711, merged commit 000a95c2

**Finding**: `test_texture_upload_p99_within_budget` (`examples/vertical_slice/tests/budget_assertions.rs:946`)
was a recurring flake inside the blocking `test-unit` gate — it failed main on CI runs 27395804782 and
27396029825 (2026-06-12) and on PR run 27340218711 (2026-06-11), after a prior "fix" attempt (PR #602,
hud-srnr5). Calibrated wall-clock budget assertions on shared CI runners violate
`about/heart-and-soul/validation.md` determinism doctrine ("flaky tests poison the feedback loop";
"Determinism is not optional").

**Remediation delivered**:
- All `assert_p99_under`/`assert_p99_calibrated` hard-failure asserts in `budget_assertions.rs` are now
  gated behind `TZE_HUD_PERF_ASSERT=1` (via `perf_assertions_enabled()` which reads that env var;
  defaults off on shared runners).
- Structural, non-timing assertions remain unconditional.
- The reference-host/Windows lane sets `TZE_HUD_PERF_ASSERT=1` to continue enforcing budget assertions
  on the appropriate hardware.
- `budget_assertions.rs` header documents the quarantine rationale.

**Residual risk**: None. The blocking shared-runner gate no longer contains calibrated wall-clock p99
assertions. The assertions remain runnable on the reference host.

---

### F4 — D18 real-decode Windows lane dead 10+ days, unmonitored

**Source**: §5.8, §8 risk 2, §9 M2
**Implementing bead**: hud-1aswu.4 (closed)
**Outcome**: FULL (re-scoped by owner)
**Evidence**: PR #746, merged commit 4e2fe573

**Finding**: The nightly Windows D18 real-decode GPU lane had been dead for 10+ consecutive scheduled runs
(queuing 24h then cancelling since ≥2026-06-02) because the self-hosted GPU runner was offline, with no
alerting. `about/craft-and-care/engineering-bar.md:119` still claimed the lane gated media changes — an
unenforceable bar.

**Owner decision (2026-06-13)**: No self-hosted runner exists or is planned. `tzehouse-windows` is the
owner's personal machine, not CI infrastructure. The original "restore runner + alerting" intent was
superseded. Root finding: GStreamer MSVC SDK was never installed (`no C:\gstreamer`, no
`GSTREAMER_1_0_ROOT_MSVC_X86_64` machine env, no `gst-inspect` on PATH); the lane's decode step was
always a stub exiting 0 (tracked in hud-ora8.1 phase 1). The lane never produced a real validation.

**Remediation delivered (re-scoped)**:
- (a) `schedule:` and `pull_request[labeled]` triggers removed from `real-decode-windows.yml`
  (`workflow_dispatch` kept as an activation skeleton). The other two GPU workflows
  (`windowed-overlay-perf.yml`, `safari-simulcast-interop.yml`) were already dispatch-only stubs.
  Daily 24h-queue-then-cancel stopped.
- (b) `engineering-bar.md` §4.7 (line 119) marks the real-decode requirement **SUSPENDED** with
  rationale and an SSH-lane pointer.
- (c) `.claude/skills/user-test/scripts/d18_validation.sh` added: performs the lane's substantive
  checks over SSH (GPU lock respect, GStreamer SDK verification, decoder capability report),
  activation-gated on SDK install + hud-ora8.1 harness.
- (d) Alerting deliverable was mooted by schedule removal.

**Residual risk**: The real-decode validation capability does not exist until the GStreamer MSVC SDK is
installed on `tzehouse-windows` and hud-ora8.1 phase 1 harness is complete. Media changes currently
lack an automated hardware validation lane. This is an accepted owner decision documented in
`engineering-bar.md`. The `d18_validation.sh` script provides the activation path when prerequisites land.

---

### F5 — No dependency CVE/license automation on a GPL-3.0 project

**Source**: §5.9, §8 risk 9, §9 Q4
**Implementing bead**: hud-1aswu.5 (closed)
**Outcome**: FULL
**Evidence**: PR merged (hud-1aswu.5 close reason: "PR merged; ledger reconcile post hook-fix")

**Finding**: No cargo-audit, cargo-deny, dependabot, renovate, or SBOM automation existed on a GPL-3.0
project with media/codec dependencies (509 locked packages in Cargo.lock). This left CVE and license
violations undetected until manual review. Anchor: `about/craft-and-care/engineering-bar.md`
dependency-hygiene section (lines 132–138).

**Remediation delivered**:
- `deny.toml` (8.3 KB) committed at repo root: `[advisories] version=2`, with two documented `ignore`
  waivers (RUSTSEC-2024-0436 `paste`, RUSTSEC-2026-0173 `proc-macro-error2` — both compile-time-only,
  with action notes); `[licenses]` section scoped to GPL-3.0-compatible licenses with rationale.
- CI job `cargo deny (advisories + licenses)` added to `.github/workflows/ci.yml`
  (`EmbarkStudios/cargo-deny-action@v2`, `command: check advisories licenses bans sources`,
  `log-level: warn`).

**Residual risk**: As noted in F2's deferral section, cargo-deny is CI-present but not currently a
required branch-protection context. A red cargo-deny does not block merge in the F2 deferral window.
This limitation is bounded by the hud-1aswu.2 expiry at 2026-09-12.

---

## 3. Audit Risk Register — Epic Coverage Map

| Audit risk (§8) | Epic finding | Implementing bead | Status |
|---|---|---|---|
| Risk 1 — auth bypass | F1 | hud-1aswu.1 | FULL |
| Risk 2 — dead D18 lane | F4 | hud-1aswu.4 | FULL (owner-rescoped) |
| Risk 5 — flake + strict:false + require-integration | F3 + F2 | hud-1aswu.3 + hud-1aswu.2 | FULL (flake) + OWNER-DEFERRED (strict/integration) |
| Risk 9 — no dep automation | F5 | hud-1aswu.5 | FULL |

**Note on risk 5 split**: The single audit risk mapped to two distinct beads with different fates.
The flake (hud-1aswu.3) is fully remediated. The branch-protection hardening (hud-1aswu.2) is
explicitly owner-deferred to 2026-09-12.

---

## 4. Residual Risk Summary

| Item | Risk | Expiry / Gate |
|---|---|---|
| F2 deferral — merge-skew window | A PR green on stale base can still land; integration suite and cargo-deny are non-blocking merge checks; the exact failure mode from 2026-06-12 remains possible | **2026-09-12** (hud-1aswu.2) |
| F4 owner-rescope — no real-decode lane | Media changes lack automated hardware GPU validation; engineering-bar real-decode requirement marked SUSPENDED | Until GStreamer MSVC SDK installed + hud-ora8.1 phase 1 complete (activation prerequisites in d18_validation.sh) |
| F5 advisory — cargo-deny not required-check | cargo-deny job runs on PRs but does not block merge; a failing advisory/license check is visible but not enforced | **2026-09-12** (folds into hud-1aswu.2 scope) |

No residual risks from F1 (auth) or F3 (flake quarantine).

---

## 5. Owner Handoffs

| Item | Decision | Handoff bead | Due |
|---|---|---|---|
| Branch-protection hardening (strict + integration suite + cargo-deny as required check) | Deferred by owner 2026-06-13 | hud-1aswu.2 | 2026-09-12 |
| Real-decode Windows validation | No runner exists or is planned; lane rescoped to SSH + activation-gated script | hud-ora8.1 (phase 1) | Activation prerequisites: SDK install + harness |

---

## 6. Reconciliation Verdict

**Bead**: hud-1aswu.6 (closed 2026-06-13, merged 5b80e758)
**Verdict**: All five enforcement-machinery findings are mapped to implementing beads. Four are
delivered and closed with verified-on-disk evidence. One (F2) is an explicit owner-deferral tracked
in hud-1aswu.2 with a 2026-09-12 expiry. No coverage gaps; no gen-2 reconciliation bead required.

---

## 7. Evidence Index

| Item | Location |
|---|---|
| Auth fix (F1) | `crates/tze_hud_protocol/src/auth.rs` (`evaluate_auth_credential`, `authenticate_session_init`); `crates/tze_hud_runtime/src/windowed.rs` (`select_grpc_bind_host`, MCP bind host, `TZE_HUD_BIND_ALL_INTERFACES` opt-in, bind regression tests ~lines 6852–6933) |
| Auth fix PR | PR #710, commit 42006006 |
| Branch protection state (verified 2026-06-13) | `gh api repos/Tzeusy/tze-hud/branches/main/protection` → `strict:false`; 10 required contexts; `cargo test (integration)` and `cargo deny (...)` absent from required set |
| Flake quarantine (F3) | `examples/vertical_slice/tests/budget_assertions.rs` header + `perf_assertions_enabled()` (`TZE_HUD_PERF_ASSERT` gate) |
| Flake fix PR | PR #711, commit 000a95c2 |
| D18 suspension (F4) | `about/craft-and-care/engineering-bar.md` §4.7; `.github/workflows/real-decode-windows.yml`; `.claude/skills/user-test/scripts/d18_validation.sh` |
| D18 fix PR | PR #746, commit 4e2fe573 |
| cargo-deny (F5) | `deny.toml` (repo root); `.github/workflows/ci.yml` job `cargo-deny` |
| Branch protection deferral | hud-1aswu.2 (status: deferred, defer_until: 2026-09-12) |
| Gen-1 reconciliation | `docs/audits/20260612_reconciliation_gen1.md` (hud-1aswu.6, commit 5b80e758) |
| Source audit | `docs/audits/20260612_project_review.md` §5.7–5.10, §8 risks 1/2/5/9 |
| Merge-skew CI evidence | Runs 27395804782, 27396029825 (2026-06-12) — PR green on stale base, main red post-merge |
