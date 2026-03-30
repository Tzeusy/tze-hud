## Why

The v1-MVP validation framework (Layer 3) covers internal compositor performance — frame budgets, telemetry, headless benchmarks. But it has no coverage for the MCP HTTP endpoint under external load. The e2e user-test (2026-03-30) revealed MCP latency spikes up to 1091ms and ~96% CPU utilization at idle frame rate, with no structured way to characterize, reproduce, or regress-test these behaviors. Before v1 release, we need to know the MCP publish throughput ceiling, latency distribution under load, and resource consumption trends.

## What Changes

- Add an external MCP stress testing capability that exercises the `publish_to_zone` endpoint from a remote client at configurable load levels
- Define load profiles (idle through burst) with per-profile latency and error budgets
- Collect cross-machine telemetry (CPU, memory, GPU) from the Windows host during test runs
- Produce structured JSON reports with time-series data for trend analysis
- Extend the validation framework's Layer 3 scope to include network-facing endpoint validation alongside internal compositor benchmarks

## Capabilities

### New Capabilities
- `mcp-stress-testing`: External load testing of the MCP HTTP endpoint — load profiles, multi-zone media type coverage, latency percentile measurement, cross-machine resource telemetry collection, and structured JSON reporting

### Modified Capabilities
- `validation-framework`: Extend Layer 3 scope to reference external MCP endpoint stress testing as a complementary validation dimension (delta spec)

## Impact

- New Python script in `.claude/skills/user-test/scripts/` — no Rust code changes
- Depends on deployed tze_hud.exe with MCP HTTP enabled (port 9090)
- Depends on SSH access to Windows host for telemetry collection
- Extends the test corpus referenced by validation-framework Layer 3
