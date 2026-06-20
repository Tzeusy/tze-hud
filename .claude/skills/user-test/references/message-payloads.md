# Message & Widget Payloads

Payload reference for zone messages (`messages` input) and widget messages
(`widget_messages` input). Used by Workflow Step 3 (publish zone messages) and
Step 4 (publish widget messages) in [../SKILL.md](../SKILL.md).

## Zone message shape

Message shape — `content` is either a plain string (StreamText) or a typed JSON object:

```json
[
  {
    "zone_name": "alert-banner",
    "content": "Deploy v2.1.0 started",
    "ttl_us": 30000000,
    "namespace": "butler-test"
  },
  {
    "zone_name": "subtitle",
    "content": "Running integration tests...",
    "ttl_us": 60000000
  },
  {
    "zone_name": "status-bar",
    "content": {"type": "status_bar", "entries": {"build": "passing", "agent": "butler", "target": "windows"}},
    "merge_key": "build-status",
    "ttl_us": 120000000,
    "namespace": "butler-test"
  },
  {
    "zone_name": "notification-area",
    "content": {"type": "notification", "text": "Build complete", "icon": "", "urgency": 1},
    "ttl_us": 10000000
  },
  {
    "zone_name": "ambient-background",
    "content": {"type": "solid_color", "r": 0.1, "g": 0.15, "b": 0.4, "a": 0.05},
    "ttl_us": 300000000
  },
  {
    "zone_name": "pip",
    "content": {"type": "solid_color", "r": 0.2, "g": 0.8, "b": 0.2, "a": 0.05},
    "ttl_us": 60000000
  }
]
```

**Content types by zone:**
- `alert-banner`, `subtitle`: plain string (StreamText)
- `status-bar`: `{"type":"status_bar","entries":{"key":"value",...}}` with `merge_key`
- `notification-area`: `{"type":"notification","text":"...","icon":"","urgency":0-3,"title":"...","actions":[...]}` (`title` and `actions` optional)
- `ambient-background`, `pip`: `{"type":"solid_color","r":0-1,"g":0-1,"b":0-1,"a":0-1}`

`merge_key`, `ttl_us`, and `namespace` are optional per message.

- `widget_messages`: array of widget publishes (optional)

Widget message shape:

```json
[
  {
    "widget_name": "gauge",
    "params": {"level": 0.75, "label": "CPU Usage"},
    "transition_ms": 500,
    "ttl_us": 60000000,
    "namespace": "user-test"
  },
  {
    "action": "clear",
    "widget_name": "gauge",
    "namespace": "user-test"
  }
]
```

**Widget parameter types:**
- `f32`: JSON number (e.g. `0.75`) — often with min/max range
- `string`: JSON string (e.g. `"CPU Usage"`)
- `color`: JSON object `{"r": 0-1, "g": 0-1, "b": 0-1, "a": 0-1}`
- `enum`: JSON string from allowed values (e.g. `"warning"`)

`transition_ms`, `ttl_us`, `namespace`, and `instance_id` are optional per message.

**`widget_name` semantics: instance name, not type name**

`widget_name` in `publish_to_widget` identifies a *widget instance*, not a widget type.
When the HUD starts, instances are created from `[[tabs.widgets]]` entries in the config,
each with an `instance_id`. That `instance_id` is the string you pass as `widget_name`.

For the production `tze_hud_app` deployment (see `app/tze_hud_app/config/production.toml`):

| `widget_name` | Widget type | What it shows |
|---|---|---|
| `main-gauge` | `gauge` | Vertical fill gauge (level, label, severity) |
| `main-progress` | `progress-bar` | Horizontal progress bar (progress, label) |
| `main-status` | `status-indicator` | Status circle with label (online/away/busy/offline) |

Use `list_widgets` to discover available instances:
```bash
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" --psk-env MCP_TEST_PSK \
  --messages-file /dev/null --list-widgets
```
`list_widgets` returns `widget_instances` (with `instance_name`) — use those names as `widget_name`.
If `list_widgets` returns no instances, the HUD binary is running without a config that declares instances.
