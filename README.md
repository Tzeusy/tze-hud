# tze_hud Build/Test/Run Commands

This README is command-first and focused on four workflows:

1. Build on Linux and Windows
2. Build/run on Linux inside TigerVNC and connect from Windows
3. Run all test categories
4. Trigger zone publishing to the server and verify UI-control/overlay path

## Overview: Canonical Runtime App vs. Demo Binaries

**Important:** This project distinguishes between the **canonical runtime application binary** and demo/reference binaries.

### Canonical Runtime App Binary
- **Purpose**: Production-ready runtime executable for cross-machine deployment and MCP publishing operations.
- **Binary name** (TBD): `tze_hud_app` (canonical application binary, part of a non-demo binary target in Cargo workspace)
- **Windows artifact**: `target/x86_64-pc-windows-gnu/release/tze_hud_app.exe`
- **Configuration**: Supports TOML configuration file with windowed display settings and network endpoint configuration.
- **Network support**: Includes full `NetworkRuntime` with MCP HTTP listener lifecycle in windowed mode.
- **Use case**: Remote deployment, cross-machine validation, automated publish workflows.

### Demo and Reference Binaries
- `vertical_slice` (`examples/vertical_slice/`): Development reference showing scene/lease/zone publish semantics. **Not** intended for operations or remote deployment.
- `benchmark` (`examples/benchmark/`): Performance profiling reference.
- `render_artifacts` (`examples/render_artifacts/`): GPU rendering artifact generation.

**Rule**: Automation and cross-machine workflows MUST target the canonical app binary, not demo binaries.

## 1) Build on Linux / Windows

### Linux (Ubuntu/Debian) - Native Build

```bash
# System deps (Rust toolchain deps + protobuf compiler + common windowing libs)
sudo apt update
sudo apt install -y \
  build-essential pkg-config protobuf-compiler \
  libx11-dev libxrandr-dev libxi-dev libxcursor-dev libxinerama-dev \
  libxkbcommon-dev libwayland-dev

# Rust toolchain (workspace requires Rust 1.88+)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup toolchain install 1.88.0
rustup default 1.88.0

# Build entire workspace
cargo build --workspace
cargo build --workspace --release

# Build canonical runtime app binary only
cargo build --bin tze_hud_app --release
```

### Linux to Windows Cross-Compile (for deployment automation)

```bash
# Install Windows toolchain target
rustup target add x86_64-pc-windows-gnu

# Install MinGW toolchain (for cross-compilation)
sudo apt install -y mingw-w64

# Build canonical app for Windows target
cargo build --bin tze_hud_app --release --target x86_64-pc-windows-gnu

# Output artifact path:
# target/x86_64-pc-windows-gnu/release/tze_hud_app.exe
```

### Windows (PowerShell) - Native Build

```powershell
# Install toolchain/deps (run in elevated PowerShell)
winget install -e --id Rustlang.Rustup
winget install -e --id ProtocolBuffers.Protobuf
winget install -e --id Microsoft.VisualStudio.2022.BuildTools

# Rust toolchain
rustup toolchain install 1.88.0-x86_64-pc-windows-msvc
rustup default 1.88.0-x86_64-pc-windows-msvc

# Build (from repo root)
cargo build --workspace
cargo build --workspace --release

# Build canonical runtime app binary only
cargo build --bin tze_hud_app --release
```

If `cl.exe` is not found, run the build in **Developer PowerShell for VS 2022**.

**Output artifact path (Windows):**
```
target\x86_64-pc-windows-msvc\release\tze_hud_app.exe
```

## 1.1) Configuration for Canonical Runtime App

The canonical `tze_hud_app` binary requires a runtime configuration file (TOML format) to:
- Set window dimensions and display mode
- Enable/disable network endpoints (gRPC, MCP HTTP)
- Configure endpoint bind addresses and authentication

**Example configuration** (`config.toml`):

```toml
# Display configuration
[display]
width = 1920
height = 1080
# full, windowed, headless
mode = "windowed"

# Network services
[network]
# Enable gRPC session server (default: false)
enable_grpc = true
grpc_bind = "127.0.0.1:50051"

# Enable MCP HTTP endpoint (default: false)
enable_mcp_http = true
mcp_http_bind = "127.0.0.1:8765"

# MCP authentication (required if enabled)
# mcp_psk = "shared-secret-key"  # Or load from environment

# Example for headless network-only mode
# mode = "headless"
# Only gRPC and MCP endpoints, no window
```

**Runtime usage:**

```bash
# Windowed with network services enabled
./tze_hud_app --config config.toml

# Or on Windows with prebuilt binary
.\tze_hud_app.exe --config config.toml
```

