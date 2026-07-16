## Why

The mandatory degradation ladder has no production consumer: active-frame telemetry never drives the runtime controller, compositor policy remains nominal, and the current broadcast transport can lose transactional notices. Owner decision `hud-o5snv` selected a cadence-derived immutable contract so production wiring can proceed without inventing device lanes or misrepresenting the windowed timing boundary.

## What Changes

- Derive an immutable degradation envelope from the effective Windows presentation cadence, preserving the 60 Hz calibration while using elapsed monotonic windows and deterministic p95 sampling.
- Define an authoritative active-frame workload metric for degradation without changing the existing Stage-1-to-Stage-7 `frame_time_us` latency contract.
- Define post-transition reset, one-level-at-a-time behavior, and bounded quiescent recovery without synthetic idle presents.
- Make the runtime controller the sole frame-history/transition authority and apply an explicit N-to-N+1 compositor policy from an atomic stable-`SceneId` scene snapshot.
- Route level notices through a bounded never-drop lane and send current degradation state after the scene snapshot on new and resumed sessions.
- Append exact protocol values for texture-quality reduction and emergency rendering while preserving all existing numeric values and blocking unspecified values from policy application.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `runtime-kernel`: replace the fixed-frame degradation trigger/recovery contract with the owner-approved cadence-derived envelope, workload metric, quiescent recovery, sole-authority, and atomic render-policy rules.
- `validation-framework`: require degradation timing provenance, per-frame applied-level telemetry, structured transition telemetry, and deterministic production-path parity evidence.
- `session-protocol`: make exact runtime-level mapping append-only, transactional delivery bounded/never-drop, and snapshot/current-state ordering normative.

## Impact

Affected surfaces are RFC 0002 and RFC 0005, runtime degradation/controller and windowed/headless frame loops, compositor render policy, telemetry schema, session protobuf and session server transport. No new dependency, device target, stereo surface, or 90/120 Hz validation lane is introduced.
