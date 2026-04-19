# GStreamer Windows CI Bootstrap Spike

**Issued for**: `hud-rebhx`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-ora8.1 (v2-embodied-media-presence, precedes phase 1 Windows real-decode lane)
**Context**: Discovered from gstreamer audit (hud-ora8.1.18, PR #521)

---

## Verdict

**APPROACH: Self-Hosted D18 Runner — One-Time Manual Install (Primary); GitHub Actions MSI Cache (Secondary)**

For tze_hud's GPU runner box (D18), the correct approach is a **one-time manual GStreamer MSVC SDK install** on the self-hosted Windows runner, not a per-run MSI download. This mirrors the Linux runner pattern (system packages installed once, available to every run) and eliminates per-run download cost for a ~500 MB installer. The GitHub Actions ephemeral-runner path (caching the MSI) is documented as a fallback for non-D18 cloud Windows jobs.

The three concrete steps are:
1. Install GStreamer MSVC SDK 1.24.x on the D18 runner box once (manual or scripted).
2. Set `GSTREAMER_1_0_ROOT_MSVC_X86_64` and prepend GStreamer's `bin` to `PATH` as system-level environment variables.
3. For ephemeral runners: use the `blinemedical/setup-gstreamer` action with `actions/cache` to avoid re-downloading on every run.

---

## Scope

This spike covers the CI bootstrap requirements for building `gstreamer-rs` and running GStreamer-backed tests on Windows. The target is the D18 GPU runner box (self-hosted Windows runner, confirmed capable) that gates the phase 1 Windows real-decode lane.

Out of scope: GStreamer application distribution (bundling DLLs), MSYS2/MinGW Windows paths, GStreamer build-from-source, D3D11 plugin runtime configuration (separate validation item).

---

## 1. GStreamer for Windows: SDK Selection

### 1.1 MSVC vs. MinGW

The GStreamer project ships two binary SDK variants for Windows:

| Variant | Toolchain | Installer filename pattern | Suitable for Rust/MSVC? |
|---|---|---|---|
| **MSVC** | MSVC (cl.exe) | `gstreamer-1.0-msvc-x86_64-<version>.msi` | **Yes — required** |
| MinGW-w64 | GCC (MinGW) | `gstreamer-1.0-mingw-x86_64-<version>.msi` | No — ABI mismatch |

tze_hud uses the stable Rust MSVC toolchain (`x86_64-pc-windows-msvc`). MinGW-linked GStreamer is ABI-incompatible with MSVC-compiled Rust. **Always use the MSVC variant.**

### 1.2 Version Selection

The gstreamer-rs audit (hud-ora8.1.18) pins `gstreamer = "0.23"` (Rust crate), which targets the GStreamer 1.24 C ABI. GStreamer 1.24.x is required:

- **Minimum**: GStreamer 1.24.0 (for `d3d11h264dec`, `d3d11vp9dec`, and the full D3D11 hardware decode element set required by v2 phase 1)
- **Recommended**: Latest 1.24.x patch — **1.24.12** as of this writing (released 2024-12-09)

GStreamer 1.26.x is available (gstreamer-rs 0.24/0.25) but requires migrating the Rust crate pin. Stick with 1.24.x until the gstreamer-rs 0.24 migration bead is scheduled.

### 1.3 Official Installer URLs

The GStreamer project hosts binary installers at:

```
https://gstreamer.freedesktop.org/data/pkg/windows/<VERSION>/msvc/
```

Two installers are needed — **both are required**:

| Package | Filename | Purpose |
|---|---|---|
| Runtime | `gstreamer-1.0-msvc-x86_64-<VERSION>.msi` | DLLs, plugins, runtime files |
| Development | `gstreamer-1.0-devel-msvc-x86_64-<VERSION>.msi` | Headers, `.pc` files, import libs |

For GStreamer 1.24.12:
```
https://gstreamer.freedesktop.org/data/pkg/windows/1.24.12/msvc/gstreamer-1.0-msvc-x86_64-1.24.12.msi
https://gstreamer.freedesktop.org/data/pkg/windows/1.24.12/msvc/gstreamer-1.0-devel-msvc-x86_64-1.24.12.msi
```

**Installer size**: The complete MSVC runtime installer is approximately 450–600 MB; the devel package adds ~50–80 MB. Plan for ~600 MB total download. Compare this to Linux: `apt install libgstreamer1.0-dev gstreamer1.0-plugins-good gstreamer1.0-libav` typically downloads ~60–80 MB — a 7–8× difference.

### 1.4 Default Install Path

The MSVC SDK installs to:
```
C:\gstreamer\1.0\msvc_x86_64\
```
with subdirectories:
```
C:\gstreamer\1.0\msvc_x86_64\bin\          # DLLs, executables, pkg-config.exe
C:\gstreamer\1.0\msvc_x86_64\lib\          # Import libraries (.lib)
C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig\ # .pc files for pkg-config
C:\gstreamer\1.0\msvc_x86_64\include\      # C headers
```

---

## 2. Environment Variable Requirements

After installation, three environment variables must be set for `cargo build` to succeed:

| Variable | Value | Purpose |
|---|---|---|
| `GSTREAMER_1_0_ROOT_MSVC_X86_64` | `C:\gstreamer\1.0\msvc_x86_64` | gstreamer-rs `build.rs` uses this to locate the SDK |
| `PKG_CONFIG_PATH` | `C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig` | Points `pkg-config-rs` to the `.pc` files |
| `PATH` (prepend) | `C:\gstreamer\1.0\msvc_x86_64\bin` | Ensures GStreamer's `pkg-config.exe` is found first |

**Critical**: The `pkg-config.exe` bundled with GStreamer must be first in `PATH`. The MSYS2 `pkg-config` and `pkg-config-lite` from Chocolatey both have known incompatibilities with GStreamer's `.pc` files on Windows (they resolve paths incorrectly). Use only GStreamer's bundled `pkg-config.exe`.

**Build-time vs. runtime**: `PKG_CONFIG_PATH` and `GSTREAMER_1_0_ROOT_MSVC_X86_64` are build-time only. At runtime, the DLLs must be on `PATH` — the same `bin` prepend covers this.

---

## 3. Silent Install

The `.msi` installer format supports silent installation via `msiexec`:

```powershell
# Silent install (no UI, no prompts)
msiexec /i "gstreamer-1.0-msvc-x86_64-1.24.12.msi" /qn INSTALLDIR="C:\gstreamer\1.0\msvc_x86_64"
msiexec /i "gstreamer-1.0-devel-msvc-x86_64-1.24.12.msi" /qn INSTALLDIR="C:\gstreamer\1.0\msvc_x86_64"
```

Flags:
- `/i` — install mode
- `/qn` — quiet, no UI (use `/passive` to show a progress bar)
- `INSTALLDIR=` — override the default install location (optional if default `C:\gstreamer\` is acceptable)

**Important caveats**:
- `msiexec` is asynchronous by default; add `/norestart /wait` or use `Start-Process -Wait` in PowerShell to block until installation completes before the next step.
- The complete install (all plugins) requires no additional flags — the "Complete" feature set is the default. Do not use `/qb-!` (hides progress bar) if the CI log needs install confirmation.
- Both installers must complete before `cargo build`. The devel package depends on the runtime package being installed first.

PowerShell pattern with error checking:
```powershell
$ErrorActionPreference = "Stop"
$gst_version = "1.24.12"
$install_dir = "C:\gstreamer\1.0\msvc_x86_64"

foreach ($pkg in @("", "-devel")) {
    $msi = "gstreamer-1.0${pkg}-msvc-x86_64-${gst_version}.msi"
    $url = "https://gstreamer.freedesktop.org/data/pkg/windows/${gst_version}/msvc/${msi}"
    Invoke-WebRequest -Uri $url -OutFile $msi
    Start-Process msiexec -ArgumentList "/i", $msi, "/qn", "INSTALLDIR=`"${install_dir}`"" -Wait -NoNewWindow
    Remove-Item $msi  # free disk space after install
}
```

---

## 4. CI Integration Patterns

### 4.1 Self-Hosted D18 GPU Runner (Primary — Recommended)

The D18 GPU runner box is a persistent Windows machine. The correct pattern is a **one-time installation** performed at machine provisioning time, not per-run:

**One-time setup on the D18 box** (run once by the operator, not part of the CI job):
```powershell
# Run as Administrator during D18 provisioning:
$gst_version = "1.24.12"
$base_url = "https://gstreamer.freedesktop.org/data/pkg/windows/${gst_version}/msvc"

Invoke-WebRequest "${base_url}/gstreamer-1.0-msvc-x86_64-${gst_version}.msi" -OutFile "gst-runtime.msi"
Invoke-WebRequest "${base_url}/gstreamer-1.0-devel-msvc-x86_64-${gst_version}.msi" -OutFile "gst-devel.msi"
Start-Process msiexec -ArgumentList "/i gst-runtime.msi /qn" -Wait -NoNewWindow
Start-Process msiexec -ArgumentList "/i gst-devel.msi /qn" -Wait -NoNewWindow

# Set system-level environment variables (persists across reboots and runner jobs)
[System.Environment]::SetEnvironmentVariable(
    "GSTREAMER_1_0_ROOT_MSVC_X86_64",
    "C:\gstreamer\1.0\msvc_x86_64",
    [System.EnvironmentVariableTarget]::Machine
)
[System.Environment]::SetEnvironmentVariable(
    "PKG_CONFIG_PATH",
    "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig",
    [System.EnvironmentVariableTarget]::Machine
)

# Prepend GStreamer bin to system PATH
$current_path = [System.Environment]::GetEnvironmentVariable("PATH", "Machine")
[System.Environment]::SetEnvironmentVariable(
    "PATH",
    "C:\gstreamer\1.0\msvc_x86_64\bin;${current_path}",
    [System.EnvironmentVariableTarget]::Machine
)
```

The GitHub Actions runner service reads machine-level environment variables on each job start — no per-job setup step is needed once this provisioning script has run.

**D18 operator checklist**:
- [ ] Windows 10 or 11 (64-bit)
- [ ] Visual Studio Build Tools 2019+ or MSVC toolchain installed
- [ ] Rust `x86_64-pc-windows-msvc` stable toolchain installed
- [ ] GStreamer 1.24.12 MSVC runtime + devel installed
- [ ] `GSTREAMER_1_0_ROOT_MSVC_X86_64`, `PKG_CONFIG_PATH`, `PATH` set at Machine scope
- [ ] Runner registered with `windows` and `gpu` labels for the real-decode lane
- [ ] Smoke-test: `cargo check -p tze_hud_compositor` passes after environment setup

### 4.2 GitHub Actions Ephemeral Runners (Secondary — Cloud/PR Jobs)

For PRs that require Windows compilation checks on `windows-latest` (Microsoft-hosted, ephemeral), the MSI must be installed each run. Use caching to avoid re-downloading the ~600 MB installer on every job.

**Recommended: `blinemedical/setup-gstreamer` action**

The `blinemedical/setup-gstreamer` community action automates Windows MSI download, install, and environment variable setup:

```yaml
# .github/workflows/ci.yml (excerpt — Windows build job)
jobs:
  build-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - name: Set up Rust (MSVC)
        uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
          targets: x86_64-pc-windows-msvc

      - name: Set up GStreamer
        uses: blinemedical/setup-gstreamer@v1
        with:
          version: "1.24.12"
          arch: x86_64
        # Sets GSTREAMER_1_0_ROOT_MSVC_X86_64 and appends bin to GITHUB_PATH

      - name: Set PKG_CONFIG_PATH
        shell: pwsh
        run: |
          $gst_root = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
          echo "PKG_CONFIG_PATH=${gst_root}\lib\pkgconfig" >> $env:GITHUB_ENV
          # Ensure GStreamer pkg-config.exe is first in PATH
          echo "${gst_root}\bin" >> $env:GITHUB_PATH

      - name: Build
        shell: pwsh
        run: cargo build --workspace
```

The `setup-gstreamer` action constructs installer URLs using the canonical pattern:
```
https://gstreamer.freedesktop.org/data/pkg/windows/<version>/msvc/gstreamer-1.0-msvc-x86_64-<version>.msi
```

It downloads both runtime and devel packages, installs them silently via `msiexec`, and sets `GSTREAMER_1_0_ROOT_MSVC_X86_64` in the GitHub environment.

**Manual approach without the community action** (lower dependency risk):

```yaml
      - name: Cache GStreamer installer
        id: cache-gst
        uses: actions/cache@v4
        with:
          path: C:\gstreamer-installers
          key: gstreamer-1.24.12-msvc-x86_64

      - name: Download GStreamer installers
        if: steps.cache-gst.outputs.cache-hit != 'true'
        shell: pwsh
        run: |
          New-Item -ItemType Directory -Force -Path "C:\gstreamer-installers"
          $v = "1.24.12"
          $base = "https://gstreamer.freedesktop.org/data/pkg/windows/${v}/msvc"
          Invoke-WebRequest "${base}/gstreamer-1.0-msvc-x86_64-${v}.msi" `
            -OutFile "C:\gstreamer-installers\gst-runtime.msi"
          Invoke-WebRequest "${base}/gstreamer-1.0-devel-msvc-x86_64-${v}.msi" `
            -OutFile "C:\gstreamer-installers\gst-devel.msi"

      - name: Install GStreamer
        shell: pwsh
        run: |
          Start-Process msiexec -ArgumentList "/i C:\gstreamer-installers\gst-runtime.msi /qn" -Wait -NoNewWindow
          Start-Process msiexec -ArgumentList "/i C:\gstreamer-installers\gst-devel.msi /qn" -Wait -NoNewWindow

      - name: Set GStreamer environment variables
        shell: pwsh
        run: |
          $gst_root = "C:\gstreamer\1.0\msvc_x86_64"
          echo "GSTREAMER_1_0_ROOT_MSVC_X86_64=${gst_root}" >> $env:GITHUB_ENV
          echo "PKG_CONFIG_PATH=${gst_root}\lib\pkgconfig" >> $env:GITHUB_ENV
          echo "${gst_root}\bin" >> $env:GITHUB_PATH
```

**Caching trade-off**: GitHub Actions cache is keyed by the GStreamer version string. The ~600 MB MSI cache hit saves ~2–4 minutes per run on `windows-latest` (where download speed is 50–100 MB/s). Cache misses on version bumps are acceptable — they are infrequent.

**Cache storage cost**: The MSI cache counts against the 10 GB Actions cache limit per repository. At ~600 MB per version, this is manageable (1 version = 6% of the cache quota).

---

## 5. gstreamer-rs Cargo Build on Windows

### 5.1 How `cargo build` Finds GStreamer

The `gstreamer` crate's `build.rs` uses `pkg-config-rs` to discover GStreamer headers and libraries. On Windows, `pkg-config-rs` reads from:

1. `PKG_CONFIG_PATH` environment variable → must point to `C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig`
2. `GSTREAMER_1_0_ROOT_MSVC_X86_64` environment variable → used by gstreamer-rs's own build script as a fallback

If either is missing or points to the wrong `pkg-config.exe`, `cargo build` fails with errors like:
```
pkg-config: error: package 'gstreamer-1.0' not found in C:\...
```
or linker errors like:
```
error: linking with `link.exe` failed: exit code: 1120
  ... unresolved external symbol gst_init ...
```

### 5.2 Path Ordering — Critical

Multiple `pkg-config.exe` binaries compete on Windows CI runners:
- `C:\ProgramData\chocolatey\bin\pkg-config.exe` (Chocolatey — present on `windows-latest`)
- `C:\msys64\usr\bin\pkg-config.exe` (MSYS2 — present on `windows-latest`)
- `C:\gstreamer\1.0\msvc_x86_64\bin\pkg-config.exe` (GStreamer — what we need)

GStreamer's `pkg-config.exe` must come first in `PATH`. Other implementations resolve Windows-style paths differently and cause build failures. The steps in §4 prepend the GStreamer `bin` dir to both `GITHUB_PATH` and `PATH`.

### 5.3 Cargo.toml — No Windows-Specific Changes Required

No `[target.'cfg(target_os = "windows")']` overrides are needed in `Cargo.toml` for gstreamer-rs 0.23. The build script detects GStreamer via `pkg-config` transparently. Feature flags are runtime-probed, not compile-time:

```toml
[dependencies]
gstreamer = "0.23"
gstreamer-app = "0.23"
gstreamer-video = "0.23"
gstreamer-audio = "0.23"
gstreamer-pbutils = "0.23"
```

This is unchanged from the Linux/macOS configuration.

### 5.4 `--target` Consideration

If the CI job cross-compiles (e.g., `cargo build --target x86_64-pc-windows-msvc` from within a MSYS2 shell), set `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=link.exe` explicitly. When building natively in a VS Developer Command Prompt or from `windows-latest`, this is unnecessary.

---

## 6. D3D11 Hardware Decode Considerations

The GStreamer MSVC "Complete" installation includes the `d3d11` plugin set (`d3d11h264dec`, `d3d11vp9dec`, `d3d11videosink`). These are available in 1.24.x without additional setup. However:

- **No extra install steps required** for the plugins themselves — they are bundled in the complete installer.
- **Driver dependency**: D3D11 hardware decode requires a GPU-capable D3D11 driver. On the D18 GPU runner box, the GPU driver must be current (WDDM 2.0+, Windows 10 1607+). This is a runtime check, not a CI bootstrap step.
- **Probe at runtime, not compile time**: As documented in the gstreamer audit (hud-ora8.1.18 §8.1), use `gst::ElementFactory::find("d3d11h264dec")` at runtime to verify availability; fall back to `avdec_h264` (software) if not found. Do not fail the build for missing D3D11 elements.
- **NVDEC on the D18 box**: If the D18 GPU is NVIDIA, `nvh264dec` and `nvvp9dec` are also available via the MSVC complete install, but require an up-to-date NVIDIA driver with NVDEC support (GeForce GTX 900+, driver ≥ 397.93). Same runtime probe pattern applies.

---

## 7. Linux CI Cost Comparison

| Aspect | Linux (Ubuntu 24.04) | Windows (MSVC SDK) |
|---|---|---|
| Install mechanism | `apt install` (system package manager) | MSI installer download + `msiexec` |
| Download size | ~60–80 MB total | ~550–650 MB total |
| Install time (CI) | ~30–60 seconds | ~3–5 minutes (cold) / ~30–60 seconds (cached MSI) |
| Environment setup | None — `pkg-config` auto-finds headers | Must set `GSTREAMER_1_0_ROOT_MSVC_X86_64`, `PKG_CONFIG_PATH`, `PATH` |
| Version pinning | Ubuntu 24.04 ships GStreamer 1.24 natively | Must pin version in download URL |
| Cache strategy | Docker layer (implicit) | `actions/cache` on MSI files (explicit) |
| pkg-config issues | None on Ubuntu 24.04 | Must use GStreamer's bundled `pkg-config.exe` first |
| Hardware decode | `gstreamer1.0-vaapi` or `gstreamer1.0-plugins-bad` | Bundled in complete MSI; requires GPU driver |

**Bottom line**: Linux CI is ~5× faster to set up and ~8× smaller to download. Windows CI is absolutely viable but requires deliberate bootstrapping, especially `PATH` ordering for `pkg-config.exe`.

---

## 8. Recommendation

| Scenario | Recommended approach |
|---|---|
| D18 GPU runner box (phase 1 gate) | One-time manual install + system env vars. No per-job download overhead. |
| `windows-latest` CI (compilation check on PRs) | `blinemedical/setup-gstreamer@v1` action + `actions/cache` on MSI files. |
| Both needed | Implement D18 first (phase 1 gate); add cloud Windows job once D18 bootstrap is confirmed. |

**Phase 1 activation sequence for D18**:

1. Operator provisions D18 box: install GStreamer 1.24.12 MSVC SDK, set env vars (§4.1 checklist).
2. Add D18 to GitHub Actions self-hosted runner pool with labels `[self-hosted, windows, gpu]`.
3. Add `.github/workflows/real-decode-windows.yml` job gated on `run-real-decode` label, targeting `[self-hosted, windows, gpu]`.
4. Smoke-test CI job: `cargo check -p tze_hud_compositor` plus `gst_inspect_1.0 --version`.
5. Activate real-decode lane: run D18 media budget validation per D18/D20 requirements.

---

## 9. Caveats Summary

1. **Installer size is non-trivial** (~600 MB MSI). On ephemeral runners, cache the MSI to avoid per-run download. On D18, install once.
2. **`pkg-config.exe` ordering** is the most common source of build failures on Windows. GStreamer's bundled binary must be first in `PATH`.
3. **Both runtime and devel MSIs are required** for `cargo build`. The devel package contains the `.pc` files and import libraries that `cargo build` needs; the runtime package provides the DLLs loaded at runtime.
4. **D3D11 hardware decode** is bundled in the complete install but requires a compatible GPU driver at runtime. Probe at runtime; fall back to software decode.
5. **gstreamer-rs 0.23 ↔ GStreamer 1.24 ABI lock**: The `gstreamer = "0.23"` Cargo pin requires exactly GStreamer 1.24.x. Installing 1.22.x or 1.26.x will cause `cargo build` failures or runtime ABI mismatches. The CI must pin the version string explicitly.
6. **`msiexec` is synchronous only with `Start-Process -Wait`**. Omitting `-Wait` causes the subsequent `cargo build` to start before installation completes, yielding "not found" errors with no obvious cause.
7. **Complete installer vs. custom feature set**: The complete GStreamer MSVC install includes all plugins (~600 MB installed). A minimal feature set install (runtime + plugins-good + plugins-bad + libav) would reduce this to ~150–200 MB but requires custom feature selection in the MSI wizard, which is not easily scriptable. For CI, use the complete install unless disk space is constrained.

---

## Sources

- GStreamer download page: https://gstreamer.freedesktop.org/download/
- GStreamer Windows installation guide: https://gstreamer.freedesktop.org/documentation/installing/on-windows.html
- GStreamer Windows deployment guide: https://gstreamer.freedesktop.org/documentation/deploying/windows.html
- GStreamer Windows binary package index (1.24.1 example): `https://gstreamer.freedesktop.org/data/pkg/windows/1.24.1/msvc/`
- gstreamer-rs README (Windows build instructions): https://github.com/GStreamer/gstreamer-rs/blob/main/README.md
- gstreamer-rs GitHub (canonical Rust bindings): https://github.com/GStreamer/gstreamer-rs
- gstreamer-rs GitLab CI (Windows Docker image and test execution): https://gitlab.freedesktop.org/gstreamer/gstreamer-rs/-/blob/main/.gitlab-ci.yml
- `blinemedical/setup-gstreamer` GitHub Action: https://github.com/blinemedical/setup-gstreamer
- GitHub Actions `actions/cache` documentation: https://github.com/actions/cache
- pkg-config-rs issue #51 (PKG_CONFIG_PATH on Windows): https://github.com/rust-lang/pkg-config-rs/issues/51
- gstreamer-rs issue #64 (Windows compilation): https://github.com/sdroege/gstreamer-rs/issues/64
- gstreamer-rs audit (hud-ora8.1.18): `docs/audits/gstreamer-media-pipeline-audit.md`
- v2 signoff packet (D18, D19, D20): `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 procurement list: `openspec/changes/v2-embodied-media-presence/procurement.md`
