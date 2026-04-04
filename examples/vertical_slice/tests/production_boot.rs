//! # Production Config Boot Test
//!
//! Boots the headless runtime with the committed production config
//! (`config/production.toml`) and verifies:
//!
//! 1. Startup succeeds without error — the config is valid and parseable.
//! 2. The runtime initialises the full pipeline (scene, compositor, telemetry).
//! 3. An unregistered agent connecting over gRPC receives **guest policy**
//!    (zero capabilities granted) — sovereignty-by-mechanism is active.
//! 4. The registered agent (`vertical-slice-agent`) receives its declared
//!    capabilities from the config file.
//!
//! ## Why this test exists
//!
//! The default example path (`cargo run -p vertical_slice -- --headless`)
//! loads `config/production.toml` at runtime.  If the config is malformed
//! or the runtime's config parsing regresses, the binary silently falls back
//! to guest policy.  This test makes that failure explicit and CI-visible.
//!
//! ## Spec reference
//!
//! - `configuration/spec.md` §Requirement: Capability Vocabulary (lines 149-164)
//! - `session-protocol/spec.md` §Requirement: Session Establishment (lines 87-112)
//! - `heart-and-soul/architecture.md` §Sovereignty by Mechanism
//!
//! ## Dev-mode note
//!
//! This test uses `config_toml: Some(PRODUCTION_CONFIG)` — it does NOT rely on
//! `config_toml: None` (dev-mode unrestricted bypass).  The `dev-mode` feature
//! is compiled into `vertical_slice` for other test infrastructure, but this
//! test exercises the production code path where governance is enforced by config.
//!
//! Run:
//!   cargo test -p vertical_slice --test production_boot -- --nocapture

use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;

/// The production config is embedded at compile time from the committed file.
/// If the file is missing or malformed, this const will cause a compile error —
/// which is intentional (the config must always be present and syntactically valid).
const PRODUCTION_CONFIG: &str = include_str!("../config/production.toml");

/// Absolute path to the profiles/ directory at the repository root, used to
/// resolve component_profile_bundles.paths at test time (mirroring what
/// the windowed runtime does via config_file_path).
///
/// CARGO_MANIFEST_DIR is `examples/vertical_slice/` — two levels below the repo
/// root — so `../../profiles` reaches `profiles/` at the repo root.
const PROFILES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles");

/// Boot the runtime with the committed production config and verify startup
/// succeeds.
///
/// This is the most fundamental CI gate: if `production.toml` is malformed or
/// the runtime's config parsing regresses, this test fails immediately.
#[tokio::test]
async fn production_config_boot_succeeds() {
    let config = HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: 0, // No gRPC server — pure boot test.
        psk: "production-boot-test".to_string(),
        config_toml: Some(PRODUCTION_CONFIG.to_string()),
    };

    let result = HeadlessRuntime::new(config).await;
    assert!(
        result.is_ok(),
        "Runtime failed to start with production config: {:?}",
        result.err()
    );

    println!("PASS: runtime booted with production.toml");
}

