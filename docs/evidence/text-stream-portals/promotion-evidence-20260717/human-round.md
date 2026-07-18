# Track B — Human Keyboard Round (PARTIAL — session 2026-07-18 ~11:00 SGT)

## Session 1 partial results

- **Row 11 (reply loop): PASS on the wire** — five operator submissions (test/test test/
  shot/…) each delivered through long-poll and acked; runtime delivery-state loop
  verified end-to-end. Operator-side ✓✓ cue rendering not verbally confirmed.
- **Scroll/geometry rows: FAIL — discovered hud-yrcev (P1)**: after operator
  move/resize, backdrop quad covers only the top of the portal; composer region has no
  background (floating caret bar, detached grip, missing header); typed draft likely
  invisible. Screenshot `shots/kbr-scroll-background-bug.png`.
- **Input-loss incident**: one substantive operator message ("backgrounds moving" report)
  never reached the pending queue — most plausibly typed into the invisible composer
  region above. Treat as consequence of hud-yrcev until disproven.
- **hud-ccj2o corroborated live**: the projection was reaped mid-conversation twice in
  ~10 minutes whenever the agent wasn't actively long-polling.
- Remaining rows (1-10 detailed observations, Ctrl+= recovery experiment) not yet
  reported — session 2 needed; portal re-attach restores full context via replay.

Operator: fill one row per check from SESSION-PLAN.md Track B (rows 1-11) with
observed result + pass/fail. Synthetic injection cannot cover these; the gate's
composer-editing live axis and beads hud-pncm3 / hud-2v8br / hud-acfvp / hud-sp8l7 /
hud-vvdvy / hud-gmwuf / hud-pdl1d / hud-2u5j7 close from this table.

Before starting, ask the agent session (or any attached session) to re-attach a demo
portal and hold a long-poll so the portal stays alive and your replies are captured —
note the hud-ccj2o finding: an idle portal is reaped in under 2 minutes.

| Row | Check | Observed | Pass? |
|---|---|---|---|
| 1 | composer soft-wrap + caret | | |
| 2 | Ctrl+Enter newlines, Enter submit | | |
| 3 | history entry wraps + renders newlines (hud-pncm3) | | |
| 4 | overflow keeps newest visible | | |
| 5 | wheel-scroll input history (hud-acfvp) | | |
| 6 | Tab ring visible, typing recovers (hud-2v8br) | | |
| 7 | resize/move, background stays opaque (hud-sp8l7) | | |
| 8 | cursor shapes over affordance bands (hud-gmwuf) | | |
| 9 | motion cadence feel (hud-pdl1d) | | |
| 10 | composer polish sweep (hud-vvdvy) | | |
| 11 | reply → sending → ✓✓ delivered (cooperative input leg) | | |
