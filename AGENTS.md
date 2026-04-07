# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd onboard` to get started.

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd sync               # Sync with git
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION -->
## Issue Tracking with bd (beads)

**IMPORTANT**: This project uses **bd (beads)** for ALL issue tracking. Do NOT use markdown TODOs, task lists, or other tracking methods.

### Why bd?

- Dependency-aware: Track blockers and relationships between issues
- Git-friendly: Auto-syncs to JSONL for version control
- Agent-optimized: JSON output, ready work detection, discovered-from links
- Prevents duplicate tracking systems and confusion

### Quick Start

**Check for ready work:**

```bash
bd ready --json
```

**Create new issues:**

```bash
bd create "Issue title" --description="Detailed context" -t bug|feature|task -p 0-4 --json
bd create "Issue title" --description="What this issue is about" -p 1 --deps discovered-from:bd-123 --json
```

**Claim and update:**

```bash
bd update <id> --claim --json
bd update bd-42 --priority 1 --json
```

**Complete work:**

```bash
bd close bd-42 --reason "Completed" --json
```

### Issue Types

- `bug` - Something broken
- `feature` - New functionality
- `task` - Work item (tests, docs, refactoring)
- `epic` - Large feature with subtasks
- `chore` - Maintenance (dependencies, tooling)

### Priorities

- `0` - Critical (security, data loss, broken builds)
- `1` - High (major features, important bugs)
- `2` - Medium (default, nice-to-have)
- `3` - Low (polish, optimization)
- `4` - Backlog (future ideas)

### Workflow for AI Agents

1. **Check ready work**: `bd ready` shows unblocked issues
2. **Claim your task atomically**: `bd update <id> --claim`
3. **Work on it**: Implement, test, document
4. **Discover new work?** Create linked issue:
   - `bd create "Found bug" --description="Details about what was found" -p 1 --deps discovered-from:<parent-id>`
5. **Complete**: `bd close <id> --reason "Done"`

### Auto-Sync

bd automatically syncs with git:

- Exports to `.beads/issues.jsonl` after changes (5s debounce)
- Imports from JSONL when newer (e.g., after `git pull`)
- No manual export/import needed!

### Important Rules

- ✅ Use bd for ALL task tracking
- ✅ Always use `--json` flag for programmatic use
- ✅ Link discovered work with `discovered-from` dependencies
- ✅ Check `bd ready` before asking "what should I work on?"
- ❌ Do NOT create markdown TODO lists
- ❌ Do NOT use external issue trackers
- ❌ Do NOT duplicate tracking systems

For more details, see README.md and docs/QUICKSTART.md.

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds

<!-- END BEADS INTEGRATION -->

<!-- bv-agent-instructions-v1 -->

---

## Beads Workflow Integration

