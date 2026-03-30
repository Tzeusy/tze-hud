---
name: th-hud-publish
description: Use when publishing content to a user's HUD display, showing notifications or status on screen, sending text/color/alerts to tze_hud display zones, or giving an LLM visual presence on a user's GUI via MCP zone publishing.
---

# HUD Publish

Publish content to a running tze_hud instance via its MCP guest surface. This skill gives any LLM agent the ability to display text, notifications, status information, and colors on a user's screen through named zones.

## Quick Start

If MCP tools are already connected (you can see `list_zones` and `publish_to_zone` as available tools):

```
1. Call list_zones() — no parameters
2. Pick a zone from the result based on what you want to show
3. Use the decision table below to build the right content format
4. Call publish_to_zone with zone_name, content, and optional ttl_us
```

If not connected, see **Setup** below.

## Setup

### 1. MCP Server Configuration

Copy `settings.template.json` from this skill directory to your project's `.claude/settings.json`, or merge the `mcpServers` entry into your existing settings:

```json
{
  "mcpServers": {
    "tze-hud": {
      "type": "url",
      "url": "http://<HUD_HOST>:9090",
      "headers": {
        "Authorization": "Bearer ${HUD_MCP_PSK}"
      }
    }
  }
}
```

Replace `<HUD_HOST>` with the hostname or IP of the machine running the HUD (e.g., `192.168.1.50`, `mypc.tail1234.ts.net`). Set `HUD_MCP_PSK` in your shell environment to the pre-shared key configured on the HUD instance.

### 2. Verify Connectivity

After configuring, the MCP tools `list_zones` and `publish_to_zone` should appear as available tools. Call `list_zones` to verify the connection is live.

## Deciding What Content Format to Use

**This is the key table.** When `list_zones` tells you a zone's `accepted_media_types`, use this to pick the right content shape:

| Zone's `accepted_media_types` contains | Content format to use | Example |
|---|---|---|
| `StreamText` | Plain string | `"content": "Hello world"` |
| `StreamText` | Or typed object | `"content": {"type": "stream_text", "text": "Hello world"}` |
| `ShortTextWithIcon` | Notification object | `"content": {"type": "notification", "text": "Done!", "icon": "", "urgency": 1}` |
| `KeyValuePairs` | Status bar object | `"content": {"type": "status_bar", "entries": {"key": "value"}}` |
| `SolidColor` | Color object | `"content": {"type": "solid_color", "r": 0.1, "g": 0.2, "b": 0.4, "a": 1.0}` |

If a zone accepts multiple media types (e.g., `["ShortTextWithIcon", "StreamText"]`), pick whichever matches your intent.

**If you use the wrong content format for a zone, you'll get a `zone media type mismatch` error.** Always check `accepted_media_types` from `list_zones` before publishing.

## Workflow

**Always discover before publishing.** Never hardcode zone names.

```
1. Call list_zones → inspect available zones, their types, and contention policies
2. Select an appropriate zone for your content type
3. Call publish_to_zone with the discovered zone name and matching content
4. Check the response for success (ephemeral zones may not return a result)
```

If `list_zones` returns an empty array, the HUD has no zones configured — do not attempt to publish.

## MCP Tool Reference

### `list_zones`

Discover all available zones on the display surface.

**Parameters**: _(none)_

**Response** (example from a typical HUD instance):
```json
{
  "result": {
    "count": 6,
    "zones": [
      {
        "name": "alert-banner",
        "description": "Alert banner — top edge, single occupant",
        "has_content": false,
        "id": "019d3db1-..."
      },
      {
        "name": "ambient-background",
        "description": "Ambient background zone — full display, behind all content",
        "has_content": false,
        "id": "019d3db1-..."
      },
      {
        "name": "notification-area",
        "description": "Notification overlay area",
        "has_content": false,
        "id": "019d3db1-..."
      },
      {
        "name": "pip",
        "description": "Picture-in-picture overlay zone",
        "has_content": false,
        "id": "019d3db1-..."
      },
      {
        "name": "status-bar",
        "description": "Status bar at the bottom of the display",
        "has_content": false,
        "id": "019d3db1-..."
      },
      {
        "name": "subtitle",
        "description": "Subtitle / caption overlay",
        "has_content": true,
        "id": "019d3db1-..."
      }
    ]
  }
}
```

### `publish_to_zone`

Publish content to a named zone.

