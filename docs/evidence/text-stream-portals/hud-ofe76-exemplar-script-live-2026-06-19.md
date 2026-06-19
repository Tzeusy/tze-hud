# Text Stream Portal Phase-1 Live Evidence - hud-ofe76

Date: 2026-06-19
Issue: `hud-ofe76`
Adapter family: exemplar-script
Reference artifact: `hud-ofe76-exemplar-script-live-2026-06-19.json`
Supplemental artifact: `hud-ofe76-diagnostic-input-rerun-2026-06-19.json`

## Scope

Ran the extended exemplar-script phase set against the live TzeHouse Windows HUD:
`markdown,overflow,composer-edit,diagnostic-input,cadence,profile-swap,window-mgmt`.

The committed JSON artifacts are sanitized: private host/user/key values are
replaced with the public placeholders used by repository docs. The recovered
runtime PSK was used only in process environment and is not stored here.

## Preflight

- Worker context: `agent/hud-ofe76` worktree.
- Local harness self-test: `text_stream_portal_exemplar.py --self-test` passed.
- SSH reachability: both Windows users reachable with the repo-specific key.
- MCP reachability: `list_widgets` via `http://windows-host.example:9090/mcp`
  returned `main-progress` and `main-status`.
- gRPC preflight: baseline portal opened and released with `cleanup_errors=[]`.

## Live Run Facts

- Scene display area: `3862x2182`.
- Resolved portal size: `1729.8541666666667x1373.851851851852`.
- Main run step count: `58`.
- Main run cleanup: `cleanup_errors=[]`.
- Main run lease release: `lease_release_on_exit=true` and
  `cleanup:lease-release` completed.
- Supplemental diagnostic cleanup: `cleanup_errors=[]` and
  `cleanup:lease-release` completed.

## Phase Verdicts

| Phase | Machine result | Gate verdict | Notes |
|---|---:|---:|---|
| Markdown | PASS | PASS, not human-confirmed | Phase completed; transcript expected visual: markdown elements distinct, readable, no raw markup leaking. |
| Overflow | PASS | PASS, not human-confirmed | Near-budget and bounded transcript steps completed without transport or cleanup errors. |
| Composer edit | PASS | PASS, not human-confirmed | All seven deterministic composer states completed, from placeholder through typed/delete/clear. |
| Window management via OS input | PASS with supplement | PASS with supplement, not human-confirmed | Main run OS injector completed and produced focus plus scroll checkpoints, but missed `drag:start`/`drag:end`. A diagnostic-only rerun immediately after produced `input:focus-gained`, `drag:start`, `drag:end`, and `scroll:output`. |
| Cadence with RTT | COMPLETED | FAIL | 20/20 appends presented, but runtime-overhead budget failed: 5 appends exceeded `16.6ms`; mean `11.213ms`, p95 `21.033ms`, max `56.205ms`; `within_present_budget=false`. |
| Profile swap | PASS | PASS, not human-confirmed | Compact, standard, expanded, and restored-standard profile steps completed. Operator evidence fields remain `confirmed=null`. |
| Cleanup / lease release | PASS | PASS | Explicit lease release completed and portal tiles were removed according to the resident session response path. |

## Blocking Follow-Up

The only hard gate failure in this run is cadence budget conformance. Per the
`hud-ofe76` acceptance criteria, the issue should not be closed while this
axis fails; route the cadence evidence to the existing cadence blocker
(`hud-sonj6`) rather than masking the result in this evidence package.

Human visual sign-off was not collected in this worker run. The artifact records
operator-facing expected visuals and structured `operator_evidence` entries, but
their `confirmed` fields remain `null`.
