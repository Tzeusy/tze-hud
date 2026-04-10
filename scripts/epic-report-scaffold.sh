#!/usr/bin/env bash
# epic-report-scaffold.sh - Generate initial report scaffold for a beads epic.
# Creates the report file, diagram directory, and populates metadata from beads.
#
# Usage: bash scripts/epic-report-scaffold.sh <epic-id> [repo_root]
#   epic-id: The beads epic ID
#   repo_root: defaults to current directory
#
# Requires: bd (beads CLI), jq, git

set -euo pipefail

if [ -z "${1:-}" ]; then
  echo "Usage: bash scripts/epic-report-scaffold.sh <epic-id> [repo_root]" >&2
  echo "  epic-id: The beads epic ID (e.g., hud-abc123)" >&2
  exit 1
fi

for cmd in bd jq git; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: Missing required command: $cmd" >&2
    exit 1
  fi
done

normalize_issue_json() {
  # Support both `bd show --json` object roots and array roots.
  printf '%s\n' "$1" | jq -ce '
    if type == "array" then
      if length > 0 then .[0] else empty end
    elif type == "object" then .
    else empty
    end
  ' 2>/dev/null
}

normalize_children_json() {
  printf '%s\n' "$1" | jq -ce '
    if type == "array" then
      .
    elif type == "object" then
      if (.children? | type) == "array" then .children else [.] end
    else
      []
    end
  ' 2>/dev/null
}

EPIC_ID="$1"
REPO="${2:-.}"
cd "$REPO"

echo "Gathering epic data for $EPIC_ID..." >&2

epic_json_raw=$(bd show "$EPIC_ID" --json 2>/dev/null) || {
  echo "ERROR: Could not find epic $EPIC_ID (bd show failed)" >&2
  exit 1
}

epic_json=$(normalize_issue_json "$epic_json_raw") || {
  echo "ERROR: Could not parse epic payload for $EPIC_ID" >&2
  exit 1
}

epic_title=$(echo "$epic_json" | jq -r '.title // empty')
if [ -z "$epic_title" ]; then
  echo "ERROR: Could not find epic $EPIC_ID or it has no title" >&2
  exit 1
fi

epic_desc=$(echo "$epic_json" | jq -r '.description // ""')
epic_status=$(echo "$epic_json" | jq -r '.status // "unknown"')
epic_type=$(echo "$epic_json" | jq -r '.issue_type // .type // "unknown"')
epic_priority=$(echo "$epic_json" | jq -r '.priority // "unknown"')

echo "  Title: $epic_title" >&2
echo "  Status: $epic_status" >&2
echo "  Type: $epic_type" >&2

children_json_raw=$(bd children "$EPIC_ID" --json 2>/dev/null || echo '[]')
children_json=$(normalize_children_json "$children_json_raw") || children_json='[]'

total_children=$(echo "$children_json" | jq 'length')
closed_children=$(echo "$children_json" | jq '[.[] | select(.status == "closed")] | length')

echo "  Children: $closed_children/$total_children closed" >&2

slug=$(
  echo "$epic_title" \
    | tr '[:upper:]' '[:lower:]' \
    | sed 's/[^a-z0-9]/-/g' \
    | sed 's/--*/-/g' \
    | sed 's/^-//' \
    | sed 's/-$//' \
    | cut -c1-50
)
if [ -z "$slug" ]; then
  slug="$EPIC_ID"
fi

report_dir="docs/reports"
diagram_dir="$report_dir/diagrams"
mkdir -p "$diagram_dir"

report_file="$report_dir/${EPIC_ID}-${slug}.md"
echo "  Report: $report_file" >&2
if [ -e "$report_file" ]; then
  echo "ERROR: Report file already exists: $report_file" >&2
  echo "Refusing to overwrite existing report." >&2
  exit 1
fi

date_str=$(date +%Y-%m-%d)

commit_log=$(
  git log --oneline --all --grep="$EPIC_ID" 2>/dev/null | head -20 \
    || echo "(no commits found referencing $EPIC_ID)"
)

files_changed="(could not determine)"
if files_changed_raw=$(git log --all --grep="$EPIC_ID" --name-only --pretty=format: 2>/dev/null); then
  files_changed=$(printf '%s\n' "$files_changed_raw" | awk 'NF' | sort -u | head -30)
  if [ -z "$files_changed" ]; then
    files_changed="(no matching files found)"
  fi
fi

