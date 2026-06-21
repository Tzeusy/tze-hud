//! MCP lifecycle + introspection payloads (`initialize`, `tools/list`).
//!
//! The server is a bare JSON-RPC method router; standard MCP clients discover
//! the available tools and their input schemas via `tools/list`, and negotiate
//! the protocol version via `initialize`. Without these, an LLM cannot
//! introspect any tool and is wholly dependent on out-of-band skill docs.
//!
//! Schemas here are hand-written (the crate does not depend on `schemars`) and
//! must stay faithful to the `*Params` structs in [`crate::tools`]. Each tool's
//! `inputSchema` lists every accepted property, marks the non-defaulted /
//! non-`Option` fields as `required`, and mirrors the field-level types. When a
//! `*Params` struct changes, update the matching descriptor below — the
//! `tools/list` introspection contract is only as honest as this file.

use serde_json::{json, Value};

/// MCP protocol revision advertised by `initialize`.
///
/// The runtime implements the JSON-RPC tool surface compatible with this
/// revision of the Model Context Protocol.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Build the `initialize` handshake result.
///
/// Returns the protocol version, server identity, and declared capabilities.
/// The runtime exposes a static tool surface, so `tools.listChanged` is
/// `false` (the list never changes mid-session).
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "serverInfo": {
            "name": "tze_hud_mcp",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {
            "tools": { "listChanged": false }
        },
    })
}

/// Build the `tools/list` result: `{ "tools": [ { name, description,
/// inputSchema }, ... ] }`.
pub fn tools_list_result() -> Value {
    json!({ "tools": tool_descriptors() })
}

/// A single tool descriptor.
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        },
    })
}

/// A scalar property: `{ "type": ty, "description": desc }`.
fn p(ty: &str, desc: &str) -> Value {
    json!({ "type": ty, "description": desc })
}

/// The RGBA color sub-object schema (`{r,g,b,a}` in `[0.0, 1.0]`, `a` defaults).
fn color_schema(desc: &str) -> Value {
    json!({
        "type": "object",
        "description": desc,
        "properties": {
            "r": { "type": "number" },
            "g": { "type": "number" },
            "b": { "type": "number" },
            "a": { "type": "number", "description": "Alpha; defaults to 1.0." },
        },
        "required": ["r", "g", "b"],
    })
}

