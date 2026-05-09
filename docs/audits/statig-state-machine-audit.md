# Statig State-Machine Crate Audit

**Issued for**: `hud-0cdu`
**Date**: 2026-05-09
**Auditor**: Codex worker
**Parent context**: `hud-ora8.2` / v2 embodied-media presence, now deferred indefinitely
**Decision packet**: `openspec/changes/_deferred/v2-embodied-media-presence/signoff-packet.md` E26
**Scope**: API stability, macro hygiene, no-std support, protobuf representation compatibility, dependency footprint, and maintainer activity for `statig` as the RFC 0014/RFC 0015 state-machine implementation crate.

---

## Verdict

**ADOPT-WITH-GUARDRAILS, but do not reactivate deferred v2 work.**

`statig` is a reasonable fit for tze_hud's media and embodied state machines when those deferred tracks are resumed. It matches the E26 direction: hierarchical runtime state machines implemented in Rust, with explicit protobuf enum mirrors on the session wire. The crate is small, MIT-licensed, `#![no_std]` at runtime, and already pinned in the workspace as `statig = "0.4"` with lockfile resolution to `statig 0.4.1` and `statig_macro 0.4.0`.

The main caveat is macro-generated public enum shape. tze_hud must continue to isolate generated `statig` state enums behind project-owned enums such as `MediaSessionState`, then map explicitly to protobuf. Do not serialize generated macro names or depend on generated enum layout as a wire contract.

## Current Project Status

The audit is partially post-facto: RFC 0014 and media runtime code already use `statig`.

| Surface | Current status |
|---|---|
| Workspace dependency | `Cargo.toml` declares `statig = "0.4"`; `Cargo.lock` resolves `statig 0.4.1` and `statig_macro 0.4.0`. |
| Runtime users | `crates/tze_hud_runtime/src/media_ingress.rs` and `crates/tze_hud_compositor/src/video_surface.rs`. |
| RFC users | `about/legends-and-lore/rfcs/0014-media-plane-wire-protocol.md` section 3.5. |
| Embodied RFC | RFC 0015 is still forthcoming/deferred; no active RFC 0015 implementation should be spawned from this audit. |
| Program status | `about/heart-and-soul/media-doctrine.md` marks media/embodied work deferred indefinitely as of 2026-05-09. |

## Upstream Snapshot

Verified from upstream sources on 2026-05-09.

| Item | Finding |
|---|---|
| Crate | `statig` |
| Latest crate version | `0.4.1`, published 2025-07-05; not yanked. |
| Macro crate | `statig_macro 0.4.0`, published 2025-07-05; not yanked. |
| Repository | `https://github.com/mdeloof/statig` |
| License | MIT |
| Rust version in crate metadata | 1.66 |
| Runtime crate mode | `#![no_std]` in `statig/src/lib.rs`. |
| Default features | `macro` by default, which pulls `statig_macro`. Optional features: `async`, `serde`, `bevy`. |
| docs.rs metadata | 0.4.1 docs build published; docs.rs reports 89.9% documented for the crate page inspected. |
| GitHub activity | Repository not archived; latest observed commit 2025-11-29 fixed transition hook source/target order; 779 stars, 39 forks, 4 open issues at inspection time. |
| crates.io adoption signal | crates.io API reported 3,884,775 total downloads and 873,648 recent downloads at inspection time. |

Sources:

- `https://crates.io/api/v1/crates/statig`
- `https://crates.io/api/v1/crates/statig_macro`
- `https://github.com/mdeloof/statig`
- `https://api.github.com/repos/mdeloof/statig`
- `https://api.github.com/repos/mdeloof/statig/commits?per_page=8`
- `https://docs.rs/statig/latest/statig/attr.state_machine.html`
- `https://docs.rs/crate/statig/latest/features`
- `https://raw.githubusercontent.com/mdeloof/statig/main/statig/Cargo.toml`
- `https://raw.githubusercontent.com/mdeloof/statig/main/statig/src/lib.rs`
- `https://raw.githubusercontent.com/mdeloof/statig/main/macro/Cargo.toml`

## Audit Findings

### API Stability

**Verdict: acceptable with a workspace pin and wrapper boundary.**

`statig` remains below 1.0, so semver stability is weaker than a mature crate. The 0.4 line is, however, recent relative to the previous 0.3 line, and the upstream repository shows maintenance after the 0.4.1 release.

The API shape fits tze_hud's E26 needs:

- event-driven state handling through `handle` / `handle_with_context`
- state-local storage for state-specific metadata
- hierarchical superstates for shared transitions such as active media states
- transition hooks suitable for audit and telemetry
- blocking and async support, with async behind a feature flag

Guardrail: keep generated `State`/`Superstate` types private to the module that owns the machine. Expose only project-owned state/event/result types. That boundary is already present in `media_ingress.rs`, which maps generated state to `MediaSessionState`.

### Macro Hygiene

**Verdict: acceptable if macro output remains private and compile-time tests cover state tables.**

`#[state_machine]` parses an `impl` block and generates state/superstate enums plus trait implementations. Upstream documentation explicitly exposes customization for generated enum names and derives, and recent upstream commits improved doc propagation for generated enums.

Risks:

- macro diagnostics and generated type names become part of developer ergonomics, even when the runtime behavior is sound
- generated enum names default to `State`/`Superstate`, which can collide or become ambiguous in larger modules
- macro output should not become a stable protobuf, serde, audit-log, or public Rust API contract

Required tze_hud pattern:

1. Name generated enums explicitly when a module contains more than one machine.
2. Derive only local debugging traits on generated enums.
3. Convert to project-owned enums before logging, telemetry, protobuf, or public API exposure.
4. Keep transition tests at the project-owned event/state layer, not the generated-symbol layer.

### No-Std Support

**Verdict: strong for runtime state-machine logic; proc-macro use remains a build-time dependency.**

The runtime crate declares `#![no_std]`, and upstream README states no heap allocation for the state machines. This is favorable for future glasses/device profiles where a no-std-compatible core may matter.

Important distinction: the default `macro` feature pulls in a proc-macro crate, which is a compile-time host dependency and uses the normal proc-macro ecosystem (`syn`, `quote`, `proc-macro2`, `proc-macro-error2`). That does not make the generated target runtime require `std`, but it does affect build footprint and toolchain compatibility.

Future glasses guardrail:

- for constrained targets, validate a target-specific build with `default-features = false` if macro host dependencies are undesirable, or keep macro generation in normal host builds and verify generated runtime code does not pull `std`
- do not enable optional `bevy` on constrained targets
- do not assume optional `serde` is free for embedded profiles; protobuf conversion can stay hand-written

### Protobuf Representation Compatibility

**Verdict: compatible only through an explicit mirror enum.**

E26 requires `statig` plus a protobuf representation in `session.proto`, mirroring the v1 lease state-machine pattern. `statig` should be treated as the runtime executor, not as the wire schema owner.

The compatible pattern is:

- Rust `statig` generated state enum stays private
- project-owned Rust enum names mirror RFC/protobuf states
- protobuf enum values remain append-only and numerically stable
- conversion functions are exhaustive and tested
- unknown/future protobuf values fail closed or map to an explicit `UNSPECIFIED`/`UNKNOWN` path before transition dispatch

RFC 0014 already follows this direction: section 3.5 states that top-level states map to `MediaSessionState` wire values and that Rust internals may use shorter names, while wire serialization must use the protobuf enum. Keep that as the canonical pattern for any future RFC 0015 embodied machine.

### Dependency Footprint

**Verdict: small enough for desktop runtime; acceptable build-time macro footprint.**

Local locked normal dependency tree:

```text
statig v0.4.1
`-- statig_macro v0.4.0 (proc-macro)
    |-- proc-macro-error2 v2.0.1
    |-- proc-macro2 v1.0.106
    |-- quote v1.0.45
    `-- syn v2.0.117
```

The non-macro runtime crate itself is small. The macro stack is standard for Rust proc-macro code and is already common in the broader Rust ecosystem. Optional `serde` and `bevy_ecs` are not enabled by the current workspace lock path.

Guardrail: keep `statig` declared at workspace level only. Any future crate using it should reference `{ workspace = true }` and should not add crate-local feature changes without a dedicated dependency review.

### Maintainer Activity

**Verdict: adequate, with below-1.0 caution.**

The repository is not archived and had post-release maintenance in late 2025. The latest observed commits included a behavioral fix to transition hooks and macro/doc improvements. Issue volume is low. This is enough for an internal state-machine helper, but not enough to delegate wire compatibility or governance semantics to the dependency.

Risk posture:

- acceptable for internal runtime machinery
- not acceptable as an unwrapped public API dependency
- pin to the 0.4 line until a specific upstream fix or feature is needed

## Alternatives Considered

| Alternative | Disposition |
|---|---|
| Hand-written state machines | Viable fallback; more boilerplate and easier to drift from RFC diagrams. Keep as fallback if macro issues appear. |
| Typestate pattern | Poor fit for externally driven runtime events where the next event order is not compile-time-known. |
| `smlang` / other DSL crates | No clear advantage over the already adopted `statig` path; would restart review and migration work without a concrete benefit. |
| Code-generated protobuf-first machine | Strong wire alignment but heavier tooling and less ergonomic for hierarchical runtime behavior. Not justified now. |

## Required Guardrails

1. **No wire leakage**: generated `statig` state names must never be serialized directly.
2. **Explicit conversion tests**: every machine must test project enum -> generated state and generated state -> protobuf mirror coverage where both directions exist.
3. **Property tests for transitions**: terminal states must remain terminal; invalid events must not mutate resource ownership; reconnect/orphan paths must not reuse stale epochs or leases.
4. **Version pin**: keep `statig = "0.4"` at workspace scope. Any update beyond 0.4.x needs a short dependency-change note.
5. **Feature discipline**: do not enable `bevy`; enable `serde` only if a concrete project-owned serialization boundary needs it. Prefer protobuf/manual conversion for wire state.
6. **Deferred-work boundary**: this audit does not authorize new RFC 0015 work or media-plane expansion while `v2-embodied-media-presence` remains deferred.

## RFC Guidance

For RFC 0014, the existing `statig` guidance is acceptable. Keep diagrams as authoritative state semantics, and keep protobuf enums as the authoritative wire contract.

For any future RFC 0015 embodied-presence state machine:

- use the same pattern as RFC 0014 section 3.5
- define a protobuf `EmbodiedPresenceState` enum independently of generated Rust symbols
- include an explicit state table, transition table, and terminal-state rule
- specify how session resume, orphan reclaim, degradation, and operator revocation compose with lease state
- mark glasses/no-std relevance as a build-profile validation concern, not a wire-contract concern

## Final Disposition

`statig` passes the E26 library audit for internal state-machine implementation, subject to the guardrails above. The strongest recommendation is boundary discipline: `statig` owns local transition execution; tze_hud owns RFC semantics, protobuf state representation, telemetry vocabulary, and policy consequences.
