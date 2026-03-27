#!/usr/bin/env bash
# check_canonical_vocabulary.sh — CI guard for pre-Round-14 capability vocabulary.
#
# Scans all .rs, .md, .toml, and .proto files for pre-Round-14 stale capability
# names and fails if any are found outside of the intentionally exempted files.
#
# Stale names (Round 14 renames per RFC 0005):
#   read_scene          → read_scene_topology
#   receive_input       → access_input_events
#   zone_publish:<zone> → publish_zone:<zone>
#
# Also checks for the typo "telemetry_read" (should be "read_telemetry") in
# the runtime subscription capability gating.
#
# Usage:
#   ./scripts/check_canonical_vocabulary.sh [--verbose]
#
# Exit codes:
#   0   all clear
#   1   stale vocabulary found
#
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

VERBOSE=0
if [[ "${1:-}" == "--verbose" ]]; then
    VERBOSE=1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# Files (relative to repo root) that legitimately reference legacy names as
# string literals for the purpose of rejecting them. This includes:
#   - implementations of the rejection gate
#   - tests that assert rejection of legacy names
#   - normative docs that document the canonical→stale mapping
#   - historical review docs describing the pre-fix state
#
# Add new entries here if a file intentionally uses legacy names.
EXEMPT_PATHS=(
    "crates/tze_hud_policy/src/security.rs"
    "crates/tze_hud_protocol/src/auth.rs"
    "crates/tze_hud_protocol/src/session_server.rs"
    "crates/tze_hud_config/src/capability.rs"
    "crates/tze_hud_config/src/tests.rs"
    "crates/tze_hud_scene/src/config/mod.rs"
    "crates/tze_hud_scene/src/policy/mod.rs"
    "examples/vertical_slice/src/main.rs"
    "openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md"
    "openspec/changes/v1-mvp-standards/specs/configuration/spec.md"
    "docs/rfcs/0005-session-protocol.md"
    "docs/prompts/07-configuration.md"
    "docs/reviews/0001-scene-contract-round3.md"
)

# Patterns to search for (ERE)
STALE_PATTERNS=(
    "\\bread_scene\\b"
    "\\breceive_input\\b"
    "zone_publish:"
    "\\btelemetry_read\\b"
)

PATTERN_LABELS=(
    "read_scene (use read_scene_topology)"
    "receive_input (use access_input_events)"
    "zone_publish:<zone> (use publish_zone:<zone>)"
    "telemetry_read (use read_telemetry)"
)

# Build a sed expression to filter out exempt paths from grep output.
# grep output format: ./path/to/file:line:match
build_filter() {
    local filter="grep -v"
    for path in "${EXEMPT_PATHS[@]}"; do
        # Escape path for use in grep pattern (handle slashes)
        escaped="${path//\//\\/}"
        filter+=" -e \"${escaped}\""
    done
    echo "${filter}"
}

FAILURES=0

for i in "${!STALE_PATTERNS[@]}"; do
    pattern="${STALE_PATTERNS[$i]}"
    label="${PATTERN_LABELS[$i]}"

    # Collect all matches, then filter out exempt files
    hits=()
    while IFS= read -r line; do
        # Extract file path from "file:line:match" format
        filepath="${line%%:*}"
        # Normalize: strip leading "./"
        filepath="${filepath#./}"

        # Check if this file is exempt
        is_exempt=0
        for exempt in "${EXEMPT_PATHS[@]}"; do
            if [[ "${filepath}" == "${exempt}" ]]; then
                is_exempt=1
                break
            fi
        done

        if [[ "${is_exempt}" -eq 0 ]]; then
            hits+=("${line}")
        fi
    done < <(
        grep -rn --include="*.rs" --include="*.md" --include="*.toml" --include="*.proto" \
            -E "${pattern}" . 2>/dev/null || true
    )

    if [[ ${#hits[@]} -gt 0 ]]; then
        echo ""
        echo "ERROR: Found stale vocabulary '${label}':"
        for hit in "${hits[@]}"; do
            echo "  ${hit}"
        done
        FAILURES=$(( FAILURES + 1 ))
    elif [[ "${VERBOSE}" -eq 1 ]]; then
        echo "  ok: no stale '${label}' found"
    fi
done

echo ""
if [[ "${FAILURES}" -gt 0 ]]; then
    echo "FAIL: ${FAILURES} stale vocabulary pattern(s) found."
    echo "      Use canonical Round-14 names (RFC 0005 §14 changelog):"
    echo "        read_scene       → read_scene_topology"
    echo "        receive_input    → access_input_events"
    echo "        zone_publish:<z> → publish_zone:<z>"
    echo "        telemetry_read   → read_telemetry"
    echo ""
    echo "      If a file legitimately needs the legacy name (e.g., for rejection tests),"
    echo "      add it to the EXEMPT_PATHS list in scripts/check_canonical_vocabulary.sh."
    exit 1
else
    echo "PASS: canonical vocabulary check — no stale names found."
    exit 0
fi
