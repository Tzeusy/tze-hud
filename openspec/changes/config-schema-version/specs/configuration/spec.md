## ADDED Requirements

### Requirement: Config Schema Version and Compatibility Policy

The configuration document SHALL support an optional top-level `schema_version` field
(a non-negative integer). The loader MUST evaluate `schema_version` against the
runtime's supported schema-version range **before** field-level validation, and MUST
apply a documented compatibility policy:

- An **absent** `schema_version` MUST be treated as the current supported version, so
  that existing v1 configurations continue to load unchanged (back-compatible default).
- A `schema_version` **within** the supported range MUST load, after applying the
  documented compatibility/migration defaults for that version, and then proceed to
  normal field-level validation.
- A `schema_version` **greater than** the runtime's maximum supported version MUST fail
  closed with the structured error `CONFIG_SCHEMA_VERSION_UNSUPPORTED`. The error MUST
  name the supported version range, and no port MUST be bound and no frame MUST be
  rendered.

`CONFIG_SCHEMA_VERSION_UNSUPPORTED` MUST be a member of the `ConfigErrorCode` family and
MUST carry the same structured fields as every other config error (`code`,
`field_path`, `expected`, `got`, `hint`). The `schema_version` field MUST appear in the
exported JSON schema (`--print-schema`), and the compatibility/migration policy MUST be
documented alongside the schema export.

Source: RFC 0006 (Configuration), `openspec/specs/configuration/spec.md` §Structured Validation Error Collection (fail-closed boot), `crates/tze_hud_config/src/raw.rs`, `crates/tze_hud_config/src/loader.rs`
Scope: v1-mandatory

#### Scenario: Absent schema_version defaults to current

- **WHEN** a configuration omits the `schema_version` field (an existing v1 config)
- **THEN** the loader SHALL treat it as the current supported schema version
- **AND** the configuration SHALL proceed to normal field-level validation with no schema-version error

#### Scenario: Newer schema_version fails closed

- **WHEN** a configuration sets `schema_version` to a value greater than the runtime's maximum supported version
- **THEN** startup SHALL fail with the structured error `CONFIG_SCHEMA_VERSION_UNSUPPORTED`
- **AND** the error SHALL name the supported version range
- **AND** no port SHALL be bound and no frame SHALL be rendered

#### Scenario: Supported schema_version is validated and applied

- **WHEN** a configuration sets `schema_version` to a value within the supported range
- **THEN** the loader SHALL apply the documented compatibility/migration defaults for that version
- **AND** the configuration SHALL proceed to normal field-level validation

#### Scenario: schema_version is exported in the JSON schema

- **WHEN** the operator runs the runtime with `--print-schema`
- **THEN** the emitted JSON schema SHALL include the optional top-level `schema_version` field
