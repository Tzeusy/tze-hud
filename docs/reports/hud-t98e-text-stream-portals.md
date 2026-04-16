# Phase-0 Text Stream Portals Epic Report

Date: 2026-04-17
Report bead: `hud-t98e.6`
Epic: `hud-t98e` (Phase-0 text stream portals raw-tile pilot)

## Source of truth for this closeout

- `docs/reconciliations/text_stream_portals_direction_report_20260410.md`
- `docs/reconciliations/text_stream_portals_backlog_materialization_20260410.md`
- `docs/reconciliations/text_stream_portals_reconciliation_gen1_20260417.md`
- `docs/reconciliations/text_stream_portals_reconciliation_gen2_20260417.md`
- `about/legends-and-lore/rfcs/0013-text-stream-portals.md`
- `openspec/changes/text-stream-portals/specs/text-stream-portals/spec.md`
- `openspec/changes/text-stream-portals/specs/scene-graph/spec.md`
- `openspec/changes/text-stream-portals/specs/input-model/spec.md`
- `openspec/changes/text-stream-portals/specs/system-shell/spec.md`

## Closeout summary

The phase-0 epic delivered a governed, content-layer text stream portal pilot without introducing tmux/process-host semantics into runtime core ownership.

Delivery sequence:
1. `hud-t98e.1` landed the runtime-owned local-first scroll/input seam (PR `#427`).
2. `hud-t98e.2` landed resident raw-tile portal surface coverage using existing node primitives (PR `#429`).
3. `hud-t98e.3` landed transport-agnostic adapter integration evidence and tmux-proof path coverage (PR `#432`).
4. `hud-t98e.4` landed governance/coalescing/interaction validation suites (PR `#435`).
5. `hud-t98e.5` gen-1 reconciliation found two real gaps and spawned follow-up work.
6. `hud-r11x` (PR `#437`) and `hud-r9u0` (PR `#438`) closed those gaps.
7. `hud-fomr` gen-2 reconciliation (PR `#441`) confirmed full 13/13 requirement coverage.

## Raw-tile pilot shape

The pilot stays within the phase-0 shape defined by RFC 0013 and OpenSpec:
- Portal surfaces are assembled from existing scene node types (`SolidColor`, `TextMarkdown`, `StaticImage`, `HitRegion`) and standard tile layering.
- Transcript behavior is bounded (byte and window limits) and supports local-first scroll authority.
- No portal-specific terminal emulator or tmux-bound runtime subsystem was introduced.

Primary evidence:
- `tests/integration/text_stream_portal_surface.rs`
- `tests/integration/text_stream_portal_coalescing.rs`
- `crates/tze_hud_scene/src/types.rs`
- `crates/tze_hud_scene/src/graph.rs`
- `crates/tze_hud_input/src/lib.rs`

## Adapter boundary and tmux proof path

The adapter seam is transport-agnostic by contract and validated with both tmux and non-tmux fixtures.
- Traffic remains on the existing primary session stream.
- Adapter auth/capability flow reuses existing `SessionInit` boundaries.
- Runtime core does not gain tmux process/window/pane ownership.

Primary evidence:
- `tests/integration/text_stream_portal_adapter.rs`
- `crates/tze_hud_protocol/proto/session.proto`
- `docs/reconciliations/text_stream_portals_reconciliation_gen2_20260417.md` (E1, E10)

## Governance, privacy, and attention evidence

Governance and policy behavior was validated in integration suites:
- Lease expiry/revocation/orphan lifecycle handling
- Redaction while preserving geometry
- Safe-mode suspend/resume semantics
- Freeze behavior via generic backpressure signaling
- Shell isolation (no portal identity/transcript leakage into shell status)
- Ambient attention default behavior, including typing/activity semantics after gap closure

Primary evidence:
- `tests/integration/text_stream_portal_governance.rs`
- `tests/integration/text_stream_portal_surface.rs`
- `docs/reconciliations/text_stream_portals_reconciliation_gen2_20260417.md` (E8, E9, E13)

## Spec compliance matrix (final, gen-2)

