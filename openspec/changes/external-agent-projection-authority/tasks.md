## 1. Planning and Contract

- [x] 1.1 Create centralized `docs/20260511_goals.md` and keep it updated as work progresses.
- [x] 1.2 Inspect current cooperative projection, session protocol, lease governance, zone/widget, attention/privacy, and Windows runtime scope docs.
- [x] 1.3 Add OpenSpec proposal, design, and delta specs for the external agent projection authority.

## 2. Vertical Slice Implementation

- [x] 2.1 Add provider-neutral launched/attached session records to `tze_hud_projection`.
- [x] 2.2 Add Windows HUD target metadata and redacted credential-source handling.
- [x] 2.3 Add governed route planning for zone, widget, and leased portal surfaces.
- [x] 2.4 Add multi-session cleanup, revocation/expiry, and reconnect bookkeeping helpers.
- [x] 2.5 Add a deterministic three-session demo plan API.
- [x] 2.6 Add bounded provider process launch supervision for `Launched` sessions outside runtime/compositor core.

## 3. Validation

- [x] 3.1 Add focused tests for three concurrent sessions routed to zone, widget, and portal surfaces.
- [x] 3.2 Add tests proving owner isolation, redacted secrets, ambient attention defaults, cleanup, and reconnect fresh-auth behavior.
- [x] 3.3 Run focused `cargo test` gates for `tze_hud_projection`.
- [x] 3.4 Run or attempt live Windows `/user-test` for three projected sessions; record evidence or exact blocker.
- [ ] 3.5 Complete successful live Windows `/user-test` replay once `hud-9m47l` restores TzeHouse reachability; store evidence under `docs/evidence/external-agent-projection-authority/` and close `hud-dl3ys`.

## 4. Follow-Up Tracking and Closeout

- [x] 4.1 Create beads for remaining work that cannot be completed in this slice; `hud-dl3ys` remains open for successful live Windows replay, and `hud-opmuj` was closed after provider process launch supervision landed.
- [x] 4.2 Update `docs/20260511_goals.md` with validation evidence and residual gaps.
- [x] 4.3 Perform prompt-to-artifact completion audit and record that the overall goal remains incomplete until 3.5 passes.
