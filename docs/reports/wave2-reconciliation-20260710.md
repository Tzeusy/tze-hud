# Portal Wave-2 Reconciliation — Failure-UX + Author Ergonomics

**Bead:** hud-uctcp (reconciliation of epic hud-3jxfr, parent hud-wse80)
**Date:** 2026-07-10
**Branch:** agent/hud-uctcp (based on origin/main @ 64ce7475)
**Method:** Deep spec-to-code audit — every verdict is cited to `file:line` in the
worktree at reconciliation time. No behavioral code was changed; behavioral gaps
are filed as follow-ups (§8), not patched inline.

Reconciled against the `portal-disconnect-resume-ux` OpenSpec change
(`openspec/changes/portal-disconnect-resume-ux/{proposal,tasks,specs/text-stream-portals/spec}.md`)
and the 2026-06-21 7-dimension portal audit catalog (hud-0yrix).

---

## 1. Epic status at reconciliation

Epic hud-3jxfr has 8 children; 6 closed, 1 blocked-polish (hud-jgf41), 1 = this
reconciliation bead. All implementation PRs are merged into the branch base
(origin/main @ 64ce7475, which already includes #1092/#1097/#1098):

| Child | PR | Area | Verdict |
|---|---|---|---|
| hud-5i16d | #973 | Wire disconnect/degraded trigger into runtime | **LIVE** (§2) |
| hud-h3mvo | #978 | One-shot forced degraded repaint on pure drop | **LIVE** (§2) |
| hud-acy4o | #972 | Plumb MCP attach identity fields | **WIRED** (§3) — 1 minor gap |
| hud-l9lf6 | #970 | tools/list + initialize introspection | **WIRED** (§3) |
| — schemars | #1014 | Derive tools/list inputSchema via schemars | **WIRED** (§3) |
| — slimming | #1094 | Schema/skill token slimming + byte-budget guard | **WIRED** (§3) |
| hud-xlx1r | #974 | Grace-expiry removal + disconnect→resume e2e tests | **VERIFIED** (§5) |
| hud-b09ag | #971 | kbd-livelock regression guard | **VERIFIED** (§5) |
| hud-jgf41 | — | Visible disconnect **badge** | **NOT SHIPPED** (blocked; §6) |

---

## 2. KEY QUESTION — is the dormant failure-UX now LIVE?

**YES — end-to-end.** An ungraceful adapter/session drop now produces a visible,
token-styled degraded state within one drain, and clean detach is provably
excluded. The pre-Wave-2 dormancy ("portal keeps looking live after an ungraceful
drop") is resolved. `connection_degraded` is rendered.

Chain of evidence (production, non-test):

1. **Trigger** — MCP `portal_op` ingress dies without a per-projection Detach →
   `drain_portal_ops` hits `TryRecvError::Disconnected` and calls
   `mark_all_projections_disconnected()`:
   `crates/tze_hud_runtime/src/windowed/portal.rs:1466-1492` (call at `:1484-1486`).
2. **Latch** — `mark_all_projections_disconnected` →
   `mark_projection_disconnected_at` → `authority.mark_hud_disconnected(..)`:
   `crates/tze_hud_runtime/src/portal_projection_driver.rs:733-780` (authority call `:754-756`),
   which clears `hud_connection`, drops the advisory lease, and records
   `last_disconnect_wall_us`: `crates/tze_hud_projection/src/authority.rs:324-341`.
   `connection_degraded` is a **derived** value (not a stored bool):
   `authority.rs:1708-1709` (`hud_connection.is_none() && last_disconnect_wall_us.is_some()`),
   surfaced into `ProjectedPortalState` at `authority.rs:1806`. The
   `last_disconnect_wall_us.is_some()` guard keeps a never-connected fresh portal
   out of the degraded state (that surfaces as "connecting", `authority.rs:1717-1718`).
3. **One-shot forced repaint (hud-h3mvo)** — the disconnect transition sets
   `needs_degraded_repaint = true` on the drive entry
   (`portal_projection_driver.rs:388-400`, set at `:764`); the post-due-loop pass
   in `drain_inner` consumes it under the scene lock, re-derives degraded state via
   the same governed `projected_portal_state`, re-renders, and one-shot-clears the
   flag: `portal_projection_driver.rs:2095-2190` (re-derive `:2130`, clear `:2162`,
   render `:2166-2173`). This makes the dim appear on a pure drop **with no
   subsequent publish**. Regression-locked:
   `pure_drop_forces_degraded_repaint_without_subsequent_publish`
   (`portal_projection_driver.rs:5902-5974`, asserts `scene.version` advances with
   no publish and the flag is one-shot).
4. **Visible + token-styled** — the resident adapter reads
   `is_connection_degraded(state)` (`resident_grpc.rs:164-165`) and applies the
   transcript dim from **design tokens**, not hardcoded values:
   `resident_grpc.rs:1165-1170` (`transcript_dim_text_color` / `transcript_dim_background`),
   sourced from `PortalPartTokens` keys `portal.transcript.dim_text_color` /
   `portal.transcript.dim_background` (`crates/tze_hud_config/src/portal_tokens.rs:168-170`).
   Test `degraded_state_uses_dim_transcript_colors` (`resident_grpc.rs:3003-3037`)
   proves the dim comes from the flag, not a constant.
5. **Clean detach does NOT falsely degrade** — `mark_projection_disconnected_at`
   early-returns `false` for an unknown `projection_id`
   (`portal_projection_driver.rs:746-748`); a cleanly-detached projection is already
   absent from the drive map, so a late drop cannot resurrect it. Tests
   `clean_detach_*` / late-drop at `portal_projection_driver.rs:5831-5864`.
6. **Reconnect clears it** — `record_hud_connection` (`authority.rs:294-322`) via
   `clear_projection_disconnect_at` (`portal_projection_driver.rs:796-835`), fired
   from the Attach re-attach path (`:1111`) and the owner-Publish path (`:1193`).

**Nuance on "badge":** the visible degraded state today is (a) the token-styled
transcript **dim** and (b) a content-free marker **line**
`"⊘ disconnected — stream stale"` (`resident_grpc.rs:27`, emitted at `:1449-1456`
before the redaction-gated lifecycle line so it survives redaction). There is **no
dedicated badge widget** — that is hud-jgf41 (§6), which is polish, not a
correctness gap. The doctrine comment at `resident_grpc.rs:1406` deliberately keeps
this ambient ("not a loud badge").

Per-session resident-gRPC stream-end detection (hud-b2llg) was correctly closed
**wontfix**: `resident_grpc_bridge.rs` is a *mirror* client of the single
MCP-driven authority; its bidi stream-end is a recoverable reconnect blip, not an
authoritative portal drop. The authoritative drop path is the MCP channel-close
above. No action needed.

---

## 3. MCP author-ergonomics — attach identity + tools/list

### 3.1 Attach identity fields (hud-acy4o, #972) — WIRED end-to-end, one minor gap

The lossy hardcoded attach (`provider_kind=Other`/generic-icon/`private`) is gone.
Fields flow tools.rs → PortalOp → driver (parsed + validated) → authority session →
identity/ProjectedPortalState:

- `PortalProjectionAttachParams` declares all six as `Option<String>` with
  `#[serde(default)]`: `crates/tze_hud_mcp/src/tools.rs:2201-2227`; forwarded into
  `PortalOp::Attach` at `tools.rs:2278-2289`.
- `PortalOp::Attach` carries them: `crates/tze_hud_mcp/src/portal_op.rs:90-117`.
- Driver parses + validates (no hardcoding):
  `crates/tze_hud_runtime/src/portal_projection_driver.rs:1020-1069` — invalid
  `provider_kind`/`content_classification` → reply `ProjectionInvalidArgument` and
  return (`:1034-1051`); missing `content_classification` defaults to `Private`
  (`driver.rs:217-219`); values passed into `AttachRequest` (`:1054-1069`).
- Stored on `ProjectionSession` and surfaced identity-gated (`reveal_identity`):
  `authority.rs:747-763` (store), `authority.rs:1810-1820` (gated builder).
- 7 tests: parse defaults/rejection (`driver.rs:4750-4811`), identity round-trip
  (`dispatch_attach_identity_fields_round_trip` `:4813`), invalid-enum rejection
  (`:4895`, `:4942`).

**GAP (NEW, minor):** `hud_target` is accepted and bounds-validated
(`contract.rs:350,378`) and forwarded through `PortalOp`, but **`ProjectionSession`
has no `hud_target` field** and the session insert never stores it
(`authority.rs:743-786`). It is accepted-but-inert — never persisted, never
surfaced — yet documented in the attach example across `.claude` / `.gemini` /
`.opencode` `hud-projection/references/operation-examples.md` (`"hud_target": "default"`).
This is the same "documented field silently vanishes" failure mode that
`ergo-attach-fields-dropped` set out to eliminate; it is inert in the current
single-HUD deployment (hence unnoticed). Filed as follow-up FU-1.

### 3.2 tools/list + initialize (hud-l9lf6 #970, schemars #1014, slimming #1094) — WIRED

- Handlers after PSK auth: `crates/tze_hud_mcp/src/server.rs:463-472`
  (`initialize` → `schema::initialize_result()`, `tools/list` → `schema::tools_list_result()`).
- `initialize` returns `protocolVersion` ("2025-06-18"), `serverInfo`,
  `capabilities.tools.listChanged`: `schema.rs:41-52`.
- `tools/list` returns `name` + schemars-derived `inputSchema` (draft-07,
  `inline_subschemas`) for every `portal_projection_*` tool: `schema.rs:56-98`,
  registrations `:146-170`.
- Attach schema carries the six identity fields — locked by
  `attach_schema_reflects_all_struct_fields` (`schema.rs:321-340`).
- #1094 slimming has a **byte-budget regression guard**:
  `tools_list_stays_within_token_budget` (`schema.rs:191-216`) — portal schemas
  ≤ 7,500 B (was 9,336), full tools/list ≤ 20,000 B (was 21,549).

**Bottom line:** an LLM can now discover every portal tool and its schema via
`tools/list`, and project a well-identified portal (provider/workspace/repo/icon/
classification) — the spec's Provider-Neutral Projection Identity requirement is
reachable via MCP. Only `hud_target` routing remains inert (FU-1).

---

## 4. Governance invariants — HELD

The Wave-2 changes route through the **same** redaction, safe-mode, and freeze
gates as pre-Wave-2 surfaces; they add no portal-specific bypass or escape hatch.

- **Redaction — HELD.** `connection_degraded` is derived only from connection
  bookkeeping, "independent of viewer redaction" (`authority.rs:1689-1709`), and
  cannot carry transcript content (`visible_transcript` is populated only when
  `expanded && projection_visible && policy.reveal_transcript`,
  `authority.rs:1685,1727-1734`). Identity fields are all `reveal_identity`-gated
  (`authority.rs:1810-1820`). The forced repaint re-derives via the same governed
  `projected_portal_state` (`portal_projection_driver.rs:2130`), so redaction
  applies identically on the degraded frame. Locked by
  `stale_to_live_transition_respects_redaction_every_frame`
  (`crates/tze_hud_projection/src/tests/mod.rs:4186`).
- **Safe-mode — HELD for transcript content; ONE NARROW GAP on the content-free
  lifecycle-accent overlay (NEW, FU-3).** Safe mode suspends all leases
  (`crates/tze_hud_runtime/src/shell/safe_mode.rs:337`). The forced degraded
  repaint applies its batch via `apply_portal_render_batch_to_scene`
  (`portal_projection_driver.rs:2166-2173` → `crates/tze_hud_protocol/src/convert.rs:1506`),
  whose four mutation branches are **not** uniformly lease-gated:
  `PublishToTile` → `set_tile_root_checked` (`convert.rs:1524`) and
  `UpdateTileInputMode` → `update_tile_input_mode` (`crates/tze_hud_scene/src/graph/tiles.rs:273-275`)
  both enforce `require_active_lease` (→ `is_mutations_allowed()` = `Active` only,
  `tabs.rs:314`, `types.rs:1682-1684`), and `AddNode` → `add_node_to_tile_checked`
  is checked — so the **transcript content is provably blocked** under safe mode
  ("content not painted", `convert.rs:1527`). **But** `SetTileLifecycleAccent` →
  `set_tile_lifecycle_accent` (`convert.rs:1547-1566`) checks only tile existence,
  **not** the lease (`crates/tze_hud_scene/src/graph/overlay.rs:388-401`), and the
  adapter batch **always emits** an accent (`resident_grpc.rs:1080`, "always
  emitted, hud-m48i0"). So a Wave-2 forced degraded repaint can still mutate the
  content-free lifecycle-accent overlay and bump `scene.version` (re-arming the
  present-gate) while safe mode has suspended the lease. This is a **content-free**
  ambient color (no transcript/identity leak) and the accent path is pre-existing,
  but the hud-h3mvo forced repaint is the new trigger that exercises it against a
  suspended lease — so "portal updates suspend under safe mode like other
  content-layer surfaces" does not fully hold for the accent overlay. Filed as
  FU-3. (Input is additionally force-disabled under safe mode, `authority.rs:1724`.)
  Credit: raised by the Codex PR reviewer and verified here.
- **Freeze — HELD.** The projection crate has no portal-specific freeze signal
  (grep-clean); governance freeze queues session-plane mutations only
  (`crates/tze_hud_runtime/src/shell/freeze.rs:8-9`), and the portal driver is not
  wired to it by design ("adapters observe only generic queue-pressure semantics").
  The forced-repaint pass adds no freeze branch.
- **Orphan / lease-grace — HELD at contract, but production reaper UNWIRED
  (pre-existing).** The degraded window has no second timer — it clears only on
  genuine reconnect; the only bound is lease grace (`DEFAULT_GRACE_PERIOD_MS =
  30_000`, `crates/tze_hud_scene/src/graph/leases.rs:12`), and grace expiry removes
  the tile via `expire_leases` → `expire_projection` (`leases.rs:417-443`,
  `authority.rs:561`). Post-grace reattach starts fresh under `lease_is_active`
  (`portal_projection_driver.rs:2375-2393`, `leases.rs:292`). **But
  `scene.expire_leases()` has ZERO production call sites** — every caller is a test
  or doc comment (verified: `portal_projection_driver.rs:5269,7000`, plus
  `tests/*.rs`); the winit frame loop sweeps only zone/widget publications, never
  leases. `disconnect_lease` is likewise test-only, and `expire_projection` in
  production is reached only via managed-session `revoke_session`
  (`crates/tze_hud_projection/src/portal.rs:321`), **not** the cooperative
  in-process driver's grace path. Consequence: on an ungraceful drop the portal
  dims (visible degradation — good) but is **never grace-removed** in the live
  runtime, so the stale window is effectively unbounded — the openspec §3.2
  "staleness bounded by lease grace" / "grace expiry removes the surface" scenario
  is proven only headlessly. This is **not a Wave-2 regression** (Wave-2 added no
  timer and no bypass; it simply did not wire the reaper) and tasks.md **already
  correctly leaves §3.2 unchecked**. Filed as follow-up FU-2 (the keystone reliability
  remainder).

**Verdict:** governance substantially held — redaction and freeze are solid, and
safe mode blocks all transcript-content and input mutations. Two soft spots, both
narrow and neither a content/privacy leak: (i) the grace reaper is unwired in
production (FU-2, pre-existing dormant trigger); (ii) the content-free
lifecycle-accent overlay escapes safe-mode lease suspension via the one
un-lease-gated mutation branch (FU-3, exercised by the Wave-2 forced repaint).

---

## 5. Test coverage — VERIFIED

All Wave-2 tests exist and assert their stated AC; no `#[ignore]`/TODO/inert
markers in the three portal test modules.

- **hud-xlx1r (#974)** — grace-expiry + resume family:
  - `disconnected_portal_surface_removed_on_grace_expiry_yields_no_further_state`
    (`portal_projection_driver.rs:6914`) — disconnect → advance past grace →
    `expire_leases` reaps tile (`:7000-7006`) → `expire_projection` →
    `projected_portal_state().is_none()` (`:7019-7025`). (Drives scene+authority
    steps directly because the production trigger is dormant — self-disclosed at
    `:6907-6912`, corroborating FU-2.)
  - `disconnect_then_reconnect_within_grace_resumes_same_surface_without_duplication`
    (`:7048`) — same tile resumes, `tile_count()==1`, `!connection_degraded`,
    interaction re-enabled, each committed unit present exactly once.
  - §4 resume family in `crates/tze_hud_projection/src/tests/mod.rs`:
    `reconnect_resumes_from_retained_window_and_clears_stale_treatment` (`:3879`),
    `reconnect_continues_in_progress_unit_in_place_via_coalesce_key` (`:3961`),
    `reconnect_replayed_logical_unit_id_stays_idempotent` (`:4036`),
    `reconnect_materializes_only_bounded_visible_window` (`:4114`),
    `reconnect_preserves_transcript_inbox_ack_state_and_requires_new_lease` (`:2008`),
    `stale_to_live_transition_respects_redaction_every_frame` (`:4186`).
- **hud-b09ag (#971)** — kbd-livelock guard: `drain_keyboard_queue_bounded`
  helper (`crates/tze_hud_runtime/src/windowed/keyboard.rs:255`, used in production
  at `:1409`); bound-guard test
  `drain_bounded_helper_stops_at_initial_limit_when_new_events_arrive_during_drain`
  (`:1731`, proven to FAIL if the bound is removed) + drain-to-zero companion
  (`:1829`).

---

## 6. hud-jgf41 relevance (inspect-only; NOT adopted)

hud-jgf41 ("Render a visible disconnect badge from `connection_degraded`", P3,
**blocked**) is the sole remaining openspec §6.2(a) gap before the
`portal-disconnect-resume-ux` change can archive. Its worktree
`.worktrees/parallel-agents/hud-jgf41` (branch `agent/hud-jgf41`) still holds the
**abandoned over-scoped WIP** — 12 dirty files spanning proto + scene-graph +
compositor (`proto/types.proto`, `scene/{types,mutation,graph/node_tree,graph/overlay}.rs`,
`convert.rs`, `session_server/{mutations,traffic}.rs`, `compositor/renderer/{frame,tile_render}.rs`,
`resident_grpc.rs`, `portal_tokens.rs`), uncommitted, on no remote branch. **Left
untouched** per instruction.

Assessment: the bead was correctly **rescoped 2026-06-22** to a simple token-styled
`SolidColorNode` dot/pill riding the existing hud-h3mvo forced-repaint path (no
proto/scene-graph change); the dirty WIP is the *rejected* heavy approach and should
not be salvaged wholesale. The badge is **polish, not a correctness gap** — the
degraded state is already perceivable (dim + "⊘ disconnected" marker line, §2). It
is gated on promotion-epic hud-g1ena (open, renderer/header ownership) to avoid a
scene-ownership conflict; that gating is still valid. **Recommendation:** keep
hud-jgf41 blocked on hud-g1ena; do not re-grab the WIP. It does not block epic
hud-3jxfr closure (it's polish), but it does block the openspec change archive.

---

## 7. Deferred items & new gaps → follow-ups

Most catalogued deferred items are already tracked and need no new bead:
hud-0yrix (Wave-2/3 backlog), hud-g0c9g (chat-grade affordances: delivery-ack,
unread, typing, timestamps, jump-to-latest), hud-hwk2m (unread count on bridge),
hud-om69w / hud-1e1ry (live disconnect→resume evidence), hud-jgf41 (badge). The
still-unextracted catalog items (connecting-state distinct from disconnected;
owner-token/idempotency-key recovery; expects_reply/Question turn signal;
visual-token discipline items; deep render perf) remain correctly parked in
hud-0yrix and should be expanded when Wave-2.5/3 starts — no premature bead spam.

**Two genuinely NEW gaps found during this reconciliation** (not previously
tracked as dedicated beads) are proposed as follow-ups:

- **FU-1 (bug, P3):** `hud_target` attach hint accepted + validated but never
  persisted on `ProjectionSession` (`authority.rs:743-786`) — documented in
  operation-examples.md but silently inert. Either store + honor it for HUD routing
  or drop it from params + docs. Untested.
- **FU-2 (bug, P2):** Wire the production lease-grace reaper for cooperative
  in-process portals. `scene.expire_leases()` / `disconnect_lease` have no
  production call site, so an ungraceful-dropped portal dims but is never
  grace-removed → unbounded stale window; openspec §3.2 holds only headlessly.
  hud-5i16d wired `mark_hud_disconnected` but not the scene-orphan → expire path.
- **FU-3 (bug, P3):** Lease-gate the lifecycle-accent overlay mutation so it obeys
  safe-mode / lease suspension like the other content-layer mutations.
  `SceneGraph::set_tile_lifecycle_accent` (`crates/tze_hud_scene/src/graph/overlay.rs:388-401`)
  checks only tile existence, not `require_active_lease`, while the sibling batch
  branches (`set_tile_root_checked`, `update_tile_input_mode`, `add_node_to_tile_checked`)
  all do. The adapter always emits an accent (`resident_grpc.rs:1080`), so the
  Wave-2 forced degraded repaint (`portal_projection_driver.rs:2166-2173`) can mutate
  the content-free accent overlay + bump `scene.version` under a safe-mode-suspended
  lease. Content-free (no privacy leak) and the accent path is pre-existing, but it
  breaks the "portal updates suspend under safe mode" invariant for the accent.
  Raised by the Codex PR reviewer, verified. discovered-from hud-uctcp / hud-5i16d /
  hud-h3mvo.

`hud-0yrix` trim: the Wave-2 items now genuinely DONE and removable from the
catalog are — `disconnect-treatment-never-triggered-in-runtime` (§2, hud-5i16d/#973),
`grace-expiry-removal-unverified` (headless-covered by hud-xlx1r/#974, with the
production-reaper remainder now tracked as FU-2), `kbd-livelock-regression-guard`
(hud-b09ag/#971), `ergo-attach-fields-dropped` (§3.1, hud-acy4o/#972 — modulo
FU-1's `hud_target`), and `ergo-no-tools-list` (§3.2, hud-l9lf6/#970). The catalog
edit is a beads mutation, deferred to the coordinator per the worker contract.

---

## 8. Epic closability

- **hud-3jxfr (Wave-2 epic):** closable. All 6 implementation children are merged
  and verified LIVE/WIRED; hud-jgf41 is polish (blocked on promotion, not a
  correctness gap) and does not block the epic. FU-1/FU-2/FU-3 are follow-ups, not
  epic blockers (the failure-UX is visible; FU-2 is the pre-existing grace-reaper
  remainder; FU-3 is a content-free accent-overlay lease-gate gap on a pre-existing
  path).
- **`portal-disconnect-resume-ux` OpenSpec change:** keep OPEN. Its own §6.2(a)
  gate (the hud-jgf41 badge) plus FU-2 (openspec §3.2 production grace-removal) are the
  remaining items before archive. The spec delta itself is sound and unchanged.

---

## Appendix — quality gates

This reconciliation made **no code changes** (report-only). No `cargo fmt` /
`clippy` / test gate applies; the two behavioral gaps (FU-1, FU-2) are filed as
follow-ups rather than patched inline, per the reconciliation-worker contract.
