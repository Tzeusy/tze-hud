# Text Stream Portals Reconciliation (gen-1)

Date: 2026-04-17
Issue: `hud-t98e.5`
Epic: `hud-t98e` (Phase-0 text stream portals raw-tile pilot)

## Inputs Audited

- `bd show hud-t98e.5 --json`
- `about/legends-and-lore/rfcs/0013-text-stream-portals.md`
- `openspec/changes/text-stream-portals/specs/text-stream-portals/spec.md`
- `openspec/changes/text-stream-portals/specs/scene-graph/spec.md`
- `openspec/changes/text-stream-portals/specs/input-model/spec.md`
- `openspec/changes/text-stream-portals/specs/system-shell/spec.md`
- `docs/reconciliations/text_stream_portals_direction_report_20260410.md`
- `tests/integration/text_stream_portal_surface.rs`
- `tests/integration/text_stream_portal_adapter.rs`
- `tests/integration/text_stream_portal_coalescing.rs`
- `tests/integration/text_stream_portal_governance.rs`
- `crates/tze_hud_scene/src/types.rs`
- `crates/tze_hud_scene/src/graph.rs`
- `crates/tze_hud_input/src/lib.rs`
- `crates/tze_hud_protocol/proto/session.proto`

## Requirement-to-Bead Coverage Matrix

| Requirement | Primary implementing bead(s) | Status | Evidence |
|---|---|---|---|
| `text-stream-portals` :: Transport-Agnostic Stream Boundary | `hud-t98e.3` | **Partially covered (GAP-1)** | Generic bridge trait + tmux-shaped adapter fixture: `tests/integration/text_stream_portal_adapter.rs:68`, `:146`, `:440`; single primary stream assertion: `tests/integration/text_stream_portal_adapter.rs:589`; clock suffix fields in adapter metadata (`*_wall_us`, `*_mono_us`): `tests/integration/text_stream_portal_adapter.rs:57`, `:58`, `:64`, `:65`; no concrete non-tmux adapter fixture landed. |
| `text-stream-portals` :: Content-Layer Portal Surface | `hud-t98e.2`, `hud-t98e.4` | Covered | Portal tile stays below runtime-managed band and off chrome priority path: `tests/integration/text_stream_portal_surface.rs:572`; shell snapshot omits portal identity/transcript: `tests/integration/text_stream_portal_governance.rs:387`. |
| `text-stream-portals` :: Phase-0 Raw-Tile Pilot | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.3` | Covered | Surface composed with existing node types only: `tests/integration/text_stream_portal_surface.rs:648`; resident session+lease flow: `tests/integration/text_stream_portal_adapter.rs:247`; no portal-specific stream RPC family added in session oneof fields (client ends at resource upload fields 36-38): `crates/tze_hud_protocol/proto/session.proto:35`. |
| `text-stream-portals` :: Bounded Transcript Viewport | `hud-t98e.2`, `hud-t98e.4` | Covered | Bounded markdown generation and byte budget (`MAX_MARKDOWN_BYTES`): `tests/integration/text_stream_portal_surface.rs:658`, `:671`; tile node-budget assertion: `tests/integration/text_stream_portal_surface.rs:751`. |
| `text-stream-portals` :: Low-Latency Text Interaction | `hud-t98e.3`, `hud-t98e.4` | **Partially covered (GAP-2)** | Incremental output streaming assertion: `tests/integration/text_stream_portal_adapter.rs:523`, `:580`; viewer input treated transactionally in bridge: `tests/integration/text_stream_portal_adapter.rs:129`, `:459`; local submit acknowledgement: `tests/integration/text_stream_portal_surface.rs:454`; explicit typing-indicator traffic-class validation is not present. |
| `text-stream-portals` :: Transcript Interaction Contract | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.4` | Covered | Runtime-owned scroll path: `crates/tze_hud_input/src/lib.rs:294`; user-scroll-authority tests in input + portal surface suites: `crates/tze_hud_input/src/lib.rs:1186`, `tests/integration/text_stream_portal_surface.rs:471`; local feedback for controls/reply: `tests/integration/text_stream_portal_surface.rs:392`, `:454`. |
| `text-stream-portals` :: Coherent Transcript Coalescing | `hud-t98e.4` | Covered | Coalesced snapshots retain complete window and ordered bounded tail: `tests/integration/text_stream_portal_coalescing.rs:30`, `:81`, `:112`. |
| `text-stream-portals` :: Governance, Privacy, and Override Compliance | `hud-t98e.2`, `hud-t98e.4` | Covered | Lease expiry/revocation/orphan lifecycle: `tests/integration/text_stream_portal_governance.rs:140`, `:161`, `:177`; redaction geometry and content suppression: `tests/integration/text_stream_portal_governance.rs:225`; safe-mode suspend/resume: `tests/integration/text_stream_portal_governance.rs:276`; freeze path uses generic backpressure signal: `tests/integration/text_stream_portal_governance.rs:314`. |
| `text-stream-portals` :: Ambient Portal Attention Defaults | `hud-t98e.4` | **Partially covered (GAP-2)** | Unread backlog remains low/ambient and coalesces without escalation: `tests/integration/text_stream_portal_governance.rs:354`; typing-indicator-specific ambient semantics remain untested. |
| `text-stream-portals` :: External Adapter Isolation | `hud-t98e.3` | Covered | Adapter authenticates via `SessionInit` with PSK + capability grants: `tests/integration/text_stream_portal_adapter.rs:256`; runtime remains free of tmux-specific process hosting codepaths (tmux references confined to integration test fixture): `tests/integration/text_stream_portal_adapter.rs:1`. |
| `scene-graph` delta :: Text Stream Portal Phase-0 Uses Raw Tiles | `hud-t98e.2` | Covered | Raw tile path with normal z-order band: `tests/integration/text_stream_portal_surface.rs:559`, `:572`. |
| `input-model` delta :: Text Stream Portal Interaction Reuses Local-First Input | `hud-t98e.1`, `hud-t98e.4` | Covered | Scroll config + local offset plumbing: `crates/tze_hud_scene/src/types.rs:405`, `crates/tze_hud_scene/src/graph.rs:292`, `crates/tze_hud_input/src/lib.rs:294`; immediate local reply acknowledgement: `tests/integration/text_stream_portal_surface.rs:454`. |
| `system-shell` delta :: Text Stream Portals Remain Outside Chrome | `hud-t98e.2`, `hud-t98e.4` | Covered | Shell diagnostics omit portal identity/transcript while preserving aggregate stats: `tests/integration/text_stream_portal_governance.rs:387`; shell dismiss override removes tile via lease revocation: `tests/integration/text_stream_portal_governance.rs:418`. |

