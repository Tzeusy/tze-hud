# Text Stream Portal Phase-1 Promotion Gate — Owner Decision: PROMOTE

Date: 2026-07-05
Issue: `hud-qfyfg`
Decision: **PASS — promote the text-stream portal out of the raw-tile pilot**
Decision owner: **Tzeusy (uniquosity@gmail.com)** — owner promotion approval GRANTED.
Supersedes: `docs/reports/hud-qfyfg-text-stream-portal-phase1-promotion-gate-20260704.md`
(2026-07-04, machine FAIL) and the 2026-06-19 assessment.

## Decision

The owner, after a live sign-off pass on the reference host (tzehouse-windows,
real RTX 3080, overlay mode), **approves promotion**. The blocking visual defect
found during that pass was fixed and re-verified live; the two failure modes the
soak axis actually guards (lease self-termination and memory drift) were proven
good; cadence is within budget. Remaining items are accepted as tracked
follow-ups, not gate blockers (see below).

This is a genuine owner decision, not a machine assessment. It reverses the
2026-07-04 machine FAIL on the basis of evidence produced after that report.

## What changed since the 2026-07-04 FAIL

The 07-04 report failed on six items. During the 2026-07-05 owner live pass
(evidence: `docs/evidence/text-stream-portals/liveverify-signoff-20260705-105700/`)
and a corroborating coordinator reconcile pass
(`docs/evidence/text-stream-portals/liveverify-reconcile-20260705-1716/`):

- **Visual sign-off (the hard blocker):** the portal rendered a **grey frame** on
  the production runtime-token path. Root-caused to the canonical
  `portal.frame.background` default (`#111720`, an opaque slate that painted the
  frame rim grey). Fixed to opaque off-black `#0A0D11` (`hud-a328c`, PR #1075),
  rebuilt, redeployed, and **owner-verified off-black live on the runtime-token
  path**. A second grey in the minimized/collapsed state
  (`portal.collapsed_card.background` `#1A1F28`) was darkened to `#12161C`
  (`hud-0hj7f`, PR #1079). Owner visual-frame sign-off: **OBTAINED.**
- **Composer / interaction:** owner physically typed 45+ keystrokes and submitted
  through the runtime-owned composer; per-char key events, coalesced
  `composer_draft_state`, and `composer_draft_submit` (`input_history_len=2`) were
  all captured — the interaction path is end-to-end functional live.
- **Lease self-termination (`hud-hk8kl`):** fixed and **validated live** — the
  soak ran 57.6 min (3454 s / 13303 cycles) vs the prior 608 s lease-expiry death.
- **Memory drift:** flat across the full run (independent HUD RSS 27–34 MiB, no
  upward trend) — within the ≤5 MiB drift intent.
- **Cadence:** reconcile pass measured runtime overhead mean 0.002 ms, p95
  0.003 ms, `over_budget_count=0` against the 16.6 ms budget — **within budget**.
  The `read_telemetry` grant added during the live pass also unlocks the real
  (non-proxy) present-ack path for future runs (`hud-vjlqh`).

## Re-scored criteria (RFC 0013 §7.2)

| Criterion | 2026-07-04 | 2026-07-05 | Basis |
|---|---|---|---|
| Recurs across multiple adapters | PARTIAL | **PASS (owner)** | Cooperative render recovered and durable; exemplar-script + cooperative + resident paths all exercised; owner-verified live. |
| Raw-tile expression creates repeated complexity | PASS (not sufficient) | **PASS** | Six-tile assembly, split mutation batches, capture tiles, drag shields, minimized-icon state, cleanup — real, recurring complexity that a first-class surface would absorb. |
| Governance requirements stable | FAIL (no live) | **PASS (owner-accepted)** | Redaction/safe-mode/freeze/orphan covered by 9 passing integration tests; orphan/grace live-drivable. Owner accepts integration coverage for Phase-1 promotion. |
| Subordinate to presence thesis | PASS (owner-gated) | **PASS (owner)** | Content-layer, lease-governed, bounded, below chrome. |
| No terminal semantics | PASS | **PASS** | Boundary held at bounded text streams + runtime-owned draft; no PTY/VT/alt-screen/process ownership. |

## Evidence-quality gates

| Gate | Verdict | Basis |
|---|---|---|
| Visual sign-off | **PASS (owner)** | Frame off-black verified live on runtime-token path after #1075; composer interaction verified. |
| Cadence within 16.6 ms budget | **PASS** | Reconcile pass: overhead p95 0.003 ms, `over_budget_count=0`. |
| 60-min soak, ≤5 MiB drift | **ACCEPTED PARTIAL** | Lease-fix + flat memory proven live (57.6 min, 27–34 MiB, no trend). Full 3600 s completion deferred to `hud-5kq8k` (aborted 146 s short on a transient `mutation_result` timeout — `hud-n5bqp`). Owner accepts the partial for promotion. |
| Owner promotion approval | **GRANTED** | Tzeusy, 2026-07-05. |

## Accepted follow-ups (not gate blockers)

- `hud-5kq8k` — clean full-duration (3600 s) soak rerun on a LAN-local path.
- `hud-n5bqp` (P2) — transient `mutation_result` timeout under sustained streaming.
- `hud-3nus3` (P2) — submitted input-pane history not painted (composer geometry/echo).
- `hud-4e6c0` (P3) — exemplar hardcodes minimize/compact control colors (residual grey).
- `hud-tlx5c` — profile-swap reskin owner eyes; `hud-t2k55` (P3) — OS-injection resize.

## Boilerplate

Promotion authorizes the first-class portal surface work (`hud-tc153`) to proceed.
The raw-tile pilot served its purpose as the evidence vehicle. Follow-ups above are
tracked and do not block the Phase-1 promotion the owner has approved.
