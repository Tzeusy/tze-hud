#!/usr/bin/env bash
# Verify that a published tze_hud.exe artifact matches its pipeline checksum
# and contains the required Windows manifest resource section.

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
OBJDUMP="${OBJDUMP:-x86_64-w64-mingw32-objdump}"

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

command -v "$OBJDUMP" >/dev/null || die "required PE inspector not found: ${OBJDUMP}"

if ! PE_HEADERS="$($OBJDUMP -p "$EXE_PATH")"; then
  die "failed to inspect PE data directories with ${OBJDUMP}: ${EXE_PATH}"
fi

RESOURCE_FIELDS="$(
  printf '%s\n' "$PE_HEADERS" \
    | awk '/^[[:space:]]*Entry[[:space:]]+2[[:space:]]/ && /Resource Directory/ && !found { print $3, $4; found = 1 }'
)"
read -r RESOURCE_RVA RESOURCE_SIZE <<< "$RESOURCE_FIELDS"

[[ -n "${RESOURCE_RVA:-}" && -n "${RESOURCE_SIZE:-}" ]] \
  || die "missing PE resource directory in ${EXE_NAME}"
[[ "$RESOURCE_RVA" =~ ^[[:xdigit:]]+$ && "$RESOURCE_RVA" =~ [1-9A-Fa-f] ]] \
  || die "nonzero PE resource directory RVA required, got: ${RESOURCE_RVA}"
[[ "$RESOURCE_SIZE" =~ ^[[:xdigit:]]+$ && "$RESOURCE_SIZE" =~ [1-9A-Fa-f] ]] \
  || die "nonzero PE resource directory size required, got: ${RESOURCE_SIZE}"

if ! SECTION_HEADERS="$($OBJDUMP -h "$EXE_PATH")"; then
  die "failed to inspect PE sections with ${OBJDUMP}: ${EXE_PATH}"
fi

RSRC_SIZE="$(printf '%s\n' "$SECTION_HEADERS" | awk '$2 == ".rsrc" { print $3; exit }')"
[[ -n "${RSRC_SIZE:-}" ]] || die "missing .rsrc section in ${EXE_NAME}"
[[ "$RSRC_SIZE" =~ ^[[:xdigit:]]+$ && "$RSRC_SIZE" =~ [1-9A-Fa-f] ]] \
  || die "nonempty .rsrc section required, got size: ${RSRC_SIZE}"

echo "[release-provenance-smoke] pass: ${EXE_NAME} matches ${CHECKSUM_NAME} and contains PE resources"
