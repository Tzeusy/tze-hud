> **Implementation prerequisites:** This exemplar requires the following compositor/runtime changes that are not yet landed:
> - `render_zone_content()` in `renderer.rs` breaks after the first publication match — MergeByKey contention requires collecting and rendering all active publications (one per merge key). Currently only the latest publication is rendered.

# Exemplar: Status Bar Zone

A polished zone exemplar defining the visual, behavioral, and test contract for the `status-bar` zone type. Proves merge-by-key contention, multi-agent coexistence, and chrome-layer always-on-top rendering through a concrete component profile and end-to-end test scenarios.

## ADDED Requirements

### Requirement: Status Bar Visual Specification
The status-bar zone exemplar SHALL render as a vertical stack positioned at the right edge of the display, attached to the chrome layer (above all content tiles). The visual treatment SHALL be defined by a component profile with the following effective rendering policy:

- **Geometry:** `Relative { x_pct: 0.92, y_pct: 0.10, width_pct: 0.07, height_pct: 0.40 }` (7% screen width, 40% screen height, positioned at right edge with 8% margin from right and 10% from top)
- **Backdrop:** opaque dark background using `color.backdrop.default` token at 90% opacity (`backdrop_opacity = 0.9`). This satisfies the `OpaqueBackdrop` readability requirement (opacity >= 0.8).
- **Text:** secondary text color using `color.text.secondary` token (#B0B0B0 canonical fallback) for readability without being visually distracting
- **Typography:** monospace font family (`font_family = "monospace"`), body size via token (`font_size_px = "{{typography.body.size}}"`)
- **Layout:** key-value pairs rendered as a vertical stack with `spacing.padding.medium` between entries
- **Layer:** chrome layer — always visible, never occluded by agent content tiles

#### Scenario: Status bar renders at right edge with opaque backdrop
- **WHEN** the status-bar zone has one or more active publications
- **THEN** the compositor SHALL render a vertical stack at the right edge (92% x-position) with a dark backdrop at 90% opacity, and all active key-value pairs SHALL be visible in secondary text color using monospace font at the resolved body size, stacked vertically

#### Scenario: Status bar readability passes OpaqueBackdrop check
- **WHEN** the exemplar component profile is loaded and its effective RenderingPolicy is validated
- **THEN** the OpaqueBackdrop readability check SHALL pass because `backdrop_opacity = 0.9` exceeds the 0.8 threshold and `backdrop` color is present and not fully transparent

#### Scenario: Status bar chrome layer renders above content tiles
- **WHEN** agent content tiles exist in the content layer AND the status-bar zone has active publications
- **THEN** the status-bar zone SHALL render above all content-layer tiles, never occluded by agent-owned tiles

---

### Requirement: Status Bar Component Profile
The exemplar SHALL ship as a component profile directory conforming to the component-shape-language specification. The profile directory structure SHALL be:

```
profiles/exemplar-status-bar/
  profile.toml
  zones/
    status-bar.toml
```

**profile.toml:**
```toml
name = "exemplar-status-bar"
version = "1.0.0"
description = "Polished status bar exemplar proving merge-by-key multi-agent coexistence"
component_type = "status-bar"
```

**zones/status-bar.toml:**
```toml
# Typography — monospace for aligned key-value display
font_family = "monospace"
font_size_px = "{{typography.body.size}}"

# Text — secondary color for non-distracting readability
text_color = "{{color.text.secondary}}"

# Backdrop — dark, nearly opaque (satisfies OpaqueBackdrop >= 0.8)
backdrop_color = "{{color.backdrop.default}}"
backdrop_opacity = 0.9

# Margins — consistent spacing between key-value pairs
margin_horizontal = "{{spacing.padding.medium}}"
margin_vertical = 0.0
```

#### Scenario: Profile loads with correct component type
- **WHEN** the runtime scans the `profiles/exemplar-status-bar/` directory
- **THEN** the profile SHALL load successfully with `component_type = "status-bar"` and be available for selection in `[component_profiles]`

#### Scenario: Token references resolve from global token map
- **WHEN** the profile's `zones/status-bar.toml` references `{{color.text.secondary}}`
- **THEN** the reference SHALL resolve against the global token map (or canonical fallback #B0B0B0) and the effective RenderingPolicy SHALL have `text_color` set to the resolved value

#### Scenario: Profile activatable in configuration
- **WHEN** the HUD configuration contains `[component_profiles] status-bar = "exemplar-status-bar"`
- **THEN** the runtime SHALL activate the exemplar profile for the status-bar component type, applying its zone rendering overrides to the status-bar zone's effective RenderingPolicy

---

### Requirement: Merge-by-Key Contention Semantics
The status-bar zone SHALL use `ContentionPolicy::MergeByKey { max_keys: 32 }`. Each `publish_to_zone` call SHALL carry a `merge_key` that matches the single entry key in the `StatusBarPayload.entries` map. The contention policy SHALL enforce:

1. **Key coexistence:** Publications with different `merge_key` values SHALL coexist as independent entries in the zone's active publications. Each key-value pair is rendered simultaneously.
2. **Key replacement:** A new publication with the same `merge_key` as an existing publication SHALL replace the existing publication's content. The display SHALL show the updated value.
3. **Key removal via empty value:** Publishing a `StatusBarPayload` with an empty string value (`""`) for a key SHALL cause the compositor to skip rendering that entry, effectively removing the key from the visible display.
4. **Key removal via TTL expiry:** When a publication's `expires_at_wall_us` passes, the runtime's `sweep_expired_zone_publications` SHALL remove the record, removing the key from the display.
5. **Max keys enforcement:** When the number of distinct merge keys reaches `max_keys` (32), additional publications with new keys SHALL evict the oldest key to make room.

#### Scenario: Three agents publish different keys — all coexist
- **WHEN** agent A publishes `merge_key: "weather"` with `entries: {"weather": "72F"}` AND agent B publishes `merge_key: "battery"` with `entries: {"battery": "85%"}` AND agent C publishes `merge_key: "time"` with `entries: {"time": "3:42 PM"}`
- **THEN** the status-bar zone SHALL have exactly 3 active publications AND all three key-value pairs SHALL be simultaneously visible

#### Scenario: Agent updates its own key — value replaced
- **WHEN** agent A has an active publication with `merge_key: "weather"` showing `"72F"` AND agent A publishes a new `merge_key: "weather"` with `entries: {"weather": "75F"}`
- **THEN** the status-bar zone SHALL still have the same number of active publications AND the "weather" entry SHALL display `"75F"` (not `"72F"`)

#### Scenario: Key removed by empty value publish
- **WHEN** agent A has an active publication with `merge_key: "weather"` showing `"72F"` AND agent A publishes `merge_key: "weather"` with `entries: {"weather": ""}`
- **THEN** the compositor SHALL skip rendering the "weather" entry AND the remaining keys from other agents SHALL continue displaying

#### Scenario: Key removed by TTL expiry
- **WHEN** agent A publishes `merge_key: "weather"` with `ttl_us: 5_000_000` (5 seconds) AND 5 seconds elapse
- **THEN** `sweep_expired_zone_publications` SHALL remove the "weather" publication AND the remaining keys from other agents SHALL continue displaying

#### Scenario: Max keys eviction
- **WHEN** 32 distinct merge keys are active in the status-bar zone AND a 33rd unique merge key is published
- **THEN** the oldest publication (by insertion order) SHALL be evicted to make room AND the new key SHALL be stored

---

### Requirement: Multi-Agent Coexistence
The status-bar zone SHALL support simultaneous publishing from multiple independent agents, each identified by a distinct `namespace` parameter. Agent isolation SHALL be enforced by the zone's contention policy (merge-by-key), not by agent-side coordination.

#### Scenario: Three agents with distinct namespaces publish simultaneously
- **WHEN** namespace `"agent-weather"` publishes `merge_key: "weather"` AND namespace `"agent-power"` publishes `merge_key: "battery"` AND namespace `"agent-clock"` publishes `merge_key: "time"`
- **THEN** all three publications SHALL coexist AND each agent's publication SHALL be independently updateable

#### Scenario: One agent's clear does not affect other agents
- **WHEN** three agents have active publications AND namespace `"agent-weather"` calls `clear_zone_for_publisher("status-bar", "agent-weather")`
- **THEN** only the "agent-weather" publications SHALL be removed AND publications from `"agent-power"` and `"agent-clock"` SHALL remain active

#### Scenario: Agent lease expiry removes only that agent's publications
- **WHEN** namespace `"agent-weather"` has an active lease and publication AND the lease expires or is revoked
- **THEN** only `"agent-weather"` publications SHALL be cleared from the status-bar zone AND other agents' publications SHALL remain

---

### Requirement: Coalesced Update Delivery
Status-bar publications use the state-stream delivery class. Coalescing is achieved through the MergeByKey contention policy at the scene graph level — when multiple publishes for the same key arrive between compositor frames, the scene graph retains only the latest value per key. This is not a separate coalescing mechanism; it is an inherent property of MergeByKey replacement semantics. The compositor reads the zone's resolved occupancy (post-contention) at render time, not individual publish events.

#### Scenario: Rapid updates within one frame coalesce
- **WHEN** agent A publishes `merge_key: "weather"` with value `"70F"` then `"71F"` then `"72F"` within the same compositor frame
- **THEN** the rendered output SHALL show `"72F"` (the latest value) and the zone SHALL have exactly one active publication for merge_key `"weather"`

---

### Requirement: MCP Integration Test Scenarios
The exemplar SHALL include integration test scenarios exercisable via the `publish_to_zone` MCP tool. Each test scenario uses the MCP JSON-RPC interface with `{"type":"status_bar","entries":{...}}` content and optional `merge_key` parameter.

#### Scenario: MCP publish single status key
- **WHEN** an MCP client calls `publish_to_zone` with `{"zone_name":"status-bar","content":{"type":"status_bar","entries":{"weather":"72F"}},"merge_key":"weather"}`
- **THEN** the call SHALL succeed AND the status-bar zone SHALL have one active publication with merge_key `"weather"`

#### Scenario: MCP publish from multiple agents with different keys
- **WHEN** MCP client A (namespace `"agent-weather"`) publishes `merge_key: "weather"` with entries `{"weather":"72F"}` AND MCP client B (namespace `"agent-power"`) publishes `merge_key: "battery"` with entries `{"battery":"85%"}` AND MCP client C (namespace `"agent-clock"`) publishes `merge_key: "time"` with entries `{"time":"3:42 PM"}`
- **THEN** the status-bar zone SHALL have exactly 3 active publications AND each publication's merge_key SHALL correspond to its entry key

#### Scenario: MCP key update preserves other keys
- **WHEN** three agents have active publications (weather, battery, time) AND agent A publishes a new `merge_key: "weather"` with entries `{"weather":"75F"}`
- **THEN** the status-bar zone SHALL still have exactly 3 active publications AND the "weather" publication SHALL show `"75F"` AND "battery" and "time" SHALL be unchanged

#### Scenario: MCP key removal via empty value
- **WHEN** agent A publishes `merge_key: "weather"` with entries `{"weather":""}` (empty value)
- **THEN** the publication SHALL be stored (merge-by-key replacement) AND the compositor SHALL skip rendering this entry (empty value convention)

---

### Requirement: User-Test Scenario
The exemplar SHALL include a user-test scenario that visually verifies multi-agent status bar rendering on a live display. The scenario SHALL be executable via the user-test skill framework.

**Scenario script:**
1. Agent A publishes `merge_key: "weather"` with value `"72F Sunny"`
2. Agent B publishes `merge_key: "battery"` with value `"85%"`
3. Agent C publishes `merge_key: "time"` with value `"3:42 PM"`
4. Visual check: all three key-value pairs visible in the status bar
5. Agent A updates `merge_key: "weather"` to `"75F Cloudy"`
6. Visual check: "weather" shows updated value; "battery" and "time" unchanged
7. Agent A publishes empty value for `merge_key: "weather"`
8. Visual check: "weather" key is no longer visible; "battery" and "time" remain
9. Wait for agent B's TTL to expire (if TTL was set)
10. Visual check: "battery" key is no longer visible; "time" remains

#### Scenario: Full user-test sequence completes
- **WHEN** the user-test script executes steps 1-10 against a live tze_hud instance
- **THEN** each visual check SHALL confirm the expected key-value display state AND no unexpected keys SHALL appear or disappear
