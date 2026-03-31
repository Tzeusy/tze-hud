## Why

The status-bar zone is a core v1 zone type (chrome layer, merge-by-key contention, always-on-top) that exercises three capabilities no other zone demonstrates simultaneously: multi-agent key coexistence, key-level replacement semantics, and chrome-layer rendering above all content tiles. An exemplar definition — complete with visual spec, behavioral contract, and multi-agent test scenario — provides a concrete reference for implementers and validates that the zone publishing engine, contention policy, and compositor chrome layer work end-to-end.

## What Changes

- Define the `exemplar-status-bar` zone exemplar: a full-width thin strip at the display's bottom edge, chrome layer, opaque dark backdrop (90% opacity), secondary text color (#B0B0B0), monospace body font at 16px, horizontal key-value pairs with consistent spacing
- Specify merge-by-key contention semantics for multi-agent coexistence: each publish carries a key (e.g., "weather", "battery", "time", "agent-status"); same key replaces value, different keys coexist, empty value or TTL expiry removes the key
- Define coalesced update delivery (state-stream message class) — multiple rapid updates to the same key within a frame are coalesced to the latest value
- Add multi-agent test scenarios exercising `publish_to_zone` MCP calls from 3 agents with different merge keys, verifying key coexistence, key update, and key removal
- Add a user-test scenario: 3 agents publish different status keys, then one updates its key — verify merge semantics visually

## Capabilities

### New Capabilities
- `exemplar-status-bar`: Zone exemplar defining the visual, behavioral, and test contract for the status-bar zone — chrome-layer merge-by-key multi-agent coexistence

### Modified Capabilities

## Impact

- No new Rust code changes — the status-bar zone type, `StatusBarPayload`, `MergeByKey` contention, and `publish_to_zone` MCP tool already exist in `crates/tze_hud_scene/` and `crates/tze_hud_mcp/`
- Exemplar serves as reference documentation and test contract for the existing implementation
- Test scenarios exercise existing `publish_to_zone` with `{"type":"status_bar","entries":{...}}` payloads and `merge_key` parameter
- User-test scenario extends the `.claude/skills/user-test/` framework
