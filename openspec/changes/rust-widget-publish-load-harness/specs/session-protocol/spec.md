# Session Protocol — Delta: Rust Publish Load Harness

## MODIFIED Requirements

### Requirement: Widget Publish Result
WidgetPublishResult SHALL be a ServerMessage payload at field 47. It MUST carry: `request_sequence` (uint64, echoing the originating ClientMessage envelope sequence for the durable `WidgetPublish`), `accepted` (bool), `widget_name` (string), and error (optional structured error with code and message). Error codes remain: WIDGET_NOT_FOUND, WIDGET_UNKNOWN_PARAMETER, WIDGET_PARAMETER_TYPE_MISMATCH, WIDGET_PARAMETER_INVALID_VALUE, WIDGET_CAPABILITY_MISSING. WidgetPublishResult SHALL only be sent for durable-widget publishes.

#### Scenario: Accepted result echoes request sequence
- **WHEN** the runtime successfully applies a durable widget publish
- **THEN** it SHALL send `WidgetPublishResult(accepted=true, request_sequence=<client-envelope-sequence>, widget_name=<name>)`

#### Scenario: Rejected result still echoes request sequence
- **WHEN** a durable widget publish is rejected for schema, capability, or lookup reasons
- **THEN** the runtime SHALL still send `WidgetPublishResult(accepted=false, request_sequence=<client-envelope-sequence>, widget_name=<name>, error=...)`

#### Scenario: Repeated publishes to the same widget remain distinguishable
- **WHEN** multiple durable publishes target the same widget instance on the same session stream
- **THEN** the runtime SHALL emit a distinct `request_sequence` in each acknowledgement so the client can correlate each result to the correct publish request

#### Scenario: No result for ephemeral publish
- **WHEN** an agent publishes to an ephemeral widget
- **THEN** the runtime SHALL NOT send a WidgetPublishResult regardless of whether the publish succeeded or failed
