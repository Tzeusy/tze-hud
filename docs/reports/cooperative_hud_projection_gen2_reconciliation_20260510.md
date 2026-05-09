# Cooperative HUD Projection Gen-2 Reconciliation

Date: 2026-05-10
Issue: `hud-ggntn.11`
OpenSpec change: `openspec/changes/cooperative-hud-projection`

## Verdict

`cooperative-hud-projection` is implementation-complete for the active delta requirements and scenarios. GAP-1 and GAP-2 are closed by landed code and tests. GAP-3 is closed for archive purposes only if the project accepts the documented evidence substitution: SSH-triggered Windows desktop capture could not observe the HUD overlay, so the proof set uses live resident gRPC transcripts and scene snapshots from Windows plus a runtime-native compositor readback artifact.

OpenSpec sync is ready. Archive is ready after sync, with the evidence-substitution note preserved in the archive trail.

## Inputs Audited

- OpenSpec artifacts:
  - `openspec/changes/cooperative-hud-projection/proposal.md`
  - `openspec/changes/cooperative-hud-projection/design.md`
  - `openspec/changes/cooperative-hud-projection/specs/cooperative-hud-projection/spec.md`
  - `openspec/changes/cooperative-hud-projection/specs/text-stream-portals/spec.md`
  - `openspec/changes/cooperative-hud-projection/tasks.md`
  - `openspec/changes/cooperative-hud-projection/reconciliation.md`
- Prior reports:
  - `docs/reconciliations/cooperative_hud_projection_reconciliation_gen1_20260429.md`
  - `docs/reports/cooperative_hud_projection_hud-ggntn_closeout_20260429.md`
- Evidence:
  - `docs/evidence/cooperative-hud-projection/validation-2026-04-29.md`
  - `docs/evidence/cooperative-hud-projection/resident-grpc-adapter-2026-04-29.md`
  - `docs/evidence/cooperative-hud-projection/live-governance-2026-05-09/README.md`
  - `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/README.md`
  - `docs/evidence/text-stream-portals/validation-2026-04-16.md`
- Implementation and tests:
  - `crates/tze_hud_projection/src/lib.rs`
  - `crates/tze_hud_projection/src/bin/projection_authority.rs`
  - `crates/tze_hud_projection/src/resident_grpc.rs`
  - `crates/tze_hud_projection/tests/projection_authority_cli.rs`
  - `tests/integration/text_stream_portal_adapter.rs`
  - `tests/integration/text_stream_portal_surface.rs`
  - `tests/integration/text_stream_portal_governance.rs`
  - `examples/render_artifacts/src/bin/cooperative_projection_readback.rs`

## Gen-1 Gap Closure

| Gap | Gen-2 status | Evidence |
| --- | --- | --- |
| GAP-1: executable LLM-facing projection authority surface | Closed | `tze_hud_projection_authority` is an in-repo stdio surface that keeps process-lifetime `ProjectionAuthority` state, dispatches newline-delimited JSON to the normative handlers, bounds input/response shape, rejects operator secrets on argv, and emits newly written audit records: `crates/tze_hud_projection/src/bin/projection_authority.rs`. CLI tests cover attach, publish, denied reads, audit redaction, oversized lines, and audit rollover: `crates/tze_hud_projection/tests/projection_authority_cli.rs`. |
| GAP-2: resident gRPC visible text-stream portal adapter | Closed | `ResidentGrpcPortalAdapter` emits existing `HudSession` `MutationBatch` and `LeaseRelease` messages, creates/reuses a content-layer tile, maps composer text into the semantic inbox, supports collapse/restore/move, and records adapter-local budget samples: `crates/tze_hud_projection/src/resident_grpc.rs`. Headless resident gRPC integration covers lifecycle, input, compact movement, cleanup, and no process authority: `tests/integration/text_stream_portal_adapter.rs`. Evidence is recorded in `docs/evidence/cooperative-hud-projection/resident-grpc-adapter-2026-04-29.md`. |
| GAP-3: live governance validation | Closed with evidence substitution | The live Windows governance attempt reached the runtime and drove the full storyboard over resident gRPC, but screenshots captured the lock screen. PR #636 adds a runtime-native readback proof and records the desktop-capture blocker. Evidence lives under `docs/evidence/cooperative-hud-projection/live-governance-2026-05-09/` and `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/`. |

## Requirement Coverage

