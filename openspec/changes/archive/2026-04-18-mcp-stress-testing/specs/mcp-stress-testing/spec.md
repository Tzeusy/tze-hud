# MCP Stress Testing

External load testing capability for the MCP HTTP `publish_to_zone` endpoint. Exercises zone publishing at configurable load levels while collecting latency and host resource telemetry.

## ADDED Requirements

### Requirement: Baseline Phases
Before load profiles begin, the tool SHALL run two baseline phases:

1. **Network baseline** — 10 `list_zones` calls (read-only, no scene lock contention). Isolates pure network round-trip time.
2. **Publish baseline** — 10 `publish_to_zone` calls at idle rate (1/s) across zones. Establishes single-publish latency without concurrent load.

Both baselines SHALL report p50/p95/p99/max latency and be included in the JSON report as `network_baseline` and `publish_baseline` respectively.

### Requirement: Load Profiles
The stress test tool SHALL support five sequential load profiles:

| Profile | Target Rate | Concurrency | Duration | Description |
|---------|------------|-------------|----------|-------------|
| idle | 1 req/s | 1 | 30s | Baseline single-publish cadence |
| low | 5 req/s | 1 | 30s | Light multi-zone publishing |
| medium | 20 req/s | 4 | 30s | Moderate sustained load |
| high | 50 req/s | 8 | 30s | Heavy sustained load |
| burst | 100 req/s | 16 | 10s | Short spike to find ceiling |

For profiles with concurrency > 1, the tool SHALL use `concurrent.futures.ThreadPoolExecutor` to dispatch requests in parallel. This models the realistic scenario of multiple LLM agents publishing simultaneously.

Profile duration, target rate, and concurrency SHALL be configurable via CLI arguments. A `--concurrency` flag SHALL override per-profile defaults. A 3-second cooldown SHALL separate each profile.

The tool SHALL report both **target rate** and **achieved rate** per profile. If the achieved rate is below the target, this indicates the throughput ceiling has been reached.

### Requirement: Zone Coverage and Contention
Each publish SHALL round-robin across all 6 default zones using the correct media type payload:

| Zone | Media Type | Contention | Example Payload |
|------|-----------|------------|----------------|
| subtitle | StreamText | LatestWins | `"Stress test message N"` |
| status-bar | KeyValuePairs | MergeByKey (max 32) | `{"type":"status_bar","entries":{"key-N":"value-N"}}` |
| notification-area | ShortTextWithIcon | Stack (max 8) | `{"type":"notification","text":"Alert N","icon":"warning"}` |
| alert-banner | StreamText | Replace | `"Alert: load profile active"` |
| pip | SolidColor | Replace | `{"type":"solid_color","r":0.2,"g":0.5,"b":0.8,"a":1.0}` |
| ambient-background | SolidColor | Replace | `{"type":"solid_color","r":0.0,"g":0.0,"b":0.0,"a":0.5}` |

**Merge key variation:** For the `status-bar` zone (MergeByKey), the first half of each profile SHALL use rotating keys (`key-0` through `key-31`) to fill the map. The second half SHALL reuse existing keys to exercise the replace-by-key path.

**Stack overflow:** For the `notification-area` zone (Stack, max depth 8), the tool SHALL publish fast enough during medium/high/burst profiles to trigger stack eviction.

The tool SHALL validate that each zone exists (via `list_zones`) before starting load profiles. If a zone is missing, the tool SHALL skip it with a warning rather than failing.

### Requirement: TTL Variation
The tool SHALL support a `--short-ttl` flag that sets publish TTL to 1 second (1,000,000 us) instead of the default 120 seconds. This exercises the TTL expiry housekeeping path and measures its impact on latency. When enabled, the report SHALL include a `ttl_mode: "short"` field.

Default TTL (without the flag) SHALL be 60 seconds (60,000,000 us).

### Requirement: Latency Measurement
For each load profile, the tool SHALL record per-request round-trip time and compute:
- p50, p95, p99, and max latency
- Mean latency
- Error count (connection errors, timeouts, JSON-RPC error responses)
- Error rate (errors / total requests)
- Target throughput (configured rate)
- Achieved throughput (successful requests / elapsed time)

Latency SHALL be measured using monotonic clock (`time.monotonic()`) to avoid wall-clock skew. For concurrent profiles, each thread SHALL record its own latencies independently; percentiles SHALL be computed over the merged set.

### Requirement: Host Telemetry Collection
During each load profile, the tool SHALL collect host resource metrics from the Windows target via SSH at 1-second intervals:
- Process CPU total seconds (from `Get-Process tze_hud .CPU`)
- Working set memory in MB
- Private memory in MB
- GPU utilization percentage and memory usage (from `nvidia-smi`)

**CPU% computation:** The tool SHALL compute instantaneous CPU% as the delta between consecutive CPU total-seconds samples divided by the wall-clock interval: `cpu_pct = (cpu_t2 - cpu_t1) / (wall_t2 - wall_t1) * 100`. The report SHALL include both per-second instantaneous CPU% and average CPU% (total CPU delta / total elapsed) per profile.

