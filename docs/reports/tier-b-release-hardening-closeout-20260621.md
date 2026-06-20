# Tier B Release-Hardening + Ops-Scrub Closeout Report

**Date:** 2026-06-21
**Report bead:** `hud-6ot6a1`
**Epic:** `bEpic` (Tier B release-hardening + ops-scrub)
**Verdict:** All four blocking beads closed; two OpenSpec changes not yet archived (noted below); artifact signing deferred.

---

## 1. Executive Summary

The Tier B release-hardening + ops-scrub epic delivered four items drawn from the 2026-06-17 project review ([`docs/audits/20260617_project_review_improvement_cycles.md`](../audits/20260617_tze_hud_project_review_improvement_cycles.md), Cycle 0 and the release/ops scorecard). Each item below is verified against actual repo state; no completion is claimed without a traced commit, PR, or file.

| Deliverable | Bead | PR / Commit | Status |
|---|---|---|---|
| Config `schema_version` gate | `hud-9tyx7g` | PR #906 (`aa0eddbd`) | **Shipped** |
| Release artifact provenance + CI smoke test | `hud-sgurmi` | PR #910 (`908ef81c`) | **Shipped** |
| Rollback runbook + CI rollback smoke test | `hud-hbxseh` | Direct merge (`36e248ef`) | **Shipped** |
| PSK investigation + ops-scrub verdict | `hud-39164f` | Direct merge (`701429c4`) | **Shipped — placeholder/never-real** |

OpenSpec changes `config-schema-version` and `release-provenance` are present in `openspec/changes/` but **have not been archived** (see §5).

---

## 2. Deliverable Evidence

### 2.1 Config `schema_version` Field and Fail-Closed Compatibility Gate

