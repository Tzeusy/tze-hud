## Why

The default `max_poll_items = 8` makes a full 32-item operator-input burst require up to four polling round trips, even when the independently bounded response-byte budget can carry more items. Raising only the default count limit reduces LLM-facing polling overhead while retaining the existing byte backpressure contract.

## What Changes

- Change the cooperative HUD projection v1 default `max_poll_items` from `8` to `32`, matching `max_pending_input_items`.
- Preserve `max_pending_input_items = 32` and `max_poll_response_bytes = 16384` unchanged.
- Add explicit scenarios proving the poll item-count limit and response-byte limit remain independently enforced when callers omit limits or request larger values.
- Keep deployment-configured stricter limits authoritative; this change does not alter request schemas, queue semantics, or expiry policy.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cooperative-hud-projection`: Update the Bounded Backpressure and Expiry default and specify independent count and byte-cap enforcement.

## Impact

- Affected specification: `openspec/specs/cooperative-hud-projection/spec.md`.
- Follow-on implementation work will update the projection authority's default and focused regression coverage under `crates/tze_hud_projection/`.
- No production code, protocol schema, MCP parameter, dependency, or runtime-threading change is included in this change.
