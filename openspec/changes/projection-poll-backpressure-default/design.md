## Context

`cooperative-hud-projection` currently permits 32 pending input items but defaults each `get_pending_input` response to eight items. A burst of small operator submissions therefore takes multiple LLM-facing polls even though the separate response-byte bound is the authoritative payload backpressure limit. The canonical contract already permits deployment-specific stricter bounds and caller-provided limits.

## Goals / Non-Goals

**Goals:**

- Reduce default polling round trips for a bounded 32-item input burst.
- Retain explicit, independent item-count and response-byte backpressure.
- Make the two cap interactions testable through normative scenarios.

**Non-Goals:**

- No adaptive polling algorithm, API/schema change, or change to caller-request clamping.
- No change to queue capacity, response-byte capacity, expiry, acknowledgement, or FIFO semantics.
- No production-code, runtime-threading, or deployment-configuration change in this spec-only bead.

## Decisions

### Fixed default of 32

Set the default `max_poll_items` to `32`, equal to `max_pending_input_items`. This lets a default poll drain a full count-bounded queue when the response-byte budget permits it. Keeping `8` preserves unnecessary round trips; an adaptive default would add policy and nondeterminism without improving the bounded v1 contract.

### Preserve byte backpressure as an independent limit

Keep `max_poll_response_bytes = 16384` unchanged and require the byte cap to stop delivery even when the item count allows more items. Raising the byte cap or coupling it to the count limit would broaden response size and weaken the existing backpressure contract.

### Specify both cap orders explicitly

Add one scenario using a configuration whose pending-input capacity exceeds 32, so a fitting 33-item queue is limited by the 32-item poll count cap without changing the default queue capacity. Add a second scenario where fewer than 32 items fit before the 16,384-byte cap. This makes the intended behavior unambiguous for the follow-on authority regression tests.

## Risks / Trade-offs

- **Larger default item batches** can return more small input items at once. → The unchanged 16,384-byte response budget and caller/deployment limits bound the response.
- **Implementation could treat count and bytes as interchangeable.** → Separate scenarios require each cap to bind while the other remains non-binding.
- **A deployment may require smaller batches.** → Existing deployment-specific stricter configuration remains authoritative.

## Migration Plan

1. Accept this delta as the canonical v1 default contract.
2. In the dependent implementation bead, update the projection default and add focused authority coverage for both cap-binding scenarios.
3. Preserve deployments that already configure a stricter item or byte limit; no stored data or wire migration is required.

## Open Questions

None. The fixed `32` default is selected by the prerequisite bead decision and remains reversible through deployment configuration.
