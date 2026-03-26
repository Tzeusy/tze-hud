# tze_hud Build/Test/Run Commands

This README is command-first and focused on four workflows:

1. Build on Linux and Windows
2. Build/run on Linux inside TigerVNC and connect from Windows
3. Run all test categories
4. Trigger zone publishing to the server and verify UI-control/overlay path

## 1) Build on Linux / Windows

### Linux (Ubuntu/Debian)

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

# Build
cargo build --workspace
cargo build --workspace --release
```

### Windows (PowerShell)

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
```

If `cl.exe` is not found, run the build in **Developer PowerShell for VS 2022**.

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

Note: `tests/integration/multi_agent.rs` currently has known compile issues in this branch. Run it once fixed:

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

### B. Visual UI-control + overlay smoke test

Run the vertical slice demo (windowed):

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
