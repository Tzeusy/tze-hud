#![windows_subsystem = "windows"]

//! # tze_hud — canonical runtime application binary
//!
//! This is the **production entrypoint** for the tze_hud windowed display runtime.
//! It is *not* a demo or example binary. Use this binary in deployment tooling and
//! operational automation.
//!
//! ## Startup options
//!
//! All options are available as CLI flags and, where applicable, as environment
//! variable overrides. Flags take priority over environment variables.
//!
//! | Flag                | Env var                | Default      | Description                              |
//! |---------------------|------------------------|--------------|------------------------------------------|
//! | `--config <path>`   | `TZE_HUD_CONFIG`       | (auto-resolved) | Path to TOML config file (`[runtime]` + `[[tabs]]` schema). |
//! | `--window-mode <m>` | `TZE_HUD_WINDOW_MODE`  | `fullscreen` | Window mode: `fullscreen` or `overlay`.  |
//! | `--width <px>`      | `TZE_HUD_WINDOW_WIDTH` | auto¹        | Window width in pixels.                  |
//! | `--height <px>`     | `TZE_HUD_WINDOW_HEIGHT`| auto¹        | Window height in pixels.                 |
//! | `--grpc-port <port>`| `TZE_HUD_GRPC_PORT`    | `50051`      | gRPC listen port (0 to disable).         |
//! | `--mcp-port <port>` | `TZE_HUD_MCP_PORT`     | `9090`       | MCP HTTP listen port (0 to disable).     |
//! | `--psk <key>`       | `TZE_HUD_PSK`          | `tze-hud-key`| Pre-shared key for session authentication.|
//! | `--fps <n>`         | `TZE_HUD_FPS`          | `60`         | Target frames per second.                |
//! | `--help`            | —                      | —            | Print this help and exit.                |
//! | `--version`         | —                      | —            | Print version and exit.                  |
//!
//! ¹ In overlay mode, the primary monitor resolution is auto-detected at startup
//!   via winit. Falls back to `1920` (width) / `1080` (height) if detection fails
//!   (headless environment, no display server). Explicit `--width`/`--height` flags
//!   or `TZE_HUD_WINDOW_WIDTH`/`TZE_HUD_WINDOW_HEIGHT` env vars override
//!   auto-detection. In fullscreen mode, `1920×1080` is the default (the compositor
//!   uses `Fullscreen::Borderless`, which always uses the monitor's native resolution).
//!
//! ## Config file resolution order
//!
//! 1. `--config <path>` CLI flag
//! 2. `$TZE_HUD_CONFIG` environment variable
//! 3. `./tze_hud.toml` in the current working directory
//! 4. `$XDG_CONFIG_HOME/tze_hud/config.toml` (Linux/macOS)
//! 5. `%APPDATA%\tze_hud\config.toml` (Windows)
//!
//! The loader schema is driven by `[runtime]` and `[[tabs]]` (plus optional
//! sections such as `[agents]`, `[widget_bundles]`, and `[component_profiles]`).
//! Legacy `[display]`/`[network]` config tables are not part of the current schema.
//!
//! In the canonical operator path, startup is fail-closed: a readable, valid
//! config file is required and the trivial default PSK is rejected. Debug/dev
//! runs may explicitly opt into insecure fallback behavior by setting
//! `TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1`.
//! Passing `--config` with a path that does not exist or cannot be read is a
//! hard error.
//!
//! ## Examples
//!
//! ```sh
//! # Fullscreen (default)
//! tze_hud
//!
//! # Overlay mode at 1280×720 with gRPC enabled
//! tze_hud --window-mode overlay --width 1280 --height 720 --grpc-port 50051
//!
//! # Load explicit config file
//! tze_hud --config /etc/tze_hud/config.toml
//!
//! # Disable gRPC (standalone compositor only)
//! tze_hud --grpc-port 0
//! ```

use tze_hud_config::{reload_config, resolve_config_path};
use tze_hud_runtime::window::{WindowConfig, WindowMode};
use tze_hud_runtime::windowed::{WindowedConfig, WindowedRuntime};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = "tze_hud";
const DEFAULT_PSK: &str = "tze-hud-key";
const DEV_ALLOW_INSECURE_STARTUP_ENV: &str = "TZE_HUD_DEV_ALLOW_INSECURE_STARTUP";

