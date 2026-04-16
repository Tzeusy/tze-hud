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
| `text-stream-portals` :: Transport-Agnostic Stream Boundary | `hud-t98e.3`, `hud-r11x` | Covered | Generic adapter bridge contract (`PortalAdapter`) carries text stream/input/session primitives without tmux/PTY coupling: `tests/integration/text_stream_portal_adapter.rs:82`; timing metadata follows `_wall_us`/`_mono_us` naming and stays advisory: `tests/integration/text_stream_portal_adapter.rs:68`, `tests/integration/text_stream_portal_adapter.rs:71`, `tests/integration/text_stream_portal_adapter.rs:72`, `tests/integration/text_stream_portal_adapter.rs:118`; tmux and non-tmux adapter contract tests both pass against unchanged bridge shape: `tests/integration/text_stream_portal_adapter.rs:568`, `tests/integration/text_stream_portal_adapter.rs:605`; portal traffic stays on one primary resident stream: `tests/integration/text_stream_portal_adapter.rs:738`, `tests/integration/text_stream_portal_adapter.rs:845`, `crates/tze_hud_protocol/proto/session.proto:22`. |
| `text-stream-portals` :: Content-Layer Portal Surface | `hud-t98e.2`, `hud-t98e.4` | Covered | Portal tile is asserted below runtime-managed zone band and not chrome-priority lease class: `tests/integration/text_stream_portal_surface.rs:620`, `tests/integration/text_stream_portal_surface.rs:625`; shell diagnostics must not expose portal identity/transcript as chrome-owned state: `tests/integration/text_stream_portal_governance.rs:387`, `tests/integration/text_stream_portal_governance.rs:408`. |
| `text-stream-portals` :: Phase-0 Raw-Tile Pilot | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.3` | Covered | Expanded portal surface materializes only existing V1 node types (`SolidColor`, `TextMarkdown`, `StaticImage`, `HitRegion`): `tests/integration/text_stream_portal_surface.rs:697`; resident session + lease flow drives pilot ownership: `tests/integration/text_stream_portal_adapter.rs:387`, `tests/integration/text_stream_portal_adapter.rs:422`; no portal-specific secondary stream RPC introduced (single `HudSession.Session` stream): `crates/tze_hud_protocol/proto/session.proto:26`. |
| `text-stream-portals` :: Bounded Transcript Viewport | `hud-t98e.2`, `hud-t98e.4` | Covered | Transcript markdown is clamped to byte budget and viewport line window: `tests/integration/text_stream_portal_surface.rs:722`, `tests/integration/text_stream_portal_surface.rs:727`; retained visible-window semantics are validated via bounded-tail coalescing checks: `tests/integration/text_stream_portal_coalescing.rs:81`, `tests/integration/text_stream_portal_coalescing.rs:99`. |
| `text-stream-portals` :: Low-Latency Text Interaction | `hud-t98e.3`, `hud-t98e.4`, `hud-r9u0` | Covered | Incremental output ingest remains ordered stream behavior: `tests/integration/text_stream_portal_adapter.rs:580`, `tests/integration/text_stream_portal_adapter.rs:620`; viewer submit path remains transactional bridge input: `tests/integration/text_stream_portal_adapter.rs:594`, `tests/integration/text_stream_portal_adapter.rs:634`; typing/activity indicators are validated as transient in-place updates plus ephemeral-realtime timing class semantics: `tests/integration/text_stream_portal_surface.rs:957`, `tests/integration/text_stream_portal_surface.rs:1039`, `tests/integration/text_stream_portal_surface.rs:1052`. |
| `text-stream-portals` :: Transcript Interaction Contract | `hud-t98e.1`, `hud-t98e.2`, `hud-t98e.4` | Covered | Runtime-owned local-first scroll path is explicit in input processor and preserves user authority over queued adapter offsets: `crates/tze_hud_input/src/lib.rs:294`, `crates/tze_hud_input/src/lib.rs:1186`; portal reply affordance clears local draft before adapter roundtrip: `tests/integration/text_stream_portal_surface.rs:459`; scroll offset remains preserved through append updates: `tests/integration/text_stream_portal_surface.rs:476`, `tests/integration/text_stream_portal_surface.rs:545`. |
| `text-stream-portals` :: Coherent Transcript Coalescing | `hud-t98e.4` | Covered | Rapid append coalescing preserves full retained window/order: `tests/integration/text_stream_portal_coalescing.rs:30`; bounded-tail coalescing keeps latest ordered retained window: `tests/integration/text_stream_portal_coalescing.rs:81`; intermediate-frame skipping still converges to complete final state: `tests/integration/text_stream_portal_coalescing.rs:112`. |
| `text-stream-portals` :: Governance, Privacy, and Override Compliance | `hud-t98e.2`, `hud-t98e.4` | Covered | Lease expiry/revocation/orphan lifecycle and frozen-state behavior are enforced: `tests/integration/text_stream_portal_governance.rs:141`, `tests/integration/text_stream_portal_governance.rs:162`, `tests/integration/text_stream_portal_governance.rs:178`; redaction preserves geometry while suppressing content: `tests/integration/text_stream_portal_governance.rs:225`; safe-mode suspend/resume blocks and resumes updates under lease policy: `tests/integration/text_stream_portal_governance.rs:276`; freeze path uses generic backpressure signal (no portal-specific signal): `tests/integration/text_stream_portal_governance.rs:314`, `tests/integration/text_stream_portal_governance.rs:349`. |
| `text-stream-portals` :: Ambient Portal Attention Defaults | `hud-t98e.4`, `hud-r9u0` | Covered | Unread backlog remains ambient and coalesces without urgency escalation: `tests/integration/text_stream_portal_governance.rs:354`, `tests/integration/text_stream_portal_governance.rs:382`; typing indicator remains ambient styling/content and does not escalate interruption class: `tests/integration/text_stream_portal_surface.rs:908`, `tests/integration/text_stream_portal_surface.rs:947`, `tests/integration/text_stream_portal_surface.rs:951`; typing timing profile is explicitly constrained to ephemeral realtime semantics: `tests/integration/text_stream_portal_surface.rs:1050`, `tests/integration/text_stream_portal_surface.rs:1068`. |
| `text-stream-portals` :: External Adapter Isolation | `hud-t98e.3`, `hud-r11x` | Covered | Adapter sessions authenticate using `SessionInit` and explicit capabilities before lease operations: `tests/integration/text_stream_portal_adapter.rs:384`, `tests/integration/text_stream_portal_adapter.rs:392`, `tests/integration/text_stream_portal_adapter.rs:422`; tmux-specific selector is kept private to the tmux fixture adapter and does not cross bridge interface: `tests/integration/text_stream_portal_adapter.rs:162`; non-tmux adapter follows the same runtime contract and single-stream path: `tests/integration/text_stream_portal_adapter.rs:749`, `tests/integration/text_stream_portal_adapter.rs:845`. |
| `scene-graph` delta :: Text Stream Portal Phase-0 Uses Raw Tiles | `hud-t98e.2` | Covered | Pilot tile uses normal agent-owned z-order band under runtime-managed bands: `tests/integration/text_stream_portal_surface.rs:620`; rendered node composition remains raw-tile V1 primitives only: `tests/integration/text_stream_portal_surface.rs:697`. |
| `input-model` delta :: Text Stream Portal Interaction Reuses Local-First Input | `hud-t98e.1`, `hud-t98e.4` | Covered | Tile-local scroll contract is explicit in scene/input plumbing: `crates/tze_hud_scene/src/types.rs:405`, `crates/tze_hud_scene/src/graph.rs:292`, `crates/tze_hud_input/src/lib.rs:294`; local reply acknowledgement and local scroll authority behaviors are test-validated: `tests/integration/text_stream_portal_surface.rs:459`, `tests/integration/text_stream_portal_surface.rs:476`. |
| `system-shell` delta :: Text Stream Portals Remain Outside Chrome | `hud-t98e.2`, `hud-t98e.4` | Covered | Shell diagnostic snapshot redacts portal identity/transcript while keeping aggregate shell stats: `tests/integration/text_stream_portal_governance.rs:387`, `tests/integration/text_stream_portal_governance.rs:412`; shell override dismiss path revokes lease and removes portal tile under normal unconditional shell rules: `tests/integration/text_stream_portal_governance.rs:418`, `tests/integration/text_stream_portal_governance.rs:423`. |

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
