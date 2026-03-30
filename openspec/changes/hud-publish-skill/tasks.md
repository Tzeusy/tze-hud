## 1. Skill Directory & SKILL.md

- [ ] 1.1 Create `.claude/skills/th-hud-publish/` directory structure
- [ ] 1.2 Write `SKILL.md` frontmatter (name, trigger description per agentskills.io)
- [ ] 1.3 Write SKILL.md body: overview, when-to-use, MCP setup instructions
- [ ] 1.4 Write SKILL.md body: guest MCP tool reference (`publish_to_zone`, `list_zones`) with JSON-RPC examples
- [ ] 1.5 Write SKILL.md body: zone model section (content types, contention policies, TTL, merge keys, ephemeral vs durable)
- [ ] 1.6 Write SKILL.md body: discovery-first workflow (list_zones → select zone → publish_to_zone)
- [ ] 1.7 Add protocol version compatibility note and protobuf pointer to SKILL.md

## 2. MCP Settings Template

- [ ] 2.1 Create `settings.template.json` with Streamable HTTP MCP server entry, placeholder host, env var PSK reference
- [ ] 2.2 Verify template contains no hardcoded secrets or hostnames

## 3. Bundled Publish Script

- [ ] 3.1 Create `scripts/publish.py` — JSON-RPC 2.0 client using only Python 3 stdlib
- [ ] 3.2 Implement `--url`, `--psk-env`, `--messages-file`, `--list-zones` CLI arguments
- [ ] 3.3 Implement batch publish loop with per-message JSON result output
- [ ] 3.4 Implement proper exit codes (0 = success, non-zero = any failure) and missing-PSK error handling
- [ ] 3.5 Test script against a running HUD instance (manual verification)

## 4. Integration & Validation

- [ ] 4.1 Verify skill loads in Claude Code (trigger description matches relevant queries)
- [ ] 4.2 Verify MCP settings template works when copied to `.claude/settings.json` with real host/PSK
- [ ] 4.3 End-to-end: agent discovers zones via `list_zones` MCP tool call, then publishes to a zone via `publish_to_zone`
