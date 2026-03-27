# Operator Checklist: Canonical App Binary Deployment

This document provides operators and automation engineers with a checklist for using the canonical `tze_hud` binary for cross-machine validation and deployment.

## Pre-Deployment Checklist

### Build Artifact

- [ ] Canonical app binary built for Windows target
  - Command: `cargo build --bin tze_hud --release --target x86_64-pc-windows-gnu`
  - Expected artifact: `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
  - Verify file exists: `ls -lh target/x86_64-pc-windows-gnu/release/tze_hud.exe`
  - Record checksum: `sha256sum target/x86_64-pc-windows-gnu/release/tze_hud.exe`

### SSH Connectivity

- [ ] SSH key configured and permissions correct
  - Key file: `~/.ssh/ecdsa_home`
  - Permissions: `-rw-------` (600)
  - Fix if needed: `chmod 600 ~/.ssh/ecdsa_home`

- [ ] SSH connectivity verified to Windows host
  - Command: `ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"`
  - Expected output: `hudbot`
  - **Do NOT proceed without successful SSH key auth**

### Windows Environment

- [ ] Windows SSH server is running
  - Check on Windows: `Get-Service sshd | Select Status`
  - If stopped, start: `Get-Service sshd | Start-Service -Force`

- [ ] Windows host has network reachability
  - Ping from Linux: `ping -c 1 tzehouse-windows.parrot-hen.ts.net`
  - Expected: Response from host

- [ ] Firewall allows SSH (port 22) and MCP HTTP (port 8765)
  - On Windows: Open Windows Defender Firewall → Advanced Settings
  - Verify inbound rules allow port 22 (SSH) and 8765 (HTTP)

### Configuration

- [ ] Runtime configuration file prepared
  - Example template: `docs/RUNTIME_APP_BINARY.md` (Configuration Schema section)
  - Key settings for remote validation:
    - `[display] mode = "windowed"`
    - `[network] enable_mcp_http = true`
    - `[network] mcp_http_bind = "0.0.0.0:8765"` (remote reachability)
    - `[network] mcp_psk_env = "MCP_TEST_PSK"`

- [ ] MCP test PSK environment variable set
  - Command: `export MCP_TEST_PSK="<shared-secret>"`
  - Verify: `echo $MCP_TEST_PSK` should print the PSK

## Deployment Workflow

### Step 1: Deploy and Launch

- [ ] Run deployment script
  ```bash
  WIN_USER=hudbot \
  SSH_OPTS='-i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes' \
  ./.claude/skills/user-test/scripts/deploy_windows_hud.sh \
    --win-host tzehouse-windows.parrot-hen.ts.net \
    --full-app-exe target/x86_64-pc-windows-gnu/release/tze_hud.exe \
    --launch-mode auto \
    --tail
  ```

- [ ] Verify output messages
  - "Deploy complete."
  - "Remote exe: C:\tze_hud\tze_hud.exe"
  - No error messages in tail output

- [ ] Note remote log paths
  - Stdout: `C:\tze_hud\logs\hud.stdout.log`
  - Stderr: `C:\tze_hud\logs\hud.stderr.log`
  - Launcher: `C:\tze_hud\logs\hud.launcher.log`

### Step 2: MCP Reachability Gate (CRITICAL - DO NOT SKIP)

**This step must succeed BEFORE proceeding to publish.**

- [ ] Verify endpoint accepts HTTP connections
  ```bash
  curl -v http://tzehouse-windows.parrot-hen.ts.net:8765
  ```
  - Expected: Connection accepted (HTTP response)
  - Failure: "Connection refused" or "Connection timed out" = endpoint not live

