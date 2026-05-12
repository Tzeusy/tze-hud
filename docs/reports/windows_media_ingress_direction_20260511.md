# Windows Media Ingress Direction Report

## Executive Summary

tze_hud is a Windows-first agent-native presence engine: the runtime owns pixels, timing, composition, input routing, permissions, attention, privacy, and resource budgets while agents request governed presence. Media/WebRTC is directionally aligned because the doctrine already names live media as part of the long-term architecture, but it was explicitly deferred during the 2026-05-09 Windows refocus.

The current implementation is closer to a reactivation seam than a blank slate. `VideoSurfaceRef`, media ingress proto messages, media admission/state machines, a `VideoSurfaceMap`, and an optional GStreamer appsink pipeline already exist, but the active doctrine/spec surface still says live media is parked and the compositor currently renders only a dark placeholder for `VideoSurfaceRef`.

The highest-priority next work is a narrow OpenSpec change: `windows-media-ingress-exemplar`. It should admit exactly one Windows-only, video-only inbound media stream into one approved runtime-owned media zone. The machine-verifiable HUD proof should use a self-owned/local video source and, after the 2026-05-12 operator/maintainer approval, may also use a Windows-only raw-frame bridge from an operator-visible official YouTube player sidecar for video ID `O0FGCxkHM-U`. It must not revive mobile/glasses, bidirectional AV, audio policy, browser-surface embedding, or embodied presence.

## Project Spirit

**Core problem**: LLMs and agents are trapped in terminals, transcripts, and generated apps; tze_hud gives them safe, governed, performant presence on a real screen.  
**Primary user**: the local operator/viewer on a Windows machine, plus agent developers validating HUD runtime behavior.  
**Success looks like**: multiple governed agents and media surfaces coexist on the Windows overlay without compromising frame timing, privacy, attention, or human override.  
**Trying to be**: a local high-performance display runtime and compositor for agent presence.  
**Not trying to be**: a browser shell, remote desktop, notification spam engine, arbitrary UI framework, or LLM-owned frame loop.

### Requirements

| # | Requirement | Class | Evidence | Status |
|---|---|---|---|
| 1 | Runtime owns pixels, timing, composition, permissions, and safety. | Hard | `about/heart-and-soul/architecture.md:3-11` | Met |
| 2 | Windows is the active deployment target. | Hard | `about/heart-and-soul/v1.md:9-16`, `openspec/changes/windows-first-performant-runtime/specs/windows-runtime-scope/spec.md` | Met |
| 3 | Live media/WebRTC is currently deferred and requires a fresh proposal. | Hard | `about/heart-and-soul/v1.md:123-129` | Unmet for requested feature |
| 4 | Media should decode as compositor surfaces, not widgets or browser DOM. | Hard | `about/heart-and-soul/architecture.md:68-80`, `:115-117` | Partially scaffolded |
| 5 | First media slice is one inbound visual stream, runtime-owned surface, default-off. | Hard | `openspec/specs/media-webrtc-bounded-ingress/spec.md:23-31` | Deferred |
| 6 | Audio, bidirectional AV, multi-feed orchestration, and embodied presence remain out of first slice. | Non-goal | `openspec/specs/media-webrtc-bounded-ingress/spec.md:35-55` | N/A |
| 7 | Protocol already reserves media ingress messages and `VideoSurfaceRef`. | Hard | `crates/tze_hud_protocol/proto/session.proto:1087-1157`, `crates/tze_hud_protocol/proto/types.proto:266-286` | Scaffolded |
| 8 | Compositor currently renders `VideoSurfaceRef` as a placeholder, not decoded frames. | Hard | `crates/tze_hud_compositor/src/renderer.rs:5600-5612`, `:5798-5810` | Gap |

### Contradictions

- [Observed] Doctrine/spec says all media/WebRTC is deferred indefinitely, but code now contains proto messages, runtime state machines, compositor state, tests, and a GStreamer feature flag for media ingress.
- [Observed] The deferred media spec defines a bounded first slice, while the current active Windows refocus says media-plane work is parked. The user now wants this surface reactivated for the Windows machine, so the correct move is a fresh Windows-only OpenSpec proposal rather than implementing against the parked deferral blocks.
- [Observed] The YouTube exemplar creates a product and policy constraint: YouTube should be embedded through official player surfaces, not downloaded, ripped, or silently separated from the player experience. The 2026-05-12 approval permits a Windows-only video-frame bridge from that official player sidecar into the HUD media ingress path, while keeping the self-owned/local stream as the independent baseline proof.

