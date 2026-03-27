#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  deploy_windows_hud.sh [options]

Automates:
1) deploy a full Windows .exe built on Linux (or optionally build a package)
2) copy .exe to Windows via scp
3) trigger interactive scheduled task launch
4) optional remote log tail

Options:
  --win-user <user>          Windows SSH user (default: hudbot)
  --win-host <host>          Windows host (default: tzehouse-windows.parrot-hen.ts.net)
  --full-app-exe <path>      Preferred: prebuilt full app .exe from Linux
  --local-exe <path>         Alias for --full-app-exe
  --package <name>           Optional fallback: cargo package/bin crate to build
  --target <triple>          Rust target (default: x86_64-pc-windows-gnu)
  --profile <name>           Cargo profile: release|debug (default: release)
  --remote-dir <path>        Windows destination dir (default: C:\tze_hud)
  --task-name <name>         Scheduled task name (default: TzeHudInteractive)
  --launch-mode <mode>       Launch mode: auto|task|direct (default: auto)
  --sim-subtitles            Enable startup subtitle simulation (direct launch path)
  --setup-script <path>      Local setup PS1 to register task (default: windows_setup_hud_automation.ps1)
  --bootstrap-task           Auto-create task if trigger fails (default: enabled)
  --no-bootstrap-task        Fail if task trigger fails
  --tail                     Tail remote stdout log after launch
  --tail-lines <n>           Initial line count for tail (default: 120)
  --skip-build               Skip cargo build (reuse existing .exe)
  --no-run                   Copy only; do not trigger scheduled task
  -h, --help                 Show help

Environment:
  WIN_USER                   Default Windows SSH user (default: hudbot)
  FULL_APP_EXE               Default full app .exe path (same as --full-app-exe)
  LOCAL_EXE                  Back-compat alias for FULL_APP_EXE
  SSH_OPTS                   Extra SSH/SCP options (split by shell)
EOF
}

WIN_HOST="tzehouse-windows.parrot-hen.ts.net"
PACKAGE=""
TARGET="x86_64-pc-windows-gnu"
PROFILE="release"
REMOTE_DIR_WIN='C:\tze_hud'
TASK_NAME="TzeHudInteractive"
LAUNCH_MODE="auto"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SETUP_SCRIPT_LOCAL="${SCRIPT_DIR}/windows_setup_hud_automation.ps1"
BOOTSTRAP_TASK=1
SIM_SUBTITLES=0
TAIL=0
TAIL_LINES=120
SKIP_BUILD=0
NO_RUN=0
WIN_USER="${WIN_USER:-hudbot}"
LOCAL_EXE_OVERRIDE="${FULL_APP_EXE:-${LOCAL_EXE:-}}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --full-app-exe) LOCAL_EXE_OVERRIDE="${2:-}"; shift 2 ;;
    --win-user) WIN_USER="${2:-}"; shift 2 ;;
    --win-host) WIN_HOST="${2:-}"; shift 2 ;;
    --package) PACKAGE="${2:-}"; shift 2 ;;
    --local-exe) LOCAL_EXE_OVERRIDE="${2:-}"; shift 2 ;;
    --target) TARGET="${2:-}"; shift 2 ;;
    --profile) PROFILE="${2:-}"; shift 2 ;;
    --remote-dir) REMOTE_DIR_WIN="${2:-}"; shift 2 ;;
    --task-name) TASK_NAME="${2:-}"; shift 2 ;;
    --launch-mode) LAUNCH_MODE="${2:-}"; shift 2 ;;
    --sim-subtitles) SIM_SUBTITLES=1; shift ;;
    --setup-script) SETUP_SCRIPT_LOCAL="${2:-}"; shift 2 ;;
    --bootstrap-task) BOOTSTRAP_TASK=1; shift ;;
    --no-bootstrap-task) BOOTSTRAP_TASK=0; shift ;;
    --tail) TAIL=1; shift ;;
    --tail-lines) TAIL_LINES="${2:-}"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --no-run) NO_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "$PROFILE" != "release" && "$PROFILE" != "debug" ]]; then
  echo "--profile must be release or debug" >&2
  exit 2
