# Epic Closeout Report: Efficiency Doctrine Operationalization (`hud-f670c`)

- **Report bead:** `hud-6e6uk`
- **Evidence cut:** 2026-07-18, against `origin/main` commit `d27005e16cee1697d88cbd4de021ab7b1dd88d7e`
- **Child-lane state:** nine predecessor lanes are closed; this report lane remains the review artifact.
**Closeout posture:** evidence is sufficient to review the completed lanes, but it is not evidence that every efficiency-doctrine requirement is complete. In particular, the active v1 change still requires change-proportional invalidation closure accounting and its 50-tile proof; no implementation evidence was found for that requirement. The parent epic should remain open until a coordinator makes that scope decision.

## Executive summary

The completed work establishes three enforceable efficiency surfaces and two supporting runtime surfaces:

- [Observed] an actual Windows overlay CI lane rejects quiescent intervals with more than zero GPU queue submissions, surface acquisitions, or presents, and more than 120 combined runtime-driven wakeups in a 60-second measurement after a five-second settle period.
- [Observed] an actual two-logical-CPU llvmpipe lane applies the constrained-envelope checker to a canonical workload and fails closed on profile, provenance, metric, or ceiling mismatch.
- [Observed] the owner-approved v1 token-footprint baseline contains three canonical measured flows, deterministic identity metadata, byte/token counts, and a five-percent regression gate.
- [Observed] production degradation wiring and resident-budget accounting were landed as supporting remediation. They are not substitutes for a device qualification program or for invalidation-closure accounting.
- [Observed] the doctrine's change-proportional-work requirement remains unproven in implementation. The audit records full visible-scene rebuilding for active or animated frames; the active OpenSpec delta requires the missing closure model and a one-node-in-50-tiles proof.

No glasses/VR or unapproved v2/candidate work is claimed in this report. The current scope remains Windows desktop quality and constrained software-renderer evidence, as explicitly bounded by the doctrine.

## Doctrine → OpenSpec → implementation traceability

| Doctrine mandate | Contracted requirement | Current implementation and evidence | Assessment |
| --- | --- | --- | --- |
| Idle means no rendered work | `about/heart-and-soul/efficiency.md:37-43`; `openspec/changes/efficiency-budgets/specs/runtime-kernel/spec.md:3-22` require zero submissions, acquisitions, and presents, plus a bounded wakeup interval. | `crates/tze_hud_runtime/tests/idle_efficiency_counters.rs`, `scripts/ci/check_idle_efficiency.py`, and `scripts/ci/windows/run-quiescent-efficiency.ps1` implement counters, a fail-closed checker, and an actual Windows overlay runner. CI run `29589493117` produced `windows-performance-budget/quiescent-efficiency/quiescent-efficiency.json` and a passing gate report. | **Implemented and measured on the declared WARP profile.** This is not a live reference-GPU qualification. |
| Changed work should be proportional to the change | `about/heart-and-soul/efficiency.md:44-47`; `openspec/changes/efficiency-budgets/specs/runtime-kernel/spec.md:24-39` require typed closure accounting and a one-node mutation in a 50-tile scene to leave the other 49 tiles untouched. | No non-spec implementation of the required closure accounting or 50-tile gate was located. `docs/reports/hud-48s45_desktop_headroom_assumption_audit_20260716.md:73` records that active/animated frames rebuild the complete visible scene and vectors. | **Deferred / not demonstrated.** This is a current active v1 contract gap, not a completed lane. |
| Degrade deliberately under pressure | `about/heart-and-soul/efficiency.md:48-50`; `openspec/changes/production-degradation-wiring/specs/runtime-kernel/spec.md:3-65` define the six-level ladder, hysteresis, and atomic render policy. | `hud-cpj4v` landed the policy, runtime/controller, compositor, and protocol wiring; its change tasks are checked in `openspec/changes/production-degradation-wiring/tasks.md`. | **Implemented supporting control surface.** It does not prove per-change closure or device readiness. |
| Retain a constrained resource envelope | `about/heart-and-soul/efficiency.md:51-53`; `openspec/changes/efficiency-budgets/specs/validation-framework/spec.md:105-138` define the two-CPU constrained profile and fail-closed validation. | `scripts/ci/run_constrained_envelope.sh`, `scripts/ci/check_constrained_envelope.py`, and `scripts/ci/test_check_constrained_envelope.py` enforce identity, CPU affinity, observed runtime provenance, metric completeness, and ceilings. CI run `29589493117` produced `constrained-envelope-budget/budget-gate.json`. | **Implemented and measured on the declared llvmpipe proxy.** The artifact explicitly sets `device_qualification=false`. |
| Make canonical interaction footprints measurable | `about/heart-and-soul/efficiency.md:61-84`; `openspec/changes/efficiency-budgets/specs/validation-framework/spec.md:20-101` define deterministic packet measurement, baseline identity, and a five-percent gate. | `scripts/ci/check_token_footprint.py`, `scripts/ci/test_check_token_footprint.py`, `examples/benchmark/TOKEN_FOOTPRINT.md`, and `scripts/ci/token_footprint_candidate_v1.json` provide the checker, tests, fixture contract, and approved baseline. CI run `29577838185` produced two byte-identical measurements and a passing `token-footprint-calibration/gate-report.json`. | **Implemented for the approved three flows.** Whether the widget flow is the doctrine's named status dashboard is **Unknown** and needs a scope decision. |
| Future glasses/VR should inherit the discipline, not silently widen v1 | `about/heart-and-soul/efficiency.md:8-26` and `about/heart-and-soul/v1.md:9-17` keep current delivery Windows-only while treating desktop headroom as preparation. | `hud-48s45` records the desktop-headroom audit and routes future-device risks rather than pretending they are shipped. | **Explicitly out of completed scope.** No VR/device qualification claim is made. |

