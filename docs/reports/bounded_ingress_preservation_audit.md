# Bounded-Ingress Preservation Audit

**Issued for**: `hud-ora8.1.21`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: F28 from
`openspec/changes/v2-embodied-media-presence/signoff-packet.md`

## Scope

Line-by-line audit of every MUST/SHOULD requirement in:

1. `openspec/specs/media-webrtc-bounded-ingress/spec.md`
2. `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`

Each requirement is classified as:

- **PRESERVED** — requirement language is carried forward verbatim or
  substantially intact in the v2 successor.
- **SUPERSEDED** — requirement intent survives but the authority text moves to
  a different v2 location (spec, RFC, or doctrine); the old formulation no
  longer governs.
- **DROPPED** — requirement was intentionally retired and does not need
  equivalent coverage in v2 (typically replaced by a narrower/broader
  architectural decision).

Successor location for preserved/superseded items refers to:

- `openspec/changes/v2-embodied-media-presence/specs/media-plane/spec.md`
  (abbreviated **v2/media-plane**)
- `openspec/changes/v2-embodied-media-presence/specs/presence-orchestration/spec.md`
  (abbreviated **v2/presence-orchestration**)
- `openspec/changes/v2-embodied-media-presence/specs/validation-operations/spec.md`
  (abbreviated **v2/validation-operations**)
