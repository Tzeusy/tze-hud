# Text Stream Portal Phase-1 Promotion Gate — Re-Score

Date: 2026-07-04
Issue: `hud-qfyfg`
Decision: **FAIL — do not promote yet**
Decision owner: owner-gated; this report records the machine re-assessment only.
Supersedes: `docs/reports/hud-qfyfg-text-stream-portal-phase1-promotion-gate-20260619.md`
(2026-06-19, also FAIL).

## Why re-score

The 2026-06-19 assessment failed on six open items. Bead notes since then
suggested the multi-adapter axis had become re-scorable after `hud-utbiy`
(cooperative render) and `hud-vewoz` (external attach live-verify) closed. This
report re-scores against all evidence that accumulated **after 2026-06-19**:
the `liveverify-20260703-*` rounds, the `soak-20260616/17-*` runs, the
`usertest-20260620-175021` cadence run, and the `vewoz-liveverify-20260621` /
`liveverify-20260621-205600` screenshot sets.

## Decision

Promotion fails closed. The raw-tile pilot remains the authoritative portal
scope. No open item reached a clean PASS on post-2026-06-19 evidence.

The failure shape has **changed** since 2026-06-19: the cooperative-render axis
moved from a hard FAIL to a partial (functional) recovery, but a same-day
regression plus missing cadence/soak/governance evidence keep the gate closed.
Most of the remaining gaps share **one systemic root cause** (see below).

## Re-scored criteria (RFC 0013 §7.2)

| Criterion | 2026-06-19 | 2026-07-04 | Basis |
|---|---|---|---|
| Recurs across multiple adapters | FAIL | **PARTIAL** | Cooperative render recovered — a human operator typed 595 keystrokes and submitted into a painted portal (`liveverify-20260703-round4/transcript.jsonl`). But the empty-tile failure **recurred the same day** (`liveverify-20260703-011800`: all 6 zones `has_content=false`, all elements `last_published_at_ms=0`, compositor frame loop exited) and there is **no fresh screenshot** of the fixed state. Not durable. |
| Raw-tile expression creates repeated complexity | PASS (not sufficient) | **PASS (not sufficient)** | Unchanged. The six-tile assembly + split mutation patterns are real complexity. |
| Governance requirements stable | PARTIAL/FAIL | **NO live evidence (FAIL for promotion)** | No live redaction/safe-mode/freeze/orphan run post-cutoff. `liveverify-20260621-205600` is a connection disconnect/resume flow, not governance. |
| Subordinate to presence thesis | PASS (owner-gated) | **PASS (owner-gated)** | Unchanged. Content-layer, lease-governed, below chrome. |
| No terminal semantics | PASS | **PASS** | Unchanged. Boundary held at bounded text streams + runtime-owned draft. |

## Evidence-quality gates

| Gate | Verdict | Basis |
|---|---|---|
| Cadence within 16.6 ms runtime-overhead budget | **FAIL (worse, and still a proxy)** | No non-proxy re-run exists. Only new data (`usertest-20260620-175021/transcript.json`) is a proxy (`present_latency ≈ rtt`; overhead = latency − a static 40.5 ms RTT baseline) and regressed to **11/20 over budget, p95 67.2 ms, max 197.3 ms** (was 5/20, p95 21, max 56). Axis is architecturally stuck as a proxy until real present-ack lands (`hud-vjlqh`). |
| 60-min reference-host soak, ≤ 5 MiB drift | **FAIL (no completed run)** | Best run `soak-20260617-163653` ran **608 s**, not 3600 s — the lease was granted `ttl=600000ms` and expired mid-run (`Mutation batch rejected — lease expired`). `soak-complete.marker` fired on lease death, not completion. Memory swing ~130 MiB is an artifact of rendering stopping at expiry, not a clean drift signal. The four `soak-20260616-*` runs are 1–5 min / aborted. |
| Human visual sign-off | **PARTIAL** | Screenshots existed (`vewoz-liveverify-20260621/portal-content-painted.png`, `liveverify-20260621-205600/lv-{1,2,3}-*.png`) but all dated to **2026-06-21** — predating both the fix and the 07-03 regression. No formal sign-off document. _(These full/partial-desktop captures were removed 2026-07-11 for operator-environment privacy — see [hud-ryawj]; git history retains them.)_ |
| Owner promotion approval | **ABSENT** | Not present in issue notes, evidence, or PR metadata. |

## Systemic root cause: portal lease self-termination (`hud-hk8kl`, P1)

Portal leases are **not renewed on long-lived/soak paths**, so sustained runs
self-terminate at the lease TTL. This single defect is the proximate cause of
three separate gate-axis failures above:

1. **Soak** died at the 600 s lease TTL instead of running 3600 s.
2. **Render durability** regressed (`liveverify-20260703-011800`): compositor
   frame loop exited, `PRESENT-WATCHDOG: scene try_lock missed 120 consecutive
   frames`, `Mutation batch rejected — lease expired`.
3. **Long cadence** runs cannot sustain past the TTL for the same reason.

Fixing `hud-hk8kl` makes the soak (60-min reachable), render durability
(sustained render), and long-cadence runs **runnable** — it is the highest-
leverage next step and is now wired as a blocker of `hud-qfyfg`.

## Required follow-up before re-scoring can pass

1. **`hud-hk8kl` (P1, NEW, blocks qfyfg)** — fix portal lease renewal so
   sustained paths do not self-terminate.
2. Re-run and archive a **60-min reference-host soak** proving ≤ 5 MiB drift on
   the fixed build (unblocked by #1).
3. Capture **fresh cooperative-render pixel proof** on the fixed build to
   convert the multi-adapter axis from PARTIAL to PASS (durability).
4. Resolve cadence via real present-ack (`hud-vjlqh`) or an explicit owner
   waiver — re-running the proxy cannot pass this axis.
5. Run and archive **live governance** confirmation (redaction, safe-mode,
   freeze, orphan/grace).
6. Collect **human visual sign-off** on the fixed build, or record an owner
   waiver.
7. **`hud-5wos2` (P2, NEW)** — fix `soak-complete.marker` so it does not
   false-pass on lease-death termination (prevents future false gate signals).

This is a machine re-assessment, not a promotion approval. The owner should
treat the gate as still FAIL.
