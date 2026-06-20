## Why

The configuration capability is the runtime's governance entry point: every structural
and policy decision is validated at startup before any port is bound or frame is
rendered (`openspec/specs/configuration/spec.md` §Purpose). Boot is fail-closed — the
loader collects all validation errors and refuses to start on any error
(§Structured Validation Error Collection).

What is missing is a **version contract for the config document itself**. `RawConfig`
(`crates/tze_hud_config/src/raw.rs`) carries no `schema_version` field, and the loader
(`crates/tze_hud_config/src/loader.rs`) never gates on one. Today a config written for a
future, incompatible schema would be parsed field-by-field with whatever partial
overlap exists, producing confusing per-field errors (or worse, silently accepting a
field whose meaning has changed) instead of one clear "this config targets a schema
this runtime does not support" failure.

For a screen-owning runtime that an operator restarts unattended, a schema/version
mismatch must fail closed with a single legible error, exactly like the existing
`CONFIG_*` family — not degrade into ambiguous validation noise. This change adds an
optional top-level `schema_version` field and a documented compatibility policy:
absent → treated as current (back-compatible for every existing v1 config), newer than
supported → fail closed with `CONFIG_SCHEMA_VERSION_UNSUPPORTED` naming the supported
range, supported → load after applying documented compatibility defaults. The field is
exported through the existing `--print-schema` path so operators can discover it.

## What Changes

- ADD an optional top-level `schema_version` (integer) field to the configuration
  document, exported via the existing JSON-schema (`--print-schema`) path.
- ADD a loader gate, evaluated before field-level validation, that compares
  `schema_version` against the runtime's supported range and applies a documented
  compatibility policy (absent → current; newer → fail closed; supported → proceed).
- ADD `CONFIG_SCHEMA_VERSION_UNSUPPORTED` to the `ConfigErrorCode` family; the error
  names the supported version range and binds no ports (fail-closed).
- DOCUMENT the schema-version + compatibility/migration policy alongside the schema
  export (README §1.1 and the configuration spec).

No existing requirement is removed. Existing configs (no `schema_version`) keep booting
unchanged.

## Impact

- Affected spec: `configuration` (ADDED requirement).
- Affected code: `crates/tze_hud_config/src/raw.rs`, `crates/tze_hud_config/src/loader.rs`.
- Affected docs: `README.md` §1.1.
- Boot remains fail-closed; the `canonical-app-production-boot` gate must stay green.
