# Cooperative HUD Projection Reconciliation (gen-1)

Date: 2026-04-29
Issue: `hud-ggntn.7`
Epic: `hud-ggntn`
OpenSpec change: `openspec/changes/cooperative-hud-projection`

## Inputs Audited

- `bd show hud-ggntn.7 --json`
- `openspec/changes/cooperative-hud-projection/proposal.md`
- `openspec/changes/cooperative-hud-projection/design.md`
- `openspec/changes/cooperative-hud-projection/specs/cooperative-hud-projection/spec.md`
- `openspec/changes/cooperative-hud-projection/specs/text-stream-portals/spec.md`
- `openspec/changes/cooperative-hud-projection/tasks.md`
- `openspec/changes/cooperative-hud-projection/reconciliation.md`
- `docs/evidence/cooperative-hud-projection/validation-2026-04-29.md`
- `docs/reports/cooperative_hud_projection_hud-ggntn_closeout_20260429.md`
- `crates/tze_hud_projection/src/lib.rs`
- `tests/integration/text_stream_portal_adapter.rs`
- `.claude/skills/hud-projection/`, `.gemini/skills/hud-projection/`, and `.opencode/skills/hud-projection/`

## Implementation Inventory

| Bead | PR / commit | Scope |
|---|---|---|
| `hud-ggntn.1` | PR #617, `8dc99cc` | Introduced `crates/tze_hud_projection`, operation envelopes, request/response schema, stable error codes, owner tokens, and initial unit tests. |
| `hud-ggntn.2` | PR #619, `e0dc3e7` | Added in-memory projection authority semantics for retained state, inbox transitions, reconnect bookkeeping, stale lease denial, cleanup, bounds, and audit records. |
| `hud-ggntn.3` | PR #620, `40a2e27` | Added projected portal state materialization and headless text-stream portal adapter integration coverage. |
| `hud-ggntn.4` | PR #618, `c80def5` | Added `/hud-projection` skill packaging and mirrored `.claude`, `.gemini`, and `.opencode` skill surfaces. |
| `hud-ggntn.5` | PR #622, `4794b32` | Added local/unit and headless integration validation evidence. |
| `hud-ggntn.6` | PR #623, `c82ad09` | Added the closeout report and residual-risk register. |

PR #621 / `92926d2` was explicitly excluded from evidence because it is unrelated Android media work.

## Requirement-to-Bead Coverage Matrix

