# Reconciliation: hud-nsyt Epic vs. Sibling Bead Deliverables

**Issue:** hud-nsyt.6
**Date:** 2026-03-26
**Scope:** Epic hud-nsyt — "Address v1 spec-code divergences from external review" — versus the
four implementation beads (hud-nsyt.1 through .4) and one documentation bead (hud-nsyt.5).
**Branch:** agent/hud-nsyt.6

---

## 1. Epic Requirements (Source of Truth)

The epic description identifies five divergences from an external static audit (2026-03-25),
grouped by priority:

| # | Priority | Divergence | Assigned Bead |
|---|----------|-----------|--------------|
| E1 | P1 | MCP zone bypass: `handle_publish_to_zone` creates tiles directly, bypassing the real zone engine | hud-nsyt.1 |
| E2 | P1 | Hot-reload contradiction: `RuntimeContext` blocks hot-reload; spec §9 requires it as v1-mandatory | hud-nsyt.2 |
| E3 | P1 | Dev-mode config leak: `HeadlessConfig::default()` grants `FallbackPolicy::Unrestricted` in production builds | hud-nsyt.3 |
| E4 | P2 | Legacy wire debt: deprecated pre-RFC-0004 messages co-reside in normative `events.proto` | hud-nsyt.4 |
| E5 | P3 | Stale audit trail: generate a gen-3 reconciliation doc after P1 closures | hud-nsyt.5 |

The epic's stated goal is to close all five divergences and reach **honest spec-complete status for
v1-MVP**.

---

## 2. Requirement-to-Bead Coverage Checklist

Each row maps a specific requirement within the epic's scope to what the assigned bead delivered.

### E1 — MCP zone bypass (hud-nsyt.1, PR #202, merged 2026-03-25)

Acceptance criteria from bead description:
1. Replace the tile-creation shortcut in `handle_publish_to_zone` with a call to
   `SceneGraph::publish_to_zone` (or equivalent mutation batch path).
2. MCP handler constructs `ZoneContent` from the markdown payload, delegates geometry, contention,
   media validation, and storage to the zone engine.
3. Fix `list_zones` to read `zone_registry.active_publishes[zone_name].is_empty()` for
   `has_content` instead of the tile-namespace heuristic.
4. Update/add tests in `tze_hud_mcp` tests and `zone_ontology.rs`.

| Criterion | Status | Evidence |
|-----------|--------|---------|
| `handle_publish_to_zone` calls real zone engine | FULL | `SceneGraph::publish_to_zone_with_lease` called; enforces `ContentionPolicy`, validates `accepted_media_types`, respects `geometry_policy`, stores to `zone_registry.active_publishes`; tile creation deferred to compositor |
| `ZoneContent` constructed from MCP payload | FULL | Lease grants `Capability::PublishZone(zone_name)` per spec capability vocabulary |
| `list_zones` reads `active_publishes` | FULL | `has_content` checks `zone_registry.active_publishes` directly (authoritative), not tile-namespace heuristic |
| `publish_to_zone` no longer requires active tab | FULL | PR removes `NoActiveTab` requirement; zone publishing is global (not tab-scoped) per v1 |
| Tests updated | FULL | `test_publish_to_zone_basic`, `test_publish_to_zone_no_tab_succeeds`, `test_publish_to_zone_contention_policy_latest_wins`, `test_list_zones_has_content_flag`; all 53 `tze_hud_mcp` tests pass |

**E1 verdict: FULL — all acceptance criteria met.**

---

### E2 — Hot-reload contradiction (hud-nsyt.2, PR #201, merged 2026-03-25)

Acceptance criteria from bead description (Option A selected):
1. Add a `reload` method to `RuntimeContext` that re-parses the hot-reloadable sections
   (`[privacy]`, `[degradation]`, `[chrome]`, `[agents.dynamic_policy]`) and atomically swaps them.
