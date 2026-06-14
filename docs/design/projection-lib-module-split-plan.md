# projection crate (lib.rs) Module Split Plan

**Issue**: hud-j8mb6 (planning prep for hud-luovo)
**Date**: 2026-06-14
**Author**: agent/hud-j8mb6
**Status**: Draft

---

## 1. Purpose

`crates/tze_hud_projection/src/lib.rs` (6,891 lines) is the monolithic crate
root for the projection authority — the single most critical shared library in
the stack (consumed by `tze_hud_runtime`, the `projection_authority` binary, and
every test that touches the wire protocol). It carries four distinct concerns in
one flat file: wire contract types, portal state types, process supervision /
external agent management, and the core `ProjectionAuthority` state machine with
all its private session logic.

This document plans splitting `lib.rs` into focused submodule files along its
logical concern boundaries, enabling the same move-only PR discipline proven in
the `session_server.rs` / `renderer.rs` split plan (hud-se14n).

This is a **crate-root split**, not a directory-module split. `lib.rs`
**remains `lib.rs`**; it gains `mod` declarations, the existing `pub mod
portal_cadence` and `#[cfg(feature = "resident-grpc")] pub mod resident_grpc`
declarations stay, and every `pub` item retains its current external path via
`pub use` re-exports. No downstream crate (`tze_hud_runtime`,
`projection_authority` binary) needs to update any `use` path after any step.

This is **planning only** — no Rust code is changed in this document.

---

## 2. Guiding Principles

1. **Move-only commits**: no logic changes in any split commit. Reviewers should
   be able to verify with `diff -u lib.rs submodules/*.rs` that nothing was added
   or deleted — with the explicit exception of **visibility modifiers**. Items
   that were implicitly private to the crate root become `pub(super)` (visible to
   `lib.rs` and sibling submodules) or `pub(crate)` as required. These are the
   minimal mechanical additions required by Rust's module privacy rules and are
   expected in every split commit. Execution PRs must list which items gained
   `pub(super)`/`pub(crate)` in their PR description.
2. **API preservation via `pub use`**: downstream callers must not need to update
   import paths. `lib.rs` re-exports everything from each submodule with `pub use
   self::<module>::*;`. The external API surface (e.g.
   `tze_hud_projection::ProjectionAuthority`) must remain exactly identical.
3. **One submodule per commit** (or a tightly coupled leaf cluster): keeps each
   PR reviewable in isolation.
4. **Tests move with their host**: the large `mod tests { ... }` block (≈ 2,870
   lines) moves to `tests/` as a final step.
5. **Prerequisite beads first**: see Section 5. Do not split before
   cross-concern coupling issues are resolved.
6. **Line numbers are approximate**: `git rebase origin/main` before each commit.
   Use function/type names as anchors, not line numbers.

---

## 3. `lib.rs`

**Location**: `crates/tze_hud_projection/src/lib.rs`
**Lines (at plan date)**: 6,891
**Production lines (excluding tests)**: ≈ 4,021 (L1–4020)
**Test lines**: ≈ 2,870 (L4021–6891)

### 3.1 Verified Section-Banner Seams

`lib.rs` has only **two explicit section banners** — far fewer than
`session_server.rs` (16) or `renderer.rs` (6). Logical boundaries are
identifiable by struct-group progression and process-concern changes rather than
banners.

| # | Approx. line | Banner / anchor text | Contents |
|---|---|---|---|
| 1 | L1017 | `// ─── Composer draft notification types (hud-5jbra.4) ───` | `AdapterDraftNotification`, `AdapterDraftSubmission`, `AdapterDraftCancel`, `AdapterDraftBatch` |
| 2 | L1124 | `// ─── Portal geometry update types (hud-5jbra.9) ───` | `AdapterPortalRect`, `AdapterGeometrySnapshot`, `AdapterGeometryBatch` |

**Logical concern boundaries (no banners, identified by structural analysis)**

