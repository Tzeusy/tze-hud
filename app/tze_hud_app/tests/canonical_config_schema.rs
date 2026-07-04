use tze_hud_config::loader::TzeHudConfig;
use tze_hud_scene::config::ConfigLoader;

/// Canonical app production config remains valid against current loader schema.
#[test]
fn canonical_app_production_toml_matches_loader_schema() {
    let toml = include_str!("../config/production.toml");
    let loader = TzeHudConfig::parse(toml).expect("production.toml should parse");
    let errors = loader.validate();
    assert!(
        errors.is_empty(),
        "canonical app production.toml should validate cleanly, got: {errors:?}"
    );

    let resolved = loader
        .freeze()
        .expect("canonical app production.toml should freeze");
    assert_eq!(resolved.profile.name, "full-display");
    assert!(
        !resolved.tab_names.is_empty(),
        "canonical app production config must declare at least one tab"
    );
}

/// The resident gRPC portal bridge (default-off) must be pre-registered as a
/// principal with exactly the least-privilege capabilities it needs, so that
/// enabling it under production config does not fail the fail-closed handshake
/// with CapabilityNotGranted. See hud-osy3m; the bridge presents identity
/// `resident-grpc-portal` and requires PORTAL_CAPABILITIES
/// (`create_tiles` + `modify_own_tiles`).
#[test]
fn canonical_app_production_registers_resident_grpc_portal_principal() {
    let toml = include_str!("../config/production.toml");
    let resolved = TzeHudConfig::parse(toml)
        .expect("production.toml should parse")
        .freeze()
        .expect("production.toml should freeze");

    let caps = resolved
        .agent_capabilities
        .get("resident-grpc-portal")
        .expect("resident-grpc-portal must be a registered principal under production config");

    // Exactly the bridge's PORTAL_CAPABILITIES — least privilege, no more.
    let mut sorted = caps.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
        "resident-grpc-portal must be granted exactly its least-privilege capabilities, got: {caps:?}"
    );
}
