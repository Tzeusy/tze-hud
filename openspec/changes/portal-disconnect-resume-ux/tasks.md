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

- [x] 5.1 Add a live/integration disconnect→stale→reconnect→resume run to the text-stream-portal evidence package, recording the degraded treatment and resume continuity. DONE two ways: (a) headless integration proof `disconnect_then_reconnect_within_grace_resumes_same_surface_without_duplication` in `crates/tze_hud_runtime/src/portal_projection_driver.rs` (drop → degraded → reconnect within grace → resume on the SAME tile with both committed units present exactly once, degraded cleared, interaction re-enabled); (b) **on-device live capture** `docs/evidence/text-stream-portals/liveverify-20260621-205600/` against the real Windows HUD (`tzehouse-windows`) on a binary built from `main`@aa67a6e5 — baseline (active, units A/B/C) → degraded (`hud_unavailable` + "upstream link lost", transcript preserved) → resume (active, A/B/C persist + new unit D, no loss/dup). FIDELITY CAVEAT (in the package README): the live capture drives the lifecycle-state disconnect/resume presentation path (`publish_status`); the separate `connection_degraded` latch (upstream link loss → transcript dim + hud-h3mvo forced repaint) is not reachable from a stateless MCP HTTP call and stays headless-verified (`pure_drop_forces_degraded_repaint_without_subsequent_publish`, PR #978).

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
- [ ] 6.2 Visible degraded treatment (the viewer side of 2.1/2.3) is **partially landed**.
  (b) is now done: **confirmed P1** `hud-h3mvo` shipped via **PR #978** (merged) — a pure
  portal drop with no subsequent publish now forces a one-shot degraded repaint in
  `drain_inner` (flag set on the disconnect transition, re-rendered under the scene lock
  after the due-loop), so the tile dims within one frame instead of keeping live colors
  indefinitely; regression-locked by `pure_drop_forces_degraded_repaint_without_subsequent_publish`.
  (a) remains the only open gap: the token-styled geometry badge replacing the zero-width
  stale sentinel — bead `hud-jgf41` (P2), which did NOT ship (it over-scoped into a
  proto/scene-graph node-type change and was left as blocked local WIP, no PR). The badge
  will ride the same forced-repaint path once it lands. Until `hud-jgf41` lands the dim is
  perceivable on a drop but the dedicated badge affordance is not.
- [x] 6.3 Grace-expiry removal + resume verification (covers 3.2 headlessly, complements
  the already-done 4.6): `hud-xlx1r` (PR #974, merged) adds the grace-expiry-removal and
  disconnect→resume integration tests (see the §3.2/§5.1 test notes above).
- [x] 6.4 Task 5.1 live evidence — DONE 2026-06-21: live Windows disconnect→resume capture
  landed at `docs/evidence/text-stream-portals/liveverify-20260621-205600/` (reference HUD
  `tzehouse-windows`, current-main binary). The earlier reference-hardware/credentials gap was
  cleared (SSH as `tzeus@tzehouse-windows`, deploy via user-test portal-hud-deploy). The
  on-device **resize** hotkey re-verify (`hud-v4k1h`) still needs a human at the display — the
  current binary with the resize fix is now deployed and running, so the keyboard/visual check
  can be done directly.

Net: keep this change OPEN until the remaining visible-treatment gap lands (`hud-jgf41`
badge — `hud-h3mvo` repaint is now merged via PR #978). 5.1/6.4 live evidence is now recorded;
the trigger wiring (6.1), forced-repaint visibility (`hud-h3mvo`), headless grace/resume
proofs (6.3), and the live disconnect→resume capture are all in. The spec delta is sound and
unchanged. (hud-jgf41 is the sole remaining blocker before archive.)
