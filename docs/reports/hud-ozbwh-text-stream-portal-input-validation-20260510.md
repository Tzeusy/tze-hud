# Text Stream Portal Windows Input Validation - hud-ozbwh

Date: 2026-05-10
Host: `tzehouse-windows.parrot-hen.ts.net`
Branch: `agent/hud-ozbwh`

## Scope

Validate the remaining text-stream portal hit-region gap from `hud-eq1m4` /
PR #631: live focus, drag, and scroll events must reach the resident portal
through the real compositor/input path, with transcript evidence and visible
HUD screenshots. The pass intentionally avoided a broad redeploy and first
checked whether the existing Windows HUD target was reachable.

## Result

Blocked before portal launch: the Windows target was unreachable over
Tailscale/SSH during the validation window.

No resident portal was created in this run, so cleanup left no portal, tile, or
lease behind.

## Evidence

Reachability evidence is recorded in:

- `docs/evidence/text-stream-portals/hud-ozbwh-20260510/reachability.json`

Key observations:

- `tzehouse-windows.parrot-hen.ts.net` resolved to `100.87.181.125`.
- Local Tailscale source route was `100.99.218.4` via `tailscale0`.
- `tailscale status` listed `tzehouse-windows` as a Windows node, but
  `tailscale ping --timeout=5s` timed out.
- SSH to both validation users timed out even with explicit non-interactive
  flags and connection timeouts:
  - `hudbot@tzehouse-windows.parrot-hen.ts.net`
  - `tzeus@tzehouse-windows.parrot-hen.ts.net`
- Direct port checks for SSH `22`, gRPC `50051`, and MCP `9090` did not
  complete within the timeout window.
- The retry loop's recorded shell exit code reflects loop completion only; all
  individual Tailscale and SSH probes in that loop timed out, so the effective
  reachability result is failure.

## Commands

The worker context helper came from the local `beads-worker` skill installation
rather than a script tracked in this repository; placeholders below identify
the required inputs without baking in one contributor's home directory.

```bash
python3 "${BEADS_WORKER_SKILL}/scripts/assert_worker_context.py" \
  --worktree-path "${WORKTREE_PATH}" \
  --repo-root "${REPO_ROOT}" \
  --issue-id hud-ozbwh \
  --current-path "$(pwd -P)" \
  --branch "$(git branch --show-current 2>/dev/null || true)"
```

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=accept-new \
  -o UserKnownHostsFile="${TMPDIR:-/tmp}/tze_hud_known_hosts" \
  -o ConnectTimeout=8 -o ConnectionAttempts=1 \
  -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
```

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=accept-new \
  -o UserKnownHostsFile="${TMPDIR:-/tmp}/tze_hud_known_hosts" \
  -o ConnectTimeout=8 -o ConnectionAttempts=1 \
  -i ~/.ssh/ecdsa_home \
  tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
```

```bash
timeout 10 bash -lc \
  'nc -vz tzehouse-windows.parrot-hen.ts.net 22; \
   nc -vz tzehouse-windows.parrot-hen.ts.net 50051; \
   nc -vz tzehouse-windows.parrot-hen.ts.net 9090'
```

```bash
for i in 1 2 3; do
  echo retry=$i
  timeout 8 tailscale ping --timeout=5s tzehouse-windows.parrot-hen.ts.net || true
  timeout 8 ssh -o BatchMode=yes -o IdentitiesOnly=yes \
    -o StrictHostKeyChecking=accept-new \
    -o UserKnownHostsFile="${TMPDIR:-/tmp}/tze_hud_known_hosts" \
    -o ConnectTimeout=5 -o ConnectionAttempts=1 \
    -i ~/.ssh/ecdsa_home \
    tzeus@tzehouse-windows.parrot-hen.ts.net "whoami" || true
  sleep 10
done
```

## Acceptance Status

| Criterion | Status |
|---|---|
| Focus transcript evidence through real compositor/input path | Not run; target unreachable |
| Drag transcript evidence through real compositor/input path | Not run; target unreachable |
| Scroll transcript evidence through real compositor/input path | Not run; target unreachable |
| Visible HUD screenshots | Not captured; target unreachable |
| No orphaned portal remains | Pass; no portal was created |

## Residual Blocker

This run did not reach the input-path question because the Windows validation
host was not reachable. Once the host is reachable again, the smallest useful
validation path is:

1. Connect with the existing scheduled-task PSK and run
   `text_stream_portal_exemplar.py --phases baseline --baseline-hold-s 120`
   so a console operator can click the composer, drag the header, and wheel the
   output pane.
2. Capture a screenshot while the portal is mounted.
3. Confirm transcript counters for `input:focus-attempt`,
   `input:focus-gained`, `drag:start`, `drag:end`, and `scroll:output`.
4. Confirm `cleanup_errors=[]` and that lease release removes the portal.

If manual console input is not available, the follow-up implementation bead
should add a diagnostic input injector that enters the same windowed
compositor/input pipeline as physical pointer and wheel events. A draft issue
body is included in the reachability evidence JSON for the coordinator because
this worker was explicitly instructed not to mutate Beads lifecycle state.