| Requirement | Primary bead(s) | Status | Evidence |
|---|---|---|---|
| `cooperative-hud-projection` :: Cooperative Attachment Contract | `hud-ggntn.1`, `hud-ggntn.4` | Covered | Provider-neutral attach schema and no-PTY boundary: `crates/tze_hud_projection/src/lib.rs:118`, `:131`, `:1367`; skill trigger and no terminal-capture instruction: `.claude/skills/hud-projection/SKILL.md:3`, `:20`; attach examples for Codex, Claude, and opencode: `.claude/skills/hud-projection/references/operation-examples.md:9`. |
| `cooperative-hud-projection` :: External Projection State Authority | `hud-ggntn.2`, `hud-ggntn.5` | Covered for in-memory authority semantics | `ProjectionAuthority` owns memory-only session state outside runtime core: `crates/tze_hud_projection/src/lib.rs:1042`; state summary excludes private text/token fields: `:1096`; restart behavior is represented by a fresh authority instance in tests: `:3263`; detach/cleanup purge session state: `:1923`, `:1975`. This is not a runnable daemon process; see GAP-1. |
| `cooperative-hud-projection` :: Low-Token LLM-Facing Operations | `hud-ggntn.1`, `hud-ggntn.4` | Partially covered (GAP-1) | Seven operations are modeled as Rust schema and handlers: `crates/tze_hud_projection/src/lib.rs:118`, `:1367`, `:1510`, `:1613`, `:1784`, `:1877`, `:1923`, `:1975`; operation examples and MCP facade notes exist in skill docs: `.claude/skills/hud-projection/references/operation-examples.md:3`, `.claude/skills/hud-projection/references/mcp-facade.md:1`. No executable projection daemon CLI, local IPC server, or MCP facade exists in the repo, so an already-running LLM session cannot yet call these operations through the documented skill without out-of-tree infrastructure. |
| `cooperative-hud-projection` :: Projection Operation Authorization | `hud-ggntn.1`, `hud-ggntn.2`, `hud-ggntn.5` | Covered | Owner tokens are 256-bit random hex, stored as verifiers: `crates/tze_hud_projection/src/lib.rs:40`, `:1437`, `:1449`; owner authorization uses verifier comparison and expiry: `:2072`; operator cleanup uses separate authority: `:1063`, `:2018`; audit records omit transcript/input/token text: `:854`. |
| `cooperative-hud-projection` :: HUD Input Inbox Delivery | `hud-ggntn.2`, `hud-ggntn.3`, `hud-ggntn.5` | Covered for authority and portal-state semantics | Inbox item states are modeled: `crates/tze_hud_projection/src/lib.rs:187`; portal composer submission creates bounded pending items with local feedback: `:1691`; polling transitions pending/deferred items to delivered and bounds results: `:1784`; acknowledgement handles handled, rejected, deferred, idempotent replay, conflicts, and expiry: `:2484`, `:2551`, `:2644`. |
| `cooperative-hud-projection` :: Projected Portal Lifecycle | `hud-ggntn.3`, `hud-ggntn.5` | Partially covered (GAP-2) | Projected portal state exposes the cooperative adapter family, resident-session lease authority, content layer, expanded/collapsed presentation, ambient attention, and policy-shaped identity/input visibility: `crates/tze_hud_projection/src/lib.rs:2173`; collapse/expand state methods exist: `:1143`, `:1156`; headless integration verifies provider-neutral state and no process authority: `tests/integration/text_stream_portal_adapter.rs:651`, `:762`. There is no resident gRPC bridge or live Windows user-test proving attach creates/reuses a visible content-layer text-stream portal, accepts HUD composer input, collapse/restores, drags/repositions, or detaches with visible cleanup. |
| `cooperative-hud-projection` :: Provider-Neutral Projection Identity | `hud-ggntn.1`, `hud-ggntn.3`, `hud-ggntn.4` | Covered | Provider kind is a neutral enum with `codex`, `claude`, `opencode`, and `other`: `crates/tze_hud_projection/src/lib.rs:131`; integration loops all provider kinds without changing runtime-facing authority: `tests/integration/text_stream_portal_adapter.rs:762`; mirrored skill examples use the same schema for Codex, Claude, and opencode. |
| `cooperative-hud-projection` :: Privacy and Attention Governance | `hud-ggntn.2`, `hud-ggntn.3`, `hud-ggntn.5` | Partially covered (GAP-3) | Missing classification defaults to private: `crates/tze_hud_projection/src/lib.rs:155`; portal state preserves geometry, redacts identity/transcript/input details, disables interactions under policy, safe mode, freeze, or dismiss, and keeps attention ambient: `:2173`; local/headless validation is recorded in `docs/evidence/cooperative-hud-projection/validation-2026-04-29.md`. Live overlay evidence for redaction, safe mode, freeze, dismiss, orphan cleanup, and backlog non-escalation is absent. |
| `cooperative-hud-projection` :: Bounded Backpressure and Expiry | `hud-ggntn.1`, `hud-ggntn.2`, `hud-ggntn.5` | Covered for local/headless semantics | Default bounds match the OpenSpec values: `crates/tze_hud_projection/src/lib.rs:16`; output rejection, retained transcript pruning, and rate coalescing are implemented: `:1510`, `:2298`, `:2376`, `:2396`; pending input count/byte bounds and expiry are implemented: `:1747`, `:2644`; property/state-machine validation is recorded in `docs/evidence/cooperative-hud-projection/validation-2026-04-29.md`. |
| `text-stream-portals` delta :: Cooperative LLM Projection Adapter | `hud-ggntn.3`, `hud-ggntn.5` | Partially covered (GAP-2) | The adapter family and runtime authority stay provider-neutral and process-agnostic: `crates/tze_hud_projection/src/lib.rs:213`, `:221`; headless integration asserts serialized state does not expose PTY, tmux, terminal, stdin/stdout, process lifecycle, or provider RPC authority: `tests/integration/text_stream_portal_adapter.rs:651`, `:762`. No concrete resident gRPC projection adapter drives a visible portal. |
| `text-stream-portals` delta :: Cooperative Projection Input Mapping | `hud-ggntn.2`, `hud-ggntn.3`, `hud-ggntn.5` | Covered for authority semantics | Portal submissions map to bounded semantic inbox items: `crates/tze_hud_projection/src/lib.rs:1691`; headless integration submits input and polls it through the cooperative contract rather than terminal input: `tests/integration/text_stream_portal_adapter.rs:697`, `:731`. |
| `text-stream-portals` delta :: Cooperative Projection State Externality | `hud-ggntn.2`, `hud-ggntn.3` | Covered | Retained transcript, pending input, acknowledgement, reconnect, and lease bookkeeping live in `ProjectionAuthority`, while `projected_portal_state` returns only bounded visible transcript, compact state, and policy-permitted status metadata: `crates/tze_hud_projection/src/lib.rs:996`, `:1096`, `:1130`, `:2173`. |

## Scenario Checklist

