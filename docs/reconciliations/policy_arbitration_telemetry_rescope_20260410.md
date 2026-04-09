# Policy Arbitration Telemetry Re-Scope (hud-1jd5)

Date: 2026-04-10
Issue: `hud-1jd5`

## Decision

Re-scope runtime arbitration telemetry event emission claims from v1 mandatory to v1 reserved until mutation-path policy wiring is explicitly enabled.

## Evidence

1. [Observed] Runtime governance documentation states `tze_hud_policy` is not wired into v1 runtime hot paths (`crates/tze_hud_runtime/src/lib.rs`).
2. [Observed] Repository search found no runtime/protocol call sites for `tze_hud_policy::mutation::evaluate_mutation` or `evaluate_batch` in active runtime paths.
3. [Observed] `tze_hud_policy` defines `ArbitrationTelemetryEvent` and emits evaluator-side events, but the runtime does not currently forward those events through the telemetry pipeline.
4. [Inferred] Existing spec text marking policy telemetry and arbitration telemetry emission as v1 mandatory over-claims shipped behavior.

## Changes Applied

1. Updated `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`:
   - `Requirement: Policy Telemetry` scope changed to `v1-reserved` and language gated on explicit mutation-path wiring.
   - `Requirement: Arbitration Telemetry Events` scope changed to `v1-reserved` and language gated on explicit mutation-path wiring.
   - Added `hud-1jd5` note under `Spec-First Handoff Notes` clarifying evaluator events are not runtime emission evidence.

## Follow-On

Mutation-path pilot and telemetry conformance work remain the route to re-promote these requirements to `v1-mandatory` once runtime wiring and validation evidence exist.