2. Wire SIGHUP handler in the runtime main loop (or equivalent gRPC `ReloadConfig` entry point).
3. Ensure `RuntimeContext` doc and spec §9 agree.

The bead's design notes offered Option B (downgrade spec to post-v1) as an alternative. PR #201
chose Option A (implement reload).

| Criterion | Status | Evidence |
|-----------|--------|---------|
| `reload_hot_config()` method on `RuntimeContext` | FULL | Atomically swaps `ArcSwap<HotReloadableConfig>` with validated config |
| `HotReloadableConfig` carries the four hot-reloadable sections | FULL | `[privacy]`, `[degradation]`, `[chrome]`, `[agents.dynamic_policy]` all in `reload.rs::HotReloadableConfig` |
| Frozen sections remain immutable after construction | FULL | `[runtime]`, `[[tabs]]`, `[agents.registered]` remain frozen; two-tier model documented in module doc |
| Lock-free read accessor | FULL | `hot_config() -> Arc<HotReloadableConfig>` returns `load_full()` snapshot |
| Spec §9 contradiction resolved | FULL | Module doc updated with field classification table matching `reload.rs` |
| Tests | FULL | 11 new tests covering reload scenarios, field isolation, cold-start defaults |

**Note on SIGHUP wiring:** The bead description calls for wiring SIGHUP in the runtime main loop.
PR #201 implements the `reload_hot_config()` integration point but the notes in the bead do not
explicitly confirm SIGHUP signal handling is live. The gen-3 reconciliation doc (hud-nsyt.5) also
does not mention SIGHUP signal wiring. The spec requires:

> "The runtime MUST support live configuration reload via SIGHUP or RuntimeService.ReloadConfig gRPC call."
> (RFC 0006 §9 — v1-mandatory)

The `reload_hot_config()` method exists and is the integration point for both signals. Whether the
SIGHUP signal handler and the `ReloadConfig` gRPC RPC are actually wired to call this method is not
confirmed by the PR body alone.

**E2 verdict: PARTIAL — core reload mechanism is FULL; SIGHUP signal handler and `ReloadConfig` RPC
wiring to `reload_hot_config()` are unconfirmed in the PR evidence.**

---

### E3 — Dev-mode config leak (hud-nsyt.3, PR #204, merged 2026-03-25)

Acceptance criteria from bead description:
1. Gate the `config_toml: None` path behind `#[cfg(any(test, feature = "dev-mode"))]`.
2. Update `HeadlessConfig::default()` to require `config_toml` in non-dev builds (or remove
   `Default` impl).
3. Update the vertical_slice example to use the `dev-mode` feature flag.
4. Ensure all test code that relies on `config_toml: None` continues to compile.
5. Document the feature flag in crate-level docs.

| Criterion | Status | Evidence |
|-----------|--------|---------|
| `HeadlessConfig::default()` gated on `cfg(any(test, feature = "dev-mode"))` | FULL | `crates/tze_hud_runtime/src/headless.rs` updated |
| `build_runtime_context()` returns `Err` in production when `config_toml: None` | FULL | Diagnostic error message directs to `dev-mode` feature |
| `HeadlessRuntime::new()` propagates error via `?` | FULL | PR body confirms |
| `dev-mode` feature added to `Cargo.toml` | FULL | `crates/tze_hud_runtime/Cargo.toml` |
| Integration tests declare `features = ["dev-mode"]` | FULL | `tests/integration/Cargo.toml` updated |
| Examples declare `dev-mode` feature | FULL | `examples/vertical_slice/Cargo.toml` updated |
| `#[cfg(test)]` code paths work without feature flag | FULL | Unit tests (462 pass) compiled with `cfg(test)` |
| Crate-level docs updated | FULL | `crates/tze_hud_runtime/src/lib.rs` documents the feature |

**E3 verdict: FULL — all acceptance criteria met.**

---

### E4 — Legacy wire debt (hud-nsyt.4, PR #203, merged 2026-03-25)

