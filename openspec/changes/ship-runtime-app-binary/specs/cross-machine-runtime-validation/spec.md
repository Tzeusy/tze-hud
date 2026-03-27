## ADDED Requirements

### Requirement: Cross-Machine Runtime Validation Flow
The project SHALL provide a reproducible validation flow for Linux cross-build, Windows deployment, runtime launch, and live MCP publish verification using the canonical app artifact.

#### Scenario: End-to-end cross-machine flow completes
- **WHEN** operators execute the documented automation flow from Linux against a reachable Windows host
- **THEN** the canonical Windows runtime artifact SHALL be built or selected
- **AND** the artifact SHALL be deployed and launched on Windows
- **AND** live MCP zone publish verification SHALL run against the launched runtime

### Requirement: MCP Reachability Gate Before Publish
Validation tooling SHALL verify MCP endpoint reachability before attempting publish assertions.

#### Scenario: Reachability gate blocks false publish claims
- **WHEN** MCP endpoint is unreachable
- **THEN** validation flow SHALL fail before publish attempts
- **AND** the failure output SHALL identify endpoint reachability as the blocking condition

### Requirement: Actionable Failure Diagnostics
Validation output SHALL include actionable diagnostics for launch/runtime mismatches.

#### Scenario: Runtime launch does not produce a publishable MCP surface
- **WHEN** deployment succeeds but runtime does not expose expected MCP behavior
- **THEN** tooling SHALL report launch path, endpoint state, and publish error payloads
- **AND** output SHALL clearly differentiate artifact/deploy failures from runtime endpoint failures
