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
//! | `--config <path>`   | `TZE_HUD_CONFIG`       | (auto-resolved) | Path to TOML config file.             |
//! | `--window-mode <m>` | `TZE_HUD_WINDOW_MODE`  | `fullscreen` | Window mode: `fullscreen` or `overlay`.  |
//! | `--width <px>`      | `TZE_HUD_WINDOW_WIDTH` | `1920`       | Window width in pixels.                  |
//! | `--height <px>`     | `TZE_HUD_WINDOW_HEIGHT`| `1080`       | Window height in pixels.                 |
//! | `--grpc-port <port>`| `TZE_HUD_GRPC_PORT`    | `50051`      | gRPC listen port (0 to disable).         |
//! | `--psk <key>`       | `TZE_HUD_PSK`          | `tze-hud-key`| Pre-shared key for session authentication.|
//! | `--fps <n>`         | `TZE_HUD_FPS`          | `60`         | Target frames per second.                |
//! | `--help`            | —                      | —            | Print this help and exit.                |
//! | `--version`         | —                      | —            | Print version and exit.                  |
//!
//! ## Config file resolution order
//!
//! 1. `--config <path>` CLI flag
//! 2. `$TZE_HUD_CONFIG` environment variable
//! 3. `./tze_hud.toml` in the current working directory
//! 4. `$XDG_CONFIG_HOME/tze_hud/config.toml` (Linux/macOS)
//! 5. `%APPDATA%\tze_hud\config.toml` (Windows)
//!
//! If no config file is found, the runtime starts with flag/env-var defaults.
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

use tze_hud_runtime::windowed::{WindowedConfig, WindowedRuntime};
use tze_hud_runtime::window::{WindowConfig, WindowMode};
use tze_hud_config::resolve_config_path;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = "tze_hud";

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
    --width <px>           Window width in pixels  [default: 1920]
                           (env: TZE_HUD_WINDOW_WIDTH)
    --height <px>          Window height in pixels  [default: 1080]
                           (env: TZE_HUD_WINDOW_HEIGHT)
    --grpc-port <port>     gRPC listen port; 0 to disable  [default: 50051]
                           (env: TZE_HUD_GRPC_PORT)
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

    The config file (if found) provides the agent capability policy and tab
    layout. CLI flags override individual settings from the config file.
    Passing --config with a path that does not exist is an error.
"#,
        BIN_NAME = BIN_NAME,
        VERSION = VERSION,
    );
}

fn print_version() {
    println!("{BIN_NAME} {VERSION}", BIN_NAME = BIN_NAME, VERSION = VERSION);
}

