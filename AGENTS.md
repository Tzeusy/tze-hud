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

- `docs/reports/validation_operations_extraction_decision_20260425.md` records the decision to extract the v1 carry-forward validation-operations backlog into a standalone OpenSpec change before canonical sync; the v2 delta is only a temporary staging location for that backlog.
- `.codex/skills` should remain a symlink to `../.claude/skills`; `.claude/skills` is the canonical tracked tree and currently strictly supersets the Codex mirror.
- `size_of::<Node>()` is tested against a 150-byte limit (scene-graph/spec.md line 302). Adding heap-allocated fields to `HitRegionNode` (which is a variant of `NodeData`) inflates `Node` inline. Box large optional structs (`AccessibilityMeta`, `LocalStyle`) to stay under budget.
- `gh pr merge <N> --squash --delete-branch` fails with "already checked out" when a worktree has the base branch checked out. The merge still succeeds via the API; the error is only about the local git cleanup. Verify with `gh pr view <N> --json state,mergedAt`.
- `beads-pr-reviewer-worker`: automated reviewer threads (Copilot, Gemini) are left by bots and count toward `UNRESOLVED_COUNT`; they must be replied to and resolved before merge just like human threads.
- When `HitRegionNode` gains new fields with `#[serde(default)]`, all existing struct literal constructions in tests and production code must add `..Default::default()`. Grep for `HitRegion(HitRegionNode {` across the workspace to find them all.
- `update_hover_state` should use `entry().or_insert_with()` rather than `get_mut` so that HitRegionNodes inserted directly into `self.nodes` (bypassing `set_tile_root`/`add_node_to_tile`) still get local state initialized on first hit.
- Zone ontology (rig-ar23): `ZoneDefinition` needs `#[serde(default)]` on `layer_attachment` and a `Default` impl returning `Content` so older serialized defs without the field still deserialize correctly.
- In `session_server.rs` tests, `LeaseStateChange` events can interleave before `MutationResult`/`RuntimeError` responses; add a `next_non_state_change()` helper that drains `LeaseStateChange` payloads before asserting.
- When `publish_to_zone` signature grows (e.g., adding `expires_at_wall_us`, `content_classification`), any main-branch wrapper that calls it (e.g., `publish_to_zone_with_lease`) will fail to compile — grep for all call sites.
- When proto publish messages gain new fields (for example `ZonePublish.element_id`, `WidgetPublish.element_id`, or `PublishToZoneMutation.element_id`), `prost` struct literals in both `src/session_server.rs` tests and `crates/tze_hud_protocol/tests/*.rs` must add explicit defaults (typically `element_id: Vec::new()`), or test-target compilation fails even if library code builds.
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
- Full-gamut Windows user-tests depend on `production.toml` loading `[widget_bundles] paths = ["widget_bundles"]`; deploy must include `C:\tze_hud\widget_bundles\gauge`, `progress-bar`, and `status-indicator`. Having bundles only under `C:\tze_hud\widgets\...` leaves widget instances unavailable.
- `.claude/skills/user-test/scripts/publish_widget_batch.py` can exit `0` even when a `published[*].response.error` payload is present (e.g., `WIDGET_PARAMETER_INVALID_VALUE` validation fixture); treat response-body errors as test outcomes, not process-exit outcomes.
- `/user-test` widget runs can clear durable stale UI with `.claude/skills/user-test/scripts/widget-cleanup.json`; `publish_widget_batch.py --cleanup-on-exit` clears touched widget instances on normal, error, and KeyboardInterrupt paths.
- Runtime widget cleanup contract: clearing/TTL expiry removes active publications, refreshes `WidgetInstance.current_params` from remaining publications/defaults, and the compositor only draws widgets with active publications; default params alone should not leave a visible stale widget.
- `/user-test` Windows SSH/SCP calls should pass `-i ~/.ssh/ecdsa_home` explicitly; this host currently has no `~/.ssh/config` stanza for `tzehouse-windows.parrot-hen.ts.net`, so default identity resolution fails for both `hudbot` and `tzeus`.
- `bd dep add <epic> <task> --type blocks` fails (`epics can only block other epics`). For epic PR review flow, create the `pr-review-task` bead independently (no `blocks` dep from epic), keep the epic blocked with `external_ref=gh-pr:<N>`, and resolve epic closure from PR merge state + dependent child status.
- Policy-planning and reconciliation must account for three policy surfaces, not two: direct runtime enforcement (`crates/tze_hud_runtime`), pure evaluators in `crates/tze_hud_policy`, and the separate scene-side contract in `crates/tze_hud_scene/src/policy/`. Ownership matrices that omit the scene-side layer are incomplete.
- Lease governance review: treat lease capability scope as a live seam separate from mid-session `CapabilityRequest` semantics. `LeaseRequest` handling must not allow a lease capability set broader than the session-granted authorization set; check this before broad policy-wiring work.
- Lease scope enforcement invariant: if any `LeaseRequest.capabilities` entry is outside the session's current `StreamSession.capabilities`, deny the entire lease request (`deny_code=PERMISSION_DENIED`) rather than clamping to a subset.
- Capability escalation source semantics: `session.policy_capabilities` must come from the full config-derived authorization scope (`agent_capabilities` / fallback mode), not from currently held grants. Mid-session `CapabilityRequest` evaluates against that scope, while `LeaseRequest` capability lists must stay within `session.capabilities` and deny with `PERMISSION_DENIED` when they exceed it.
- `docs/reconciliations/policy_wiring_seam_contract.md` is the canonical PW-02 seam artifact; policy-wiring implementation/reconciliation beads should use it as the source of truth for level input provenance, ownership boundaries, and PolicyContext/ArbitrationOutcome contracts.
- `crates/tze_hud_protocol::session_server` test helpers now rely on `handshake()` requesting both `create_tiles` and `modify_own_tiles` by default; policy-gated mutation tests that need narrower scopes must override capabilities explicitly.
- Mutation-path pilot latency conformance now lives in `crates/tze_hud_telemetry/src/validation.rs` under `evaluate_policy_mutation_latency_conformance` with budget constant `POLICY_MUTATION_EVAL_BUDGET_US = 50`; `session_server` policy-admission logs emit the structured conformance payload, and future policy telemetry work should extend that harness instead of inventing a parallel metric.
- Worktree `.beads/dolt-server.port` can get corrupted (e.g., concatenated to `428893307` instead of `3307`) when a worktree is created while the port file is written. Symptom: `bd` commands fail with "database not found on Dolt server at 127.0.0.1:42889". Fix: `printf '3307' > <worktree>/.beads/dolt-server.port`.
- `beads-pr-reviewer-worker`: the `list_review_threads.py` helper may return 0 threads even when threads exist; always verify with `evaluate_merge_readiness.py` or `gh api repos/.../pulls/<N>/comments` before treating unresolved count as zero.
- `openspec/.../policy-arbitration/spec.md` still marks Policy Telemetry / Arbitration Telemetry Events / Capability Grant Audit as `v1-mandatory`, but runtime telemetry currently uses `tze_hud_runtime::channels::TelemetryRecord` without policy fields; treat this as an active spec-to-runtime reconciliation seam for closeout work.
- Epic report scaffolding lives at `scripts/epic-report-scaffold.sh`; it must normalize both object-root and array-root payloads from `bd show --json`.
- `.claude/skills/user-test/scripts/hud_grpc_client.py` now exposes resident-flow helpers for avatar PNG creation + content-addressed ResourceIds, Presence Card tile sequencing, and explicit graceful vs hard disconnect paths; the avatar hash helper falls back to a cached cargo-built BLAKE3 binary when Python `blake3` is unavailable.
- `.claude/skills/user-test-performance/scripts/perf_common.py` owns the `results.csv` schema and now auto-migrates existing CSV headers during append; add new audit fields there first, then wire both MCP/gRPC scripts. `grpc_widget_publish_perf.py` now imports vendored stubs from `user-test-performance/scripts/proto_gen/` (self-contained, no `/user-test` path dependency).
- `WidgetPublishResult.request_sequence` is now present in the proto (field 5), runtime handler (`crates/tze_hud_protocol/src/session_server.rs`), and active OpenSpec (`openspec/specs/session-protocol/spec.md:774`); the contract drift from RFC 0005 was resolved via hud-as4t / PR #486. The Rust publish-load harness (`examples/widget_publish_load_harness/`) is the canonical gRPC widget benchmark path; `/user-test-performance` routes gRPC widget benchmarks through it.
- Current `tze_hud.exe` strict startup rejects the default PSK. Any Windows `/user-test` or scheduled-task launch must pass a non-default `--psk` (or set `TZE_HUD_PSK`) explicitly; the stock `run_hud.ps1`/`TzeHudOverlay` launch shape without a PSK will start the task but not bring up the HUD/MCP server.
- `bd ready --json --limit 0` can omit open `feature` beads in this repo’s default view; use explicit type filters (for example `bd ready --type feature --json --limit 0`) during coordinator dispatch so ready feature work isn’t skipped.
- `crates/tze_hud_protocol/proto/session.proto` currently has no resident `UploadResource`/`ResourceUploadStart` client message; raw-tile `/user-test` Python scenarios can drive tiles and node mutations over `HudSession`, but `StaticImageNode` upload still needs a separate helper or transport.
- RFC 0011 already defines `ResourceErrorResponse` and says chunked uploads get a runtime-assigned `upload_id`, but it still lacks a clean start-ack message that returns that `upload_id`; resident scene-resource upload is therefore a spec-contract seam first, not just an unimplemented handler.
- The canonical Windows `app/tze_hud_app/config/production.toml` runs ad hoc resident gRPC agents as guests (no tile/lease caps); Presence Card `/user-test` needs temporary `[agents.registered.<agent>]` grants or another explicitly authorized runtime config.
- Overlay `/user-test` sizing must keep runtime config, compositor surface, and `SceneGraph.display_area` in sync. Resident exemplar scripts should prefer the `SceneSnapshot.display_area` dimensions over hardcoded 1920x1080 drag/placement bounds.
- The current Presence Card exemplar contract is the expanded interactive glass variant: 320x112 tile, 24px left/bottom margins, 12px vertical gaps, `InputMode::Capture`, and a 13-node flat stack (background root plus sheen, accent rail, avatar plate, avatar, eyebrow, name, status, chip background, chip text, dismiss background, dismiss label, dismiss hit region). Any remaining 200x80, 3-node, or `Passthrough` assumptions are stale.
- Exemplar OpenSpec change directories can have all task checkboxes left unchecked even after implementation lands; use reconciliation/coverage docs plus live/manual-review artifacts as the closure signal, not `tasks.md` counts alone.
- `openspec archive <change> --yes` updates `openspec/specs/`, but if a change with ADDED-only deltas was already synced earlier (for example `exemplar-notification`), rerun archive with `--skip-specs` to avoid duplicate-requirement failures.
- Text-stream-portals phase-0 intentionally avoids new portal-specific proto RPCs; transport-agnostic adapter proof currently lives in integration tests (`text_stream_portal_adapter.rs`) over the existing primary `HudSession` stream.
- Text-stream portal composer text currently uses a full-span `TextMarkdownNode.color_runs` marker (same color as base text) to request literal monospace rendering through proto conversion; normal printable input should come from runtime character events, with key-down fallback only for Space on the Windows path.
- `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` releases its lease on exit by default so portal tiles disappear immediately after normal exit/Ctrl-C; use `--leave-lease-on-exit` only for explicit orphan/grace-path tests.
- `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` may run manual input handling concurrently with scripted phases; keep `set_tile_root` + `add_node` sequences serialized with the shared mutation lock or `add_node` can target a stale server-assigned root.
- Text-stream portal caret/input checks now have deterministic local and live paths: `text_stream_portal_exemplar.py --self-test` validates Space fallback + wrap/caret math without gRPC, and `--phases composer-smoke` renders `hello world` plus a long markdown-like paste on the live HUD with transcript metrics.
- Text Stream Portal minimized-icon restore should be pointer-down driven; current gRPC hit-region input can drop `pointer_up` for icon gestures, leaving click-vs-drag state stuck until an idle watchdog fires.
- Current Windows `/user-test` MCP JSON-RPC publishes should use `http://tzehouse-windows.parrot-hen.ts.net:9090/mcp`; the base `:9090` URL can return an empty/non-JSON response that makes `publish_*_batch.py` fail with `Expecting value`. The deployed `C:\tze_hud\tze_hud.toml` currently grants `agent-alpha/beta/gamma` only `create_tiles`, `modify_own_tiles`, and `access_input_events`; simply appending `upload_resource` to those caps made `tze_hud.exe` exit during startup, so restore the timestamped backup before further Presence Card/Dashboard Tile live runs.
- When extracting the Windows HUD PSK from `TzeHudOverlay` over SSH for resident gRPC user-tests, strip PowerShell `\r` output before setting `TZE_HUD_PSK`; otherwise auth fails with an apparent PSK mismatch.
- If `bd` reports `database "hud" not found` after Dolt recovery, `bd bootstrap` may import into the SQL server's exposed `dolt` database rather than creating `hud`. Temporary recovery is `bd dolt set database dolt` plus `bd import .beads/issues.jsonl`; restore `.beads/metadata.json` before committing.
- The local Beads Dolt store currently has no configured `origin`; `bd dolt push` fails with `remote 'origin' not found`. Session closeout should still run normal `git pull --rebase && git push`, but Beads DB pushes require configuring a Dolt remote first.
- If `/user-test` needs the visible Windows overlay and `TzeHudOverlay` runs without gRPC listening, recreate it with `schtasks /Create /F /TN TzeHudOverlay /SC ONCE /ST 23:59 /IT /RL HIGHEST /TR "C:\tze_hud\tze_hud.exe --config C:\tze_hud\tze_hud.toml --window-mode overlay --grpc-port 50051 --mcp-port 9090 --psk <psk>"`; this launches in console session 1 with both ports bound.
- `openspec/changes/cooperative-hud-projection/` defines `/hud-projection` as cooperative opt-in for already-running LLM sessions: no PTY attachment, no terminal capture, daemon owns durable transcript/inbox/HUD state outside token context, and the LLM session publishes/polls/acks through a provider-neutral contract.
- As of `hud-ggntn.8`, `crates/tze_hud_projection` ships `tze_hud_projection_authority`, a daemon-local stdio CLI surface for cooperative HUD projection operations; it retains state only for the process lifetime and emits a bounded `ProjectionResponse` plus newly written audit records for each JSON-line operation.
- Cooperative HUD projection resident adapter code lives behind the `tze_hud_projection` `resident-grpc` feature (`crates/tze_hud_projection/src/resident_grpc.rs`); it is daemon-side glue that emits existing `HudSession` raw-tile mutations and lease release messages, not a replacement for the authority/control surface.
- After `hud-ggntn.9`, the remaining cooperative projection sync/archive blocker is live governance validation unless explicitly waived.
- Pixel-readback tests for display-relative canonical scenes must sample display-relative coordinates too; stale fixed 800x600-era samples can hit the clear background (`[63, 63, 89, 255]`) on the 1920x1080 runtime and look like z-order/contention failures.
- Windows perf baseline `hud-1753c`: `docs/reports/windows_perf_baseline_2026-05.md` records the first reference-hardware pass. The deployed `C:\tze_hud\tze_hud.toml` is not benchmark-ready for live widget publishing because benchmark agents lack `publish_widget:*` caps; `examples/benchmark` also does not sample `input_to_local_ack` because it injects no input events.
- `openspec validate <change> --strict` rejects proposal/design/tasks-only change directories; even scope/bookkeeping changes need at least one `specs/<capability>/spec.md` delta with requirement/scenario blocks.
- `docs/audits/statig-state-machine-audit.md` records the E26 library audit: `statig` is acceptable for internal media/embodied state machines only behind project-owned protobuf mirror enums, with generated macro state kept private and no reactivation of deferred v2 work.
- Windows `/user-test` synthetic input: direct SSH `SetCursorPos` runs in a non-interactive context and may report `0,0`; an interactive scheduled task under `tzeus` can move the console cursor, but HUD scene coordinates may be 1.5x Windows input coordinates (e.g. HUD `3840x2160` vs OS `2560x1440`). Cursor movement alone is not proof that hit-region events reached the HUD.