## Current State

| Dimension | Status | Summary | Key Evidence |
|---|---|---|---|
| Spec adherence | Weak | Media specs exist but are explicitly parked. | `openspec/specs/media-webrtc-bounded-ingress/spec.md:1-5` |
| Core workflows | Missing | No live media admission-to-render workflow is active. | renderer placeholder branches |
| Test confidence | Adequate for scaffolds | State machines and placeholder rendering exist; real decode/live Windows path lacks acceptance evidence. | `crates/tze_hud_runtime/tests/media_ingress_proptest.rs`, `renderer.rs:14460-14545` |
| Observability | Partial | Media state/audit concepts exist, but live pipeline observability is not wired end to end. | `crates/tze_hud_runtime/src/media_admission.rs` |
| Delivery readiness | Weak | GStreamer is optional and platform dependency handling needs Windows validation. | `crates/tze_hud_runtime/Cargo.toml:13-21` |
| Architectural fitness | Adequate | The architecture supports media surfaces if kept bounded. | `architecture.md:39-42`, `:115-117` |

The architecture is fit for a one-stream Windows vertical slice because the scene/proto/compositor vocabulary already anticipates it. It is not fit for broad media revival yet: browser surfaces, audio, multi-feed scheduling, mobile profiles, and embodied presence would cross several deferred boundaries at once.

## Alignment Review

### Aligned next steps

| Item | Alignment | Value | Leverage | Tractability | Timing | Risk | Churn |
|---|---|---|---|---|---|---|---|
| Reactivate bounded Windows media ingress spec | Core | High | High | Needs spec | Now, after prerequisite/exception | Medium | Low |
| Wire synthetic media source to real `VideoSurfaceRef` rendering | Core | High | High | Ready after spec | Soon | Medium | Medium |
| Add Windows GStreamer/WebRTC producer path | Core | High | High | Needs architecture | Soon | High | Medium |
| Add YouTube official-player source evidence and approved bridge | Supporting | High | Medium | Policy approval recorded 2026-05-12; bridge implementation still needed | Soon | High | Low |
| Add live Windows user-test | Core | High | Medium | Blocked by implementation | Later | Medium | Low |

### Misaligned directions

- Embedding a full browser surface directly in the compositor for YouTube playback. This contradicts the "not a browser shell" and "native GPU pipeline" doctrine and would bypass the runtime's media surface contract.
- Downloading or extracting YouTube media streams as the first acceptance path. Raw frame bridging is now approved only from an operator-visible official player sidecar into the HUD media ingress contract; it must not become download/extraction/caching or a compositor browser surface.

### Premature work

- Multi-stream media layouts, audio routing, recording, cloud relay, and bidirectional AV. These are real future surfaces but would turn this into a V2 revival instead of a Windows exemplar.
- Dynamic media orchestration by agents. The first slice should use one configured approved media zone.

### Deferred

- Clocked subtitle/cue synchronization against live media clocks. Revisit after one video stream can render with stable timing and teardown.
- Hardware decode optimization. Start with correctness and bounded frame upload; optimize once baseline artifacts exist.

### Rejected

- Agent-owned GPU textures or renderer plugins. Agents can request media ingress; they cannot own compositor resources or inject renderer code.

## Gap Analysis

### Blockers

| Gap | Why it matters | Who | Evidence | Response | Effort |
|---|---|---|---|---|---|
| Active spec still defers media. | Implementation would violate source of truth. | Maintainers | `v1.md:123-129` | OpenSpec change first. | M |
| No end-to-end admission-to-frame path. | The exemplar cannot show real video. | Operator/developers | renderer placeholder branches | Wire runtime media source to compositor texture upload. | L |
| Windows producer path undefined. | HUD media needs a lawful supported source path and YouTube needs policy-safe evidence. | Operator | YouTube IFrame docs, repo has no producer | Specify self-owned/local producer for baseline HUD ingress and official-player sidecar for the approved YouTube bridge. | L |
| Governance gate is not active end to end. | Media can leak sensitive content or ignore override. | Operator/viewers | media policy specs deferred | Wire explicit enablement, capability, privacy, and operator disable. | M |

