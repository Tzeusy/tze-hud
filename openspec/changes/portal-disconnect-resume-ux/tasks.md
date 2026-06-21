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
- [ ] 3.2 Bound the degraded window by lease grace; remove the surface on grace expiry via the existing orphan path (scene+authority contract headless-covered by `disconnected_portal_surface_removed_on_grace_expiry_yields_no_further_state` in `crates/tze_hud_runtime/src/portal_projection_driver.rs`: a portal held disconnected until lease grace has its tile removed by `SceneGraph::expire_leases` and then produces no further `ProjectedPortalState`. Production trigger that drives liveness-gap → `mark_hud_disconnected` → scene orphan → `expire_projection` is wired separately by hud-5i16d)
- [x] 3.3 Keep degraded-transition presentation timing runtime-owned; clock-domain typed metadata only

## 4. Reconnect and resume presentation

- [x] 4.1 Resume from the authority-preserved retained visible window; clear the stale treatment (latch is derived from `hud_connection`/`last_disconnect_wall_us`, so `record_hud_connection` clears it and the never-cleared retained window resumes; locked by `reconnect_resumes_from_retained_window_and_clears_stale_treatment`)
- [x] 4.2 Preserve identity continuity: keep `logical_unit_id` idempotency-only and update a continued unit in place via its `coalesce_key` (locked by `reconnect_continues_in_progress_unit_in_place_via_coalesce_key` + `reconnect_replayed_logical_unit_id_stays_idempotent`)
- [x] 4.3 Coalesce resumed appends under the existing state-stream cadence rules (coalesce-key in-place path is reconnect-agnostic; exercised under the 1/window rate limit in the §4.2 coalesce test)
- [x] 4.4 Materialize only the bounded visible window (no scene-graph history reconstruction) (locked by `reconnect_materializes_only_bounded_visible_window`)
- [x] 4.5 Preserve non-terminal pending input/ack state across reconnect (`mark_hud_disconnected` does not touch `pending_input`; locked by `reconnect_preserves_transcript_inbox_ack_state_and_requires_new_lease`)
- [x] 4.6 Start a fresh portal on attach after grace expiry (no silent revival of stale content) — fix + test in hud-pk9pz: gate `InProcessPortalDriver::ensure_driver_lease` reuse on the new scene-layer `SceneGraph::lease_is_active` predicate (active-state, not mere map presence) so a post-grace re-attach grants a FRESH lease and creates a FRESH tile instead of reusing the resident-but-Expired lease (which made `create_tile` fail `require_active_lease` and silently yield no portal). No new timer — grace still lives on the lease orphan path (cf. hud-0q1dh/#883)
- [x] 4.7 Respect redaction at every frame of the stale-to-live transition (locked by `stale_to_live_transition_respects_redaction_every_frame`)

## 5. Evidence

- [ ] 5.1 Add a live/integration disconnect→stale→reconnect→resume run to the text-stream-portal evidence package, recording the degraded treatment and resume continuity (headless integration proof landed by `disconnect_then_reconnect_within_grace_resumes_same_surface_without_duplication` in `crates/tze_hud_runtime/src/portal_projection_driver.rs`: drop → degraded → reconnect within grace → resume on the SAME tile with both committed units present exactly once, degraded cleared, interaction re-enabled. NOTE: live-Windows evidence package run is still owed — this is the headless integration proof, not the on-device capture)

## 6. Reconciliation — Wave-2 (portal audit 2026-06-21)

The 2026-06-21 portal audit found that several §2/§3 tasks were marked done as
*spec-delta-complete* but their runtime behavior was **dormant**: the degraded
bookkeeping existed with no production trigger, and the "geometry-only disconnect
indicator" (2.3) was a zero-width sentinel run that painted nothing a viewer could see.
Wave-2 of epic `hud-wse80` (sub-epic `hud-3jxfr`) supplies the trigger wiring; the
*visible* half is only partially landed (see 6.2):

- [x] 6.1 Production trigger for the degraded transition (completes the runtime side of 3.1):
  `hud-5i16d` (PR #973, merged) wires `mark_all_projections_disconnected` from the MCP
  portal_op channel-close path so an ungraceful upstream drop actually flips
  `connection_degraded`. Clean detach does not. **Follow-ups:** per-session resident gRPC
  bidi stream-end detection is `hud-b2llg`; the in-process forced repaint that makes the
  flip *visible* on a pure drop is `hud-h3mvo` (see 6.2).
- [ ] 6.2 Visible degraded treatment (the viewer side of 2.1/2.3) is **NOT yet landed**.
  Two gaps remain: (a) the token-styled geometry badge replacing the zero-width stale
  sentinel — bead `hud-jgf41`, which did NOT ship (it over-scoped into a proto/scene-graph
  node-type change and was left as blocked local WIP, no PR); and (b) **confirmed P1**
  `hud-h3mvo`: on a pure portal drop with no subsequent publish, `mark_hud_disconnected`
  flips the latch but nothing re-renders, so the tile keeps live colors indefinitely
  (verified in the #973 review). Until both land, the degraded state is bookkept but not
  reliably perceivable.
- [x] 6.3 Grace-expiry removal + resume verification (covers 3.2 headlessly, complements
  the already-done 4.6): `hud-xlx1r` (PR #974, merged) adds the grace-expiry-removal and
  disconnect→resume integration tests (see the §3.2/§5.1 test notes above).
- [ ] 6.4 Task 5.1 live evidence remains owed — the live Windows disconnect→resume run is
  blocked on the same reference-hardware/credentials gap as the resize live-verify
  (`hud-v4k1h` / `hud-0yrix`). Do NOT archive this change until 5.1 lands.

Net: keep this change OPEN until the visible degraded treatment lands (`hud-h3mvo` P1 +
`hud-jgf41`) and 5.1 live evidence is recorded. The trigger wiring (6.1) and headless
grace/resume proofs (6.3) are merged; the spec delta itself is sound and unchanged.
