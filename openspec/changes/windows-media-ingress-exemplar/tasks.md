# Tasks — Windows Media Ingress Exemplar

No implementation begins until this change is reviewed and accepted.
No implementation begins until `windows-first-performant-runtime` release/performance criteria are met or maintainers record an explicit exception for this one-stream Windows-only media slice.

## 1. Contract and review

- [ ] 1.1 Validate this OpenSpec change with `openspec validate windows-media-ingress-exemplar --strict`
- [ ] 1.2 Review doctrine alignment against `about/heart-and-soul/v1.md`, `architecture.md`, `security.md`, `privacy.md`, and `attention.md`
- [ ] 1.3 Confirm the final scope keeps audio, mobile/glasses, multi-feed, browser-surface, and embodied-presence work deferred
- [ ] 1.4 After acceptance, update `about/heart-and-soul/v1.md`, `about/heart-and-soul/media-doctrine.md`, and the media spec deferral blocks with a narrow pointer to this Windows-only exception

## 2. Runtime configuration and media zone

- [ ] 2.1 Add explicit Windows media-ingress enablement config
- [ ] 2.2 Add one approved `media-pip` zone with `VideoSurfaceRef` acceptance and fixed content-layer geometry
- [ ] 2.3 Reject media ingress to all non-approved zones
- [ ] 2.4 Require content classification for media admission
- [ ] 2.5 Add canonical `media_ingress` capability grants for authenticated resident/local producers only

## 3. Synthetic media render path

- [ ] 3.1 Wire synthetic `VideoFrame` production into the runtime/compositor media surface path
- [ ] 3.2 Upload decoded RGBA frames to runtime-owned `wgpu` textures
- [ ] 3.3 Render active `VideoSurfaceRef` frames clipped to the media zone geometry
- [ ] 3.4 Preserve deterministic placeholder rendering before first frame and after teardown
- [ ] 3.5 Add headless validation for frame changes, placeholder behavior, clipping, and teardown

## 4. Session protocol and admission

- [ ] 4.1 Wire `MediaIngressOpen` admission through the session server
- [ ] 4.2 Emit `MediaIngressOpenResult`, `MediaIngressState`, and `MediaIngressCloseNotice`
- [ ] 4.3 Enforce explicit enablement, capability, privacy, schedule, stream-count, and budget gates
- [ ] 4.4 Reject audio-bearing or bidirectional requests deterministically
- [ ] 4.5 Add synthetic validation for second-stream rejection, policy denials, lease revocation, reconnect snapshots, and disabled-gate responses

## 5. Windows live producer and YouTube source evidence

- [ ] 5.1 Perform and record YouTube policy/feasibility review before any raw-frame bridge is attempted
- [ ] 5.2 Build a Windows local producer using a self-owned/local video source and bridge its video-only output to the HUD media ingress path
- [ ] 5.3 Build a sidecar/source-evidence runner that launches YouTube video ID `O0FGCxkHM-U` through an official embed/player path
- [ ] 5.4 Keep audio out of the HUD runtime for this tranche
- [ ] 5.5 Add a live `/user-test` script or exemplar runner that verifies HUD presentation with the self-owned/local source and records YouTube sidecar evidence separately

## 6. Evidence, soak, and closeout

- [ ] 6.1 Record live Windows validation evidence under `docs/reports/`
- [ ] 6.2 Run a 10-minute Windows media soak and capture frame timing, dropped frames, CPU/GPU/memory, and teardown behavior; treat performance metrics as record-only until a follow-up budget is approved
- [ ] 6.3 Create follow-up beads for any performance, policy, or Windows dependency gaps
- [ ] 6.4 Reconcile implementation against every requirement in this change before archive/sync

## 7. Beads handoff graph

- [ ] 7.1 Create one epic and six child beads for config, render path, protocol/admission, producer/source evidence, validation/report, and gen-1 reconciliation
- [ ] 7.2 Add dependency edges so validation/report waits on implementation children and terminal reconciliation waits on all implementation and evidence children
- [ ] 7.3 Verify the graph with `bd show`, `bd dep tree`, and `bd ready`