| Cluster | Approx. lines | Key contents |
|---|---|---|
| Existing submodules | L11–18 | `pub mod portal_cadence`, `pub mod resident_grpc` (cfg-gated) |
| Wire constants | L29–68 | `DEFAULT_MAX_*`, `PORTAL_UPDATE_RATE_WINDOW_WALL_US`, `MAX_PROJECTION_ID_BYTES`, `MAX_OWNER_TOKEN_BYTES`, `MAX_RETAINER_BYTES` |
| Wire enums (core types) | L70–287 | `ProjectionErrorCode`, `INITIAL_ERROR_CODES`, `ProjectionOperation`, `ProviderKind`, `ProjectionLifecycleState`, `ContentClassification`, `OutputKind`, `InputAckState`, `InputDeliveryState`, `CleanupAuthority`, `ProjectedPortal*` enums (7), `PortalInputFeedbackState`, `ProjectionAuditCategory` |
| Wire structs — bounds/ops | L288–598 | `ProjectionBounds`, `OperationEnvelope`, `AttachRequest`, `PublishOutputRequest`, `PublishStatusRequest`, `GetPendingInputRequest`, `AcknowledgeInputRequest`, `DetachRequest`, `CleanupRequest` |
| Wire structs — input/session | L600–756 | `PendingInputItem`, `PortalInputSubmission`, `PortalInputFeedback`, `HudConnectionMetadata`, `AdvisoryLeaseIdentity`, `ReconnectBookkeeping`, `TranscriptUnit` |
| Wire structs — response/state | L757–1015 | `PortalTranscriptUpdate`, `ProjectionStateSummary`, `ProjectionResponse` (public methods + private `with_portal_update_state`), `ProjectionAuditRecord`, `ProjectionIdentitySummary`, `ProjectedPortalPolicy`, `ProjectedPortalState` |
| Adapter draft batch types | L1017–1122 | `AdapterDraftNotification`, `AdapterDraftSubmission`, `AdapterDraftCancel`, `AdapterDraftBatch` (banner 1) |
| Adapter geometry types | L1124–1219 | `AdapterPortalRect`, `AdapterGeometrySnapshot`, `AdapterGeometryBatch` (banner 2) |
| Managed session types | L1220–1533 | `ManagedSessionOrigin`, `LaunchSessionSpec`, `HudCredentialSource`, `WindowsHudTarget`, `ProjectionAttentionIntent`, `PresenceSurfaceRoute`, `PortalSurfaceKind`, `WidgetParameterValue`, `ManagedSessionRequest`, `HudSurfaceCommandPlan`, `ManagedSessionRoutePlan`, `ManagedSessionHandle`, `RuntimeAuthenticationMaterial` |
| Process supervision | L1534–1894 | `ManagedSessionRecord` (private), `ProviderProcessState` (pub), `ProviderProcessStatus` (pub), `ProviderProcessRecord` (private, holds `child: Child`), `ExternalAgentProjectionAuthority` |
| Contract error type | L1896–1915 | `ProjectionContractError` (pub enum) |
| Authority internals | L1917–3359 | `ProjectionSession` (private), `ProjectionAuditEvent<'a>` (private), `ProjectionAuthority` (pub struct + full `impl` block), `impl Default for ProjectionAuthority` |
| Free helper functions | L3360–4020 | `route_plan_for_request`, `projected_portal_state`, `redacted_feedback`, `portal_id_for_projection`, `validate_pending_input_item`, `append_transcript_unit`, `promote_to_active_if_recovering`, `portal_update_allowed`, `prune_retained_transcript`, `visible_transcript_window`, `capabilities_are_subset`, `remember_logical_unit`, `requested_delivery_state`, `terminal_ack_replay_response`, `remember_terminal_input`, `prune_terminal_pending_input`, `acknowledge_input`, `expire_pending`, `validate_owner_token`, `validate_non_empty_bounded`, `validate_optional_bounded`, `validate_non_zero`, `generate_owner_token`, `verifier_for_secret`, `constant_time_eq`, `hex_encode`, `bounded_copy` |
| Tests | L4021–6891 | `mod tests { use super::*; ... }` (proptest + unit tests) |

### 3.2 Proposed Submodule Breakdown

Target: new files alongside `lib.rs` in `crates/tze_hud_projection/src/`

`lib.rs` **stays as `lib.rs`** (this is a crate root — it cannot become
`mod.rs`). It gains `mod` declarations and `pub use` re-exports.