### Important Enhancements

| Gap | Why it matters | Who | Evidence | Response | Effort |
|---|---|---|---|---|---|
| GStreamer Windows dependency bootstrap. | Live decode validation needs reproducible install/build. | Developers | `Cargo.toml:13-21`, `gst_decode_pipeline.rs:8-14` | Add Windows bootstrap docs/scripts and CI probe. | M |
| Telemetry for frame drops and decode health. | Debugging media without screenshots requires structured data. | Agents/developers | media state proto has health fields | Emit machine-readable media state. | M |

### Strategic Gaps

| Gap | Why it matters | Who | Evidence | Response | Effort |
|---|---|---|---|---|---|
| No browser-surface policy. | Future web media surfaces need a separate contract. | Maintainers | architecture rejects browser host as renderer | Keep out of this change; open separate proposal later. | XL |
| No audio privacy/routing policy. | YouTube includes audio but first slice must be silent/video-only. | Viewers | bounded ingress non-goals | Reject audio in this tranche. | L |

## Work Plan

### Immediate alignment work

### Chunk 1: Approve Windows media ingress scope

**Objective**: Establish the narrow Windows-only contract for one video-only inbound media stream.
**Spec reference**: `openspec/changes/windows-media-ingress-exemplar/`
**Dependencies**: none
**Why ordered here**: Implementation is currently forbidden by active deferral language.
**Scope**: M
**Parallelizable**: No — this defines the contract for all later work.
**Serialize with**: all media implementation chunks

**Acceptance criteria**:
- [ ] OpenSpec proposal, design, delta specs, and tasks exist.
- [ ] The change explicitly keeps mobile, audio, multi-feed, browser-surface, and embodied scope deferred.
- [ ] `openspec validate windows-media-ingress-exemplar --strict` passes.

### Chunk 2: Configure an approved media zone

**Objective**: Add a Windows runtime configuration path for a single approved media zone accepting `VideoSurfaceRef`.
**Spec reference**: `windows-media-ingress-exemplar`, `scene-graph`, `configuration`
**Dependencies**: Chunk 1
**Why ordered here**: Runtime admission and render paths need a canonical zone identity.
**Scope**: M
**Parallelizable**: Partly — after Chunk 1, can run alongside synthetic render wiring if file ownership is separated.
**Serialize with**: protocol/admission changes touching zone registry.

**Acceptance criteria**:
- [ ] Zone uses content layer and configured geometry.
- [ ] Non-approved zones reject media ingress.
- [ ] Missing privacy classification rejects media publication.

### Near-term delivery work

### Chunk 3: Render synthetic decoded frames

**Objective**: Replace placeholder-only `VideoSurfaceRef` rendering with real frame upload for a synthetic source.
**Spec reference**: `windows-media-ingress-exemplar`, `runtime-kernel`, `validation-framework`
**Dependencies**: Chunks 1-2
**Why ordered here**: Synthetic frames prove compositor correctness before live WebRTC/YouTube complexity.
**Scope**: L
**Parallelizable**: No — touches shared compositor/runtime media interfaces.
**Serialize with**: all frame upload/render path work.

**Acceptance criteria**:
- [ ] Headless test shows changing synthetic frames in the approved media zone.
- [ ] Placeholder remains deterministic before first frame and after teardown.
- [ ] Lease revoke/operator disable clears media within one frame.

### Chunk 4: Wire media ingress protocol and admission

**Objective**: Make `MediaIngressOpen`/state/close messages drive the approved zone lifecycle.
**Spec reference**: `session-protocol`, `media-webrtc-bounded-ingress`, `media-webrtc-privacy-operator-policy`
**Dependencies**: Chunks 1-3
**Why ordered here**: Protocol should drive an already-proven rendering path.
**Scope**: L
**Parallelizable**: Limited — can parallelize tests/docs but not shared protocol path.
**Serialize with**: session server mutation work.

**Acceptance criteria**:
- [ ] Valid open admits one stream and publishes `VideoSurfaceRef`.
- [ ] Second stream is rejected deterministically.
- [ ] Audio-bearing ingress is rejected.
- [ ] State and close notices are emitted.

### Chunk 5: Build Windows producer and YouTube source evidence