fi

if [[ "$LAUNCH_MODE" != "auto" && "$LAUNCH_MODE" != "task" && "$LAUNCH_MODE" != "direct" ]]; then
  echo "--launch-mode must be auto, task, or direct" >&2
  exit 2
fi

REMOTE_DIR_SCP="/${REMOTE_DIR_WIN//\\//}"

if [[ -n "$LOCAL_EXE_OVERRIDE" ]]; then
  LOCAL_EXE="$LOCAL_EXE_OVERRIDE"
  EXE_BASENAME="$(basename "$LOCAL_EXE")"
  PROCESS_NAME="${EXE_BASENAME%.exe}"
elif [[ -n "$PACKAGE" ]]; then
  LOCAL_EXE="target/${TARGET}/${PROFILE}/${PACKAGE}.exe"
  EXE_BASENAME="${PACKAGE}.exe"
  PROCESS_NAME="${PACKAGE}"
else
  echo "No executable source provided." >&2
  echo "Use --full-app-exe /path/to/full-app.exe (recommended) or --package <name>." >&2
  exit 2
fi

REMOTE_EXE_WIN="${REMOTE_DIR_WIN}\\${EXE_BASENAME}"
REMOTE_EXE_SCP="${REMOTE_DIR_SCP}/${EXE_BASENAME}"

IFS=' ' read -r -a EXTRA_SSH_OPTS <<< "${SSH_OPTS:-}"

ssh_win() {
  ssh "${EXTRA_SSH_OPTS[@]}" "${WIN_USER}@${WIN_HOST}" "$@"
}

copy_setup_script() {
  local remote_setup_win="${REMOTE_DIR_WIN}\\windows_setup_hud_automation.ps1"
  local remote_setup_scp="${REMOTE_DIR_SCP}/windows_setup_hud_automation.ps1"

  if [[ ! -f "$SETUP_SCRIPT_LOCAL" ]]; then
    echo "Setup script not found: ${SETUP_SCRIPT_LOCAL}" >&2
    exit 4
  fi

  echo "Task bootstrap: copying ${SETUP_SCRIPT_LOCAL} -> ${WIN_USER}@${WIN_HOST}:${remote_setup_scp}" >&2
  scp "${EXTRA_SSH_OPTS[@]}" "${SETUP_SCRIPT_LOCAL}" "${WIN_USER}@${WIN_HOST}:${remote_setup_scp}"
  echo "${remote_setup_win}"
}

bootstrap_task() {
  local remote_setup_win
  remote_setup_win="$(copy_setup_script)"

  echo "Task bootstrap: registering scheduled task ${TASK_NAME}"
  ssh_win "powershell -NoProfile -ExecutionPolicy Bypass -File ${remote_setup_win} -BaseDir ${REMOTE_DIR_WIN} -TaskName ${TASK_NAME} -ExeName ${EXE_BASENAME}"
}

bootstrap_runner_only() {
  local remote_setup_win
  remote_setup_win="$(copy_setup_script)"
  echo "Direct-launch bootstrap: ensuring run_hud.ps1 exists (no task registration)"
  ssh_win "powershell -NoProfile -ExecutionPolicy Bypass -File ${remote_setup_win} -BaseDir ${REMOTE_DIR_WIN} -TaskName ${TASK_NAME} -ExeName ${EXE_BASENAME} -SkipTaskRegistration"
}

run_task_once() {
  ssh_win "schtasks /Run /TN \"${TASK_NAME}\""
}

launch_direct() {
  if [[ "$SIM_SUBTITLES" -eq 1 ]]; then
    echo "Direct launch: executing ${REMOTE_DIR_WIN}\\run_hud.ps1 with TZE_HUD_SIM_SUBTITLES=1"
    ssh_win "cmd /c set TZE_HUD_SIM_SUBTITLES=1&& powershell -NoProfile -ExecutionPolicy Bypass -File ${REMOTE_DIR_WIN}\\run_hud.ps1"
  else
    echo "Direct launch: executing ${REMOTE_DIR_WIN}\\run_hud.ps1"
    ssh_win "powershell -NoProfile -ExecutionPolicy Bypass -File ${REMOTE_DIR_WIN}\\run_hud.ps1"
  fi
}

