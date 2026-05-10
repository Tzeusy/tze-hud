# windowed-runtime-network-services Specification

## Purpose
Defines network service initialization, MCP listener lifecycle, and authentication enforcement inside the canonical windowed runtime process.

## Requirements
### Requirement: Windowed Runtime Network Service Initialization
Windowed runtime startup SHALL initialize network service infrastructure when network endpoints are configured, rather than running as compositor-only mode.

#### Scenario: Network runtime initializes in windowed path
- **WHEN** windowed runtime is launched with one or more enabled network endpoints
- **THEN** network runtime threads/tasks SHALL be initialized during startup
- **AND** endpoint listeners SHALL be started without requiring a separate headless runtime process

### Requirement: MCP HTTP Listener Lifecycle
The runtime SHALL manage MCP HTTP listener lifecycle within the canonical app process, including bind, operational serving, and clean shutdown.

#### Scenario: MCP HTTP listener binds and serves requests
- **WHEN** MCP HTTP is enabled with a valid bind address
- **THEN** the runtime SHALL bind the listener and accept JSON-RPC requests
- **AND** authenticated zone operations SHALL be processable while runtime is active

#### Scenario: MCP HTTP listener stops on runtime shutdown
- **WHEN** runtime shutdown is triggered
- **THEN** MCP HTTP listener SHALL stop accepting new requests
- **AND** runtime shutdown SHALL complete without hanging on MCP listener teardown

### Requirement: MCP Authentication Enforcement
The MCP HTTP listener SHALL enforce configured authentication requirements for all MCP calls.

#### Scenario: Unauthorized MCP request is rejected
- **WHEN** a request is sent without valid configured credentials
- **THEN** the runtime SHALL reject the request with an authentication error response
- **AND** protected operations SHALL not mutate scene state
