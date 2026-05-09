# Engineering Bar

The unified quality standard for tze_hud. Every behavior-changing PR must meet this bar. Every reviewer must check against it.

## 1. Testing Standards

The five-layer testing architecture in `heart-and-soul/validation.md` is authoritative. Read it. What follows is what it does not say.

**When to write tests.** Every PR that changes observable behavior ships with tests. "Observable" means: API output, rendered pixels, telemetry values, error codes, state transitions. Pure refactors that preserve all observable behavior may skip new tests but must not break existing ones.

**Adequate coverage.** Layer 0 (scene graph assertions) should cover 60%+ of test cases. Property-based tests (`proptest`) are expected for state machines, constraint systems, and any logic with a combinatorial input space. Example-based tests are acceptable for deterministic transforms and protocol conformance. If you are writing `assert_eq!(result, 42)` and cannot explain why 42 is the only correct answer, write a property test instead.

**Test quality over quantity.** A test that validates an invariant across 10,000 random inputs is worth more than 50 point-value assertions. Tests must be deterministic (injectable clocks, seeded randomness), diagnostic (structured failure output, not bare `assert!`), and honest (hard to game by overfitting).

## 2. Performance Budgets

Consolidated from RFCs. All times are p99 unless noted. Hardware-normalized per `validation.md` calibration protocol.

| Budget | Value | Source |
|--------|-------|--------|
| Total frame time | < 16.6ms | RFC 0002, RFC 0003 |
| Stage 1: Input Drain | < 500us | RFC 0003 section 5.1 |
| Stage 2: Local Feedback | < 500us | RFC 0003 section 5.1 |
| Stage 3: Mutation Intake | < 1ms | RFC 0003 section 5.1 |
| Stage 4: Scene Commit | < 1ms | RFC 0003 section 5.1 |
| Stage 5: Layout Resolve | < 1ms | RFC 0003 section 5.1 |
| Stage 6: Render Encode | < 4ms | RFC 0003 section 5.1 |
| Stage 7: GPU Submit+Present | < 8ms | RFC 0003 section 5.1 |
| Stage 8: Telemetry Emit | < 200us | RFC 0003 section 5.1 |
| Input to local ack | < 4ms | validation.md, RFC 0002 |
| Input to scene commit | < 50ms (local agent) | validation.md |
| Input to next present | < 33ms (2 frames @ 60Hz) | validation.md |
| Event classification | < 5us per event | RFC 0010 |
| Event delivery to subscriber | < 100us from emission | RFC 0010 |
| Gesture recognizer update | < 50us per recognizer | RFC 0004 |
| Total gesture recognition | < 1ms from final event | RFC 0004 |
| Sync group drift | < 500us | validation.md, RFC 0003 |
| Session memory overhead | < 64 KB per session | RFC 0002 |
| Max aggregate event rate | 1000 events/second | RFC 0010 |

A change that moves a metric closer to its budget ceiling (even while still passing) is a regression. Track trends, not just pass/fail.

### Windows-First Locked Budgets (May 2026)

These budgets apply only to the Windows-first runtime lane. They are calibrated against the May 2026 baseline in `docs/reports/windows_perf_baseline_2026-05.md`.

