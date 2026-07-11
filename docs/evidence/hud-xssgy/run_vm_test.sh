#!/usr/bin/env bash
# Reproduce the hud-xssgy live console-attach evidence run.
#
# Feeds test-matrix-input.txt to an interactive PTY session on the hud-windows
# VM, one line at a time with a pacing delay — ConPTY renders asynchronously,
# so a single-shot non-interactive `ssh host "command"` (or even `ssh -tt host
# "cmd.exe /c ..."`) tears the session down before the render pipe flushes and
# silently drops the output (see README.md "Methodology pitfall"). An
# interactive session fed line-by-line via stdin avoids that.
#
# Usage:
#   .claude/skills/user-test/scripts/hud_vm_env.sh > /tmp/hud-xssgy-env.sh
#   ./run_vm_test.sh <remote-exe-filename-under-C:\tze_hud_test> <output-path-prefix>
#
# Prereqs: exe already scp'd to C:\tze_hud_test\<remote-exe-filename> on the
# host (see README.md for the deploy step); HUD_SSH_KEY / hud_vm_env.sh
# resolve host + auth.
set -euo pipefail
EXE_NAME="$1"
OUT_FILE="$2"
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/test-matrix-input.txt"
ENV_FILE="${HUD_XSSGY_ENV_FILE:-/tmp/hud-xssgy-env.sh}"
HUD_SSH_KEY="${HUD_SSH_KEY:-$HOME/.ssh/hud-ssh-key}"

# shellcheck disable=SC1090
source "$ENV_FILE"

{
  while IFS= read -r line; do
    line="${line//TZE_HUD_EXE/$EXE_NAME}"
    printf '%s\r\n' "$line"
    sleep 1.2
  done < "$SRC"
  sleep 2
} | ssh -tt -i "$HUD_SSH_KEY" -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no \
  "admin-user@${TZE_HUD_TEST_HOST}" \
  > "$OUT_FILE.raw" 2>&1

# Strip ANSI/VT escape sequences (ConPTY control codes) for a readable transcript.
sed -E 's/\x1B\[[0-9;?]*[a-zA-Z]//g; s/\x1B\][^\x07]*\x07//g' "$OUT_FILE.raw" | tr -d '\r' > "$OUT_FILE.txt"
echo "Wrote $OUT_FILE.raw and $OUT_FILE.txt"