| Spec file | Requirement | Implementing bead(s) | Status | Evidence |
|---|---|---|---|---|
| `text-stream-portals` | Transport-Agnostic Stream Boundary | `hud-t98e.3`, `hud-r11x` | Covered | `tests/integration/text_stream_portal_adapter.rs:568`, `:605`, `:738` |
| `text-stream-portals` | Content-Layer Portal Surface | `hud-t98e.2`, `hud-t98e.4` | Covered | `tests/integration/text_stream_portal_surface.rs:620`, `tests/integration/text_stream_portal_governance.rs:387` |
| `text-stream-portals` | Phase-0 Raw-Tile Pilot | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.3` | Covered | `tests/integration/text_stream_portal_surface.rs:697`, `crates/tze_hud_protocol/proto/session.proto:26` |
| `text-stream-portals` | Bounded Transcript Viewport | `hud-t98e.2`, `hud-t98e.4` | Covered | `tests/integration/text_stream_portal_surface.rs:722`, `tests/integration/text_stream_portal_coalescing.rs:81` |
| `text-stream-portals` | Low-Latency Text Interaction | `hud-t98e.3`, `hud-t98e.4`, `hud-r9u0` | Covered | `tests/integration/text_stream_portal_adapter.rs:580`, `tests/integration/text_stream_portal_surface.rs:957` |
| `text-stream-portals` | Transcript Interaction Contract | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.4` | Covered | `crates/tze_hud_input/src/lib.rs:294`, `tests/integration/text_stream_portal_surface.rs:459` |
| `text-stream-portals` | Coherent Transcript Coalescing | `hud-t98e.4` | Covered | `tests/integration/text_stream_portal_coalescing.rs:30`, `:112` |
| `text-stream-portals` | Governance, Privacy, and Override Compliance | `hud-t98e.2`, `hud-t98e.4` | Covered | `tests/integration/text_stream_portal_governance.rs:141`, `:225`, `:276`, `:349` |
| `text-stream-portals` | Ambient Portal Attention Defaults | `hud-t98e.4`, `hud-r9u0` | Covered | `tests/integration/text_stream_portal_governance.rs:354`, `tests/integration/text_stream_portal_surface.rs:908`, `:1050` |
| `text-stream-portals` | External Adapter Isolation | `hud-t98e.3`, `hud-r11x` | Covered | `tests/integration/text_stream_portal_adapter.rs:384`, `:422`, `:845` |
| `scene-graph` | Text Stream Portal Phase-0 Uses Raw Tiles | `hud-t98e.2` | Covered | `tests/integration/text_stream_portal_surface.rs:620`, `:697` |
| `input-model` | Text Stream Portal Interaction Reuses Local-First Input | `hud-t98e.1`, `hud-t98e.4` | Covered | `crates/tze_hud_scene/src/types.rs:405`, `crates/tze_hud_scene/src/graph.rs:292`, `crates/tze_hud_input/src/lib.rs:294` |
| `system-shell` | Text Stream Portals Remain Outside Chrome | `hud-t98e.2`, `hud-t98e.4` | Covered | `tests/integration/text_stream_portal_governance.rs:387`, `:418`, `:423` |

## Follow-up beads and gap closure

Gen-1 reconciliation identified two concrete gaps and created follow-up beads:
- `hud-r11x` (non-tmux adapter conformance) closed via PR `#437`.
- `hud-r9u0` (typing indicator ambient/ephemeral semantics) closed via PR `#438`.

Then `hud-fomr` executed gen-2 reconciliation and confirmed full coverage via PR `#441`.

No additional spec-coverage follow-up beads were required by gen-2.

## Residual risks and deferred seams

1. `/opsx:sync` was marked eligible by gen-2 reconciliation but is a coordinator closeout action, not executed in this bead.
2. Future adapter families (beyond tmux and the current non-tmux fixture) still require the same contract-level conformance evidence; treat adapter expansion as separate scoped work.
3. Epic closure bookkeeping (`bd update --append-notes` linking this report and epic close) is coordinator-owned and remains pending outside this worker bead.

## Artifact index

- Direction/backlog: `docs/reconciliations/text_stream_portals_direction_report_20260410.md`, `docs/reconciliations/text_stream_portals_backlog_materialization_20260410.md`
- Reconciliation: `docs/reconciliations/text_stream_portals_reconciliation_gen1_20260417.md`, `docs/reconciliations/text_stream_portals_reconciliation_gen2_20260417.md`
- Validation evidence: `docs/evidence/text-stream-portals/validation-2026-04-16.md`
- Integration tests: `tests/integration/text_stream_portal_surface.rs`, `tests/integration/text_stream_portal_adapter.rs`, `tests/integration/text_stream_portal_coalescing.rs`, `tests/integration/text_stream_portal_governance.rs`