fn print_help() {
    println!(
        r#"{BIN_NAME} {VERSION}
Canonical tze_hud windowed display runtime.

USAGE:
    {BIN_NAME} [OPTIONS]

OPTIONS:
    --config <path>        Path to TOML config file
                           (env: TZE_HUD_CONFIG; auto-resolved if omitted)
    --window-mode <mode>   Window mode: fullscreen | overlay  [default: fullscreen]
                           (env: TZE_HUD_WINDOW_MODE)
    --width <px>           Window width in pixels  [default: auto-detect in overlay mode, 1920 otherwise]
                           (env: TZE_HUD_WINDOW_WIDTH)
    --height <px>          Window height in pixels  [default: auto-detect in overlay mode, 1080 otherwise]
                           (env: TZE_HUD_WINDOW_HEIGHT)
    --grpc-port <port>     gRPC listen port; 0 to disable  [default: 50051]
                           (env: TZE_HUD_GRPC_PORT)
    --mcp-port <port>      MCP HTTP listen port; 0 to disable  [default: 9090]
                           (env: TZE_HUD_MCP_PORT)
    --psk <key>            Pre-shared key for session authentication  [default: tze-hud-key]
                           (env: TZE_HUD_PSK)
    --fps <n>              Target frames per second  [default: 60]
                           (env: TZE_HUD_FPS)
    --help                 Print this help and exit
    --version              Print version and exit

NOTES:
    This binary is the canonical production entrypoint for tze_hud. It starts
    the windowed display runtime with a real wgpu swapchain and winit event loop.
    For headless/CI usage, use the tze_hud_runtime crate directly with
    HeadlessRuntime.

    Canonical startup is fail-closed: a readable, valid config file is required.
    Strict mode also rejects the trivial default PSK value.
    For debug/dev runs only, set TZE_HUD_DEV_ALLOW_INSECURE_STARTUP=1 to permit
    fallback startup behavior without a config file. In canonical startup, the
    required config file uses the loader schema rooted at [runtime] and [[tabs]]
    (plus optional sections such as [agents], [widget_bundles], and
    [component_profiles]). In insecure dev mode, the same schema applies when a
    config file is provided. Legacy [display]/[network] tables are unsupported.
    CLI flags override individual settings from the config file.
    Passing --config with a path that does not exist or cannot be read is an error.
"#,
    );
}

fn print_version() {
    println!("{BIN_NAME} {VERSION}");
}

