//! Canonical app production-config boot gate.
//!
//! This test suite boots the runtime with the committed canonical app config:
//! `app/tze_hud_app/config/production.toml`.
//!
//! The gate is intentionally CI-visible:
//! - startup must succeed
//! - config-declared widget instances/types must be registered
//! - component-profile rendering overrides must be visible in zone policy
//!
//! If startup silently falls back to a default/headless policy, these assertions
//! fail even when runtime construction itself succeeds.

use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;

const PRODUCTION_CONFIG: &str = include_str!("../config/production.toml");
const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn canonical_config_for_headless() -> String {
    PRODUCTION_CONFIG
        // HeadlessConfig only accepts config_toml (string) and does not carry
        // config_file_path, so relative asset paths cannot be resolved against
        // app/tze_hud_app/config/. Rebase only asset roots for deterministic CI.
        .replace(
            "paths = [\"widget_bundles\"]",
            &format!("paths = [\"{REPO_ROOT}/widget_bundles\"]"),
        )
        .replace(
            "paths = [\"profiles\"]",
            &format!("paths = [\"{REPO_ROOT}/profiles\"]"),
        )
}

#[tokio::test]
async fn canonical_app_production_config_boot_succeeds() {
    let config = HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: 0,
        psk: "canonical-app-production-boot-test".to_string(),
        config_toml: Some(canonical_config_for_headless()),
    };

    let result = HeadlessRuntime::new(config).await;
    assert!(
        result.is_ok(),
        "runtime failed to start with app/tze_hud_app/config/production.toml: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn canonical_app_production_config_registers_declared_state() {
    let config = HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: 0,
        psk: "canonical-app-production-boot-test".to_string(),
        config_toml: Some(canonical_config_for_headless()),
    };

    let runtime = HeadlessRuntime::new(config)
        .await
        .expect("runtime must start with canonical app production config");

    let state = runtime.shared_state().lock().await;
    let scene = state.scene.lock().await;

    // Config declares three concrete widget instances on the Main tab. If startup
    // fell back to defaults, these instances are absent.
    for instance in ["main-gauge", "main-progress", "main-status"] {
        assert!(
            scene.widget_registry.get_instance(instance).is_some(),
            "expected widget instance `{instance}` from canonical app config; startup likely fell back"
        );
    }

    // The corresponding widget types must also be loaded.
    for widget_type in ["gauge", "progress-bar", "status-indicator"] {
        assert!(
            scene.widget_registry.get_definition(widget_type).is_some(),
            "expected widget type `{widget_type}` from widget bundles; startup likely fell back"
        );
    }

    // The active notification profile sets color.text.primary = #F5F7FA.
    // Verify the resolved zone policy reflects that override, not default fallback.
    let notification_zone = scene
        .zone_registry
        .zones
        .get("notification-area")
        .expect("notification-area zone must be present");
    let text_color = notification_zone
        .rendering_policy
        .text_color
        .expect("notification-area text_color must be populated");

    let expected = (245.0f32 / 255.0f32, 247.0f32 / 255.0f32, 250.0f32 / 255.0f32);
    let eps = 1e-3f32;
    assert!(
        (text_color.r - expected.0).abs() < eps
            && (text_color.g - expected.1).abs() < eps
            && (text_color.b - expected.2).abs() < eps,
        "expected notification-area text_color to resolve to #F5F7FA from active profile, got ({:.4}, {:.4}, {:.4})",
        text_color.r,
        text_color.g,
        text_color.b
    );
}
