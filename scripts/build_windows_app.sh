#!/usr/bin/env bash
# build_windows_app.sh — Cross-compile the canonical tze_hud application binary for Windows from Linux.
#
# Outputs the built .exe path to stdout on success.
# All diagnostic messages go to stderr.
#
# Usage:
#   scripts/build_windows_app.sh [OPTIONS]
#   OUTPUT_EXE=$(scripts/build_windows_app.sh --profile release)
#
# Options:
#   --target <triple>   Rust cross-compile target (default: x86_64-pc-windows-gnu)
#   --profile <name>    Cargo profile: release | debug  (default: release)
#   --package <name>    Cargo package name (default: tze_hud_app)
#   --bin <name>        Binary name to build (default: tze_hud)
#   --skip-toolchain    Skip `rustup target add` check
#   -h, --help          Show this help
#
# Exit codes:
#   0  success — exe path printed to stdout
#   1  build failed
#   2  bad argument
#   3  toolchain setup failed

set -euo pipefail

TARGET="x86_64-pc-windows-gnu"
PROFILE="release"
PACKAGE="tze_hud_app"
BIN="tze_hud"
SKIP_TOOLCHAIN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)       TARGET="${2:?--target requires a value}"; shift 2 ;;
    --profile)      PROFILE="${2:?--profile requires a value}"; shift 2 ;;
    --package)      PACKAGE="${2:?--package requires a value}"; shift 2 ;;
    --bin)          BIN="${2:?--bin requires a value}"; shift 2 ;;
    --skip-toolchain) SKIP_TOOLCHAIN=1; shift ;;
    -h|--help)
      sed -n '2,/^set -/p' "$0" | grep '^#' | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$PROFILE" != "release" && "$PROFILE" != "debug" ]]; then
  echo "--profile must be 'release' or 'debug', got: ${PROFILE}" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
EXE_PATH="${REPO_ROOT}/target/${TARGET}/${PROFILE}/${BIN}.exe"

echo "[build] package=${PACKAGE} bin=${BIN} target=${TARGET} profile=${PROFILE}" >&2

# Ensure the Rust cross-compilation target is installed.
if [[ "$SKIP_TOOLCHAIN" -eq 0 ]]; then
  echo "[build] ensuring Rust target ${TARGET} is installed..." >&2
  if ! rustup target add "${TARGET}" >&2; then
    echo "[build] ERROR: failed to install Rust target ${TARGET}" >&2
    exit 3
  fi
fi

echo "[build] running: cargo build -p ${PACKAGE} --bin ${BIN} --${PROFILE} --target ${TARGET}" >&2
if ! cargo build \
    -p "${PACKAGE}" \
    --bin "${BIN}" \
    "--${PROFILE}" \
    --target "${TARGET}" \
    --manifest-path "${REPO_ROOT}/Cargo.toml" \
    2>&1 | sed 's/^/[cargo] /' >&2; then
  echo "[build] ERROR: cargo build failed" >&2
  exit 1
fi

if [[ ! -f "$EXE_PATH" ]]; then
  echo "[build] ERROR: expected output not found: ${EXE_PATH}" >&2
  exit 1
fi

echo "[build] built: ${EXE_PATH}" >&2
echo "[build] size: $(du -sh "${EXE_PATH}" | cut -f1)" >&2
echo "[build] sha256: $(sha256sum "${EXE_PATH}" | awk '{print $1}')" >&2

# Print path to stdout for capture by callers.
echo "${EXE_PATH}"