The active `efficiency-budgets` change validates strictly (`openspec validate efficiency-budgets --strict`), but its unchecked task boxes are not treated as completion evidence. The evidence above uses merged code, checked-in contracts, and CI artifacts instead.

## Measured baselines and guard semantics

### Quiescent Windows overlay lane

The checker contract in `docs/operations/idle-efficiency-measurement.md:1-90` and `scripts/ci/check_idle_efficiency.py:16-212` is:

| Property | Required guard | Observed artifact result |
| --- | --- | --- |
| Measurement window | Five-second settle followed by a 60-second interval | `settle_seconds=5`, `interval_seconds=60` |
| Runtime-driven wakeups | At most 120 combined runtime-driven wakeups | `combined_runtime_driven=0` (main and compositor both `0`) |
| Quiescent GPU/surface work | Exactly zero queue submissions, surface acquisitions, and presents | All three counts were `0` |
| Runtime profile | Actual Windows overlay process, WARP/DX12, two CPUs set at process creation | Microsoft Basic Render Driver, `software=true`, process affinity `0x3` |

Source: GitHub Actions run `29589493117`, artifact `windows-performance-budget/quiescent-efficiency/{quiescent-efficiency.json,quiescent-efficiency-gate.json}`. The operation guide is explicit that this WARP lane is an actual runtime measurement but not a replacement for a live reference-GPU run.

### Constrained-envelope lane

The constrained runner uses an enforced two-logical-CPU `taskset` profile, llvmpipe provenance, a fixed 1920×1080 viewport, and `device_qualification=false`; see `docs/operations/constrained-envelope-benchmark.md:3-74`, `scripts/ci/run_constrained_envelope.sh:16-53`, and `scripts/ci/check_constrained_envelope.py:24-303`.

| Workload / metric | Normalized observed result | Ceiling | Result |
| --- | ---: | ---: | --- |
| Steady-state frame p99 | 661.017 µs | 8,300 µs | Pass |
| Steady-state frame p99.9 | 1,270.892 µs | 16,600 µs | Pass |
| High-mutation frame p99 | 365.803 µs | 8,300 µs | Pass |
| High-mutation frame p99.9 | 368.720 µs | 16,600 µs | Pass |
| Local acknowledgement (worst listed) | 4.534 µs | 2,000 µs | Pass |
| Commit (worst listed) | 15.869 µs | 25,000 µs | Pass |

Source: GitHub Actions run `29589493117`, artifact `constrained-envelope-budget/budget-gate.json`. The artifact records Ubuntu 24.04, llvmpipe LLVM 20.1.2, logical CPU limit `2`, allowed CPUs `0-1`, and an enforced taskset. These figures establish the declared proxy envelope only; they are not a hardware/device benchmark.