Reference hardware tag: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`), Intel Core i5-13600KF, NVIDIA GeForce RTX 3080 driver `32.0.15.9636`, 16 GiB RAM, 4096x2160 at 60 Hz, Windows 11 Pro `10.0.26200` build `26200`, `C:\tze_hud\tze_hud.exe` in overlay mode. Baseline calibration factors from the 600-frame run: CPU `0.854`, GPU `0.338`, texture upload `0.215`.

Any future Windows performance number quoted in docs, PRs, or release notes must carry this reference tag, a newer approved reference tag, or an explicit statement that the number is not comparable.

#### CI-Enforced Headless Windows Gate

The PR gate runs the existing `examples/benchmark` headless harness on `windows-latest`, emits the normal benchmark JSON artifact, and validates it with `scripts/ci/check_windows_perf_budgets.py`. The checker scales these TzeHouse raw budgets by `current_factor / TzeHouse_factor`; missing calibration is a gate failure. This avoids a live TzeHouse dependency in PR CI while still failing regressions in the benchmark/artifact path.

| Metric | Locked TzeHouse budget | Normalization | Scope |
|--------|------------------------|---------------|-------|
| `steady_state_render.frame_time` p99 | ≤ 8.3 ms | GPU factor | Headless Windows benchmark |
| `steady_state_render.frame_time` p99.9 | ≤ 16.6 ms | GPU factor | Headless Windows benchmark |
| `steady_state_render.input_to_local_ack` p99 | ≤ 2 ms | CPU factor | Headless Windows benchmark |
| `steady_state_render.input_to_scene_commit` p99 | ≤ 25 ms | CPU factor | Headless Windows benchmark |
| `steady_state_render.input_to_next_present` p99 | ≤ 16.6 ms | GPU factor | Headless Windows benchmark |
| `high_mutation.frame_time` p99 | ≤ 8.3 ms | GPU factor | Headless Windows benchmark |
| `high_mutation.frame_time` p99.9 | ≤ 16.6 ms | GPU factor | Headless Windows benchmark |
| `high_mutation.input_to_local_ack` p99 | ≤ 2 ms | CPU factor | Headless Windows benchmark |
| `high_mutation.input_to_scene_commit` p99 | ≤ 25 ms | CPU factor | Headless Windows benchmark |
| `high_mutation.input_to_next_present` p99 | ≤ 16.6 ms | GPU factor | Headless Windows benchmark |
| Lease violations / budget overruns / sync-drift violations | `0` | none | Both benchmark scenarios |

#### Manual Or Reference-Host Gates

These budgets require TzeHouse or another explicitly approved reference host. They are not required on ordinary PRs because they need a live desktop session, exclusive GPU access, or long-duration soak conditions.

| Metric | Locked budget | Gate |
|--------|---------------|------|
| Widget SVG re-rasterization, gauge 512x512 | ≤ 7.0 ms p99/max regression ceiling; aspirational target remains ≤ 1.0 ms after profiling | Reference-host Criterion artifact |
| Transparent-overlay composite cost vs fullscreen | ≤ +0.5 ms p99 added | Manual `windowed-overlay-perf` workflow with `fail_on_budget=true` |
| Idle CPU, overlay mode, no agents | ≤ 1% of one core | Reference-host resource sample |
| Idle GPU, overlay mode, no agents | ≤ 4.0% Windows GPU engine sum regression ceiling; aspirational target remains ≤ 0.5% after cleaner sampling | Reference-host resource sample |
| Memory growth, three-agent 60-minute soak | ≤ 5 MiB total drift | Reference-host soak report |

Reference-host claim path: coordinate through the current Windows benchmark owner before running live perf work, acquire the GPU lock for the whole measurement window, launch the benchmark HUD with `app/tze_hud_app/config/benchmark.toml` and a non-default PSK, write artifacts under `C:\tze_hud\perf\<bead-id>\`, then copy the report inputs into `docs/reports/` or attach them to the PR. A run that cannot prove the reference tag and command shape is informational only.

### D18 Media Budgets (v2 real-decode lane)

The following thresholds apply to the dedicated self-hosted GPU runner nightly real-decode CI lane. Reference codecs are H.264 + VP9; reference streams are the fixed library checked into LFS. Source: signoff-packet D18.

| Budget | Threshold | Notes |
|--------|-----------|-------|
| Glass-to-glass latency p50 | ≤ 150 ms | Measured end-to-end from capture to display |
| Glass-to-glass latency p99 | ≤ 400 ms | Hard ceiling; regression if trend moves toward it |
| Decode-drop rate | ≤ 0.5% | Frames dropped during decode under reference load |
| Lip-sync drift | ≤ ±40 ms | AV offset; measured against reference stream timestamps |
| Time-to-first-frame (TTFF) | ≤ 500 ms | From session media-admit to first decoded frame presented |

These budgets are gated on the real-decode lane (nightly + label-gated on PRs via `run-real-decode`). Synthetic-only CI does not enforce them but reports trends. See D20 for the full CI cadence matrix.

## 3. Code Quality

**Clippy.** `cargo clippy` must pass clean. Suppress with `#[allow(clippy::...)]` only with a comment explaining why the lint is wrong for this case.

**Unsafe.** Allowed only for FFI boundaries (libc thread priority, platform APIs) and GPU resource mapping. Every `unsafe` block must have a `// SAFETY:` comment explaining the invariants upheld. No unsafe for performance shortcuts.

**Error handling.** Use `thiserror` for all error types. Every user-visible error gets a stable error code (see section 5). Errors must carry enough context to diagnose without a debugger: what happened, what was expected, what input caused it. Never `unwrap()` in library code; `expect()` only for invariants that are truly impossible to violate.

**API design.** Zero-copy where the API boundary allows it. Builder pattern for structs with more than three optional fields. Accept `impl Into<T>` at public API boundaries. Return concrete types, not `impl Trait`, from public APIs (callers need to name the type). Enums over stringly-typed variants.

**Edition and MSRV.** Rust 2024 edition. MSRV 1.88, pinned by the glyphon/wgpu/winit upgrade chain. Do not bump without coordinating the full dependency cascade.

## 4. Review Standards

Every code review checks:

1. **Correctness.** Does the logic match the spec? Are edge cases handled? Are state machine transitions valid?
2. **Performance.** Does any hot-path change risk exceeding a budget in section 2? Are allocations on frame-critical paths justified?
3. **API surface.** Are new public types, methods, or fields intentional? Do they follow the conventions in section 3?
4. **Test coverage.** Does the PR include tests? Do the tests validate the right properties (invariants, not point values)?
5. **Error handling.** Are new error variants documented with stable codes? Do error messages carry diagnostic context?
6. **Documentation.** If the PR changes a public API or adds a crate, are docs updated?
7. **Real-decode lane.** For any change touching the media pipeline: real-decode lane green on the GPU runner per D18 thresholds (label `run-real-decode` on PR or confirm nightly pass).
8. **Device lane.** For any change touching device profiles, capability negotiation, or input handling: primary device lane green (1× iPhone, 1× Android, 1× Mac, 1× Windows, 1× Linux) per D19/D20 coverage requirements.

PR merge requires: CI green, no unresolved review threads, approval present, branch up-to-date, no merge conflicts, no `.beads/` divergence. See `development.md` for the full six-condition guard.

## 5. Observability

**Stable error codes.** Every error type that reaches an agent or log has a string code (e.g., `LEASE_EXPIRED`, `BUDGET_EXCEEDED`). Codes are append-only -- never rename or reuse a code.

**Structured tracing.** Use the `tracing` crate for all instrumentation. Every span carries the subsystem name. Frame-level telemetry is emitted per DR-V3 (`validation.md`). Tracing output is JSON-structured for LLM consumption.

**Telemetry for debugging.** Per-frame telemetry records (frame time, draw calls, texture uploads, lease violations, sync drift) are the primary diagnostic surface. They are designed for LLM parsing first, human reading second.

## 6. Dependency Hygiene

**Minimal dependencies.** Every new crate dependency requires justification. Prefer std or existing workspace dependencies. No "convenience" crates that wrap three lines of code.

**Version pins.** GPU stack versions are co-pinned: wgpu 24 + winit 0.30 + glyphon 0.8 + Rust 1.88. Bumping any one requires bumping the chain. This is documented in `Cargo.toml` workspace dependencies.

**Workspace dependencies.** All shared dependencies are declared in the workspace `Cargo.toml` `[workspace.dependencies]` and referenced via `{ workspace = true }` in crate-level manifests. No crate declares its own version of a workspace dependency.

## 7. Documentation

**When to update.** Any PR that changes a public API, adds a new crate, modifies protobuf schemas, or changes observable behavior must update the relevant docs (openspec delta specs, RFC amendments, or crate-level rustdoc).

**What not to document.** Internal implementation details, private helper functions, temporary workarounds. These change too fast and mislead future readers.

**Rustdoc.** Every public type and function has a doc comment. Module-level docs explain the crate's role in the system and link to the relevant RFC/spec. Examples in doc comments must compile (`cargo test --doc`).

## 8. v2 Release Gate — Tiered Issue Classification (D21)

The following tier contents are the v2 release gate. Source: signoff-packet D21. Every issue discovered during v2 development or validation is classified into one of these tiers before a phase closeout or release tag proceeds.

**Critical — always blocks release:**
- Compositor hang or crash
- Audit log gap (any missing mandatory audit event per C17)
- Embodied session state-machine violation
- Revocation completing in more than 1 second
- Media escaping its sandboxed surface

**Major — blocks unless waived by named approver (v2 tech lead + ≥1 external reviewer):**
- p99 latency regression > 20% above D18 thresholds
- Decode-drop rate > 1%
- Recording artifact left unflushed on session close
- Lip-sync drift > 50 ms

**Minor — warning, does not block:**
- Unit test flake rate < 1%
- Non-primary device lane failure (cloud farm / long-tail breadth)
- Documentation gap
- Performance regression < 5% below budget ceiling

Approver identity is recorded per-phase in the phase closeout report (`docs/reports/`). Waivers require a named approver's explicit sign-off in the PR thread.

## 9. Validation Lane Co-tenancy

The existing `mcp-stress-testing` harness (MCP HTTP endpoint stress test tool; see `openspec/specs/mcp-stress-testing/spec.md`) cohabits with the v2 media validation lane in Layer 3 of the validation framework. Both extend Layer 3 as complementary dimensions: `mcp-stress-testing` characterizes network-facing MCP endpoint latency and throughput; the v2 media lane characterizes real-decode pipeline performance against D18 budgets. The v2 `validation-operations` spec must not conflict with the existing Layer 3 MCP stress integration — share the Layer 4 artifact output conventions (`benchmarks/`) and the structured JSON report format already established by the `mcp-stress-testing` harness.
