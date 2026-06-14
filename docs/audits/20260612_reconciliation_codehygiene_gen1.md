# Reconciliation (gen-1): Code-Hygiene Chore-Wave Audit Remediation

**Epic**: hud-3qpgv — "Audit 2026-06-12 remediation: code hygiene chore wave"
**Reconciliation bead**: hud-3qpgv.7
**Date**: 2026-06-15
**Source findings**: `docs/audits/20260612_project_review.md` §5.3–5.9, §7 (10x / 1-year scale), §8 risks 6/12, §9 quick-wins/Q5
**Method**: Re-read epic + all six sibling bead descriptions/acceptance criteria, then audited the *actually delivered* state on `origin/main` (rebased this worktree onto current main so PR #879 / `df7509e9` is in scope) via `git grep`, scoped `cargo clippy -p tze_hud_protocol`, and direct file reads of the touched crates.

This bead **cannot mutate beads** (worker contract). It records the reconciliation as a durable artifact and reports every coverage gap as a structured follow-up for the coordinator to materialize. It does **not** close hud-3qpgv.7 or the epic.

> **Structural note**: since the audit was written (2026-06-12), the two named god-files were split into submodule trees: `tze_hud_scene/src/graph.rs` → `graph/` (G-7…G-13 refactor PRs #860–#868) and `tze_hud_protocol/src/session_server.rs` → `session_server/`, and `tze_hud_compositor/src/renderer.rs` → `renderer/`. The audit's `file:line` anchors (e.g. `graph.rs:17 sites`, `session_server.rs:450/484/627/1875/2730`, `renderer.rs:487/549/4436/7388/7602`) therefore no longer resolve verbatim; this reconciliation maps each finding to its **current** location.

---

## 1. Finding → Bead Coverage Matrix

| # | Source finding (audit anchor) | Implementing bead | Status | Coverage | Verified delivered state |
|---|---|---|---|---|---|
| F1 | Dual `ResourceBudget` structs with conflicting defaults — `scene/src/types.rs:330` (256 MiB, u32 tiles) vs `scene/src/lease/mod.rs:118` (64 MiB, u8 tiles, max 8 leases); drift exactly at scale pressure (§7 1-year risks, §8 risk 12, §9 Q5) | **hud-3qpgv.1** (closed, PR #772) | closed | **FULL** | Single canonical `pub struct ResourceBudget` now lives in `crates/tze_hud_scene/src/types.rs:350`; the lease layer **re-exports** it (`lease/budget.rs:32` `use super::ResourceBudget;`). `lease/budget.rs:8-15` documents the two-level relationship: `ResourceBudget` is "the single canonical type defined in `crate::types` and re-exported through the parent module; this file adds the *checking* layer on top of it." No second `struct ResourceBudget` remains anywhere in `tze_hud_scene/src/`. |
| F2 | Silent `try_lock`-skip on the shared `Arc<Mutex<SceneGraph>>` (~15 sites in `windowed.rs`); class bit twice (commits 3cc8692c, b6fd414d) (§5.4, §7 10x, §8 risk 6, §9 Q5) | **hud-3qpgv.2** (closed, PR #781) | closed | **FULL** | `FrameTelemetry::scene_lock_miss_count: u64` added (`tze_hud_telemetry/src/record.rs:183`, zero-init :214). Incremented at the frame-loop try_lock-miss site (`windowed.rs:2278`, `saturating_add(1)`) and snapshotted into telemetry every frame (`windowed.rs:2225`). Session-level rollup `scene_lock_misses` (`record.rs:588`, running-max merge :652-655). Drop-on-full safety preserved (counter is a plain `u64` mutated on the compositor thread, not via the bounded sender). Unit tests assert zero-init + JSON round-trip (`record.rs:1089`, `:1100`). Double-buffer follow-on recorded in close reason as DEFERRED. |
| F3a | ~22 production `unwrap()` vs the bar's "never `unwrap()` in library code" — 17 in `graph.rs`, 5 in `session_server.rs` (§5.4, §5.5, §5.10) | **hud-3qpgv.3** (closed, PR #879 / `df7509e9`) | closed | **FULL** | `graph.rs` sites were eliminated by the prior graph→submodule split (verified: **zero** bare `unwrap()` in any `graph/*.rs` production file; the 368 unwraps in `graph/tests.rs`+`spec_scenarios.rs` are test-only and exempt). The one remaining production check-then-unwrap, in `session_server/service.rs::reset_element_geometry`, is now `expect(...)` with an invariant string (`service.rs:358`). **Zero** production `unwrap()` remain in `session_server/*.rs` (all 368 are in `session_server/tests.rs`, gated by `#[cfg(test)] mod tests;` at `mod.rs:1568`). |
| F3b | Unjustified `#[allow(clippy::too_many_arguments)]` (5 sites in `renderer.rs`) vs the bar's "comment explaining why the lint is wrong" rule (§5.3) | **hud-3qpgv.3** (closed, PR #879) | closed | **FULL** | All 12 `too_many_arguments` allows across `tze_hud_compositor/src/` now carry a justification comment. PR #879 added the two that lacked one: `renderer/text.rs:144-146` (`make_zone_text_item`) and `renderer/tile_render.rs:419-424` (`render_node`, combined with `only_used_in_recursion`). Spot-checked every site (pipeline.rs, markdown.rs, token_colors.rs, text.rs, widget.rs, renderer/mod.rs) — each has rationale on the preceding line(s). `cargo clippy -p tze_hud_protocol --lib` green; PR also verified compositor clippy `-D warnings` clean. |
| F3c | Hand-rolled constant-time PSK compare with in-code TODO (`auth.rs:25-30`) — swap for `subtle::ConstantTimeEq` (§5.10) | **hud-3qpgv.3** (closed, PR #879) | closed | **FULL** | `auth.rs:27` `use subtle::ConstantTimeEq;`; `ct_eq_str` (`auth.rs:41`) is `a.as_bytes().ct_eq(b.as_bytes()).into()`; used at both PSK comparison sites (`auth.rs:96` capability path, `:165` legacy path). `subtle = { workspace = true }` added to `crates/tze_hud_protocol/Cargo.toml:43` with a dependency-policy justification comment (`:35-42`, BSD-3-Clause, "de-facto-standard constant-time" crate already used by mcp/projection). The hand-rolled fold is gone. |
| F3d | ~10 missing `// SAFETY:` comments in `media_apple`/`media_android` FFI crates (§5.9) | **hud-3qpgv.3** (closed, PR #879) | closed | **FULL (intent met)** | PR #879 backfilled `# Safety`/`// SAFETY:` on the previously-uncommented FFI sites: media_apple `vt_output_callback` + `create_format_description`; media_android `extern "C"` block + `JNI_OnLoad` + `run_gst_android_init`. Manual sweep of every `unsafe {` block in `media_apple/src/session.rs` and `media_android/src/lib.rs` found **no** unsafe expression lacking an adjacent SAFETY rationale. The raw word-count mismatch (apple 39 `unsafe`/38 SAFETY; android 24/23) is a **counting artifact, not a gap**: one SAFETY comment legitimately covers consecutive `unsafe {}` exprs (e.g. the three `CMTime`/`kCMTimeInvalid` sites at `session.rs:312-314` share the SAFETY at `:311`), and `unsafe fn`/`unsafe impl`/`unsafe extern` *declarations* carry `# Safety` doc sections rather than `// SAFETY:` line comments. |
| F4 | `projection_authority` **binary** has zero tracing — diagnostic hole for an externally-facing daemon (§5.6 weakness 1) | **hud-3qpgv.4** (closed, merged `33fa3ca9`) | closed | **FULL** | `crates/tze_hud_projection/src/bin/projection_authority.rs` now has 28 tracing call-sites. `init_tracing()` matches the app's `TZE_HUD_LOG`/`TZE_HUD_LOG_JSON` convention; `info` on startup/shutdown/attach/detach/cleanup, `debug` on publish/poll/ack/set_token_map, `warn` on rejects + malformed stdin. Observability-only, no behavior change (per close reason: fmt/clippy/check clean, 12+79+5 tests pass). |
| F5 | No `rust-toolchain.toml` (drift from CI's 1.88), no task runner, lint policy only as CI flags / no `[workspace.lints]` (§5.8 weakness 2) | **hud-3qpgv.5** (closed, PR #786) | closed | **FULL** | `rust-toolchain.toml` (378 B) and `justfile` (6.0 KB) present at repo root; `Cargo.toml` has `[workspace.lints.rust]` (:133) and `[workspace.lints.clippy]` (:137) encoding the `-D warnings` policy. |
| F6 | 312 merged-but-undeleted branches; `--delete-branch` documented but unenforced (§5.15, §8, §9 Q5) | **hud-3qpgv.6** (closed, PR #785) | closed | **FULL (automation leg verified)** | `.github/workflows/delete-merged-branches.yml` present — the post-merge automation leg is delivered. (The one-time bulk-delete of the 312 historical branches is a transient git-state action, not a repo artifact, so it cannot be re-verified from the tree; close reason + PR #785 record it.) |

### Epic-level / scale findings not owned by a single sibling

| Audit anchor | Status in this epic | Note |
|---|---|---|
| §8 risk 6 (silent try_lock-skip) | covered by F2 | telemetry counter delivered; double-buffer snapshot is an explicitly DEFERRED follow-on, not an epic gap |
| §8 risk 12 (dual ResourceBudget) | covered by F1 | — |
| §7 1-year risk (ResourceBudget drift) | covered by F1 | — |
| §7 10x ceiling (single scene mutex) | partially observed by F2 | F2 instruments the contention; the structural remedy (double-buffer / snapshot eval) is deferred by design |

---

## 2. Coverage Summary

- **Findings fully covered (9 of 9):** F1 (ResourceBudget, hud-3qpgv.1), F2 (try_lock telemetry, hud-3qpgv.2), F3a–F3d (unwrap→expect / clippy justifications / subtle PSK / SAFETY backfill, all hud-3qpgv.3), F4 (projection tracing, hud-3qpgv.4), F5 (toolchain/justfile/lints, hud-3qpgv.5), F6 (merged-branch automation, hud-3qpgv.6).
- **Findings owner-deferred:** none.
- **Uncovered gaps requiring NEW beads:** **none** from the chore wave's own scope. Every §5.3–5.9 / §8 risk 6&12 / §7 finding the epic claimed maps to a closed, verified sibling. The epic's acceptance condition "every finding mapped to an implementing bead" is **met**, and "gaps become child beads" is vacuously satisfied (no gaps).

Two observations adjacent to the audit but **outside this epic's deliberately-narrowed chore-wave scope** surfaced during verification (see §3). They were already split off into other epics by the audit's own taxonomy and are reported as follow-up candidates for the coordinator's discretion, **not** as failures of hud-3qpgv.

---

## 3. Adjacent observations (NOT epic gaps — coordinator discretion)

These are explicitly **out of scope** for the code-hygiene chore wave. The epic description scopes itself to "§5.3–5.9, §8 risks 6/12, §9 quick wins/Q5" and "deliberately avoids the high-churn god files' active feature areas." The following belong to other §5 domains and/or the enforcement-machinery epic (hud-1aswu), and are recorded only so the coordinator has full visibility.

1. **§5.6 weakness 2 — no automated telemetry trend gate across CI runs** (`validation.md` demands trend surfacing). hud-3qpgv.2 delivered the *counter*; surfacing miss-rate trends across CI runs is a separate observability-CI item, not part of the chore wave. Candidate follow-up.
2. **§5.4 / §7 deferred — double-buffered scene snapshot for the commit path.** hud-3qpgv.2 explicitly DEFERRED this and recorded a recommendation in its close reason; it is the intended next step once miss-rate data accrues. Candidate follow-up (M/L effort), gated on observed miss data from F2's counter.

Neither blocks closing hud-3qpgv.7 or the epic: both are deferrals the wave acknowledged by design, not unaddressed in-scope findings.

---

## 4. Verification commands (reproducible)

```bash
# rebased this worktree onto current main first (PR #879 = df7509e9 in scope)

# F1: single canonical ResourceBudget, no second struct
git grep -n "struct ResourceBudget" -- crates/tze_hud_scene/src   # only types.rs:350

# F2: counter present + wired
git grep -n "scene_lock_miss_count" -- crates/tze_hud_telemetry crates/tze_hud_runtime/src/windowed.rs

# F3a: zero production unwrap in graph/ and session_server/ (368 in *tests.rs only)
git grep -n "\.unwrap()" -- crates/tze_hud_scene/src/graph        # tests.rs/spec_scenarios.rs only
git grep -n "\.unwrap()" -- crates/tze_hud_protocol/src/session_server  # tests.rs only

# F3b: every too_many_arguments allow justified
git grep -n "allow(clippy::too_many_arguments" -- crates/tze_hud_compositor/src

# F3c: subtle PSK compare
git grep -n "ConstantTimeEq\|ct_eq" -- crates/tze_hud_protocol/src/auth.rs

# F4: projection_authority tracing
git grep -cn "tracing::\|info!\|debug!\|warn!\|init_tracing" -- crates/tze_hud_projection/src/bin/projection_authority.rs

# F5/F6: harness + automation artifacts
ls rust-toolchain.toml justfile .github/workflows/delete-merged-branches.yml
git grep -n "workspace.lints" -- Cargo.toml

# clippy spot-check (scoped — full -p suites hang headless)
cargo clippy -p tze_hud_protocol --lib   # green
```

---

## 5. Conclusion

The code-hygiene chore wave (hud-3qpgv) achieves **full coverage** of its claimed source findings (§5.3–5.9, §8 risks 6/12, §7 scale risks). All six implementing siblings are closed and merged to main; every deliverable was re-verified against the *current* (post-god-file-split) tree. No in-scope gaps remain, so no gen-2 reconciliation and no new child beads are required. The two adjacent observations in §3 are design-acknowledged deferrals outside the wave's scope, reported for coordinator visibility only.

The epic hud-3qpgv is **reconcile-clean and ready to close** at the coordinator's discretion.
