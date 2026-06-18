#!/usr/bin/env bash
# Verify that a published tze_hud.exe artifact matches its pipeline checksum.

set -euo pipefail

die() {
  echo "[release-provenance-smoke] ERROR: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'USAGE'
Usage: release_artifact_provenance_smoke.sh --artifact-dir <dir>

The artifact directory must contain:
  tze_hud.exe
  tze_hud.exe.sha256
USAGE
}

ARTIFACT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir)
      ARTIFACT_DIR="${2:-}"
      [[ -n "$ARTIFACT_DIR" ]] || die "--artifact-dir requires a value"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "$ARTIFACT_DIR" ]] || {
  usage
  die "missing --artifact-dir"
}

EXE_NAME="tze_hud.exe"
CHECKSUM_NAME="tze_hud.exe.sha256"
EXE_PATH="${ARTIFACT_DIR}/${EXE_NAME}"
CHECKSUM_PATH="${ARTIFACT_DIR}/${CHECKSUM_NAME}"

[[ -d "$ARTIFACT_DIR" ]] || die "artifact directory not found: $ARTIFACT_DIR"
[[ -f "$EXE_PATH" ]] || die "missing release artifact: $EXE_PATH"
[[ -f "$CHECKSUM_PATH" ]] || die "missing checksum artifact: $CHECKSUM_PATH"

CHECKSUM_RECORD="$(awk 'NF { print $1, $2, $3; exit }' "$CHECKSUM_PATH")"

[[ -n "$CHECKSUM_RECORD" ]] || die "checksum file is empty: $CHECKSUM_PATH"
read -r EXPECTED_HASH CHECKSUM_TARGET EXTRA_FIELD <<< "$CHECKSUM_RECORD"
[[ -z "${EXTRA_FIELD:-}" ]] || die "checksum file must contain exactly '<sha256>  ${EXE_NAME}'"
[[ "$EXPECTED_HASH" =~ ^[0-9A-Fa-f]{64}$ ]] || die "malformed SHA-256 digest: $EXPECTED_HASH"
[[ "$CHECKSUM_TARGET" == "$EXE_NAME" ]] || {
  die "checksum target must reference ${EXE_NAME}, got: ${CHECKSUM_TARGET:-<missing>}"
}

ACTUAL_HASH="$(sha256sum "$EXE_PATH" | awk '{ print tolower($1) }')"
EXPECTED_HASH="$(printf '%s' "$EXPECTED_HASH" | tr 'A-F' 'a-f')"

[[ "$ACTUAL_HASH" == "$EXPECTED_HASH" ]] || {
  die "checksum mismatch for ${EXE_NAME}: expected ${EXPECTED_HASH}, got ${ACTUAL_HASH}"
}

(
  cd "$ARTIFACT_DIR"
  sha256sum -c "$CHECKSUM_NAME" >/dev/null
)

echo "[release-provenance-smoke] pass: ${EXE_NAME} matches ${CHECKSUM_NAME}"
