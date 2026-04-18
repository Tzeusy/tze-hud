# hud-publish-skill Specification

## Purpose
TBD - created by archiving change hud-publish-skill. Update Purpose after archive.
## Requirements
### Requirement: Skill artifact directory structure

The project SHALL publish a skill at `.claude/skills/th-hud-publish/` containing at minimum:
- `SKILL.md` — frontmatter + markdown skill definition per agentskills.io spec
- `scripts/publish.py` — standalone Python 3 MCP publish client
- `settings.template.json` — MCP server configuration template with placeholder values

#### Scenario: Skill directory is complete
- **WHEN** an agent or user lists `.claude/skills/th-hud-publish/`
- **THEN** the directory SHALL contain `SKILL.md`, `scripts/publish.py`, and `settings.template.json`

#### Scenario: No external dependencies beyond Python 3 stdlib
- **WHEN** the bundled `scripts/publish.py` is executed
- **THEN** it SHALL use only Python 3 standard library modules (urllib, json, sys, os, argparse)

---

### Requirement: SKILL.md frontmatter follows agentskills.io spec

The `SKILL.md` file SHALL include YAML frontmatter with:
- `name`: `th-hud-publish` (letters, numbers, hyphens only)
- `description`: A trigger-oriented description starting with "Use when" that describes when an LLM should activate the skill (zone publishing, HUD interaction, display presence)

The description SHALL NOT summarize the skill's internal workflow.

#### Scenario: Frontmatter is parseable
- **WHEN** a skill loader parses the SKILL.md frontmatter
- **THEN** the `name` field SHALL equal `th-hud-publish` and the `description` field SHALL begin with "Use when"

#### Scenario: Description triggers on relevant queries
- **WHEN** an LLM agent is asked to "publish a message to the HUD", "show something on screen", "send content to a display zone", or "interact with the user's GUI display"
- **THEN** the skill description SHALL match and the skill SHALL be loaded

---

### Requirement: SKILL.md documents guest MCP tool set

The SKILL.md body SHALL document all v1 guest MCP methods with:
- Method name, parameter schema (name, type, required/optional), and return shape
- At minimum: `publish_to_zone` and `list_zones`
- Example JSON-RPC 2.0 request/response pairs for each method

#### Scenario: publish_to_zone method is documented
- **WHEN** an agent reads the SKILL.md
- **THEN** it SHALL find a complete specification of `publish_to_zone` including parameters: `zone_name` (string, required), `content` (string, required), `ttl_us` (uint64, optional), `merge_key` (string, optional), `namespace` (string, optional)

#### Scenario: list_zones method is documented
- **WHEN** an agent reads the SKILL.md
- **THEN** it SHALL find a complete specification of `list_zones` including its empty parameter set and its return shape (array of zone definitions with name, type, contention policy, accepted media types)

---

### Requirement: SKILL.md teaches zone model semantics

The SKILL.md SHALL include a zone model section explaining:
- Zone content types: `stream_text`, `notification`, `status_bar`, `solid_color`
- Contention policies: `LATEST_WINS`, `REPLACE`, `STACK`, `MERGE_BY_KEY`
- TTL semantics: microsecond units, 0 = zone default, auto-clear behavior
- Merge key semantics: required for `MERGE_BY_KEY` zones, ignored otherwise
- Ephemeral vs durable zones: fire-and-forget vs transactional acknowledgement

#### Scenario: Agent avoids merge_key misuse
- **WHEN** an agent reads the zone model section and encounters a zone with `LATEST_WINS` contention
- **THEN** the documentation SHALL make clear that `merge_key` is ignored for that contention policy

#### Scenario: Agent understands TTL units
- **WHEN** an agent reads a TTL value of `60000000`
- **THEN** the documentation SHALL make clear this is 60 seconds (microsecond units)

---

### Requirement: SKILL.md prescribes discovery-first workflow

The SKILL.md SHALL instruct agents to call `list_zones` before any `publish_to_zone` call to discover available zone names, types, and policies. Agents SHALL NOT hardcode zone names.

#### Scenario: Agent discovers zones before publishing
- **WHEN** an agent follows the SKILL.md workflow to publish content
- **THEN** it SHALL first call `list_zones`, inspect the response, select an appropriate zone by name and type, and then call `publish_to_zone` with a discovered zone name

#### Scenario: Agent handles empty zone list
- **WHEN** `list_zones` returns an empty array
- **THEN** the agent SHALL report that no zones are available and SHALL NOT attempt a `publish_to_zone` call

---

### Requirement: MCP settings template is portable

The `settings.template.json` SHALL define an MCP server entry with:
- `type`: `"url"` (Streamable HTTP transport)
- `url`: a placeholder value (e.g., `http://<HUD_HOST>:9090`) that the user replaces
- `headers.Authorization`: `"Bearer ${HUD_MCP_PSK}"` referencing an environment variable
- No hardcoded hostnames or credentials

#### Scenario: User configures settings for their HUD instance
- **WHEN** a user copies `settings.template.json` to `.claude/settings.json` and sets `HUD_MCP_PSK` in their environment
- **THEN** Claude Code SHALL resolve the env var reference and authenticate MCP tool calls with the bearer token

#### Scenario: Template does not contain secrets
- **WHEN** the `settings.template.json` file is inspected
- **THEN** it SHALL contain no actual PSK values, hostnames, or credentials — only placeholder references

---

### Requirement: Bundled publish script supports batch and diagnostic use

The `scripts/publish.py` SHALL:
- Accept `--url <mcp_url>` and `--psk-env <env_var_name>` for connection configuration
- Accept `--messages-file <path>` for batch publishing (JSON array of message objects)
- Accept `--list-zones` to call `list_zones` and print results
- Use JSON-RPC 2.0 protocol with proper `id`, `method`, `params` fields
- Print per-message success/failure results to stdout as JSON
- Exit with code 0 on full success, non-zero on any failure
- Use only Python 3 standard library (no pip dependencies)

#### Scenario: Batch publish with messages file
- **WHEN** `publish.py --url http://host:9090 --psk-env HUD_MCP_PSK --messages-file msgs.json` is executed
- **THEN** it SHALL read the JSON array from `msgs.json`, send one `publish_to_zone` JSON-RPC call per message, and print a JSON result array

#### Scenario: List zones for diagnostics
- **WHEN** `publish.py --url http://host:9090 --psk-env HUD_MCP_PSK --list-zones` is executed
- **THEN** it SHALL call `list_zones` and print the zone definitions to stdout as JSON

#### Scenario: Missing PSK environment variable
- **WHEN** the env var referenced by `--psk-env` is not set
- **THEN** the script SHALL exit with a non-zero code and a clear error message

---

### Requirement: Skill references protocol version

The SKILL.md SHALL include a version or compatibility note indicating which tze_hud MCP protocol version it targets (v1 guest surface). This enables agents to detect staleness if the protocol evolves.

#### Scenario: Version note is present
- **WHEN** an agent reads the SKILL.md
- **THEN** it SHALL find a compatibility note referencing "v1" or the specific protocol version, and a pointer to the authoritative protobuf definitions
