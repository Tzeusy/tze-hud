# Windows Media Ingress Exemplar — Gen-1 Terminal Reconciliation

Date: 2026-07-11
Issue: `hud-gog64.6` (terminal gen-1 reconciliation child of epic `hud-gog64`)
OpenSpec change: `windows-media-ingress-exemplar`
Strict validation: `openspec validate windows-media-ingress-exemplar --strict` → **valid**

## Purpose

Reconcile the `windows-media-ingress-exemplar` OpenSpec change against **every**
ADDED and MODIFIED requirement across its eight delta specs, and against every
task in `tasks.md`, before an archive/sync decision. Each requirement is cited to
concrete code (`file:symbol`), tests, or live validation evidence. Owner-gated
items (the raw-YouTube-frame-bridge cluster) are recorded as **explicit tracked
blockers**, not gaps.

This report closes the reconciliation lane after the media-lane PR reconciliation
(`hud-cvgvg`, 2026-07-11):

- **PR #958** (live Windows validation evidence + harness) — **MERGED** (`79aacc3d`).
- **PR #657** (May-11 blocked-attempt validation report) — **CLOSED obsolete**
  (superseded by merged evidence; branch leaked pre-scrub host identifiers).
- **PR #933** (ingress-rejection test) — **CLOSED obsolete**; its novel test was
  salvaged and merged as **PR #1123** (`71bfaa00`,
  `media_ingress_operator_disabled_rejects_authenticated_open`).
- **PRs #658 / #659** (approved YouTube bridge lane / frame-capture adapter) —
  **OPEN, PARKED** on an owner policy decision (see §Tracked Blockers).

Evidence in this report uses placeholder host identifiers only
(`windows-host.example`, `198.51.100.125`); no real host/IP is quoted.

## Scope of the change

One Windows-only, video-only inbound media stream into one runtime-owned
`media-pip` zone, admitted only behind explicit enablement / capability /
privacy / operator / budget gates. Audio, bidirectional AV, multi-feed,
mobile/glasses, recording, cloud relay, browser-surface nodes, and YouTube
download/extraction are all out of scope (design.md §1).

Two source-of-truth admission surfaces exist and agree:
- **Wire-active path** — `crates/tze_hud_protocol/src/session_server/media.rs`
  (the messages actually served on the gRPC session stream today).
- **RFC-canonical model** — `crates/tze_hud_runtime/src/media_ingress.rs`
  (RFC 0014 state machine) + `crates/tze_hud_runtime/src/media_admission.rs`
  (RFC 0002/0009 activation gate + audit sink).

## Requirement-to-evidence reconciliation

### configuration (ADDED)

| # | Requirement | Status | Evidence (file:symbol) |
|---|---|---|---|
| 1 | Windows Media Ingress Configuration | **SATISFIED** | `crates/tze_hud_scene/src/config/mod.rs::MediaIngressConfig` — `#[derive(Default)]` with `enabled:false`, plus `approved_zone`, `zone_geometry` (runtime-owned `GeometryPolicy`), `max_active_streams`, `default_classification`, `operator_disabled`. Fail-closed TOML validation/resolve: `crates/tze_hud_config/src/media_ingress.rs::validate_media_ingress` (requires `approved_zone==media-pip`, `max_active_streams==REQUIRED_MAX_ACTIVE_STREAMS(=1)`, non-empty `default_classification`, explicit `operator_disabled`, fixed absolute geometry) + `resolve_media_ingress` (any error → disabled default). Approved-zone identity `tze_hud_scene::config::APPROVED_MEDIA_ZONE` = `media-pip`; approved `ZoneDefinition` built by `media_ingress.rs::approved_media_zone` (`VideoSurfaceRef`, `WebRtcRequired`, `max_publishers:1`). Deployment config `app/tze_hud_app/config/windows-media-ingress.toml` (`media_ingress` table + `windows-local-media-producer` capability grant `["media_ingress","publish_zone:media-pip",…]`). Capability grants: `crates/tze_hud_runtime/src/media_admission.rs::MediaCapabilityConfig::{from_media_ingress_config,windows_media_ingress_enabled}` (`CAPABILITY_MEDIA_INGRESS`, Owner/Admin only via `OperatorRole::may_grant_media_capability`). Tests: `tze_hud_config/src/tests.rs::{media_ingress_absent_defaults_to_disabled,media_ingress_enabled_requires_approved_zone_and_fixed_geometry,invalid_media_ingress_does_not_resolve_or_register_zone}`, `media_admission.rs::test_media_ingress_default_config_disabled`, integration `session_server/tests.rs::media_ingress_disabled_gate_rejects_without_admission`. |

