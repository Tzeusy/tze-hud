## 1. Doctrine and RFC Alignment

- [x] 1.1 Update `about/heart-and-soul/` doctrine to name low-latency text interaction portals as a valid use case without collapsing the product into chat or terminal hosting
- [x] 1.2 Add RFC 0013 defining text stream portals, adapter boundaries, surface model, governance, and phase boundary
- [x] 1.3 Reconcile `about/legends-and-lore/README.md` and local legends-and-lore skill indexing with RFC 0013

## 2. Capability Specification

- [x] 2.1 Write the `text-stream-portals` capability spec with normative requirements and WHEN/THEN scenarios
- [x] 2.2 Add delta specs clarifying raw-tile pilot constraints in `scene-graph`
- [x] 2.3 Add delta specs clarifying transcript interaction expectations in `input-model`
- [x] 2.4 Add delta specs clarifying chrome exclusion and override behavior in `system-shell`

## 3. Direction and Readiness

- [x] 3.1 Publish a direction report capturing alignment, blockers, and explicit anti-goals
- [x] 3.2 Define the phase-0 pilot shape: resident raw tile, external adapter, no terminal-emulator semantics
- [x] 3.3 Define promotion criteria for any future first-class portal surface or node type

## 4. Pre-Implementation Review

- [x] 4.1 Run reconciliation review passes over doctrine, RFC, and OpenSpec artifacts
- [x] 4.2 Resolve review findings and stabilize the planning set
- [x] 4.3 Stop before bead generation and request signoff on the RFC/spec direction

## 5. Post-Archive Implementation History (for the record)

This change was scoped to doctrine/RFC/spec planning only and intentionally stopped before beads (task 4.3). After signoff, implementation moved forward under a separate epic tracked outside this archived change. Summarized here so future readers do not mistake the archive for the project's current state.

- [x] 5.1 Epic `hud-t98e` allocated and executed for the phase-0 raw-tile pilot
- [x] 5.2 `hud-t98e.1` — runtime-owned local-first scroll/input seam (PR #427)
- [x] 5.3 `hud-t98e.2` — resident raw-tile portal surface coverage using existing node primitives (PR #429)
- [x] 5.4 `hud-t98e.3` — transport-agnostic adapter integration evidence + tmux proof path (PR #432)
- [x] 5.5 `hud-t98e.4` — governance, coalescing, and interaction validation suites (PR #435)
- [x] 5.6 `hud-t98e.5` — gen-1 reconciliation; surfaced two follow-up gaps
- [x] 5.7 `hud-r11x` — non-tmux adapter conformance gap closed (PR #437)
- [x] 5.8 `hud-r9u0` — typing indicator ambient/ephemeral semantics gap closed (PR #438)
- [x] 5.9 `hud-fomr` — gen-2 reconciliation confirmed full 13/13 spec coverage (PR #441)
- [x] 5.10 `hud-t98e.6` — epic closeout report published at `docs/reports/hud-t98e-text-stream-portals.md`
- [ ] 5.11 Live user-test exemplar script (`text_stream_portal_exemplar.py`) and manual visual sign-off recorded in `docs/exemplar-manual-review-checklist.md` row 11

Primary evidence anchors (see the closeout report for full spec-to-test matrix):
- `tests/integration/text_stream_portal_surface.rs`
- `tests/integration/text_stream_portal_adapter.rs`
- `tests/integration/text_stream_portal_coalescing.rs`
- `tests/integration/text_stream_portal_governance.rs`
- `docs/evidence/text-stream-portals/validation-2026-04-16.md`
