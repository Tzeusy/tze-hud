# Windows D18 Runner Setup — tzehouse-windows RTX 3080

**Issue**: `hud-ora8.1.20`
**Date**: 2026-04-25
**Author**: agent worker (claude-sonnet-4-6)
**Companion workflow**: `.github/workflows/real-decode-windows.yml`
**Audit source**: `docs/audits/gstreamer-windows-ci-bootstrap.md` (§4.1, §5)

---

## Purpose

This runbook covers the one-time setup of the tzehouse-windows box as a D18
self-hosted GitHub Actions GPU runner for the real-decode CI lane. It is written
for **Tzeusy (human operator)** — some steps require interactive Windows access
and cannot be performed by an agent.

Hardware confirmed (2026-04-25):

| Item | Value |
|---|---|
| GPU | NVIDIA GeForce RTX 3080 (10 GB) |
| Driver | 595.97 |
| CUDA | 13.2 |
| Network | Tailnet (`parrot-hen.ts.net`) |
| OS | Windows (see §2) |

This hardware **exceeds** the original RTX 4060-class procurement bar for the
D18 role. No additional hardware is needed.

---

## 1. Context and Scope

### 1.1 What is D18?

D18 is the phase 1 GPU runner node for the tze_hud real-decode lane. Its role is:

- Run nightly `cargo test` jobs that exercise the GStreamer D3D11/NVDEC hardware
  decode path against real media inputs.
- Gate the v2 embodied-media-presence phase 1 activation sequence (see audit §8).
- Report decode correctness and — eventually — throughput / timing metrics.

The glass-to-glass latency capture rig (D18 p50<=150ms / p99<=400ms) is tracked
in `hud-ora8.1.28` and requires a separate external capture setup. This runbook
does NOT cover that.

### 1.2 What this runbook covers

1. OS and toolchain prerequisites (§2)
2. GStreamer 1.24.12 MSVC SDK installation (§3) — **requires human action**
3. Environment variable setup (§4) — **requires human action**
4. GitHub Actions self-hosted runner registration (§5) — **requires human action**
5. Smoke-test verification (§6)
6. Dual-use scheduling caveat (§7)

### 1.3 What the agent has already done

- Added `.github/workflows/real-decode-windows.yml` to the repository (this PR).
  The workflow is ready to run once the D18 runner is provisioned and registered.
- Documented this runbook.

---

## 2. OS and Toolchain Prerequisites

Verify the following are present on tzehouse-windows before proceeding:

| Prerequisite | Minimum version | Notes |
|---|---|---|
| Windows | Windows 10 (64-bit, 1607+) or Windows 11 | D3D11 hardware decode requires WDDM 2.0+ |
| Visual Studio Build Tools | 2019 or later | Provides `link.exe`, MSVC runtime, Windows SDK |
| Rust | stable `x86_64-pc-windows-msvc` | Install via `rustup` |
| Git | Any recent version | For `actions/checkout` |
| GitHub Actions runner | Latest (from GitHub) | Downloaded in §5 |

To verify Rust is installed correctly:

```powershell
rustup show
# Expected: stable-x86_64-pc-windows-msvc (default)
rustc --version
# Expected: rustc 1.xx.x (YYYY-MM-DD)
```

If Visual Studio Build Tools are not installed:
```powershell
# Download the Build Tools installer from:
# https://visualstudio.microsoft.com/visual-cpp-build-tools/
# Select: "C++ build tools" workload
# Required components: MSVC v143+, Windows 11 SDK
```

---

## 3. GStreamer 1.24.12 MSVC SDK Installation

**This section requires Administrator access on tzehouse-windows.**

The GStreamer SDK must be installed **once** at provisioning time. It is not
downloaded per CI run. See audit §4.1 for rationale.

### 3.1 Download both installers

Open a PowerShell session as Administrator:

```powershell
$ErrorActionPreference = "Stop"
$gst_version = "1.24.12"
$base_url = "https://gstreamer.freedesktop.org/data/pkg/windows/${gst_version}/msvc"

# Create a temporary download directory
New-Item -ItemType Directory -Force -Path "C:\gst-install-tmp"
Set-Location "C:\gst-install-tmp"

# Download runtime installer (~450-550 MB)
Write-Host "Downloading GStreamer runtime..."
Invoke-WebRequest "${base_url}/gstreamer-1.0-msvc-x86_64-${gst_version}.msi" `
    -OutFile "gst-runtime.msi"