**Bead:** `hud-9tyx7g`
**PR:** [#906](https://github.com/Tzeusy/tze-hud/pull/906) — merged to main, commit `aa0eddbd`
**Bead close reason (verbatim):** "Merged PR #906 — config schema_version field + fail-closed gate; 357 tests pass, CI green."

**Evidence:**

- `crates/tze_hud_config/src/raw.rs:406` — `pub schema_version: Option<u32>` with `JsonSchema` derive; field appears in `--print-schema` output.
- `crates/tze_hud_config/src/loader.rs:55` — `CURRENT_CONFIG_SCHEMA_VERSION: u32 = 1`
- `crates/tze_hud_config/src/loader.rs:61` — `MAX_SUPPORTED_CONFIG_SCHEMA_VERSION: u32 = 1`
- `crates/tze_hud_config/src/loader.rs:138–157` — schema_version gate at step (0) of `validate`, before all field-level checks. Absent → treated as current. In-range → proceed. Newer → fail closed with `CONFIG_SCHEMA_VERSION_UNSUPPORTED`, no port bound.
- `crates/tze_hud_config/src/tests.rs` — three scenario tests: absent (loads as current), in-range, newer-than-supported.

**OpenSpec change:** `openspec/changes/config-schema-version/` — active, not archived (see §5).

Note: `openspec/changes/config-schema-version/tasks.md` shows all checkboxes unchecked. This is tasks.md checkbox drift — the implementation shipped in PR #906 before the tasks.md was updated. The bead close reason and the merged code are the canonical closure signals.

---

### 2.2 Release Artifact Provenance: Pipeline Checksum + CI Smoke Test

**Bead:** `hud-sgurmi`
**PR:** [#910](https://github.com/Tzeusy/tze-hud/pull/910) — merged to main, squash commit `908ef81c`
**Bead close reason (verbatim):** "PR #910 merged to main (squash merge 908ef81c) after dedicated review bead hud-anzaq. Release artifact provenance build/smoke gate, workflow reuse, verifier tests, and OpenSpec task updates landed."

**Evidence:**

- `scripts/ci/release_artifact_provenance_smoke.sh` — CI gate that builds the release `.exe` cross-compiled for `x86_64-pc-windows-gnu`, computes its SHA-256, and asserts it matches the published checksum. Gate fails on mismatch.
- `scripts/ci/test_release_artifact_provenance_smoke.py` — verifier unit tests for the smoke gate logic.
- `openspec/changes/release-provenance/tasks.md` — items 1.1–1.3, 2.1–2.3, 3.1–3.2 checked. Item 3.3 (optional headless smoke-boot of packaged exe) is unchecked and explicitly optional.

**OpenSpec change:** `openspec/changes/release-provenance/` — active, not archived (see §5).

---

### 2.3 Windows Rollback Runbook + CI Rollback Smoke Test

**Bead:** `hud-hbxseh`
**Merge:** Direct fast-forward to main, commit `36e248ef`
**Bead close reason (verbatim):** "Direct fast-forward merge to main at 36e248ef and pushed. Added rollback runbook docs/operations/tzehouse-windows-rollback.md plus scripts/ci/rollback_smoke_test.sh; smoke test passed locally."

**Evidence:**

- `docs/operations/tzehouse-windows-rollback.md` — full runbook: preflight checks, known-good artifact verification (SHA-256 against pipeline checksum), `TzeHudOverlay` stop/replace/restart sequence, post-rollback verification, failure escalation path. Cross-references `docs/operations/tzehouse-windows-recovery.md` for host-offline scenarios.
- `scripts/ci/rollback_smoke_test.sh` — scripted smoke test exercising the documented rollback steps against a fixture. Note: the runbook explicitly cross-references the pipeline-generated `tze_hud.exe.sha256` as the provenance gate, consistent with §2.2 above.

---

### 2.4 PSK Investigation and Ops-Scrub Verdict

**Bead:** `hud-39164f`
**Merge:** Direct merge to `origin/main`, commit `701429c4` (via branch `agent/hud-ev2lr`)
**Bead close reason (verbatim):** "PSK investigation findings doc merged to main (701429c4). Verdict: PLACEHOLDER/NEVER-REAL — no real production PSK ever committed (current or history); DEFAULT_PSK 'tze-hud-key' is a startup-rejected placeholder, all other values are test fixtures, AGENTS.md uses <psk> placeholder, evidence files redacted. Independently corroborates owner-accepted finding from hud-yotlg3. No rotation/history-rewrite required."

**Evidence:**

- `docs/audits/hud-39164f-psk-investigation-20260621.md` (commit `701429c4`) — full investigation report. Surfaces searched: working tree, full git history of AGENTS.md, scripts (`*.ps1`, `*.sh`, `*.bat`), docs/evidence, configs. Key findings:
  - `app/tze_hud_app/src/main.rs:83` — `DEFAULT_PSK: &str = "tze-hud-key"` is a trivial placeholder explicitly rejected by the runtime at startup (`psk_is_trivial_default`, line 634).
  - All test-scoped PSK values (e.g. `"v1-thesis-proof-key"`, `"subtitle-streaming-test-key"`, `"test-psk"`) are obviously test fixtures, not production secrets.
  - AGENTS.md has only used `--psk <psk>` (literal angle-bracket placeholder) or shell variables (`$psk`, `$Psk`) across its full git history — no real value ever appeared.
  - Evidence documents in `docs/evidence/` that captured Windows process listings had any `--psk` arguments redacted.

**Decision outcome:** No PSK rotation, no Bitwarden BWS entry update, and no git history rewrite required. The embedded value was never real. This verdict independently corroborates the finding from bead `hud-yotlg3`.

---

## 3. OpenSpec Change Status

| Change | Location | Archival status | Tasks.md |
|---|---|---|---|
| `config-schema-version` | `openspec/changes/config-schema-version/` | **Not archived** | All items unchecked (tasks.md checkbox drift; see §2.1) |
| `release-provenance` | `openspec/changes/release-provenance/` | **Not archived** | Items 1.1–3.2 checked; 3.3 optional/unchecked |

Neither change has been moved to `openspec/changes/archive/`. The `openspec/changes/archive/` directory contains changes from April 2026 and earlier; these two June 2026 changes are the newest and remain in the active `openspec/changes/` directory.

Both changes were reviewed and accepted prior to implementation (per their `proposal.md` and `tasks.md` review sections), and the implementation has shipped (§2.1 and §2.2 above). Archiving is a follow-up administrative step — see §5.

---

## 4. Blocking Bead Status Summary

All four beads that blocked `hud-6ot6a1` are closed:

| Bead | Title | Close date | Close signal |
|---|---|---|---|
| `hud-9tyx7g` | Config schema_version field + fail-closed gate | 2026-06-17 | PR #906 merged |
| `hud-hbxseh` | Rollback runbook + rollback smoke test | 2026-06-18 | Direct merge `36e248ef` |
| `hud-sgurmi` | CI release-artifact smoke test (provenance round-trip) | 2026-06-18 | PR #910 merged |
| `hud-39164f` | SECURITY: rotate the Windows HUD PSK (if real) | 2026-06-20 | Investigation `701429c4`; verdict: no rotation needed |

---

## 5. Deferred Follow-Ups

| Item | Priority | Notes |
|---|---|---|
| Archive `openspec/changes/config-schema-version/` | Low | Use `openspec archive config-schema-version`. Implementation shipped in PR #906; archiving is bookkeeping. |
| Archive `openspec/changes/release-provenance/` | Low | Use `openspec archive release-provenance`. Implementation shipped in PR #910; archiving is bookkeeping. |
| Artifact signing (cosign/gpg) | Low — explicitly deferred | Named in `openspec/changes/release-provenance/proposal.md` as optional/deferred for v1. No bead created; track when key infrastructure is available. |
| Headless smoke-boot of packaged exe (release-provenance 3.3) | Low — optional | Marked optional in tasks.md. Depends on feasibility of headless Windows exe execution in CI. |
| Update `openspec/changes/config-schema-version/tasks.md` | Administrative | Tasks.md checkbox drift: all items unchecked despite PR #906 being merged. Can be updated or left as-is before archival. |

---

## 6. What Was Not In Scope

- The broader ops-scrub items from the 2026-06-17 project review (e.g. scrubbing host/user details from public docs) were not tracked as Tier B implementation beads and are not covered by this epic. They remain as open backlog items.
- Live Windows HUD end-to-end validation of the rollback runbook (physical rollback on the production host) was not part of the acceptance criteria for `hud-hbxseh`; the runbook and CI smoke test cover the scripted path.
- PSK rotation was conditionally scoped: the condition (real PSK ever committed) evaluated to false, so no rotation work exists or was deferred.
