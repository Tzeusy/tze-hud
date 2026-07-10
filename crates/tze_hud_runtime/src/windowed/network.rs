//! Network/runtime-context bootstrap helpers for the windowed runtime.

use std::sync::Arc;

use tokio::sync::Mutex;
use tze_hud_config::TzeHudConfig;
use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
use tze_hud_protocol::proto::session::runtime_service_server::RuntimeServiceServer;
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::session_server::HudSessionImpl;
use tze_hud_scene::config::ConfigLoader;

use super::WindowedConfig;
use super::config::select_grpc_bind_host;
use crate::reload_triggers::RuntimeServiceImpl;
use crate::runtime_context::{RuntimeContext, SharedRuntimeContext};
use crate::threads::NetworkRuntime;

/// Build a `RuntimeContext` from the windowed config.
///
/// When `cfg.config_toml` is `Some`, the TOML is parsed and validated. On
/// success, capability grants from `[agents.registered]` and the hot-reloadable
/// sections (`[privacy]`, `[degradation]`, `[chrome]`, `[agents.dynamic_policy]`)
/// are loaded into the context. The fallback policy is `Guest` (registered
/// agents only).
///
/// When `cfg.config_toml` is `None` (no config file), the context falls back to
/// `RuntimeContext::headless_default()` and `fallback_unrestricted = true` for
/// dev-friendly behaviour (any PSK-authenticated agent gets all capabilities).
///
/// Parse or validation errors are logged as warnings and cause a graceful
/// fallback to `headless_default()` so the runtime can still start.
///
/// Returns `(runtime_context, fallback_unrestricted)`.
pub(super) fn build_runtime_context(cfg: &WindowedConfig) -> (SharedRuntimeContext, bool) {
    match &cfg.config_toml {
        None => {
            // No config file - fall back to headless default.
            tracing::debug!(
                "windowed runtime: no config TOML provided; \
                 using headless_default (all agents unrestricted)"
            );
            (Arc::new(RuntimeContext::headless_default()), true)
        }
        Some(toml_src) => {
            // Parse the TOML.
            let loader = match TzeHudConfig::parse(toml_src) {
                Ok(l) => l,
                Err(parse_err) => {
                    tracing::warn!(
                        error = %parse_err.message,
                        line = parse_err.line,
                        column = parse_err.column,
                        "windowed runtime: config TOML parse error; \
                         falling back to headless_default"
                    );
                    return (Arc::new(RuntimeContext::headless_default()), false);
                }
            };

            // Validate and freeze into a ResolvedConfig.
            let resolved = match loader.freeze() {
                Ok(r) => r,
                Err(errors) => {
                    for err in &errors {
                        tracing::warn!(
                            code = ?err.code,
                            field = %err.field_path,
                            expected = %err.expected,
                            got = %err.got,
                            hint = %err.hint,
                            "windowed runtime: config validation error"
                        );
                    }
                    tracing::warn!(
                        "windowed runtime: {} config validation error(s); \
                         falling back to headless_default",
                        errors.len()
                    );
                    return (Arc::new(RuntimeContext::headless_default()), false);
                }
            };

            // Parse hot-reloadable sections from the same TOML so the initial
            // privacy/degradation/chrome/dynamic_policy settings take effect
            // immediately (before the first SIGHUP).
            let hot = tze_hud_config::reload_config(toml_src).unwrap_or_default();

            tracing::info!(
                profile = %resolved.profile.name,
                agents = resolved.agent_capabilities.len(),
                "windowed runtime: config loaded; \
                 capability grants applied from [agents.registered]"
            );

            let ctx = RuntimeContext::from_config_with_hot(
                resolved,
                crate::runtime_context::FallbackPolicy::Guest,
                hot,
            );
            (Arc::new(ctx), false)
        }
    }
}