### runtime-kernel (MODIFIED)

| # | Requirement | Status | Evidence (file:symbol) |
|---|---|---|---|
| 2 | Media Worker Pool | **SATISFIED (synthetic-source gen-1 scope)** | Gate: `media_admission.rs::evaluate` (`worker_pool_has_slot`, `CapabilityNotEnabled` short-circuit) — no pool spawn when disabled. Decode primitive `crates/tze_hud_runtime/src/gst_decode_pipeline.rs::GstDecodePipeline` (Drop → NULL state) exists behind the `MediaDecodePipeline` trait. Teardown reasons `media_ingress.rs::MediaCloseReason::{LeaseRevoked,CapabilityRevoked,OperatorMute,BudgetWatchdog,ScheduleExpired}` + pause triggers `MediaPauseTrigger::{OperatorRequest,SafeMode,PolicyQuietHours}`; frame-bounded stop at the compositor by `crates/tze_hud_compositor/src/renderer/video.rs::prune_terminal_video_surfaces` / `evict_video_frame_texture` (terminal-surface uploads rejected). Workers stay outside the frame loop (RFC 0014 model). **Scope note (by-design, not a gap):** the full GStreamer-decode → bounded-channel → compositor worker integration is not yet wired end-to-end; gen-1 deliberately drives the surface via the synthetic frame path (design.md §5). |
| 3 | Decoded Frame Upload Contract | **SATISFIED in headless; live rendered-frame proof OUTSTANDING** | Runtime-owned upload: `crates/tze_hud_compositor/src/renderer/video.rs::upload_video_frame` (RGBA8 `VideoFrame` → runtime `wgpu` texture; rejects terminal surfaces, degenerate dims, byte-count mismatch), `collect_video_frame_cmds` / `encode_video_frame_pass` render the latest accepted frame clipped to `resolve_zone_geometry`. Deterministic placeholder: `crates/tze_hud_compositor/src/video_surface.rs::VideoRenderState::Placeholder` (Admitted-before-first-frame and Closed/Revoked both map to placeholder); tests `video_surface.rs` (initial Placeholder; Closing/Revoked → dark placeholder), synthetic-render pixel readback `crates/tze_hud_runtime/tests/pixel_readback.rs`. Synthetic producer: `video_surface.rs::SyntheticTestPipeline` / `gst_decode_pipeline.rs::new_from_test_src`. Missing-decode-dependency → v1 `VideoSurfaceMap` stub returns `Placeholder` unconditionally + admission fails with structured reason (`media.rs` `MEDIA_DISABLED`; `media_admission.rs` `TextureHeadroomLow`/`CodecUnsupported`). **The "decoded frame replaces placeholder" scenario is proven only in headless/synthetic tests.** On the live Windows lane the producer admitted a stream but presented **no frames** (`producer-soak-final-evidence.json`: `first_frame_time_ms=null`, `nonzero_frame_sample_count=0`) — because the live GStreamer decode → compositor path is not wired (Scope note #1). Live rendered-frame proof is a tracked carve-out, tied to the decode-path follow-on. |

### session-protocol (MODIFIED)

| # | Requirement | Status | Evidence (file:symbol) |
|---|---|---|---|
| 4 | Media Ingress Session Messages | **SATISFIED** | Proto: `crates/tze_hud_protocol/proto/session.proto` `MediaIngressOpen`(60)/`Close`(61)/`OpenResult`(60)/`State`(61)/`CloseNotice`(62) + SDP/ICE. `session_server/media.rs::handle_media_ingress_open` returns `MediaIngressOpenResult{admitted:true, stream_epoch=next_media_epoch(), assigned_surface_id=SceneId::new()}` + `send_media_state(Admitted)`; `handle_media_ingress_close`, `close_active_media_ingress` emit `MediaIngressState` + `MediaIngressCloseNotice`. Disabled gate: `media_open_rejection` → `MEDIA_DISABLED` with no worker spawn. Deferred-message rejection: `session_server/mod.rs:973-984` (`MediaSdpAnswer`/`MediaPauseRequest`/`MediaResumeRequest` → RuntimeError "deferred outside the one-stream Windows ingress slice"); test `session_server/tests.rs::media_ingress_still_deferred_messages_return_runtime_error`. Message roundtrip suite: `crates/tze_hud_protocol/tests/media_signaling.rs`. Live proof: admitted stream `stream_epoch=1 selected_codec=VIDEO_H264_BASELINE` (soak evidence, §Live evidence). |

### scene-graph (MODIFIED)

| # | Requirement | Status | Evidence (file:symbol) |
|---|---|---|---|
| 5 | VideoSurfaceRef and WebRtcRequired | **SATISFIED** | Admitted stream publishes `ZoneContent::VideoSurfaceRef(surface_id)` into `media-pip` via `media.rs::handle_media_ingress_open` → `scene.publish_to_zone`. Non-`media-pip` zones rejected in `media.rs::media_open_rejection` (`SURFACE_NOT_FOUND` when `zone_name != approved_zone`). Transport constraint enforced: only `MediaTransportMode::WebrtcStandard` admitted (`CAPABILITY_NOT_IMPLEMENTED` otherwise). Compositor renders only the `APPROVED_MEDIA_ZONE` surface (`renderer/video.rs` `.get(APPROVED_MEDIA_ZONE)`); default `pip`/`ambient-background` never gain implicit `VideoSurfaceRef` acceptance. Zone media-type mapping `crates/tze_hud_scene/src/graph/zone_ops.rs::content_media_type`. Tests: `session_server/tests.rs::media_ingress_rejects_second_stream_wrong_zone_missing_classification_and_audio`; `crates/tze_hud_scene/tests/zone_ontology.rs::{default_pip_and_ambient_background_do_not_accept_video_surface_ref,video_surface_ref_schema_defined_but_not_rendered,zone_media_type_mismatch_rejected}`. |

### media-webrtc-bounded-ingress (MODIFIED)

| # | Requirement | Status | Evidence (file:symbol) |
|---|---|---|---|
| 6 | Post-v1 Activation Boundary | **SATISFIED** | Default runtime media-disabled (`MediaIngressConfig` default). Activation requires enablement + approved zone + capability + classification + budget, all conjoined in `media.rs::media_open_rejection` and `media_admission.rs::evaluate`. |
| 7 | Directional Transport Boundary | **SATISFIED (audio reject code needs spec/code reconciliation)** | One global stream: `media.rs::handle_media_ingress_open` `MediaPublishAdmission::GlobalLimit` (`st.media_ingress_active.is_some()`) + per-session `SESSION_STREAM_LIMIT`. Audio rejected: `media.rs` `has_audio_track` → `AUDIO_NOT_SUPPORTED`; bidirectional excluded (video-only track required, `has_video_track`). Test: `media_ingress_rejects_second_stream_wrong_zone_missing_classification_and_audio`, `media_ingress_limit_is_global_and_disconnect_releases_slot`. **Spec/code divergence to resolve before sync:** the wire reject code + tests use `AUDIO_NOT_SUPPORTED`, but the delta-spec scenario mandates `AUDIO_UNSUPPORTED`. The rejection is deterministic and correct, but the string literally differs from the spec — this must be reconciled at archive/sync (amend the spec scenario wording to `AUDIO_NOT_SUPPORTED`, or file a code+test rename bead), not left divergent. |
| 8 | Timing, Lease, and Budget Bounds | **PARTIAL — lease/budget/teardown SATISFIED; expired-frame-drop scenario OUTSTANDING** | Timing (admission): `media.rs::media_open_rejection` calls `validate_timing_hints` (present/expiry bounds). Lease revoke → `subscriptions_cap.rs:231 close_active_media_ingress(MediaCloseReason::CapabilityRevoked)`; `media_ingress.rs::MediaSessionEvent::Revoke(LeaseRevoked)`. Budget: `media.rs` `declared_peak_kbps > MEDIA_INGRESS_PEAK_KBPS_BUDGET` → `BUDGET_EXCEEDED`; `media_ingress.rs::MediaCloseReason::BudgetWatchdog`. Frame-bounded stop at compositor (`renderer/video.rs::prune_terminal_video_surfaces`). Tests: `media_ingress.rs` revoke/pause suite (`test_terminal_revoked_rejects_all_events`, safe-mode/quiet-hours). **Not yet satisfied:** the scenario "a media frame arrives after its expiry → MUST NOT replace the presented surface AND MUST record a dropped-frame reason" has no shipped runtime frame-path implementation or test — expiry is enforced at admission only, and no live per-frame decode/present path exists (Scope note #1). Tracked as a carve-out with the decode-path follow-on. |
| 9 | Reconnect and Snapshot Behavior | **SATISFIED** | No implicit surface inheritance: `close_active_media_ingress` clears both `session.media_ingress` and global `st.media_ingress_active` + `clear_zone_for_publisher`; a reconnecting producer re-runs the full `media_open_rejection` gate (fresh admission, new `stream_epoch`). Test: `session_server/tests.rs::media_ingress_limit_is_global_and_disconnect_releases_slot`. |

### media-webrtc-privacy-operator-policy (MODIFIED)

| # | Requirement | Status | Evidence (file:symbol) |
|---|---|---|---|
| 10 | Explicit Enablement Policy | **SATISFIED** | `media.rs::media_open_rejection` denies before transport/decode when `!enabled`; `MediaIngressConfig` default-off. Live: authenticated `MEDIA_DISABLED` proof (§Live evidence). |
| 11 | Human Operator Overrides | **SATISFIED** | `operator_disabled` short-circuits **before** capability check (`media.rs:122`) — proven live and by test `media_ingress_operator_disabled_rejects_authenticated_open` (PR #1123). No auto-resume: `media_ingress.rs` pause-resume authority (agent may not resume operator/safe-mode pause; `test_safe_mode_resume_dropped_for_operator_pause`). Frame-bounded suppression via compositor terminal-surface prune. |
| 12 | Media Privacy Classification and Viewer Ceiling | **SATISFIED (model-complete; live single-viewer)** | Missing classification fails closed: `media.rs::media_open_rejection` (`CONTENT_CLASS_DENIED` when empty or `!= default_classification`). Viewer-class floor: `media_ingress.rs` gate step 7 (RFC 0009, `content_class_passes` → `ContentClassDenied`). Test: `media_admission.rs::test_admission_rejects_content_class_denied`. The "viewer context becomes unknown → suppress within one compositor frame" scenario is enforced through the same terminal-surface prune path; the exemplar's live lane runs single-viewer/operator-present, so dynamic viewer-context-loss suppression is exercised at the model/state-machine level rather than end-to-end live (acceptable for gen-1; not a gap). |
| 13 | Media Attention Governance | **SATISFIED** | Quiet-hours/safe-mode governance: `media_ingress.rs::MediaPauseTrigger::{PolicyQuietHours,SafeMode}` + `MediaSessionEvent::{PolicyQuietHoursPause,SafeModePause}`; quiet presentation is the default admitted state (no focus/expand). Tests: `test_quiet_hours_resume_dropped_for_safe_mode_pause`, safe-mode suite. |
| 14 | Media Policy Audit and Precedence | **SATISFIED** | Audit events `crates/tze_hud_telemetry/src/media_audit.rs::MediaAuditEvent::{MediaAdmissionGrant,MediaAdmissionDeny,MediaStreamClose,MediaStreamRevoke,MediaOperatorOverride,…}` emitted through `media_admission.rs::MediaAuditSink` + `record_operator_override`, `record_capability_revoke`, `record_stream_close`, `record_degradation_step`, `record_preempt`; `evaluate` denials call `deny(...)` (audit) and short-circuit **before** decode/transport work. Wire path logs structured `reject_code`/`reject_reason` (`media.rs` `tracing::warn!`). Test: `media_admission.rs::test_record_stream_revoke_emits_revoke_event`. |

### validation-framework (ADDED)

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 15 | Windows Media Validation Lanes | **Lane A SATISFIED; Lane B admission/soak-only (render OUTSTANDING); Lane C source-evidence only, bridge policy-gated** | Lane A (synthetic/headless): `session_server/tests.rs` media suite + `media_admission.rs` + `video_surface.rs` + `pixel_readback.rs` tests — admission, placeholder, clipping, teardown, second-stream, policy-denial, classification, revoke, reconnect gating, disabled-gate, deferred-message, all machine-verifiable. Lane B (live Windows, self-owned/local source): merged PR #958 evidence proves **authenticated admission** (`stream_epoch=1`, `selected_codec=VIDEO_H264_BASELINE`), **10-min record-only soak** (`docs/evidence/media-ingress/hud-gog64.8-20260620/`) with CPU/GPU/mem + no-leak drift, and **`MEDIA_DISABLED` operator-disabled proof** (`docs/evidence/media-ingress/hud-8dht5/`). **But the scenario clause "MUST show a self-owned/local video source rendered in the approved HUD media zone" and "first-frame time" are NOT met live:** the soak evidence records `first_frame_time_ms=null`, `nonzero_frame_sample_count=0` — the stream was admitted and held but never presented a frame (decode path unwired, Scope note #1). Lane B is therefore **admission + governance + soak proven, live rendered-frame proof outstanding**. Lane C (YouTube): official-embed **source evidence only**; raw-frame bridge into `MediaIngressOpen` is **not implemented** and is owner-policy-gated (`RAW_YOUTUBE_BRIDGE_DECISION="blocked_pending_policy_approval"`, `.claude/skills/user-test/scripts/windows_media_ingress_exemplar.py:52`). Report artifacts written under `docs/reports/` and `docs/evidence/` as required. |

### windows-media-ingress-exemplar (ADDED)

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 16 | Windows Media Ingress Exemplar Scope | **SATISFIED (admission); live render outstanding** | One video-only stream into runtime-owned `media-pip` on native Windows; no audio/multi-feed/mobile/glasses/bidirectional (enforced by admission gates above). Live-**admitted** on the Windows host (PR #958); the "resulting presentation MUST render in the approved content-layer media zone" clause is render-proven only in headless synthetic tests, not yet on the live lane (see Requirement 3/15). |
| 17 | YouTube Source Evidence Boundary | **TRACKED BLOCKER (owner-gated, not a gap)** | Official-embed source evidence launched for `O0FGCxkHM-U`; prohibited paths (`yt-dlp`/download/direct-URL/cache) explicitly rejected by the exemplar harness. The approved Windows-only raw-frame bridge (frames enter HUD only via `MediaIngressOpen`) is **owner-policy-gated**: `RAW_YOUTUBE_BRIDGE_DECISION="blocked_pending_policy_approval"`. Scoping: `docs/evidence/media-ingress/youtube-bridge-scoping-20260620.md`. Beads `hud-o33hj`/`hud-d82p7`/`hud-s0pit` blocked; PRs #658/#659 OPEN+PARKED. |
| 18 | Exemplar Demonstrates Operator Control | **SATISFIED** | Operator disable / safe mode / lease revoke remove media within one compositor frame and require fresh admission (no auto-resume): `media.rs` operator-disable precedence + `media_ingress.rs` pause-resume authority + compositor terminal prune. Live `MEDIA_DISABLED` proof (§Live evidence). |

## Tasks reconciliation (`tasks.md`)

| Task | Status | Note |
|---|---|---|
| 1.1 validate `--strict` | **DONE** | valid (this session). |
| 1.2 doctrine review / 1.3 keep out-of-scope deferred | **DONE** | Scope held across all children; out-of-scope classes rejected in admission. |
| 1.4 doctrine narrow-exception pointers | **DONE (implementation)** | Delivered under `hud-gog64.1` (PR #653). |
| 2.1–2.5 config + zone + capability | **DONE** | `hud-gog64.1` / PR #653 (closed). |
| 3.1–3.5 synthetic render + headless tests | **DONE** | `hud-gog64.2` / PR #654 (closed). |
| 4.1–4.5 protocol + admission + synthetic validation | **DONE** | `hud-gog64.3` / PR #655 (closed) + salvage test PR #1123. |
| 5.1–5.5 producer + YouTube source evidence | **DONE** | `hud-gog64.4` / PR #656 (closed). |
| 5.6 approved YouTube raw-frame bridge | **BLOCKED (owner policy gate)** | `RAW_YOUTUBE_BRIDGE_DECISION="blocked_pending_policy_approval"`; `hud-o33hj`/`hud-d82p7` blocked, PRs #658/#659 parked. |
| 6.1 live evidence under docs/reports | **DONE** | PR #958 merged (evidence tree). |
| 6.2 10-min soak (record-only) | **DONE** | `hud-gog64.8`/`hud-156qr`, 600 s hold, no leak. |
| 6.3 follow-up beads for gaps | **DONE** | YouTube-bridge cluster scoped (`hud-o33hj`/`d82p7`/`s0pit`/`t1900`); observations `hud-gcn01` (fullscreen bench hang), close-reason keepalive note. |
| 6.4 reconcile before archive/sync | **DONE (this report)** | — |
| 7.1–7.3 beads handoff graph | **DONE** | Epic `hud-gog64` + six children created, dependency-wired, verifiable via `bd show`/`bd dep tree`. |

## Child bead graph — terminal state

| Bead | Title | Status |
|---|---|---|
| `hud-gog64` | Windows media ingress exemplar (epic) | blocked (awaiting this reconciliation) |
| `hud-gog64.1` | Configure approved Windows media zone | closed (PR #653) |
| `hud-gog64.2` | Render synthetic VideoSurfaceRef frames | closed (PR #654) |
| `hud-gog64.3` | Wire media ingress admission protocol | closed (PR #655) |
| `hud-gog64.4` | Windows media producer + YouTube source evidence | closed (PR #656) |
| `hud-gog64.5` | Validate Windows media ingress + report | closed (media-lane reconciliation `hud-cvgvg`) |
| `hud-gog64.6` | Terminal gen-1 reconciliation | in_progress (this report) |
| `hud-gog64.7` | Exclusive GPU window for validation | closed (evidence `752589a3`) |
| `hud-gog64.8` | 10-min record-only soak | closed (evidence tree) |

No implementation child remains open without a tracked reason.

## Tracked blockers (owner-gated — NOT gaps)

The raw-YouTube-frame-bridge cluster is **owner-policy-gated**, not incomplete
engineering. Main deliberately holds
`RAW_YOUTUBE_BRIDGE_DECISION="blocked_pending_policy_approval"`; flipping it is a
maintainer policy call (PR #658 is the flip).

| Bead / PR | What it needs | Gate |
|---|---|---|
| `hud-o33hj` / PR #658 (open, draft) | Approved Windows-only raw-frame bridge (official player → `MediaIngressOpen`) | Owner flips `RAW_YOUTUBE_BRIDGE_DECISION` |
| `hud-d82p7` / PR #659 (open) | Windows frame-capture adapter (non-dry-run frame proof) | Same policy gate + `hud-t1900` Chrome Error 153 |
| `hud-s0pit` | Live Windows HUD pixel/readback proof for the bridge | Dep-blocked on the two above + exclusive GPU window |
| `hud-t1900` | Windows Chrome Error 153 fix for official-player render | Prereq for live YouTube lane |

These map to spec Requirement 17 (YouTube Source Evidence Boundary) and task 5.6.
Everything else in the change is satisfied by shipped code, tests, and merged live
evidence.

## Scope notes (by-design, not gaps)

These surfaced during code reconciliation and are consistent with the change's
stated gen-1 scope; none blocks archival:

1. **GStreamer decode worker not wired end-to-end.** `GstDecodePipeline` exists as
   a primitive but is not yet plumbed through a bounded channel into the compositor
   loop. Gen-1 deliberately drives the surface via the synthetic frame path
   (design.md §5: "first implementation uses synthetic frames"). Full decode-path
   wiring belongs with the YouTube-bridge follow-on or a later change.
2. **No dedicated "expired-frame-dropped-within-one-frame" test symbol.** The
   behavior is enforced by terminal-surface upload rejection
   (`renderer/video.rs::upload_video_frame`) + per-60-frame
   `maybe_prune_terminal_video_surfaces`, and by `validate_timing_hints` at
   admission. A focused timing-drop test is a nice-to-have, not a requirement gap.
3. **Exemplar producer/sidecar is a Python `/user-test` script**, not a Rust
   `examples/` crate — explicitly permitted by design.md §4 ("example application,
   test script, or local sidecar").

## Verdict

**ARCHIVABLE with THREE carve-outs explicitly tracked — one owner-policy-gated,
two tied to the (deliberately deferred) live decode/render path.**

Requirement scorecard (18 total):

- **14 fully satisfied** (config, worker-pool gate, protocol messages, scene-graph,
  activation boundary, reconnect, all five privacy/operator/attention/audit
  requirements, validation Lane A, exemplar scope-admission, operator control).
- **Requirement 12** (viewer ceiling) — model-complete, live single-viewer coverage.
- **Carve-out A — owner-policy-gated (NOT an engineering gap):** Requirement 17 /
  task 5.6, the YouTube raw-frame bridge (`RAW_YOUTUBE_BRIDGE_DECISION=
  blocked_pending_policy_approval`; `hud-o33hj`/`hud-d82p7`/`hud-s0pit`/`hud-t1900`,
  PRs #658/#659 parked).
- **Carve-out B — live rendered-frame proof outstanding:** Requirement 3 (decoded
  frame replaces placeholder) and Requirement 15/16's live-render clause are proven
  only in headless synthetic tests; the live Windows lane admitted a stream but
  rendered **no frames** (`first_frame_time_ms=null`, `nonzero_frame_sample_count=0`).
  Root cause is the deliberately-deferred GStreamer-decode → compositor wiring
  (design.md §5 ships gen-1 on the synthetic path). Tied to the decode-path follow-on.
- **Carve-out C — expired-frame-drop scenario unimplemented:** Requirement 8's
  "frame after expiry dropped + dropped-frame reason recorded" has no shipped
  runtime frame-path code/test (same decode-path root cause).

**Recommendation:** the admission/governance/policy/protocol layer of the exemplar
is complete and the delta specs may be synced/archived, **provided** the archive
record explicitly records carve-outs A/B/C as tracked follow-ons rather than
silently treating the change as fully implemented. If the owner requires live
rendered-frame proof (carve-out B) as a gen-1 acceptance bar, hold archival until
the decode-path lane lands; otherwise archive-with-carve-outs is appropriate given
the media lane's `hud-cvgvg` acceptance of #958 as the evidence of record. Also
resolve the `AUDIO_NOT_SUPPORTED` vs `AUDIO_UNSUPPORTED` spec/code divergence
(Requirement 7) at sync time.

## Recommended bead actions (for the COORDINATOR — this worker files nothing)

1. **Close `hud-gog64.6`** with a reason linking this report and stating verdict =
   archivable-with-carve-out.
2. **Archive `windows-media-ingress-exemplar`** (`openspec archive`) and sync the
   eight delta specs into `openspec/specs/`, with an archive note carving out the
   owner-gated YouTube-bridge lane.
3. **Unblock/close epic `hud-gog64`** once 1–2 land; its acceptance ("terminal
   reconciliation states archive/readiness or tracked blockers") is met by this
   report. The YouTube-bridge cluster should re-parent to a standalone follow-on
   (it is not a gen-1 exemplar gap).
4. **Keep `hud-o33hj`/`hud-d82p7`/`hud-s0pit`/`hud-t1900` blocked** and PRs
   #658/#659 parked pending the owner `RAW_YOUTUBE_BRIDGE_DECISION` call.
5. **File a decode-path / live-render follow-on bead (carve-outs B + C):** wire the
   GStreamer decode → bounded-channel → compositor frame path so the live Windows
   lane can prove a rendered frame (`first_frame_time_ms` non-null,
   `nonzero_frame_sample_count > 0`) and implement + test the expired-frame-drop
   scenario (Requirement 8). Decide whether this is a gen-1 acceptance bar or an
   accepted gen-2 follow-on before archiving.
6. **Resolve the audio reject-code divergence (Requirement 7) at sync time:** amend
   the delta-spec scenario wording to `AUDIO_NOT_SUPPORTED` to match shipped code +
   tests, or file a code/test rename bead. Do not sync the spec with the divergent
   `AUDIO_UNSUPPORTED` string.
7. **Optional hygiene:** note the long-hold `close_reason=SESSION_DISCONNECTED` vs
   `AGENT_CLOSED` keepalive observation from the soak (non-blocking).
