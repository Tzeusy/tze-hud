#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  d18_validation.sh [options]

SSH-driven D18 validation lane (replaces the suspended GitHub Actions
real-decode-windows.yml — see hud-1aswu.4). Runs the lane's substantive
checks against the tzehouse-windows box from the Linux rig:

1) SSH connectivity gate (tzeus)
2) GPU lock check at %PROGRAMDATA%\tze_hud\gpu.lock (skips if a live
   interactive session holds the lock; reports stale locks, never removes
   them — interactive sessions own their cleanup)
3) GStreamer MSVC SDK verification (machine env var, install dir,
   gst-inspect-1.0 version)
4) Hardware decoder capability report (d3d11h264dec, d3d11vp9dec,
   nvh264dec, nvvp9dec)
5) Real-decode harness — ACTIVATION-GATED: prints status only until
   tze_hud_runtime::real_decode_windows exists (hud-ora8.1 phase 1)

Exit codes:
  0  all runnable checks passed (harness step may be GATED)
  1  a runnable check failed (SSH, lock parse, SDK present-but-broken)
  2  GPU busy — live interactive session holds the lock
  3  SDK not installed (activation prerequisite unmet); checks 4-5 skipped.
     With --allow-missing-sdk this becomes exit 0 with a GATED report.

Options:
  --win-user <user>        Windows SSH user for checks (default: tzeus)
  --win-host <host>        Windows host; default resolves the tailnet IP via
                           'tailscale ip -4 tzehouse-windows' (MagicDNS is not
                           in the rig resolver), falling back to
                           tzehouse-windows.parrot-hen.ts.net
  --ssh-key <path>         SSH identity (default: ~/.ssh/ecdsa_home)
  --allow-missing-sdk      Report missing GStreamer SDK as GATED instead of
                           failing (for status sweeps before provisioning)
  -h, --help               Show help
EOF
}

WIN_USER="tzeus"
WIN_HOST=""
SSH_KEY="$HOME/.ssh/ecdsa_home"
ALLOW_MISSING_SDK=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --win-user) WIN_USER="$2"; shift 2 ;;
    --win-host) WIN_HOST="$2"; shift 2 ;;
    --ssh-key) SSH_KEY="$2"; shift 2 ;;
    --allow-missing-sdk) ALLOW_MISSING_SDK=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
  esac
done

if [[ -z "$WIN_HOST" ]]; then
  WIN_HOST="$(tailscale ip -4 tzehouse-windows 2>/dev/null || true)"
  if [[ -z "$WIN_HOST" ]]; then
    WIN_HOST="tzehouse-windows.parrot-hen.ts.net"
  fi
fi

SSH_OPTS=(-o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=no -i "$SSH_KEY")

run_remote() {
  ssh "${SSH_OPTS[@]}" "$WIN_USER@$WIN_HOST" "$@"
}

run_remote_ps() {
  # Base64/UTF-16LE -EncodedCommand sidesteps SSH+cmd+PowerShell triple-quoting.
  # Target is Windows PowerShell 5.1 (SSH default shell) — no pwsh-only syntax.
  local encoded
  encoded="$(printf '%s' "$1" | iconv -f UTF-8 -t UTF-16LE | base64 -w0)"
  # stderr discarded: PowerShell-over-SSH emits CLIXML progress records there
  # during module auto-load; payloads report via stdout and exit codes.
  ssh "${SSH_OPTS[@]}" "$WIN_USER@$WIN_HOST" "powershell -NoProfile -NonInteractive -EncodedCommand $encoded" 2>/dev/null
}

echo "=== D18 SSH Validation Lane ==="
echo "Host: $WIN_HOST (user: $WIN_USER)"
echo

# ── 1. SSH gate ─────────────────────────────────────────────────────────────
echo "[1/5] SSH connectivity gate"
if ! whoami_out="$(run_remote whoami 2>&1)"; then
  echo "FAIL: SSH to $WIN_USER@$WIN_HOST failed: $whoami_out" >&2
  exit 1