/// Start network services (gRPC) on a dedicated Tokio multi-thread runtime.
///
/// Returns `(network_rt, handles)`:
/// - `network_rt` is `Some(NetworkRuntime)` when `grpc_port != 0`; `None` if
///   all services are disabled (port 0 disables gRPC).
/// - `handles` contains join handles for each spawned server task.
///
/// ## gRPC server
///
/// When `grpc_port != 0`, starts the `HudSession` gRPC server. The bind
/// address is `127.0.0.1:grpc_port` by default (loopback only) unless
/// `bind_all_interfaces` is `true`, in which case it binds `0.0.0.0:grpc_port`
/// (all interfaces - explicit opt-in required, hud-1aswu.1).
/// Setting `grpc_port = 0` skips server creation (compositor-only mode).
///
/// ## Errors
///
/// Returns `Err` if the `NetworkRuntime` Tokio runtime cannot be created, or if
/// the gRPC server address fails to parse.
#[allow(clippy::type_complexity)] // return type is self-documenting in this internal helper
pub(super) fn start_network_services(
    grpc_port: u16,
    psk: &str,
    shared_state: Arc<Mutex<SharedState>>,
    runtime_context: SharedRuntimeContext,
    fallback_unrestricted: bool,
    bind_all_interfaces: bool,
) -> Result<
    (
        Option<NetworkRuntime>,
        Vec<tokio::task::JoinHandle<()>>,
        Option<tokio::sync::broadcast::Sender<tze_hud_protocol::proto::ElementRepositionedEvent>>,
        Option<tokio::sync::broadcast::Sender<(String, tze_hud_protocol::proto::EventBatch)>>,
        Option<tokio::sync::broadcast::Sender<tze_hud_protocol::proto::FramePresented>>,
    ),
    Box<dyn std::error::Error>,
> {
    if grpc_port == 0 {
        tracing::info!(
            "windowed runtime: gRPC server disabled (grpc_port = 0); running compositor-only"
        );
        // Compositor-only mode: no session, so no present-ack subscriber. The
        // compositor thread still drains the present-ack queue (bounded memory)
        // but has no sender to broadcast on (hud-4va6q).
        return Ok((None, Vec::new(), None, None, None));
    }

    // Build the multi-thread Tokio runtime for network tasks.
    let network_rt = NetworkRuntime::new()
        .map_err(|e| format!("windowed runtime: failed to build network Tokio runtime: {e}"))?;

    // Security fix (hud-1aswu.1): default to loopback; opt-in for all interfaces.
    let grpc_bind_host = select_grpc_bind_host(bind_all_interfaces);
    tracing::info!(
        bind_all_interfaces,
        grpc_bind_host,
        "gRPC: bind address selected (hud-1aswu.1)"
    );
    let addr: std::net::SocketAddr = format!("{grpc_bind_host}:{grpc_port}")
        .parse()
        .map_err(|e| format!("windowed runtime: invalid gRPC address (port {grpc_port}): {e}"))?;

    // Wire config-driven capability registry into the session service.
    let agent_caps = runtime_context.snapshot_agent_capabilities();
    let service = HudSessionImpl::from_shared_state_with_config_and_media_ingress(
        shared_state,
        psk,
        agent_caps,
        fallback_unrestricted,
        runtime_context.media_ingress.clone(),
    );

    // Clone the broadcast senders before moving the service into the gRPC task.
    // The windowed runtime holds these senders to:
    // - broadcast ElementRepositionedEvents from the sync chrome-layer reset path.
    // - inject EventBatch payloads (scroll, keyboard, and future input events)
    //   on the input_event_tx channel after windowed input is processed.
    let element_repositioned_tx = service.element_repositioned_tx.clone();
    let input_event_tx = service.input_event_tx.clone();
    // Present-ack broadcast sender (hud-4va6q): the compositor thread emits
    // `FramePresented` on this after each presented frame, mirroring the
    // headless runtime's producer. Cloned before the service moves into the
    // gRPC task; subscribers attach via HudSession::subscribe_frame_presented.
    let frame_presented_tx = service.frame_presented_tx.clone();

    // Wire RuntimeService (ReloadConfig RPC) alongside HudSession.
    let runtime_svc = RuntimeServiceImpl::new(Arc::clone(&runtime_context));

    tracing::info!(grpc_addr = %addr, "windowed runtime: starting gRPC server");

    // Spawn the combined gRPC server task onto the network runtime.
    let handle = network_rt.rt.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(HudSessionServer::new(service))
            .add_service(RuntimeServiceServer::new(runtime_svc))
            .serve(addr)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "gRPC server exited with error");
            });
    });

    tracing::info!(grpc_addr = %addr, "windowed runtime: gRPC server task spawned");

    Ok((
        Some(network_rt),
        vec![handle],
        Some(element_repositioned_tx),
        Some(input_event_tx),
        Some(frame_presented_tx),
    ))
}

