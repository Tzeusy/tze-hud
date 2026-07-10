# Live owner visual sign-off pass — text-stream portal (tzehouse)

Date: 2026-07-05
Host: windows-host.example (real GPU, overlay mode)
Build: C:\tze_hud\tze_hud.exe LastWriteTime 2026-07-04 14:43 (round5 build)
Driver: text_stream_portal_exemplar.py (has #1010 lease renewal: renew_lease + 0.75-TTL loop)
Config change (reversible, backed up to tze_hud.toml.pre-telemetry.bak):
  added "read_telemetry" to agents.registered.{agent-alpha,beta,gamma}.capabilities
  — required for the lease scope AND enables the non-proxy present-ack cadence path (hud-vjlqh).

## What passed
- gRPC resident session + lease grant (ttl=720000ms, renewing). No lease-expiry.
- Streaming reveal (10 chunks) + baseline render: portal content PAINTS (cooperative render functional).
- Runtime-owned composer input path WORKS end to end at the protocol level:
  per-char key-down/up observed, coalesced composer_draft_state (cursor 45->49),
  composer_draft_submit recorded ("why is this window now grey? It used to be black?"),
  input_history_len incremented to 2. Owner physically typed + submitted.

## Defects found (owner live sign-off)
1. GREY FRAME (hud-a328c, P1) — frame background paints grey on the runtime-handshake
   token path (production default). Forcing local mirror (--ignore-runtime-tokens,
   portal.frame.background=#111720) renders off-black. Owner confirmed A/B live: grey -> black.
   => runtime active-profile token resolution/delivery of portal.frame.background diverges
      from the #111720 default. Real agents (runtime tokens) see grey.
2. INPUT-PANE HISTORY NOT RENDERED (hud-3nus3, P2) — submitted input is tracked
   (input_history_len=2) but not visibly painted beneath the composer in the LEFT pane.
   Related blocked bead hud-acfvp. Possibly compounded by grey-on-grey contrast / composer.anchor=top.

## Gate impact
- Multi-adapter render axis: FUNCTIONAL (content paints, sustained, input works).
- Visual sign-off axis: NOT CLEAN on the production token path (grey frame) => stays FAIL
  until hud-a328c fixed and re-verified. Render is correct only when bypassing runtime tokens.
- Note: GDI CopyFromScreen cannot capture the transparent overlay (only wallpaper);
  owner eyes are the ground truth for this pass.

## Artifacts
- interactive-transcript.jsonl / interactive.log — streaming + composer input run (runtime tokens, grey)
- ab-localtokens-transcript.jsonl / ab-localtokens.log — forced local mirror (#111720, off-black, owner-confirmed)

## UPDATE 2026-07-05 — grey P1 fixed + verified live
- Fix landed: PR #1075 merged to main (55f9c19d) — canonical portal.frame.background default #111720 -> #0A0D11 (opaque near-black, matches transcript pane, backdrop-independent). Owner rejected translucent #0000004D for backdrop-dependence.
- Rebuilt windows-gnu exe (sha 9aa4da04...) deployed to tzehouse; remote sha verified match. Includes main's composer fix #1074.
- RE-VERIFY on the PRODUCTION runtime-token path (token_source=runtime-handshake, 47 tokens — the exact path that showed grey): owner confirmed frame is now OPAQUE OFF-BLACK. hud-a328c CLOSED + verified live.
- Visual-frame sign-off: OBTAINED. Remaining separate item: hud-3nus3 (P2, input-pane history not painted — runtime composer geometry/echo, dedicated follow-up).

## 60-min soak result (2026-07-05, corrected off-black build, runtime tokens)
Lease ttl=3720000ms (renews at 75%). Soak phase: append ~250ms, 60-line tail window, read_telemetry granted.
- LEASE SELF-TERMINATION FIXED: ran 3453.887s / 13303 cycles (was 608s on lease expiry before hud-hk8kl). hud-hk8kl validated live.
- MEMORY: independent HUD WorkingSet64 sampling (soak-hud-rss.log) held 27-34 MiB across 57.6 min, NO upward trend (first ~33 -> last ~30, net ~flat). Drift PASSES on trend; formal read_telemetry drift figure NOT emitted (exemplar reports it only on full-duration success).
- HONEST FAILURE MARKER: wrote soak-aborted.marker (SOAK_ABORTED), not a false-pass — hud-5wos2 marker fix validated.
- ABORT: at cycle 13303 / 3453.887s (146s short of 3600s) on "TimeoutError: Timed out waiting for mutation_result". No prior backpressure/retry warnings across 13303 cycles => looks transient (single mutation-ack blip), not systemic. NEW defect filed.
VERDICT: soak axis NOT yet a clean PASS (did not reach 3600s), but the two things it tests (no lease self-termination + flat memory) both look GOOD. Needs one clean 3600s completion for the formal artifact.
