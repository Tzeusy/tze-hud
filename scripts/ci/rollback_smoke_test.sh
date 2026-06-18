#!/usr/bin/env bash
# Smoke-test the Windows rollback runbook against a local fixture.
#
# The fixture intentionally avoids the real Windows host. It mirrors the
# documented sequence: verify a known-good checksum, preserve and replace the
# active C:\tze_hud\tze_hud.exe equivalent, verify the active checksum, then
# restart TzeHudOverlay through a mocked schtasks command.

set -euo pipefail

die() {
  echo "[rollback-smoke] ERROR: $*" >&2
  exit 1
}

assert_hash_matches_checksum() {
  local exe_path="$1"
  local checksum_path="$2"
  local expected_hash actual_hash

  [[ -f "$exe_path" ]] || die "missing executable: $exe_path"
  [[ -f "$checksum_path" ]] || die "missing checksum: $checksum_path"

  expected_hash="$(awk '{print tolower($1)}' "$checksum_path")"
  [[ "$expected_hash" =~ ^[0-9a-f]{64}$ ]] || die "malformed checksum in $checksum_path"

  actual_hash="$(sha256sum "$exe_path" | awk '{print tolower($1)}')"
  [[ "$actual_hash" == "$expected_hash" ]] || {
    die "checksum mismatch for $exe_path: expected $expected_hash, got $actual_hash"
  }
}

assert_log_contains() {
  local needle="$1"
  local log_path="$2"
  grep -Fxq "$needle" "$log_path" || die "missing restart log entry: $needle"
}

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tze-hud-rollback-smoke.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT

ACTIVE_DIR="$WORK_DIR/C/tze_hud"
KNOWN_GOOD_DIR="$WORK_DIR/releases/known-good"
ROLLBACK_BACKUP_DIR="$ACTIVE_DIR/rollback/pre-rollback-fixture"
MOCK_BIN="$WORK_DIR/bin"
SCHTASKS_LOG="$WORK_DIR/schtasks.log"

mkdir -p "$ACTIVE_DIR" "$KNOWN_GOOD_DIR" "$ROLLBACK_BACKUP_DIR" "$MOCK_BIN"

ACTIVE_EXE="$ACTIVE_DIR/tze_hud.exe"
KNOWN_GOOD_EXE="$KNOWN_GOOD_DIR/tze_hud.exe"
KNOWN_GOOD_CHECKSUM="$KNOWN_GOOD_DIR/tze_hud.exe.sha256"

printf 'regressed active executable fixture\n' >"$ACTIVE_EXE"
printf 'prior known-good executable fixture\n' >"$KNOWN_GOOD_EXE"

(
  cd "$KNOWN_GOOD_DIR"
  sha256sum tze_hud.exe >tze_hud.exe.sha256
  sha256sum -c tze_hud.exe.sha256 >/dev/null
)

cat >"$MOCK_BIN/schtasks" <<'EOS'
#!/usr/bin/env bash
set -euo pipefail
: "${SCHTASKS_LOG:?SCHTASKS_LOG must be set}"
printf '%s\n' "$*" >>"$SCHTASKS_LOG"
case "$*" in
  "/End /TN TzeHudOverlay"|"/Run /TN TzeHudOverlay")
    exit 0
    ;;
  *)
    echo "unexpected schtasks arguments: $*" >&2
    exit 64
    ;;
esac
EOS
chmod +x "$MOCK_BIN/schtasks"

# Step 1: verify the prior known-good artifact before touching active state.
assert_hash_matches_checksum "$KNOWN_GOOD_EXE" "$KNOWN_GOOD_CHECKSUM"

# Step 2: preserve the active binary and stop the scheduled task.
cp -f "$ACTIVE_EXE" "$ROLLBACK_BACKUP_DIR/tze_hud.exe"
PATH="$MOCK_BIN:$PATH" SCHTASKS_LOG="$SCHTASKS_LOG" schtasks /End /TN TzeHudOverlay

# Step 3: replace active C:\tze_hud\tze_hud.exe and verify the active checksum.
cp -f "$KNOWN_GOOD_EXE" "$ACTIVE_EXE"
cp -f "$KNOWN_GOOD_CHECKSUM" "$ACTIVE_DIR/tze_hud.exe.sha256"
assert_hash_matches_checksum "$ACTIVE_EXE" "$KNOWN_GOOD_CHECKSUM"

# Step 4: restart through Task Scheduler.
PATH="$MOCK_BIN:$PATH" SCHTASKS_LOG="$SCHTASKS_LOG" schtasks /Run /TN TzeHudOverlay

BACKUP_CHECKSUM="$WORK_DIR/pre-rollback-tze_hud.exe.sha256"
sha256sum "$ROLLBACK_BACKUP_DIR/tze_hud.exe" >"$BACKUP_CHECKSUM"
assert_hash_matches_checksum "$ROLLBACK_BACKUP_DIR/tze_hud.exe" "$BACKUP_CHECKSUM"
assert_log_contains "/End /TN TzeHudOverlay" "$SCHTASKS_LOG"
assert_log_contains "/Run /TN TzeHudOverlay" "$SCHTASKS_LOG"

echo "[rollback-smoke] pass: checksum-verified rollback fixture restarted TzeHudOverlay"
