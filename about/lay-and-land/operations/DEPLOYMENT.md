# Cross-Machine Deployment and Validation Guide

This guide covers deploying the canonical `tze_hud` runtime binary to Windows for testing and validation workflows.

## Overview

The **canonical runtime app binary** (`tze_hud`, from the `tze_hud_app` crate) is the production-ready executable designed for:
- Cross-machine deployment (Linux build → Windows deploy)
- MCP HTTP endpoint exposure in windowed mode
- Automated zone publishing validation
- Configuration-backed runtime bootstrap with CLI/env-controlled endpoint startup

This is distinct from demo binaries (`vertical_slice`, `benchmark`, `render_artifacts`), which are development references.

v1 scope note:
- Live media/WebRTC is explicitly deferred in v1 (`about/heart-and-soul/v1.md`).

## Canonical App Binary Identity

- **Crate**: `tze_hud_app` (non-demo workspace member)
- **Binary name**: `tze_hud`
- **Windows artifact path** (cross-compiled from Linux):
  - `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
- **Windows native build**:
  - `target/x86_64-pc-windows-msvc/release/tze_hud.exe`
- **Remote deployment path** (default):
  - `C:\tze_hud\tze_hud.exe`

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
cargo build --bin tze_hud --release --target x86_64-pc-windows-gnu

# Verify artifact exists
ls -lh target/x86_64-pc-windows-gnu/release/tze_hud.exe

# Record checksum for integrity verification
sha256sum target/x86_64-pc-windows-gnu/release/tze_hud.exe
```

### Native Windows Build (Optional)

If building directly on Windows:

```powershell
# PowerShell (Developer VS 2022)
cargo build --bin tze_hud --release
# Output: target\x86_64-pc-windows-msvc\release\tze_hud.exe
```

## Configuration

The canonical operator path uses the committed app config:
- `app/tze_hud_app/config/production.toml` (deploy as `C:\tze_hud\tze_hud.toml`)

Current loader schema minimum:
- `[runtime] profile = "..."`
- at least one `[[tabs]]`

Window mode and network endpoint controls are runtime flags/env vars, not `[display]`/`[network]` tables:
- `--window-mode` / `TZE_HUD_WINDOW_MODE` (`fullscreen` | `overlay`)
- `--grpc-port` / `TZE_HUD_GRPC_PORT` (`0` disables gRPC)
- `--mcp-port` / `TZE_HUD_MCP_PORT` (`0` disables MCP HTTP)
- `--psk` / `TZE_HUD_PSK`

### Example Configuration

Minimal schema example:

```toml
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
default_tab = true
```

### Deployment Configuration

For remote validation, launch with explicit endpoint and mode controls:

Configuration file should be deployed alongside the `.exe`:
- Linux source: `app/tze_hud_app/config/production.toml`
- Windows target: `C:\tze_hud\tze_hud.toml`

Example launch:

```powershell
C:\tze_hud\tze_hud.exe --config C:\tze_hud\tze_hud.toml --window-mode overlay --mcp-port 8765 --grpc-port 50051 --psk <shared-secret>
```

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
  --full-app-exe target/x86_64-pc-windows-gnu/release/tze_hud.exe \
  --launch-mode auto \
  --tail

# Expected output:
# Deploy complete.
# Remote exe: C:\tze_hud\tze_hud.exe
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
     "powershell -Command 'Get-Process tze_hud -ErrorAction SilentlyContinue | Select ProcessName, Id, Handles'"
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

**Problem:** `Local exe not found: target/x86_64-pc-windows-gnu/release/tze_hud.exe`

```bash
# Rebuild
cargo build --bin tze_hud --release --target x86_64-pc-windows-gnu
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
     "powershell -Command 'Get-Process tze_hud -ErrorAction SilentlyContinue'"
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
- Verify `MCP_TEST_PSK` matches the runtime launch secret (`--psk` or `TZE_HUD_PSK`)
- Check MCP log output in `hud.stdout.log`

**Problem:** Configuration file not found

```bash
# Ensure tze_hud.toml is deployed
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Get-Item C:\\tze_hud\\tze_hud.toml'"

# Copy if missing
scp app/tze_hud_app/config/production.toml hudbot@tzehouse-windows.parrot-hen.ts.net:C:\\tze_hud\\tze_hud.toml
```

### Cleanup

**Stop running runtime:**

```bash
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command \"Get-Process tze_hud -ErrorAction SilentlyContinue | Stop-Process -Force\""
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
- [RUNTIME_APP_BINARY.md](RUNTIME_APP_BINARY.md) - Canonical app binary specification
- [OPERATOR_CHECKLIST.md](OPERATOR_CHECKLIST.md) - Operator deployment checklist
- `app/tze_hud_app/tests/canonical_config_schema.rs` - Canonical config CI guard
- `.claude/skills/user-test/SKILL.md` - User-test skill documentation
- `openspec/changes/ship-runtime-app-binary/` - Specification artifacts