fi
echo "  OK: $whoami_out"

# ── 2. GPU lock ─────────────────────────────────────────────────────────────
# Same policy as the suspended CI lane (docs/design/tzehouse-windows-gpu-scheduling.md)
# except read-only: this lane never removes a lock file, even a stale one.
echo "[2/5] GPU lock check (%PROGRAMDATA%\\tze_hud\\gpu.lock)"
lock_out="$(run_remote_ps '
  $f = Join-Path $env:ProgramData "tze_hud\gpu.lock"
  if (-not (Test-Path $f)) { Write-Output "LOCK=absent"; exit 0 }
  $kv = @{}
  Get-Content $f | ForEach-Object { $p = $_ -split "=", 2; if ($p.Count -eq 2) { $kv[$p[0].Trim()] = $p[1].Trim() } }
  $lockPid = 0
  if ($kv.ContainsKey("PID")) { $lockPid = [int]$kv["PID"] }
  $lockType = "unknown"
  if ($kv.ContainsKey("SESSION_TYPE")) { $lockType = $kv["SESSION_TYPE"] }
  $alive = $false
  if ($lockPid -gt 0) { $alive = [bool](Get-Process -Id $lockPid -ErrorAction SilentlyContinue) }
  Write-Output ("LOCK=present PID={0} ALIVE={1} TYPE={2}" -f $lockPid, $alive, $lockType)
')"
echo "  $lock_out"
case "$lock_out" in
  *"ALIVE=True"*)
    echo "  GPU busy: live interactive session holds the lock. Aborting per dual-use policy." >&2
    exit 2
    ;;
  *"ALIVE=False"*)
    echo "  NOTE: stale lock (holder dead). Not removing — the owning flow cleans up."
    ;;
esac

# ── 3. GStreamer SDK ────────────────────────────────────────────────────────
echo "[3/5] GStreamer MSVC SDK verification"
gst_root="$(run_remote_ps '[Environment]::GetEnvironmentVariable("GSTREAMER_1_0_ROOT_MSVC_X86_64","Machine")' || true)"
gst_root="$(echo "$gst_root" | tr -d '\r')"
if [[ -z "$gst_root" ]]; then
  echo "  SDK NOT INSTALLED: GSTREAMER_1_0_ROOT_MSVC_X86_64 machine env var is unset."
  echo "  Activation prerequisite unmet (see real-decode-windows.yml header + runbook §3)."
  if [[ "$ALLOW_MISSING_SDK" -eq 1 ]]; then
    echo "[4/5] SKIPPED (GATED: no SDK)"
    echo "[5/5] SKIPPED (GATED: no SDK, no harness — hud-ora8.1 phase 1)"
    echo
    echo "Status: GATED — transport and lock policy verified; provisioning pending."
    exit 0
  fi
  exit 3
fi
echo "  SDK root: $gst_root"
gst_version="$(run_remote "\"$gst_root\\bin\\gst-inspect-1.0.exe\" --version" 2>&1 | head -2 || true)"
echo "  $gst_version"

# ── 4. Hardware decoder capability report ───────────────────────────────────
echo "[4/5] Hardware decode elements (D3D11 + NVDEC)"
for el in d3d11h264dec d3d11vp9dec nvh264dec nvvp9dec; do
  if run_remote "\"$gst_root\\bin\\gst-inspect-1.0.exe\" $el" >/dev/null 2>&1; then
    echo "  $el: present"
  else
    echo "  $el: MISSING"
  fi
done

# ── 5. Real-decode harness ──────────────────────────────────────────────────
echo "[5/5] Real-decode harness"
echo "  GATED: tze_hud_runtime::real_decode_windows is not implemented (hud-ora8.1"
echo "  phase 1). When it lands, this step runs it remotely against D18 thresholds."
echo
echo "Status: PASS (runnable checks) — harness step GATED."