| Scenario | Status | Evidence / gap |
|---|---|---|
| Already-running session opts in | Covered | `handle_attach` registers a logical projection and returns owner-bound state: `crates/tze_hud_projection/src/lib.rs:1367`. |
| Arbitrary terminal capture is out of scope | Covered | Skill docs and integration assertions reject PTY/terminal/process authority: `.claude/skills/hud-projection/SKILL.md:20`, `tests/integration/text_stream_portal_adapter.rs:753`. |
| Transcript retained outside token context | Covered | Authority retains bounded transcript and returns summaries/windows: `crates/tze_hud_projection/src/lib.rs:1096`, `:1124`, `:2298`. |
| Reconnect preserves projection state | Covered | Reconnect metadata and tests preserve transcript/inbox while dropping stale leases: `crates/tze_hud_projection/src/lib.rs:1169`, `:3510`. |
| Daemon restart purges private projection state | Covered for in-memory authority | Fresh `ProjectionAuthority` has no prior state and requires reattach; no persistent store exists. |
| Stale lease identity cannot authorize republish | Covered | Lease record and republish authorization require live HUD connection, unexpired lease, and requested capabilities within grants: `crates/tze_hud_projection/src/lib.rs:1245`, `:1269`. |
| Agent publishes output without transcript drain | Covered | `handle_publish_output` appends/coalesces and returns compact metadata: `crates/tze_hud_projection/src/lib.rs:1510`. |
| Pending input check is compact | Covered | `handle_get_pending_input` bounds item count and bytes with remaining counts: `crates/tze_hud_projection/src/lib.rs:1784`. |
| Attach conflict is deterministic | Covered | Conflicting attach returns `PROJECTION_ALREADY_ATTACHED` without token disclosure: `crates/tze_hud_projection/src/lib.rs:1396`. |
| Operation audit record is structured | Covered | Audit record shape excludes transcript/input/token data: `crates/tze_hud_projection/src/lib.rs:854`. |
| Cross-projection input read is denied | Covered | Owner authorization denies wrong-token reads; tests cover cross-projection denial. |
| Unauthorized cleanup is denied | Covered | Owner and operator cleanup paths require distinct credentials: `crates/tze_hud_projection/src/lib.rs:1975`. |
| Operator cleanup uses separate authority | Covered | `set_operator_authority` and operator cleanup category are distinct: `crates/tze_hud_projection/src/lib.rs:1063`, `:2018`. |
| Submitted HUD input becomes pending item | Covered for authority semantics | `submit_portal_input` enqueues pending inbox items: `crates/tze_hud_projection/src/lib.rs:1691`. |
| Submitted HUD input gets local pending feedback | Covered for authority semantics | `PortalInputFeedback` returns accepted/rejected state immediately: `crates/tze_hud_projection/src/lib.rs:636`, `:1691`. |
| Acknowledgement updates visible state | Covered for authority semantics | Handled/rejected acknowledgement updates state and pending counts: `crates/tze_hud_projection/src/lib.rs:2551`. |
| Deferred input is redelivered after not-before time | Covered | Deferred transition validates `not_before_wall_us` and polling hides until due: `crates/tze_hud_projection/src/lib.rs:2578`. |
| Conflicting terminal acknowledgement is rejected | Covered | Terminal replay accepts matching state and rejects conflicting state: `crates/tze_hud_projection/src/lib.rs:2484`. |
| Input is not terminal keystroke passthrough | Covered | Headless adapter state has no PTY/terminal/stdin authority: `tests/integration/text_stream_portal_adapter.rs:753`. |
| Attach creates projected portal | Partial (GAP-2) | Headless `projected_portal_state` exists, but no resident gRPC/live visible portal creation evidence exists. |
| Collapse preserves session affordance | Partial (GAP-2) | Collapse state exists in memory, but live compact card/icon behavior is not exercised. |
| Detach cleans up portal | Partial (GAP-2) | Detach purges authority state, but live tile/lease cleanup is not exercised. |
| Codex and Claude use same contract | Covered | Provider-neutral test covers Codex, Claude, opencode, and other: `tests/integration/text_stream_portal_adapter.rs:762`. |
| Unknown provider still projects | Covered | `ProviderKind::Other` is included in the same provider-neutral loop. |
| Collapsed projection redacts private identity | Partial (GAP-3) | Policy-shaped state redacts fields, but live collapsed redaction behavior has no overlay evidence. |
| Missing classification fails closed | Covered | `ContentClassification::Private` is the default: `crates/tze_hud_projection/src/lib.rs:155`. |
| Unread backlog remains ambient | Partial (GAP-3) | `ProjectedPortalAttention::Ambient` is fixed in state, but live backlog behavior is not validated. |
| Oversized output is bounded | Covered | Publish validation rejects oversized output with stable error code: `crates/tze_hud_projection/src/lib.rs:414`, `:1510`. |
| Pending input queue reaches limit | Covered | Input queue bounds reject new submissions with visible local feedback: `crates/tze_hud_projection/src/lib.rs:1747`. |
| Retained transcript overflow prunes oldest non-visible units | Covered | Retention pruning preserves the visible tail window: `crates/tze_hud_projection/src/lib.rs:2396`. |
| Cooperative adapter satisfies portal boundary | Partial (GAP-2) | Headless state satisfies provider-neutral/process-agnostic boundary; no concrete resident adapter drives the HUD. |
| Cooperative adapter does not imply process hosting | Covered | Runtime-facing state omits process authority in integration assertions: `tests/integration/text_stream_portal_adapter.rs:811`. |
| Submitted text maps to semantic inbox | Covered | Headless integration maps HUD input through `submit_portal_input` then `get_pending_input`: `tests/integration/text_stream_portal_adapter.rs:697`. |
| Raw keystroke passthrough remains out of scope | Covered | No terminal keystroke channel is exposed by projection state or skills. |
| Full projection state remains external | Covered | Full transcript/inbox/reconnect state remains in `ProjectionAuthority`, not scene graph. |
| Pending input not mirrored into scene graph | Covered for authority state | Portal state exposes counts/feedback only, not full queue: `crates/tze_hud_projection/src/lib.rs:2173`. |