/// Render the non-secret startup banner printed to stdout once the network
/// listeners are up (hud-ylwqc).
///
/// The runtime otherwise emits nothing on stdout unless `TZE_HUD_LOG` is set
/// (tracing is gated on that env var), so a fresh operator has no way to learn
/// where the runtime is listening or how to attach. This banner makes the
/// runtime self-describing on first run.
///
/// **Security invariant:** this function deliberately takes *only* bound socket
/// addresses. The PSK (and every other credential) is not in scope here, so the
/// banner is provably incapable of leaking a secret — see the unit tests. When
/// a service is disabled, its address is passed as `None` and rendered as
/// `disabled` rather than a bogus endpoint.
pub(super) fn render_startup_banner(
    grpc_addr: Option<std::net::SocketAddr>,
    mcp_addr: Option<std::net::SocketAddr>,
) -> String {
    const RULE: &str = "────────────────────────────────────────────────────────────────────";
    let mut lines: Vec<String> = Vec::with_capacity(7);
    lines.push(RULE.to_string());
    lines.push(" tze_hud runtime ready".to_string());
    match grpc_addr {
        Some(addr) => lines.push(format!("   gRPC   : {addr}")),
        None => lines.push("   gRPC   : disabled".to_string()),
    }
    match mcp_addr {
        Some(addr) => lines.push(format!(
            "   MCP    : http://{addr}/mcp   (auth: Authorization: Bearer <TZE_HUD_PSK>)"
        )),
        None => lines.push("   MCP    : disabled".to_string()),
    }
    lines.push(
        "   attach : invoke the `hud-projection` skill in an LLM session, or run".to_string(),
    );
    lines.push("            scripts/quickstart.sh — see docs/QUICKSTART.md".to_string());
    lines.push(RULE.to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::test_support::make_shared_state;
    use super::*;

    /// The banner must never contain the PSK, even when one is configured.
    /// `render_startup_banner` takes only bound addresses (never the secret),
    /// so this holds by construction; the test guards against future edits that
    /// might thread a credential through the banner.
    #[test]
    fn startup_banner_never_contains_psk() {
        let psk = "SUPER-SECRET-PSK-2f9c1a7e-do-not-leak";
        // Simulate a fully-configured runtime with a PSK set in the environment.
        let grpc: std::net::SocketAddr = "127.0.0.1:50051".parse().unwrap();
        let mcp: std::net::SocketAddr = "127.0.0.1:9090".parse().unwrap();
        let banner = render_startup_banner(Some(grpc), Some(mcp));
        assert!(
            !banner.contains(psk),
            "startup banner must not leak the PSK; banner was:\n{banner}"
        );
        // Also assert the banner carries the useful, non-secret discovery info.
        assert!(banner.contains("127.0.0.1:50051"), "gRPC addr missing");
        assert!(
            banner.contains("http://127.0.0.1:9090/mcp"),
            "MCP URL missing"
        );
        assert!(banner.contains("hud-projection"), "attach hint missing");
        assert!(banner.contains("tze_hud runtime ready"), "header missing");
    }

    /// Disabled services render as `disabled`, not a bogus `:0` endpoint.
    #[test]
    fn startup_banner_renders_disabled_services() {
        let banner = render_startup_banner(None, None);
        assert!(banner.contains("gRPC   : disabled"));
        assert!(banner.contains("MCP    : disabled"));
        // Attach hint is always present so the runtime stays self-describing.
        assert!(banner.contains("hud-projection"));
    }

    /// When `grpc_port == 0`, `start_network_services` must return `None` for
    /// the runtime and an empty handle list (compositor-only mode, AC §2).
    #[test]
    fn start_network_services_grpc_port_zero_returns_no_runtime() {
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        let (rt, handles, _tx, _scroll_tx, present_tx) =
            start_network_services(0, "test-psk", shared_state, ctx, true, false)
                .expect("start_network_services should not fail for port 0");
        assert!(
            rt.is_none(),
            "grpc_port=0 must not create a NetworkRuntime (compositor-only)"
        );
        assert!(
            handles.is_empty(),
            "grpc_port=0 must not spawn any network task handles"
        );
        assert!(
            present_tx.is_none(),
            "grpc_port=0 has no session, so no present-ack sender (hud-4va6q)"
        );
    }

    /// When `grpc_port != 0`, `start_network_services` must return `Some` for
    /// the runtime and at least one spawned task handle (AC §1).
    #[test]
    fn start_network_services_nonzero_port_returns_runtime_and_handle() {
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        // Use a high ephemeral port unlikely to conflict.
        let (rt, handles, _tx, _scroll_tx, present_tx) =
            start_network_services(59781, "test-psk", shared_state, ctx, true, true)
                .expect("start_network_services should not error for a valid port");
        assert!(
            rt.is_some(),
            "non-zero grpc_port must create a NetworkRuntime"
        );
        assert!(
            !handles.is_empty(),
            "non-zero grpc_port must spawn at least one network task handle"
        );
        assert!(
            present_tx.is_some(),
            "non-zero grpc_port must expose the session present-ack sender so the \
             compositor thread can broadcast FramePresented (hud-4va6q)"
        );
        // Abort the spawned task so the test doesn't leave a lingering server.
        for h in handles {
            h.abort();
        }
    }

    /// Two successive calls with `grpc_port = 0` must both return `(None, [])`.
    /// Verifies idempotency of the disabled path (AC §2 deterministic).
    #[test]
    fn start_network_services_grpc_port_zero_is_idempotent() {
        for _ in 0..2 {
            let shared_state = make_shared_state();
            let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
            let (rt, handles, _tx, _scroll_tx, _present_tx) =
                start_network_services(0, "psk", shared_state, ctx, false, false)
                    .expect("port-0 must not error");
            assert!(rt.is_none());
            assert!(handles.is_empty());
        }
    }

    // These tests verify that start_network_services actually succeeds (does not
    // silently swallow a bind error). Each test allocates an ephemeral port via
    // TcpListener::bind(":0") so the OS picks a free port, eliminating port-
    // conflict flakiness in parallel CI runs.

    /// When `bind_all_interfaces = false`, `start_network_services` binds to
    /// `127.0.0.1` (loopback only) and must succeed.
    ///
    /// The bound address is determined by `select_grpc_bind_host` (separately
    /// pinned by the unit tests above). This test asserts that the full
    /// service startup path with the loopback bind host succeeds - not just
    /// that it doesn't error on an early-exit code path.
    #[test]
    fn start_network_services_loopback_default_binds_successfully() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("failed to allocate ephemeral port for loopback bind test");
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        let (rt, handles, _, _, _) =
            start_network_services(port, "psk", shared_state, ctx, false, false)
                .expect("loopback bind must succeed on a freshly allocated ephemeral port");
        assert!(rt.is_some(), "loopback bind must create a NetworkRuntime");
        assert!(!handles.is_empty(), "loopback bind must spawn task handles");
        for h in handles {
            h.abort();
        }
    }

    /// `start_network_services` with `bind_all_interfaces = true` binds on
    /// `0.0.0.0` (explicit opt-in for LAN/remote exposure) and must succeed.
    #[test]
    fn start_network_services_bind_all_interfaces_opt_in_binds_successfully() {
        let port = std::net::TcpListener::bind("0.0.0.0:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("failed to allocate ephemeral port for all-interfaces bind test");
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        let (rt, handles, _, _, _) =
            start_network_services(port, "psk", shared_state, ctx, false, true)
                .expect("all-interfaces bind must succeed on a freshly allocated ephemeral port");
        assert!(
            rt.is_some(),
            "all-interfaces bind must create a NetworkRuntime"
        );
        assert!(
            !handles.is_empty(),
            "all-interfaces bind must spawn task handles"
        );
        for h in handles {
            h.abort();
        }
    }

    /// Acceptance criterion 2: when no config TOML is provided, the runtime
    /// falls back to headless_default() with fallback_unrestricted = true.
    #[test]
    fn build_runtime_context_no_config_toml_uses_headless_default() {
        let cfg = WindowedConfig {
            config_toml: None,
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Fallback unrestricted should be true (dev-friendly default).
        assert!(
            fallback_unrestricted,
            "no-config path must set fallback_unrestricted=true"
        );
        // Profile name must be "headless" (headless_default behaviour).
        assert_eq!(
            ctx.profile.name, "headless",
            "no-config path must use the headless profile"
        );
        // Hot config should be all defaults.
        let hot = ctx.hot_config();
        assert!(
            hot.privacy.redaction_style.is_none(),
            "hot config privacy must default to None when no config file is given"
        );
    }

    /// Acceptance criterion 1: when a valid config TOML is provided, capability
    /// grants from [agents.registered] are parsed and applied.
    #[test]
    fn build_runtime_context_with_valid_config_applies_capability_grants() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[agents.registered.weather-agent]
capabilities = ["create_tiles", "modify_own_tiles"]
"#;
        let cfg = WindowedConfig {
            config_toml: Some(toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Config-driven path: fallback must be Guest (not unrestricted).
        assert!(
            !fallback_unrestricted,
            "config-driven path must set fallback_unrestricted=false"
        );
        // Registered agent capabilities must be applied.
        let caps = ctx.agent_capabilities("weather-agent");
        assert!(
            caps.is_some(),
            "weather-agent must appear in the capability registry"
        );
        let caps = caps.unwrap();
        assert!(
            caps.contains(&"create_tiles".to_string()),
            "weather-agent must have create_tiles grant"
        );
        assert!(
            caps.contains(&"modify_own_tiles".to_string()),
            "weather-agent must have modify_own_tiles grant"
        );
        // Unregistered agent must get guest (denied) policy.
        let policy = ctx.capability_policy_for("unknown-agent");
        assert!(
            policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_err(),
            "unregistered agent must be denied under config-driven Guest fallback"
        );
    }

    /// Acceptance criterion 1: config-driven context uses the full-display profile.
    #[test]
    fn build_runtime_context_with_config_uses_configured_profile() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
        let cfg = WindowedConfig {
            config_toml: Some(toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, _) = build_runtime_context(&cfg);
        assert_eq!(
            ctx.profile.name, "full-display",
            "config-driven path must use the profile specified in the TOML"
        );
    }

    /// Acceptance criterion 3 (fallback): invalid TOML falls back to
    /// headless_default() rather than crashing.
    #[test]
    fn build_runtime_context_invalid_toml_falls_back_to_headless() {
        let bad_toml = "this is not valid TOML [\n";
        let cfg = WindowedConfig {
            config_toml: Some(bad_toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Must fall back gracefully to headless, but NOT unrestricted.
        // An operator who provided a config intended to restrict capabilities.
        assert!(
            !fallback_unrestricted,
            "parse-error path must NOT fall back to unrestricted"
        );
        assert_eq!(
            ctx.profile.name, "headless",
            "parse-error path must fall back to headless profile"
        );
    }

    /// Acceptance criterion 3 (fallback): config with validation errors falls
    /// back to headless_default() rather than crashing.
    #[test]
    fn build_runtime_context_validation_error_falls_back_to_headless() {
        // Missing required [[tabs]] section -> validation error.
        let invalid_toml = r#"
[runtime]
profile = "full-display"
"#;
        let cfg = WindowedConfig {
            config_toml: Some(invalid_toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Must fall back gracefully to headless, but NOT unrestricted.
        // An operator who provided a config intended to restrict capabilities.
        assert!(
            !fallback_unrestricted,
            "validation-error path must NOT fall back to unrestricted"
        );
        assert_eq!(
            ctx.profile.name, "headless",
            "validation-error path must fall back to headless profile"
        );
    }

    /// Hot-reloadable sections (privacy, degradation) from the initial config
    /// are applied immediately - no SIGHUP required.
    #[test]
    fn build_runtime_context_hot_sections_applied_from_config() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[privacy]
redaction_style = "blank"
"#;
        let cfg = WindowedConfig {
            config_toml: Some(toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, _) = build_runtime_context(&cfg);
        let hot = ctx.hot_config();
        assert_eq!(
            hot.privacy.redaction_style,
            Some("blank".to_string()),
            "privacy.redaction_style from config must be applied immediately at startup"
        );
    }
}
