## Why

The resolved display profile is described as the runtime's budget envelope, but production consumers currently use it only for configuration-time override validation and truncation bounds. Session admission defaults, scene-resource totals, durable asset limits, and compositor caches remain independently sourced, so selecting a tighter profile does not produce one enforceable operating envelope.

## What Changes

- Define a frozen operational runtime envelope derived once from the selected display profile.
- Make profile ceilings govern session admission defaults, aggregate leased tiles, aggregate agent update rate, and aggregate agent-leased texture residency.
- Define which scene-resource, widget-source, compositor-cache, and font allocations debit the shared in-memory envelope, while keeping durable on-disk widget storage separately governed.
- Require one production-visible accounting snapshot so operators and tests can prove every governed consumer uses the selected profile.
- Preserve the Windows-only v1 execution boundary and exclude cadence/quiescence, device profiles, and device-specific implementation.
- **BREAKING**: broaden profile budget enforcement from configuration validation to runtime admission and aggregate resident-memory accounting; configurations that relied on independently larger runtime/cache defaults may be rejected or evicted under the selected profile.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `configuration`: define the selected profile as the sole frozen source for an operational budget envelope and make its ownership boundaries explicit.
- `runtime-kernel`: derive admission defaults and aggregate runtime accounting from that envelope.
- `resource-store`: debit decoded/resource/cache residency against the runtime envelope without conflating durable disk budgets.

## Impact

Affected surfaces include `DisplayProfile`/`ResolvedConfig`, `RuntimeContext`, session and lease admission, resource-store and protocol widget-store construction, compositor cache construction/accounting, startup diagnostics, and configuration/runtime/resource integration tests. No wire-protocol schema, device target, compositor cadence, or third-party dependency changes are proposed.
