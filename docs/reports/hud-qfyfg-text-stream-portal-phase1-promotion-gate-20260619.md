# Text Stream Portal Phase-1 Promotion Gate Assessment

Date: 2026-06-19
Issue: `hud-qfyfg`
Decision: **FAIL - do not promote yet**
Decision owner: owner-gated; this report records the machine assessment only.

## Scope

This assessment covers the RFC 0013 section 7.2 promotion gate for moving the
text-stream portal beyond the raw-tile pilot into a first-class runtime portal
surface or node type.

Primary contract:
- `about/legends-and-lore/rfcs/0013-text-stream-portals.md` section 7.2
- `openspec/changes/text-stream-portal-phase1/specs/text-stream-portals/spec.md`
  requirement "Phase-1 Promotion Evidence Gate"
- `openspec/changes/text-stream-portal-phase1/specs/text-stream-portals/spec.md`
  requirement "Promotion Scope Boundary"

Evidence package reviewed:
- C3 / `hud-ofe76`: `docs/evidence/text-stream-portals/hud-ofe76-exemplar-script-live-2026-06-19.md`
- C3 transcript: `docs/evidence/text-stream-portals/hud-ofe76-exemplar-script-live-2026-06-19.json`
- C3 diagnostic supplement: `docs/evidence/text-stream-portals/hud-ofe76-diagnostic-input-rerun-2026-06-19.json`
- C4 / `hud-kylt0`: `docs/evidence/text-stream-portals/hud-kylt0-cooperative-projection-live-2026-06-19.md`
- C5 / `hud-pnofj`: PR #899, merge commit `a786e7c0`
  (`feat(user-test): constant-window soak phase for sustained streaming [hud-pnofj]`)
- Checklist: `docs/reports/exemplar-manual-review-checklist.md`

## Decision

Promotion fails closed. The raw-tile pilot remains the authoritative portal
scope, and Phase-1 behavioral requirements remain in force on raw tiles.

The gate cannot pass because the evidence package is incomplete and contains
hard failures:

- The exemplar-script adapter cadence axis failed the 16.6 ms runtime-overhead
  budget: 20/20 appends presented, but 5/20 exceeded budget, with p95
  `21.033 ms` and max `56.205 ms`.
- The cooperative-projection adapter did not complete the required
  attach -> stream -> poll/ack input -> detach lifecycle through the vendored
  skill surface. Live calls were blocked by `resident_mcp` capability gating or
  missing deployed methods.
- C5 landed the soak driver, not a completed 60-minute reference-host soak
  artifact proving <= 5 MiB memory drift for the portal path.
- Live Phase-1 governance confirmation is incomplete: ordinary cleanup and
  lease release passed, and integration tests cover redaction/safe-mode/freeze
  /orphan behavior, but the refreshed live package does not confirm redaction,
  safe mode, freeze, and orphan path during the Phase-1 runs.
- Human visual sign-off was not collected for the exemplar-script visual axes.
- No owner approval for promotion is present in the reviewed issue notes,
  evidence files, checklist, or PR #899 metadata.

## Criteria Assessment