/// Full ordered list of tool descriptors. Order mirrors the dispatch router in
/// [`crate::server`] for easy cross-checking.
fn tool_descriptors() -> Vec<Value> {
    vec![
        tool(
            "create_tab",
            "Create a new tab in the scene.",
            json!({
                "name": p("string", "Human-readable name for the tab."),
                "display_order": p("integer", "Display order (unique across tabs). Defaults to next available."),
            }),
            &["name"],
        ),
        tool(
            "create_tile",
            "Create a tile within a tab and grant it a lease.",
            json!({
                "tab_id": p("string", "Tab UUID to place the tile in. Defaults to the active tab."),
                "namespace": p("string", "Namespace (agent identity) used as the lease namespace."),
                "bounds": {
                    "type": "object",
                    "description": "Tile bounds in display pixels.",
                    "properties": {
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "width": { "type": "number" },
                        "height": { "type": "number" },
                    },
                    "required": ["x", "y", "width", "height"],
                },
                "z_order": p("integer", "Z-order (front = higher). Defaults to 1."),
                "ttl_ms": p("integer", "Lease TTL in milliseconds. Defaults to 60000."),
            }),
            &["namespace", "bounds"],
        ),
        tool(
            "set_content",
            "Set markdown text content on a tile.",
            json!({
                "tile_id": p("string", "UUID of the tile to set content on."),
                "content": p("string", "Markdown text to display."),
                "font_size_px": p("number", "Font size in pixels. Defaults to 16."),
                "color": color_schema("Text color. Defaults to white (#ffffff)."),
                "background": color_schema("Optional background color."),
                "alignment": p("string", "Text alignment: 'start', 'center', or 'end'. Defaults to 'start'."),
            }),
            &["tile_id", "content"],
        ),
        tool(
            "dismiss",
            "Delete a tile and release its lease.",
            json!({
                "tile_id": p("string", "UUID of the tile to delete."),
            }),
            &["tile_id"],
        ),
        tool(
            "publish_to_zone",
            "Publish content to a named zone.",
            json!({
                "zone_name": p("string", "Name of the target zone (must exist in the zone registry)."),
                "content": {
                    "type": ["string", "object"],
                    "description": "A plain string (StreamText) or a tagged object with a 'type' field: stream_text, notification, status_bar, solid_color, or static_image.",
                },
                "namespace": p("string", "Lease namespace. Defaults to 'mcp'."),
                "font_size_px": p("number", "Font size in pixels. Defaults to 16."),
                "ttl_us": p("integer", "TTL in microseconds. 0 selects the 60000ms default."),
                "merge_key": p("string", "Merge key for idempotent zone publishes."),
                "breakpoints": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Byte-offset breakpoints for word-by-word reveal of StreamText.",
                },
            }),
            &["zone_name", "content"],
        ),
        tool(
            "list_zones",
            "List all registered zones.",
            json!({}),
            &[],
        ),
        tool(
            "list_scene",
            "List the scene's tabs.",
            json!({}),
            &[],
        ),
        tool(
            "list_elements",
            "List addressable elements (tiles, zones, widgets).",
            json!({
                "namespace_filter": p("string", "Optional namespace prefix filter."),
                "element_type": p("string", "Optional element type filter: 'tile', 'zone', or 'widget'."),
            }),
            &[],
        ),
        tool(
            "publish_to_widget",
            "Publish typed parameter values to a widget instance.",
            json!({
                "widget_name": p("string", "Widget instance name (instance_id or type name for single-instance)."),
                "instance_id": p("string", "Explicit instance_id to disambiguate multiple instances of a type."),
                "params": {
                    "type": "object",
                    "description": "Parameter values keyed by name. Numbers, strings, color objects, or enum strings per the widget schema.",
                    "additionalProperties": true,
                },
                "transition_ms": p("integer", "Transition duration in ms (0 = instant). Defaults to 0."),
                "namespace": p("string", "Namespace. Defaults to 'mcp'."),
                "ttl_us": p("integer", "TTL in microseconds (0 = widget instance default)."),
            }),
            &["widget_name", "params"],
        ),
        tool(
            "list_widgets",
            "List registered widget types and instances.",
            json!({}),
            &[],
        ),
        tool(
            "clear_widget",
            "Clear a namespace's publications from a widget instance.",
            json!({
                "widget_name": p("string", "Widget instance name (addressing key)."),
                "namespace": p("string", "Agent namespace performing the clear. Defaults to ''."),
                "instance_id": p("string", "Optional disambiguation when instances share a name."),
            }),
            &["widget_name"],
        ),
        tool(
            "register_widget_asset",
            "Register (or dedup-preflight) an SVG asset for a widget type.",
            json!({
                "widget_type_id": p("string", "Widget type id to associate with this SVG asset."),
                "svg_filename": p("string", "SVG filename (must end with '.svg')."),
                "content_hash_blake3": p("string", "64-char hex BLAKE3 hash of the payload bytes."),
                "total_size_bytes": p("integer", "Declared payload size in bytes."),
                "transport_crc32c": p("integer", "Optional transport integrity checksum (CRC32C)."),
                "payload": p("string", "Optional payload bytes as UTF-8 text (raw SVG XML)."),
                "metadata_only_preflight": p("boolean", "When true, run metadata-only dedup preflight."),
            }),
            &["widget_type_id", "svg_filename", "content_hash_blake3", "total_size_bytes"],
        ),
        tool(
            "publish_to_element",
            "Publish content to a tile, zone, or widget by stable element UUID.",
            json!({
                "element_id": p("string", "Stable element UUID."),
                "content": {
                    "type": ["string", "object"],
                    "description": "Content payload following the same rules as set_content / publish_to_zone (tiles/zones) or widget params (widgets).",
                },
                "namespace": p("string", "Namespace for zone/widget publish bookkeeping. Defaults to 'mcp'."),
                "merge_key": p("string", "Optional zone merge key."),
                "breakpoints": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Optional zone breakpoints for stream_text content.",
                },
                "ttl_us": p("integer", "Optional zone/widget TTL in microseconds."),
                "transition_ms": p("integer", "Optional widget transition duration in ms."),
                "font_size_px": p("number", "Optional tile font size override."),
                "color": color_schema("Optional tile text color override."),
                "background": color_schema("Optional tile background color override."),
                "alignment": p("string", "Optional tile text alignment override."),
            }),
            &["element_id", "content"],
        ),
        tool(
            "inject_composer_paste",
            "Inject text into the active composer draft buffer.",
            json!({
                "text": p("string", "Text to inject. CR/LF/control chars stripped; truncated at the draft cap."),
            }),
            &["text"],
        ),
        // ── Portal projection tools (cooperative HUD self-projection) ─────────
        tool(
            "portal_projection_attach",
            "Attach a new cooperative projection session to the in-process authority.",
            json!({
                "projection_id": p("string", "Caller-assigned unique identifier for this projection session (max 128 bytes)."),
                "display_name": p("string", "Human-readable label for this projection session (max 128 bytes)."),
                "idempotency_key": p("string", "Optional key to replay-safely re-attach after a network interruption."),
            }),
            &["projection_id", "display_name"],
        ),
        tool(
            "portal_projection_publish",
            "Append output text to an existing projection transcript.",
            json!({
                "projection_id": p("string", "Projection id from a prior portal_projection_attach call."),
                "owner_token": p("string", "Owner token returned by the successful attach."),
                "output_text": p("string", "Text to append to the projection transcript."),
                "logical_unit_id": p("string", "Optional logical-unit id for idempotent dedup (max 128 bytes)."),
                "output_kind": p("string", "Optional output kind: assistant (default), tool, status, error, other."),
                "content_classification": p("string", "Optional classification: public, household, private (default), sensitive."),
                "coalesce_key": p("string", "Optional coalesce key; repeated publishes sharing it collapse in-place."),
            }),
            &["projection_id", "owner_token", "output_text"],
        ),
        tool(
            "portal_projection_publish_status",
            "Publish a lifecycle status to an existing projection session.",
            json!({
                "projection_id": p("string", "Projection id from a prior portal_projection_attach call."),
                "owner_token": p("string", "Owner token returned by the successful attach."),
                "lifecycle_state": p("string", "Lifecycle state: attached, active, degraded, hud_unavailable, detached, cleanup_pending, expired."),
                "status_text": p("string", "Optional human-readable status detail recorded with the state."),
            }),
            &["projection_id", "owner_token", "lifecycle_state"],
        ),
        tool(
            "portal_projection_get_pending_input",
            "Drain HUD-originated pending input for a projection session.",
            json!({
                "projection_id": p("string", "Projection id from a prior portal_projection_attach call."),
                "owner_token": p("string", "Owner token returned by the successful attach."),
                "max_items": p("integer", "Optional cap on items returned; clamped to the authority's max_poll_items."),
                "max_bytes": p("integer", "Optional cap on response byte budget; clamped to the authority's max_poll_response_bytes."),
            }),
            &["projection_id", "owner_token"],
        ),
        tool(
            "portal_projection_acknowledge_input",
            "Acknowledge a delivered input item for a projection session.",
            json!({
                "projection_id": p("string", "Projection id from a prior portal_projection_attach call."),
                "owner_token": p("string", "Owner token returned by the successful attach."),
                "input_id": p("string", "Identifier of the input item being acknowledged."),
                "ack_state": p("string", "Acknowledgement state: handled, deferred, or rejected."),
                "ack_message": p("string", "Optional human-readable message recorded with the ack."),
                "not_before_wall_us": p("integer", "Optional re-delivery floor (wall-clock µs); valid only when ack_state is deferred."),
            }),
            &["projection_id", "owner_token", "input_id", "ack_state"],
        ),
        tool(
            "portal_projection_detach",
            "Detach a projection session, purging its private state.",
            json!({
                "projection_id": p("string", "Projection id from a prior portal_projection_attach call."),
                "owner_token": p("string", "Owner token returned by the successful attach."),
                "reason": p("string", "Human-readable reason recorded in the audit log."),
            }),
            &["projection_id", "owner_token", "reason"],
        ),
        tool(
            "portal_projection_cleanup",
            "Cleanup a projection session via owner or operator authority.",
            json!({
                "projection_id": p("string", "Projection id from a prior portal_projection_attach call."),
                "cleanup_authority": p("string", "Cleanup authority: 'owner' or 'operator'."),
                "owner_token": p("string", "Owner token required when cleanup_authority = 'owner'."),
                "operator_authority": p("string", "Operator credential required when cleanup_authority = 'operator'."),
                "reason": p("string", "Human-readable reason recorded in the audit log."),
            }),
            &["projection_id", "cleanup_authority", "reason"],
        ),
    ]
}
