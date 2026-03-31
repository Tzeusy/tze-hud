# Tasks

## 1. Component Profile

- [ ] 1.1 Create `profiles/exemplar-status-bar/profile.toml` with `name = "exemplar-status-bar"`, `version = "1.0.0"`, `component_type = "status-bar"`, and description
- [ ] 1.2 Create `profiles/exemplar-status-bar/zones/status-bar.toml` with rendering overrides: `font_family = "monospace"`, `font_size_px = 16.0`, `text_color = "{{color.text.secondary}}"`, `backdrop_color = "{{color.backdrop.default}}"`, `backdrop_opacity = 0.9`, `margin_horizontal = "{{spacing.padding.medium}}"`, `margin_vertical = 0.0`
- [ ] 1.3 Verify profile loads: add `profiles/exemplar-status-bar/` to a test config's `[component_profile_bundles].paths` and confirm the runtime loads it without validation errors
- [ ] 1.4 Verify OpaqueBackdrop readability: confirm the effective RenderingPolicy for status-bar passes the OpaqueBackdrop check (backdrop present, opacity 0.9 >= 0.8)

## 2. Integration Tests — Merge-by-Key Contention

- [ ] 2.1 Add test `exemplar_status_bar_three_agents_coexist`: three namespaces publish different merge_keys ("weather", "battery", "time") to status-bar zone; assert 3 active publications coexist
- [ ] 2.2 Add test `exemplar_status_bar_key_update_replaces`: agent publishes merge_key "weather" twice with different values; assert publication count unchanged and content reflects second value
- [ ] 2.3 Add test `exemplar_status_bar_key_removal_empty_value`: agent publishes merge_key "weather" with empty-string value; assert the publication record exists but entry value is empty (compositor skip convention)
- [ ] 2.4 Add test `exemplar_status_bar_key_removal_ttl_expiry`: agent publishes merge_key "weather" with short TTL; call `sweep_expired_zone_publications` after simulated time advance; assert publication removed
- [ ] 2.5 Add test `exemplar_status_bar_max_keys_eviction`: publish 33 distinct merge_keys to status-bar zone (max_keys: 32); assert oldest key evicted and 32 keys remain

## 3. Integration Tests — Multi-Agent Isolation

- [ ] 3.1 Add test `exemplar_status_bar_clear_per_publisher`: three agents publish; one calls `clear_zone_for_publisher`; assert only that agent's publications removed, others intact
- [ ] 3.2 Add test `exemplar_status_bar_lease_expiry_isolation`: three agents have active leases and publications; one lease expires; assert only that agent's publications cleared via `clear_zone_publications_for_namespace`

## 4. Integration Tests — MCP Tool

- [ ] 4.1 Add test `exemplar_status_bar_mcp_publish_single_key`: call `handle_publish_to_zone` with status_bar content and merge_key; assert success and active publication
- [ ] 4.2 Add test `exemplar_status_bar_mcp_multi_agent`: call `handle_publish_to_zone` three times with different namespaces and merge_keys; assert 3 coexisting publications
- [ ] 4.3 Add test `exemplar_status_bar_mcp_key_update`: publish then re-publish same merge_key with updated value; assert publication count stable and value updated

## 5. User-Test Script

- [ ] 5.1 Create user-test script `scripts/exemplar_status_bar.py` (or extend existing user-test framework) that publishes to a live tze_hud instance: step 1-3 (three agents publish different keys), step 4 (visual check), step 5-6 (one agent updates), step 7-8 (key removal via empty value), step 9-10 (TTL expiry)
- [ ] 5.2 Add a 2-second pause between steps for visual verification; print expected state at each checkpoint
- [ ] 5.3 Verify script runs against deployed tze_hud.exe with MCP HTTP enabled

## 6. Documentation

- [ ] 6.1 Add exemplar reference to the change's spec as the canonical status-bar zone exemplar
- [ ] 6.2 Verify all spec scenarios from `exemplar-status-bar/spec.md` have corresponding test coverage (tasks 2.x, 3.x, 4.x map to spec scenarios)
