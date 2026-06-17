# external-agent-projection-authority Specification
Status: v1-reserved

## Purpose
Define the contract for an **external** agent projection authority that supervises
multiple provider-neutral LLM sessions (launched or attached), authenticates to a
local `tze_hud` runtime over MCP/gRPC, and routes each session to existing v1 HUD
surfaces (zones, widgets, leased text-stream/raw-tile portals) without terminal
capture, PTY injection, or an LLM in the frame loop.

## Status Note: External Daemon Is Aspirational; v1 Authority Is In-Process
**This external, separately-hosted projection authority is the future deployment
model. It is NOT what is wired in v1.** In v1 the authority role is filled by an
**in-process** `ProjectionAuthority` hosted by `InProcessPortalDriver` inside the
windowed runtime (`crates/tze_hud_runtime/src/portal_projection_driver.rs`; the
channel/driver are created in `crates/tze_hud_runtime/src/windowed.rs` ~4924-5106
and drained by `drain_portal_ops` at ~3746-3788; MCP wiring in
`crates/tze_hud_runtime/src/mcp.rs` ~96-100). Cooperative output operations
(`attach`, `publish_output`) are wired into it through the runtime MCP server's
portal-projection facade tools (`portal_projection_attach`,
`portal_projection_publish`, `crates/tze_hud_mcp/src/server.rs` ~556-565). Those
two tools are not yet reachable by an external session, however: they are
classified Resident and rejected with `CAPABILITY_REQUIRED` unless the caller
holds `resident_mcp` (`crates/tze_hud_mcp/src/server.rs` ~198-199, ~396), while
the runtime's HTTP MCP transport mints only bearer/guest contexts with no
capabilities (`crates/tze_hud_runtime/src/mcp.rs` ~256-260). The resident-capable
ingress that makes the served subset reachable externally is tracked by
hud-bq0gl.1.

The implementation pointer below is the **stdio dev harness**, not a live
external daemon: `crates/tze_hud_projection/src/bin/projection_authority.rs`
retains authority state only for its own process lifetime and emits stdout drain
records for a caller to forward; it does not connect to the running runtime and
has no live MCP server + bearer token of its own. The multi-session external
supervisor, the standalone HUD-target authentication, and the separately-hosted
ingress described in the requirements below remain future work (tracked by
hud-bq0gl.1/.3). Treat these requirements as the contract the external authority
must meet **if/when** it ships, not as a description of current v1 wiring.

Implementation: crates/tze_hud_projection/src/bin/projection_authority.rs (stdio dev harness; the live in-process authority is in crates/tze_hud_runtime/)

## Requirements
### Requirement: External Session Authority Boundary
An external agent projection authority SHALL manage provider-neutral LLM session launch and attach records outside the compositor and outside runtime core. The authority MAY supervise launched provider processes or attach already-running sessions, but it MUST NOT rely on terminal capture, PTY injection, raw stdin/stdout interception, provider-specific RPCs in the runtime, or an LLM inside the frame loop.

#### Scenario: attached session remains cooperative
- **WHEN** an already-running Codex, Claude, opencode, or other LLM session is attached to the authority
- **THEN** the authority SHALL register a provider-neutral session record
- **AND** output and input SHALL flow through explicit cooperative projection operations rather than terminal capture

#### Scenario: launched session is not captured
- **WHEN** the authority launches a provider command for an LLM session
- **THEN** the launched session SHALL still publish HUD presence through the cooperative operation contract or an equivalent semantic adapter
- **AND** the authority SHALL NOT claim raw terminal transcript capture as the source of HUD truth

### Requirement: Windows HUD Target Authentication
The authority SHALL target the local Windows `tze_hud` runtime through explicit HUD target metadata, including MCP and/or gRPC endpoint, runtime audience, and credential source. Credential values SHALL be supplied from protected local configuration or environment and MUST NOT appear in audit records, route plans, docs, scene nodes, or bounded operation responses.

#### Scenario: target authenticates without leaking secret
- **WHEN** the authority builds a route plan for a Windows HUD target using a PSK-backed credential source
- **THEN** the route plan SHALL include only the credential source identity or redacted credential marker
- **AND** no secret value SHALL be serialized into the route plan or audit record

#### Scenario: runtime auth material is resolved only at execution edge
- **WHEN** a managed session is ready to execute an MCP or gRPC runtime command
- **THEN** the authority MAY resolve credential material from the registered environment or protected config source
- **AND** the secret-bearing material SHALL NOT be serializable or included in debug/audit output
- **AND** missing or empty credential material SHALL fail closed before attempting runtime publish or lease operations

#### Scenario: runtime remains final authorizer
- **WHEN** the authority routes a session to a zone, widget, or portal surface
- **THEN** the runtime SHALL still enforce the authenticated session capabilities, content policy, lease scope, TTL, revocation, safe mode, and resource budgets
- **AND** the authority SHALL treat runtime denial as authoritative

### Requirement: Governed Presence Surface Routing
Each managed LLM session SHALL be routed to existing v1 HUD surfaces only: named zones, registered widget instances, or leased text-stream/raw-tile portals. The authority SHALL NOT create agent-rendered chrome or require new compositor node types for v1. Every route SHALL carry projection ID, provider kind, lifecycle state, content classification, attention intent, TTL or lease intent, and cleanup behavior.

#### Scenario: three sessions use distinct surfaces
- **WHEN** three provider-neutral LLM sessions request presence concurrently
- **THEN** the authority SHALL be able to route one session to a zone publish, one to a widget publish, and one to a leased portal
- **AND** each route SHALL remain independently revocable and bounded

#### Scenario: attention remains ambient by default
- **WHEN** a session publishes status, questions, alerts, progress, or attention-worthy events without explicit higher-priority policy
- **THEN** the route SHALL use ambient or gentle attention intent
- **AND** backlog growth alone SHALL NOT escalate the interruption class

#### Scenario: portal route identifies existing raw-tile materialization
- **WHEN** a managed session requests a leased portal route
- **THEN** the route plan SHALL identify the portal as the existing text-stream raw-tile surface
- **AND** the live replay path SHALL use the resident gRPC text-stream portal adapter rather than a new compositor primitive

### Requirement: Multi-Session Lifecycle Management
The authority SHALL maintain independent lifecycle state for every managed session, including launched/attached origin, provider-neutral identity, route state, last connection state, reconnect bookkeeping, owner token or equivalent verifier, and cleanup/expiry deadline. Cleanup, revocation, detach, and expiry SHALL purge private projection state for the affected session without exposing or mutating other sessions.

#### Scenario: one session cleanup leaves others intact
- **WHEN** three sessions are active and one session is revoked or expires
- **THEN** the authority SHALL mark only that session cleanup-pending or removed
- **AND** route plans and owner state for the other sessions SHALL remain valid subject to their own runtime leases and TTLs

#### Scenario: reconnect requires fresh runtime authority
- **WHEN** a HUD connection drops and reconnects
- **THEN** the authority MAY preserve in-memory session bookkeeping
- **AND** it SHALL regain authenticated runtime capabilities before republishing routes or reusing advisory lease identity