**Objective**: Demonstrate HUD media ingress through a self-owned/local producer and demonstrate the requested YouTube video through an official-player sidecar.
**Spec reference**: `windows-media-ingress-exemplar`
**Dependencies**: Chunks 1-4
**Why ordered here**: Live exemplar should sit on proven runtime contracts.
**Scope**: L
**Parallelizable**: Yes after protocol stabilizes — producer can live in scripts/examples.
**Serialize with**: any protocol schema changes.

**Acceptance criteria**:
- [ ] Producer emits video-only stream/frames from a self-owned/local source to the local Windows HUD runtime.
- [ ] Sidecar/source-evidence runner uses the YouTube embed/player path for video ID `O0FGCxkHM-U`.
- [ ] YouTube frame bridging uses the 2026-05-12 approved official-player sidecar path and enters the HUD only through media ingress.
- [ ] No YouTube download/ripping path is introduced.
- [ ] User-test evidence shows the local video source in the HUD media zone and separately records YouTube source evidence.

### Strategic future work

### Chunk 6: Windows media soak and release evidence

**Objective**: Validate sustained media rendering on the reference Windows machine.
**Spec reference**: `windows-media-ingress-exemplar`, `validation-framework`
**Dependencies**: Chunks 1-5
**Why ordered here**: Soak only matters once the vertical slice works.
**Scope**: M
**Parallelizable**: No — requires integrated artifact.
**Serialize with**: release tagging.

**Acceptance criteria**:
- [ ] 10-minute media soak records frame timing, decode health, dropped frames, texture memory, CPU/GPU.
- [ ] Regression report is attached under `docs/reports/`.
- [ ] Follow-up beads are created for any budget gaps.

## Do Not Do Yet

| Item | Reason | Revisit when |
|---|---|---|
| Audio playback/routing | Requires household-aware audio policy and mute semantics. | Video-only ingress is stable and operator controls are proven. |
| Multi-feed layouts | First stream is not proven yet. | One-stream soak passes with low frame/texture overhead. |
| Browser surface node | Contradicts current renderer/browser boundary. | Separate browser-surface proposal is approved. |
| Mobile/glasses media | Current target is Windows. | Windows media path is released and porting is explicitly reopened. |
| Embodied presence | Too broad; would revive deferred V2 scope. | Separate identity/role/media contract is approved. |

## Appendix

### A. Repository Map

- `crates/tze_hud_protocol/proto/`: media ingress and `VideoSurfaceRef` schema placeholders.
- `crates/tze_hud_scene/src/types.rs`: zone media types and `ZoneContent::VideoSurfaceRef`.
- `crates/tze_hud_runtime/src/media_ingress.rs`: media session state machine.
- `crates/tze_hud_runtime/src/media_admission.rs`: media capability/budget gate scaffolding.
- `crates/tze_hud_runtime/src/gst_decode_pipeline.rs`: optional GStreamer appsink pipeline.
- `crates/tze_hud_compositor/src/video_surface.rs`: compositor media surface state and synthetic pipeline.
- `crates/tze_hud_compositor/src/renderer.rs`: placeholder render path and test.

### B. Critical Workflows

1. Synthetic media: config enables media -> approved zone exists -> synthetic source produces frames -> compositor uploads and renders -> teardown clears.
2. Protocol media: resident agent opens media ingress -> admission gate checks policy -> `VideoSurfaceRef` published -> state messages emitted.
3. Live producer: local/self-owned video source -> authenticated producer sends video-only media -> HUD renders governed media zone.
4. YouTube source evidence and bridge: source sidecar embeds video ID `O0FGCxkHM-U` through official player path -> report records player state, approved bridge path, and whether video frames reached HUD through `MediaIngressOpen`.
5. Override: operator disables/dismisses/safe mode -> media stops within one frame -> no auto-resume.

### C. Spec Inventory

- `media-webrtc-bounded-ingress`: deferred but directly reusable for one-stream envelope.
- `media-webrtc-privacy-operator-policy`: deferred but directly reusable for enablement/operator/privacy.
- `scene-graph`: currently marks `VideoSurfaceRef` deferred.
- `session-protocol`: already has media message definitions.
- `runtime-kernel`: currently reserves/deactivates media worker pool.
- `validation-framework`: needs synthetic and live Windows media lanes.

### D. Evidence Index