**Parameters**:

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `zone_name` | string | yes | Target zone name (from `list_zones`) |
| `content` | string or object | yes | Content payload — plain string for StreamText zones, or a typed JSON object (see decision table above) |
| `ttl_us` | uint64 | no | Time-to-live in **microseconds**. 0 or omitted = zone default (typically 60s). Example: `60000000` = 60 seconds |
| `merge_key` | string | no | Key for `MERGE_BY_KEY` zones. Same key replaces; different keys coexist. Ignored for other contention policies |
| `namespace` | string | no | Publisher namespace for grouping/filtering (default: `"mcp"`) |

#### Content formats

**Plain string** (for zones accepting `StreamText`, like `alert-banner`, `subtitle`):
```json
"content": "Build passed — all 79 tests green"
```

**Typed stream_text** (equivalent to plain string, useful when you want explicit typing):
```json
"content": {"type": "stream_text", "text": "Build passed — all 79 tests green"}
```

**Notification** (for zones accepting `ShortTextWithIcon`, like `notification-area`):
```json
"content": {"type": "notification", "text": "Deploy complete", "icon": "", "urgency": 1}
```
- `text` (string, required): The notification message
- `icon` (string, optional, default `""`): Icon resource name
- `urgency` (int, optional, default `1`): 0=low, 1=normal, 2=urgent, 3=critical

**Status bar** (for zones accepting `KeyValuePairs`, like `status-bar`):
```json
"content": {"type": "status_bar", "entries": {"build": "passing", "agent": "ci-bot"}}
```
- `entries` (object, required): Non-empty map of string key-value pairs. Use with `merge_key`.

**Solid color** (for zones accepting `SolidColor`, like `ambient-background`, `pip`):
```json
"content": {"type": "solid_color", "r": 0.1, "g": 0.2, "b": 0.4, "a": 1.0}
```
- `r`, `g`, `b` (float, optional, default `0.0`): Color channels 0.0–1.0
- `a` (float, optional, default `1.0`): Alpha 0.0–1.0

#### Example requests

**Text to alert-banner:**
```json
{
  "zone_name": "alert-banner",
  "content": "Build passed — all tests green",
  "ttl_us": 30000000
}
```

**Status bar entries (with merge key):**
```json
{
  "zone_name": "status-bar",
  "content": {"type": "status_bar", "entries": {"build": "passing", "agent": "ci-bot", "branch": "main"}},
  "merge_key": "build-status",
  "ttl_us": 120000000,
  "namespace": "ci-agent"
}
```

**Notification toast:**
```json
{
  "zone_name": "notification-area",
  "content": {"type": "notification", "text": "Deployment complete: v2.1.0 → production", "urgency": 1},
  "ttl_us": 10000000
}
```

**Ambient color:**
```json
{
  "zone_name": "ambient-background",
  "content": {"type": "solid_color", "r": 0.05, "g": 0.1, "b": 0.2, "a": 1.0}
}
```

#### Success response

```json
{
  "result": {
    "zone_name": "alert-banner",
    "ttl_us": 30000000
  }
}
```

The response echoes the zone name and the effective TTL applied (which may differ from your request if the server applied a default). For ephemeral zones, the response body may be minimal or absent.

#### Error responses and recovery

| Error message | Cause | Fix |
|---|---|---|
| `zone '<name>' not found` | Zone name doesn't exist on this HUD instance | Call `list_zones` and use a name from the result |
| `zone media type mismatch for zone '<name>'` | Your content format doesn't match what the zone accepts | Check the zone's `accepted_media_types` in `list_zones` and use the matching content format from the decision table |
| `content must be non-empty` | Empty string or null content | Provide a non-empty string or typed content object |
| `object content must have a "type" field` | Content is a JSON object but missing `"type"` | Add `"type": "notification"`, `"status_bar"`, `"solid_color"`, or `"stream_text"` |
| `unknown content type "<type>"` | Unrecognized `type` value | Use one of: `stream_text`, `notification`, `status_bar`, `solid_color` |
| `status_bar content must have a non-empty "entries" object` | Empty or missing `entries` in status_bar content | Provide at least one key-value pair in `entries` |
| `zone_name must be non-empty` | Blank zone name | Provide the zone name from `list_zones` |

## Zone Model

Zones are named, runtime-owned publishing surfaces on the display. The HUD runtime handles all layout, rendering, and contention — agents just publish content by zone name.

### Default Zones (typical HUD instance)