# Download devel installer (~50-80 MB)
Write-Host "Downloading GStreamer devel..."
Invoke-WebRequest "${base_url}/gstreamer-1.0-devel-msvc-x86_64-${gst_version}.msi" `
    -OutFile "gst-devel.msi"
```

**Why two installers?**
- `gstreamer-1.0-msvc-x86_64-1.24.12.msi` — Runtime DLLs and plugins (required at build AND run time)
- `gstreamer-1.0-devel-msvc-x86_64-1.24.12.msi` — Headers, `.pc` files, import libs (required by `cargo build`)

Both are required. The devel package depends on the runtime package being installed first.

### 3.2 Install both packages

```powershell
# Install runtime first (devel depends on it)
Write-Host "Installing GStreamer runtime (this takes 3-5 minutes)..."
Start-Process msiexec -ArgumentList "/i", "gst-runtime.msi", "/qn" -Wait -NoNewWindow
if ($LASTEXITCODE -ne 0) {
    Write-Error "GStreamer runtime install failed with exit code $LASTEXITCODE"
    exit 1
}
Write-Host "Runtime installed."

# Install devel package
Write-Host "Installing GStreamer devel (this takes 1-2 minutes)..."
Start-Process msiexec -ArgumentList "/i", "gst-devel.msi", "/qn" -Wait -NoNewWindow
if ($LASTEXITCODE -ne 0) {
    Write-Error "GStreamer devel install failed with exit code $LASTEXITCODE"
    exit 1
}
Write-Host "Devel installed."

# Clean up installers
Remove-Item -Rf "C:\gst-install-tmp"
Write-Host "Installation complete."
```

**Important**: `Start-Process ... -Wait` is required. Without `-Wait`, `msiexec`
returns immediately and `cargo build` will start before installation finishes,
producing confusing "not found" errors. See audit §9 caveat 6.

Default install path: `C:\gstreamer\1.0\msvc_x86_64\`

### 3.3 Verify installation

```powershell
# Confirm files exist
Test-Path "C:\gstreamer\1.0\msvc_x86_64\bin\gst-inspect-1.0.exe"  # must be True
Test-Path "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig\gstreamer-1.0.pc"  # must be True

# Quick version check (after env vars are set in §4)
& "C:\gstreamer\1.0\msvc_x86_64\bin\gst-inspect-1.0.exe" --version
```

---

## 4. Environment Variable Setup

**This section requires Administrator access on tzehouse-windows.**

Three system-level (Machine scope) environment variables must be set. Machine-scope
variables persist across reboots and are inherited by the GitHub Actions runner
service on each job start — no per-job setup step is needed once this is done.

```powershell
# Run as Administrator
$gst_root = "C:\gstreamer\1.0\msvc_x86_64"

# 1. SDK root — used by gstreamer-rs build.rs as a fallback discovery path
[System.Environment]::SetEnvironmentVariable(
    "GSTREAMER_1_0_ROOT_MSVC_X86_64",
    $gst_root,
    [System.EnvironmentVariableTarget]::Machine
)

# 2. pkg-config path — points pkg-config-rs to GStreamer's .pc files
[System.Environment]::SetEnvironmentVariable(
    "PKG_CONFIG_PATH",
    "${gst_root}\lib\pkgconfig",
    [System.EnvironmentVariableTarget]::Machine
)

# 3. Prepend GStreamer bin to system PATH
#    CRITICAL: GStreamer's pkg-config.exe must come BEFORE Chocolatey and MSYS2 pkg-config.
#    See audit §5.2 for why path ordering matters.
$current_path = [System.Environment]::GetEnvironmentVariable("PATH", "Machine")
[System.Environment]::SetEnvironmentVariable(
    "PATH",
    "${gst_root}\bin;${current_path}",
    [System.EnvironmentVariableTarget]::Machine
)

