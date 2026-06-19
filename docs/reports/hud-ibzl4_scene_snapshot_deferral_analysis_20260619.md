# hud-ibzl4 Scene Snapshot Deferral Analysis

Issue: `hud-ibzl4`
Date: 2026-06-19
Decision: blocked; do not implement double-buffered scene snapshot yet

## Question

`hud-ibzl4` asks whether the runtime should implement a double-buffered scene
snapshot for the Stage 4 commit path to de-risk the shared
`Arc<Mutex<SceneGraph>>` at 10x scale.

The issue was explicitly deferred by `hud-3qpgv.2` until
`scene_lock_miss_count` data accrued. The right first step is therefore to
inspect the telemetry evidence, not to start a broad scene ownership rewrite.

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

## Interpretation

The available telemetry is sufficient to prove the counter wiring works, but it
is not sufficient to justify a double-buffered scene snapshot implementation.

The current evidence says:

- Production-shaped gated sessions: `scene_lock_misses = 0`.
- Synthetic worst-case contention: non-zero by construction and saturated at
  1.0 miss/frame.
- No measured production-shaped concurrent gRPC/MCP mutation distribution exists
  yet.

A double-buffered scene snapshot would touch the core Stage 4 commit ownership
model. That path is constrained by the runtime-kernel budget:

- Stage 4 scene commit p99 must stay under 1 ms.
- Commit of 10 mutations must stay under 50 us at the scene-graph layer.
- After commit, the runtime must publish the updated hit-test snapshot via
  ArcSwap.
- Hot-connect snapshots must be delivered from network threads without blocking
  the compositor.

Without production-shaped miss data, changing that ownership model would be a
speculative architecture change rather than a risk-scaled implementation.

## Recommendation

Do not implement `hud-ibzl4` yet.

Resume this issue only after one of these unblock conditions is met:

1. A production-shaped contention benchmark or live-runtime run records a
   non-zero `scene_lock_misses` distribution under realistic concurrent
   gRPC/MCP mutation-handler load, with enough samples to set an explicit
   healthy/concerning ceiling.
2. `hud-pio04` or an equivalent follow-up closes with a data-justified bounded
   contention model and a documented target ceiling.
3. The coordinator explicitly re-scopes `hud-ibzl4` from implementation to a
   design/spec spike for double-buffered scene ownership, with acceptance
   criteria that do not depend on the missing telemetry distribution.

Until then, the safest state is to keep the existing zero baseline gate for
production-shaped sessions and leave the synthetic contention scenario
informational.

## Reproduction Pointers

Read-only commands used for this analysis:

```bash
bd show hud-ibzl4 --json
bd show hud-3qpgv.2 --json
bd show hud-iky7b --json
gh pr view 781 --json number,title,state,url,body,comments,reviews,commits,mergeCommit,headRefName,baseRefName --repo Tzeusy/tze-hud
gh pr list --repo Tzeusy/tze-hud --search "scene_lock_misses OR scene_lock_contention OR scene_lock_miss_count" --state all --json number,title,state,url,mergedAt,body --limit 20
rg -n "scene_lock_contention|scene_lock_misses|scene_lock_miss_count|hud-iky7b|windows-performance-budget" docs about .github scripts examples crates -S --glob '!target/**'
```