## Gaps Requiring Follow-On Beads

**GAP-1: Runnable LLM-facing projection authority surface is missing.**

The Rust crate defines the operation contract and an in-memory authority, and the skill documents how agents should use an external projection-daemon MCP server, CLI, or local IPC surface. The repo does not yet contain that executable surface. This leaves `/hud-projection` packaging dependent on out-of-tree infrastructure.

**GAP-2: Resident gRPC and visible text-stream portal lifecycle are not implemented or validated for cooperative projection.**

`projected_portal_state` provides a headless materialization boundary, but no concrete adapter drives the existing `HudSession` raw-tile path for cooperative projection. The spec scenarios for attach-created portal, collapse/restore affordance, HUD composer input, drag/reposition, lease release, and visible detach cleanup are therefore only partially covered.

**GAP-3: Live privacy/governance validation is absent.**

Local policy shaping covers redaction, safe mode, freeze, dismiss, and ambient attention in state, but no Windows overlay evidence proves those paths with actual projected-session UI.

## Coverage Verdict

1. The operation schema, in-memory authority semantics, authorization model, inbox state machine, provider-neutral identity, and bounded backpressure behavior are covered by landed code and local/headless tests.
2. The implementation does not yet satisfy the full cooperative workflow as a usable already-running LLM projection path because the repo lacks an executable projection daemon surface and a concrete resident gRPC adapter that creates a visible portal.
3. The OpenSpec change is not ready for sync/archive. Sync should wait until GAP-1 through GAP-3 are closed or explicitly waived in a follow-up reconciliation.
4. A gen-2 reconciliation bead is appropriate after the gap beads land. No gen-4 reconciliation is proposed.

## Coordinator Follow-On Proposals

The worker did not mutate Beads lifecycle state. Materialize the following as new child beads under epic `hud-ggntn`:

1. `title`: `Implement executable projection daemon control surface`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ggntn.7`
   `description`: `Close GAP-1 by adding an in-repo external projection authority surface that already-running LLM sessions can call through at least one supported path: daemon-local CLI, OS-protected local IPC, or external projection-daemon MCP. It must delegate to the normative tze_hud_projection operation contract, preserve owner-token rules, emit bounded responses/audit records, and remain separate from runtime v1 MCP.`

2. `title`: `Wire cooperative projection daemon to resident text-stream portal`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ggntn.7`
   `description`: `Close GAP-2 by adding the concrete resident gRPC adapter that turns cooperative projection attach/output/status/input state into an existing text-stream portal raw-tile surface. Evidence must cover attach creating or reusing a content-layer portal, HUD composer submission into the semantic inbox, collapse/restore, drag/reposition or movable compact affordance, detach/cleanup lease release, and no PTY/tmux/process lifecycle authority.`

3. `title`: `Run live Windows governance validation for cooperative HUD projection`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ggntn.7`
   `description`: `Close GAP-3 by recording visible Windows HUD evidence for attach -> publish output -> submit HUD input -> poll/acknowledge -> collapse/restore -> detach cleanup, plus redaction, safe mode, freeze, dismiss, orphan cleanup, and backlog non-escalation behavior. Store the artifact under docs/evidence/cooperative-hud-projection/.`

4. `title`: `Reconcile spec-to-code (gen-2) for cooperative HUD projection`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `the three GAP beads proposed by hud-ggntn.7`
   `description`: `Run the gen-2 reconciliation after GAP-1 through GAP-3 are implemented or explicitly waived. Verify every cooperative-hud-projection and text-stream-portals delta requirement and scenario against code, tests, live evidence, and reports; then decide whether OpenSpec sync/archive is ready.`
