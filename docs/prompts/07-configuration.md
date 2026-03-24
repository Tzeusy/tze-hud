# Epic 7: Configuration and Profiles

> **Dependencies:** Epic 0 (ConfigLoader trait contract), Epic 4 (capability vocabulary), Epic 6 (capability negotiation in handshake)
> **Depended on by:** Epics 8, 9, 11 (policy reads config, events read quiet hours, shell reads chrome config)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/configuration/spec.md`
> **Secondary specs:** `lease-governance/spec.md` (budget defaults), `session-protocol/spec.md` (capability negotiation)

## Prompt

Create a `/beads-writer` epic for **configuration and profiles** — the TOML-based configuration system that owns the canonical capability vocabulary, display profile resolution, and runtime parameter governance.

### Context

Configuration is the single source of truth for capability names, display profiles, agent registration, zone registry defaults, and privacy settings. The existing codebase has minimal config handling. Epic 0 provides the `ConfigLoader` trait contract (parse→normalize→validate→freeze). The spec defines three built-in profiles: full-display, headless, and mobile (schema-reserved, startup error in v1).

### Epic structure

Create an epic with **4 implementation beads**:

#### 1. TOML schema and file loading (depends on Epic 0 ConfigLoader contract)
Implement the config file format per `configuration/spec.md` Requirement: TOML Schema.
- Single-file model in v1; `includes` field produces startup error
- File resolution order: CLI flag → env var → platform default paths
- All fields have documented defaults; absent sections use defaults
- Parse → normalize → validate → freeze into immutable runtime structs
- **Acceptance:** ConfigLoader trait tests from Epic 0 pass. Valid config loads successfully. Invalid config produces structured errors. `includes` field rejected.
- **Spec refs:** `configuration/spec.md` Requirement: TOML Schema, Requirement: Layered Config Composition (v1-reserved: hard error)

#### 2. Display profile resolution (depends on #1)
Implement profile selection per `configuration/spec.md` Requirement: Display Profiles.
- Built-in profiles: full-display (max_tiles=1024, target_fps=60), headless (max_tiles=256, offscreen), mobile (startup error)
- Auto-detection when `profile = "auto"`: headless if no display/Docker/software-only GPU, full-display if VRAM > 4GB and refresh >= 60Hz
- Custom profiles via `[display_profile].extends` — cannot exceed base profile budgets
- Mobile never auto-selected in v1
- **Acceptance:** Auto-detection scenarios from spec pass. Profile budget escalation prevented. Mobile profile rejected at startup. Headless profile works without display server.
- **Spec refs:** `configuration/spec.md` Requirement: Display Profiles, Requirement: Profile Auto-Detection

#### 3. Capability vocabulary validation (depends on #1)
Implement canonical capability enforcement per `configuration/spec.md` Requirement: Capability Vocabulary.
- 19 canonical v1 capabilities in snake_case: `create_tiles`, `modify_own_tiles`, `manage_tabs`, `manage_sync_groups`, `upload_resource`, `read_scene_topology`, `subscribe_scene_events`, `overlay_privileges`, `access_input_events`, `high_priority_z_order`, `exceed_default_budgets`, `read_telemetry`, `publish_zone:<zone_name>`, `publish_zone:*`, `emit_scene_event:<event_name>`, `resident_mcp`, `lease:priority:<N>`
- Legacy names (`read_scene`, `receive_input`, `zone_publish`) rejected with `CONFIG_UNKNOWN_CAPABILITY` and hint
- No synonyms, aliases, or camelCase permitted
- **Acceptance:** All canonical capabilities accepted. Legacy names rejected with structured error and hint. Unknown capabilities rejected. Parameterized capabilities (publish_zone:subtitle) parsed correctly.
- **Spec refs:** `configuration/spec.md` Requirement: Capability Vocabulary, lines 149-164

#### 4. Privacy, zone registry, and agent registration (depends on #1, #2)
Implement remaining config sections per `configuration/spec.md`.
- `[privacy]` section: `redaction_style` (pattern|blank only), quiet hours, viewer context defaults
- `[zone_registry]` section: static zone instance definitions per tab, geometry policies, contention policies
- `[agents]` section: per-agent capability grants, budget overrides, lease defaults
- Hot-reload where safe (zone definitions, agent budgets); immutable for profiles and capabilities
- **Acceptance:** Privacy settings loaded correctly. Zone registry matches test scene expectations. Agent grants validated against capability vocabulary. Hot-reload triggers config refresh without restart for allowed fields.
- **Spec refs:** `configuration/spec.md` Requirement: Privacy Configuration, Requirement: Zone Registry Configuration, Requirement: Agent Registration

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `configuration/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference exact spec scenarios
3. **Acceptance criteria** — which Epic 0 ConfigLoader tests must pass
4. **Crate/file location** — new `crates/tze_hud_config/` or module in runtime
5. **Immutability contract** — specify which config values are frozen at startup vs hot-reloadable

### Dependency chain

```
Epics 0+4+6 ──→ #1 TOML Loading ──→ #2 Profile Resolution
                                 ──→ #3 Capability Validation
                                 ──→ #4 Privacy/Zones/Agents
```