- [ ] Test endpoint with MCP JSON-RPC (without auth first)
  ```bash
  curl -s -X POST http://tzehouse-windows.parrot-hen.ts.net:8765 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"list_resources","params":{},"id":1}' | jq .
  ```
  - Expected: HTTP response (may be 401 due to missing auth, that's OK)
  - Failure: Connection error = endpoint unreachable

- [ ] **STOP if endpoint unreachable**
  - Collect diagnostics (see Troubleshooting section)
  - Do NOT proceed to publish
  - Investigate and fix launch before continuing

### Step 3: Publish Zone Messages

**Only proceed after MCP Reachability Gate succeeds.**

- [ ] Create test message file
  ```bash
  cat > /tmp/hud-test-zones.json <<'EOF'
  [
    {
      "zone_name": "status-bar",
      "content": "Canonical app deployed and MCP live",
      "merge_key": "deploy-status",
      "namespace": "butler-test"
    }
  ]
  EOF
  ```

- [ ] List available zones (optional verification)
  ```bash
  python3 ./.claude/skills/user-test/scripts/publish_zone_batch.py \
    --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
    --psk-env MCP_TEST_PSK \
    --list-zones
  ```

- [ ] Publish zone messages
  ```bash
  python3 ./.claude/skills/user-test/scripts/publish_zone_batch.py \
    --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
    --psk-env MCP_TEST_PSK \
    --messages-file /tmp/hud-test-zones.json
  ```

- [ ] Verify publish success
  - Expected: "publish_to_zone" results with status 200 or "success"
  - Failure: 401/403 = authentication issue
  - Failure: "zone not found" = invalid zone name

## Post-Deployment Validation

- [ ] Record test results
  - Deployment timestamp
  - Artifact SHA256 checksum
  - MCP endpoint reachability confirmed
  - Zone publish results (success/failure, zone names, timestamps)

- [ ] Cleanup (if needed)
  - Stop runtime: `ssh hudbot@tzehouse-windows.parrot-hen.ts.net "powershell -Command 'Get-Process tze_hud -ErrorAction SilentlyContinue | Stop-Process -Force'"`
  - Or preserve for additional testing

## Troubleshooting Guide

### SSH Connection Fails

**Symptom**: "Permission denied (publickey)" or "Connection refused"

**Diagnosis**:
```bash
# Check key permissions
ls -la ~/.ssh/ecdsa_home

# Test connectivity with verbose output
ssh -vv -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net whoami
```

**Solutions**:
- Fix key permissions: `chmod 600 ~/.ssh/ecdsa_home`
- Verify SSH server running on Windows: `Get-Service sshd | Start-Service`
- Check firewall allows port 22
- Verify network connectivity: `ping tzehouse-windows.parrot-hen.ts.net`

### Deployment Script Fails

**Symptom**: Script exits with error during copy or launch phase

**Diagnosis**:
```bash
# Re-run with debug output
bash -x ./.claude/skills/user-test/scripts/deploy_windows_hud.sh \
  --win-host tzehouse-windows.parrot-hen.ts.net \
  --full-app-exe target/x86_64-pc-windows-gnu/release/tze_hud.exe
```

**Solutions**:
- Verify artifact exists: `ls -lh target/x86_64-pc-windows-gnu/release/tze_hud.exe`
- Verify SSH connectivity (see above)
- Check Windows disk space: `ssh hudbot@tzehouse-windows.parrot-hen.ts.net "powershell -Command 'Get-Volume C | Select SizeRemaining, Size'"`

### MCP Endpoint Unreachable

**Symptom**: `curl` to `http://tzehouse-windows.parrot-hen.ts.net:8765` fails with "Connection refused" or "Connection timed out"

**Diagnosis** (on Windows, via SSH):
```bash
# Check if process is running
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Get-Process tze_hud -ErrorAction SilentlyContinue | Select ProcessName, Id'"

# Check if port 8765 is listening
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "netstat -an | findstr :8765"

# Check runtime logs
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "type C:\\tze_hud\\logs\\hud.stdout.log"
```

**Solutions**:
- If process not running: Check launcher logs `C:\tze_hud\logs\hud.launcher.log`
- If port not listening: Check configuration file `C:\tze_hud\config.toml`
  - Verify `[network] enable_mcp_http = true`
  - Verify `[network] mcp_http_bind = "0.0.0.0:8765"`
- If Windows firewall blocks: Add inbound rule for port 8765
- If MCP PSK wrong: Verify `MCP_TEST_PSK` environment variable and config match

### MCP Publish Returns 401 (Unauthorized)

**Symptom**: `publish_to_zone` returns 401 error, publish rejected

**Diagnosis**:
```bash
# Verify PSK is set
echo "PSK is: $MCP_TEST_PSK"

# Check runtime logs for auth errors
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "type C:\\tze_hud\\logs\\hud.stdout.log | findstr -i auth"
```

**Solutions**:
- Set PSK environment variable: `export MCP_TEST_PSK="correct-shared-secret"`
- Verify PSK in Windows config matches: `ssh hudbot@tzehouse-windows.parrot-hen.ts.net "type C:\\tze_hud\\config.toml"`
- Verify PSK is passed correctly in curl/python requests (`-H "Authorization: Bearer $MCP_TEST_PSK"`)

### MCP Publish Returns "zone not found"

**Symptom**: Zone publish succeeds for HTTP but reports invalid zone name

**Diagnosis**:
```bash
# List valid zones
python3 ./.claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
  --psk-env MCP_TEST_PSK \
  --list-zones
```

**Solutions**:
- Use zone names from `--list-zones` output
- Verify zone names in test message file match exactly (case-sensitive)
- Common zones: `status-bar`, `notification-area` (confirm with --list-zones)

## Common Commands Quick Reference

```bash
# Build canonical app
cargo build --bin tze_hud --release --target x86_64-pc-windows-gnu

# Verify SSH connectivity
ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"

# Deploy and launch
./.claude/skills/user-test/scripts/deploy_windows_hud.sh \
  --full-app-exe target/x86_64-pc-windows-gnu/release/tze_hud.exe \
  --launch-mode auto --tail

# Test MCP endpoint
curl http://tzehouse-windows.parrot-hen.ts.net:8765

# List zones
python3 ./.claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
  --psk-env MCP_TEST_PSK --list-zones

# Publish zones
python3 ./.claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "http://tzehouse-windows.parrot-hen.ts.net:8765" \
  --psk-env MCP_TEST_PSK --messages-file /tmp/test-zones.json

# View logs (from Windows via SSH)
ssh hudbot@tzehouse-windows.parrot-hen.ts.net "type C:\\tze_hud\\logs\\hud.stdout.log"

# Stop process
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Get-Process tze_hud | Stop-Process -Force'"

# Check if process running
ssh hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -Command 'Get-Process tze_hud -ErrorAction SilentlyContinue'"
```

## Documentation References

- [README.md](../README.md) - Main build/test guide (includes deployment overview)
- [RUNTIME_APP_BINARY.md](RUNTIME_APP_BINARY.md) - Canonical app binary specification
- [DEPLOYMENT.md](DEPLOYMENT.md) - Detailed deployment automation guide
- `.claude/skills/user-test/SKILL.md` - User-test skill documentation
- `openspec/changes/ship-runtime-app-binary/` - OpenSpec change artifacts (requirements, design)