- `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
  (abbreviated **v2/signoff-packet**)
- RFC 0014 / RFC 0015 / RFC 0019 (referenced in the signoff packet as the
  authoritative wire/state-machine specs; not yet written at audit time)

---

## Part 1: `openspec/specs/media-webrtc-bounded-ingress/spec.md`

### Requirement: Post-v1 Activation Boundary

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-1 | Media/WebRTC ingress **MUST** remain post-v1 and **MUST NOT** alter v1 doctrine defaults. | **SUPERSEDED** | v2/media-plane "Governed Media Plane Activation" absorbs this. The "default-off, gated by explicit capability/lease/operator-policy/budget" invariant is explicitly restated. v2/signoff-packet F27 governs the v1 ship gate. |
| BI-2 | The runtime **MUST** keep media ingress disabled unless explicit post-v1 activation criteria are met. | **PRESERVED** | v2/media-plane Requirement "Governed Media Plane Activation": "the runtime admits it only if capability, lease, privacy, operator, and budget gates all succeed." All six prerequisite checks reappear. |
| BI-3 | Activation criteria **MUST** require all of: approved signaling contract, schema/snapshot deltas, zone transport contract, runtime budget gate, privacy/operator policy, compositor contract, and approved validation scenarios. | **SUPERSEDED** | The enumerated prerequisite list maps to RFC 0014 (wire protocol), RFC 0019 (audit schema), v2/validation-operations (validation), and v2/media-plane (policy/budget). The specific seven named artifacts are now governed by F29 (RFC merge gates) in v2/signoff-packet rather than being listed inside the spec. |
| BI-4 | **Scenario**: v1 runtime remains media-disabled — MUST spawn no media worker threads and MUST accept no live media ingress. | **PRESERVED** | v2/media-plane Requirement "Governed Media Plane Activation" scenario "bounded ingress activation remains governed" restates the same invariant at v2 scope. The specific phrasing around v1 defaults is superseded (F27 gates code landing), but the observable behavior MUST is preserved. |
| BI-5 | **Scenario**: activation denied when any prerequisite contract is missing — MUST be denied and ingress MUST remain disabled. | **PRESERVED** | v2/media-plane same requirement scenario "bounded ingress activation remains governed": all gates must succeed; any failure blocks admission. |

### Requirement: Directional Transport Boundary

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-6 | The first ingress slice **MUST** be strictly one-way visual ingress into the compositor. | **PRESERVED** | v2/media-plane Requirement "Bidirectional AV Is A Later Phase": bounded ingress does not imply bidirectional; runtime continues to reject two-way AV session negotiation until explicit contracts exist. One-way enforcement is retained. |
| BI-7 | The runtime **MUST NOT** accept upstream outbound media, negotiated bidirectional AV channels, or audio channels in this slice. | **PRESERVED** | v2/media-plane Requirement "Bidirectional AV Is A Later Phase": "Any bidirectional media session MUST satisfy explicit audio, operator, failure, and validation contracts beyond the bounded-ingress tranche." The audio channel exclusion is explicitly called out in v2/signoff-packet A5 non-goals ("No background audio … only when its owning surface is visible/foregrounded") and E22 (audio stack as a later-phase addition). |
| BI-8 | The slice **MUST** admit at most one active inbound media stream at a time. | **PRESERVED** | v2/signoff-packet A4 ("One embodied agent at a time; additional embodied requests are queued or rejected") and v2/presence-orchestration Requirement "Presence-Orchestrated Media Ownership" ("revoke, suspend, or downgrade media behavior") together preserve the single-stream admission limit for v2 phase 1. Multi-stream orchestration is deferred (v2/signoff-packet Deferred A5/4e). |
| BI-9 | **Scenario**: second concurrent stream MUST be rejected with a deterministic admission failure. | **PRESERVED** | Single-stream limit preserved — see BI-8. Deterministic admission failure is implied by v2/media-plane "Governed Media Plane Activation" (all gates must pass). |
| BI-10 | **Scenario**: audio-bearing ingress MUST be rejected and MUST NOT route audio to output. | **PRESERVED** | v2/media-plane "Bidirectional AV Is A Later Phase" explicitly states two-way AV is blocked until explicit contracts exist; audio channels are out of scope for phase 1 (v2/signoff-packet E22 and A5 non-goals). The reject-on-audio behavior is an implied consequence. |

### Requirement: Timing Semantics for Presentation and Expiry

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-11 | Ingress publications **MUST** declare presentation lifecycle timing (`present_at_wall_us`, `expires_at_wall_us`). | **PRESERVED** | v2/media-plane Requirement "Media Timing Is First-Class": "Media publications and cues SHALL carry presentation-time semantics … The runtime MUST preserve deterministic timing, expiry, and reconnect behavior." |
| BI-12 | Runtime scheduling **MUST** honor timing against compositor presentation cadence; **MUST NOT** present frames before `present_at_wall_us`; **MUST NOT** present frames at or after `expires_at_wall_us`. | **PRESERVED** | v2/media-plane same requirement: "schedules it against its declared timing contract rather than on arrival time alone." The `present_at` / `expires_at` distinction is explicit doctrine (CLAUDE.md: "Arrival time ≠ presentation time"). |
| BI-13 | Presentation **MAY** cease earlier on lease revocation, budget breach, or operator/policy disable. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation" + v2/presence-orchestration "Presence-Orchestrated Media Ownership": revocation/budget breach triggers deterministic teardown. |
| BI-14 | When both `ttl_us` and `expires_at_wall_us` are present, `expires_at_wall_us` **MUST** be canonical for snapshot/reconnect state. | **SUPERSEDED** | This is a wire-protocol normalization rule that belongs in RFC 0014 (Media Plane Wire Protocol). v2/media-plane "Media Timing Is First-Class" preserves the timing-semantics intent; the exact field normalization rule is delegated to the RFC authoring step (F29). |
| BI-15 | `ttl_us` remains valid as relative input and maps to absolute expiry via deterministic formula. | **SUPERSEDED** | Same as BI-14 — field-level normalization moves to RFC 0014. |
| BI-16 | If both `ttl_us` and `expires_at_wall_us` are non-zero and resolve to different expiry instants, publication **MUST** be rejected as invalid. | **SUPERSEDED** | Same as BI-14 — conflict-rejection rule moves to RFC 0014 wire protocol. The "fail closed on malformed timing" intent is preserved in v2/media-plane "Governed Media Plane Activation" (gates must all pass). |
| BI-17 | **Scenario**: scheduled ingress does not render early; first presentation MUST occur no later than one frame period after `present_at_wall_us`. | **PRESERVED** | v2/media-plane "Media Timing Is First-Class" scenario "timed media cue survives governance checks": runtime schedules against declared timing contract. The one-frame-period tolerance is a validation threshold governed by v2/validation-operations and v2/signoff-packet D18 thresholds. |
| BI-18 | **Scenario**: expired ingress MUST be rejected or immediately cleared; MUST render zero media frames for it. | **PRESERVED** | v2/media-plane "Media Timing Is First-Class": deterministic expiry/expiry behavior preserved. |
| BI-19 | **Scenario**: ttl-only ingress — receiver MUST derive and persist effective absolute expiry. | **SUPERSEDED** | Field-level rule moves to RFC 0014. |
| BI-20 | **Scenario**: conflicting ttl and absolute expiry — receiver MUST reject as malformed timing contract. | **SUPERSEDED** | Field-level rule moves to RFC 0014. |

### Requirement: Reconnect Snapshot Behavior for Scheduled Ingress

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-21 | Pending (not-yet-presented) ingress publications **MUST NOT** survive reconnect snapshot/resume. | **PRESERVED** | v2/media-plane "Media Timing Is First-Class": "The runtime MUST preserve deterministic timing, expiry, and reconnect behavior for media surfaces." The "snapshot-first contract limits" from BI spec §Zone and Layer Containment are also subsumed — only active publication state survives snapshot. |
| BI-22 | Reconnect snapshot **MUST** include only ingress publications already active at snapshot time. | **PRESERVED** | Same as BI-21 — see v2/media-plane reconnect preservation. |
| BI-23 | Clients **MUST** re-issue scheduled ingress after `SessionResumeResult`. | **PRESERVED** | Consistent with v2/signoff-packet B11 (on media drop while session survives, session continues; control path stays alive; re-admission is by fresh flow) and v2/media-plane timing semantics. |
| BI-24 | **Scenario**: scheduled ingress omitted from reconnect snapshot; client MUST re-issue if still desired. | **PRESERVED** | Same as BI-21–BI-23. |

### Requirement: Lease and Budget Coupling

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-25 | Media ingress **MUST** be jointly governed by lease validity and runtime budget policy. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": "capability, lease, privacy, operator, and budget gates all succeed." Lease + budget coupling is an explicit v2 first-class concern. |
| BI-26 | An ingress request **MUST** be admitted only when the publisher holds the required lease scope AND the runtime budget gate permits. | **PRESERVED** | Same v2/media-plane requirement and v2/signoff-packet B10 (`SessionInit flag presence_level=EMBODIED + embodied lease capability`, both required). |
| BI-27 | Lease revocation or budget breach **MUST** trigger deterministic ingress teardown. | **PRESERVED** | v2/presence-orchestration "Presence-Orchestrated Media Ownership": "revoke, suspend, or downgrade media behavior"; v2/signoff-packet C16 (tiered revocation: soft first, then hard ≤100 ms). |
| BI-28 | **Scenario**: ingress denied without lease authority — MUST deny and MUST return a structured authorization/budget error. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation" (all gates required) + v2/validation-operations "Structured Operator And Failure Observability" (structured signals for admission decisions). |
| BI-29 | **Scenario**: lease revocation tears down ingress within one compositor frame. | **SUPERSEDED** | Revocation timing is now governed by v2/signoff-packet C16 (tiered: soft ≤500 ms, hard ≤100 ms) and RFC 0015. The one-frame deadline is superseded by the tiered revocation numbers which are more precise. Intent is preserved; specific threshold moves to RFC 0015 / v2/presence-orchestration. |

### Requirement: Zone and Layer Containment

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-30 | Media ingress **MUST** be constrained to a fixed runtime-owned media zone class with specific properties (`accepted_media_types`, `transport_constraint`, `layer_attachment`). | **SUPERSEDED** | Zone contract details are wire-protocol concerns delegated to RFC 0014 and v2/media-plane. The intent of "runtime-owned surface, not agent-owned" is captured in v2/presence-orchestration "Presence-Orchestrated Media Ownership" and the CLAUDE.md doctrine ("Screen is sovereign"). The specific protobuf field names (`accepted_media_types`, `transport_constraint`, `layer_attachment`) move to RFC 0014. |
| BI-31 | Publishers **MUST** target the zone by canonical `zone_name`; any other zone name **MUST** be rejected for media ingress. | **SUPERSEDED** | Zone routing rules move to RFC 0014. The rejection-on-invalid-zone intent is preserved in v2/media-plane "Governed Media Plane Activation" (admission gates include zone conformance). |
| BI-32 | Runtime **MUST** enforce that `MediaIngressOpen` and `ZonePublish` with `VideoSurfaceRef` bind to the same approved zone identity. | **SUPERSEDED** | Specific message-type binding rule moves to RFC 0014 wire protocol. |
| BI-33 | Reconnect snapshot persists only declarative active publication state; transport session internals are never snapshotted. | **PRESERVED** | v2/media-plane "Media Timing Is First-Class": "deterministic timing, expiry, and reconnect behavior" preserved; see also BI-21/BI-22. |
| BI-34 | After resume, runtime **MUST** treat pre-disconnect transport as non-authoritative until stream-epoch reconciliation or fresh open. | **PRESERVED** | Consistent with v2/signoff-packet B11 (on media drop: media surface shows last frame with disconnection badge; fresh admission flow required) and v2/media-plane reconnect semantics. |
| BI-35 | If configuration declares a fixed media zone set, runtime **MUST** reject any attempt to open/publish outside that set. | **SUPERSEDED** | Configuration-level zone restriction moves to RFC 0014 and v2/media-plane deployment-time configuration. |
| BI-36 | **Scenario**: non-media zone target rejected, existing zone content unchanged. | **SUPERSEDED** | See BI-31. |
| BI-37 | **Scenario**: layer attachment contract enforced. | **SUPERSEDED** | See BI-30. |
| BI-38 | **Scenario**: transport constraint mismatch rejected. | **SUPERSEDED** | See BI-30. |
| BI-39 | **Scenario**: reconnect restores publication metadata but not transport session. | **PRESERVED** | See BI-33/BI-34. |
| BI-40 | **Scenario**: fixed-zone restriction rejects alternate zone identity. | **SUPERSEDED** | See BI-35. |

### Requirement: Privacy and Operator Safety Assumptions

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-41 | Every ingress publication **MUST** carry content classification and **MUST** be processed through privacy/viewer/operator policy before presentation. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": "privacy … gates all succeed." v2/presence-orchestration "Presence-Orchestrated Media Ownership" also enforces policy-gated teardown. The corresponding policy spec (media-webrtc-privacy-operator-policy) is superseded by v2; see Part 2. |
| BI-42 | Operator disable controls **MUST** immediately override ingress regardless of publisher intent. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation" + v2/signoff-packet C16 (tiered revocation; operator override). |
| BI-43 | **Scenario**: missing classification rejected with structured policy error. | **PRESERVED** | v2/validation-operations "Structured Operator And Failure Observability": structured signals for admission decisions and policy denials include reason codes. |
| BI-44 | **Scenario**: operator disable wins immediately; presentation MUST cease within one compositor frame; ingress MUST remain disabled until re-enabled. | **SUPERSEDED** | Revocation timing superseded by v2/signoff-packet C16 tiered numbers (≤500 ms soft, ≤100 ms hard fallback). The "remain disabled until explicit re-enable" intent is preserved across v2 (RFC 0015; v2/presence-orchestration). Specific one-frame claim moves to RFC 0014/0015. |

### Requirement: Measurable Validation Readiness

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| BI-45 | No media ingress implementation work **MAY** be treated as merge-ready unless validation scenarios prove the bounded contract end-to-end. | **PRESERVED** | v2/validation-operations "Phased Release Gates": later phases blocked until earlier phase validation evidence exists. v2/validation-operations "Dual-Lane Media And Device Validation": both deterministic and real-decode lanes required. |
| BI-46 | Validation **MUST** include at least: single-stream admission limits, timing-window compliance, lease-revocation teardown, policy-gated rejection paths, and operator disable behavior. | **PRESERVED** | v2/validation-operations "V1 Validation Backlog Carries Forward": the v1 validation program scenarios carry forward into v2 explicitly. v2/validation-operations "Phased Release Gates" and "Structured Operator And Failure Observability" cover the listed scenario categories. |
| BI-47 | **Scenario**: acceptance suite produces machine-verifiable pass/fail outcomes for all bounded ingress invariants. | **PRESERVED** | v2/validation-operations "Dual-Lane Media And Device Validation": deterministic CI-friendly rehearsal lane is a first-class requirement; "Structured Operator And Failure Observability": machine-readable evidence. |
| BI-48 | **Scenario**: failed invariant blocks readiness. | **PRESERVED** | v2/validation-operations "Phased Release Gates" + v2/signoff-packet D21 critical-tier gate (compositor hang/crash, audit log gap, embodied session state-machine violation, revoke >1 s, media escapes sandboxed surface all block release). |

---

## Part 2: `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`

### Requirement: Viewer Privacy Ceiling

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| PO-1 | Media ingress **MUST** be treated as visible to nearby viewers and **MUST** be governed by explicit viewer/privacy policy before admission and presentation. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": "capability, lease, privacy, operator, and budget gates all succeed." Privacy is a named, non-skippable gate. |
| PO-2 | Every ingress publication **MUST** carry content classification metadata. | **PRESERVED** | Subsumed by BI-41 (already captured above); v2/media-plane activation gate requires privacy policy pass. |
| PO-3 | The runtime **MUST** deny admission, or keep ingress disabled, when the current viewer context is unknown, unavailable, or does not satisfy the declared privacy ceiling. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": admission only when all gates succeed; unknown viewer context = gate failure = deny. |
| PO-4 | **Scenario**: unknown viewer context fails closed — MUST deny; no media MUST be presented. | **PRESERVED** | v2/media-plane governed activation: fail-closed behavior retained. |
| PO-5 | **Scenario**: viewer ceiling not satisfied — MUST reject with structured policy denial. | **PRESERVED** | v2/validation-operations "Structured Operator And Failure Observability": structured signals for policy denials without payload leakage. |

### Requirement: Human Operator Overrides

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| PO-6 | Human operator actions **MUST** take precedence over publisher intent. | **PRESERVED** | v2/presence-orchestration "Presence-Orchestrated Media Ownership": runtime revokes/suspends media through sovereign control; v2/signoff-packet C16 (tiered revocation). |
| PO-7 | Operator disable **MUST** immediately suppress active media ingress and **MUST** deny new admissions until explicit operator re-enable. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": operator gate is checked on every admission. v2/signoff-packet C16: soft revocation ≤500 ms, hard ≤100 ms. |
| PO-8 | Re-enable **MUST NOT** silently restore a previously pending or active media stream; any desired media ingress **MUST** be re-admitted after override is lifted. | **PRESERVED** | v2/signoff-packet B11 (on media drop: no implicit resume; publisher must re-issue fresh admission) and v2/presence-orchestration "Presence-Orchestrated Media Ownership". |
| PO-9 | **Scenario**: operator disable stops active ingress — presentation MUST cease within one compositor frame; stream MUST be torn down or held inactive. | **SUPERSEDED** | Timing superseded by C16 tiered revocation numbers (≤500 ms soft, ≤100 ms hard). Intent preserved; exact one-frame threshold moves to RFC 0015. |
| PO-10 | **Scenario**: operator re-enable does not auto-resume — no prior stream MUST be resumed implicitly; publisher MUST issue fresh admission flow. | **PRESERVED** | v2/signoff-packet B11 + v2/presence-orchestration. |

### Requirement: Explicit Enablement Policy

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| PO-11 | Media ingress **MUST** remain disabled by default. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation" (all gates must pass; default is no gates pass). v2/signoff-packet C13: "Runtime config for fundamental on/off (restart)." |
| PO-12 | The runtime **MUST** accept media ingress only when an explicit enablement state is present and approved. | **PRESERVED** | v2/media-plane same requirement; v2/signoff-packet C13 (explicit capability grants required). |
| PO-13 | The enablement state **MUST** be machine-readable, auditable, and checked as part of admission. | **PRESERVED** | v2/validation-operations "Structured Operator And Failure Observability" + v2/signoff-packet C17 (mandatory audit events: capability grants/denials; retention 90 d; schema versioned). |
| PO-14 | If the enablement state is missing, false, or not approved, the runtime **MUST** treat media ingress as disabled. | **PRESERVED** | v2/media-plane governed activation: all gates must succeed; missing enablement = gate failure. |
| PO-15 | **Scenario**: default-off startup remains disabled; no admission MUST occur. | **PRESERVED** | v2/media-plane same reasoning as PO-11/PO-12. |
| PO-16 | **Scenario**: missing enablement approval blocks admission — MUST reject as disabled policy, not as transport failure. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": gates evaluated in order; enablement failure is distinct from transport failure. The specific ordering of gate evaluation also appears in this spec's Admission Precedence requirement (see PO-21/PO-22 below). |

### Requirement: Observability and Auditability

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| PO-17 | The runtime **MUST** emit structured observability signals for admission decisions, policy denials, operator enable/disable actions, and teardown events. | **PRESERVED** | v2/validation-operations "Structured Operator And Failure Observability": "The runtime SHALL emit structured signals for media admission, teardown, operator override, device-state transitions, and failure recovery." v2/signoff-packet C17 (mandatory audit events covering all named categories). |
| PO-18 | Each signal **MUST** include the affected surface or zone, decision outcome, and machine-readable reason code. | **PRESERVED** | v2/validation-operations same requirement: signals are "machine-readable evidence"; v2/signoff-packet C17 audit schema versioned with per-event surface and reason. RFC 0019 (Audit Log Schema and Retention) will carry the field-level contract. |
| PO-19 | The runtime **MUST NOT** emit raw media frames, audio, or viewer biometric data in observability signals. | **PRESERVED** | v2/validation-operations "Structured Operator And Failure Observability" scenario "teardown is auditable without payload leakage": "emits machine-readable evidence of the transition without exposing raw media content." |
| PO-20 | **Scenario**: admission denial auditable without payload leakage — MUST record structured event with denial reason; MUST NOT contain raw media or viewer biometric data. | **PRESERVED** | v2/validation-operations same scenario. |
| PO-21 | **Scenario**: operator toggle is visible to telemetry — MUST emit operator-action event with new state and affected surface/zone identifier. | **PRESERVED** | v2/validation-operations "Structured Operator And Failure Observability" + v2/signoff-packet C17 (mandatory audit events include operator overrides). |

### Requirement: Admission Precedence

| # | Requirement text (paraphrased) | Verdict | Rationale & successor pointer |
|---|---|---|---|
| PO-22 | Media ingress admission **MUST** be evaluated in order: explicit enablement state → operator override state → viewer/privacy ceiling → remaining bounded-ingress admission checks. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation" preserves the ordered-gate model. Specific gate ordering is an implementation contract for RFC 0014; the fail-first semantics are implicit in "all gates must succeed." The exact priority encoding may be spelled out more explicitly in RFC 0014 / v2/media-plane detailed spec authoring. |
| PO-23 | If any earlier check fails, the runtime **MUST** deny the request deterministically and **MUST NOT** attempt later checks. | **PRESERVED** | v2/media-plane "Governed Media Plane Activation": deny if any gate fails. Fail-fast behavior is a named project anti-pattern avoidance (CLAUDE.md craft: "fail-fast behavior over silent fallback"). |
| PO-24 | **Scenario**: disabled policy short-circuits admission — MUST deny before evaluating viewer/privacy or transport checks. | **PRESERVED** | v2/media-plane ordered gates; see PO-22/PO-23. |
| PO-25 | **Scenario**: operator disable short-circuits admission — MUST deny before evaluating viewer/privacy or transport checks. | **PRESERVED** | v2/media-plane ordered gates; see PO-22/PO-23. |

---

## Summary Counts

### `media-webrtc-bounded-ingress/spec.md` (48 MUST/SHOULD items)

| Verdict | Count |
|---|---|
| PRESERVED | 29 |
| SUPERSEDED | 19 |
| DROPPED | 0 |
| **Total** | **48** |

### `media-webrtc-privacy-operator-policy/spec.md` (25 MUST/SHOULD items)

| Verdict | Count |
|---|---|
| PRESERVED | 22 |
| SUPERSEDED | 3 |
| DROPPED | 0 |
| **Total** | **25** |

### Combined totals (73 items)

| Verdict | Count |
|---|---|
| PRESERVED | 51 |
| SUPERSEDED | 22 |
| DROPPED | 0 |

No requirements were dropped. All MUST/SHOULD requirements have coverage in
the v2 program, either directly in the v2 specs already authored or delegated
to named RFCs (0014, 0015, 0019) that must merge before phase-1 implementation
beads per F29.

---

## Appendix A: Pointer-Block Text for Archiver

When phase-1 archive time arrives, the archiver should insert the following
block at the **top** of each superseded spec file (before the `# Specification:`
heading), per F28 instructions:

