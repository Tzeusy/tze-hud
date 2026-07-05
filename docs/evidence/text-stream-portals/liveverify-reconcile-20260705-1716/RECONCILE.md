# Reconcile pass — text-stream portal live beads (tzehouse, 2026-07-05 ~17:16)

Follows the owner's live sign-off pass (liveverify-signoff-20260705-105700).
Rig: tzehouse-windows.parrot-hen.ts.net, build 9aa4da04 (owner's, #1075 grey-fix + #1074 composer-fix), overlay, real RTX 3080.
Driver: text_stream_portal_exemplar.py, reference-tagged (TzeHouse / RTX 3080).

## hud-sonj6 (cadence 5.5-5.7)
- 5.7 cadence/RTT (this pass, cadence-transcript.jsonl):
  - transport_rtt_baseline_ms = 38.5
  - runtime_overhead_ms: mean 0.002, p95 0.003, max 0.003, OVER_BUDGET_COUNT = 0 (budget 16.6ms)  => PASS
  - 20/20 cycles presented, within_present_budget = true
  - present_source = rtt-proxy (true FramePresented path not exercised in this short run)
  - transport RTT variable (7/20 > 100ms, max 362ms) = WAN latency, isolated from runtime overhead
- 5.5 soak (owner's run, soak.log): memory drift STABLE (RSS 27-34 MiB across 57 min) => drift budget PASS;
  full 60-min duration FAILED — aborted cycle 13303/3454s on a single "Timed out waiting for mutation_result"
  ~2.4 min short. Root cause: WAN transport stall (corroborated by cadence RTT spikes), NOT a runtime hang
  (HUD PID 46880 healthy + MCP responsive immediately after). Needs a clean-duration rerun or explicit split per AC#4.

## Owner pass already covered (liveverify-signoff-20260705-105700/SUMMARY.md)
- hud-a328c grey-frame P1: fixed (#1075) + owner-verified live on runtime-token path => CLOSED.
- hud-3nus3 (input-pane history not painted): filed + CLOSED.
- Composer input path (relevant to hud-o9ybl): owner physically typed + submitted, per-char keydown/up,
  coalesced composer_draft_state, composer_draft_submit recorded, input_history_len=2 => end-to-end functional.
- Visual-frame sign-off OBTAINED on production runtime-token path.

## Still open / needs owner
- hud-sonj6: attach this cadence PASS; decide soak rerun vs explicit split (drift already proven).
- hud-o9ybl: owner composer sign-off done today; needs the reference-tagged composer-edit artifact if required for closure.
- hud-tlx5c: profile-swap reskin — needs owner eyes on before/after (token A/B grey->black already confirmed today).
- hud-t2k55 (P3): live OS input-injection resize — automatable via diagnostic-input path, not yet run.
- hud-qfyfg gate: owner-gated decision; visual-frame axis now clean, cadence within budget.


## Reconcile runs completed (this pass)
- cadence: PASS (runtime overhead over_budget_count=0) — hud-sonj6 5.7
- composer-edit: PASS (7 caret-tracked states, no phantoms) — hud-o9ybl
- window-mgmt: PASS geometry axis (move/clamp/minimize/restore) — adjacent to hud-t2k55 but NOT OS-injection resize
- hud-sonj6 soak: SPLIT — drift PASS on existing evidence; full-duration -> hud-5kq8k (LAN-local rerun)

## Still needs owner
- hud-tlx5c: profile-swap reskin — needs eyes on physical display (token A/B grey->black already confirmed today)
- hud-t2k55 (P3): OS-injection resize (§6b.7) — needs SendInput path with owner watching (livelock risk)
- hud-qfyfg: owner-gated promotion decision — visual-frame axis clean, cadence within budget
