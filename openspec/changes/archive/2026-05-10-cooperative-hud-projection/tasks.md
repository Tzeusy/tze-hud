## 1. Projection Contract and Packaging

- [x] 1.1 Define the provider-neutral projection operation schema for attach, publish_output, publish_status, get_pending_input, acknowledge_input, detach, and cleanup.
- [x] 1.2 Define bounded request/response shapes, size limits, idempotency fields, stable error codes, audit fields, and lifecycle states for projection operations.
- [x] 1.3 Define projection operation authentication, owner binding, cross-projection denial behavior, and local IPC or credential requirements.
- [x] 1.4 Create the first `/hud-projection` skill package and mirrored tool-facing instructions for cooperative attachment from an already-running LLM session.
- [x] 1.5 Add example flows for Codex, Claude, and opencode using the same projection contract.

## 2. Projection Daemon

- [x] 2.1 Implement daemon session storage for projection identity, HUD connection metadata, advisory portal lease identity, retained transcript bounds, pending input inbox, acknowledgement state, unread state, lifecycle, and fail-closed privacy classification.
- [x] 2.2 Implement HUD-facing resident gRPC connection management using the existing `HudSession` stream and text-stream portal raw-tile path.
- [x] 2.3 Implement attach, detach, cleanup, heartbeat, reconnect, fresh-auth-after-restart, stale-lease rejection, and lease release behavior.
- [x] 2.4 Implement bounded transcript retention, portal-visible window generation, output coalescing, retained-history pruning, and oversized-output rejection.
- [x] 2.5 Implement pending input queue bounds, expiry, acknowledgement transitions, and compact polling responses.

## 3. Portal Surface Integration

- [x] 3.1 Reuse the existing text-stream portal surface for projected session expanded and collapsed states.
- [x] 3.2 Render provider-neutral session identity, lifecycle status, unread state, pending-input state, and icon/profile hints without provider-specific behavior.
- [x] 3.3 Wire HUD composer submission into the daemon pending-input inbox as transactional bounded input with local-first accepted/rejected pending feedback.
- [x] 3.4 Verify collapse, restore, drag/reposition, and cleanup behavior against the current portal live UX harness.
- [x] 3.5 Ensure redaction, safe mode, freeze, dismiss, and orphan paths follow existing portal governance.

## 4. Validation

- [x] 4.1 Add unit tests for projection operation schema validation, bounds, idempotency, authorization, audit fields, stable error codes, and error handling.
- [x] 4.2 Add daemon tests for transcript retention, pending input acknowledgement, reconnect, stale lease handling, heartbeat, detach, and cleanup.
- [x] 4.3 Add integration tests proving cooperative projection does not require PTY, tmux, terminal capture, provider-specific RPCs, or runtime process lifecycle authority.
- [x] 4.4 Add live `/user-test` coverage for attach -> publish output -> submit HUD input -> poll/acknowledge -> collapse/restore -> detach cleanup on the Windows HUD.
- [x] 4.5 Add property/state-machine tests for projection lifecycle and pending-input queue transitions.
- [x] 4.6 Add bounded polling/output rate tests and local-ack/input-to-scene budget checks for the live portal flow.
- [x] 4.7 Record validation evidence under `docs/evidence/cooperative-hud-projection/`.

## 5. Reconciliation and Closeout

- [x] 5.1 Reconcile implementation against `cooperative-hud-projection` and `text-stream-portals` requirements, including privacy, attention, and external-adapter isolation.
- [x] 5.2 Generate a closeout report under `docs/reports/` with a requirement-to-evidence matrix and residual risk register.
- [x] 5.3 Sync accepted delta specs into `openspec/specs/` after implementation is verified.
- [x] 5.4 Archive the OpenSpec change after specs, implementation, validation evidence, and report are complete.