Acceptance criteria from bead description (Option A selected):
1. Move deprecated messages to a separate file (Option A: `events_legacy.proto`).
2. `events.proto` contains only the current RFC 0004 model.
3. Existing consumers that import the legacy types continue to work without Rust-code changes.
4. `build.rs` updated.

| Criterion | Status | Evidence |
|-----------|--------|---------|
| Deprecated messages moved to `events_legacy.proto` | FULL | `InputEvent`, `InputEventKind`, `TileCreatedEvent`, `TileDeletedEvent`, `TileUpdatedEvent`, `LeaseEvent`, `LeaseEventKind`, `SceneEvent` moved |
| Legacy file has prominent deprecation header | FULL | `option deprecated = true`; header directs readers to RFC 0004 types |
| `events.proto` contains only RFC 0004 model | FULL | `InputEnvelope`, `EventBatch`, pointer/key/gesture/focus events with binary UUIDv7 IDs only |
| Same `package tze_hud.protocol.v1` — no Rust code changes needed | FULL | Both files use same package; generated types land in same `crate::proto` module |
| `session.proto` imports `events_legacy.proto` for `SceneDelta` references | FULL | PR body confirms; `SceneDelta` still references legacy tile/lease event types |
| `build.rs` updated to compile all four proto files | FULL | PR body confirms |

**E4 verdict: FULL — all acceptance criteria met.**

---

### E5 — Stale audit trail / gen-3 reconciliation (hud-nsyt.5, merged 2026-03-26)

Acceptance criteria from bead description:
1. Generate `docs/reconciliation-gen3.md` after P1 closures.
2. Update coverage numbers from gen-2 baseline (32 FULL / 13 PARTIAL / 9 RFC-ONLY / 1 ABSENT).
3. Close the audit trail opened by gen-1 and gen-2.

| Criterion | Status | Evidence |
|-----------|--------|---------|
| `docs/reconciliation-gen3.md` created | FULL | File exists in repo; 531 lines |
| P1 closure verification (hud-nsyt.1, .2, .3) documented | FULL | §1 of gen-3 doc covers all three with line-level evidence |
| hud-nsyt.4 housekeeping noted | FULL | §2 notes the proto split with no matrix-row impact |
| Coverage matrix updated | FULL | 58 rows across 11 categories; gen-2 delta documented |
| Remaining gaps identified | FULL | GAP-G3-4 (live capability revocation), GAP-G3-5 (Layer 1 colour assertions), GAP-G3-6 (per-frame correctness fields) |

**E5 verdict: FULL — all acceptance criteria met.**

---

## 3. Summary Coverage Table

| Epic Req | Title | Assigned Bead | Verdict |
|----------|-------|--------------|---------|
| E1 (P1) | MCP zone bypass | hud-nsyt.1 | **FULL** |
| E2 (P1) | Hot-reload contradiction | hud-nsyt.2 | **PARTIAL** |
| E3 (P1) | Dev-mode config leak | hud-nsyt.3 | **FULL** |
| E4 (P2) | Legacy wire debt | hud-nsyt.4 | **FULL** |
| E5 (P3) | Gen-3 reconciliation doc | hud-nsyt.5 | **FULL** |

**Overall: 4 of 5 requirements FULL, 1 PARTIAL (E2 — SIGHUP/ReloadConfig wiring unconfirmed).**

The epic's stated goal ("reach honest spec-complete status for v1-MVP") is substantially met. The
one PARTIAL item (E2) does not break the v1 thesis — the `reload_hot_config()` integration point
is in place and the core contradiction is resolved. The unconfirmed gap is in the signal/RPC
plumbing that calls that method.

---

## 4. Gap Analysis

### GAP-NSYT-1: SIGHUP handler and ReloadConfig RPC wiring not confirmed

**Requirement source:** RFC 0006 §9 (v1-mandatory): "The runtime MUST support live configuration
reload via SIGHUP or RuntimeService.ReloadConfig gRPC call."

