# Canonical Runtime Application Binary

## Overview

The **canonical runtime application binary** (`tze_hud`, from the `tze_hud_app` crate) is the production-ready, operator-facing executable for tze_hud. It is distinct from and should not be confused with demo/reference binaries.

## Binary Classifications

### Canonical Runtime App Binary

**Purpose**: Production runtime for cross-machine deployment, MCP publishing, and automated workflows.

**Location in codebase**:
- Crate: `tze_hud_app` (non-demo workspace member)
- Source: Main crates + minimal app entrypoint
- **Binary name**: `tze_hud`

**Key features**:
- Configuration-driven startup (window mode, dimensions, network endpoints)
- Full `NetworkRuntime` support in windowed mode
- MCP HTTP endpoint with authentication enforcement
- Deterministic Windows artifact naming for automation
- Production-ready lifecycle (clean startup, shutdown)

**Build targets**:
- Linux cross-compile to Windows: `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
- Windows native: `target/x86_64-pc-windows-msvc/release/tze_hud.exe`
- Linux native: `target/release/tze_hud` (or `target/x86_64-unknown-linux-gnu/release/tze_hud` when building that target explicitly)

**Use cases**:
- Remote deployment to Windows for cross-machine validation
- Automated zone publishing workflows
- Operator testing and integration scenarios
- Production-like environment simulation

**Configuration**: Uses the `TzeHudConfig` schema (`[runtime]` + `[[tabs]]` minimum). Window/network settings are selected via CLI/env flags.

**Deployment**: Via `deploy_windows_hud.sh` automation script with MCP reachability gating.

### Demo and Reference Binaries

**Purpose**: Development references for understanding architecture, semantics, and example patterns.

**Included demos**:

#### vertical_slice
- **Location**: `examples/vertical_slice/`
- **Binary name**: `vertical_slice`
- **Features**: Minimal example showing scene creation, lease handling, zone publishing
- **Dev mode**: Includes `dev-mode` feature flag for unrestricted capability grants
- **Use**: Learning, local prototyping, CI baseline validation
- **NOT for operations**: No remote deployment, unreliable network service startup
- **Build**: `cargo run -p vertical_slice` or `cargo run -p vertical_slice -- --headless`

#### benchmark
- **Location**: `examples/benchmark/`
- **Binary name**: `benchmark`
- **Features**: Performance profiling and throughput measurement
- **Use**: Performance regression detection, optimization validation
- **Build**: `cargo build -p benchmark --release`

#### render_artifacts
- **Location**: `examples/render_artifacts/`
- **Binary name**: `render_artifacts`
- **Features**: GPU rendering artifact generation
- **Use**: Render validation, visual regression testing
- **Build**: `cargo build -p render_artifacts --release`

## Key Differences

| Aspect | Canonical App | Demo Binaries |
|--------|---------------|---------------|
| **Purpose** | Production ops | Learning/testing |
| **Network services** | Full MCP/gRPC in windowed | No network services |
| **Configuration** | TOML config required | Dev-mode (unrestricted) |
| **Remote deployment** | Supported via automation | Not supported |
| **Artifact naming** | Deterministic, stable | Example-specific |
| **Authentication** | Enforced (MCP PSK) | None (dev-mode) |
| **Lifecycle** | Production-ready | Development simplicity |

## Building the Canonical App

### All Platforms (Debug)

```bash
cargo build --bin tze_hud
# Output: target/debug/tze_hud (or .exe on Windows)
```

### All Platforms (Release)

```bash
cargo build --bin tze_hud --release
# Output: target/release/tze_hud (or .exe on Windows)
```

### Linux Only (Release)

```bash
cargo build --bin tze_hud --release --target x86_64-unknown-linux-gnu
# Output: target/x86_64-unknown-linux-gnu/release/tze_hud
```

### Windows Native (Release)

```powershell
# Windows PowerShell (Developer VS 2022)
cargo build --bin tze_hud --release --target x86_64-pc-windows-msvc
# Output: target\x86_64-pc-windows-msvc\release\tze_hud.exe
```

### Linux Cross-Compile to Windows (Recommended for Automation)

```bash
# Ensure target and MinGW toolchain installed
rustup target add x86_64-pc-windows-gnu
sudo apt install -y mingw-w64