| RFC 0013 section 7.2 criterion | Machine verdict | Rationale |
|---|---|---|
| Same layout/behavior pattern recurs across multiple adapters | **FAIL** | The exemplar-script adapter produced a live six-axis artifact, but the cooperative-projection adapter did not create or reuse a visible portal. Its lifecycle calls returned `CAPABILITY_REQUIRED` for `resident_mcp` or `Method not found`, so there is no completed multi-adapter behavior pattern yet. |
| Raw-tile expression is creating repeated complexity | **PASS, but not sufficient** | The exemplar-script path now requires a six-tile raw-tile assembly (`capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, `minimized_icon`), split `set_tile_root`/`add_node` mutation patterns, separate capture tiles for input and output, drag shields, minimized-icon state, explicit cleanup, and OS-input diagnostics. This is real complexity, but the gate still fails because the adapter, cadence, soak, governance, and owner gates are not satisfied. |
| Governance requirements are stable | **PARTIAL / FAIL for promotion** | Phase-0 integration coverage validates redaction, safe mode, freeze, orphan lifecycle, chrome exclusion, and ambient attention defaults. C3 proves ordinary cleanup and explicit lease release on the live HUD. The Phase-1 evidence package does not yet include live redaction, safe-mode, freeze, and explicit orphan/grace confirmation, so the refreshed promotion gate is not satisfied. |
| Feature remains subordinate to the broader presence thesis | **PASS, owner-gated** | The implementation and evidence remain content-layer, lease-governed, bounded, and below chrome. The failed gate itself supports the presence thesis by keeping the raw-tile pilot authoritative until the surface is both governed and ergonomic. This is a machine assessment, not owner approval. |
| System still does not need full terminal semantics | **PASS** | Reviewed artifacts and contracts keep the boundary at bounded text streams, runtime-owned draft state, and semantic input. No VT100/xterm compatibility, PTY hosting, alternate screen, terminal mouse reporting, terminal keystroke passthrough, dedicated portal transport, or external process lifecycle ownership is introduced. |

## C3 / Exemplar-Script Adapter

The `hud-ofe76` live run is valid reference-host evidence for the
exemplar-script adapter family. It carries the `TzeHouse` reference tag, used
the live Windows HUD, covered `markdown,overflow,composer-edit,diagnostic-input,
cadence,profile-swap,window-mgmt`, and recorded `cleanup_errors=[]` plus
successful explicit lease release. The diagnostic supplement captured
`input:focus-gained`, `drag:start`, `drag:end`, and `scroll:output`.

It is not a passing promotion artifact because cadence failed the runtime
overhead budget and human visual confirmations remain `null`.

## C4 / Cooperative Projection Adapter

The `hud-kylt0` note proves the Windows host and HUD were reachable: SSH
worked, `TzeHudOverlay` was running, ports `9090` and `50051` were open, and
ordinary MCP discovery succeeded.

It is not passing adapter evidence. The cooperative-projection lifecycle did
not reach a visible portal through the vendored skill surface:

- `portal_projection_attach`: `CAPABILITY_REQUIRED`
- `portal_projection_publish`: `CAPABILITY_REQUIRED`
- `portal_projection_get_pending_input`: `CAPABILITY_REQUIRED`
- `portal_projection_acknowledge_input`: `CAPABILITY_REQUIRED`
- `portal_projection_detach`: `CAPABILITY_REQUIRED`
- `portal_projection_cleanup`: `Method not found`
- `projection_operation`: `Method not found`
- `portal_projection_publish_status`: `Method not found`

This blocks the agent-ergonomics criterion because no LLM-driven
attach/stream/input/detach lifecycle was demonstrated without scene-graph
ceremony in the LLM context.

## C5 / Soak Driver

PR #899 is useful implementation evidence, but it is not sufficient promotion
evidence. It adds a constant-window `soak` phase to
`.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`, plus
headless tests for bounded tail history, publish-time-adjusted pacing, and
soak-sized lease TTL.

The promotion gate requires a completed 60-minute sustained-streaming run on
the reference host with <= 5 MiB memory drift recorded as an artifact. No such
artifact is present in `docs/evidence/text-stream-portals/` or `docs/reports/`
for C5, so C5 remains a harness-readiness input rather than a gate-passing
artifact.

## Required Owner Follow-Up

The owner should treat this as a failed machine assessment, not a promotion
approval. Before promotion can be reconsidered:

1. Fix or explicitly waive the cadence overhead failure with owner approval.
2. Make cooperative projection usable by the intended external skill surface,
   including resident-capable ingress and deployed lifecycle method parity.
3. Run and archive the LLM-driven agent-ergonomics demonstration.
4. Run and archive the 60-minute reference-host portal soak proving <= 5 MiB
   memory drift.
5. Add live Phase-1 governance confirmation for redaction, safe mode, freeze,
   and explicit orphan/grace behavior.
6. Collect human visual sign-off for the visual axes or record an explicit
   owner waiver.

This worker did not create or close Beads. Coordinator-owned follow-up tracking
is still required for any gaps not already covered by existing beads.