## Gaps Requiring Follow-On Beads

- **GAP-1:** Transport-agnostic boundary lacks a concrete non-tmux adapter fixture/test to satisfy the `non-tmux adapter satisfies contract` scenario with executable evidence.
- **GAP-2:** No explicit typing/activity-indicator test verifies ambient default behavior *and* ephemeral realtime traffic-class semantics.

## Coverage Verdict

1. 11 of 13 requirements are covered with landed code/tests.
2. 2 requirements are partially covered (GAP-1, GAP-2).
3. Because gaps remain, this gen-1 reconciliation should stay open until follow-on beads land and a gen-2 pass is completed.
4. `/opsx:sync` is **not** applicable yet because full coverage is not achieved.

## Coordinator Follow-On Proposals

The worker cannot mutate bead state. Materialize the following as new child beads under epic `hud-t98e`:

1. `title`: `Add non-tmux adapter conformance coverage for text stream portal bridge`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-t98e.5`
   `rationale`: `Close GAP-1 by adding at least one concrete non-tmux adapter fixture/integration test proving the same generic portal contract works unchanged.`

2. `title`: `Validate typing indicator ambient/ephemeral semantics for text stream portals`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-t98e.5`
   `rationale`: `Close GAP-2 by adding explicit tests that typing/activity indicators stay ambient and use ephemeral realtime behavior instead of transactional or urgency-escalating paths.`

3. `title`: `Reconcile spec-to-code (gen-2) for phase-0 text stream portals`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-t98e.5`
   `rationale`: `Required follow-up reconciliation pass after GAP-1 and GAP-2 beads land; verifies all 13 requirements are fully covered and determines closeout readiness.`