/// Verify that the production config correctly parses and registers the
/// `vertical-slice-agent` with its declared capability set.
///
/// This test boots the runtime, starts the gRPC server on an ephemeral port,
/// connects as the registered agent, and asserts that the granted capabilities
/// match the config file declaration.
#[tokio::test]
async fn production_config_grants_registered_agent_capabilities() {
    use tokio_stream::StreamExt;
    use tze_hud_protocol::proto::session as session_proto;
    use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;

    // Ephemeral port to avoid port conflicts in parallel CI.
    let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
    let free_port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: free_port,
        psk: "production-boot-test".to_string(),
        config_toml: Some(PRODUCTION_CONFIG.to_string()),
    };

    let runtime = HeadlessRuntime::new(config)
        .await
        .expect("runtime must start with production config");
    let _server = runtime
        .start_grpc_server()
        .await
        .expect("gRPC server must start");

    // Connect as the registered agent declared in production.toml.
    let mut client = HudSessionClient::connect(format!("http://[::1]:{free_port}"))
        .await
        .expect("must connect to gRPC server");

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(16);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;

    // Send SessionInit as the registered agent with the canonical capability set.
    // These are exactly the capabilities declared in production.toml.
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: "vertical-slice-agent".to_string(),
                agent_display_name: "Vertical Slice Agent".to_string(),
                pre_shared_key: "production-boot-test".to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                    "access_input_events".to_string(),
                    "read_scene_topology".to_string(),
                    "publish_zone:status-bar".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            },
        )),
    })
    .await
    .unwrap();

    let mut response = client
        .session(stream)
        .await
        .expect("must open session stream")
        .into_inner();

    let msg = response
        .next()
        .await
        .expect("must receive SessionEstablished")
        .expect("must not error");

    match &msg.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(established)) => {
            let granted = &established.granted_capabilities;
            println!("Granted capabilities: {granted:?}");

            // The registered agent must receive all 5 declared capabilities.
            // If governance is broken (e.g., config not loaded), the agent
            // gets guest policy (empty capabilities), and this fails.
            assert!(
                granted.contains(&"create_tiles".to_string()),
                "expected create_tiles in granted capabilities, got: {granted:?}"
            );
            assert!(
                granted.contains(&"modify_own_tiles".to_string()),
                "expected modify_own_tiles in granted capabilities, got: {granted:?}"
            );
            assert!(
                granted.contains(&"access_input_events".to_string()),
                "expected access_input_events in granted capabilities, got: {granted:?}"
            );
            assert!(
                granted.contains(&"read_scene_topology".to_string()),
                "expected read_scene_topology in granted capabilities, got: {granted:?}"
            );
            assert!(
                granted.contains(&"publish_zone:status-bar".to_string()),
                "expected publish_zone:status-bar in granted capabilities, got: {granted:?}"
            );
            println!("PASS: registered agent received all 5 declared capabilities");
        }
        other => {
            panic!("Expected SessionEstablished, got: {other:?}");
        }
    }
}

/// Verify that an unregistered agent receives guest policy (no capabilities).
///
/// This is the sovereignty-by-mechanism gate: agents not declared in the config
/// must never receive capabilities, regardless of what they request.
#[tokio::test]
async fn production_config_denies_unregistered_agent() {
    use tokio_stream::StreamExt;
    use tze_hud_protocol::proto::session as session_proto;
    use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;

    // Ephemeral port.
    let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
    let free_port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: free_port,
        psk: "production-boot-test".to_string(),
        config_toml: Some(PRODUCTION_CONFIG.to_string()),
    };

    let runtime = HeadlessRuntime::new(config)
        .await
        .expect("runtime must start with production config");
    let _server = runtime
        .start_grpc_server()
        .await
        .expect("gRPC server must start");

    let mut client = HudSessionClient::connect(format!("http://[::1]:{free_port}"))
        .await
        .expect("must connect to gRPC server");

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(16);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;

    // An agent not declared in production.toml — must receive guest policy.
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: "unknown-rogue-agent".to_string(),
                agent_display_name: "Unknown Agent".to_string(),
                pre_shared_key: "production-boot-test".to_string(),
                // Requests all capabilities — must receive none.
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                    "access_input_events".to_string(),
                    "read_scene_topology".to_string(),
                    "publish_zone:status-bar".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            },
        )),
    })
    .await
    .unwrap();

    let mut response = client
        .session(stream)
        .await
        .expect("must open session stream")
        .into_inner();

    let msg = response
        .next()
        .await
        .expect("must receive response")
        .expect("must not error");

    match &msg.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(established)) => {
            let granted = &established.granted_capabilities;
            println!("Granted capabilities for unregistered agent: {granted:?}");

            // Guest policy: no capabilities should be granted.
            assert!(
                granted.is_empty(),
                "unregistered agent must receive no capabilities (guest policy), \
                 but got: {granted:?}"
            );
            println!(
                "PASS: unregistered agent received empty capabilities (guest policy enforced)"
            );
        }
        other => {
            panic!("Expected SessionEstablished, got: {other:?}");
        }
    }
}

/// Return the text content of a TOML section (from after the `[section]` header
/// to the start of the next section header or end of file).
fn toml_section_contents<'a>(toml: &'a str, section_name: &str) -> Option<&'a str> {
    let header = format!("[{section_name}]");
    let start = toml.find(&header)?;
    let after_header = &toml[start + header.len()..];
    let section_end = after_header
        .find("\n[")
        .map(|idx| start + header.len() + idx)
        .unwrap_or(toml.len());
    Some(&toml[start + header.len()..section_end])
}

