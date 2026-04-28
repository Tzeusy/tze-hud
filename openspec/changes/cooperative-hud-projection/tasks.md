## 1. Projection Contract and Packaging

- [ ] 1.1 Define the provider-neutral projection operation schema for attach, publish_output, publish_status, get_pending_input, acknowledge_input, detach, and cleanup.
- [ ] 1.2 Define bounded request/response shapes, size limits, idempotency fields, error codes, and lifecycle states for projection operations.
- [ ] 1.3 Create the first `/hud-projection` skill package and mirrored tool-facing instructions for cooperative attachment from an already-running LLM session.
- [ ] 1.4 Add example flows for Codex, Claude, and opencode using the same projection contract.

## 2. Projection Daemon

- [ ] 2.1 Implement daemon session storage for projection identity, HUD connection metadata, portal lease identity, retained transcript bounds, pending input inbox, acknowledgement state, unread state, lifecycle, and privacy classification.
- [ ] 2.2 Implement HUD-facing resident gRPC connection management using the existing `HudSession` stream and text-stream portal raw-tile path.
- [ ] 2.3 Implement attach, detach, cleanup, heartbeat, reconnect, and lease release behavior.
- [ ] 2.4 Implement bounded transcript retention, portal-visible window generation, output coalescing, and oversized-output rejection or truncation.
- [ ] 2.5 Implement pending input queue bounds, expiry, acknowledgement transitions, and compact polling responses.

## 3. Portal Surface Integration

- [ ] 3.1 Reuse the existing text-stream portal surface for projected session expanded and collapsed states.
- [ ] 3.2 Render provider-neutral session identity, lifecycle status, unread state, pending-input state, and icon/profile hints without provider-specific behavior.
- [ ] 3.3 Wire HUD composer submission into the daemon pending-input inbox as transactional bounded input.
- [ ] 3.4 Verify collapse, restore, drag/reposition, and cleanup behavior against the current portal live UX harness.
- [ ] 3.5 Ensure redaction, safe mode, freeze, dismiss, and orphan paths follow existing portal governance.

## 4. Validation

- [ ] 4.1 Add unit tests for projection operation schema validation, bounds, idempotency, and error handling.
- [ ] 4.2 Add daemon tests for transcript retention, pending input acknowledgement, reconnect, heartbeat, detach, and cleanup.
- [ ] 4.3 Add integration tests proving cooperative projection does not require PTY, tmux, terminal capture, provider-specific RPCs, or runtime process lifecycle authority.
- [ ] 4.4 Add live `/user-test` coverage for attach -> publish output -> submit HUD input -> poll/acknowledge -> collapse/restore -> detach cleanup on the Windows HUD.
- [ ] 4.5 Record validation evidence under `docs/evidence/cooperative-hud-projection/`.

## 5. Reconciliation and Closeout

- [ ] 5.1 Reconcile implementation against `cooperative-hud-projection` and `text-stream-portals` requirements, including privacy, attention, and external-adapter isolation.
- [ ] 5.2 Generate a closeout report under `docs/reports/` with a requirement-to-evidence matrix and residual risk register.
- [ ] 5.3 Sync accepted delta specs into `openspec/specs/` after implementation is verified.
- [ ] 5.4 Archive the OpenSpec change after specs, implementation, validation evidence, and report are complete.
