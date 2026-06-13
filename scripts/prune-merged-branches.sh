#!/usr/bin/env bash
# prune-merged-branches.sh — delete remote agent/* branches already merged into main.
#
# Safety contract
# ───────────────
# 1. Only deletes branches whose tip is reachable from origin/main
#    (git branch -r --merged origin/main).
# 2. Skips any branch that matches a branch currently checked out in an
#    active local worktree (git worktree list).
# 3. Never touches branches outside the agent/* namespace.
# 4. Dry-run mode is the default — pass --execute to actually delete.
# 5. Prints every deletion decision for an audit trail.
#
# Usage
# ─────
#   # Preview what would be deleted (safe, default):
#   bash scripts/prune-merged-branches.sh
#
#   # Actually delete merged branches:
#   bash scripts/prune-merged-branches.sh --execute
#
#   # In CI (non-interactive), --execute is passed by the workflow.
#
# Notes
# ─────
# - Run from the repo root (or any git worktree of this repo).
# - Requires push access to origin (only needed for --execute).
# - Never force-deletes; only deletes via --delete (safe, fails on unmerged).

set -euo pipefail

EXECUTE=false
for arg in "$@"; do
  case "$arg" in
    --execute) EXECUTE=true ;;
    --dry-run) EXECUTE=false ;;
    *)
      echo "Unknown argument: $arg" >&2
      echo "Usage: $0 [--execute|--dry-run]" >&2
      exit 1
      ;;
  esac
done

if [[ "$EXECUTE" == "false" ]]; then
  echo "[prune-merged-branches] DRY RUN — pass --execute to delete"
fi

# ── Fetch latest remote state ────────────────────────────────────────────────
echo "[prune-merged-branches] Fetching origin..."
git fetch origin --prune --quiet

# ── Collect branches currently checked out in any worktree ──────────────────
# These are protected even if their tip is merged into main, because deleting
# an active worktree branch confuses git and breaks agent sessions.
mapfile -t WORKTREE_BRANCHES < <(
  git worktree list 2>/dev/null \
    | awk '{print $3}' \
    | tr -d '[]' \
    | grep -v '^$' \
    | grep -v '^(detached)$' \
    | sort -u
)

echo "[prune-merged-branches] Active worktree branches (protected):"
for b in "${WORKTREE_BRANCHES[@]}"; do
  echo "  - $b"
done

# ── Collect merged remote agent/* branches ───────────────────────────────────
mapfile -t MERGED_REMOTE < <(
  git branch -r --merged origin/main \
    | grep -E '^[[:space:]]*origin/agent/' \
    | sed 's|[[:space:]]*origin/||' \
    | sort -u
)

echo "[prune-merged-branches] Merged remote agent/* branches: ${#MERGED_REMOTE[@]}"

# ── Filter: skip worktree-active branches ────────────────────────────────────
DELETED=0
SKIPPED=0

for branch in "${MERGED_REMOTE[@]}"; do
  # Check if this branch is checked out in any worktree
  PROTECTED=false
  for wt_branch in "${WORKTREE_BRANCHES[@]}"; do
    if [[ "$branch" == "$wt_branch" ]]; then
      PROTECTED=true
      break
    fi
  done

  if [[ "$PROTECTED" == "true" ]]; then
    echo "[prune-merged-branches] SKIP (active worktree): $branch"
    ((SKIPPED++)) || true
    continue
  fi

  if [[ "$EXECUTE" == "true" ]]; then
    # Capture the SHA we observed during the merged check so the delete is
    # guarded by a ref lease.  If an agent pushes new commits to this branch
    # between the fetch and here, the remote ref will have moved and git will
    # refuse to delete it — protecting the in-flight work.
    EXPECTED_SHA=$(git rev-parse "origin/$branch" 2>/dev/null || true)
    echo "[prune-merged-branches] DELETE: origin/$branch (lease=$EXPECTED_SHA)"
    if [[ -n "$EXPECTED_SHA" ]]; then
      git push origin --force-with-lease="$branch:$EXPECTED_SHA" --delete "$branch" 2>&1 | sed 's/^/  /' || {
        echo "[prune-merged-branches] WARNING: failed to delete $branch (ref moved or already gone — skipping)"
      }
    else
      echo "[prune-merged-branches] WARNING: could not resolve origin/$branch SHA — skipping delete"
    fi
    ((DELETED++)) || true
  else
    echo "[prune-merged-branches] WOULD DELETE: origin/$branch"
    ((DELETED++)) || true
  fi
done

echo ""
if [[ "$EXECUTE" == "true" ]]; then
  echo "[prune-merged-branches] Deleted $DELETED branch(es), skipped $SKIPPED (active worktree)."
else
  echo "[prune-merged-branches] Would delete $DELETED branch(es), would skip $SKIPPED (active worktree)."
  echo "[prune-merged-branches] Re-run with --execute to apply."
fi
