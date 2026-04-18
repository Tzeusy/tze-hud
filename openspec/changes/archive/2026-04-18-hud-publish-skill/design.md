## Context

tze_hud already exposes a guest MCP surface over Streamable HTTP (JSON-RPC 2.0) with two primary methods (`publish_to_zone`, `list_zones`) and one restricted method (`list_scene`). Authentication uses a pre-shared key via `Authorization: Bearer <PSK>` header. The existing `user-test` skill demonstrates this flow for internal cross-machine validation, but it is tightly coupled to the deploy/SSH/launch workflow and not reusable as a standalone LLM interaction surface.

The agentskills.io specification defines a portable skill format: a `SKILL.md` frontmatter+markdown file that teaches an LLM agent *when* and *how* to use a capability, plus optional bundled scripts and reference materials. Claude Code discovers skills from `.claude/skills/` directories and loads the matching SKILL.md when the skill's trigger description matches.

## Goals / Non-Goals

**Goals:**
- Publish a skill artifact at `.claude/skills/th-hud-publish/` that any Claude Code session can install and use
- Provide an MCP settings template that configures the HUD as a tool provider (Streamable HTTP, PSK auth)
- Bundle a reference publish script for batch operations and diagnostics
- Document the full guest MCP tool set with message shapes, zone semantics, and error handling
- Make the skill self-contained: an agent with only the skill + MCP settings + a running HUD instance can publish to zones

**Non-Goals:**
- Changing the MCP protocol surface (no new methods, no schema changes)
- Supporting resident/gRPC session workflows (those require lease management, bidirectional streams — out of scope for a guest skill)
- Creating an npm/pip package or CLI tool (the skill *is* the distribution format)
- Automated HUD deployment or launch (that remains in `user-test`)
- Eval suite or benchmark harness (can be added later per skill-creator workflow)

## Decisions

### 1. Skill naming: `th-hud-publish`

**Choice**: Prefix with `th-` (tze_hud) to namespace within the skills directory; use `hud-publish` as the action verb.

**Rationale**: Avoids collision with generic "publish" skills. The `th-` prefix is short, grep-friendly, and consistent with the project's `tze_hud` identity. Users invoke as `/hud-publish` (the `th-` prefix is the directory name, not the slash-command name).

**Alternative considered**: `tze-hud-publish` — too long for a skill name; `publish-to-zone` — too specific (skill covers list_zones and list_scene too).

### 2. MCP settings as a template file, not hardcoded

**Choice**: Ship a `settings.template.json` inside the skill directory. The SKILL.md instructions tell the agent (or user) to copy it to `.claude/settings.json` and fill in the host/PSK values.

**Rationale**: The HUD server host and PSK vary per user. Hardcoding `tzehouse-windows.parrot-hen.ts.net` (the dev/test host) would be wrong for anyone else. A template with placeholder values (`${HUD_MCP_HOST}`, `${HUD_MCP_PSK}`) is portable.

**Alternative considered**: Inline the settings JSON in SKILL.md — harder to copy-paste correctly; generating settings via a script — over-engineering for a JSON file.

### 3. Reuse `publish_zone_batch.py` pattern, don't fork it

**Choice**: Create a new `scripts/publish.py` derived from `user-test/scripts/publish_zone_batch.py` but stripped of user-test-specific assumptions (no `--list-zones` coupling, cleaner CLI, proper exit codes).

**Rationale**: The existing script is proven but entangled with the user-test workflow. A clean derivative keeps the same JSON-RPC 2.0 client logic while being independently usable.

### 4. SKILL.md teaches zone semantics, not just API mechanics

**Choice**: Include a "Zone Model" section in SKILL.md that explains contention policies, TTL, merge keys, ephemeral vs durable zones, and the zone content types.

**Rationale**: An LLM that only knows the method signature will misuse zones (e.g., publishing stream_text to a status-bar zone, or omitting merge_key for a merge-by-key zone). The skill must encode enough domain knowledge for correct use. This follows the agentskills.io principle that skills should prevent misuse, not just enable use.

### 5. No auth credential in settings — use env var reference

**Choice**: The settings template references `${HUD_MCP_PSK}` as the bearer token. The actual secret lives in the user's environment, never in a committed file.

**Rationale**: Security. PSKs must not appear in version-controlled settings files. Claude Code expands `${ENV_VAR}` references in settings.json headers at runtime.

## Risks / Trade-offs

- **[Risk] MCP method surface changes upstream** → Mitigation: The proposal explicitly notes that future MCP bridge changes must update the skill artifact. Add a version note in SKILL.md referencing the protocol version.
- **[Risk] Zone names are instance-specific** → Mitigation: SKILL.md teaches the agent to always call `list_zones` first to discover available zones, never hardcode zone names.
- **[Risk] Skill becomes stale if zone content types expand** → Mitigation: Reference the protobuf `ZoneContent` oneof as the source of truth; SKILL.md documents v1 types with a pointer to the proto file.
- **[Trade-off] Bundled Python script vs pure MCP tool calls** → The script is for batch/diagnostic use; normal LLM interaction should use MCP tool calls directly. Both paths are documented.
