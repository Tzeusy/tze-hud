# Text Stream Portals Reconciliation (gen-2)

Date: 2026-04-17
Issue: `hud-fomr`
Epic: `hud-t98e` (Phase-0 text stream portals raw-tile pilot)

## Inputs Audited

- `bd show hud-fomr --json`
- `docs/reconciliations/text_stream_portals_reconciliation_gen1_20260417.md`
- `about/legends-and-lore/rfcs/0013-text-stream-portals.md`
- `openspec/changes/text-stream-portals/specs/text-stream-portals/spec.md`
- `openspec/changes/text-stream-portals/specs/scene-graph/spec.md`
- `openspec/changes/text-stream-portals/specs/input-model/spec.md`
- `openspec/changes/text-stream-portals/specs/system-shell/spec.md`
- `tests/integration/text_stream_portal_adapter.rs`
- `tests/integration/text_stream_portal_surface.rs`
- `tests/integration/text_stream_portal_coalescing.rs`
- `tests/integration/text_stream_portal_governance.rs`
- `crates/tze_hud_input/src/lib.rs`
- `crates/tze_hud_scene/src/types.rs`
- `crates/tze_hud_scene/src/graph.rs`
- `crates/tze_hud_protocol/proto/session.proto`

## Requirement-to-Bead Coverage Matrix

| Requirement | Primary implementing bead(s) | Status | Evidence |
|---|---|---|---|
| `text-stream-portals` :: Transport-Agnostic Stream Boundary | `hud-t98e.3`, `hud-r11x` | Covered | E1 |
| `text-stream-portals` :: Content-Layer Portal Surface | `hud-t98e.2`, `hud-t98e.4` | Covered | E2 |
| `text-stream-portals` :: Phase-0 Raw-Tile Pilot | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.3` | Covered | E3 |
| `text-stream-portals` :: Bounded Transcript Viewport | `hud-t98e.2`, `hud-t98e.4` | Covered | E4 |
| `text-stream-portals` :: Low-Latency Text Interaction | `hud-t98e.3`, `hud-t98e.4`, `hud-r9u0` | Covered | E5 |
| `text-stream-portals` :: Transcript Interaction Contract | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.4` | Covered | E6 |
| `text-stream-portals` :: Coherent Transcript Coalescing | `hud-t98e.4` | Covered | E7 |
| `text-stream-portals` :: Governance, Privacy, and Override Compliance | `hud-t98e.2`, `hud-t98e.4` | Covered | E8 |
| `text-stream-portals` :: Ambient Portal Attention Defaults | `hud-t98e.4`, `hud-r9u0` | Covered | E9 |
| `text-stream-portals` :: External Adapter Isolation | `hud-t98e.3`, `hud-r11x` | Covered | E10 |
| `scene-graph` delta :: Text Stream Portal Phase-0 Uses Raw Tiles | `hud-t98e.2` | Covered | E11 |
| `input-model` delta :: Text Stream Portal Interaction Reuses Local-First Input | `hud-t98e.1`, `hud-t98e.4` | Covered | E12 |
| `system-shell` delta :: Text Stream Portals Remain Outside Chrome | `hud-t98e.2`, `hud-t98e.4` | Covered | E13 |

## Evidence Index

