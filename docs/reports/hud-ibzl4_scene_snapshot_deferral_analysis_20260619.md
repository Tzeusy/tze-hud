# hud-ibzl4 Scene Snapshot Decision Analysis

Issue: `hud-ibzl4`
Initial analysis: 2026-06-19
Re-evaluation: 2026-07-16
Decision: do not implement the double-buffered scene snapshot from the current evidence

## Question

`hud-ibzl4` asks whether the runtime should implement a double-buffered scene
snapshot for the Stage 4 commit path to de-risk the shared
`Arc<Mutex<SceneGraph>>` at 10x scale.

The issue was explicitly deferred by `hud-3qpgv.2` until
`scene_lock_miss_count` data accrued. The initial analysis below preserved that
deferral because only a saturation scenario existed. This re-evaluation follows
the closure of `hud-pio04` / PR #935 and asks whether its paced contention gate
now supplies enough evidence for the behavior change.

## Evidence Reviewed

1. `hud-3qpgv.2` / PR #781 added the telemetry surface:
   - `FrameTelemetry::scene_lock_miss_count` records a cumulative frame-loop
     `try_lock` miss count.
   - `SessionSummary::scene_lock_misses` tracks the peak running total observed
     across recorded frames.
   - The windowed frame loop increments the counter only when
     `compositor_scene.try_lock()` fails.

2. `docs/audits/20260612_reconciliation_codehygiene_gen1.md` confirms the
   audit finding was covered by instrumentation only. It records the
   double-buffered scene snapshot as an adjacent deferred follow-up, not as work
   completed by the code-hygiene wave.

3. PR #887 and `about/craft-and-care/engineering-bar.md` now gate
   `scene_lock_misses == 0` for the two production-shaped headless Windows
   benchmark sessions:
   - `steady_state_render`
   - `high_mutation`

   The documented rationale is that these sessions are single-threaded and have
   no concurrent gRPC/MCP handler holding the scene lock. Any non-zero value in
   those gated sessions is therefore a regression.

4. `hud-iky7b` / PR #891 added the synthetic `scene_lock_contention` benchmark
   session. Its purpose is only to prove the counter has a real non-zero signal
   through the same `tokio::sync::Mutex::<SceneGraph>::try_lock` path used by
   `windowed.rs`.

5. The downstream `hud-pio04` notes record the most important data point:
   synthetic contention is deterministic 1.0 miss/frame
   (`100 -> 100`, `300 -> 300`, `600 -> 600`, `1200 -> 1200`) while
   `steady_state_render` and `high_mutation` remain `0`. That is a 100 percent
   saturation worst case, not a production-shaped stochastic contention
   distribution.

6. `hud-pio04` / PR #935 subsequently added
   `scene_lock_paced_contention`. It models three resident mutation-handler
   streams at an aggregate 6 Hz. The harness creates six tiles, schedules one
   single-mutation lock hold at frames 5, 15, and 25 of each 30-frame period,
   waits until the holder owns the mutex, and only then attempts the frame-loop
   `try_lock`. For the 180-frame CI run this targets exactly 18 contended frames.
   The accepted counter range is `1..=20` misses; values in that range are
   explicitly classified as healthy.

7. A fresh release-mode run on current `origin/main` (`b619a83c`) reproduced the
   committed model exactly:

   | Session | Frames | `scene_lock_misses` | Gate interpretation |
   |---|---:|---:|---|
   | `steady_state_render` | 180 | 0 | healthy |
   | `high_mutation` | 180 | 0 | healthy |
   | `scene_lock_contention` | 180 | 180 | saturation reference |
   | `scene_lock_paced_contention` | 180 | 18 | healthy (`1..=20`) |

   The overall benchmark verdict was `pass`. The paced session reports zeroed
   frame-latency samples for its lock probe, so it does not establish a latency
   regression caused by the misses.

8. The production seam remains broad. The canonical graph is still shared as
   `Arc<tokio::sync::Mutex<SceneGraph>>`; the current source contains 62 direct
   `lock` / `try_lock` sites matching that handle across runtime, protocol, and
   MCP code (including inline test support). The windowed compositor's Stage 4
   path takes the same mutex, performs expiry and animation maintenance, builds
   a self-contained `WindowedFrameBuild`, refreshes hit regions, drains present
   acknowledgements, then releases the mutex before GPU submission/present.
   The existing ArcSwap seam is deliberately narrow: it publishes only the
   `HitTestSnapshot` required by local feedback.

## Interpretation

The new telemetry is sufficient to prove a bounded paced model and to gate that
model against accidental saturation. It is still not sufficient to justify a
double-buffered scene snapshot implementation for the 10x ceiling.

The current evidence says:

- Non-contended production-shaped sessions remain at zero misses.
- The paced model produces 18 misses because it deliberately synchronizes all 18
  targeted lock holds with a frame attempt; it does not measure the overlap
  probability of a live runtime.
- The accepted 18/180 result is inside the documented healthy range and carries
  no paired missed-frame, staleness, or input-to-present budget failure.
- The model uses three streams, six tiles, and one-mutation batches. It does not
  exercise the audit's 10x shape (up to 30 agents / 240 tiles), where scene size
  and real handler hold times are the relevant unknowns.

A double-buffered scene snapshot would touch the core Stage 4 commit ownership
model. That path is constrained by the runtime-kernel budget:

- Stage 4 scene commit p99 must stay under 1 ms.
- Commit of 10 mutations must stay under 50 us at the scene-graph layer.
- After commit, the runtime must publish the updated hit-test snapshot via
  ArcSwap.
- Hot-connect snapshots must be delivered from network threads without blocking
  the compositor.

The obvious implementation variants also fail the current doctrine/spec gate:

1. **Clone-on-dirty frame snapshot.** Clone `SceneGraph` while holding the
   mutex, release it, then build from the clone. This shortens the lock hold but
   turns a one-node mutation into a full-scene copy. It conflicts with
   `efficiency.md` (work proportional to change; full-scene work for one-node
   diffs is an anti-pattern) and adds an unmeasured allocation/copy cost to the
   Stage 4 `< 1 ms` budget.
2. **Clone-on-writer ArcSwap snapshot.** Publish a cloned graph after every
   protocol/MCP mutation and let the compositor load it lock-free. This moves
   the same full-scene cost onto mutation handlers, expands the change across
   all writer paths, and risks violating the scene-graph commit budget of
   `< 50 us` for 10 mutations.
3. **True front/back graphs with delta replay.** Keep two graphs synchronized by
   replaying every mutation and swap committed ownership. This can be
   change-proportional, but the current code has many non-batch mutations
   (expiry, leases, zones/widgets, local runtime state, resource and present-ack
   drains). Making all of them replayable changes the Stage 3/4 ownership model
   specified in RFC 0002 and requires a design/spec change before code.

The existing path already releases the scene mutex before GPU
submit/present. Given a passing bounded gate and no measured 10x budget breach,
all three variants have more demonstrated risk than the current one-frame stale
fallback.

## Recommendation

Do not implement the double-buffered snapshot under the current acceptance
evidence. This is a completed engineering decision, not an unresolved choice:

> [decision] chose retaining the measured shared-mutex path over speculative
> double buffering: the accepted 18/180 paced result is healthy, does not model
> 10x scene size or establish a latency breach, and the available snapshot
> variants violate work-proportional-to-change or require a spec-level ownership
> redesign. Reversible: yes.

Reopen the structural remedy only when one of these evidence triggers occurs:

1. A live windowed three-agent run correlates `scene_lock_misses` with missed
   presents, staleness, or a locked input/present latency budget failure.
2. A 10x benchmark (30 mutation streams and/or the 240-tile ceiling with
   production-shaped node counts) exceeds the committed miss or latency budget.
3. An OpenSpec/RFC change defines a change-proportional front/back ownership
   model for every canonical mutation class, including compositor-owned expiry,
   resource drains, hit regions, and present acknowledgements.

Until then, keep the current zero baselines, the `1..=20` paced gate, the
saturation reference, and the Stage 4 miss/watchdog telemetry. Do not lower the
paced ceiling merely to force an architectural rewrite; the gate should change
only when observed data justifies it.

## Reproduction Pointers

Read-only commands used for this analysis:

```bash
bd show hud-ibzl4 --json
bd show hud-3qpgv.2 --json
bd show hud-iky7b --json
bd show hud-pio04 --json
git rebase origin/main
target/release/benchmark --frames 180 --emit /tmp/hud-ibzl4-current-180.json
jq '.sessions[] | {name, total_frames: .summary.total_frames, scene_lock_misses: .summary.scene_lock_misses}' /tmp/hud-ibzl4-current-180.json
gh pr view 781 --json number,title,state,url,body,comments,reviews,commits,mergeCommit,headRefName,baseRefName --repo Tzeusy/tze-hud
gh pr view 935 --json number,title,state,url,body,comments,reviews,commits,mergeCommit,headRefName,baseRefName --repo Tzeusy/tze-hud
gh pr list --repo Tzeusy/tze-hud --search "scene_lock_misses OR scene_lock_contention OR scene_lock_miss_count" --state all --json number,title,state,url,mergedAt,body --limit 20
rg -n "scene_lock_contention|scene_lock_misses|scene_lock_miss_count|hud-iky7b|windows-performance-budget" docs about .github scripts examples crates -S --glob '!target/**'
```