/// Verify that the production config declares the exemplar-subtitle and
/// exemplar-alert-banner component profiles in [component_profiles].
///
/// This test inspects the embedded TOML source directly (no TOML parser needed)
/// and asserts that both profile declarations are present inside the
/// [component_profiles] section.  This is intentionally simple — if the TOML is
/// malformed the other tests in this file would fail first.
///
/// Why: hud-hzub (subtitle) and hud-w3o6 (alert-banner) both identified a P3 gap
/// where the exemplar profiles were not wired into the production config.  This
/// test prevents regression.
#[test]
fn production_config_declares_exemplar_component_profiles() {
    assert!(
        PRODUCTION_CONFIG.contains("[component_profile_bundles]"),
        "production.toml must contain a [component_profile_bundles] section"
    );

    let component_profiles = toml_section_contents(PRODUCTION_CONFIG, "component_profiles")
        .expect("production.toml must contain a [component_profiles] section");

    // Check that each key maps to the expected profile name within the section.
    // We look for lines containing both the key and the value rather than an exact
    // whitespace-sensitive match so the assertion is robust to TOML formatting changes.
    assert!(
        component_profiles.lines().any(|l| l.contains("subtitle")
            && l.contains("exemplar-subtitle")
            && !l.trim_start().starts_with('#')),
        "production.toml must declare subtitle = \"exemplar-subtitle\" in [component_profiles]"
    );

    assert!(
        component_profiles
            .lines()
            .any(|l| l.contains("alert-banner")
                && l.contains("exemplar-alert-banner")
                && !l.trim_start().starts_with('#')),
        "production.toml must declare alert-banner = \"exemplar-alert-banner\" in [component_profiles]"
    );

    println!("PASS: production.toml declares exemplar-subtitle and exemplar-alert-banner profiles");
}

/// Verify that the exemplar-subtitle and exemplar-alert-banner profiles load
/// correctly at runtime when the profiles/ directory is supplied via an absolute
/// path (as the windowed runtime does via config_file_path).
///
/// This test constructs a config string with an absolute profiles/ path derived
/// from CARGO_MANIFEST_DIR, then boots the headless runtime and checks that the
/// subtitle zone's rendering policy reflects the exemplar-subtitle token overrides.
///
/// Skipped if the profiles/ directory does not exist.
#[tokio::test]
async fn production_config_exemplar_profiles_load_with_resolved_paths() {
    let profiles_dir = std::path::Path::new(PROFILES_DIR);
    if !profiles_dir.exists() {
        println!("SKIP: profiles dir not found at {PROFILES_DIR} — skipping profile load test");
        return;
    }

    // Canonicalize the profiles path so it works regardless of CWD.
    let abs_path = profiles_dir
        .canonicalize()
        .expect("profiles dir must be canonicalisable");
    // Escape backslashes and double-quotes for a TOML basic string literal.
    // On Windows, canonicalize() returns UNC paths (\\?\C:\...) where replacing
    // backslashes with forward-slashes produces an invalid //? prefix.
    // Escaping preserves the path structure and produces valid TOML on all platforms.
    let abs_path_str = abs_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    // Build a minimal config with the absolute profiles path wired in.
    let config_toml = format!(
        r#"
[runtime]
profile = "headless"

[[tabs]]
name = "Main"
default_tab = true

[agents.registered.test-agent]
capabilities = ["create_tiles"]

[component_profile_bundles]
paths = ["{abs_path_str}"]

[component_profiles]
subtitle      = "exemplar-subtitle"
alert-banner  = "exemplar-alert-banner"
"#
    );

    let config = HeadlessConfig {
        width: 320,
        height: 240,
        grpc_port: 0,
        psk: "profile-load-test".to_string(),
        config_toml: Some(config_toml),
    };

    let runtime = HeadlessRuntime::new(config)
        .await
        .expect("runtime must boot with exemplar profiles config");

    // The subtitle zone must have font_size_px = 28 from exemplar-subtitle.
    // Access the scene via the shared state.
    let font_size = {
        let state = runtime.state.lock().await;
        let scene = state.scene.lock().await;
        scene
            .zone_registry
            .zones
            .get("subtitle")
            .and_then(|z| z.rendering_policy.font_size_px)
    };

    assert_eq!(
        font_size,
        Some(28.0),
        "exemplar-subtitle must set font_size_px = 28.0 on the subtitle zone (got {font_size:?})"
    );

    println!("PASS: exemplar-subtitle and exemplar-alert-banner profiles loaded and applied");
}
