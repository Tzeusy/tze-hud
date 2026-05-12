use tze_hud_config::loader::TzeHudConfig;
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;
use tze_hud_scene::config::ConfigLoader;

const BENCHMARK_CONFIG: &str = include_str!("../config/benchmark.toml");
const WINDOWS_MEDIA_CONFIG: &str = include_str!("../config/windows-media-ingress.toml");
const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn freeze_benchmark_config() -> tze_hud_scene::config::ResolvedConfig {
    let loader = TzeHudConfig::parse(BENCHMARK_CONFIG).expect("benchmark.toml should parse");
    let errors = loader.validate();
    assert!(
        errors.is_empty(),
        "benchmark.toml should validate cleanly, got: {errors:?}"
    );
    loader.freeze().expect("benchmark.toml should freeze")
}

fn media_config_for_headless() -> String {
    let mut config: toml::Value = WINDOWS_MEDIA_CONFIG
        .parse()
        .expect("windows-media-ingress.toml must be valid TOML");

    config["widget_bundles"]["paths"] = toml::Value::Array(vec![
        toml::Value::String(format!("{REPO_ROOT}/widget_bundles")),
        toml::Value::String(format!("{REPO_ROOT}/assets/widget_bundles")),
        toml::Value::String(format!("{REPO_ROOT}/assets/widgets")),
    ]);
    config["component_profile_bundles"]["paths"] =
        toml::Value::Array(vec![toml::Value::String(format!("{REPO_ROOT}/profiles"))]);

    toml::to_string(&config).expect("headless media config must serialize")
}

fn assert_caps(resolved: &tze_hud_scene::config::ResolvedConfig, agent: &str, expected: &[&str]) {
    let caps = resolved
        .agent_capabilities
        .get(agent)
        .unwrap_or_else(|| panic!("expected registered benchmark agent {agent}"));
    for cap in expected {
        assert!(
            caps.iter().any(|actual| actual == cap),
            "expected {agent} to have {cap}, got {caps:?}"
        );
    }
}

fn benchmark_config_for_headless() -> String {
    let mut config: toml::Value = BENCHMARK_CONFIG
        .parse()
        .expect("benchmark.toml must be valid TOML");

    config["widget_bundles"]["paths"] = toml::Value::Array(vec![
        toml::Value::String(format!("{REPO_ROOT}/widget_bundles")),
        toml::Value::String(format!("{REPO_ROOT}/assets/widget_bundles")),
        toml::Value::String(format!("{REPO_ROOT}/assets/widgets")),
    ]);
    config["component_profile_bundles"]["paths"] =
        toml::Value::Array(vec![toml::Value::String(format!("{REPO_ROOT}/profiles"))]);

    toml::to_string(&config).expect("headless benchmark config must serialize")
}

fn benchmark_headless_config() -> HeadlessConfig {
    HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: 0,
        psk: "benchmark-config-test".to_string(),
        config_toml: Some(benchmark_config_for_headless()),
    }
}

#[test]
fn benchmark_config_matches_loader_schema() {
    let resolved = freeze_benchmark_config();
    assert_eq!(resolved.profile.name, "full-display");
    assert!(
        resolved.tab_names.iter().any(|name| name == "Main"),
        "benchmark config must declare the Main tab"
    );
}

#[test]
fn benchmark_config_registers_publish_load_harness_agent() {
    let resolved = freeze_benchmark_config();
    assert_caps(
        &resolved,
        "widget-publish-load-harness",
        &["publish_widget:main-progress", "read_telemetry"],
    );
}

#[test]
fn benchmark_config_registers_three_soak_agents_with_widget_and_zone_caps() {
    let resolved = freeze_benchmark_config();
    for agent in ["agent-alpha", "agent-beta", "agent-gamma"] {
        assert_caps(
            &resolved,
            agent,
            &[
                "create_tiles",
                "modify_own_tiles",
                "access_input_events",
                "publish_widget:main-progress",
                "publish_widget:main-gauge",
                "publish_widget:main-status",
                "publish_zone:subtitle",
                "publish_zone:notification-area",
                "publish_zone:status-bar",
                "read_telemetry",
            ],
        );
    }
}

#[tokio::test]
async fn benchmark_config_boot_registers_widgets_for_live_publish() {
    let runtime = HeadlessRuntime::new(benchmark_headless_config())
        .await
        .expect("runtime must start with benchmark config");

    let scene_handle = {
        let state = runtime.shared_state().lock().await;
        state.scene.clone()
    };
    let scene = scene_handle.lock().await;

    for instance in ["main-gauge", "main-progress", "main-status"] {
        assert!(
            scene.widget_registry.get_instance(instance).is_some(),
            "expected widget instance `{instance}` from benchmark config"
        );
    }
}

#[test]
fn windows_media_config_names_approved_media_zone_and_producer() {
    let loader =
        TzeHudConfig::parse(WINDOWS_MEDIA_CONFIG).expect("windows-media-ingress.toml should parse");
    let errors = loader.validate();
    assert!(
        errors.is_empty(),
        "windows-media-ingress.toml should validate cleanly, got: {errors:?}"
    );
    let resolved = loader
        .freeze()
        .expect("windows-media-ingress.toml should freeze");
    assert!(resolved.media_ingress.enabled);
    assert_eq!(
        resolved.media_ingress.approved_zone.as_deref(),
        Some("media-pip")
    );
    assert_eq!(resolved.media_ingress.max_active_streams, 1);
    assert_caps(
        &resolved,
        "windows-local-media-producer",
        &["media_ingress", "publish_zone:media-pip"],
    );
    assert_caps(
        &resolved,
        "windows-youtube-frame-bridge",
        &["media_ingress", "publish_zone:media-pip"],
    );
}

#[tokio::test]
async fn windows_media_config_boot_registers_only_media_pip_for_video_surface_ref() {
    let runtime = HeadlessRuntime::new(HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: 0,
        psk: "media-config-test".to_string(),
        config_toml: Some(media_config_for_headless()),
    })
    .await
    .expect("runtime must start with media config");

    let scene_handle = {
        let state = runtime.shared_state().lock().await;
        state.scene.clone()
    };
    let scene = scene_handle.lock().await;
    let accepting = scene
        .zone_registry
        .zones_accepting(tze_hud_scene::types::ZoneMediaType::VideoSurfaceRef)
        .into_iter()
        .map(|zone| zone.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(accepting, vec!["media-pip"]);
}
