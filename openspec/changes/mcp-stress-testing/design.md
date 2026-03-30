## Context

The MCP HTTP server (`crates/tze_hud_runtime/src/mcp.rs`) is a minimal HTTP/1.0 handler — one request per connection, no keep-alive. It shares the `Arc<Mutex<SceneGraph>>` with the compositor thread, meaning every publish contends with the 60fps frame loop for the scene lock. The e2e user-test (2026-03-30) showed baseline MCP latency of 40-226ms with a 1091ms spike at idle, and CPU at ~96% of one core (dominated by `device.poll(Wait)` in the frame loop).

The existing `publish_zone_batch.py` script handles single-batch publishes but has no load ramping, telemetry collection, or reporting. The validation-framework Layer 3 spec covers internal compositor telemetry but not external endpoint testing.

## Goals / Non-Goals

**Goals:**
- Characterize MCP publish latency distribution (p50/p95/p99/max) under 5 load profiles (1/5/20/50/100 req/s)
- Identify the throughput ceiling where error rate exceeds 1%
- Collect cross-machine host telemetry (CPU%, memory, GPU) via SSH during each profile
- Produce a machine-readable JSON report with time-series data
- Cover all 6 default zone types with their correct media type payloads
- Run from Linux against a deployed Windows tze_hud.exe — same parameters as `/user-test`

**Non-Goals:**
- Modifying the MCP server implementation (this is a measurement tool, not an optimization)
- GUI or dashboard for results (JSON + stdout table is sufficient)
- Testing gRPC endpoints (MCP HTTP only)
- Long-running soak tests (max profile duration ~30s; soak testing is a separate concern)
- Run-to-run regression comparison (future: `--baseline <path>` flag)

## Decisions

### Single Python script with stdlib only + ThreadPoolExecutor for concurrency
**Decision:** Pure Python 3 with `urllib`, `subprocess`, `json`, `time`, `statistics`, `concurrent.futures` — no pip dependencies.
**Rationale:** Matches `publish_zone_batch.py` pattern. Avoids requiring a virtualenv on the test runner. Low-rate profiles (idle, low) use single-threaded sequential publishing. High-rate profiles (medium, high, burst) use `ThreadPoolExecutor` to achieve target rates that exceed single-threaded throughput (~10 req/s at 100ms latency).
**Alternative considered:** asyncio + aiohttp. Rejected because `concurrent.futures` is stdlib and sufficient for the concurrency levels needed (max 16 threads). The blocking `urllib` calls work naturally in thread pools.

### SSH-based host telemetry sampling
**Decision:** Spawn a background SSH command that samples `Get-Process tze_hud` and `nvidia-smi` every 1s during each profile, collecting output into a buffer.
**Rationale:** Avoids installing any agent on the Windows host. SSH is already configured for `/user-test`. PowerShell `Get-Process` provides CPU/memory without elevation.
**Alternative considered:** WMI/CIM queries over the network. Rejected because SSH is already proven and simpler.

### Load profiles as sequential phases (not concurrent)
**Decision:** Run each profile to completion before starting the next. Brief cooldown (3s) between profiles.
**Rationale:** Clean measurement isolation. Each profile's telemetry is independent. Avoids warmup/cooldown contamination between profiles.

## Risks / Trade-offs

- **[Scene lock contention]** At high publish rates, the MCP handler will contend with the compositor for `Arc<Mutex<SceneGraph>>`. This is the behavior we want to MEASURE, not avoid. → No mitigation needed; this is the point of the test.
- **[Network variability]** Tailscale latency can spike. → Report raw latencies and let the operator interpret. Include a network-only baseline (list_zones with no publish) at the start.
- **[SSH telemetry sampling gaps]** If SSH connection drops during a profile, telemetry for that profile is incomplete. → Log a warning, continue the test, mark the profile as "telemetry-incomplete" in the report.
- **[MCP HTTP/1.0 connection-per-request overhead]** Each publish opens a new TCP connection. At 100 req/s this is 100 TCP handshakes/s. → This is realistic because the production MCP server is HTTP/1.0 by design.
- **[ThreadPoolExecutor GIL contention]** Python's GIL limits true parallelism, but since each thread blocks on I/O (`urllib.urlopen`), the GIL is released during the network wait. Thread pool concurrency is effective for I/O-bound workloads.
- **[Telemetry thread cleanup]** SSH subprocess may hang if the remote host is unresponsive. → Join with 5s timeout, then `process.kill()` to prevent orphan SSH processes.
