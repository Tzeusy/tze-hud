# Cross-Machine Deployment and Validation Guide

This guide covers deploying the canonical `tze_hud_app` runtime binary to Windows for testing and validation workflows.

## Overview

The **canonical runtime app binary** (`tze_hud_app`) is the production-ready executable designed for:
- Cross-machine deployment (Linux build → Windows deploy)
- MCP HTTP endpoint exposure in windowed mode
- Automated zone publishing validation
- Configuration-driven network service startup

This is distinct from demo binaries (`vertical_slice`, `benchmark`, `render_artifacts`), which are development references.

## Canonical App Binary Identity

- **Crate**: Part of non-demo workspace members
- **Binary name**: `tze_hud_app`
- **Windows artifact path** (cross-compiled from Linux):
  - `target/x86_64-pc-windows-gnu/release/tze_hud_app.exe`
- **Windows native build**:
  - `target/x86_64-pc-windows-msvc/release/tze_hud_app.exe`
- **Remote deployment path** (default):
  - `C:\tze_hud\tze_hud_app.exe`

## Building the Windows Artifact

### Linux to Windows Cross-Compile (Recommended for Automation)

```bash
# From repository root
cd /home/tze/gt/tze_hud/mayor/rig

# Ensure Windows target is installed
rustup target add x86_64-pc-windows-gnu

# Install MinGW toolchain
sudo apt install -y mingw-w64

# Build canonical app for Windows
cargo build --bin tze_hud_app --release --target x86_64-pc-windows-gnu

# Verify artifact exists
ls -lh target/x86_64-pc-windows-gnu/release/tze_hud_app.exe

# Record checksum for integrity verification
sha256sum target/x86_64-pc-windows-gnu/release/tze_hud_app.exe
```

### Native Windows Build (Optional)

If building directly on Windows:

```powershell
# PowerShell (Developer VS 2022)
cargo build --bin tze_hud_app --release
# Output: target\x86_64-pc-windows-msvc\release\tze_hud_app.exe
```

## Configuration

The canonical app requires a TOML configuration file specifying:
- Display mode (windowed, headless, full)
- Window dimensions
- Network endpoint configuration (gRPC, MCP HTTP)
- MCP authentication (PSK)

### Example Configuration

Create `config.toml`:

```toml
# Display settings
[display]
width = 1920
height = 1080
mode = "windowed"

# Network services
[network]
# Enable gRPC session server
enable_grpc = true
grpc_bind = "127.0.0.1:50051"

# Enable MCP HTTP endpoint
enable_mcp_http = true
mcp_http_bind = "0.0.0.0:8765"  # Bind to all interfaces for remote reachability

# MCP authentication (required if enabled)
# Load from environment:
# mcp_psk_env = "MCP_APP_PSK"
# Or inline (not recommended for production):
mcp_psk = "test-shared-secret"
```

### Deployment Configuration

For remote validation, ensure Windows deployment includes:

```toml
[display]
mode = "windowed"

[network]
enable_mcp_http = true
mcp_http_bind = "0.0.0.0:8765"  # Must be reachable from deployment host
mcp_psk_env = "MCP_TEST_PSK"
```

Configuration file should be deployed alongside the `.exe`:
- Linux source: `config.toml`
- Windows target: `C:\tze_hud\config.toml`

## Deployment Automation

### Prerequisites

1. **SSH key-based authentication** to Windows host
   ```bash
   ssh-keygen -t ecdsa -f ~/.ssh/ecdsa_home  # Generate if needed
   ```

2. **Windows SSH server** (e.g., OpenSSH for Windows)
   - User: `hudbot` (default, customizable)
   - Host: `tzehouse-windows.parrot-hen.ts.net` (default, customizable)
   - Port: 22 (SSH)

3. **Network connectivity**
   - Windows host reachable from Linux
   - MCP HTTP port (8765) reachable for publish validation
   - (Optional) Tailnet/VPN for secure SSH tunnel

### SSH Connectivity Gate

Always verify SSH access BEFORE deploying:

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
```

**Expected output**: `hudbot`

If this fails, do not proceed. Troubleshoot:
- SSH key permissions: `chmod 600 ~/.ssh/ecdsa_home`
- Windows SSH server running: `Get-Service sshd | Start-Service` (Windows PowerShell)
- Firewall allows port 22

### Automated Deploy with Deploy Script

The `.claude/skills/user-test/scripts/deploy_windows_hud.sh` script handles:
1. Building or locating prebuilt `.exe`
2. Copying to Windows via SCP
3. Creating remote directories
4. Launching via scheduled task or direct PowerShell
5. Tailing logs (optional)

**Usage:**

```bash
# With prebuilt canonical app exe (recommended)
WIN_USER=hudbot \
SSH_OPTS='-i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes' \
./.claude/skills/user-test/scripts/deploy_windows_hud.sh \
  --win-host tzehouse-windows.parrot-hen.ts.net \
  --full-app-exe target/x86_64-pc-windows-gnu/release/tze_hud_app.exe \
  --launch-mode auto \
  --tail

