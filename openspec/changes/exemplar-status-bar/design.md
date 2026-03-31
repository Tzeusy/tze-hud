## Context

The status-bar zone type is already implemented in `tze_hud_scene`:
- `ZoneRegistry::with_defaults()` registers `"status-bar"` with `ContentionPolicy::MergeByKey { max_keys: 32 }`, `LayerAttachment::Chrome`, `GeometryPolicy::EdgeAnchored { edge: Bottom, height_pct: 0.04, width_pct: 1.0 }`, and `ZoneMediaType::KeyValuePairs`
- `StatusBarPayload { entries: HashMap<String, String> }` carries key-value pairs as `ZoneContent::StatusBar`
- `publish_to_zone` MCP tool accepts `{"type":"status_bar","entries":{...}}` with an optional `merge_key` parameter
- `SceneGraph::publish_to_zone` applies `MergeByKey` contention: same key replaces, different keys coexist up to `max_keys`
- `clear_zone_for_publisher` removes all publications by a specific namespace
- TTL-based expiry via `expires_at_wall_us` is supported; `sweep_expired_zone_publications` reaps expired records

The component-shape-language epic (hud-sc0a) is fully implemented. This means design tokens, extended `RenderingPolicy`, component profiles with `zones/{zone_type}.toml` overrides, and zone readability enforcement (OpaqueBackdrop for status-bar) are all live. The exemplar can and should ship as a concrete component profile with a `zones/status-bar.toml` override file referencing tokens.

What does not yet exist is a cohesive exemplar document that ties visual rendering spec, behavioral contract, component profile definition, and multi-agent test scenarios into a single reference.

## Goals / Non-Goals

**Goals:**
- Define a complete zone exemplar covering visual spec (geometry, typography, colors), behavioral contract (merge-by-key semantics), and test scenarios (multi-agent coexistence)
- Ground all visual parameters in existing design tokens from the component-shape-language spec
- Provide MCP-callable test scenarios that validate merge-by-key contention end-to-end
- Include a user-test scenario for visual verification of multi-agent status bar rendering

**Non-Goals:**
- Modifying the existing Rust implementation (zone type, contention policy, or MCP tool)
- Rendering engine changes (the compositor's chrome-layer rendering is already implemented)
- Widget-based status bar alternatives (the exemplar covers the zone-only pattern)
- New design token definitions (the exemplar uses existing canonical tokens)

## Decisions

### Key removal via empty-entries publish
**Decision:** Publishing a `StatusBarPayload` with an empty value string for a key removes that key from the display. The scene engine already handles this via `MergeByKey` replacement — the compositor skips rendering entries with empty values.
**Rationale:** Reuses existing contention mechanics. No new API surface needed. Consistent with TTL-based expiry as a complementary removal mechanism.
**Alternative considered:** A dedicated `clear_zone_key` mutation. Rejected because it adds API surface for something achievable with the existing publish path plus empty-value convention.

### Merge key equals status-bar display key
**Decision:** The `merge_key` parameter in `publish_to_zone` SHALL equal the status-bar entry key (e.g., `merge_key: "weather"` paired with `entries: {"weather": "72F"}`). This 1:1 mapping means each logical status key is one MergeByKey record.
**Rationale:** The existing `StatusBarPayload.entries` is a `HashMap<String, String>`, but the MergeByKey contention operates on the `merge_key` field of `ZonePublishRecord`, not on individual entries within the payload. By convention, each publish carries exactly one entry whose key matches `merge_key`, so the contention policy correctly replaces the value for that logical key.
**Alternative considered:** Multi-entry payloads with per-entry merge logic inside the zone engine. Rejected because it would fork contention semantics for one zone type.

### Test scenarios use MCP publish_to_zone with distinct namespaces
**Decision:** Multi-agent scenarios use three distinct `namespace` values (e.g., `"agent-weather"`, `"agent-power"`, `"agent-clock"`) to simulate independent agents.
**Rationale:** The namespace parameter in `publish_to_zone` already identifies the publishing agent. Using distinct namespaces proves that `MergeByKey` contention works across agent boundaries, not just within one agent's publications.

### Exemplar ships as a component profile + test contract
**Decision:** The exemplar produces a concrete component profile directory (`profiles/exemplar-status-bar/`) containing `profile.toml` and `zones/status-bar.toml`, plus integration test scenarios and a user-test script. No new Rust code is needed.
**Rationale:** The component-shape-language infrastructure is fully implemented. A real profile directory is the canonical way to define visual treatment for a zone type. The profile's `zones/status-bar.toml` overrides set the exemplar's visual parameters (monospace font, secondary text color, 90% backdrop opacity) while referencing design tokens. Integration tests and user-test scripts exercise the behavioral contract.

### Status-bar profile uses token references, not hardcoded values
**Decision:** The `zones/status-bar.toml` file references design tokens via `{{token.key}}` syntax rather than hardcoding hex colors or pixel sizes. The only literal values are font_size_px (16.0), backdrop_opacity (0.9), and font_family ("monospace") since these are exemplar-specific choices that intentionally differ from the canonical defaults.
**Rationale:** Token references enable the profile to inherit global theme changes. Hardcoded values are used only where the exemplar intentionally overrides the token default (e.g., 16px instead of the canonical body size, monospace instead of the canonical body family).

## Risks / Trade-offs

- **[Empty-value removal is a convention, not enforced]** The scene engine does not reject or special-case empty-value StatusBarPayload entries. The compositor must skip rendering them. If the compositor renders empty values, the status bar shows blank entries. → Mitigation: test scenarios explicitly verify that empty-value publish removes the key from visible output.
- **[merge_key/entries key mismatch]** If an agent publishes `merge_key: "A"` with `entries: {"B": "val"}`, the contention policy replaces records by merge_key "A" but the displayed key is "B". → Mitigation: the exemplar spec requires 1:1 correspondence and test scenarios verify it.
- **[max_keys=32 is generous]** The default zone allows 32 concurrent keys. A misbehaving agent could fill the status bar. → Mitigation: per-agent key budget is a post-v1 concern; the exemplar documents the max_keys limit and tests max_keys overflow behavior.