```
crates/tze_hud_projection/src/
├── lib.rs                   # mod declarations + pub use * from each submodule
│                            # retains: use imports, pub mod portal_cadence,
│                            #   pub mod resident_grpc (cfg-gated),
│                            #   wire constants (L29–68), ProjectionContractError
├── contract.rs              # all pure wire types (no process deps, no authority deps):
│                            #   core enums (L70–287), ProjectionBounds (L288–353),
│                            #   request/response structs (L355–1219 incl. both adapter batches),
│                            #   input/session metadata types (L600–756),
│                            #   ProjectionAuditRecord, ProjectionIdentitySummary,
│                            #   ProjectedPortalPolicy, ProjectedPortalState (L879–1015),
│                            #   PortalTranscriptUpdate, ProjectionStateSummary (L757–878)
│                            #   NOTE: ProjectionResponse moves here too (see §3.3)
├── managed_session.rs       # session orchestration types (no process/Child deps):
│                            #   ManagedSessionOrigin, LaunchSessionSpec, HudCredentialSource,
│                            #   WindowsHudTarget, ProjectionAttentionIntent,
│                            #   PresenceSurfaceRoute, PortalSurfaceKind, WidgetParameterValue,
│                            #   ManagedSessionRequest, HudSurfaceCommandPlan,
│                            #   ManagedSessionRoutePlan, ManagedSessionHandle,
│                            #   RuntimeAuthenticationMaterial (L1220–1533)
├── portal.rs                # process supervision + external authority:
│                            #   ManagedSessionRecord, ProviderProcessState,
│                            #   ProviderProcessStatus, ProviderProcessRecord (holds Child),
│                            #   ExternalAgentProjectionAuthority (L1534–1894)
├── authority.rs             # core authority state machine:
│                            #   ProjectionContractError (L1896–1915),
│                            #   ProjectionSession (private), ProjectionAuditEvent<'a>,
│                            #   ProjectionAuthority struct + full impl block,
│                            #   impl Default for ProjectionAuthority,
│                            #   all free helper functions (L3360–4020)
└── tests/
    └── mod.rs               # existing mod tests { ... } content (L4021–6891)
```

**`lib.rs` retains after split**:
- `#![...] / #[allow(...)]` crate-level attributes
- `#[cfg(feature = "resident-grpc")] pub mod resident_grpc;`
- `pub mod portal_cadence;`
- `mod contract; pub use self::contract::*;`
- `mod managed_session; pub use self::managed_session::*;`
- `mod portal; pub use self::portal::*;`
- `mod authority; pub use self::authority::*;`
- `mod tests;` (after step P-5)
- Top-level `use` imports (L20–27): `use crate::portal_cadence::PortalCadenceCoalescer`, serde, std::collections, std::process, std::env, std::fmt, subtle, thiserror
- Wire constants (L29–68): `DEFAULT_MAX_*`, `PORTAL_UPDATE_RATE_WINDOW_WALL_US`, private limits

**Approximate `lib.rs` size post-split**: ≈ 80–120 lines (attributes + mod decls + pub uses + constants + imports)

### 3.3 Cross-Section Coupling

These coupling points require careful handling and ordering:

| Coupling | Where | Mitigation |
|---|---|---|
| `ProjectionResponse::with_portal_update_state` borrows `&ProjectionSession` | `ProjectionResponse` is a wire type (target: `contract.rs`); `ProjectionSession` is an authority-internal private struct (target: `authority.rs`) | **Move `with_portal_update_state` out of `contract.rs`**: add an `impl ProjectionResponse` block in `authority.rs` containing only this private method. `contract.rs` holds the public struct and public methods; `authority.rs` extends it with the session-coupling method. This is valid Rust (split impl blocks within the same module via `pub use self::contract::ProjectionResponse`). |
| `ProviderProcessRecord` holds `child: Child` (`std::process::Child`) | In `portal.rs` | `portal.rs` imports `use std::process::{Child, Command, Stdio};` — these imports stay scoped to `portal.rs`, not leaked via `lib.rs` |
| `ExternalAgentProjectionAuthority` wraps `ProjectionAuthority` | `portal.rs` wraps a type from `authority.rs` | Import ordering: `authority.rs` must be compiled first; `portal.rs` imports `use crate::authority::ProjectionAuthority` (or `use super::authority::ProjectionAuthority`) |
| `ExternalAgentProjectionAuthority` also holds `ManagedSessionRecord` using `ManagedSessionHandle` | `portal.rs` uses types from `managed_session.rs` | `portal.rs` imports from both `authority.rs` and `managed_session.rs` — no circular dep |
| All free helpers call `ProjectionSession` fields | Free helpers (L3360–4020) reference `ProjectionSession` private fields | Move free helpers INTO `authority.rs` alongside `ProjectionSession` — they are already in the same file and are all private; this is the natural home |
| `PortalCadenceCoalescer` used inside `ProjectionAuthority::handle_publish_output` | From `crate::portal_cadence` (already a separate module) | No change — `authority.rs` imports `use crate::portal_cadence::PortalCadenceCoalescer` (same `crate::` path) |
| Proptest macros in tests | `mod tests` uses `use super::*` | Tests move last; they continue using `use super::*` which expands through `pub use` chain in `lib.rs` — no import path changes needed |
| Constants referenced across submodules | `DEFAULT_MAX_*` etc. referenced in `ProjectionAuthority::new()` and in tests | Keep constants in `lib.rs`; submodules reference them via `crate::DEFAULT_MAX_AUDIT_RECORDS` etc. |

### 3.4 Incremental Sequencing

Perform one step per PR. Each step is a pure move with no logic changes.

**Step P-1: Wire contract types (largest single cluster — leaf, no authority deps)**

Move to `contract.rs`:
- All wire enums (L70–287): `ProjectionErrorCode`, `INITIAL_ERROR_CODES`, `ProjectionOperation`, `ProviderKind`, `ProjectionLifecycleState`, `ContentClassification`, `OutputKind`, `InputAckState`, `InputDeliveryState`, `CleanupAuthority`, all `ProjectedPortal*` enums, `PortalInputFeedbackState`, `ProjectionAuditCategory`
- `ProjectionBounds` + `Default` + `validate` (L288–353)
- All request/envelope structs (L355–598): `OperationEnvelope`, `AttachRequest`, `PublishOutputRequest`, `PublishStatusRequest`, `GetPendingInputRequest`, `AcknowledgeInputRequest`, `DetachRequest`, `CleanupRequest`
- Input/session metadata structs (L600–756): `PendingInputItem`, `PortalInputSubmission`, `PortalInputFeedback`, `HudConnectionMetadata`, `AdvisoryLeaseIdentity`, `ReconnectBookkeeping`, `TranscriptUnit`
- Response/state structs (L757–1015): `PortalTranscriptUpdate`, `ProjectionStateSummary`, `ProjectionResponse` (public struct + public methods only — see §3.3 for the private method), `ProjectionAuditRecord`, `ProjectionIdentitySummary`, `ProjectedPortalPolicy`, `ProjectedPortalState`
- Adapter draft batch types (L1017–1122, banner 1)
- Adapter geometry types (L1124–1219, banner 2)

Add to `lib.rs`:
```rust
mod contract;
pub use self::contract::*;
```

This step accounts for ≈ 1,190 lines leaving `lib.rs`. It is the highest-value
split because `tze_hud_runtime` and the `projection_authority` binary import
almost exclusively from this cluster.

**Visibility changes expected**: none (all types are already `pub`; the module
itself is re-exported via `pub use`). Helper methods on types that are currently
private within `lib.rs` become `pub(super)` if called from `authority.rs`.

**Step P-2: Managed session types**

Move to `managed_session.rs`:
- `ManagedSessionOrigin`, `LaunchSessionSpec`, `HudCredentialSource`, `WindowsHudTarget`, `ProjectionAttentionIntent`, `PresenceSurfaceRoute`, `PortalSurfaceKind`, `WidgetParameterValue`, `ManagedSessionRequest`, `HudSurfaceCommandPlan`, `ManagedSessionRoutePlan`, `ManagedSessionHandle`, `RuntimeAuthenticationMaterial` (L1220–1533)

Add to `lib.rs`:
```rust
mod managed_session;
pub use self::managed_session::*;
```