# Build
cargo build --bin tze_hud --release --target x86_64-pc-windows-gnu
# Output: target/x86_64-pc-windows-gnu/release/tze_hud.exe
```

## Configuration

The canonical app can load a TOML configuration file via `--config`, `TZE_HUD_CONFIG`, or auto-resolution (`./tze_hud.toml` first). The canonical committed operator config is:

- `app/tze_hud_app/config/production.toml`

For deployment, copy that file as `tze_hud.toml` beside `tze_hud.exe`.

### Loader Schema (Current)

Minimal valid config:

```toml
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
default_tab = true
```

Legacy `[display]`/`[network]` tables are not supported by the current loader.

### Runtime Endpoint/Window Controls

Window mode and network endpoint enable/disable are controlled by CLI/env:

- `--window-mode` / `TZE_HUD_WINDOW_MODE`
- `--grpc-port` / `TZE_HUD_GRPC_PORT` (`0` disables gRPC)
- `--mcp-port` / `TZE_HUD_MCP_PORT` (`0` disables MCP HTTP)
- `--psk` / `TZE_HUD_PSK`

Config loading examples:

```bash
./tze_hud --config /path/to/config.toml
# or on Windows:
.\tze_hud.exe --config C:\path\to\config.toml
```

## Runtime Lifecycle

### Startup

```
1. Parse CLI arguments (--config path)
2. Load TOML configuration
3. Initialize display subsystem (windowed, headless, or full mode)
4. If windowed:
   - Create OS window via winit
   - Start compositor
5. If network enabled:
   - Initialize NetworkRuntime
   - Bind gRPC listener (if enabled)
   - Bind MCP HTTP listener (if enabled)
   - Enforce MCP authentication
6. Signal readiness (logs, exit code)
```

### Shutdown

```
1. Signal received (SIGTERM, user close window, etc.)
2. Stop accepting new MCP/gRPC requests
3. Teardown network listeners gracefully
4. Stop compositor and GPU rendering
5. Close display (window)
6. Exit cleanly (code 0)
```

## Artifact Identity

For automation purposes, the canonical app produces stable, deterministic artifacts.

### Artifact Naming

- **Base name**: `tze_hud` (always the same)
- **Extension**: `.exe` on Windows, no extension on Unix-like
- **Full name examples**:
  - Windows: `tze_hud.exe`
  - Linux: `tze_hud`

### Build Output Paths

| Target | Profile | Path |
|--------|---------|------|
| Linux (native) | debug | `target/debug/tze_hud` |
| Linux (native) | release | `target/release/tze_hud` |
| Windows (cross) | debug | `target/x86_64-pc-windows-gnu/debug/tze_hud.exe` |
| Windows (cross) | release | `target/x86_64-pc-windows-gnu/release/tze_hud.exe` |
| Windows (native) | debug | `target/x86_64-pc-windows-msvc/debug/tze_hud.exe` |
| Windows (native) | release | `target/x86_64-pc-windows-msvc/release/tze_hud.exe` |

### Deployment Path (Windows)

- **Remote directory**: `C:\tze_hud\` (created by automation script)
- **Remote artifact**: `C:\tze_hud\tze_hud.exe`
- **Remote config**: `C:\tze_hud\config.toml`
- **Remote logs**:
  - `C:\tze_hud\logs\hud.stdout.log`
  - `C:\tze_hud\logs\hud.stderr.log`
  - `C:\tze_hud\logs\hud.launcher.log`

## Integration with Automation

### Automation Scripts

**Deploy and launch canonical app:**

```bash
./.claude/skills/user-test/scripts/deploy_windows_hud.sh \
  --full-app-exe target/x86_64-pc-windows-gnu/release/tze_hud.exe \
  --launch-mode auto \
  --tail
```

See `.claude/skills/user-test/SKILL.md` for full automation workflow.

### Expectations for Automation

1. **Artifact availability**: Build succeeds, artifact exists at expected path
2. **Deterministic naming**: Artifact name never changes across builds
3. **Configuration support**: App accepts `--config` and loads valid loader-schema TOML
4. **Network startup**: MCP/gRPC endpoints bind when their CLI/env ports are non-zero
5. **Authentication enforcement**: MCP rejects unauthenticated requests
6. **Clean shutdown**: Process terminates without hanging on listener teardown
7. **Structured logging**: Startup/shutdown events logged with clear messaging

## Verification Checklist

Before deploying to production or automation workflows:

- [ ] Binary builds successfully on all required platforms
- [ ] Artifact name is stable and deterministic
- [ ] `--config` CLI argument is parsed and config is loaded
- [ ] Display mode applies correctly (`fullscreen` or `overlay`)
- [ ] Network endpoints bind and accept connections when enabled
- [ ] MCP HTTP endpoint enforces authentication (rejects without valid PSK)
- [ ] Shutdown is clean (no hanging processes, logs on exit)
- [ ] Logs include startup and endpoint readiness messages
- [ ] Cross-platform Windows artifact works on target Windows version

## Related Documentation

- [DEPLOYMENT.md](DEPLOYMENT.md) - Deployment automation guide
- [README.md](../README.md) - Build and test overview
- [OPERATOR_CHECKLIST.md](OPERATOR_CHECKLIST.md) - Operator deployment checklist
- `app/tze_hud_app/tests/canonical_config_schema.rs` - Canonical config CI guard
- `.claude/skills/user-test/SKILL.md` - User-test automation skill
- `openspec/changes/ship-runtime-app-binary/` - Specification artifacts (design, requirements)
