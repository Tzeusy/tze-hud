# WebRTC/Media V1 Final Reconciliation Coverage

Date: 2026-04-09
Issue: `hud-nn9d`
Execution brief: `docs/reconciliations/webrtc_media_v1_epic_prompt.md`

## Purpose

Close the epic-level direction loop for WebRTC/media v1 scope by verifying that:

1. The direction decision is explicit.
2. The smallest credible slice and non-goals are explicit.
3. Spec-first decomposition is established before implementation backlog.
4. Human-readable recommendation/signoff artifacts exist.
5. The cited evidence set has explicit reconciliation coverage.

This is a reconciliation artifact. It does not authorize implementation beyond the existing spec-first contract tranche.

## Final Direction Decision

Decision: keep v1 GA media/WebRTC behavior unchanged (deferred), and proceed only with a post-v1 bounded-ingress contract tranche.

- [Observed] v1 doctrine explicitly defers live media/WebRTC and clocked media cues (`about/heart-and-soul/v1.md:112`, `about/heart-and-soul/v1.md:115`, `about/heart-and-soul/v1.md:117`).
- [Observed] Architecture keeps media/WebRTC in the long-term three-plane model (`about/heart-and-soul/architecture.md:27`, `about/heart-and-soul/architecture.md:215`).
- [Observed] Session protocol keeps embodied/WebRTC signaling explicitly post-v1 (`openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:713`).
- [Observed] Runtime kernel keeps media worker pool explicitly post-v1 (`openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:383`).

Conclusion: promote media/WebRTC by contract-first, not by widening v1 truth claims.

## Smallest Credible Slice and Explicit Non-Goals

Smallest credible slice (post-v1 tranche): one-way inbound `VideoSurfaceRef` to approved zone contract, default-off runtime activation gate, no audio, no embodied bidirectional semantics.

- [Observed] Direction recommendation and bounded-slice framing are already captured in `docs/reconciliations/webrtc_media_v1_direction_report.md` and `docs/reconciliations/webrtc_media_v1_human_signoff_report.md`.
- [Observed] Seam inventory formalizes boundary assumptions needed to keep this slice bounded (`docs/reconciliations/webrtc_media_v1_seam_inventory.md`).

Explicit non-goals remain:

- Bidirectional AV/WebRTC session negotiation and embodied presence.
- Audio routing/mixing policy.
- Multi-feed compositing and adaptive bitrate orchestration.

These are tracked as explicit deferred scope markers in backlog decomposition docs (`docs/reconciliations/webrtc_media_v1_backlog_materialization.md`).

## Spec-First Decomposition Status

Decomposition remains spec-first, with implementation creation gated behind renewed signoff and reconciliation.

Current epic child state (from `bd show hud-nn9d --json` on 2026-04-09):

- Closed: direction pass (`hud-nn9d.1`), backlog materialization (`hud-nn9d.2`), human signoff (`hud-nn9d.3`), gen-1 reconciliation (`hud-nn9d.4`), seam inventory (`hud-nn9d.5`), bounded-ingress capability spec (`hud-nn9d.6`).
- In progress: signaling-shape decision (`hud-nn9d.7`).
- Open spec/docs/review gate work: `hud-nn9d.8` through `hud-nn9d.17` (protocol/schema, zone contract, runtime gate, privacy/operator policy, compositor contract, validation thresholds, docs alignment, refreshed signoff, corrected-contract reconciliation).

Reconciliation result:

- [Observed] Missing contract seams are converted into explicit spec/docs beads before implementation beads.
- [Observed] Implementation tranche is intentionally blocked until signoff/reconciliation gates (`hud-nn9d.16`, `hud-nn9d.17`) close.
- [Inferred] The decomposition is now execution-safe and low-churn relative to the original optimistic tranche.

## Evidence-Set Reconciliation Matrix

The execution brief required this minimum evidence set; each item is reconciled below.