Telemetry collection SHALL run in a background thread using `subprocess.Popen`. At the end of each profile, the telemetry thread SHALL be joined with a 5-second timeout. If it does not stop, the SSH subprocess SHALL be killed via `process.kill()`. If SSH connection fails at profile start, the profile SHALL continue with a `"telemetry": "incomplete"` flag.

### Requirement: Payload Size Variation
The tool SHALL support a `--large-payloads` flag that generates StreamText content at escalating sizes:
- Small: ~100 bytes (default)
- Medium: ~1 KB
- Large: ~10 KB
- Max: ~60 KB (approaching the MCP server's 65,536-byte read buffer)

When enabled, payload sizes rotate across publishes. This tests how content size affects latency and whether the server correctly handles near-limit payloads.

### Requirement: Report Output
The tool SHALL produce a JSON report file containing:
- Test metadata: timestamp, target URL, profiles run, total duration, flags (short-ttl, large-payloads, concurrency)
- Network baseline: p50/p95/p99/max for `list_zones` calls
- Publish baseline: p50/p95/p99/max for idle publishes
- Per-profile results: latency percentiles, error rate, target vs achieved throughput, host telemetry snapshots
- Time-series: timestamped records at 1s resolution with fields: `{ts, latency_p50_ms, latency_p99_ms, reqs_in_interval, errors_in_interval, host_cpu_pct, host_private_mem_mb}`

The tool SHALL also print a summary table to stdout:

```
Profile    | Reqs | Errs | p50    | p95    | p99    | Max    | Tgt r/s | Got r/s | CPU%  | PMem MB
-----------+------+------+--------+--------+--------+--------+---------+---------+-------+--------
baseline   |   10 |    0 |  42ms  |  65ms  |  80ms  | 120ms  |     1.0 |     1.0 |  48.2 |  274
idle       |   30 |    0 |  45ms  |  85ms  | 120ms  | 180ms  |     1.0 |     1.0 |  48.5 |  274
low        |  150 |    0 |  55ms  | 110ms  | 190ms  | 340ms  |     5.0 |     5.0 |  52.1 |  275
medium     |  600 |    0 |  62ms  | 140ms  | 250ms  | 520ms  |    20.0 |    18.7 |  68.3 |  278
high       | 1500 |   12 |  78ms  | 210ms  | 480ms  | 1100ms |    50.0 |    31.2 |  89.4 |  285
burst      | 1000 |   85 | 105ms  | 350ms  | 800ms  | 2100ms |   100.0 |    22.5 |  97.1 |  290
```

### Requirement: Connection Parameters
The tool SHALL accept connection parameters via CLI arguments with defaults matching the `/user-test` skill:
- `--url` (default: `http://tzehouse-windows.parrot-hen.ts.net:9090`)
- `--psk-env` (default: `MCP_TEST_PSK`)
- `--ssh-host` (default: `tzeus@tzehouse-windows.parrot-hen.ts.net`)
- `--ssh-key` (default: `~/.ssh/ecdsa_home`)
- `--output` (default: `stress_report_{ISO8601_compact}.json`, e.g. `stress_report_20260330T162200.json`)
- `--concurrency` (default: per-profile, see load profiles table)
- `--short-ttl` (flag, default: off)
- `--large-payloads` (flag, default: off)
- `--duration` (default: 30, seconds per profile)
- `--profiles` (default: all; comma-separated subset, e.g. `idle,medium,burst`)

No credentials SHALL be hardcoded. The PSK SHALL be read from the environment variable specified by `--psk-env`.

### Scenario: Successful stress test run
- **GIVEN** a running tze_hud.exe with MCP HTTP on port 9090
- **WHEN** the stress test script executes with default parameters
- **THEN** both baselines and all 5 load profiles run sequentially
- **AND** a JSON report is written to the output path
- **AND** a summary table is printed to stdout
- **AND** the exit code is 0

### Scenario: Throughput ceiling detected
- **GIVEN** a running tze_hud.exe
- **WHEN** the burst profile runs at 100 req/s target with concurrency 16
- **THEN** the achieved throughput is reported alongside the target
- **AND** the gap between target and achieved rate quantifies the server's ceiling

### Scenario: MCP endpoint unreachable
- **GIVEN** no tze_hud.exe running on the target host
- **WHEN** the stress test script attempts the network baseline
- **THEN** the script exits with code 1 and a clear error message

### Scenario: Telemetry SSH failure
- **GIVEN** SSH key authentication fails for the telemetry host
- **WHEN** a load profile runs
- **THEN** the profile completes with latency data but telemetry marked as "incomplete"
- **AND** a warning is printed to stderr

### Scenario: Stack and MergeByKey contention exercise
- **GIVEN** a running tze_hud.exe with default zones
- **WHEN** the medium or higher profile publishes to notification-area rapidly
- **THEN** the Stack (max depth 8) evicts oldest entries
- **AND** publishes to status-bar with rotating merge keys fill the MergeByKey map (max 32)
- **AND** latency for these contention-exercising publishes is included in the profile statistics