No deps on `contract.rs` types from this cluster (these are purely session
orchestration enums and structs). Depends on P-1 being merged first so that
`contract.rs` types referenced by `managed_session.rs` (e.g. `ContentClassification`
used in `HudSurfaceCommandPlan`) resolve via `crate::` paths.

**Visibility changes expected**: `ManagedSessionRecord` is already private; it
moves to `portal.rs` in P-3, not here. All types in this cluster are `pub`.

**Step P-3: Process supervision + ExternalAgentProjectionAuthority**

Move to `portal.rs`:
- `ManagedSessionRecord` (private struct, L1534–1537)
- `ProviderProcessState` (pub), `ProviderProcessStatus` (pub), `ProviderProcessRecord` (private, L1542–1622)
- `ExternalAgentProjectionAuthority` + full `impl` block (L1624–1894)

Add to `lib.rs`:
```rust
mod portal;
pub use self::portal::*;
```

`portal.rs` imports:
```rust
use std::process::{Child, Command, Stdio};
use crate::authority::ProjectionAuthority;          // from P-4 (see ordering note)
use crate::managed_session::{ManagedSessionHandle, ManagedSessionRecord, ...};
```

**Ordering note**: `ExternalAgentProjectionAuthority` wraps `ProjectionAuthority`,
which is not moved until P-4. For this step, `ProjectionAuthority` still lives
in `lib.rs`. The import path `use super::ProjectionAuthority` works during P-3
(before P-4) and changes to `use crate::authority::ProjectionAuthority` after P-4.
To avoid a two-step import fixup, execute P-3 and P-4 in a single co-ordinated
PR **or** reverse the order and do P-4 before P-3 (see note in §3.5).

**Visibility changes expected**: `ManagedSessionRecord` and `ProviderProcessRecord`
gain `pub(super)` if accessed from `ExternalAgentProjectionAuthority` (they are
both private structs used only within `portal.rs`).

**Step P-4: Authority internals + helpers**

Move to `authority.rs`:
- `ProjectionContractError` (L1896–1915)
- `ProjectionSession` (private struct, L1917–1962)
- `ProjectionAuditEvent<'a>` (private, L1964–1972)
- `ProjectionAuthority` pub struct + full `impl ProjectionAuthority` block (L1977–3354)
- `impl Default for ProjectionAuthority` (L3356–3359)
- All free helper functions (L3360–4020)
- The private `impl ProjectionResponse` block containing `with_portal_update_state` (see §3.3)

Add to `lib.rs`:
```rust
mod authority;
pub use self::authority::*;
```

`authority.rs` imports:
```rust
use crate::portal_cadence::PortalCadenceCoalescer;
use crate::contract::*;        // all wire types
use crate::managed_session::*; // session route types
```

This is the most import-sensitive step: `ProjectionAuthority` is imported by:
- `tze_hud_runtime/src/portal_projection_driver.rs` (via `tze_hud_projection::ProjectionAuthority`)
- `projection_authority` binary (via `tze_hud_projection::ProjectionAuthority`)
- `ExternalAgentProjectionAuthority` in `portal.rs`

The external import path `tze_hud_projection::ProjectionAuthority` is preserved
by `pub use self::authority::*;` in `lib.rs`. No downstream change needed.

**Merge and verify CI before proceeding to P-5.**

**Visibility changes expected**: `ProjectionSession` gains `pub(super)` so that
the `impl ProjectionResponse` block for `with_portal_update_state` (also in
`authority.rs`) can borrow it. All free helper functions become `pub(super)` if
called from any `impl` block also in `authority.rs` — since they are all in the
same file, standard `fn` visibility suffices (no change needed within the file).

**Step P-5: Tests**

Move to `tests/mod.rs`:
- Entire `mod tests { ... }` block (L4021–6891)

Create `crates/tze_hud_projection/src/tests/mod.rs` with the block contents.

Change in `lib.rs`:
```rust
#[cfg(test)]
mod tests;
```

The existing `use super::*;` inside the test module expands through `lib.rs`'s
full `pub use` chain — all types remain visible to tests without any import
changes.

**Step P-6 (optional): Further sub-splitting of `authority.rs`**

