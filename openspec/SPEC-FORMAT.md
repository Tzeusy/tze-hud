# OpenSpec Format Reference

This file documents the conventions used by `openspec/specs/` files in this
repository. Follow these rules when creating or updating spec documents.

## Spec File Structure

Each spec file lives at `openspec/specs/<family>/spec.md` and follows this
template:

```
# <family> Specification
Status: <implemented | v1-reserved | deferred | parked>

## Purpose
<One or two sentences describing what the spec governs. Must be specific —
no scaffold text.>

Implementation: <crate-or-module-path>

## Requirements

### Requirement: <Name>
<Prose requirement text.>
Source: <RFC citation(s) or doc path(s)>
Scope: <v1-mandatory | v1-reserved | post-v1>

#### Scenario: <Name>
- **WHEN** <trigger condition>
- **THEN** <expected outcome>
- **AND** <additional outcome>
```

## Spec Status Convention

Every spec file MUST carry a header-level `Status:` line immediately after the
H1 title (before `## Purpose`). This records the spec's *implementation
lifecycle* and is **orthogonal** to the requirement-level `Scope:` field
(`v1-mandatory | v1-reserved | post-v1`) used inside `### Requirement:` blocks:
`Status:` describes the whole spec's realization; `Scope:` describes an
individual requirement's v1 obligation.

| `Status:` value | Meaning |
|---|---|
| `implemented` | The spec is realized in shipping code (an `Implementation:` crate exists and is substantial). |
| `v1-reserved` | The contract is frozen for v1 but implementation is partial or not-yet-wired (e.g. a tracked integration seam). |
| `deferred` | Parked for post-v1; not implemented and not gating v1. |
| `parked` | Indefinitely parked (no planned implementation), e.g. deferred-indefinitely contracts. |

Where a spec also carries an explanatory banner (for example a
`> **DEFERRED INDEFINITELY**` note), the banner prose MAY remain as context, but
the canonical machine-readable signal is the `Status:` field.

## Implementation Source-Reference Convention

Every spec file SHOULD carry an `Implementation:` line immediately after the
`## Purpose` prose. This line cites the primary Rust crate(s) or module(s)
that implement the spec, so readers can navigate from a contract to its code.

**Format:**

```
Implementation: <path> [; <path> ...]
```

Where `<path>` is a repo-root-relative path to a crate directory or source
file — the same form used in `Source:` fields at requirement level.

**Examples:**

```
Implementation: crates/tze_hud_scene/
Implementation: crates/tze_hud_widget/
Implementation: crates/tze_hud_projection/
Implementation: crates/tze_hud_input/src/drag.rs; crates/tze_hud_scene/src/element_store.rs
Implementation: examples/widget_publish_load_harness/
```

**Rules:**

- The cited path MUST exist in the repository at time of writing.
- Cite the most specific path that is still stable (prefer a crate root over a
  single `.rs` file unless the spec maps to a single well-defined module).
- When a spec spans two crates (e.g., scene model + runtime wiring), list both
  separated by `; `.
- Specs whose implementation is entirely deferred (scope `post-v1`) MAY omit
  the `Implementation:` line or carry `Implementation: (deferred)`.
- Do not invent paths. If the implementation does not yet exist, omit the field.

## Source Reference Convention (Requirement Level)

Individual requirements carry a `Source:` field that cites the authoritative
RFC section or document that originated the requirement:

```
Source: RFC 0001 §2.1
Source: RFC 0008 §3.1, §4.3
Source: about/heart-and-soul/architecture.md, crates/tze_hud_runtime/src/lib.rs
```

`Source:` and `Implementation:` serve different purposes:

| Field | Level | Purpose |
|---|---|---|
| `Implementation:` | Spec (header) | Code that realizes the spec |
| `Source:` | Requirement | RFC section / doc that originated the requirement |

## Scope Values

| Value | Meaning |
|---|---|
| `v1-mandatory` | Required for v1 ship |
| `v1-reserved` | Contract frozen for v1; implementation may be partial |
| `post-v1` | Explicitly deferred beyond v1 |
