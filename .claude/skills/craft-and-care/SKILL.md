---
name: craft-and-care
description: >
  Load the project's engineering quality bar before writing, reviewing, or merging code.
  The about/craft-and-care/ directory defines testing standards, performance budgets, code
  quality conventions, review expectations, observability requirements, and dependency hygiene
  for tze_hud. Consult before: writing tests, reviewing PRs, adding dependencies, designing
  error types, or making performance-sensitive changes. Triggers: "quality bar", "review
  checklist", "performance budget", "testing standards", "code conventions", "dependency policy".
---

# Engineering Standards — Craft and Care

The `about/craft-and-care/` directory defines **who we are when we build** — the engineering quality bar that every contributor (human or model) must meet.

**Consult before:**
- Writing or reviewing any behavior-changing PR
- Adding new dependencies to the workspace
- Designing error types or telemetry
- Making performance-sensitive changes on the frame pipeline
- Deciding whether a test is "good enough"

**Do NOT load all files at once.** Start with what you need.

## Document Index

| Document | Read when... | Key content |
|----------|-------------|-------------|
| `about/craft-and-care/README.md` | Quick orientation | Pillar overview, relationship to other pillars |
| `about/craft-and-care/engineering-bar.md` | Before any code review or implementation | Testing standards, performance budgets, code quality, review checklist, observability, dependency hygiene, documentation expectations |

## Key Standards (quick reference)

1. **Testing.** Every behavior-changing PR ships with tests. Property-based (`proptest`) for state machines and combinatorial domains. Deterministic, diagnostic, honest. See `heart-and-soul/validation.md` for the five-layer architecture.
2. **Performance.** 19 quantitative budgets consolidated from RFCs. Frame time < 16.6ms. Input to local ack < 4ms. Event classification < 5us. Track trends, not just pass/fail.
3. **Code.** Clippy clean. Unsafe only for FFI/GPU with `// SAFETY:` comments. `thiserror` for all error types. Stable error codes (append-only).
4. **Review.** Six-point checklist: correctness, performance, API surface, test coverage, error handling, docs.
5. **Dependencies.** Minimal policy. GPU stack co-pinned (wgpu 24 + winit 0.30 + glyphon 0.8 + Rust 1.88). Workspace-level declarations only.

## Relationship to Other Pillars

| Need | Skill |
|------|-------|
| Why an invariant exists | `/heart-and-soul` |
| Wire-level budget source | `/legends-and-lore` |
| What must be built | `/spec-and-spine` |
| Where the code lives | `/lay-and-land` |