After P-1 through P-5, `authority.rs` will be ≈ 1,500–1,700 lines (excluding
helpers). If desired as a follow-on, split further:
- `authority/session.rs` ← `ProjectionSession` + `ProjectionAuditEvent<'a>`
- `authority/helpers.rs` ← all free helper functions
- `authority/mod.rs` ← `ProjectionContractError` + `ProjectionAuthority` impl

This is an optional follow-on, not part of the initial mechanical split.

### 3.5 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `ProjectionResponse::with_portal_update_state` straddles wire and authority concerns | High (it is the only cross-concern private method in the file) | Split the `impl ProjectionResponse` block: public methods stay in `contract.rs`; the private `with_portal_update_state` goes in an `impl ProjectionResponse` extension block in `authority.rs` (same module via `pub use`) — Rust allows split impl blocks within the same module. Document explicitly in the P-4 PR description. |
| P-3 and P-4 ordering dependency (`ExternalAgentProjectionAuthority` wraps `ProjectionAuthority`) | Medium | Option A: do P-4 before P-3 (reverse order). Option B: land P-3 and P-4 as a single PR. Option C: use `super::ProjectionAuthority` in P-3 and update to `crate::authority::ProjectionAuthority` in P-4. Option A (reverse) is cleanest — P-4 first, then P-3 imports `crate::authority::ProjectionAuthority`. Execution agents should use Option A. |
| `pub use self::contract::*` glob re-export may conflict if `authority.rs` also re-exports items with same names | Low | No name clashes exist across the four clusters. Verify with `cargo build --lib` after each step. |
| `std::process::{Child, Command, Stdio}` leaking into lib.rs scope | Medium | After P-3, `portal.rs` owns these imports. Ensure `lib.rs` does NOT include `use std::process::{...}` in its top-level imports after P-3 lands — remove that import from `lib.rs` as part of the P-3 PR. This is a platform-concern boundary: process supervision is isolated to `portal.rs`. |
| Integration test imports (`tests/projection_authority_cli.rs` uses `tze_hud_projection::DEFAULT_MAX_AUDIT_RECORDS`) | Low | Constants stay in `lib.rs` — no path change needed. Verify test crate builds after P-1. |
| Line numbers drift before execution | High (lib.rs receives frequent edits) | Use type and function names as anchors. Verify with `grep -n "struct ProjectionSession\|fn with_portal_update_state\|ExternalAgentProjectionAuthority"` before each PR. |
| `#[cfg(feature = "resident-grpc")]` gate on `pub mod resident_grpc` | Low | `resident_grpc.rs` is already a separate file and is untouched by this plan. The `use crate::portal_cadence::PortalCadenceCoalescer` import in `resident_grpc.rs` is unaffected. |
| `cargo fmt` may reorder items after moves | Low | Run `cargo fmt -- crates/tze_hud_projection/src/` after each PR; include fmt commit in the same PR if needed, but keep it as the last commit in the PR so the move diff is visible. |

---

## 4. Downstream Crate Inventory

These are the consumers of `tze_hud_projection` public items. All are API-
preserved by `pub use self::<module>::*` in `lib.rs` — no import path changes
required in any of these files after any split step.

| Consumer | File(s) | Imported public items |
|---|---|---|
| `tze_hud_runtime` | `src/portal_projection_driver.rs` | `AdapterGeometrySnapshot`, `AdapterPortalRect`, `AttachRequest`, `ContentClassification`, `OperationEnvelope`, `OutputKind`, `ProjectedPortalPolicy`, `ProjectionAuthority`, `ProjectionBounds`, `ProjectionOperation`, `ProviderKind`, `PublishOutputRequest` + `resident_grpc::*` |
| `tze_hud_runtime` | `src/portal_cadence.rs` | `pub use tze_hud_projection::portal_cadence::*` (re-export, unaffected) |
| `tze_hud_runtime` | `src/portal_tokens.rs` | `resident_grpc::PortalVisualTokens`, `resident_grpc::portal_visual_tokens_from_part_tokens` (unaffected, already a submodule) |
| `projection_authority` binary | `src/bin/projection_authority.rs` | `ProjectedPortalPolicy`, `AcknowledgeInputRequest`, `AdvisoryLeaseIdentity`, `AttachRequest`, `CleanupRequest`, `ContentClassification`, `DetachRequest`, `ExternalAgentProjectionAuthority`, `GetPendingInputRequest`, `HudConnectionMetadata`, `HudCredentialSource`, `ManagedSessionOrigin`, `ManagedSessionRequest`, `PresenceSurfaceRoute`, `ProjectionAttentionIntent`, `ProjectionAuditRecord`, `ProjectionAuthority`, `ProjectionBounds`, `ProjectionErrorCode`, `ProjectionOperation`, `ProjectionResponse`, `ProviderKind`, `PublishOutputRequest`, `PublishStatusRequest`, `WidgetParameterValue`, `WindowsHudTarget` + `resident_grpc::*` |
| Integration tests | `tests/projection_authority_cli.rs` | `DEFAULT_MAX_AUDIT_RECORDS` (constant, stays in `lib.rs`) |

