# Epic Report: Release/Ops Hardening + Public Ops-Detail Scrub (hud-e3bmtp)

**Epic**: hud-e3bmtp — "Release/ops hardening + public ops-detail scrub"
**Report bead**: hud-e3bmtp (this closeout, filed as the epic's final deliverable)
**Date**: 2026-07-05
**Source**: `docs/20260617_tze_hud_project_review_improvement_cycles.md` — Cycle 0 (scrub) + Cycle 3 (release/ops)
**OpenSpec changes**: `config-schema-version`, `release-provenance` (both archived — see §3)
**Related prior report**: `docs/reports/tier-b-release-hardening-closeout-20260621.md` (hud-6ot6a1) — covers the four Tier B implementation beads; this report supersedes it by adding the P0 scrub deliverable and confirming the OpenSpec archival that the prior report listed as an open follow-up.
**Status**: READY TO CLOSE — all six deliverables landed on main; both OpenSpec changes archived; no owner-deferred sub-item blocks acceptance.

---

## 1. Executive Summary

The 2026-06-17 project review flagged two hardening cycles: Cycle 0 (scrub public ops
detail — hostnames, users, keys, PSK) and Cycle 3 (release/ops integrity — config
version contract, artifact provenance, rollback path). This epic tracked six child
deliverables across those cycles. Each is verified below against actual repo state on
`main` (commit `2d29b262` at time of writing); no completion is claimed without a traced
bead, commit, PR, or file.

All six deliverables are landed. Both OpenSpec changesets (`config-schema-version`,
`release-provenance`) are archived under `openspec/changes/archive/2026-06-20-*`. Every
child bead is closed. The only deferred items are explicitly optional/out-of-scope for
v1 (artifact signing; optional headless smoke-boot) — none block acceptance, and unlike
the sibling audit epic hud-1aswu (owner-deferred F2 to 2026-09-12) this epic has no
time-bounded residual-risk window.

**The release/ops-hardening + scrub epic is ready to close.**

---

## 2. Deliverable-by-Deliverable Outcomes

### D1 — Config `schema_version` fail-closed compatibility gate

**Source**: Cycle 3 (release/ops); OpenSpec `config-schema-version`
**Implementing bead**: hud-9tyx7g (closed) — PR #906, merged commit `aa0eddbd`
**Outcome**: FULL

**Evidence**:
- `crates/tze_hud_config/src/raw.rs` — `schema_version: Option<u32>` with `JsonSchema`
  derive, so the field surfaces in `--print-schema` output.
- `crates/tze_hud_config/src/loader.rs:53–59` — documented `MAX_SUPPORTED_CONFIG_SCHEMA_VERSION`;
  `loader.rs:138–157` — the gate runs at step (0) of `validate`, before field-level
  checks: absent → treated as current; in-range → proceed; newer → fail closed with
  `CONFIG_SCHEMA_VERSION_UNSUPPORTED`, binding no port.
- `crates/tze_hud_config/src/schema.rs:81–89` — test asserting the exported schema
  contains `schema_version`.
- `crates/tze_hud_config/src/tests.rs:1988–2047` — three scenario tests: absent
  (loads as current), in-range (proceeds), newer-than-supported (fails closed).

### D2 — CI checksum provenance (pipeline-generated SHA-256)

**Source**: Cycle 3; OpenSpec `release-provenance`
**Implementing bead**: hud-sgurmi (closed) — PR #910, squash commit `908ef81c`
**Outcome**: FULL

**Evidence**:
- `.github/workflows/release-provenance.yml` — cross-builds `tze_hud.exe`
  (`x86_64-pc-windows-gnu`), generates `tze_hud.exe.sha256`, and publishes both as
  workflow artifacts. This is pipeline-generated, replacing the former manual
  `sha256sum` README note.
- `.github/workflows/ci.yml:542–598` — `release-artifact-provenance-build` job stages
  the artifact + checksum; a distinct downstream job consumes them.

### D3 — Release-artifact smoke test (provenance round-trip)

**Source**: Cycle 3; OpenSpec `release-provenance` §3
**Implementing bead**: hud-sgurmi (closed) — same PR #910
**Outcome**: FULL

**Evidence**:
- `scripts/ci/release_artifact_provenance_smoke.sh` — builds the real release `.exe`,
  computes its SHA-256, and asserts it matches the published checksum; fails on mismatch.
- `.github/workflows/ci.yml:595–598` — `release-artifact-provenance-smoke` job
  (`needs: [release-artifact-provenance-build]`) runs the round-trip, kept distinct from
  the config-only `canonical-app-production-boot` gate per the spec.
- Verifier logic is unit-tested (`scripts/ci/test_release_artifact_provenance_smoke.py`,
  per prior report §2.2).

### D4 — Rollback runbook

**Source**: Cycle 3
**Implementing bead**: hud-hbxseh (closed) — direct fast-forward merge, commit `36e248ef`
**Outcome**: FULL

**Evidence**:
- `docs/operations/tzehouse-windows-rollback.md` — preflight checks, known-good artifact
  verification (SHA-256 against the pipeline checksum from D2/D3), `TzeHudOverlay`
  stop/replace/restart sequence, post-rollback verification, and escalation path;
  cross-references `docs/operations/tzehouse-windows-recovery.md` for host-offline cases.
- `scripts/ci/rollback_smoke_test.sh` — scripted smoke test exercising the documented
  steps against a fixture.

### D5 — AGENTS.md secret/host scrub (P0)

**Source**: Cycle 0 (scrub)
**Implementing bead**: hud-yotlg3 (closed) — PR #902, commit `67239987`
**Outcome**: FULL

**Evidence**:
- `git log -S "windows-host.example"` confirms the placeholder set landed in `67239987`
  ("security(hud-yotlg3): scrub real Windows host/user/key from all tracked files").
- `AGENTS.md:281` documents the contract: the real Windows host/user/SSH key are
  scrubbed from all tracked files and replaced with placeholders (`windows-host.example`,
  `hud-user`/`admin-user`, `hud-ssh-key`); the real mapping lives only in the
  git-ignored `docs/operations/private/tzehouse-windows.local.md` (template:
  `docs/operations/HOST-TARGET.example.md`). `git check-ignore` confirms the private
  mapping file is untracked.
- A tracked-file scan for RFC1918 host IPs, real usernames, and PSK values found no
  leak of the production tailnet host/user/key. Remaining `--psk <psk>` occurrences in
  `AGENTS.md` are literal angle-bracket placeholders or shell variables, corroborated by
  the D6 investigation.

### D6 — PSK rotation (P0)

**Source**: Cycle 0 (scrub) — conditional on a real PSK ever having been committed
**Implementing bead**: hud-39164f (closed) — direct merge, commit `701429c4`
**Outcome**: FULL — verdict PLACEHOLDER/NEVER-REAL; no rotation required

**Evidence**:
- `docs/audits/hud-39164f-psk-investigation-20260621.md` (commit `701429c4`) —
  investigation across the working tree, full git history of `AGENTS.md`, scripts,
  docs/evidence, and configs. Findings: `DEFAULT_PSK = "tze-hud-key"`
  (`app/tze_hud_app/src/main.rs:91`) is a trivial placeholder the runtime rejects at
  startup; all other PSK-shaped values are test fixtures; `AGENTS.md` only ever used a
  `<psk>` placeholder; captured Windows process listings in `docs/evidence/` had `--psk`
  arguments redacted.
- Decision: no rotation, no Bitwarden entry change, no history rewrite — the embedded
  value was never a real secret. Independently corroborates the owner-accepted finding
  from hud-yotlg3.

---

## 3. OpenSpec Change Status

| Change | Location | Archival |
|---|---|---|
| `config-schema-version` | `openspec/changes/archive/2026-06-20-config-schema-version/` | **Archived** |
| `release-provenance` | `openspec/changes/archive/2026-06-20-release-provenance/` | **Archived** |

Both changes were archived in commit `6eb19244` ("docs: archive config-schema-version +
release-provenance OpenSpec changes [hud-xv1ak]"). The archived `tasks.md` files show all
required items checked; `release-provenance` item 3.3 (optional headless smoke-boot)
remains unchecked and is explicitly optional. This clears the sole open follow-up that
the prior Tier B report (§5) had left pending.

---

## 4. Child Bead Status Summary

| Bead | Title | Status |
|---|---|---|
| hud-9tyx7g | Config schema_version field + fail-closed version gate (loader) | closed |
| hud-sgurmi | CI: release-artifact smoke test (package + integrity round-trip) | closed |
| hud-hbxseh | Rollback runbook + rollback smoke test | closed |
| hud-39164f | SECURITY: rotate the Windows HUD PSK if the embedded value was real | closed |
| hud-yotlg3 | SECURITY: scrub host/PSK/topology from git-tracked AGENTS.md | closed |
| hud-6ot6a1 | Tier B release-hardening + ops-scrub closeout report | closed |

All child work is closed. (Dep-links from hud-e3bmtp to these beads are not recorded in
the graph; membership is established here by source-cycle, subject matter, and traced
commits.)

---

## 5. Acceptance

The epic's stated acceptance was: *all children closed; both OpenSpec changes archived;
closeout report filed.*

- All children closed — §4. ✔
- Both OpenSpec changes archived — §3, commit `6eb19244`. ✔
- Closeout report filed — this document. ✔

**Acceptance met. The epic is ready to close.**

---

## 6. Deferred / Out-of-Scope (non-blocking)

| Item | Disposition |
|---|---|
| Artifact signing (cosign/gpg) | Explicitly optional/deferred for v1 in `release-provenance/proposal.md`; no key infrastructure yet. Not a bead. |
| Headless smoke-boot of packaged exe (`release-provenance` task 3.3) | Marked optional; depends on feasibility of headless Windows exe execution in CI. |
| Evidence-log host-IP hygiene | `docs/evidence/text-stream-portals/vmvalidate-20260704/*` records the disposable autonomous IaC test-VM address (`192.168.4.45`, sentinel Proxmox lab host), not the scrubbed production tailnet host. Out of scope for the D5 P0 scrub (which targeted the production host/user/key); noted only for future evidence-capture redaction hygiene. |

None of these introduce a time-bounded residual-risk window; unlike sibling epic
hud-1aswu (owner-deferred F2 to 2026-09-12), this epic carries no analogous deferral.