/// Parsed startup options.
#[derive(Debug)]
struct StartupOptions {
    config_path: Option<String>,
    window_mode: WindowMode,
    width: u32,
    height: u32,
    grpc_port: u16,
    psk: String,
    fps: u32,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            config_path: None,
            window_mode: WindowMode::Fullscreen,
            width: 1920,
            height: 1080,
            grpc_port: 50051,
            psk: "tze-hud-key".to_string(),
            fps: 60,
        }
    }
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
    }
    if let Ok(v) = std::env::var("TZE_HUD_WINDOW_HEIGHT") {
        opts.height = v
            .parse::<u32>()
            .map_err(|_| format!("TZE_HUD_WINDOW_HEIGHT: invalid integer: {v:?}"))?;
    }
    if let Ok(v) = std::env::var("TZE_HUD_GRPC_PORT") {
        opts.grpc_port = v
            .parse::<u16>()
            .map_err(|_| format!("TZE_HUD_GRPC_PORT: invalid port: {v:?}"))?;
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
                let val = args
                    .get(i)
                    .ok_or_else(|| "--window-mode requires an argument: fullscreen | overlay".to_string())?;
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
            }
            "--height" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--height requires a pixel count argument".to_string())?;
                opts.height = val
                    .parse::<u32>()
                    .map_err(|_| format!("--height: invalid integer: {val:?}"))?;
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
            flag if flag.starts_with('-') => {
                return Err(format!("unknown flag: {flag}\nRun '{BIN_NAME} --help' for usage."));
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
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_env("TZE_HUD_LOG"),
            )
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_env("TZE_HUD_LOG"),
            )
            .init();
    }

    // Collect CLI args, skipping argv[0] (the binary name).
    let args: Vec<String> = std::env::args().skip(1).collect();

    let opts = parse_options(&args).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    // Resolve config file path (for future config-file-driven startup).
    // The resolved path is logged so operators can confirm which file is in use.
    let config_path = resolve_config_path(opts.config_path.as_deref());
    match &config_path {
        Ok(path) => {
            tracing::info!(config_path = %path, "config file resolved");
        }
        Err(searched) => {
            // No config file found — run with flag/env-var defaults.
            // This is not an error; config files are optional when all required
            // settings are supplied via flags or defaults.
            tracing::debug!(
                searched = ?searched,
                "no config file found; using flag/env-var defaults"
            );
        }
    }

    tracing::info!(
        version = VERSION,
        window_mode = %opts.window_mode,
        width = opts.width,
        height = opts.height,
        grpc_port = opts.grpc_port,
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
        grpc_port: opts.grpc_port,
        psk: opts.psk,
        target_fps: opts.fps,
    };

    let runtime = WindowedRuntime::new(config);
    runtime.run()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_window_mode ────────────────────────────────────────────────────

    #[test]
    fn parse_window_mode_fullscreen() {
        assert_eq!(parse_window_mode("fullscreen").unwrap(), WindowMode::Fullscreen);
        assert_eq!(parse_window_mode("FULLSCREEN").unwrap(), WindowMode::Fullscreen);
        assert_eq!(parse_window_mode("Fullscreen").unwrap(), WindowMode::Fullscreen);
    }

    #[test]
    fn parse_window_mode_overlay() {
        assert_eq!(parse_window_mode("overlay").unwrap(), WindowMode::Overlay);
        assert_eq!(parse_window_mode("OVERLAY").unwrap(), WindowMode::Overlay);
    }

    #[test]
    fn parse_window_mode_unknown_returns_error() {
        let err = parse_window_mode("windowed").unwrap_err();
        assert!(err.contains("windowed"), "error should mention the bad value");
        assert!(err.contains("fullscreen") || err.contains("overlay"), "error should mention valid values");
    }

    // ── parse_options: defaults ───────────────────────────────────────────────

    #[test]
    fn parse_options_defaults_when_no_args() {
        // Safety: test process is single-threaded at this point; env mutation
        // is safe when no other threads read these vars concurrently.
        // Rust 2024 requires unsafe for remove_var.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_MODE");
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
            std::env::remove_var("TZE_HUD_GRPC_PORT");
            std::env::remove_var("TZE_HUD_PSK");
            std::env::remove_var("TZE_HUD_FPS");
        }

        let opts = parse_options(&[]).unwrap();
        assert_eq!(opts.window_mode, WindowMode::Fullscreen);
        assert_eq!(opts.width, 1920);
        assert_eq!(opts.height, 1080);
        assert_eq!(opts.grpc_port, 50051);
        assert_eq!(opts.fps, 60);
        assert!(opts.config_path.is_none());
    }

    // ── parse_options: CLI flags ─────────────────────────────────────────────

    #[test]
    fn parse_options_window_mode_overlay() {
        // Safety: see parse_options_defaults_when_no_args.
        unsafe { std::env::remove_var("TZE_HUD_WINDOW_MODE"); }
        let args: Vec<String> = vec!["--window-mode".to_string(), "overlay".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.window_mode, WindowMode::Overlay);
    }

    #[test]
    fn parse_options_width_and_height() {
        // Safety: see parse_options_defaults_when_no_args.
        unsafe {
            std::env::remove_var("TZE_HUD_WINDOW_WIDTH");
            std::env::remove_var("TZE_HUD_WINDOW_HEIGHT");
        }
        let args: Vec<String> = vec![
            "--width".to_string(), "1280".to_string(),
            "--height".to_string(), "720".to_string(),
        ];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.width, 1280);
        assert_eq!(opts.height, 720);
    }

    #[test]
    fn parse_options_grpc_port_zero_disables() {
        // Safety: see parse_options_defaults_when_no_args.
        unsafe { std::env::remove_var("TZE_HUD_GRPC_PORT"); }
        let args: Vec<String> = vec!["--grpc-port".to_string(), "0".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.grpc_port, 0);
    }

    #[test]
    fn parse_options_fps() {
        // Safety: see parse_options_defaults_when_no_args.
        unsafe { std::env::remove_var("TZE_HUD_FPS"); }
        let args: Vec<String> = vec!["--fps".to_string(), "30".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.fps, 30);
    }

    #[test]
    fn parse_options_config_path() {
        let args: Vec<String> = vec!["--config".to_string(), "/etc/tze_hud/config.toml".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.config_path.as_deref(), Some("/etc/tze_hud/config.toml"));
    }

    #[test]
    fn parse_options_psk() {
        // Safety: see parse_options_defaults_when_no_args.
        unsafe { std::env::remove_var("TZE_HUD_PSK"); }
        let args: Vec<String> = vec!["--psk".to_string(), "my-secret-key".to_string()];
        let opts = parse_options(&args).unwrap();
        assert_eq!(opts.psk, "my-secret-key");
    }

    // ── parse_options: errors ─────────────────────────────────────────────────

    #[test]
    fn parse_options_unknown_flag_returns_error() {
        let args: Vec<String> = vec!["--unknown-flag".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(err.contains("unknown flag"), "error should mention unknown flag");
    }

    #[test]
    fn parse_options_window_mode_missing_value_returns_error() {
        let args: Vec<String> = vec!["--window-mode".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(err.contains("--window-mode"), "error should mention the flag");
    }

    #[test]
    fn parse_options_width_non_integer_returns_error() {
        // Safety: see parse_options_defaults_when_no_args.
        unsafe { std::env::remove_var("TZE_HUD_WINDOW_WIDTH"); }
        let args: Vec<String> = vec!["--width".to_string(), "bad".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(err.contains("--width"), "error should mention the flag");
    }

    #[test]
    fn parse_options_positional_arg_returns_error() {
        let args: Vec<String> = vec!["unexpected".to_string()];
        let err = parse_options(&args).unwrap_err();
        assert!(err.contains("unexpected positional argument"), "error should explain positional arg");
    }
}
