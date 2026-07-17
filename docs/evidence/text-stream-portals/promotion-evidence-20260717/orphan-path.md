# Governance — Orphan Path Live Observation

Deliberate client idle after attach; cooperative projection adapter; tzehouse reference
host; exe sha256 `26bedaca…`; 2026-07-17 (times local SGT).

| t | Event | Evidence |
|---|---|---|
| 22:18:10 | Re-attach (`continuity_replayed_count: 5`), then client goes fully idle | task log |
| +9 s | Portal live and rendering: two-pane chrome, `Active · 1 unread`, replayed transcript | `shots/orphan-t0-live.png` |
| +105 s | Portal **entirely removed** — no stale marker, no degraded treatment, no residual chrome | `shots/orphan-t95-stale-window.png` |
| +311 s | Still removed; desktop unchanged | `shots/orphan-t300-removed.png` |
| +311 s | `get_pending_input` → `PROJECTION_NOT_FOUND` with actionable reattach hint | task log |

## Reading

**Pass (lease governance):** the governed surface is removed under lease rules after the
liveness gap; post-removal attach starts a fresh portal (verified repeatedly this
session); removal never left stale content presented as live; the NOT_FOUND error is
structured and self-describing.

**Fail (degradation presentation):** the v1-mandatory Stale-Content Degradation +
Disconnect Presentation contracts (`spec.md:600`, `:637`) require a *visible* degraded/
stale window (dimming, stale marker, disconnect affordance) between liveness gap and
grace expiry. None was observed: live at +9 s → absent at +105 s. Either the degraded
phase never renders for cooperative projections, or the default TTL+grace window
(~60 s lease TTL, unconfigured grace) is so short the phase is unobservable. Both are
defects for a primary conversational surface — an agent that isn't continuously
long-polling loses its portal (and any pending operator input, which is memory-only)
between turns, with no on-screen explanation.

Filed/updated: **hud-ccj2o** (agent-liveness degraded treatment + TTL/grace tuning for
conversational cadence) carries this evidence and the repro.
