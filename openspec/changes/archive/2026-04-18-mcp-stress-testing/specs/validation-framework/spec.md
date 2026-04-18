# Validation Framework — Delta: MCP Stress Testing

Extends Layer 3 (Compositor Telemetry and Performance Validation) to include external MCP endpoint stress testing.

## ADDED Requirements

### Requirement: Layer 3 — External Endpoint Validation
In addition to internal compositor telemetry benchmarks, Layer 3 SHALL include external MCP HTTP endpoint stress testing as a complementary validation dimension. The MCP stress test tool (see `mcp-stress-testing` capability spec) provides network-facing latency and throughput characterization that cannot be observed from internal telemetry alone.

The MCP stress test results SHALL be includable in Layer 4 artifacts (`benchmarks/mcp-stress/`) alongside internal compositor benchmarks, using the same JSON report format conventions.

**Note:** Layer 4 artifact generation is not yet implemented. The integration below is aspirational — the stress test JSON report is self-contained and useful standalone. Layer 4 integration will be implemented when the artifact pipeline is built.

#### Scenario: MCP stress results included in Layer 4 artifacts (aspirational)
- **WHEN** Layer 4 artifacts are generated after an MCP stress test run
- **THEN** the stress test JSON report SHALL be copied to `benchmarks/mcp-stress/` in the artifact output directory
- **AND** the `manifest.json` SHALL reference the MCP stress report