| Zone | Accepts | Contention | Use For |
|------|---------|-----------|---------|
| `alert-banner` | `ShortTextWithIcon`, `StreamText` | Replace | Important alerts, one at a time |
| `subtitle` | `StreamText` | LatestWins | Live captions, transient text |
| `status-bar` | `KeyValuePairs` | MergeByKey | Persistent key-value status entries |
| `notification-area` | `ShortTextWithIcon` | Stack (max 8) | Toast notifications, auto-dismiss |
| `ambient-background` | `SolidColor`, `StaticImage`, `VideoSurfaceRef` | Replace | Background color/media |
| `pip` | `SolidColor`, `StaticImage`, `VideoSurfaceRef` | Replace | Picture-in-picture overlay |

Zone names and configurations are instance-specific. Always call `list_zones` to discover what's available.

### Contention Policies

| Policy | Behavior | Example Zone |
|--------|----------|-------------|
| `LatestWins` | Most recent publish replaces previous | `subtitle` |
| `Replace` | Single occupant; new publish evicts current | `alert-banner`, `ambient-background`, `pip` |
| `Stack` | Publishes accumulate in a queue, auto-dismiss by TTL | `notification-area` |
| `MergeByKey` | Same `merge_key` replaces; different keys coexist | `status-bar` |

### TTL

All TTL values are in **microseconds** (not milliseconds).

| Human Duration | `ttl_us` Value |
|---------------|----------------|
| 5 seconds | `5000000` |
| 30 seconds | `30000000` |
| 1 minute | `60000000` |
| 5 minutes | `300000000` |

`ttl_us: 0` or omitted = server default (typically 60 seconds).

### Ephemeral vs Durable

- **Ephemeral zones** (`ephemeral: true`): Fire-and-forget. The publish call may not return a result body. Content auto-clears after TTL.
- **Durable zones** (`ephemeral: false`): Transactional. The publish call returns `{"zone_name": "...", "ttl_us": ...}`. Content persists until replaced or TTL expires.

## Common Mistakes

- **Hardcoding zone names** — Always call `list_zones` first. Zone names are instance-specific.
- **Using milliseconds for TTL** — TTL is in microseconds. `60000` is 60ms, not 60 seconds. Use `60000000` for 60 seconds.
- **Sending plain string to a non-StreamText zone** — Status bar, notification, and color zones require typed content objects. Check the decision table.
- **Omitting `merge_key` on a MergeByKey zone** — Without a merge key, the entry has no stable identity and cannot be updated in place.
- **Publishing wrong content type** — A `notification` object to a `status_bar` zone will fail. Match content `type` to the zone's `accepted_media_types`.

## Batch Publish Script

For batch operations or diagnostics outside of MCP tool calls, use the bundled script:

```bash
# List available zones
python3 .claude/skills/th-hud-publish/scripts/publish.py \
  --url http://<HUD_HOST>:9090 \
  --psk-env HUD_MCP_PSK \
  --list-zones

# Publish a single message inline
python3 .claude/skills/th-hud-publish/scripts/publish.py \
  --url http://<HUD_HOST>:9090 \
  --psk-env HUD_MCP_PSK \
  --zone alert-banner \
  --content "Build passed"

# Publish structured content inline
python3 .claude/skills/th-hud-publish/scripts/publish.py \
  --url http://<HUD_HOST>:9090 \
  --psk-env HUD_MCP_PSK \
  --zone status-bar \
  --content '{"type":"status_bar","entries":{"build":"passing"}}' \
  --merge-key build-status

# Publish a batch from file
python3 .claude/skills/th-hud-publish/scripts/publish.py \
  --url http://<HUD_HOST>:9090 \
  --psk-env HUD_MCP_PSK \
  --messages-file /tmp/messages.json
```

Message file format (all content types supported):
```json
[
  {
    "zone_name": "alert-banner",
    "content": "Deploy started",
    "ttl_us": 30000000
  },
  {
    "zone_name": "status-bar",
    "content": {"type": "status_bar", "entries": {"build": "passing", "agent": "ci-bot"}},
    "merge_key": "build-status",
    "ttl_us": 120000000
  },
  {
    "zone_name": "notification-area",
    "content": {"type": "notification", "text": "Deploy complete", "urgency": 1}
  },
  {
    "zone_name": "ambient-background",
    "content": {"type": "solid_color", "r": 0.05, "g": 0.1, "b": 0.2, "a": 1.0}
  }
]
```

**Compatibility**: tze_hud v1 MCP guest surface. Protobuf source of truth: `crates/tze_hud_protocol/proto/types.proto`.