For Windows deployment automation, see [Cross-Machine Deployment](#cross-machine-deployment) below.

## 1.2) Cross-Machine Deployment

The canonical `tze_hud_app` binary is designed for automated cross-machine deployment using SSH+SCP.

### Prerequisites

- Linux host with built canonical app binary for Windows target
- Windows remote host reachable via SSH (tailnet or VPN)
- SSH key-based authentication configured

### Deployment Workflow

**Step 1: Build Windows artifact on Linux**

```bash
# From repo root
cargo build --bin tze_hud_app --release --target x86_64-pc-windows-gnu
WINDOWS_EXE="target/x86_64-pc-windows-gnu/release/tze_hud_app.exe"
echo "Artifact ready: $WINDOWS_EXE"
```

**Step 2: Deploy and launch via user-test automation**

See [Cross-Machine Validation via user-test](#cross-machine-validation-via-user-test) below for the full automation script.

**Key deployment points:**
1. Verify SSH connectivity BEFORE deploying
2. Build or locate prebuilt canonical app `.exe`
3. Use deployment script to copy and launch on Windows
4. **Verify MCP HTTP reachability gate BEFORE publish assertions**
5. Publish zone test messages via MCP HTTP once endpoint is live

### Deployment Artifact Identity

For automation purposes, the canonical app binary produces:

- **Artifact name**: `tze_hud_app.exe` (stable, deterministic)
- **Linux build output**: `target/x86_64-pc-windows-gnu/release/tze_hud_app.exe`
- **Windows remote path**: `C:\tze_hud\tze_hud_app.exe` (default deployment location)
- **Checksum**: Use `sha256sum` on Linux before/after deployment for integrity verification

## 2) Linux + TigerVNC, then connect from Windows

### On Linux host (start VNC desktop)

```bash
# Install VNC server + lightweight desktop
sudo apt update
sudo apt install -y tigervnc-standalone-server tigervnc-common xfce4 xfce4-goodies

# Set VNC password (first run)
vncpasswd

# Create VNC startup script
cat > ~/.vnc/xstartup <<'XEOF'
#!/bin/sh
unset SESSION_MANAGER
unset DBUS_SESSION_BUS_ADDRESS
startxfce4 &
XEOF
chmod +x ~/.vnc/xstartup

# Start VNC display :1 (TCP 5901)
vncserver :1 -localhost no -geometry 1920x1080 -depth 24

# Run the windowed demo inside that display
export DISPLAY=:1
cargo run -p vertical_slice
```

### From Windows client

```powershell
# Secure option: tunnel VNC over SSH
ssh -L 5901:localhost:5901 <linux-user>@<linux-host>
```

Then open TigerVNC Viewer and connect to:

```text
localhost:5901
```

(Direct LAN option without tunnel: `<linux-host>:5901`.)

To stop VNC on Linux:

```bash
vncserver -kill :1
```

## 3) Run tests (all categories)

### Fast baseline (workspace tests except `integration` package)

```bash
cargo test --workspace --all-targets --exclude integration
```

### Scene/property tests

```bash
cargo test -p tze_hud_scene --test proptest_invariants -- --nocapture
cargo test -p tze_hud_scene --test fuzz_scene_graph -- --nocapture
```

### Protocol/session tests

```bash
cargo test -p tze_hud_protocol -- --nocapture
```

### Runtime/render validation tests

```bash
cargo test -p tze_hud_runtime --test pixel_readback -- --nocapture
cargo test -p tze_hud_validation --test layer2_headless -- --nocapture
cargo test -p tze_hud_validation --test layer4 -- --nocapture
```

### Integration tests

```bash
cargo test -p integration --test trace_regression -- --nocapture
cargo test -p integration --test soak -- --nocapture
```

Long soak runs:

```bash
TZE_HUD_SOAK_SECS=3600 cargo test -p integration --test soak -- --nocapture   # 1 hour
TZE_HUD_SOAK_SECS=21600 cargo test -p integration --test soak -- --nocapture  # 6 hours
```

Multi-agent integration tests:

```bash
cargo test -p integration --test multi_agent -- --nocapture
```

## 4) Trigger publish-to-server + UI-control/overlay checks

### A. Explicit server publish path (gRPC session server)

This test sends `ZonePublish` to the session server and checks `ZonePublishResult` behavior:

```bash
cargo test -p tze_hud_protocol test_durable_zone_publish_result -- --nocapture
cargo test -p tze_hud_protocol test_ephemeral_zone_no_publish_result -- --nocapture
```

### B. Development/Reference Demo (vertical_slice - NOT for operations)

The `vertical_slice` example is a **reference implementation** for understanding scene/lease/zone semantics.
**It is NOT intended for production operations or remote deployment.**

Run the demo locally for development/testing:

```bash
cargo run -p vertical_slice
```

You should see logs for:
- session + lease handshake,
- tile creation and hit-region input handling,
- zone publishes (`status-bar`, `notification-area`).

Headless variant (for server-side environments):

```bash
cargo run -p vertical_slice -- --headless
```

**For operational workflows**, use the **canonical runtime app binary** instead. See [Cross-Machine Deployment](#cross-machine-deployment) and [Cross-Machine Validation](#cross-machine-validation-via-user-test).

## 5) Cross-Machine Validation via user-test

For automated cross-machine deployment and MCP publish validation, use the `user-test` skill workflow.

### Workflow Overview

1. **Build canonical app for Windows target** (Linux cross-compile)
2. **Deploy to Windows** via SSH+SCP
3. **MCP Reachability Gate** - Verify endpoint is live before publish
4. **Publish test zones** - Validate MCP authentication and zone semantics
5. **Diagnostics** - Structured failure output on endpoint/auth mismatches

### Quick Start

**Prerequisites:**
- `~/.ssh/ecdsa_home` SSH key (or override via `SSH_OPTS`)
- Windows host: `tzehouse-windows.parrot-hen.ts.net` (or override `--win-host`)
- Windows SSH user: `hudbot` (or override `--win-user`)
- MCP test PSK in environment: `export MCP_TEST_PSK="..."`

**Step 1: Verify SSH connectivity**

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Must return `hudbot`. Do not proceed without successful key auth.

**Step 2: Build canonical app for Windows**

```bash
cargo build --bin tze_hud_app --release --target x86_64-pc-windows-gnu
FULL_APP_EXE="target/x86_64-pc-windows-gnu/release/tze_hud_app.exe"
```

**Step 3: Deploy and launch with MCP reachability gate**

```bash
# From repo root
WIN_USER=hudbot \
SSH_OPTS='-i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes' \
.claude/skills/user-test/scripts/deploy_windows_hud.sh \
  --win-host tzehouse-windows.parrot-hen.ts.net \
  --full-app-exe "$FULL_APP_EXE" \
  --launch-mode auto \
  --tail
```

**Expected output:**
- Remote exe path: `C:\tze_hud\tze_hud_app.exe`
- Launcher logs tail (remote)

**Step 4: Verify MCP endpoint reachability (MCP Reachability Gate)**

```bash
# Test MCP HTTP endpoint
curl -s -X POST http://tzehouse-windows.parrot-hen.ts.net:8765 \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $MCP_TEST_PSK" \
  -d '{"jsonrpc":"2.0","method":"list_resources","params":{},"id":1}' | jq .
```

If the endpoint is unreachable, stop and investigate launch logs. Do not proceed to publish.

**Step 5: Publish test zone messages via MCP HTTP**

```bash
# Create test message JSON
cat > /tmp/hud-test-zones.json <<'EOF'
[
  {
    "zone_name": "status-bar",
    "content": "Canonical app deployed and live",
    "merge_key": "deploy-status",
    "namespace": "butler-test"
  },
  {
    "zone_name": "notification-area",
    "content": "MCP publish validation successful",
    "merge_key": "mcp-test",
    "ttl_us": 60000000
  }
]
EOF

# Publish via MCP HTTP
python3 .claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
  --psk-env MCP_TEST_PSK \
  --messages-file /tmp/hud-test-zones.json
```

### Troubleshooting

**Symptom**: SSH connectivity fails at step 1
- Verify `~/.ssh/ecdsa_home` exists and has correct permissions
- Check Windows SSH server is running
- Verify firewall rules allow SSH (port 22)

**Symptom**: Deployment succeeds but MCP endpoint unreachable
- Check Windows target's `C:\tze_hud\logs\hud.stdout.log` and `hud.stderr.log`
- Verify MCP HTTP endpoint config in runtime config file
- Verify firewall allows HTTP (port 8765 by default) from Linux host

**Symptom**: MCP publish request rejected with 401/403
- Verify `MCP_TEST_PSK` environment variable is set
- Verify PSK matches value in Windows runtime config
- Check MCP authentication enforcement in runtime logs

### Debugging Tips

**Tail launcher logs on Windows:**

```bash
ssh -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command \"Get-Content -Path 'C:\\tze_hud\\logs\\hud.launcher.log' -Tail 50 -Wait\""
```

**Stop running runtime and check process state:**

```bash
ssh -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command \"Get-Process tze_hud_app -ErrorAction SilentlyContinue | Stop-Process -Force\""
```

**Verify artifact was copied:**

```bash
ssh -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command \"Get-Item 'C:\\tze_hud\\tze_hud_app.exe' | Select-Object FullName, Length, LastWriteTime\""
```
