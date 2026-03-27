## ADDED Requirements

### Requirement: Canonical Runtime Application Executable
The project SHALL provide a canonical, non-demo runtime application executable target distinct from example/demo binaries. The canonical executable SHALL be documented as the primary operator-facing runtime artifact.

#### Scenario: Runtime executable target is discoverable
- **WHEN** build metadata is queried for binary targets
- **THEN** exactly one canonical runtime app target is identifiable as non-demo
- **AND** demo/example targets remain available but are not designated as the primary runtime artifact

### Requirement: Configuration-Driven Runtime Startup
The canonical runtime executable SHALL support configuration-driven startup for window mode and network endpoint settings, including deterministic enable/disable behavior for each endpoint.

#### Scenario: Runtime starts with configured display and network settings
- **WHEN** the runtime executable is launched with a valid runtime configuration
- **THEN** it SHALL apply configured window mode and dimensions
- **AND** it SHALL enable only the configured network endpoints
- **AND** it SHALL keep disabled endpoints unbound

### Requirement: Windows Artifact Identity for Automation
The canonical runtime executable SHALL produce a deterministic Windows build artifact identity suitable for deployment automation.

#### Scenario: Automation resolves artifact path without heuristics
- **WHEN** the runtime executable is built for a Windows target triple
- **THEN** the artifact name and output location SHALL be stable and documented
- **AND** deployment automation SHALL be able to reference the artifact without relying on demo-target fallbacks