**What hud-nsyt.2 delivered:** `RuntimeContext::reload_hot_config()` — the integration point that
atomically swaps `HotReloadableConfig`. The mechanism is sound.

**What is unconfirmed:**
- Is a `signal_hook` or `tokio::signal::unix::Signal` listener wired to call `reload_hot_config()`
  on SIGHUP in the runtime main loop?
- Is `RuntimeService.ReloadConfig` RPC defined in `session.proto` and routed to
  `reload_hot_config()` in the gRPC server?

**Why this matters:** Without at least one of these two wiring points, the spec requirement is not
exercisable end-to-end. An operator sending SIGHUP would get no reload behavior.

**Suggested action:** `task`, P2. Verify and, if absent, implement:
1. SIGHUP handler in runtime main loop calling `RuntimeContext::reload_hot_config()`.
2. OR `RuntimeService.ReloadConfig` gRPC RPC routed to `reload_hot_config()`.
3. Test: reload test scene sends SIGHUP (or gRPC call), asserts config swap observed via
   `hot_config()` snapshot.

---

## 5. Spec Requirements Beyond the Epic's Scope

The hud-nsyt epic specifically addresses five divergences from the 2026-03-25 external audit. Many
additional spec requirements from RFC 0006 (configuration) are **not in scope for this epic** but
are worth flagging for awareness, as gen-3 does not cover them.

The configuration spec (RFC 0006, `openspec/changes/v1-mvp-standards/specs/configuration/spec.md`)
contains **v1-mandatory** requirements beyond what hud-nsyt.1–.5 address:

| Spec Requirement | Notes |
|----------------|-------|
| TOML parse errors with line/column numbers | RFC 0006 §1.2 |
| Config file resolution order (CLI > env > cwd > XDG > APPDATA) | RFC 0006 §1.3 |
| Profile auto-detection (`profile = "auto"`) | RFC 0006 §3.5 |
| Mobile profile schema-reserved (`CONFIG_MOBILE_PROFILE_NOT_EXERCISED`) | RFC 0006 §3.3 |
| Profile budget escalation prevention | RFC 0006 §3.6 |
| Zone registry configuration and validation | RFC 0006 §2.5 |
| Structured validation error collection (all errors, not first) | RFC 0006 §2.9 |
| `--print-schema` CLI flag | RFC 0006 §8 |
| Quiet hours configuration | RFC 0006 §7.1 |
| Headless virtual display dimensions (`headless_width`, `headless_height`) | RFC 0006 §4.4 |
| Redaction style ownership (must live in `[privacy]`, not `[chrome]`) | RFC 0006 §2.8 |

These are **not gaps in hud-nsyt** — they are outside the epic's stated scope (which was the five
identified divergences, not full RFC 0006 coverage). They are noted here for completeness and should
be addressed in a dedicated RFC 0006 configuration hardening epic.

---

## 6. Final Assessment

**Scope adherence:** All five sibling beads worked within the epic's stated scope. No bead
overreached or introduced unrequested changes.

**P1 divergences:** 2 of 3 are definitively closed (E1, E3). E2 has its core contradiction resolved
but has one unconfirmed wiring step (GAP-NSYT-1).

**P2 divergence:** E4 (legacy wire debt) is fully resolved.

**P3 divergence:** E5 (gen-3 audit trail) is fully resolved.

**Remaining gap requiring follow-up:**
- **GAP-NSYT-1** (suggested: `task`, P2): Confirm or implement SIGHUP/ReloadConfig wiring to
  `reload_hot_config()`.

**V1 thesis impact of gaps:** None. The v1 thesis ("An LLM with only MCP access can publish a
subtitle to a zone with one tool call") was established by hud-nsyt.1 and is provable from the
code. GAP-NSYT-1 affects operational hot-reload usability, not the core thesis proof.

---

*Report generated by Beads Worker agent on branch `agent/hud-nsyt.6`.*
