# Reconciliation (gen-1): Enforcement-Machinery Audit Remediation

**Epic**: hud-1aswu — "Audit 2026-06-12 remediation: enforcement machinery (security + CI trust)"
**Reconciliation bead**: hud-1aswu.6
**Date**: 2026-06-13
**Source findings**: `docs/audits/20260612_project_review.md` §5.7–5.10, §8 risks 1/2/5/9, §9 Q1/Q2/Q4/M2
**Method**: Re-read epic + all sibling bead descriptions/acceptance criteria, then audited the *actually delivered* state — auth code, GitHub branch protection (`gh api repos/Tzeusy/tze-hud/branches/main/protection`), `ci.yml` lanes, `deny.toml`/cargo-deny, and `engineering-bar.md` — in this worktree.

This bead **cannot mutate beads** (worker contract). It records the reconciliation as a durable artifact and reports every coverage gap as a structured follow-up for the coordinator to materialize. It does **not** close hud-1aswu.6.

---

## 1. Finding → Bead Coverage Matrix

| # | Source finding (audit anchor) | Implementing bead | Status | Coverage | Verified delivered state |
|---|---|---|---|---|---|
| F1 | LocalSocket auth bypass on `0.0.0.0` default binds — normative violation of `security.md:19` (§5.10, §8 risk 1, §9 Q1) | **hud-1aswu.1** (closed, PR #710) | closed | **FULL** | `auth.rs` `evaluate_auth_credential` now takes `peer_addr: Option<IpAddr>`; `Credential::LocalSocket` accepted **only** when `addr.is_loopback()`, else `AuthResult::Failed` (AUTH_FAILED); `None` peer rejected conservatively. `windowed.rs` default binds flipped to `127.0.0.1` with explicit `TZE_HUD_BIND_ALL_INTERFACES=1` opt-in for `0.0.0.0`; `select_grpc_bind_host`/`mcp_bind_host` honor the flag, with regression tests (`select_grpc_bind_host_default_is_loopback`, `..._all_interfaces_is_not_loopback`, `start_network_services_loopback_default_binds_successfully`). Both code+doc-comment legs of the audit remedy ("reject non-loopback **or** bind loopback") delivered. |
| F2 | `cargo test (integration)` not a required check + `strict:false` branch protection (§5.7 weaknesses 2/3, §8 risk 5, §9 Q2) | **hud-1aswu.2** | **deferred** (defer_until 2026-09-12) | **UNCOVERED (owner-deferred)** | Live branch protection confirms the finding is **unremediated**: `required_status_checks.strict = false`; the 10 required contexts do **not** include `cargo test (integration)`. The `test-integration` job *exists* in `ci.yml` (job name `cargo test (integration)`) and runs on every PR, but it is **not** a required merge gate. Owner decision 2026-06-13 deferred the branch-protection hardening ~3 months and detached this bead as a blocker of hud-1aswu.6 so the epic can reconcile now. **Per owner instruction, recorded as owner-deferred, NOT as an open gap.** |
| F3 | Flaky calibrated wall-clock perf test inside blocking unit gate — `test_texture_upload_p99_within_budget` broke main twice 2026-06-12; violates `validation.md` determinism doctrine (§5.7 weakness 1, §8 risk 5, §9 Q2) | **hud-1aswu.3** (closed, PR #711) | closed | **FULL** | `budget_assertions.rs` header documents the quarantine: all `assert_p99_under`/`assert_p99_calibrated` hard-failure asserts are now **gated behind `TZE_HUD_PERF_ASSERT=1`** (`perf_assertions_enabled()` reads that env var; defaults off). Structural/non-timing assertions stay unconditional. Reference-host/Windows lane sets the env var to enforce budgets. Removes calibrated wall-clock p99 asserts from the blocking shared-runner lane while keeping them runnable elsewhere — matches the audit remedy and the bead's acceptance criteria 1–3. |
| F4 | D18 real-decode Windows lane dead 10+ days, unmonitored; engineering bar still claims it gates media changes (§5.8, §8 risk 2, §9 M2) | **hud-1aswu.4** (closed, PR #746) | closed | **FULL (re-scoped by owner)** | Owner decision 2026-06-13: no self-hosted runner exists or is planned. Delivered: (a) `schedule:` + `pull_request[labeled]` triggers removed from `real-decode-windows.yml` (workflow_dispatch kept as activation skeleton); other two GPU workflows already dispatch-only stubs — daily 24h-queue-then-cancel stopped; (b) `engineering-bar.md` §4.7 (line 119) marks the real-decode requirement **SUSPENDED** with rationale + SSH-lane pointer; (c) new `.claude/skills/user-test/scripts/d18_validation.sh` performs the lane's substantive checks over SSH (gpu.lock respect, GStreamer SDK verify, decoder capability report), activation-gated on SDK install + hud-ora8.1 phase 1; (d) alerting deliverable mooted by schedule removal. Original "restore runner + alerting" intent superseded by the owner's no-runner decision. |
| F5 | No dependency CVE/license automation on a GPL-3.0 project with codec deps (§5.9, §8 risk 9, §9 Q4) | **hud-1aswu.5** (closed, PR merged) | closed | **FULL** | `deny.toml` (8.3 KB) committed at repo root: `[advisories] version=2`, two documented `ignore` waivers (RUSTSEC-2024-0436 `paste`, RUSTSEC-2026-0173 `proc-macro-error2` — both compile-time-only, with action notes); `[licenses]` section scoped to GPL-3.0-compatible licenses with rationale. CI `cargo-deny` job (`name: cargo deny (advisories + licenses)`, `EmbarkStudios/cargo-deny-action@v2`, `command: check advisories licenses bans sources`, `log-level: warn`) added to `ci.yml`. |

### Epic-level findings not owned by a single sibling

| Audit risk | Status in this epic | Note |
|---|---|---|
| §8 risk 1 (auth bypass) | covered by F1 | — |
| §8 risk 2 (dead D18 lane) | covered by F4 | re-scoped to owner no-runner decision |
| §8 risk 5 (flake + strict:false + require-integration) | **split**: flake covered by F3; strict + require-integration are F2 (deferred) | The single audit risk maps to two beads with different fates. |
| §8 risk 9 (no dep automation) | covered by F5 | — |

---

## 2. Coverage Summary

- **Findings fully covered (5):** F1 (auth, hud-1aswu.1), F3 (flake, hud-1aswu.3), F4 (D18, hud-1aswu.4), F5 (cargo-deny, hud-1aswu.5). F4 was re-scoped by an explicit owner decision but is delivered and closed.
- **Findings owner-deferred (1):** F2 (require `cargo test (integration)` + enable `strict`/merge queue, hud-1aswu.2). Live `gh api` confirms the repo state is unchanged: `strict:false`, integration not a required context. Per the bead's owner-decision note and this worker's instructions, this is recorded as **owner-deferred (tracked in hud-1aswu.2, defer_until 2026-09-12)**, not as an uncovered gap requiring a new child bead.
- **Uncovered gaps requiring NEW beads:** none from the five primary findings. Two *adjacent* observations surfaced during verification (see §3) and are reported as follow-ups for the coordinator's discretion — they are NOT in the epic's original scope.

Every source finding (§5.7–5.10, §8 risks 1/2/5/9, §9 Q1/Q2/Q4/M2) maps to an implementing bead. The epic's acceptance condition "every finding mapped to an implementing bead" is **met**.

---

## 3. Observations Surfaced During Verification (out of epic scope)

These are *not* gaps in the five primary findings; they are adjacent items noticed while auditing the delivered state. Reported as Discovered-Follow-Ups for the coordinator to triage — the coordinator decides whether any become child beads.

1. **cargo-deny is CI-present but not a required branch-protection context.** The `cargo deny (advisories + licenses)` job runs on PRs but is absent from the 10 required contexts in `main` protection — so a red cargo-deny does not block merge. hud-1aswu.5's acceptance criteria explicitly allowed "may start non-blocking if documented, then promoted to required," so this is conformant-as-shipped, not a defect. Promotion to a required context naturally rides with the F2/hud-1aswu.2 branch-protection work (also deferred). Flagging so the coordinator can decide whether to fold "promote cargo-deny to required" into the deferred hud-1aswu.2 scope.

2. **F2 deferral leaves a standing exposure window the epic report should name.** While hud-1aswu.2 is deferred to 2026-09-12, merge-skew breakage (the exact failure mode that broke main twice on 2026-06-12, runs 27395804782/27396029825) remains possible: `strict:false` still allows PRs green on a stale base to land, and the integration suite is still non-blocking. This is an accepted-risk owner decision, but the gen-1 reconciliation / epic report (hud-1aswu.7) should explicitly list it as residual risk with its expiry date so it is not silently forgotten when the defer window lapses.

---

## 4. Reconciliation Verdict

- All five enforcement-machinery findings are **mapped to implementing beads**; four are delivered+closed with verified-on-disk evidence, one (F2) is an explicit owner-deferral tracked in hud-1aswu.2.
- **No new gap beads are required from the primary findings.** The two §3 observations are advisory follow-ups, not coverage gaps.
- Because F2 is owner-deferred rather than uncovered, a **gen-2 reconciliation bead is NOT warranted** on coverage grounds. (Coordinator owns that call; this worker cannot create beads.)
- The epic (hud-1aswu) is **reconciled** to the limit a docs/audit worker can establish. Remaining lifecycle actions — closing hud-1aswu.6, deciding on hud-1aswu.7 (epic report), and any follow-up bead materialization — are the **coordinator's** responsibility.

---

## 5. Evidence Index (verified in this worktree, 2026-06-13)

- Auth: `crates/tze_hud_protocol/src/auth.rs` (`evaluate_auth_credential`, `authenticate_session_init` — `peer_addr` loopback gate); `crates/tze_hud_runtime/src/windowed.rs` (`select_grpc_bind_host`, MCP bind host, `TZE_HUD_BIND_ALL_INTERFACES` opt-in, bind regression tests ~lines 6852–6933).
- Branch protection: `gh api repos/Tzeusy/tze-hud/branches/main/protection` → `strict:false`; 10 required contexts; `cargo test (integration)` and `cargo deny (...)` absent from required set.
- Flake quarantine: `examples/vertical_slice/tests/budget_assertions.rs` header + `perf_assertions_enabled()` (`TZE_HUD_PERF_ASSERT` gate).
- cargo-deny: `deny.toml` (repo root); `.github/workflows/ci.yml` job `cargo-deny` (lines ~530–547).
- D18 suspension: `about/craft-and-care/engineering-bar.md` §4.7 line 119; `.github/workflows/real-decode-windows.yml`; `.claude/skills/user-test/scripts/d18_validation.sh`.
- Beads: hud-1aswu (epic), hud-1aswu.1 (PR #710), hud-1aswu.2 (deferred), hud-1aswu.3 (PR #711), hud-1aswu.4 (PR #746), hud-1aswu.5 (PR merged).
