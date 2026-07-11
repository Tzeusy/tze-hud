# Live verification — `attach_parent_console()` rebinds Rust stdio on real Windows

**Bead:** hud-xssgy (live-Windows verification of a hud-8a2s0/#1143-review-flagged doubt)
**Date:** 2026-07-11
**Host:** autonomous `windows-vm.example` HUD testhost (Proxmox, software GPU / WARP-Vulkan)
**Binaries:** both built fresh from current `main`, commit `3b22565d` (hud-q2glv/#1143, hoisted
`attach_parent_console()`), `x86_64-pc-windows-gnu` release:
- `tze_hud_xssgy_prefix.exe` — unmodified baseline. sha256 `fde34ee6…e6510c`.
- `tze_hud_xssgy_noattach.exe` — negative control; only `attach_parent_console()`'s single
  production call site (top of `main()`) commented out, `#[allow(dead_code)]` added to keep the
  now-unused function compiling. sha256 `01c80bc2…d3f388`.

(Full hashes in `logs/exe.sha256.txt`.)

## What was verified

Whether `AttachConsole(ATTACH_PARENT_PROCESS)` alone (no `CONOUT$`/`CONIN$` reopen, no
`SetStdHandle`) is sufficient to make Rust's `println!`/`eprintln!` output reach the launching
terminal for this specific binary — the primary scenario the whole hud-b7c0m/#1112 →
hud-41q9t/#1140 → hud-q2glv/#1143 fix chain targets, and the one none of those PRs' own
verification (compile/link only) actually confirmed.

## Test matrix

All cells: direct invocation, **no output redirection**, real interactive ConPTY-backed console
session (see README "Method"). Three scenarios × two shells:

| Scenario | PowerShell | cmd.exe |
|---|---|---|
| `--version` | tested | tested |
| `--help` | tested | tested |
| invalid flag (`--this-flag-does-not-exist`, error path) | tested | tested |

## Per-scenario verdicts — baseline (`tze_hud_xssgy_prefix.exe`)

| # | Scenario | Shell | Verdict | Evidence |
|---|---|---|---|---|
| 1 | `--version` prints `tze_hud 0.1.0 (3b22565d)` | PowerShell | **PASS** | `logs/matrix-prefix-console.txt` lines ~7-10 |
| 2 | `--version` prints `tze_hud 0.1.0 (3b22565d)` | cmd.exe | **PASS** | same file, `MARK-CMD-VERSION-*` block |
| 3 | `--help` prints full usage text (all flags, NOTES section) | PowerShell | **PASS** | same file, `MARK-PS-HELP-*` block |
| 4 | `--help` prints full usage text | cmd.exe | **PASS** | same file, `MARK-CMD-HELP-*` block |
| 5 | Invalid flag prints `error: unknown flag: --this-flag-does-not-exist` + hint | PowerShell | **PASS** | same file, `MARK-PS-BADFLAG-*` block |
| 6 | Invalid flag prints the same error | cmd.exe | **PASS** | same file, `MARK-CMD-BADFLAG-*` block |

All six cells: the printed text is correct and complete (matches the exact commit hash of the
deployed binary, the exact flag list, and the exact error wording from the source), not garbled
or partial.

## A/B negative control (`tze_hud_xssgy_noattach.exe`)

Identical matrix, identical commit, only `attach_parent_console()`'s call disabled:

| # | Scenario | Shell | Verdict | Evidence |
|---|---|---|---|---|
| 1-6 | All six scenarios above | both | **FAIL (as expected)** — completely silent between the `MARK-*-START`/`MARK-*-END` markers in every case | `logs/matrix-noattach-control.txt` |

This is the control that makes the baseline result trustworthy rather than a methodology
artifact: the SAME capture pipeline, SAME commit, SAME shells, only the one call disabled —
output goes from fully-visible-and-correct to fully-silent. `attach_parent_console()` is
confirmed as the causal mechanism, not a coincidence of how the SSH/ConPTY session was captured.

## Methodology pitfall (caught before it could produce a false negative)

The first two capture attempts (`ssh -tt host 'cmd.exe /c "tze_hud.exe --version"'`, and the same
wrapped in `echo BEFORE-MARKER && ... && echo AFTER-MARKER`) produced **empty output** even for a
bare `echo HELLO-WORLD-TEST` control with no `tze_hud.exe` involved at all — proving the capture
pipeline itself was broken (ConPTY renders asynchronously; a single-shot `cmd.exe /c` command
completes and tears the SSH session down before the render pipe flushes to the wire), not that
output was missing. Switched to an interactive PTY session (`ssh -tt host` with no command, PS
default shell) fed line-by-line via stdin with a 1.2s pacing delay between lines and a trailing
settle delay before `exit` — this is what `run_vm_test.sh` (referenced in the bead's own
worktree, not committed here) actually used, and it is what produced the transcripts in this
directory. Flagging this because a naive one-shot SSH command would have reported a false
"broken" verdict for a mechanism that actually works.

## Likely root cause of why `AttachConsole` alone suffices here

The classic C/C++ "hybrid console/GUI app" pattern pairs `AttachConsole` with
`freopen("CONOUT$", "w", stdout)` (+ `CONIN$`/`stderr`) because the C runtime's buffered `FILE*`
streams bind to whatever `GetStdHandle` returns **at CRT init time**, which can run before (or
independent of) any `AttachConsole` call the program makes in `main`. Rust's
`std::io::Stdout`/`Stderr` do not go through the C runtime's `FILE*` layer at all — they resolve
`GetStdHandle` **lazily, on first use**, and in this binary that first use (the structured-logging
`tracing_subscriber::fmt().init()` call, or the first `println!`/`eprintln!`) happens strictly
after `attach_parent_console()` already ran (it is the literal first statement of `main()`). By
the time Rust actually asks for the standard handle, the process is already attached to the
parent's console, so `GetStdHandle` resolves correctly without needing an explicit reopen. This
is consistent with, and explains, the empirical result above; it was not independently verified
against Rust's standard-library source in this pass (no local `rust-src` component available) —
treat it as the most likely explanation, not a proven mechanism.

## Conclusion

No code fix needed. `attach_parent_console()`'s doc comment (`app/tze_hud_app/src/main.rs`) was
updated to record this empirical result so the next reader does not have to re-litigate the
theoretical doubt this bead was filed to resolve.
