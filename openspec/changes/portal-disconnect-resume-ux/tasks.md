# Tasks — Portal Disconnect/Resume Viewer UX

This change is a spec delta closing a design-coverage gap. No runtime
implementation begins until the change is reviewed and accepted; acceptance
authorizes the disconnect/resume viewer-facing work on the existing raw-tile
pilot, bounded by the existing lease orphan/grace lifecycle.

## 1. Contract and review

- [ ] 1.1 Validate this OpenSpec change with `openspec validate portal-disconnect-resume-ux --strict`
- [ ] 1.2 Confirm doctrine alignment: "graceful degradation is not a bug", "arrival time ≠ presentation time", "visual identity is modular" (CLAUDE.md, `about/heart-and-soul`)
- [ ] 1.3 Confirm the delta adds no new transport, no portal-specific disconnect RPC, and no scene-graph transcript history (disconnect rides the existing primary session stream / orphan-lease path)
- [ ] 1.4 Confirm staleness is bounded by the existing lease grace, not a second timer authority

## 2. Disconnect presentation (raw-tile pilot)

- [x] 2.1 Drive a token-resolved degraded treatment (dim + stale marker + disconnect affordance) from the component-profile path; no hardcoded styling
- [x] 2.2 Clear typing/activity/ephemeral-realtime indicators on disconnect
- [x] 2.3 Keep the disconnect indicator geometry-only and present under redaction
- [x] 2.4 Verify the retained coherent window is preserved on disconnect (no committed-unit loss)

## 3. Degradation contract

- [x] 3.1 Wire the degraded threshold (liveness gap) to the existing `ProjectionLifecycleState::Degraded`/`HudUnavailable` transition and `mark_hud_disconnected` bookkeeping
- [ ] 3.2 Bound the degraded window by lease grace; remove the surface on grace expiry via the existing orphan path
- [x] 3.3 Keep degraded-transition presentation timing runtime-owned; clock-domain typed metadata only

## 4. Reconnect and resume presentation

- [x] 4.1 Resume from the authority-preserved retained visible window; clear the stale treatment (latch is derived from `hud_connection`/`last_disconnect_wall_us`, so `record_hud_connection` clears it and the never-cleared retained window resumes; locked by `reconnect_resumes_from_retained_window_and_clears_stale_treatment`)
- [x] 4.2 Preserve identity continuity: keep `logical_unit_id` idempotency-only and update a continued unit in place via its `coalesce_key` (locked by `reconnect_continues_in_progress_unit_in_place_via_coalesce_key` + `reconnect_replayed_logical_unit_id_stays_idempotent`)
- [x] 4.3 Coalesce resumed appends under the existing state-stream cadence rules (coalesce-key in-place path is reconnect-agnostic; exercised under the 1/window rate limit in the §4.2 coalesce test)
- [x] 4.4 Materialize only the bounded visible window (no scene-graph history reconstruction) (locked by `reconnect_materializes_only_bounded_visible_window`)
- [x] 4.5 Preserve non-terminal pending input/ack state across reconnect (`mark_hud_disconnected` does not touch `pending_input`; locked by `reconnect_preserves_transcript_inbox_ack_state_and_requires_new_lease`)
- [ ] 4.6 Start a fresh portal on attach after grace expiry (no silent revival of stale content) — lease-orphan/grace path, outside the projection authority; tracked by hud-0q1dh (`degraded portal surface removed via lease orphan grace`)
- [x] 4.7 Respect redaction at every frame of the stale-to-live transition (locked by `stale_to_live_transition_respects_redaction_every_frame`)

## 5. Evidence

- [ ] 5.1 Add a live/integration disconnect→stale→reconnect→resume run to the text-stream-portal evidence package, recording the degraded treatment and resume continuity