/// Parsed startup options.
#[derive(Debug)]
struct StartupOptions {
    config_path: Option<String>,
    window_mode: WindowMode,
    width: u32,
    height: u32,
    /// Whether `width` was explicitly set via `--width` or `TZE_HUD_WINDOW_WIDTH`.
    ///
    /// When `false` (the default), overlay mode auto-detects the primary monitor
    /// resolution at startup and ignores the default `width` value.
    explicit_width: bool,
    /// Whether `height` was explicitly set via `--height` or `TZE_HUD_WINDOW_HEIGHT`.
    ///
    /// When `false` (the default), overlay mode auto-detects the primary monitor
    /// resolution at startup and ignores the default `height` value.
    explicit_height: bool,
    grpc_port: u16,
    mcp_port: u16,
    psk: String,
    fps: u32,
    /// When true, render zone boundaries with colored debug tints.
    debug_zones: bool,
    /// Monitor index for overlay placement (0-based). `None` = primary monitor.
    monitor_index: Option<usize>,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            config_path: None,
            window_mode: WindowMode::Fullscreen,
            width: 1920,
            height: 1080,
            explicit_width: false,
            explicit_height: false,
            grpc_port: 50051,
            mcp_port: 9090,
            psk: DEFAULT_PSK.to_string(),
            fps: 60,
            debug_zones: false,
            monitor_index: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupSecurityMode {
    Strict,
    DevInsecureOverride,
}

fn startup_security_mode_for_env(
    dev_override_env: Option<&str>,
    is_debug_build: bool,
) -> StartupSecurityMode {
    if is_debug_build && dev_override_env == Some("1") {
        StartupSecurityMode::DevInsecureOverride
    } else {
        StartupSecurityMode::Strict
    }
}

fn startup_security_mode() -> StartupSecurityMode {
    startup_security_mode_for_env(
        std::env::var(DEV_ALLOW_INSECURE_STARTUP_ENV)
            .ok()
            .as_deref(),
        cfg!(debug_assertions),
    )
}

fn psk_is_trivial_default(psk: &str) -> bool {
    psk == DEFAULT_PSK
}

fn validate_config_toml_for_startup(toml_src: &str) -> Result<(), String> {
    reload_config(toml_src).map(|_| ()).map_err(|errors| {
        let mut rendered = String::new();
        for (idx, err) in errors.iter().enumerate() {
            if idx > 0 {
                rendered.push_str("; ");
            }
            rendered.push_str(&format!(
                "[{:?}] {} (expected: {}, got: {}, hint: {})",
                err.code, err.field_path, err.expected, err.got, err.hint
            ));
        }
        format!(
            "config validation failed with {} error(s): {}",
            errors.len(),
            rendered
        )
    })
}

/// Parse startup options from CLI arguments and environment variables.
///
/// CLI flags take priority over environment variables.
fn parse_options(args: &[String]) -> Result<StartupOptions, String> {
    let mut opts = StartupOptions::default();

    // Apply environment variables first (lowest priority).
    if let Ok(v) = std::env::var("TZE_HUD_WINDOW_MODE") {
        opts.window_mode = parse_window_mode(&v)?;
    }
    if let Ok(v) = std::env::var("TZE_HUD_WINDOW_WIDTH") {
        opts.width = v
            .parse::<u32>()
            .map_err(|_| format!("TZE_HUD_WINDOW_WIDTH: invalid integer: {v:?}"))?;
        opts.explicit_width = true;
    }
    if let Ok(v) = std::env::var("TZE_HUD_WINDOW_HEIGHT") {
        opts.height = v
            .parse::<u32>()
            .map_err(|_| format!("TZE_HUD_WINDOW_HEIGHT: invalid integer: {v:?}"))?;
        opts.explicit_height = true;
    }
    if let Ok(v) = std::env::var("TZE_HUD_GRPC_PORT") {
        opts.grpc_port = v
            .parse::<u16>()
            .map_err(|_| format!("TZE_HUD_GRPC_PORT: invalid port: {v:?}"))?;
    }
    if let Ok(v) = std::env::var("TZE_HUD_MCP_PORT") {
        opts.mcp_port = v
            .parse::<u16>()
            .map_err(|_| format!("TZE_HUD_MCP_PORT: invalid port: {v:?}"))?;
    }
    if let Ok(v) = std::env::var("TZE_HUD_PSK") {
        opts.psk = v;
    }
    if let Ok(v) = std::env::var("TZE_HUD_FPS") {
        opts.fps = v
            .parse::<u32>()
            .map_err(|_| format!("TZE_HUD_FPS: invalid integer: {v:?}"))?;
    }

    // Parse CLI flags (override env vars).
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                print_version();
                std::process::exit(0);
            }
            "--config" => {
                i += 1;
                opts.config_path = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--config requires a path argument".to_string())?,
                );
            }
            "--window-mode" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    "--window-mode requires an argument: fullscreen | overlay".to_string()
                })?;
                opts.window_mode = parse_window_mode(val)?;
            }
            "--width" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--width requires a pixel count argument".to_string())?;
                opts.width = val
                    .parse::<u32>()
                    .map_err(|_| format!("--width: invalid integer: {val:?}"))?;
                opts.explicit_width = true;
            }
            "--height" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--height requires a pixel count argument".to_string())?;
                opts.height = val
                    .parse::<u32>()
                    .map_err(|_| format!("--height: invalid integer: {val:?}"))?;
                opts.explicit_height = true;
            }
            "--grpc-port" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--grpc-port requires a port number argument".to_string())?;
                opts.grpc_port = val
                    .parse::<u16>()
                    .map_err(|_| format!("--grpc-port: invalid port: {val:?}"))?;
            }
            "--mcp-port" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--mcp-port requires a port number argument".to_string())?;
                opts.mcp_port = val
                    .parse::<u16>()
                    .map_err(|_| format!("--mcp-port: invalid port: {val:?}"))?;
            }
            "--psk" => {
                i += 1;
                opts.psk = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--psk requires a key argument".to_string())?;
            }
            "--fps" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--fps requires an integer argument".to_string())?;
                opts.fps = val
                    .parse::<u32>()
                    .map_err(|_| format!("--fps: invalid integer: {val:?}"))?;
            }
            "--debug-zones" => {
                opts.debug_zones = true;
            }
            "--monitor" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--monitor requires a monitor index (0-based)".to_string())?;
                opts.monitor_index = Some(
                    val.parse::<usize>()
                        .map_err(|_| format!("--monitor: invalid index: {val:?}"))?,
                );
            }
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "unknown flag: {flag}\nRun '{BIN_NAME} --help' for usage."
                ));
            }
            _ => {
                return Err(format!(
                    "unexpected positional argument: {}\nRun '{BIN_NAME} --help' for usage.",
                    args[i]
                ));
            }
        }
        i += 1;
    }

    Ok(opts)
}