No other crates in `mayor/rig/crates/` depend on `tze_hud_projection`
(`tze_hud_mcp`, `tze_hud_compositor`, `tze_hud_scene`, etc. have no dependency
entry in their `Cargo.toml`).

---

## 5. Execution Prerequisites (Before Any Split PR)

1. **Rebase** on `origin/main` before each PR — `lib.rs` receives frequent
   edits
2. **Verify type name anchors** with `grep -n "struct ProjectionSession\|fn
   with_portal_update_state\|ExternalAgentProjectionAuthority\|fn generate_owner_token"`
   immediately before starting each step — line numbers will have shifted
3. **Confirm no in-flight PRs touch `lib.rs`** — coordinate with any other
   workers that have open PRs against this file; rebase-order conflicts will
   require manual merge resolution
4. **No blocking logic-change beads for P-1/P-2**: the wire contract and managed
   session clusters have no pending dedup or refactor blockers at plan date. Verify
   with `bd show hud-luovo --json` for child beads status before each step.

---

## 6. Discovered Follow-Ups

These are separate tasks discovered during planning, not part of the mechanical
split:

| Bead candidate | Description |
|---|---|
| Extract `ProjectionResponse::with_portal_update_state` parameters | The private method currently takes `&ProjectionSession` — a coupling between wire and authority concerns. A clean follow-on would replace the `ProjectionSession` argument with its three constituent output values (bool, usize, portal_update_state) so `ProjectionResponse` has zero authority deps. This is a logic change, not a mechanical move. |
| `ExternalAgentProjectionAuthority` process supervision: platform isolation | The `ProviderProcessRecord` holding `std::process::Child` is Linux/macOS-only in practice (no Windows process management). A follow-on should add `#[cfg(unix)]` gating or abstract the process handle behind a trait. |
| `ProjectionAuthority` god-method dedup | Several `handle_*` methods in `ProjectionAuthority` share validation patterns (`validate_owner_token`, `validate_non_empty_bounded`). After the split, these could be extracted into a validation sub-trait or builder. This is a logic refactor, not mechanical. |
| Optional `authority.rs` sub-split (P-6) | After P-1–P-5, `authority.rs` will be ≈ 1,500–1,700 lines. If that remains a hotspot, split into `authority/session.rs` (ProjectionSession), `authority/helpers.rs` (free functions), and `authority/mod.rs` (ProjectionAuthority impl). |

---

## 7. Acceptance Criteria Checklist

Per hud-j8mb6:

- [x] Split plan with module boundaries and migration order written and reviewed (this document)
- [ ] `lib.rs` production lines ≤ 150 post-split (target ≈ 80–120 lines of mod declarations + constants + imports)
- [ ] All splits are mechanical move-only commits (verifiable by diff); each PR description lists items that gained `pub(super)` or `pub(crate)` visibility as part of the move
- [ ] External API preserved: `cargo test -p tze_hud_projection` and `cargo test -p tze_hud_runtime` pass after each step with no import-path changes in any downstream crate
- [ ] Test suite green after each step (no behavior change)
- [ ] `contract.rs` ≤ 1,300 lines, `managed_session.rs` ≤ 350 lines, `portal.rs` ≤ 400 lines, `authority.rs` ≤ 1,800 lines post-split