| Evidence item | Coverage | Key finding |
|---|---|---|
| `about/heart-and-soul/vision.md` | Covered | Vision remains media-rich long-term, but does not override v1 deferment. |
| `about/heart-and-soul/v1.md` | Covered | Canonical v1 boundary remains no live media/WebRTC. |
| `about/heart-and-soul/architecture.md` | Covered | Three-plane architecture keeps WebRTC/media as target end-state. |
| `about/heart-and-soul/mobile.md` | Covered | Mobile lane preserves WebRTC direction as post-v1 compatibility envelope. |
| `about/heart-and-soul/validation.md` | Covered | Media validation expectations exist; first-slice rehearsal thresholds must be specified before implementation. |
| `about/heart-and-soul/failure.md` | Covered | Degradation semantics imply explicit media fallback policy requirements before activation. |
| `about/heart-and-soul/presence.md` | Covered | Zone media-type and transport ontology requires explicit zone contract/spec seams. |
| `about/legends-and-lore/rfcs/0001-scene-contract.md` | Covered | `VideoSurfaceRef`/`WebRtcRequired` are schema-level post-v1 constructs requiring contract completion for runtime use. |
| `about/legends-and-lore/rfcs/0003-timing.md` | Covered | Arrival-time vs presentation-time invariants and media clock deferment constrain first-slice timing design. |
| `about/legends-and-lore/rfcs/0005-session-protocol.md` | Covered | Embodied/WebRTC signaling remains reserved post-v1; reconnect/snapshot contract must remain coherent. |
| `openspec/.../session-protocol/spec.md` | Covered | Embodied media signaling and reserved fields are deferred; explicit signaling-shape decision is required. |
| `openspec/.../runtime-kernel/spec.md` | Covered | Media worker pool must stay unspawned in v1; activation criteria must be spec-gated. |
| `openspec/.../validation-framework/spec.md` | Covered | Media SSIM and scene-registry hooks exist, but ingress-specific pass/fail rehearsal contract is still required. |
| `crates/tze_hud_runtime` | Covered | Runtime surfaces support tracing/controls but no approved live media activation path in v1. |
| `crates/tze_hud_scene/src/types.rs` | Covered | Zone media/transport placeholders exist (`VideoSurfaceRef`, `WebRtcRequired`) without full runtime contract. |
| `crates/tze_hud_protocol` | Covered | Wire conversion/session handling still contain explicit post-v1 placeholders for media payload fields and metadata parity seams. |

## Acceptance Criteria Traceability (`hud-nn9d`)

| Epic acceptance criterion | Reconciliation verdict | Primary artifacts |
|---|---|---|
| 1) Direction report decides whether/how media belongs in v1 | Met | `webrtc_media_v1_direction_report.md`, this final reconciliation |
| 2) Smallest credible slice + non-goals are explicit | Met | `webrtc_media_v1_direction_report.md`, `webrtc_media_v1_human_signoff_report.md`, `webrtc_media_v1_seam_inventory.md` |
| 3) Missing spec work identified before implementation backlog | Met | `webrtc_media_v1_backlog_materialization.md`, epic children `hud-nn9d.7`-`hud-nn9d.17` |
| 4) Human-readable recommendation/risks/deferred work report | Met | `webrtc_media_v1_human_signoff_report.md` |
| 5) Final reconciliation verifies cited evidence coverage | Met | `webrtc_media_v1_reconciliation_gen1.md`, this final reconciliation |

## What Should Happen Next

1. Finish remaining contract beads (`hud-nn9d.7` through `hud-nn9d.15`) without creating implementation beads early.
2. Complete refreshed human signoff (`hud-nn9d.16`) and corrected-contract reconciliation (`hud-nn9d.17`).
3. Only after those gates close, allow coordinator to materialize implementation tranche.

## Blunt Conclusion

- Real direction: media/WebRTC remains post-v1 in implementation, but is now in a concrete spec-first admission pipeline.
- Work on next: complete signaling/protocol/zone/runtime/privacy/compositor/validation specs and alignment docs.
- Stop pretending: that v1 can claim live media/WebRTC runtime behavior before those contracts and gates are complete.