# Expected output:
# Deploy complete.
# Remote exe: C:\tze_hud\tze_hud_app.exe
# Remote stdout log: C:\tze_hud\logs\hud.stdout.log
# Remote stderr log: C:\tze_hud\logs\hud.stderr.log
# [Tailing launcher log...]
```

**Options:**
- `--win-user` (default: `hudbot`)
- `--win-host` (default: `tzehouse-windows.parrot-hen.ts.net`)
- `--full-app-exe` (path to prebuilt `.exe`, recommended)
- `--launch-mode` (auto, task, direct - default: auto)
- `--tail` (tail launcher logs after deploy)
- `--no-run` (copy only, do not launch)

## MCP Reachability Gate

**Critical**: Before claiming publish success, verify MCP endpoint reachability.

### Reachability Check

```bash
# Test with curl
curl -v http://tzehouse-windows.parrot-hen.ts.net:8765
```

**Expected:** Connection accepted, HTTP response with MCP error (expected, no auth).

**Failure modes:**
- `Connection refused`: Runtime not accepting connections. Check logs.
- `Connection timed out`: Firewall blocks port, network unreachable.
- `HTTP 401/403`: Endpoint live but authentication failed. Check PSK.

### Diagnostic Output

If unreachable, collect:

1. **Remote runtime logs:**
   ```bash
   ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
     "type C:\\tze_hud\\logs\\hud.stdout.log"
   ```

2. **Process state:**
   ```bash
   ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
     "powershell -Command 'Get-Process tze_hud_app -ErrorAction SilentlyContinue | Select ProcessName, Id, Handles'"
   ```

3. **Network binding (Windows):**
   ```bash
   ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
     "powershell -Command 'netstat -an | findstr :8765'"
   ```

## Publish Zone Messages

Once MCP endpoint is verified reachable, publish test zones.

### Zone Publish Shape

```json
[
  {
    "zone_name": "status-bar",
    "content": "Test message from deployment workflow",
    "merge_key": "deployment-status",
    "ttl_us": 60000000,
    "namespace": "butler-test"
  }
]
```

**Fields:**
- `zone_name` (required): Target zone identifier
- `content` (required): Message content
- `merge_key` (optional): For content merging
- `ttl_us` (optional): Time-to-live in microseconds
- `namespace` (optional): Message namespace

### Publish via Python Script

```bash
python3 ./.claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
  --psk-env MCP_TEST_PSK \
  --messages-file /tmp/test-zones.json \
  --list-zones
```

## Troubleshooting

### SSH Connection Issues

**Problem:** `Permission denied (publickey)`

```bash
# Check key permissions
ls -la ~/.ssh/ecdsa_home
# Should be: -rw------- (600)
chmod 600 ~/.ssh/ecdsa_home

# Test key auth
ssh -v -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net whoami
```

**Problem:** `Connection refused`

```bash
# Windows: Verify SSH server is running
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Get-Service sshd | Select Status'"

# If stopped, start it:
# Get-Service sshd | Start-Service -Force
```

### Deployment Issues

**Problem:** `Local exe not found: target/x86_64-pc-windows-gnu/release/tze_hud_app.exe`

```bash
# Rebuild
cargo build --bin tze_hud_app --release --target x86_64-pc-windows-gnu
```

**Problem:** Remote directory creation fails

```bash
# Verify Windows user has permission to C:\tze_hud
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Test-Path C:\\ -PathType Container'"

# Verify write permission
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command '[io.file]::WriteAllText(\"C:\\tze_hud\\test.txt\", \"ok\")'"
```

### Runtime Launch Issues

**Problem:** MCP endpoint unreachable

1. Check remote process running:
   ```bash
   ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
     "powershell -Command 'Get-Process tze_hud_app -ErrorAction SilentlyContinue'"
   ```

2. Check runtime logs:
   ```bash
   ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
     "powershell -Command 'Get-Content C:\\tze_hud\\logs\\hud.stdout.log -Tail 100'"
   ```

3. Check network binding:
   ```bash
   ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
     "netstat -an | findstr :8765"
   ```

**Problem:** MCP publish rejected with 401

- Verify PSK environment variable is set: `echo $MCP_TEST_PSK`
- Verify PSK matches Windows config
- Check MCP log output in `hud.stdout.log`

**Problem:** Configuration file not found

```bash
# Ensure config.toml is deployed
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Get-Item C:\\tze_hud\\config.toml'"

# Copy if missing
scp config.toml hudbot@tzehouse-windows.parrot-hen.ts.net:C:\\tze_hud\\config.toml
```

### Cleanup

**Stop running runtime:**

```bash
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command \"Get-Process tze_hud_app -ErrorAction SilentlyContinue | Stop-Process -Force\""
```

**Remove deployment files:**

```bash
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command \"Remove-Item -Path C:\\tze_hud -Recurse -Force\""
```

## Best Practices

1. **Always perform the MCP Reachability Gate** before claiming publish success
2. **Record checksums** before/after deployment for artifact integrity
3. **Use prebuilt canonical app `.exe`** (--full-app-exe) rather than building on Windows
4. **Configure MCP HTTP bind to 0.0.0.0** for remote reachability in test environments
5. **Tail launcher logs** (--tail flag) to catch startup errors immediately
6. **Verify SSH key permissions** (600) to avoid authentication failures
7. **Keep configuration file in sync** with Windows remote path
8. **Use automation scripts** to avoid manual SSH complexity

## Related Documentation

- [README.md](../README.md) - Build and test overview
- `docs/RUNTIME_APP_BINARY.md` - Canonical app binary specification
- `docs/OPERATOR_CHECKLIST.md` - Operator deployment checklist
- `.claude/skills/user-test/SKILL.md` - User-test skill documentation
- `openspec/changes/ship-runtime-app-binary/` - Specification artifacts
