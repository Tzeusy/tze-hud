## Why

tze_hud exposes zone publishing via MCP (JSON-RPC 2.0 over Streamable HTTP), but there is no discoverable, portable skill artifact that tells an LLM *how* to use it. Today an agent needs the `user-test` internal skill or manual prompt engineering to learn the endpoint, auth scheme, available methods, message shape, and zone semantics. Publishing a `/hud-publish` skill following the agentskills.io spec makes tze_hud a **first-class LLM-addressable display target**: any Claude Code session (or compatible agent runtime) can install the skill, point it at a running HUD instance, and start publishing to zones with zero bespoke integration.

## What Changes

- **New publishable skill** (`th-hud-publish`): A self-contained `.claude/skills/` directory with SKILL.md, helper scripts, and MCP settings that teaches any LLM agent to discover zones, publish content, and interpret results against a user's running HUD.
- **MCP settings template**: A `.claude/settings.json` snippet (or standalone template) that configures the HUD's Streamable HTTP MCP server as a tool provider, with PSK auth, so LLM tool calls route directly to the HUD runtime.
- **Reference publish script**: A lightweight Python 3 publish client (derived from `user-test/scripts/publish_zone_batch.py`) bundled inside the skill for batch and diagnostic use.
- **No protocol changes**: The existing MCP bridge surface (`publish_to_zone`, `list_zones`, restricted `list_scene`) is unchanged. This change is purely an ergonomic/discoverability layer.

## Capabilities

### New Capabilities

- `hud-publish-skill`: Defines the skill artifact structure, SKILL.md content contract, MCP settings template, and bundled scripts that allow an LLM agent to interact with the HUD's MCP guest surface (zone publishing, zone listing, basic scene introspection).

### Modified Capabilities

_(none — existing MCP bridge protocol and zone model are unchanged)_

## Impact

- **New files**: `.claude/skills/th-hud-publish/` directory tree (SKILL.md, scripts/, settings template)
- **Existing code**: No runtime code changes. The skill consumes the existing MCP HTTP surface as-is.
- **Dependencies**: Python 3 (for bundled publish script); no new Rust/Cargo dependencies.
- **APIs**: Documents and formalizes the guest MCP tool set (`publish_to_zone`, `list_zones`) as a stable, skill-addressable contract. Any future changes to these methods must update the skill artifact.
- **Users**: Anyone running the HUD application on their interactive OS can share the skill (or point another agent at their MCP endpoint) to give LLMs GUI presence on their screen.