- `about/heart-and-soul/v1.md:9-16`, `:123-141`, `:174-184`
- `about/heart-and-soul/architecture.md:3-11`, `:17-42`, `:68-80`, `:115-117`, `:158-172`
- `openspec/specs/media-webrtc-bounded-ingress/spec.md:1-221`
- `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
- `crates/tze_hud_protocol/proto/session.proto:1048-1195`
- `crates/tze_hud_protocol/proto/types.proto:258-286`
- `crates/tze_hud_runtime/Cargo.toml:13-21`
- `crates/tze_hud_runtime/src/gst_decode_pipeline.rs:1-145`
- `crates/tze_hud_compositor/src/video_surface.rs:1-165`, `:443-520`
- `crates/tze_hud_compositor/src/renderer.rs:5596-5620`, `:5794-5812`, `:14460-14545`
- YouTube IFrame Player API: official embed/player control surface.
- YouTube API Services Developer Policies: policy-sensitive source handling, user control, API client transparency, and compliance obligations.

### E. Reconciliation Passes

R1 doctrine/project-shape review found the initial draft structurally valid but not acceptable until it recorded the Windows-first prerequisite or explicit exception, anchored the narrow media-deferral exception, named authenticated producer identity, and added privacy/attention requirements.

R2 spec-spine review found the initial deltas too thin. The change now adds `Source:`/`Scope:` traceability, a `configuration` delta, stricter `media-pip` zone semantics, active-vs-deferred session protocol wording, timing/lease/reconnect/budget requirements, deterministic audio rejection, decode dependency behavior, and explicit validation artifacts.

R3 scheduling review found the original tasks were a checklist rather than an execution graph. The handoff now uses one epic and six child beads: config, synthetic rendering, protocol/admission, producer/source evidence, validation/report, and terminal gen-1 reconciliation.

R4 handoff review found the YouTube acceptance path was the main weak assumption. The final scope separates YouTube official-player source evidence from baseline machine-verifiable HUD frame-ingress proof. As of 2026-05-12, raw YouTube frame bridging is policy-approved for a Windows-only official-player sidecar that feeds video frames through `MediaIngressOpen`; it remains constrained by the no-download, no-cache, no-audio, no-compositor-browser boundaries.

### F. Beads Handoff Graph

Epic: `hud-gog64` — `Windows media ingress exemplar`

Child graph:

1. `hud-gog64.1` — `Configure approved Windows media zone`; depends on OpenSpec acceptance. Owns `configuration`, `scene-graph`, and runtime config for default-off `media-pip`.
2. `hud-gog64.2` — `Render synthetic VideoSurfaceRef frames`; depends on `hud-gog64.1`. Owns compositor/runtime upload, placeholder, clipping, and teardown proof.
3. `hud-gog64.3` — `Wire media ingress admission protocol`; depends on `hud-gog64.1` and `hud-gog64.2`. Owns `MediaIngress*`, capability, privacy, stream-count, timing, lease, reconnect, budget, and audit gates.
4. `hud-gog64.4` — `Build Windows media producer and YouTube source evidence`; depends on `hud-gog64.3`. Owns self-owned/local producer, YouTube official-player sidecar, and policy review record.
5. `hud-gog64.5` — `Validate Windows media ingress and write report`; depends on `hud-gog64.2`, `hud-gog64.3`, and `hud-gog64.4`. Owns synthetic gate, live Windows HUD proof, YouTube source-evidence artifact, and 10-minute record-only soak.
6. `hud-gog64.6` — `Reconcile Windows media ingress gen-1 closeout`; depends on `hud-gog64.1` through `hud-gog64.5`. Owns spec-to-code reconciliation, follow-up beads, and archive-readiness assessment.

Verification: `bd dep cycles --json` returned no cycles, and `bd ready --json --limit 0` exposes `hud-gog64.1` as the first implementation child.

---

## Conclusion

**Real direction**: tze_hud should reactivate media as a narrow Windows-owned compositor surface, not as a broad V2/media/browser revival.

**Work on next**: approve `windows-media-ingress-exemplar`, resolve the Windows-first prerequisite/exception, render synthetic `VideoSurfaceRef` frames, then build the Windows local producer and YouTube source-evidence sidecar.

**Stop pretending**: the repo can ship "WebRTC/media" generically before the Windows one-stream admission, render, governance, and validation loop works.