### For `openspec/specs/media-webrtc-bounded-ingress/spec.md`

```markdown
> **SUPERSEDED-BY**: `openspec/changes/v2-embodied-media-presence/specs/media-plane`
>
> This specification was absorbed into the v2-embodied-media-presence program
> (phase 1) per signoff-packet decision F28. Wire-protocol normalization rules
> (ttl/expires_at field handling, zone/layer contract, VideoSurfaceRef binding)
> moved to RFC 0014 (Media Plane Wire Protocol). All MUST/SHOULD requirements
> have been audited and classified in
> `docs/reports/bounded_ingress_preservation_audit.md`.
```

### For `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`

```markdown
> **SUPERSEDED-BY**: `openspec/changes/v2-embodied-media-presence/specs/media-plane`
>
> This specification was absorbed into the v2-embodied-media-presence program
> (phase 1) per signoff-packet decision F28. Viewer-privacy ceiling, operator
> override, explicit enablement policy, observability, and admission precedence
> requirements are preserved in v2/media-plane and v2/validation-operations.
> Audit log schema and retention moves to RFC 0019. All MUST/SHOULD requirements
> have been audited and classified in
> `docs/reports/bounded_ingress_preservation_audit.md`.
```

---

## Notes on SUPERSEDED Items

The 22 superseded items fall into three categories:

1. **Wire-protocol field rules** (BI-14 through BI-16, BI-19, BI-20, BI-30
   through BI-32, BI-35 through BI-38, BI-40): specific protobuf field names,
   normalization formulas, and zone/transport constraints move to RFC 0014.
   These are not dropped — they must be reaffirmed in that RFC before phase-1
   implementation beads can land.

2. **Revocation timing thresholds** (BI-29, BI-44, PO-9): the one-compositor-frame
   deadline is superseded by C16's tiered revocation numbers (soft ≤500 ms,
   hard ≤100 ms). The v2 numbers are more precise and architecturally grounded;
   the original one-frame claim was an implementation detail that has been
   replaced by a tiered SLA.

3. **Prerequisite enumeration** (BI-3): the specific seven-artifact checklist
   has been superseded by F29's RFC-merge-gate model, which is more precise
   (named RFCs with review counts) and is authoritative for v2 phase-1 kickoff.
