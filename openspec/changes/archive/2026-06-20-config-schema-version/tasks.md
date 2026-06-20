# Tasks — Config Schema Version and Compatibility Policy

This change adds a version contract to the configuration document. No runtime
implementation begins until the change is reviewed and accepted; acceptance authorizes
the loader and schema-export work. Boot remains fail-closed throughout.

## 1. Contract and review

- [x] 1.1 Validate this change: `openspec validate config-schema-version --strict`
- [x] 1.2 Confirm the policy preserves fail-closed boot doctrine (loader collects errors, refuses to start on any error)
- [x] 1.3 Confirm absent `schema_version` keeps every existing v1 config booting unchanged

## 2. Implementation

- [x] 2.1 Add `schema_version: Option<u32>` to `RawConfig` (`crates/tze_hud_config/src/raw.rs`) with `JsonSchema` derive so it appears in `--print-schema`
- [x] 2.2 Define the supported schema-version range constant(s) in the config crate
- [x] 2.3 Add `ConfigErrorCode::ConfigSchemaVersionUnsupported` (`CONFIG_SCHEMA_VERSION_UNSUPPORTED`) with the structured fields and a hint naming the supported range
- [x] 2.4 Gate `schema_version` early in `loader.rs::validate` (before field-level checks): absent → current; supported → apply compatibility defaults; newer → fail closed
- [x] 2.5 Document the schema-version + compatibility/migration policy in README §1.1 and alongside the schema export

## 3. Tests

- [x] 3.1 Unit test: absent `schema_version` loads as current (no error)
- [x] 3.2 Unit test: newer-than-supported `schema_version` fails with `CONFIG_SCHEMA_VERSION_UNSUPPORTED` naming the range, binds no port
- [x] 3.3 Unit test: in-range `schema_version` proceeds to field-level validation
- [x] 3.4 Confirm `--print-schema` output includes `schema_version`
- [x] 3.5 `canonical-app-production-boot` gate stays green
