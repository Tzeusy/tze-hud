# Tasks

> **ARCHIVED 2026-04-18** — All tasks completed as part of epic hud-d0c8.
> Implementation in `.claude/skills/user-test/scripts/stress_test_zones.py`.
> Gen-1 reconciliation report: `docs/reports/mcp_stress_testing_d0c8_epic_report_20260418.md` (PR #487).
> Archive bead: hud-b99p.

## Task 1: Implement stress_test_zones.py

**File:** `.claude/skills/user-test/scripts/stress_test_zones.py`

Implement the MCP zone publish stress test script per the `mcp-stress-testing` spec.

**Subtasks:**
1. Scaffold CLI argument parsing (--url, --psk-env, --ssh-host, --ssh-key, --output, --duration, --profiles, --concurrency, --short-ttl, --large-payloads)
2. Implement `rpc_call()` (reuse pattern from `publish_zone_batch.py`)
3. Implement zone payload generators for all 6 media types with correct contention behavior:
   - StreamText for subtitle/alert-banner
   - KeyValuePairs for status-bar with rotating merge keys (key-0 through key-31, then reuse)
   - ShortTextWithIcon for notification-area (triggers Stack eviction at depth > 8)
   - SolidColor for pip/ambient-background
4. Implement network baseline phase (10x `list_zones`)
5. Implement publish baseline phase (10x idle `publish_to_zone`)
6. Implement load profile runner with `ThreadPoolExecutor` for concurrency > 1; rate limiting via `time.sleep` between dispatch batches
7. Implement SSH telemetry collector: background thread with `subprocess.Popen`, 1s sampling of `Get-Process` + `nvidia-smi`, 5s join timeout + `process.kill()` cleanup
8. Implement CPU% computation: delta total CPU seconds between consecutive samples / wall interval
9. Implement statistics computation (p50/p95/p99/max/mean/error rate/target vs achieved throughput per profile)
10. Implement `--short-ttl` flag (1s TTL) and `--large-payloads` flag (100B/1KB/10KB/60KB rotation)
11. Implement JSON report writer with time-series data (1s resolution buckets)
12. Implement stdout summary table formatter with target vs achieved rate columns
13. Add MCP reachability gate (exit 1 if endpoint unreachable before starting baselines)

**Acceptance criteria:**
- Script runs both baselines and all 5 load profiles, produces JSON report
- Each profile reports latency p50/p95/p99/max, error rate, target vs achieved throughput
- Host telemetry sampled via SSH with correct CPU% computation (delta, not cumulative)
- Concurrent profiles use ThreadPoolExecutor; merged latency stats across threads
- MergeByKey and Stack contention exercised with rotating keys / rapid publishes
- `--short-ttl` and `--large-payloads` flags work
- No pip dependencies (stdlib only)
- Handles SSH failure gracefully (telemetry-incomplete flag)
- SSH subprocess cleaned up via kill on timeout

**Estimated effort:** 120 minutes

## Task 2: Update bead hud-66q9 with spec references

Link the existing bead `hud-66q9` to the OpenSpec spec sections so the worker has direct references.

**Subtasks:**
1. Update `hud-66q9` description to reference `openspec/changes/mcp-stress-testing/specs/mcp-stress-testing/spec.md`
2. Add acceptance criterion: "Verify implementation matches spec requirements for load profiles, zone coverage, latency measurement, host telemetry, contention exercising, and report output"

**Estimated effort:** 10 minutes
