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

- [ ] 4.1 Resume from the authority-preserved retained visible window; clear the stale treatment
- [ ] 4.2 Preserve identity continuity: keep `logical_unit_id` idempotency-only and update a continued unit in place via its `coalesce_key` (no duplicate, no `logical_unit_id` semantics change)
- [ ] 4.3 Coalesce resumed appends under the existing state-stream cadence rules
- [ ] 4.4 Materialize only the bounded visible window (no scene-graph history reconstruction)
- [ ] 4.5 Preserve non-terminal pending input/ack state across reconnect
- [ ] 4.6 Start a fresh portal on attach after grace expiry (no silent revival of stale content)
- [ ] 4.7 Respect redaction at every frame of the stale-to-live transition

## 5. Evidence

- [ ] 5.1 Add a live/integration disconnect→stale→reconnect→resume run to the text-stream-portal evidence package, recording the degraded treatment and resume continuity
