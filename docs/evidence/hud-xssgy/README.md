# liveverify — `attach_parent_console()` actually rebinds Rust stdio (2026-07-11, hud-xssgy)

Live evidence, from the autonomous `windows-vm.example` HUD testhost, that
`attach_parent_console()` (`app/tze_hud_app/src/main.rs`, hoisted to the first
statement of `main()` by hud-q2glv / PR #1143) makes Rust's `println!`/`eprintln!`
output actually appear in the launching terminal — not just that the FFI links
compiled (PR #1112's original verification was compile/link-only).

**Headline result: it works.** No code fix was needed. `AttachConsole(ATTACH_PARENT_PROCESS)`
alone — with no `CONOUT$`/`CONIN$` reopen or `SetStdHandle` call, which the classic C/C++
hybrid console/GUI pattern normally requires — is sufficient for this Rust binary. See
`VERDICTS.md` for the reasoning and the A/B negative control that rules out a test-methodology
false positive.

## Background

Filed from the hud-q2glv / PR #1143 review (wrk-n95zc): `AttachConsole` binds the process to the
parent's console *object*, but per documented Windows behavior does not by itself rebind
`GetStdHandle(STD_OUTPUT/ERROR/INPUT_HANDLE)` — which is what Rust's `println!`/`eprintln!`
actually read. No `CONOUT$`/`CONIN$` reopen exists anywhere in this codebase. This bead
empirically settles whether that theoretical gap is a real defect on live Windows.

## Contents

| Path | What |
|------|------|
| `VERDICTS.md` | Per-scenario PASS verdicts, the A/B negative-control table, and the likely root-cause explanation |
| `run_vm_test.sh` | Reproduce a matrix run against a given deployed exe filename |
| `test-matrix-input.txt` | The literal PowerShell/cmd.exe command sequence fed to the VM session |
| `logs/matrix-prefix-console.txt` | Full captured transcript: pre-fix baseline exe (`attach_parent_console()` present, commit 3b22565d), `--version`/`--help`/invalid-flag, in both PowerShell and `cmd.exe`, no redirection |
| `logs/matrix-noattach-control.txt` | Same matrix, same commit, with the only production call site of `attach_parent_console()` commented out — the negative control |
| `logs/exe.sha256.txt` | sha256 of both deployed exes |

## Reproduce

```bash
# 1) Build the exe (baseline, current main):
cargo build --release --target x86_64-pc-windows-gnu -p tze_hud_app

# 2) Resolve host + auth (self-heals the VM):
.claude/skills/user-test/scripts/hud_vm_env.sh > /tmp/hud-xssgy-env.sh

# 3) Deploy to a scratch dir that does NOT touch the production scheduled task:
source /tmp/hud-xssgy-env.sh
ssh -o BatchMode=yes -o ConnectTimeout=10 -i ~/.ssh/hud-ssh-key admin-user@$TZE_HUD_TEST_HOST 'mkdir C:\tze_hud_test' 2>/dev/null || true
scp -o BatchMode=yes -o ConnectTimeout=10 -i ~/.ssh/hud-ssh-key target/x86_64-pc-windows-gnu/release/tze_hud.exe \
  "hud-user@${TZE_HUD_TEST_HOST}:C:/tze_hud_test/tze_hud_test.exe"

# 4) Run the matrix:
docs/evidence/hud-xssgy/run_vm_test.sh tze_hud_test.exe /tmp/matrix-out
cat /tmp/matrix-out.txt

# 5) For the A/B control: comment out the sole `attach_parent_console();` call
#    at the top of main() in app/tze_hud_app/src/main.rs, add
#    #[allow(dead_code)] to the (now Windows-only-unused) function, rebuild,
#    deploy under a different remote filename, rerun step 4, then revert.
```

## Method

The target scenario ("user runs `tze_hud --version` from a real interactive shell, no
redirection") cannot be reproduced by a plain non-interactive `ssh host "command"` — OpenSSH
does not allocate a pseudo-console for a non-pty session, which would silently test the
*already-redirected-handle* case instead (irrelevant — `AttachConsole` never touches an
already-set handle, so that case was never in doubt). Instead: `ssh -tt` forces a genuine
ConPTY-backed console session on the Windows host; commands were fed to that interactive session
line-by-line over stdin with a pacing delay (ConPTY renders asynchronously — a single-shot
`cmd.exe /c "..."` command tears the SSH session down before the render pipe flushes, producing
a false-negative silent capture; this was caught and corrected before it could contaminate the
verdict — see `VERDICTS.md` "Methodology pitfall"). Every test uses direct invocation
(`& 'C:\...\tze_hud.exe' --version` / `tze_hud.exe --version`) — no `>`, no piping.

Two exes were built from the exact same source (current `main`, commit `3b22565d`, the
already-merged hud-q2glv/#1143 hoisted-attach code): the unmodified baseline, and a negative
control with only `attach_parent_console()`'s single production call site commented out. Running
the identical matrix against both isolates the call as the causal variable.

## Hygiene

Placeholders only: `windows-vm.example` for the real host IP (scrubbed from all committed logs).
No PSK appears in any captured output — the `--psk`/`PSK` substrings present are static `--help`
text (`[default: tze-hud-key]`, a public placeholder default), not a real secret; none of these
tests touch MCP/gRPC or the resident-principal PSK at all. Test artifacts (`C:\tze_hud_test\`)
were removed from the host after the run; the production `TzeHudFullscreen` scheduled task and
its `C:\tze_hud\tze_hud.exe` were never touched.