Write-Host "Environment variables set. Restart the GitHub Actions runner service to pick them up."
```

### 4.1 Why path ordering matters

Multiple `pkg-config.exe` binaries may exist on the system:
- `C:\ProgramData\chocolatey\bin\pkg-config.exe` (Chocolatey)
- `C:\msys64\usr\bin\pkg-config.exe` (MSYS2)
- `C:\gstreamer\1.0\msvc_x86_64\bin\pkg-config.exe` (**this is what we need**)

Only GStreamer's bundled `pkg-config.exe` correctly resolves Windows-style paths
in `.pc` files. Other implementations cause `cargo build` failures. The GStreamer
`bin` directory must appear first in `PATH`. See audit §5.2 for details.

### 4.2 Restart the runner service

After setting environment variables, restart the Actions runner service so it
inherits the new Machine-scope variables:

```powershell
# Restart the GitHub Actions runner service (service name may vary; check Services)
Restart-Service -Name "actions.runner.*" -Force
# OR if not registered yet, the runner will pick up the env vars on first start.
```

---

## 5. GitHub Actions Self-Hosted Runner Registration

**This section requires a GitHub repository admin token and interactive access on tzehouse-windows.**

**HUMAN ACTION REQUIRED**: The registration command includes a `--token` that is
issued by GitHub for the repository and expires after a short window. An agent
cannot obtain this token or execute the registration interactively. **Tzeusy must
run the steps in this section on the Windows box.**

### 5.1 Download the runner

1. Go to the repository on GitHub: `https://github.com/<owner>/tze_hud`
2. Navigate to **Settings → Actions → Runners → New self-hosted runner**
3. Select **Windows** and **x64**
4. Follow the displayed download and configuration commands

Typical commands (exact version may differ — use what GitHub shows):

```powershell
# Create a directory for the runner
mkdir C:\actions-runner ; Set-Location C:\actions-runner

# Download the runner package (URL from GitHub Settings page)
Invoke-WebRequest -Uri "https://github.com/actions/runner/releases/download/v2.x.x/actions-runner-win-x64-2.x.x.zip" `
    -OutFile "actions-runner-win-x64.zip"
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::ExtractToDirectory("$PWD\actions-runner-win-x64.zip", "$PWD")
```

### 5.2 Configure the runner with the correct labels

**Critical**: The runner must be registered with labels `self-hosted,windows,gpu`.
The workflow `.github/workflows/real-decode-windows.yml` targets `[self-hosted, windows, gpu]`.

```powershell
# Run the configuration script (token from GitHub Settings → Runners → New runner)
# --labels MUST include self-hosted,windows,gpu for the workflow to pick up this runner
.\config.cmd `
    --url "https://github.com/<owner>/tze_hud" `
    --token "<TOKEN_FROM_GITHUB_SETTINGS>" `
    --name "tzehouse-windows-d18" `
    --labels "self-hosted,windows,gpu" `
    --work "_work" `
    --runasservice
```

### 5.3 Install as a Windows service

The `--runasservice` flag in §5.2 installs the runner as a Windows service so it
starts automatically on reboot. If you omit it, add the service separately:

```powershell
.\svc.cmd install
.\svc.cmd start
```

### 5.4 Verify registration

In GitHub: **Settings → Actions → Runners** should show `tzehouse-windows-d18`
with status **Idle** and labels `self-hosted`, `windows`, `gpu`.

---

## 6. Smoke-Test Verification

After completing §3–§5, run the smoke-test to confirm the runner and GStreamer
SDK are wired correctly before enabling nightly runs.

### 6.1 Manual dispatch smoke-test

1. In GitHub: **Actions → Windows D18 Real-Decode → Run workflow**
2. Set `decode_target` to `smoke`
3. Watch the run — it should:
   - Pass **Verify GStreamer SDK environment** (env vars present, pkg-config resolves gstreamer-1.0)
   - Pass **Report GStreamer GPU capabilities** (gst-inspect reports d3d11h264dec, nvh264dec)
   - Pass **cargo check** (GStreamer SDK linkage confirmed)
   - Exit 0 on the placeholder step

### 6.2 Expected gst-inspect output

```
NVIDIA GeForce RTX 3080 — expected elements:
  d3d11h264dec   — D3D11 H.264 hardware decoder
  d3d11vp9dec    — D3D11 VP9 hardware decoder
  nvh264dec      — NVDEC H.264 decoder (requires CUDA driver >= 397.93; D18 has 595.97)
  nvvp9dec       — NVDEC VP9 decoder