children_table=""
if [ "$total_children" -gt 0 ]; then
  children_table=$(
    echo "$children_json" \
      | jq -r '
          def mdcell($default):
            if . == null then
              $default
            else
              tostring
              | gsub("\r\n|\r|\n"; " ")
              | gsub("\\|"; "\\|")
            end;
          .[]
          | "| \((.id | mdcell("-"))) | \((.title | mdcell("-"))) | \((.status | mdcell("unknown"))) | \((.priority | mdcell("-"))) | \((.issue_type // .type) | mdcell("task")) |"
        '
  )
fi

cat > "$report_file" << SCAFFOLD
# Epic Report: $epic_title

**Epic ID**: \`$EPIC_ID\`
**Date**: $date_str
**Status**: $closed_children/$total_children children closed ($epic_status)
**Priority**: $epic_priority
**Spec coverage**: <!-- TODO: list spec sections covered -->

## Summary

<!-- TODO: 2-3 paragraphs covering:
  - What was built and why (link to project spirit)
  - Key design decisions made during implementation
  - Current state: what works, what's provisional, what's deferred
-->

$epic_desc

---

## Architecture

<!-- TODO: Generate 1-2 excalidraw diagrams showing what was built.
  Color conventions:
  - New/added: #a7f3d0 (green)
  - Modified: #fef3c7 (yellow)
  - Existing: #e2e8f0 (gray)
  - Removed: #fecaca (red)
  - External: #ddd6fe (purple)

  Generate .excalidraw file using /excalidraw-diagram skill, then render to PNG.
-->

<!-- ![Architecture overview](diagrams/${EPIC_ID}-architecture.png) -->

---

## Implementation

### Children

| Bead ID | Title | Status | Priority | Type |
|---------|-------|--------|----------|------|
$children_table

<!-- TODO: For each child bead, expand with:
  - What was done (1-3 sentences)
  - Key code locations (file:line-range format)
  - Design decisions
  - Caveats / known limitations
-->

---

## Spec Compliance

<!-- TODO: Map spec sections to implementation status -->

| Spec Section | Status | Evidence | Notes |
|-------------|--------|---------|-------|
| <!-- spec/section --> | <!-- Implemented/Partial/Deferred --> | <!-- file:line --> | <!-- notes --> |

---

## Test Coverage

### New/changed test files

| File | Tests | What it covers |
|------|-------|---------------|
| <!-- test file --> | <!-- count --> | <!-- description --> |

### Coverage gaps

| Area | Why untested | Risk | Follow-up? |
|------|------------|------|-----------|
| <!-- component --> | <!-- reason --> | <!-- H/M/L --> | <!-- bead ID or "no" --> |

### Test confidence

<!-- TODO: Brief assessment - behavior vs implementation testing, critical path coverage -->

---

## Subsequent Work

### Open beads (existing)

<!-- TODO: List any remaining open children -->

### New follow-up beads

<!-- TODO: Create follow-up beads for remaining TODOs:
  bd create --title="..." --type=task --priority=2 --parent=$EPIC_ID --json
-->

| Bead ID | Title | Type | Priority | Rationale |
|---------|-------|------|----------|-----------|
| <!-- new-bead-id --> | <!-- title --> | <!-- task/bug --> | <!-- P0-P4 --> | <!-- why --> |

### Deferred decisions

| Decision | Context | Revisit when |
|----------|---------|-------------|
| <!-- what --> | <!-- why deferred --> | <!-- trigger --> |

---

## Risks & Notes for Reviewer

### Known risks

| Risk | Severity | Mitigation | Evidence |
|------|----------|-----------|----------|
| <!-- risk --> | <!-- H/M/L --> | <!-- action --> | <!-- file:line --> |

### Questions for reviewer

<!-- TODO: Design decisions needing human judgment, assumptions made -->

### What to look at first

<!-- TODO: Prioritized files/areas for human review -->

---

## Appendix

### A. Commits referencing this epic

\`\`\`
$commit_log
\`\`\`

### B. Files changed

\`\`\`
$files_changed
\`\`\`

### C. Diagram source files

| Diagram | Source | Rendered |
|---------|--------|----------|
| <!-- Architecture --> | \`diagrams/${EPIC_ID}-architecture.excalidraw\` | \`diagrams/${EPIC_ID}-architecture.png\` |
SCAFFOLD

echo "" >&2
echo "=== Scaffold generated: $report_file ===" >&2
echo "" >&2
echo "Next steps:" >&2
echo "  1. Fill in TODO sections" >&2
echo "  2. Generate excalidraw diagrams and render to PNG" >&2
echo "  3. Create follow-up beads for remaining work" >&2
echo "  4. Link report to epic notes" >&2
echo "  5. Commit report + diagrams" >&2