| Capability requirement | Status | Evidence |
| --- | --- | --- |
| Cooperative Attachment Contract | Covered | Attach schema and handler remain provider-neutral and cooperative-only: `crates/tze_hud_projection/src/lib.rs`. Skill packaging states no terminal capture or PTY attachment: `.claude/skills/hud-projection/SKILL.md`. CLI surface makes attach callable by an already-running session through stdio JSON. |
| External Projection State Authority | Covered | `ProjectionAuthority` owns memory-only transcript, inbox, lifecycle, HUD connection, advisory lease, and audit state outside runtime core; detach/cleanup purge private state. CLI process lifetime matches the v1 memory-only persistence decision. |
| Low-Token LLM-Facing Operations | Covered | The stdio CLI dispatches `attach`, `publish_output`, `publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, and `cleanup` to the normative handlers and returns bounded `ProjectionResponse` plus audit records. |
| Projection Operation Authorization | Covered | Owner tokens, verifier storage, cross-projection denial, operator cleanup separation, token expiry, and audit redaction are covered by local/unit tests and CLI tests. |
| HUD Input Inbox Delivery | Covered | `submit_portal_input`, `handle_get_pending_input`, and `handle_acknowledge_input` implement pending/delivered/deferred/handled/rejected/expired transitions, FIFO delivery, idempotent terminal ack replay, conflict rejection, and expiry. |
| Projected Portal Lifecycle | Covered | Resident adapter creates/reuses a tile, renders expanded/collapsed state, moves compact geometry, restores expanded composer state, and releases the lease; integration verifies stale tile cleanup. |
| Provider-Neutral Projection Identity | Covered | Provider kinds `codex`, `claude`, `opencode`, and `other` share the same operation and portal semantics; provider-specific behavior is limited to optional profile hints. |
| Privacy and Attention Governance | Covered with evidence substitution | Local policy shaping preserves geometry, redacts identity/transcript/input details, disables redacted interactions, fails missing classification closed, and keeps attention ambient. Live storyboard records redaction, safe/freeze/dismiss, orphan cleanup, and backlog non-escalation states, with runtime-native readback standing in for unavailable desktop screenshots. |
| Bounded Backpressure and Expiry | Covered | Default bounds, oversized output/input rejection, queue full behavior, poll limits, retained-transcript pruning, coalescing, and adapter-local update/input feedback budgets are tested and evidenced. |
| Text-stream delta: Cooperative LLM Projection Adapter | Covered | Cooperative projection is a valid non-tmux adapter family using text-stream portal semantics; integration asserts runtime-facing state exposes no PTY, tmux, terminal, stdin/stdout, process lifecycle, spawn, kill, or provider RPC authority. |
| Text-stream delta: Cooperative Projection Input Mapping | Covered | Resident adapter maps composer text into the cooperative semantic inbox; tests poll it through `get_pending_input` rather than sending raw keystrokes. |
| Text-stream delta: Cooperative Projection State Externality | Covered | Full retained transcript, pending input, acknowledgement state, reconnect metadata, and advisory lease bookkeeping remain in `ProjectionAuthority`; portal materialization exposes only bounded visible state and policy-permitted metadata. |

## Scenario Coverage

All delta scenarios have code or evidence coverage. The only non-literal coverage is visible Windows screenshot evidence, replaced by live Windows scene snapshots plus runtime-native readback:

- Live Windows proof transcript: `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/logs/live-projection-proof-transcript.json`
- Interim scene snapshot with proof tile: `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/logs/post-cleanup-scene-snapshot.json`
- Final cleanup snapshot with `tiles=0`: `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/logs/final-cleanup-scene-snapshot.json`
- Runtime-native readback metadata and PNG: `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/readback/cooperative-projection-readback.json`, `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/readback/cooperative-projection-readback.png`
- Governance storyboard states: `docs/evidence/cooperative-hud-projection/live-governance-2026-05-09/logs/live-governance-storyboard-transcript.json`

The readback metadata records `tile_count=1`, `node_count=1`, `active_leases=1`, background samples `[63,63,89,255]`, and projection-tile samples `[56,69,89,255]`. The Windows final cleanup snapshot records zero remaining tiles and no proof text or `agent-alpha` references.

## Task Ledger

`openspec/changes/cooperative-hud-projection/tasks.md` has been normalized to mark implementation, validation, reconciliation, and report tasks complete. The only remaining unchecked tasks are intentionally the final lifecycle tasks:

- `5.3` Sync accepted delta specs into `openspec/specs/`.
- `5.4` Archive the OpenSpec change after sync and evidence acceptance.

## Residual Risk Register

| Risk | Disposition |
| --- | --- |
| SSH-triggered Windows desktop capture does not observe the HUD overlay. | Accepted archive risk if the project accepts runtime-native readback as the proof source. Keep the blocker documented in `docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/README.md`. |
| The stdio authority is daemon-local and process-lifetime only. | Matches the v1 memory-only design. Durable daemon state remains out of scope and would need a future encrypted-store contract. |
| Live pointer/synthetic input sign-off remains weak in the broader text-stream portal harness. | Not a cooperative projection archive blocker because the projection adapter maps composer submissions to semantic inbox state and cleanup is validated; broader hit-region sign-off is tracked by text-stream portal evidence. |
| OpenSpec archive before spec sync would lose accepted deltas. | Do not archive before running the sync step and preserving this report in the archive trail. |

## Sync / Archive Checklist

1. Run `openspec validate cooperative-hud-projection --strict`.
2. Sync accepted deltas into canonical specs:
   - `cooperative-hud-projection`
   - `text-stream-portals`
3. Re-run validation after sync.
4. Record in the archive note that runtime-native readback is the accepted substitute for SSH desktop screenshot capture.
5. Archive `cooperative-hud-projection`.

## Follow-Ups

No implementation follow-up beads are required for `hud-ggntn.11`.
