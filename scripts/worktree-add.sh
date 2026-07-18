#!/usr/bin/env bash
# worktree-add.sh — create a tze_hud git worktree with its Rust target/ dir
# pre-symlinked onto /data instead of the tight root filesystem.
#
# Why
# ───
# `/` on this build host is a chronically tight disk; `/data` is a separate,
# mostly-empty disk. Rust target/ dirs are the bulk of worktree disk usage
# and are fully regenerable, so they're the safe thing to relocate. See the
# "Host disk imbalance" note in AGENTS.md's Notes to self / CI-Build section.
#
# This script wraps `git worktree add`, then symlinks the new worktree's
# target/ to /data/tze_hud-cargo-target/<name>/target BEFORE any cargo
# command runs there, so no build output ever lands on / in the first place.
#
# Usage
# ─────
#   scripts/worktree-add.sh <path> [-b <branch>] [git worktree add args...]
#
# Example (matches the pattern in AGENTS.md's Worker Isolation section):
#   scripts/worktree-add.sh .worktrees/agent-hud-XXXX -b agent/hud-XXXX
#
# Requires /data/tze_hud-cargo-target to already exist and be owned by the
# invoking user (one-time root setup: `sudo mkdir -p /data/tze_hud-cargo-target
# && sudo chown "$USER":"$USER" /data/tze_hud-cargo-target`).

set -euo pipefail

CACHE_ROOT="/data/tze_hud-cargo-target"

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <worktree-path> [git worktree add args...]" >&2
  exit 1
fi

WORKTREE_PATH="$1"
shift

if [ ! -d "$CACHE_ROOT" ]; then
  echo "error: $CACHE_ROOT does not exist. One-time setup:" >&2
  echo "  sudo mkdir -p $CACHE_ROOT && sudo chown \"\$USER\":\"\$USER\" $CACHE_ROOT" >&2
  exit 1
fi

git worktree add "$WORKTREE_PATH" "$@"

WORKTREE_NAME="$(basename "$WORKTREE_PATH")"
TARGET_CACHE="$CACHE_ROOT/$WORKTREE_NAME/target"
mkdir -p "$TARGET_CACHE"

if [ -e "$WORKTREE_PATH/target" ] && [ ! -L "$WORKTREE_PATH/target" ]; then
  echo "warning: $WORKTREE_PATH/target already exists and is not a symlink; leaving it alone" >&2
else
  ln -sfn "$TARGET_CACHE" "$WORKTREE_PATH/target"
  echo "linked $WORKTREE_PATH/target -> $TARGET_CACHE"
fi