fn parse_window_mode(s: &str) -> Result<WindowMode, String> {
    match s.to_lowercase().as_str() {
        "fullscreen" => Ok(WindowMode::Fullscreen),
        "overlay" => Ok(WindowMode::Overlay),
        other => Err(format!(
            "unknown window mode: {other:?}; expected \"fullscreen\" or \"overlay\""
        )),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialise structured logging. JSON if TZE_HUD_LOG_JSON=1.
    let log_json = std::env::var("TZE_HUD_LOG_JSON").as_deref() == Ok("1");
    if log_json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(tracing_subscriber::EnvFilter::from_env("TZE_HUD_LOG"))
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_env("TZE_HUD_LOG"))
            .init();
    }

    // Collect CLI args, skipping argv[0] (the binary name).
    let args: Vec<String> = std::env::args().skip(1).collect();

    let opts = parse_options(&args).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    // Resolve config file path and read its contents.
    // The resolved path is logged so operators can confirm which file is in use.
    // We track both the TOML content and the file path so that relative
    // [widget_bundles].paths entries can be resolved relative to the config file's
    // parent directory (spec §Widget Bundle Configuration).
    let security_mode = startup_security_mode();
    let mut searched_paths_for_missing: Vec<String> = Vec::new();
    let mut config_read_error_detail: Option<String> = None;
    let (config_toml, config_file_path): (Option<String>, Option<String>) =
        match resolve_config_path(opts.config_path.as_deref()) {
            Ok(path) => {
                match std::fs::read_to_string(&path) {
                    Ok(toml_src) => {
                        tracing::info!(config_path = %path, "config file loaded");
                        (Some(toml_src), Some(path))
                    }
                    Err(io_err) => {
                        // If a path was explicitly given via --config, this is a hard error.
                        // If it was auto-resolved, treat it as a warning and continue.
                        if opts.config_path.is_some() {
                            eprintln!("error: failed to read config file {path:?}: {io_err}");
                            std::process::exit(1);
                        }
                        searched_paths_for_missing = vec![path.clone()];
                        config_read_error_detail =
                            Some(format!("failed to read config file {path:?}: {io_err}"));
                        if security_mode == StartupSecurityMode::Strict {
                            tracing::warn!(
                                config_path = %path,
                                error = %io_err,
                                "config file found but not readable; strict startup will fail"
                            );
                        } else {
                            tracing::warn!(
                                config_path = %path,
                                error = %io_err,
                                "config file found but not readable; using flag/env-var defaults"
                            );
                        }
                        (None, None)
                    }
                }
            }
            Err(searched) => {
                searched_paths_for_missing = searched;
                // No config file found at any location.
                if opts.config_path.is_some() {
                    // --config was given explicitly but the file was not found.
                    // This is a hard error (RFC 0006 §1.3).
                    eprintln!(
                        "error: config file not found: {}",
                        searched_paths_for_missing
                            .first()
                            .map(String::as_str)
                            .unwrap_or("(unknown path)")
                    );
                    std::process::exit(1);
                }
                if security_mode == StartupSecurityMode::Strict {
                    tracing::debug!(
                        searched = ?searched_paths_for_missing,
                        "no config file found; strict startup will fail"
                    );
                } else {
                    // Config files are optional in dev-insecure override mode.
                    tracing::debug!(
                        searched = ?searched_paths_for_missing,
                        "no config file found; using flag/env-var defaults"
                    );
                }
                (None, None)
            }
        };

    if security_mode == StartupSecurityMode::Strict {
        if config_toml.is_none() {
            let searched_joined = if searched_paths_for_missing.is_empty() {
                "(no search paths reported)".to_string()
            } else {
                searched_paths_for_missing.join(", ")
            };
            eprintln!(
                "error: canonical startup requires a readable config file; searched: {searched_joined}"
            );
            if let Some(detail) = &config_read_error_detail {
                eprintln!("detail: {detail}");
            }
            eprintln!(
                "hint: this fail-closed behavior is mandatory for production startup. \
set {DEV_ALLOW_INSECURE_STARTUP_ENV}=1 only in debug/dev runs if you need fallback defaults."
            );
            std::process::exit(1);
        }

        if psk_is_trivial_default(&opts.psk) {
            eprintln!(
                "error: refusing startup with default PSK value {:?} in strict mode",
                DEFAULT_PSK
            );
            eprintln!("hint: set --psk <strong-key> or TZE_HUD_PSK to a non-default secret.");
            std::process::exit(1);
        }

        let toml_src = config_toml
            .as_ref()
            .expect("strict mode already checked config_toml presence");
        if let Err(msg) = validate_config_toml_for_startup(toml_src) {
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
    } else {
        tracing::warn!(
            env = DEV_ALLOW_INSECURE_STARTUP_ENV,
            "development insecure startup override enabled; allowing permissive fallback behavior"
        );
    }

    // Auto-size is enabled for overlay mode when neither --width nor --height
    // was explicitly set (env var or CLI flag).  If either dimension was given
    // explicitly, auto-detection is disabled so the user's intent is honoured.
    let overlay_auto_size =
        opts.window_mode == WindowMode::Overlay && !opts.explicit_width && !opts.explicit_height;

    tracing::info!(
        version = VERSION,
        window_mode = %opts.window_mode,
        width = opts.width,
        height = opts.height,
        overlay_auto_size,
        grpc_port = opts.grpc_port,
        mcp_port = opts.mcp_port,
        fps = opts.fps,
        "tze_hud runtime starting"
    );

    let config = WindowedConfig {
        window: WindowConfig {
            mode: opts.window_mode,
            width: opts.width,
            height: opts.height,
            title: "tze_hud".to_string(),
        },
        overlay_auto_size,
        grpc_port: opts.grpc_port,
        mcp_port: opts.mcp_port,
        psk: opts.psk,
        target_fps: opts.fps,
        config_toml,
        config_file_path,
        debug_zones: opts.debug_zones,
        monitor_index: opts.monitor_index,
    };

    // Diagnostic: write resolved config to disk so we can verify args were parsed.
    let diag = format!(
        "mode={} width={} height={} auto_size={} grpc={} mcp={}\nargs={:?}\n",
        opts.window_mode,
        opts.width,
        opts.height,
        overlay_auto_size,
        opts.grpc_port,
        opts.mcp_port,
        std::env::args().collect::<Vec<_>>(),
    );
    let _ = std::fs::write("C:\\tze_hud\\logs\\startup_diag.txt", &diag);

    let runtime = WindowedRuntime::new(config);
    runtime.run()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Serialize all tests that mutate env vars.
    // Rust's test harness runs tests in parallel by default; without this mutex,
    // concurrent tests can observe or overwrite each other's env var changes,
    // causing data races (UB) and flaky failures.
    // Pattern mirrors tze_hud_compositor::renderer::ENV_VAR_MUTEX.
    static ENV_VAR_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── parse_window_mode ────────────────────────────────────────────────────

    #[test]
    fn parse_window_mode_fullscreen() {
        assert_eq!(
            parse_window_mode("fullscreen").unwrap(),
            WindowMode::Fullscreen
        );
        assert_eq!(
            parse_window_mode("FULLSCREEN").unwrap(),
            WindowMode::Fullscreen
        );
        assert_eq!(
            parse_window_mode("Fullscreen").unwrap(),
            WindowMode::Fullscreen
        );
    }

    #[test]
    fn parse_window_mode_overlay() {
        assert_eq!(parse_window_mode("overlay").unwrap(), WindowMode::Overlay);
        assert_eq!(parse_window_mode("OVERLAY").unwrap(), WindowMode::Overlay);
    }

    #[test]
    fn parse_window_mode_unknown_returns_error() {
        let err = parse_window_mode("windowed").unwrap_err();
        assert!(
            err.contains("windowed"),
            "error should mention the bad value"
        );
        assert!(
            err.contains("fullscreen") || err.contains("overlay"),
            "error should mention valid values"
        );
    }

    // ── parse_options: defaults ───────────────────────────────────────────────

    #[test]
    fn parse_options_defaults_when_no_args() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard; no other test
        // touches these vars while _guard is held. Rust 2024 requires unsafe
        // for remove_var.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
            std::env::remove_var("TZE_HUD_GRPC_PORT");
            std::env::remove_var("TZE_HUD_MCP_PORT");
            std::env::remove_var("TZE_HUD_PSK");
            std::env::remove_var("TZE_HUD_FPS");
        }

        let opts = parse_options(&[]).unwrap();
        assert_eq!(opts.window_mode, WindowMode::Fullscreen);
        assert_eq!(opts.width, 1920);
        assert_eq!(opts.height, 1080);
        assert_eq!(opts.grpc_port, 50051);
        assert_eq!(opts.mcp_port, 9090);
        assert_eq!(opts.fps, 60);
        assert!(opts.config_path.is_none());
    }

    // ── parse_options: CLI flags ─────────────────────────────────────────────

    #[test]
    fn parse_options_window_mode_overlay() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
        }
        let args: Vec<String> = vec!["--window-mode".to_string(), "overlay".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.window_mode, WindowMode::Overlay);
    }

    #[test]
    fn parse_options_width_and_height() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
        let args: Vec<String> = vec![
            "--width".to_string(),
            "1280".to_string(),
            "--height".to_string(),
            "720".to_string(),
        ];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.width, 1280);
        assert_eq!(opts.height, 720);
    }

    #[test]
    fn parse_options_grpc_port_zero_disables() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_GRPC_PORT");
        }
        let args: Vec<String> = vec!["--grpc-port".to_string(), "0".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.grpc_port, 0);
    }

    #[test]
    fn parse_options_mcp_port() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_MCP_PORT");
        }
        let args: Vec<String> = vec!["--mcp-port".to_string(), "8080".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.mcp_port, 8080);
    }

    #[test]
    fn parse_options_mcp_port_zero_disables() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_MCP_PORT");
        }
        let args: Vec<String> = vec!["--mcp-port".to_string(), "0".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.mcp_port, 0);
    }

    #[test]
    fn parse_options_fps() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_FPS");
        }
        let args: Vec<String> = vec!["--fps".to_string(), "30".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.fps, 30);
    }

    #[test]
    fn parse_options_config_path() {
        let args: Vec<String> = vec![
            "--config".to_string(),
            "/etc/tze_hud/config.toml".to_string(),
        ];
        let opts = parse_options(&args).unwrap();
        assert_eq!(
            opts.config_path.as_deref(),
            Some("/etc/tze_hud/config.toml")
        );
    }

    #[test]
    fn parse_options_psk() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_PSK");
        }
        let args: Vec<String> = vec!["--psk".to_string(), "my-secret-key".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.psk, "my-secret-key");
    }

    #[test]
    fn startup_security_mode_debug_with_override_is_dev_insecure() {
        let mode = startup_security_mode_for_env(Some("1"), true);
        assert_eq!(mode, StartupSecurityMode::DevInsecureOverride);
    }

    #[test]
    fn startup_security_mode_debug_without_override_is_strict() {
        let mode = startup_security_mode_for_env(None, true);
        assert_eq!(mode, StartupSecurityMode::Strict);
    }

    #[test]
    fn startup_security_mode_release_ignores_override_and_is_strict() {
        let mode = startup_security_mode_for_env(Some("1"), false);
        assert_eq!(mode, StartupSecurityMode::Strict);
    }

    #[test]
    fn psk_is_trivial_default_detects_default_only() {
        assert!(psk_is_trivial_default(DEFAULT_PSK));
        assert!(!psk_is_trivial_default("test-psk-do-not-use"));
    }

    #[test]
    fn validate_config_toml_for_startup_accepts_minimal_valid_config() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
        let result = validate_config_toml_for_startup(toml);
        assert!(result.is_ok(), "valid config should pass, got: {result:?}");
    }

    #[test]
    fn validate_config_toml_for_startup_rejects_invalid_toml() {
        let bad = "not valid toml [";
        let result = validate_config_toml_for_startup(bad);
        assert!(
            result.is_err(),
            "invalid TOML must be rejected by startup validation"
        );
    }

    #[test]
    fn validate_config_toml_for_startup_rejects_validation_errors() {
        let invalid = r#"
[runtime]
profile = "full-display"
"#;
        let result = validate_config_toml_for_startup(invalid);
        assert!(
            result.is_err(),
            "config missing [[tabs]] must be rejected by startup validation"
        );
    }

    // ── parse_options: errors ─────────────────────────────────────────────────

    #[test]
    fn parse_options_unknown_flag_returns_error() {
        let args: Vec<String> = vec!["--unknown-flag".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(
            err.contains("unknown flag"),
            "error should mention unknown flag"
        );
    }

    #[test]
    fn parse_options_window_mode_missing_value_returns_error() {
        let args: Vec<String> = vec!["--window-mode".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(
            err.contains("--window-mode"),
            "error should mention the flag"
        );
    }

    #[test]
    fn parse_options_width_non_integer_returns_error() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
        }
        let args: Vec<String> = vec!["--width".to_string(), "bad".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(err.contains("--width"), "error should mention the flag");
    }

    #[test]
    fn parse_options_positional_arg_returns_error() {
        let args: Vec<String> = vec!["unexpected".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(
            err.contains("unexpected positional argument"),
            "error should explain positional arg"
        );
    }

    // ── Non-default dimension regression tests (hud-q5hx) ────────────────────
    //
    // Verify that the exact CLI invocation reported in hud-q5hx parses correctly.
    // The crash was triggered by `--window-mode overlay --width 2560 --height 1440`;
    // the root cause was in the windowed runtime's surface initialization, not
    // argument parsing, but these tests document the contract end-to-end.

    /// The exact command line from the bug report must parse without error and
    /// produce the correct overlay mode and 2560x1440 dimensions.
    #[test]
    fn parse_options_overlay_2560x1440_bug_repro_command() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }

        // Mirrors: tze_hud.exe --window-mode overlay --width 2560 --height 1440
        let args: Vec<String> = vec![
            "--window-mode".to_string(),
            "overlay".to_string(),
            "--width".to_string(),
            "2560".to_string(),
            "--height".to_string(),
            "1440".to_string(),
        ];
        let opts = parse_options(&args).expect("must parse without error");
        assert_eq!(
            opts.window_mode,
            WindowMode::Overlay,
            "window mode must be Overlay"
        );
        assert_eq!(opts.width, 2560, "width must be 2560");
        assert_eq!(opts.height, 1440, "height must be 1440");
    }

    /// Verify 4K (3840x2160) dimensions also parse correctly.
    #[test]
    fn parse_options_overlay_4k_dimensions() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }

        let args: Vec<String> = vec![
            "--window-mode".to_string(),
            "overlay".to_string(),
            "--width".to_string(),
            "3840".to_string(),
            "--height".to_string(),
            "2160".to_string(),
        ];
        let opts = parse_options(&args).expect("must parse without error");
        assert_eq!(opts.window_mode, WindowMode::Overlay);
        assert_eq!(opts.width, 3840);
        assert_eq!(opts.height, 2160);
    }

    // ── overlay_auto_size flag computation (hud-48ml) ─────────────────────────
    //
    // These tests verify the three-way interaction that controls whether the
    // windowed runtime should auto-detect the primary monitor resolution:
    // 1. overlay mode + no explicit dimensions → auto_size=true
    // 2. overlay mode + explicit --width/--height → auto_size=false (user intent)
    // 3. fullscreen mode → auto_size=false (fullscreen always uses monitor native)

    /// In overlay mode with no explicit dimensions, auto-detection must be enabled
    /// (acceptance criterion 1: overlay auto-sizes to primary monitor).
    #[test]
    fn overlay_mode_no_explicit_dims_enables_auto_size() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
        let args: Vec<String> = vec!["--window-mode".to_string(), "overlay".to_string()];
        let opts = parse_options(&args).expect("must parse");
        assert_eq!(opts.window_mode, WindowMode::Overlay);
        assert!(!opts.explicit_width, "width must not be marked explicit");
        assert!(!opts.explicit_height, "height must not be marked explicit");
        // Derived: overlay_auto_size would be true
        let overlay_auto_size = opts.window_mode == WindowMode::Overlay
            && !opts.explicit_width
            && !opts.explicit_height;
        assert!(
            overlay_auto_size,
            "overlay without explicit dims must enable auto-size"
        );
    }

    /// In overlay mode with explicit --width AND --height, auto-detection must be
    /// disabled (acceptance criterion 2: explicit flags override auto-detection).
    #[test]
    fn overlay_mode_with_explicit_dims_disables_auto_size() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
        let args: Vec<String> = vec![
            "--window-mode".to_string(),
            "overlay".to_string(),
            "--width".to_string(),
            "2560".to_string(),
            "--height".to_string(),
            "1440".to_string(),
        ];
        let opts = parse_options(&args).expect("must parse");
        assert!(
            opts.explicit_width,
            "width must be marked explicit when --width is given"
        );
        assert!(
            opts.explicit_height,
            "height must be marked explicit when --height is given"
        );
        let overlay_auto_size = opts.window_mode == WindowMode::Overlay
            && !opts.explicit_width
            && !opts.explicit_height;
        assert!(
            !overlay_auto_size,
            "explicit --width/--height must disable auto-size"
        );
    }

    /// In overlay mode with only --width set, auto-detection is disabled
    /// (either dimension being explicit disables auto-size for consistency).
    #[test]
    fn overlay_mode_with_explicit_width_only_disables_auto_size() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
        let args: Vec<String> = vec![
            "--window-mode".to_string(),
            "overlay".to_string(),
            "--width".to_string(),
            "1280".to_string(),
        ];
        let opts = parse_options(&args).expect("must parse");
        assert!(opts.explicit_width, "explicit_width must be set");
        assert!(!opts.explicit_height, "explicit_height must not be set");
        let overlay_auto_size = opts.window_mode == WindowMode::Overlay
            && !opts.explicit_width
            && !opts.explicit_height;
        assert!(
            !overlay_auto_size,
            "any explicit dimension must disable auto-size"
        );
    }

    /// In fullscreen mode, auto-size is always disabled regardless of explicit dims
    /// (fullscreen handles sizing via Fullscreen::Borderless, not overlay path).
    #[test]
    fn fullscreen_mode_never_enables_auto_size() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
        let opts = parse_options(&[]).expect("must parse");
        assert_eq!(opts.window_mode, WindowMode::Fullscreen);
        let overlay_auto_size = opts.window_mode == WindowMode::Overlay
            && !opts.explicit_width
            && !opts.explicit_height;
        assert!(
            !overlay_auto_size,
            "fullscreen mode must never enable overlay auto-size"
        );
    }

    /// Explicit width/height via environment variables also disables auto-size.
    #[test]
    fn overlay_mode_with_env_var_dims_disables_auto_size() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        // Safety: single-threaded within ENV_VAR_MUTEX guard.
        unsafe {
            std::env::set_var("TZE_HUD_WINDOW_MODE", "overlay");
            std::env::set_var("TZE_HUD_WINDOW_WIDTH", "3840");
            std::env::set_var("TZE_HUD_WINDOW_HEIGHT", "2160");
        }
        let opts = parse_options(&[]).expect("must parse");
        assert_eq!(opts.window_mode, WindowMode::Overlay);
        assert_eq!(opts.width, 3840);
        assert_eq!(opts.height, 2160);
        assert!(opts.explicit_width, "env-var width must count as explicit");
        assert!(
            opts.explicit_height,
            "env-var height must count as explicit"
        );
        let overlay_auto_size = opts.window_mode == WindowMode::Overlay
            && !opts.explicit_width
            && !opts.explicit_height;
        assert!(
            !overlay_auto_size,
            "env-var explicit dims must disable auto-size"
        );
        // Clean up.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
    }
}
