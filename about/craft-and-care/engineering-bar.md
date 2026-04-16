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
