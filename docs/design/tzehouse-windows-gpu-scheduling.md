# tzehouse-windows GPU Scheduling Policy

**Bead**: hud-0mo6n
**Date**: 2026-04-25
**Author**: agent worker (claude-sonnet-4-6)
**Status**: ADOPTED
**Cross-references**: hud-ora8.1.20 (D18 runner setup, PR #595), real-decode workflow (`real-decode-windows.yml`), runbook (`docs/ci/windows-d18-runner-setup.md §7`)

---

## 1. Problem Statement

The tzehouse-windows box (`parrot-hen.ts.net`, RTX 3080) is dual-use:

| Workload | GPU use | Trigger |
|---|---|---|
| Nightly real-decode CI | Active — GStreamer D3D11 / NVDEC decoders | Cron 02:00 SGT (18:00 UTC) |
| Interactive `/user-test` overlay sessions | Active — wgpu compositor, `tze_hud.exe` | Human-initiated, daytime SGT |

Both workloads claim exclusive-ish GPU access. If they run concurrently:

- The CI job may fail with DXGI device-lost or CUDA OOM errors.
- The HUD overlay may stutter, freeze, or crash mid-session.
- GStreamer decode correctness tests may produce garbage output, causing silent false-passes or noisy false-fails.

PR #595 (commit 2292c32) landed the CI workflow with a best-effort 02:00 SGT scheduling window but explicitly left the hard-mutex gap open. This document closes that gap.

---

## 2. Candidate Approaches

### 2.1 File-based GPU lock (RECOMMENDED — see §3)

**Mechanism**: A well-known lock file at `%PROGRAMDATA%\tze_hud\gpu.lock` is:
- Created (with PID, timestamp, session type) when either workload starts.
- Removed when the workload exits cleanly.
- Checked by the CI workflow's pre-job step — if the lock exists and the PID is alive, the job skips with an annotated exit.
- Checked (optionally) by `tze_hud.exe` startup — if the lock signals a CI job, warn the user and offer to defer.

**Pros**:
- Simple — one `Test-Path` + `Get-Process -Id` check in PowerShell.
- No GitHub API calls or label mutations required.
- Works regardless of network connectivity.
- Lock state is observable with a single file read.
- Naturally self-healing: if `tze_hud.exe` crashes, the lock file becomes stale; the CI job can detect this via PID liveness check and proceed.

**Cons**:
- Stale lock files after an unclean crash require a liveness check (see §3.2).
- The lock is advisory — either process can ignore it if misbehaved or forked.
- Only works on the same machine; no cross-machine coordination.

### 2.2 GitHub Actions runner label rotation

**Mechanism**: Remove the `gpu` runner label before an interactive session starts, restore it after the session ends. The workflow only picks up the job if the runner has the `gpu` label.

**Pros**:
- CI jobs cannot be dispatched to the box at all while the label is absent — stronger than advisory.
- No inter-process communication needed.

**Cons**:
- Requires a GitHub token with `manage_runners` permission available on the Windows box.
- Label mutation is an API call; can fail if GitHub is unreachable.
- Does not prevent a job already queued (picked up before label was removed) from running.
- Adds operational complexity: if the session ends abnormally, someone must restore the label manually.
- PR-label-triggered jobs (the `run-real-decode` label path) bypass the runner-label guard — those would queue up and run as soon as the runner label is restored.

**Verdict**: Useful as a secondary kill-switch for planned maintenance windows, but not suitable as the primary mechanism for routine session protection.

### 2.3 Pre-job session check (workflow-only detection)

**Mechanism**: A pre-job workflow step calls a REST endpoint or checks a process/port to detect whether `tze_hud.exe` is running, then fails or skips the job if it is.

**Pros**:
- Self-contained within the workflow; no host-side daemon or lock file required.
- Can check real runtime state (port 50051 / 9090 liveness, or process list).

**Cons**:
- `Get-Process tze_hud` / `netstat` on the runner is fragile if the process name or port changes.
- A crashed-but-not-cleaned-up process can leave zombie socket listeners, causing false-positive deferrals.
- No corresponding guard on the interactive side — `tze_hud.exe` startup does not know a CI job is running.
- Does not protect against a session that starts _after_ the pre-job check passes but _before_ the GPU-intensive decode steps run.

**Verdict**: Useful as a secondary detection layer on top of the lock file, but insufficient alone.

### 2.4 Scheduled window enforcement only (current state)

The nightly cron at 02:00 SGT is chosen to minimize overlap with daytime interactive use. This is the current state and is not sufficient because:

- Interactive sessions can start at any time (late-night debugging, timezone overlap with collaborators).
- The nightly window cannot account for a session left running overnight.
- PR-label-gated runs can trigger at any hour.

**Verdict**: Retain the time window as a defence-in-depth measure but do not rely on it as the only control.

---

## 3. Recommendation: File-Based GPU Lock

### 3.1 Lock file specification

**Path**: `%PROGRAMDATA%\tze_hud\gpu.lock`

On tzehouse-windows this resolves to: `C:\ProgramData\tze_hud\gpu.lock`

**Format** (line-delimited text, UTF-8):

```
SESSION_TYPE=<ci|interactive>
PID=<process-id>
STARTED_AT=<ISO-8601 UTC timestamp>
DESCRIPTION=<human-readable reason>
```

Example — CI job holding the lock:

```
SESSION_TYPE=ci
PID=14872
STARTED_AT=2026-04-25T18:00:05Z
DESCRIPTION=real-decode-windows nightly run (github.run_id=1234567890)
```

Example — interactive HUD session holding the lock:

```
SESSION_TYPE=interactive
PID=9204
STARTED_AT=2026-04-25T13:22:11Z
DESCRIPTION=tze_hud.exe /user-test overlay session
```

### 3.2 Lock lifecycle

**Acquire**:
1. Create `C:\ProgramData\tze_hud\` directory if it does not exist.
2. If `gpu.lock` already exists: read the PID and check `Get-Process -Id <PID>`. If the process is not alive, the lock is stale — remove it and proceed. If the process is alive, respect the lock: skip or fail as appropriate (see §3.3, §3.4).
3. Write the lock file atomically (write to `gpu.lock.tmp`, rename to `gpu.lock`).

**Release**:
1. Read the lock file, verify the PID matches the current process.
2. Remove the file.
3. On unclean exit (crash, killed), the lock becomes stale. The next holder performs the liveness check in step 2 of the acquire sequence.

### 3.3 CI workflow behaviour (automated)

The pre-job step checks the lock:

- **Lock absent**: proceed normally. Acquire the lock. Release after the last real-GPU step.
- **Lock held by a live interactive session**: skip the entire job with exit code 0. Emit a GitHub Actions warning annotation:
  `::warning:: GPU lock held by interactive tze_hud session (PID=<pid>). Nightly job skipped. Re-run manually when the session is complete.`
- **Lock held by a stale process**: treat as absent. Remove the stale lock, acquire, proceed.
- **Lock file unreadable/corrupt**: treat as absent (defensive; log a warning).

If the job is skipped, GitHub Actions marks the run as a success (exit 0) to avoid spurious nightly failure alerts. The warning annotation is visible in the run summary.

### 3.4 Interactive session behaviour (semi-automated, requires Tzeusy action)

The `tze_hud.exe` process does not currently write a lock file. Until native lock support is added to the runtime (tracked as a follow-up — see §7), the workflow is:

**Before starting a /user-test session**:
1. Check `%PROGRAMDATA%\tze_hud\gpu.lock`. If it contains `SESSION_TYPE=ci`, a nightly job is running. Do not start the HUD session until the job completes or is cancelled in GitHub Actions.
2. Write the interactive lock manually (or via the `start-hud-session.ps1` helper — see §5.2).

**After ending a /user-test session**:
1. Remove `%PROGRAMDATA%\tze_hud\gpu.lock` (or use the `stop-hud-session.ps1` helper).

This is a human-action step until the runtime gains native lock support.

---

## 4. Failure Modes and Mitigations

| Failure | Effect | Mitigation |
|---|---|---|
| CI job crashes before releasing lock | Interactive session sees stale lock → blocked | PID liveness check: lock is released on next CI run or interactive start |
| `tze_hud.exe` crashes before releasing lock | Next CI run sees stale lock → blocked | Same PID liveness check |
| `C:\ProgramData\tze_hud\` directory does not exist | Lock acquire fails | `New-Item -Force` in both the pre-job step and the helper scripts |
| Operator forgets to write interactive lock | CI job runs concurrently with session | Defence-in-depth: 02:00 SGT window reduces (not eliminates) this risk |
| Lock file is on a different volume (path misconfiguration) | Two processes write to different locks → no mutual exclusion | Lock path is hardcoded and checked in the CI step; document it in the runbook |
| NTFS rename atomicity | Non-atomic write causes a corrupt lock file | Write-then-rename pattern; on corruption, treat as absent and log a warning |

---

## 5. Observability

### 5.1 CI run observability

Every lock-related decision in the CI pre-job step is logged to stdout with a structured prefix (`[gpu-lock]`), visible in the GitHub Actions run log:

```
[gpu-lock] Checking C:\ProgramData\tze_hud\gpu.lock ...
[gpu-lock] No lock file found. Proceeding.
[gpu-lock] Lock acquired (PID=14872). Writing C:\ProgramData\tze_hud\gpu.lock
[gpu-lock] Lock released.
```

Or on skip:

```
[gpu-lock] Lock file found: SESSION_TYPE=interactive PID=9204
[gpu-lock] Process 9204 is running. GPU is in use by an interactive session.
[gpu-lock] Skipping nightly real-decode job. See docs/design/tzehouse-windows-gpu-scheduling.md
::warning:: GPU lock held by interactive tze_hud session (PID=9204). Nightly job skipped.
```

### 5.2 PowerShell helper scripts

Two helper scripts are provided (see §6) to make the interactive-session side observable and scriptable:

- `scripts/ci/windows/gpu-lock-start.ps1` — acquires the lock for a session type.
- `scripts/ci/windows/gpu-lock-release.ps1` — releases the lock, verifying PID ownership.

Both scripts emit `[gpu-lock]` prefixed log lines and exit non-zero on unexpected conditions.

### 5.3 How to tell when a job was deferred and why

1. **GitHub Actions UI**: The run shows as "Success" with a yellow warning annotation visible on the job summary. Filter runs by the `::warning::` text.
2. **Run log search**: Search for `[gpu-lock]` in the workflow step log.
3. **Lock file inspection** (on the Windows box):
   ```powershell
   Get-Content "C:\ProgramData\tze_hud\gpu.lock" -ErrorAction SilentlyContinue
   ```

---

## 6. Implementation

### 6.1 Automated (in this PR)

- **Pre-job workflow step** added to `.github/workflows/real-decode-windows.yml`:
  - Checks for the lock file before any GPU work begins.
  - Skips the job (exit 0 + warning annotation) if a live interactive session holds the lock.
  - Acquires the CI lock on proceed.
  - A final `if: always()` step releases the lock (even on failure).
- **Helper scripts** at `scripts/ci/windows/`:
  - `gpu-lock-start.ps1` — for the interactive session side.
  - `gpu-lock-release.ps1` — for cleanup.
- **Updated workflow header** — the `DUAL-USE CAVEAT` note now points to this design doc.
- **Updated runbook §7** — replaces the "gap documented" stub with the actual policy.

### 6.2 Requires Tzeusy's action on the Windows host

The following steps cannot be automated from this agent context and require interactive Windows access:

1. **Create the lock directory** (one-time):
   ```powershell
   New-Item -ItemType Directory -Force "C:\ProgramData\tze_hud"
   ```
   (The CI pre-job step also creates it, but doing it manually ensures it exists before any CI run.)

2. **Use the helper scripts when starting/stopping interactive sessions**:
   ```powershell
   # Before starting tze_hud.exe for a /user-test session:
   C:\tze_hud\scripts\gpu-lock-start.ps1 -SessionType interactive -Description "user-test overlay session"
   
   # After the session ends:
   C:\tze_hud\scripts\gpu-lock-release.ps1
   ```
   These scripts must be deployed to tzehouse-windows alongside `tze_hud.exe`. The CI workflow does not deploy them — Tzeusy must copy them from the repo checkout or the D18 runner's checkout directory.

3. **Verify lock path is writable** by the GitHub Actions runner service account:
   ```powershell
   # Run as the runner service account (or check ACLs):
   icacls "C:\ProgramData\tze_hud" | Select-String "BUILTIN\Users"
   ```
   `%PROGRAMDATA%` is typically writable by all standard users, but verify this on tzehouse-windows.

4. **Add a PSK-aware tze_hud session wrapper** (optional, long-term):
   Until `tze_hud.exe` natively writes the lock, the helper scripts are the only mechanism for the interactive-session side. Integrating lock acquisition into the HUD startup sequence (e.g., via a wrapper `run_hud.ps1`) is tracked as a follow-up — see §7.

---

## 7. Follow-Up Work (out of scope for this bead)

The following items are discovered from implementing this policy and should become their own beads:

1. **Native lock support in `tze_hud.exe`**: The runtime should write and release the GPU lock file automatically at startup and shutdown. This removes the reliance on the manual helper script for interactive sessions and closes the gap where a user starts `tze_hud.exe` directly (not via the helper).

2. **Lock-aware `run_hud.ps1` wrapper**: A Windows PowerShell wrapper that checks for CI lock, acquires interactive lock, starts `tze_hud.exe`, and releases the lock on process exit. This is a thin shell — the right short-term fix before native runtime support is added.

3. **GitHub Actions workflow-level job cancellation on CI→interactive conflict**: Currently the CI job skips (exit 0). A more visible policy would cancel any currently-queued jobs when an interactive session starts. This requires runner-side GitHub API calls.

---

## 8. Decision Record

| Question | Decision |
|---|---|
| Primary mechanism | File-based GPU lock at `%PROGRAMDATA%\tze_hud\gpu.lock` |
| Secondary mechanism | Retain 02:00 SGT nightly window as defence-in-depth |
| CI-side enforcement | Automated: pre-job step in the workflow |
| Interactive-side enforcement | Manual: helper scripts; native support is a follow-up |
| On conflict: CI behaviour | Skip job (exit 0) + warning annotation |
| On conflict: interactive behaviour | Warn and defer (human decides) |
| Stale lock handling | PID liveness check; remove and proceed if dead |
| Observability | `[gpu-lock]` log prefix + GitHub warning annotation |
| Label rotation | Not primary; remains available as manual kill-switch |