- **E1** Generic adapter bridge contract (`PortalAdapter`) stays tmux-agnostic and retains advisory `_wall_us`/`_mono_us` timing metadata: `tests/integration/text_stream_portal_adapter.rs:68`, `tests/integration/text_stream_portal_adapter.rs:82`, `tests/integration/text_stream_portal_adapter.rs:118`; tmux + non-tmux adapter contracts remain compatible and traffic stays on the primary `HudSession.Session` stream: `tests/integration/text_stream_portal_adapter.rs:568`, `tests/integration/text_stream_portal_adapter.rs:605`, `tests/integration/text_stream_portal_adapter.rs:738`, `crates/tze_hud_protocol/proto/session.proto:22`.
- **E2** Portal tile remains below runtime-managed zone bands and outside chrome-priority lease classes; shell diagnostics do not expose portal identity/transcript as chrome-owned state: `tests/integration/text_stream_portal_surface.rs:620`, `tests/integration/text_stream_portal_surface.rs:625`, `tests/integration/text_stream_portal_governance.rs:387`, `tests/integration/text_stream_portal_governance.rs:408`.
- **E3** Phase-0 portal surface uses only existing V1 node types (`SolidColor`, `TextMarkdown`, `StaticImage`, `HitRegion`), with resident session + lease flow and no portal-specific stream RPC: `tests/integration/text_stream_portal_surface.rs:697`, `tests/integration/text_stream_portal_adapter.rs:387`, `tests/integration/text_stream_portal_adapter.rs:422`, `crates/tze_hud_protocol/proto/session.proto:26`.
- **E4** Transcript viewport remains bounded by byte budget and line window, and retained visible-window semantics are covered by bounded-tail coalescing tests: `tests/integration/text_stream_portal_surface.rs:722`, `tests/integration/text_stream_portal_surface.rs:727`, `tests/integration/text_stream_portal_coalescing.rs:81`, `tests/integration/text_stream_portal_coalescing.rs:99`.
- **E5** Ordered incremental output ingest and transactional submit path remain intact; typing/activity updates stay transient with ephemeral-realtime timing semantics: `tests/integration/text_stream_portal_adapter.rs:580`, `tests/integration/text_stream_portal_adapter.rs:594`, `tests/integration/text_stream_portal_surface.rs:957`, `tests/integration/text_stream_portal_surface.rs:1052`.
- **E6** Runtime local-first scroll authority is defined in implementation (`InputProcessor::queue_set_scroll_offset`, `InputProcessor::process_scroll_event`, `InputProcessor::commit_scroll_updates`) and test-validated against reply/append behavior: `crates/tze_hud_input/src/lib.rs:286`, `crates/tze_hud_input/src/lib.rs:294`, `crates/tze_hud_input/src/lib.rs:331`, `tests/integration/text_stream_portal_surface.rs:459`, `tests/integration/text_stream_portal_surface.rs:545`.
- **E7** Coalescing keeps retained-window completeness and ordering across rapid appends, bounded tails, and intermediate-frame skips: `tests/integration/text_stream_portal_coalescing.rs:30`, `tests/integration/text_stream_portal_coalescing.rs:81`, `tests/integration/text_stream_portal_coalescing.rs:112`.
- **E8** Governance paths enforce lease expiry/revocation/orphan/frozen behavior, preserve geometry under redaction, and use generic backpressure signaling under safe-mode freeze: `tests/integration/text_stream_portal_governance.rs:141`, `tests/integration/text_stream_portal_governance.rs:225`, `tests/integration/text_stream_portal_governance.rs:276`, `tests/integration/text_stream_portal_governance.rs:349`.
- **E9** Attention defaults remain ambient: unread backlog coalesces without urgency escalation, typing stays ambient/non-interruptive, and timing stays ephemeral-realtime: `tests/integration/text_stream_portal_governance.rs:354`, `tests/integration/text_stream_portal_surface.rs:908`, `tests/integration/text_stream_portal_surface.rs:1050`.
- **E10** Adapter isolation remains enforced: authenticated `SessionInit` + capability flow before lease operations, tmux selector stays private to fixture adapter, and non-tmux path shares the same runtime contract/stream: `tests/integration/text_stream_portal_adapter.rs:384`, `tests/integration/text_stream_portal_adapter.rs:422`, `tests/integration/text_stream_portal_adapter.rs:162`, `tests/integration/text_stream_portal_adapter.rs:845`.
- **E11** Scene-graph delta remains raw-tile-only: pilot tile z-order stays in normal agent-owned band and node composition remains V1 primitives: `tests/integration/text_stream_portal_surface.rs:620`, `tests/integration/text_stream_portal_surface.rs:697`.
- **E12** Input-model delta reuses existing local-first plumbing from scene types through graph to input processor, with local reply acknowledgement/scroll authority verified in integration tests: `crates/tze_hud_scene/src/types.rs:405`, `crates/tze_hud_scene/src/graph.rs:292`, `crates/tze_hud_input/src/lib.rs:294`, `tests/integration/text_stream_portal_surface.rs:459`, `tests/integration/text_stream_portal_surface.rs:476`.
- **E13** System-shell delta keeps portals outside chrome ownership: diagnostic snapshot redacts identity/transcript while preserving shell stats, and shell dismiss revokes lease + removes tile via unconditional shell rules: `tests/integration/text_stream_portal_governance.rs:387`, `tests/integration/text_stream_portal_governance.rs:412`, `tests/integration/text_stream_portal_governance.rs:418`, `tests/integration/text_stream_portal_governance.rs:423`.

## GAP Closure Verification From gen-1

- **GAP-1 (non-tmux adapter conformance): CLOSED** by `hud-r11x` / PR `#437`.
  - Evidence: dedicated non-tmux contract and primary-stream integration tests (`tests/integration/text_stream_portal_adapter.rs:605`, `tests/integration/text_stream_portal_adapter.rs:749`).
- **GAP-2 (typing ambient + ephemeral semantics): CLOSED** by `hud-r9u0` / PR `#438`.
  - Evidence: ambient typing indicator styling/content assertions and ephemeral-realtime timing semantics (`tests/integration/text_stream_portal_surface.rs:908`, `tests/integration/text_stream_portal_surface.rs:1050`).

## Coverage Verdict

1. All 13 requirements across the 4 text-stream-portals specs are now covered with executable evidence.
2. No new specification gaps were found in this pass.
3. `hud-t98e` is **closeout-ready from a spec-to-code reconciliation perspective**.
4. `/opsx:sync` is now eligible if the coordinator chooses to finalize spec synchronization in the closeout flow.