### Canonical token-footprint lane

`scripts/ci/token_footprint_candidate_v1.json:4-69` records the owner-approved baseline using `tiktoken-rs 0.12` and `o200k_base`. `scripts/ci/check_token_footprint.py:172-234` rejects an operation above the five-percent budget and rejects incompatible fixture or tokenizer identity.

| Approved canonical flow | Bytes | Tokens | Evidence |
| --- | ---: | ---: | --- |
| `publish_to_zone` | 669 | 197 | Baseline and CI measurement |
| Portal turn | 2,541 | 683 | Baseline and CI measurement |
| `publish_to_widget` | 539 | 168 | Baseline and CI measurement |

Source: GitHub Actions run `29577838185`, artifact `token-footprint-calibration/{measurement.json,repeat.json,gate-report.json}`. Both measurements had the same hash; the gate reported no incompatibilities, regressions, or warnings. `examples/benchmark/TOKEN_FOOTPRINT.md:3-58` documents the real headless/MCP boundary and fail-closed baseline rules.

## Closed child-lane outcomes

| Bead | Merged evidence | Outcome, locations, and caveat |
| --- | --- | --- |
| `hud-5cmia` | PR #1177, merge `b619a83c` | Established the doctrine baseline in `about/heart-and-soul/efficiency.md`, `v1.md`, `vision.md`, `mobile.md`, RFC 0004, and repository guidance. This is the normative source; it does not itself enforce a runtime budget. |
| `hud-le1e0` | PR #1178, merge `789dfd26` | Created the active `openspec/changes/efficiency-budgets/` proposal, design, runtime-kernel delta, validation-framework delta, and RFC alignment. It defines the testable targets, including the still-unimplemented invalidation closure requirement. |
| `hud-gt92q` | PR #1206, merge `be55511f` | Pinned the token-footprint numeric/identity contract in the active efficiency-budgets change and owner-approved baseline. It establishes what a comparable measurement is; the harness execution is supplied by `hud-pngbn`. |
| `hud-cpj4v` | PR #1195, merge `846c7c32` | Wired the production degradation ladder across runtime, compositor, and protocol, with the completed `production-degradation-wiring` OpenSpec tasks as the checked contract. It is related remediation, not proof of a VR cadence path or changed-work proportionality. |
| `hud-k3lfx` | PR #1198, merge `a8d3de54` | Added the profile runtime-budget envelope and resident ledger in `crates/tze_hud_runtime/src/runtime_context.rs` and `crates/tze_hud_resource/src/resident_ledger.rs`, documented by `docs/operations/runtime-resident-memory.md`. The envelope separates full and headless budgets; it is accounting/control, not measured process RSS or a device qualifier. |
| `hud-hnigs` | PR #1209 work included in PR #1204, merge `6b9afb51` | Added producer-wakeup-driven idle behavior, counters, checker, WARP runner, operation guide, and `idle_efficiency_counters` tests. The Windows CI artifact above is the actual measurement proof and carries the WARP limitation. |
| `hud-pngbn` | PR #1192, merge `a7349a43` | Added deterministic token-footprint fixture, checker tests, baseline documentation, and CI calibration artifact. It proves exactly the approved three flows; it does not silently define a separate status-dashboard flow. |
| `hud-rofdj` | PR #1196, merge `fa79dedf` | Added the constrained-envelope runner, checker, checker tests, operation guide, and CI lane. It rejects missing/incorrect provenance rather than broadening thresholds for a different machine. |
| `hud-48s45` | PR #1181, merge `eab64877` | Produced `docs/reports/hud-48s45_desktop_headroom_assumption_audit_20260716.md`, which made desktop assumptions and VR-readiness gaps explicit. Findings F3, F8, and F9 remain reviewer-relevant; F11's missing production command path was remediated later by PR #1185 (`90a7a308`). |

## Supporting runtime decisions

`hud-cpj4v` and `hud-k3lfx` were deliberate supporting lanes rather than substitutes for the core measurements:

- The degradation controller owns a six-level policy with cadence/quality/resource-pressure signals, hysteresis, quiescent recovery, and atomic render-policy publication (`openspec/changes/production-degradation-wiring/specs/runtime-kernel/spec.md:3-65`). This avoids each renderer inventing an independent degradation decision.
- The runtime profile envelope separates full and headless resident budgets (1,024 MiB and 512 MiB respectively) and uses disjoint resident-resource classes (`docs/operations/runtime-resident-memory.md:3-26`; `crates/tze_hud_resource/src/resident_ledger.rs:6-206`). It intentionally prevents cross-class borrowing.

Both are useful remediation from the audit, but neither changes the conclusion on invalidation closure or device qualification.

## Diagram decision

**Decision: no diagram was created.** This is an evidence-and-guard crosswalk, not a new component topology or data-flow design. A diagram would be less precise than the traceability matrix and could misleadingly depict the unimplemented change-proportional path as an operating runtime path. No `docs/reports/diagrams/hud-f670c-*` assets are needed for this report.

## Risks, gaps, and reviewer focus

1. **Active v1 change-proportional-work gap.** The invalidation closure artifact and 50-tile one-node gate are still required by the active `efficiency-budgets` delta. Audit finding F3 is direct evidence that the existing full-scene rebuild behavior needs resolution. This is the principal closeout blocker for claiming doctrine completion.
2. **Status-dashboard token identity is unknown.** The doctrine names a status dashboard, while the approved calibration names zone, portal, and widget operations. A reviewer must decide whether a specific widget fixture is the canonical dashboard or require a separately versioned flow before claiming full coverage.
3. **Constrained CI is not device qualification.** WARP and llvmpipe are valuable fail-closed proxy lanes, but their artifacts explicitly do not establish performance on a physical reference GPU, glasses, or VR hardware.
4. **VR-readiness findings remain outside scope.** The audit's output-view monoscopic path (F8) and blocking `Maintain::Wait` path (F9) require separately scoped decisions. F11's specific production reachability gap was subsequently closed by PR #1185 (`90a7a308`): the Windows keyboard adapter now delivers `RawCommandEvent` through `CommandProcessor` and transactional `CommandInputEvent` dispatch. Other device-specific sources remain deferred. This report makes no future-device delivery claim.
5. **OpenSpec task markers are stale as status signals.** The active efficiency-budgets `tasks.md` retains unchecked boxes even where later merged work supplies evidence. Its strict validation proves structure, not completion; this report deliberately relies on code and artifacts instead.

## Evidence appendix

| Evidence | Location | What it establishes |
| --- | --- | --- |
| Doctrine | `about/heart-and-soul/efficiency.md:8-84` | Idle-zero-work, proportional-change, constrained-envelope, canonical-footprint principles and current Windows-only boundary. |
| Active OpenSpec | `openspec/changes/efficiency-budgets/specs/{runtime-kernel,validation-framework}/spec.md` | The v1 requirements, guard semantics, and outstanding closure requirement. |
| Idle CI artifact | Run `29589493117`, `windows-performance-budget/quiescent-efficiency/{quiescent-efficiency.json,quiescent-efficiency-gate.json}` | Actual WARP overlay zero-work result and passing gate. |
| Constrained CI artifact | Run `29589493117`, `constrained-envelope-budget/budget-gate.json` | Actual two-CPU llvmpipe normalized metrics, provenance, and passing gate. |
| Token CI artifact | Run `29577838185`, `token-footprint-calibration/{measurement.json,repeat.json,gate-report.json}` | Deterministic owner-approved token baseline and passing regression gate. |
| Desktop-headroom audit | `docs/reports/hud-48s45_desktop_headroom_assumption_audit_20260716.md:13-152` | Assumptions, current gaps, and future-device routing. |
| F11 remediation | PR #1185 / `90a7a308`; `crates/tze_hud_runtime/src/windowed/{keyboard,input_dispatch}.rs` | Keyboard-derived pointer-free commands now reach the production `CommandProcessor` and transactional runtime dispatch; this does not add device-specific sources. |

The following commits provide the report's closed-lane provenance: `b619a83c`, `789dfd26`, `be55511f`, `846c7c32`, `a8d3de54`, `6b9afb51`, `a7349a43`, `fa79dedf`, and `eab64877`.