```

If any element is missing:
- `d3d11*` missing: D3D11 plugin not in GStreamer complete install — reinstall with
  the "Complete" feature set (not a minimal install). See audit §9 caveat 7.
- `nv*` missing: NVIDIA Video Codec SDK (NVDEC) not detected. Verify driver version
  with `nvidia-smi`. D18 driver 595.97 should include NVDEC support.

### 6.3 D18 operator checklist

- [ ] Windows 10 or 11 (64-bit)
- [ ] Visual Studio Build Tools 2019+ installed
- [ ] Rust `x86_64-pc-windows-msvc` stable toolchain installed
- [ ] GStreamer 1.24.12 MSVC runtime + devel installed (§3)
- [ ] `GSTREAMER_1_0_ROOT_MSVC_X86_64` set at Machine scope (§4)
- [ ] `PKG_CONFIG_PATH` set at Machine scope (§4)
- [ ] GStreamer `bin` prepended to system `PATH` (§4)
- [ ] GitHub Actions runner registered with labels `[self-hosted, windows, gpu]` (§5)
- [ ] Runner shows **Idle** in GitHub Settings → Actions → Runners
- [ ] Smoke-test workflow dispatch passes all steps (§6.1)
- [ ] `gst-inspect-1.0 d3d11h264dec` returns element info (not "not found") (§6.2)

---

## 7. Dual-Use Scheduling Caveat

**SCHEDULING GAP — NOT YET RESOLVED**

The tzehouse-windows box is used for two distinct workloads:

| Workload | Trigger | GPU usage |
|---|---|---|
| `/user-test` interactive overlay sessions | Human-initiated, daytime SGT | Active (wgpu compositor, tze_hud.exe) |
| Real-decode CI jobs (this workflow) | Nightly at 02:00 SGT | Active (GStreamer D3D11/NVDEC) |

**The problem**: Both workloads use the GPU. If a CI job runs while an interactive
HUD session is active, they may race for GPU resources, causing:
- CI job failures with GPU/DXGI access errors
- HUD session stuttering or crashes
- Corrupted decode output in tests

**Current mitigation**: The nightly schedule is set to 02:00 SGT (18:00 UTC),
which reduces overlap with typical daytime interactive use. This is a best-effort
window, **not a hard exclusive lock**.

**Open gap**: A proper mutual-exclusion mechanism is needed. Options include:
- A lock file checked by both the CI job and `/user-test` startup
- A GitHub Actions runner label rotation (remove `gpu` label when session is active)
- A custom pre-job script that checks for active HUD sessions before running decode tests

**Action item**: File a follow-up bead to implement the scheduling policy.
The gap is documented here as `OPEN — scheduling policy bead needed`.

In the meantime: **if you are running an interactive `/user-test` session during
the 02:00 SGT window, manually cancel the nightly workflow run** from GitHub
Actions to avoid conflicts.

---

## 8. GStreamer SDK Version Upgrade Notes

The workflow and this runbook pin to GStreamer **1.24.12** because:

- `gstreamer = "0.23"` (Rust crate) targets the GStreamer 1.24 C ABI
- GStreamer 1.24.0+ includes `d3d11h264dec`, `d3d11vp9dec`, and full D3D11 decode
- GStreamer 1.26.x requires migrating to `gstreamer-rs 0.24/0.25` (separate bead)

When upgrading GStreamer, update **both**:
1. The SDK installed on D18 (§3)
2. The `gst_version` default in `.github/workflows/real-decode-windows.yml`

Do not upgrade GStreamer without a matching `gstreamer-rs` crate pin update.
See audit §9 caveat 5 (gstreamer-rs 0.23 ↔ GStreamer 1.24 ABI lock).

---

## 9. SSH Access Notes

If you need to verify or troubleshoot D18 remotely via SSH:

```bash
# Public key auth for user tzeus (or hudbot)
# Must use explicit identity: -i ~/.ssh/ecdsa_home
ssh -i ~/.ssh/ecdsa_home tzeus@parrot-hen.ts.net

# SCP with explicit identity
scp -i ~/.ssh/ecdsa_home <file> tzeus@parrot-hen.ts.net:C:/path/
```

Default SSH identity resolution fails for this host. Always pass `-i ~/.ssh/ecdsa_home`
explicitly. See AGENTS.md notes on Windows SSH pubkey auth.

---

## References

- Companion workflow: `.github/workflows/real-decode-windows.yml`
- Audit (primary source): `docs/audits/gstreamer-windows-ci-bootstrap.md`
- Bead: `hud-ora8.1.20`
- Glass-to-glass latency harness (separate scope): `hud-ora8.1.28`
- GStreamer Windows install guide: https://gstreamer.freedesktop.org/documentation/installing/on-windows.html
- GitHub Actions self-hosted runner docs: https://docs.github.com/en/actions/hosting-your-own-runners
