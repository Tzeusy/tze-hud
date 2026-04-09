# Policy Wiring Closeout Reconciliation (hud-s98v.5)

Date: 2026-04-10
Parent epic: `hud-s98v`
Scope: Deep-dive reconciliation of retained policy claims after closeout-path decision (`hud-s98v.3`) and claim-shrink pass (`hud-s98v.4`).

---

## Executive Result

[Observed] The closeout path taken in repository state is the **shrink path**: core stack-wiring requirements are now explicitly marked `v1-reserved` in `policy-arbitration/spec.md`, while runtime-sovereignty and capability-governance seams remain `v1-mandatory`.

[Observed] Runtime ownership seams and Level 3 capability-source semantics are implemented and tested in current code.

[Observed] Three retained `v1-mandatory` claim families still lack end-to-end implementation evidence in runtime hot paths:
1. Policy telemetry in runtime `TelemetryRecord`.
2. Arbitration telemetry event emission from runtime paths.
3. Structured capability grant/revocation audit records with required fields (`agent_id`, capability, timestamp, source).

[Inferred] These should be tracked as follow-on fix beads before final policy closeout signoff (`hud-s98v.6`) claims full retained-claim coverage.

---

## Inputs Audited

- Doctrine:
  - `about/heart-and-soul/architecture.md`
  - `about/heart-and-soul/v1.md`
  - `about/heart-and-soul/validation.md`
- Specs:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
- Reconciliation doctrine/contracts:
  - `docs/reconciliations/policy_wiring_seam_contract.md`
  - `docs/reconciliations/policy_wiring_completion_direction_20260409.md`
- Runtime/protocol/policy code + tests:
  - `crates/tze_hud_runtime/src/lib.rs`
  - `crates/tze_hud_runtime/src/budget.rs`
  - `crates/tze_hud_runtime/src/channels.rs`
  - `crates/tze_hud_protocol/src/session_server.rs`
  - `crates/tze_hud_policy/src/telemetry.rs`
  - `crates/tze_hud_policy/src/security.rs`
  - `crates/tze_hud_protocol/proto/session.proto`

---

## Retained-Claim Evidence Matrix

Legend: `COVERED` | `PARTIAL` | `MISSING`

| Retained claim family | Status | Evidence | Reconciliation notes |
|---|---|---|---|
| V1 runtime authority remains runtime/session/scene-owned; `tze_hud_policy` not hot-path authority | COVERED | `openspec/.../policy-arbitration/spec.md` (Requirement: V1 Authority Boundary, `Scope: v1-mandatory`), `crates/tze_hud_runtime/src/lib.rs:12`, `crates/tze_hud_runtime/src/lib.rs:25`, `docs/reconciliations/policy_wiring_seam_contract.md:20` | Runtime/docs/spec align on sovereignty boundary after shrink pass. |
| Target full 7-level stack wiring is tracked separately (not claimed as active v1 hot path) | COVERED | `openspec/.../policy-arbitration/spec.md:22`, `...:31`, `...:40`, `...:224`, `...:237`, `...:246`, `...:324` (`Scope: v1-reserved`) | Closeout decision is reflected in spec scopes; no remaining implicit full-stack-v1 claim in this spec surface. |
| Level 3 split capability-source semantics: live session grants for enforcement; policy scope for CapabilityRequest | COVERED | `openspec/.../policy-arbitration/spec.md:147`, `crates/tze_hud_protocol/src/session_server.rs:3535`, `...:3546`, `...:2984`, tests `...:7522`, `...:7598` | Governance seam repaired and test-backed (deny out-of-scope lease requests; allow escalation only via authorization scope). |
| Lease scope requests outside session-held grants are denied as whole request (`PERMISSION_DENIED`) | COVERED | `crates/tze_hud_protocol/src/session_server.rs:2973-2990`, test `...:7253` | Matches closeout invariant: deny, do not clamp. |
| Canonical capability vocabulary enforcement (reject superseded names with hints) | COVERED | `crates/tze_hud_protocol/src/session_server.rs:2913`, `crates/tze_hud_policy/src/security.rs:1`, test `crates/tze_hud_protocol/src/session_server.rs:7211` | Spec-language and runtime behavior aligned for canonical naming seam. |
| Policy telemetry required in per-frame runtime telemetry (`PolicyTelemetry` fields) | MISSING | Spec retained as mandatory: `openspec/.../policy-arbitration/spec.md:342` + `...:345`; runtime telemetry record has only `frame_number/frame_time_us/overflow_drops`: `crates/tze_hud_runtime/src/channels.rs:468`; no runtime/pipeline references to `PolicyTelemetry` in `tze_hud_runtime` | `PolicyTelemetry` exists only in `tze_hud_policy` (`crates/tze_hud_policy/src/telemetry.rs:29`) and is not wired to runtime frame telemetry. |
| Arbitration telemetry events required for Level 3 reject / Level 5 shed and lower-rate Level 2/4 events | PARTIAL | Spec retained as mandatory: `openspec/.../policy-arbitration/spec.md:351`; event type exists: `crates/tze_hud_policy/src/telemetry.rs:106`; runtime side has no usage in `tze_hud_runtime` | Data model exists, but no runtime emission path evidence in the authoritative runtime flow. |
| Capability grant/revocation audit with source + timestamp | PARTIAL | Spec retained as mandatory: `openspec/.../policy-arbitration/spec.md:360`; audit struct exists: `crates/tze_hud_policy/src/telemetry.rs:210`; protocol messages currently expose `CapabilityNotice` and `LeaseStateChange` without explicit `granted_by` field (`crates/tze_hud_protocol/proto/session.proto:403`, `...:483`) | Revocation notifications exist, but structured audit surface required by spec is not evidenced as an emitted runtime record. |

---

## Doctrine Consistency Check

[Observed] `about/heart-and-soul/architecture.md` requires runtime sovereignty and non-model hot-path ownership; retained mandatory claim set now honors that boundary.

[Observed] `about/heart-and-soul/validation.md` requires machine-readable diagnostics and measurable evidence for claims. The three uncovered telemetry/audit claim families above currently violate that evidentiary bar.

[Inferred] Closing policy-wiring honestly requires either (a) wiring these retained telemetry/audit claims, or (b) demoting their scopes out of `v1-mandatory` with explicit post-v1 reservation language.

---

## Follow-On Fix Bead Payload

Materialized to:
- `docs/reconciliations/policy_wiring_closeout_followups_20260410.proposed_beads.json`

This payload contains candidate beads for each retained claim family still lacking code/test evidence.

---

## Conclusion

[Observed] The shrink-path closeout successfully resolved the largest claim drift (full-stack policy hot-path in v1).

[Observed] Retained mandatory claims are still over-assertive in telemetry/audit surfaces relative to current runtime wiring.

[Inferred] `hud-s98v.5` is complete as a reconciliation pass with explicit evidence and follow-up payload, but final human signoff (`hud-s98v.6`) should treat these follow-ons as required closure gates unless the spec scopes are further reduced.