This project uses [beads_viewer](https://github.com/Dicklesworthstone/beads_viewer) for issue tracking. Issues are stored in `.beads/` and tracked in git.

### Essential Commands

```bash
# View issues (launches TUI - avoid in automated sessions)
bv

# CLI commands for agents (use these instead)
bd ready              # Show issues ready to work (no blockers)
bd list --status=open # All open issues
bd show <id>          # Full issue details with dependencies
bd create --title="..." --type=task --priority=2
bd update <id> --status=in_progress
bd close <id> --reason="Completed"
bd close <id1> <id2>  # Close multiple issues at once
bd sync               # Commit and push changes
```

### Workflow Pattern

1. **Start**: Run `bd ready` to find actionable work
2. **Claim**: Use `bd update <id> --status=in_progress`
3. **Work**: Implement the task
4. **Complete**: Use `bd close <id>`
5. **Sync**: Always run `bd sync` at session end

### Key Concepts

- **Dependencies**: Issues can block other issues. `bd ready` shows only unblocked work.
- **Priority**: P0=critical, P1=high, P2=medium, P3=low, P4=backlog (use numbers, not words)
- **Types**: task, bug, feature, epic, question, docs
- **Blocking**: `bd dep add <issue> <depends-on>` to add dependencies

### Session Protocol

**Before ending any session, run this checklist:**

```bash
git status              # Check what changed
git add <files>         # Stage code changes
bd sync                 # Commit beads changes
git commit -m "..."     # Commit code
bd sync                 # Commit any new beads changes
git push                # Push to remote
```

### Best Practices

- Check `bd ready` at session start to find available work
- Update status as you work (in_progress → closed)
- Create new issues with `bd create` when you discover tasks
- Use descriptive titles and set appropriate priority/type
- Always `bd sync` before ending session

<!-- end-bv-agent-instructions -->

# Notes to self

- `size_of::<Node>()` is tested against a 150-byte limit (scene-graph/spec.md line 302). Adding heap-allocated fields to `HitRegionNode` (which is a variant of `NodeData`) inflates `Node` inline. Box large optional structs (`AccessibilityMeta`, `LocalStyle`) to stay under budget.
- `gh pr merge <N> --squash --delete-branch` fails with "already checked out" when a worktree has the base branch checked out. The merge still succeeds via the API; the error is only about the local git cleanup. Verify with `gh pr view <N> --json state,mergedAt`.
- `beads-pr-reviewer-worker`: automated reviewer threads (Copilot, Gemini) are left by bots and count toward `UNRESOLVED_COUNT`; they must be replied to and resolved before merge just like human threads.
- When `HitRegionNode` gains new fields with `#[serde(default)]`, all existing struct literal constructions in tests and production code must add `..Default::default()`. Grep for `HitRegion(HitRegionNode {` across the workspace to find them all.
- `update_hover_state` should use `entry().or_insert_with()` rather than `get_mut` so that HitRegionNodes inserted directly into `self.nodes` (bypassing `set_tile_root`/`add_node_to_tile`) still get local state initialized on first hit.
- Zone ontology (rig-ar23): `ZoneDefinition` needs `#[serde(default)]` on `layer_attachment` and a `Default` impl returning `Content` so older serialized defs without the field still deserialize correctly.
- In `session_server.rs` tests, `LeaseStateChange` events can interleave before `MutationResult`/`RuntimeError` responses; add a `next_non_state_change()` helper that drains `LeaseStateChange` payloads before asserting.
- When `publish_to_zone` signature grows (e.g., adding `expires_at_wall_us`, `content_classification`), any main-branch wrapper that calls it (e.g., `publish_to_zone_with_lease`) will fail to compile — grep for all call sites.
- `tests/integration/multi_agent.rs` has pre-existing compilation failures (missing `auth_credential`, `max_protocol_version`, `min_protocol_version` on `SessionInit`, and `timing` on `MutationBatch`) unrelated to zone ontology; exclude with `--exclude integration` when running tests for zone work.
- Pre-existing test failures on `main` (tracked as hud-3m8h): `examples/vertical_slice/tests/budget_assertions.rs` (`test_transaction_validation_p99_within_budget`, `test_texture_upload_p99_within_budget`) and `tests/integration/v1_thesis.rs` (`test_v1_thesis_proof`). These cause CI `test-unit` and `test-v1-thesis` jobs to fail with UNSTABLE status.
- `tze_hud_protocol` requires `protoc` (protobuf-compiler) as a build dependency; GitHub-hosted runners don't include it by default. All CI jobs that compile Rust must install `protobuf-compiler` via apt before running cargo commands.
- Windows GUI launches over SSH as `hudbot` need an interactive desktop session; `qwinsta`/`query user` returning no sessions means detached `start`/`Start-Process` invocations exit quickly and no visible overlay appears. Foreground `ssh -tt ... vertical_slice.exe` can keep it alive only while the SSH session remains open.
- Windows OpenSSH pubkey auth for a non-admin user (e.g., `hudbot`) can fail with `Failed publickey`/`Authentication refused` even when key material matches if `C:\Users\<user>\.ssh` or `authorized_keys` is owned by another account. Fix by setting owner to the target user and ACLs to only `<user>`, `SYSTEM`, and `Administrators`, then restart `sshd`.
- `RenderingPolicy::from_zone_policy()` margin fallback: `margin_horizontal`/`margin_vertical` must fall back to `margin_px` (via `.or(policy.margin_px)`), not hardcoded 8.0 — per spec §Extended RenderingPolicy "when None, falls back to margin_px". Missing this fallback is a spec compliance bug.
- `update_zone_animations` (PR #260) only iterates `current_active` (zones in `active_publishes`). Zones removed from registry entirely (unregistered) disappear from that map, so fade-out on unregistration must be handled by pruning stale animation states for absent zones after the main loop.
- Runtime hover/tooltip behavior is now widget-definition-driven: `WidgetDefinition.hover_behavior` declares trigger rect + delay + target f32 param, `windowed.rs` builds generic trackers via `widget_hover.rs`, and local hover writes MUST use `SceneGraph::set_widget_param_local` (not `publish_to_widget`) to avoid polluting contention/publication state.
- Local `bd` CLI in this repo currently does not support `bd sync` (returns `unknown command "sync"`). End-of-session sync should use explicit git pull/rebase + push flow instead.
- Notification profile styling only applies when the deployed `tze_hud.toml` includes both `[component_profile_bundles] paths = ["profiles"]` and `[component_profiles] notification = "notification-stack-exemplar"`, and the `profiles/notification-stack-exemplar/` directory is copied beside the exe on Windows.
- `user-test` notification coverage now has a dedicated full-gamut batch file at `.claude/skills/user-test/scripts/notification-full-gamut.json` (urgency 0-3, two-line title/body, long-body containment, and action-button rows); `SKILL.md` references it under "Notification Full-Gamut Pass".