if [[ -n "$LOCAL_EXE_OVERRIDE" ]]; then
  echo "[1/5] Using full app exe: ${LOCAL_EXE}"
  echo "[2/5] Skipping cargo build (prebuilt exe provided)"
elif [[ "$SKIP_BUILD" -eq 0 && -n "$PACKAGE" ]]; then
  echo "[1/5] Ensuring Rust target ${TARGET} is installed..."
  rustup target add "${TARGET}"

  echo "[2/5] Building ${PACKAGE} (${PROFILE}) for ${TARGET}..."
  cargo build -p "${PACKAGE}" "--${PROFILE}" --target "${TARGET}"
else
  echo "[1/5] Skipping build (--skip-build, package=${PACKAGE})"
fi

if [[ ! -f "$LOCAL_EXE" ]]; then
  echo "Local exe not found: ${LOCAL_EXE}" >&2
  exit 3
fi

echo "[3/5] Ensuring remote directory exists: ${REMOTE_DIR_WIN}"
ssh_win "powershell -NoProfile -ExecutionPolicy Bypass -Command \"New-Item -Path '${REMOTE_DIR_WIN}' -ItemType Directory -Force | Out-Null; New-Item -Path '${REMOTE_DIR_WIN}\\logs' -ItemType Directory -Force | Out-Null\""

echo "[3.5/5] Stopping existing ${PROCESS_NAME}.exe if running"
ssh_win "powershell -NoProfile -ExecutionPolicy Bypass -Command \"Get-Process -Name '${PROCESS_NAME}' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue\"" || true

echo "[4/5] Copying ${LOCAL_EXE} -> ${WIN_USER}@${WIN_HOST}:${REMOTE_EXE_SCP}"
scp "${EXTRA_SSH_OPTS[@]}" "${LOCAL_EXE}" "${WIN_USER}@${WIN_HOST}:${REMOTE_EXE_SCP}"

if [[ "$NO_RUN" -eq 0 ]]; then
  echo "[5/5] Launching HUD (mode=${LAUNCH_MODE})"
  if [[ "$LAUNCH_MODE" == "task" ]]; then
    if ! run_task_once; then
      if [[ "$BOOTSTRAP_TASK" -eq 1 ]]; then
        echo "Task trigger failed; attempting bootstrap."
        bootstrap_task
        run_task_once
      else
        echo "Task trigger failed and bootstrap disabled (--no-bootstrap-task)." >&2
        exit 5
      fi
    fi
  elif [[ "$LAUNCH_MODE" == "direct" ]]; then
    bootstrap_runner_only
    launch_direct
  else
    if ! run_task_once; then
      if [[ "$BOOTSTRAP_TASK" -eq 1 ]]; then
        echo "Task trigger failed; attempting bootstrap."
        if bootstrap_task && run_task_once; then
          :
        else
          echo "Task launch path unavailable; falling back to direct launch."
          bootstrap_runner_only
          launch_direct
        fi
      else
        echo "Task trigger failed; falling back to direct launch (--no-bootstrap-task set)."
        bootstrap_runner_only
        launch_direct
      fi
    fi
  fi
else
  echo "[5/5] Skipped task run (--no-run)"
fi

echo
echo "Deploy complete."
echo "Remote exe: ${REMOTE_EXE_WIN}"
echo "Remote stdout log: ${REMOTE_DIR_WIN}\\logs\\hud.stdout.log"
echo "Remote stderr log: ${REMOTE_DIR_WIN}\\logs\\hud.stderr.log"

if [[ "$TAIL" -eq 1 ]]; then
  echo
  echo "Tailing remote launcher log (Ctrl-C to stop)..."
  ssh_win "powershell -NoProfile -ExecutionPolicy Bypass -Command \"\$launcher='${REMOTE_DIR_WIN}\\logs\\hud.launcher.log'; if (Test-Path \$launcher) { Get-Content -Path \$launcher -Tail ${TAIL_LINES} -Wait } else { Write-Host 'No launcher log yet.'; exit 0 }\""
fi
