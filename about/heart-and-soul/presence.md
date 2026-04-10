# Presence

Presence is the core abstraction of this system. An LLM does not "display content." It occupies territory on a living surface, holds it over time, and negotiates its use with the runtime and with other agents.

![Scene Hierarchy and Presence Levels](assets/scene-hierarchy_dark.svg)

## Tabs and tiles

Tabs are not browser tabs. They are modes of the environment: Morning, Kitchen, Work, Security, Doorbell Interrupt, Night.

Tiles are not just containers. They are territories with:

- geometry
- z-order
- update policy
- sync-group membership
- input affordances
- latency class
- ownership or lease
- resource budget

Inside tiles live nodes. V1 ships: solid color, text/markdown, static image, and interactive hit region. Future node types include: live video surfaces, camera feeds, transcript strips, text-stream portals, subtitle overlays, canvases, browser surfaces, charts, and agent avatars. The architecture supports adding node types without restructuring (see architecture.md).

The point is not to let an LLM generate pages. The point is to let it hold and negotiate territory on a living surface.

## Scene mutations are atomic

An agent often needs to make cohesive changes: create two tiles, set their content, assign them to a sync group, and switch to a new tab — all as one logical operation. The user should never see intermediate garbage: a half-built layout, a tab with one tile missing, or a flicker of default content before the real content arrives.

Scene mutations are therefore staged and committed atomically. An agent builds a batch of mutations, then commits the batch. The compositor applies the entire batch in one frame. If any mutation in the batch fails validation (invalid tile ID, exceeded budget, lease violation), the entire batch is rejected — no partial application.

This is not optional. Without transactional mutations, every multi-step scene change becomes a race between the agent's update rate and the compositor's frame rate. The Scene Contract RFC will define the batch format, but the principle is: agents never expose intermediate state to the user.

## Zones: the LLM-first publishing surface

Tiles are the low-level primitive: an agent requests a lease, specifies geometry, creates nodes, manages content. This is the full-control path. It is also too much work for the most common operations.

LLM interactivity is the core directive of this project. That means the common path — the path an LLM takes to put content on screen — must be as simple as possible. Zones are that path.

### Definition

A zone is a named, schema-typed, runtime-owned publishing surface that the compositor realizes using one or more managed surfaces and optional adjunct effects (sound, haptics).

Zones are more than rectangles. A notification zone may play a sound on urgent publishes. A subtitle zone on smart glasses may be audio-only with a minimal visual flash. An alert-banner may trigger a haptic pulse. These adjunct effects are part of the zone's rendering policy, not separate systems. The visual surface is the primary output; adjunct effects are policy-governed extensions of it.

### Zone anatomy

A zone has four distinct layers of definition:

- **Zone type.** The schema: accepted media/payload types, contention policy, default rendering policy, default privacy classification ceiling, default interruption class, and whether adjunct effects are available. "Subtitle" and "notification" are zone types.
- **Zone instance.** A zone type bound into a specific tab with a geometry policy and a layer attachment (see below). A "Morning" tab might have an instance of the "notification" zone type anchored to the top-right of the content layer.
- **Publication.** One publish event into a zone instance: content payload, TTL, key (for merge-by-key zones), priority, privacy classification, and optional stream/session identity for ongoing content.
- **Occupancy.** The runtime's resolved current state of a zone instance: what content is visible, which publications are active, what the effective geometry is after layout resolution. Agents can query occupancy but cannot set it directly.

This four-level structure prevents the common confusion of "is a zone the schema or the instance or the content?" The answer is: a zone type defines the schema, a zone instance is placed in a tab, publications push content into instances, and occupancy is what the runtime renders.

### Layer attachment

Every zone instance attaches to a specific compositor layer (see architecture.md, "Compositing model"):

- **Background layer.** ambient-background attaches here. Lowest z-order, behind all tiles. Runtime-owned; agents publish content but do not control positioning.
- **Content layer.** Most zones attach here: subtitle, notification, pip. They are realized as runtime-managed tiles within the content layer's z-order.
- **Chrome layer.** alert-banner and status-bar attach here. They render above all agent content. Agents can publish content to chrome-layer zones, but the content is rendered by the runtime — agents do not "render into chrome." The zone is the mediation layer: the agent publishes data, the runtime renders it in chrome using the zone's rendering policy.

This resolves the apparent contradiction "agents cannot render into chrome" vs "alert-banner renders in chrome." Agents publish to the zone; the runtime renders in chrome. The agent never touches the chrome layer directly.

### What zones are

A zone instance is a named publishing surface with:

- **A name and description.** Human-readable, discoverable by agents. "subtitle", "notification", "status-bar", "picture-in-picture", "ambient-background".
- **A geometry policy.** The zone knows where it goes on screen. The agent does not need to compute coordinates, margins, or aspect ratios. The runtime resolves the zone's geometry based on the current display profile, tab layout, and active zones.
- **Accepted media types.** Each zone declares exactly what content it accepts. A subtitle zone accepts stream-text with breakpoints. A notification zone accepts short text and an optional icon. A picture-in-picture zone accepts a video surface. Publishing the wrong type is a validation error, not a rendering bug.
- **Rendering policy.** Font size, alignment, margins, transitions, timeout behavior — all defined by the zone, not by the agent. The subtitle zone renders centered text in the bottom 5% of the screen with a semi-transparent backdrop. The agent does not specify any of this.
- **Upload/download protocol subset.** A zone can constrain which protocol plane is used for content delivery. A subtitle zone accepts ephemeral-realtime stream-text over gRPC. A video zone requires WebRTC media. This prevents agents from accidentally using the wrong transport for the content type.

### How agents use zones

Publishing to a zone is a single call:

```
publish("subtitle", stream_text("The quick brown fox", breakpoints=[4, 10, 16]))
publish("notification", {text: "Doorbell rang", icon: "doorbell", urgency: "urgent"})
publish("pip", video_surface_ref)
```

No tile creation. No geometry. No z-order. No lease negotiation for the common case. The zone handles all of it internally.

An agent needs a capability grant to publish to a zone (zones are governed like everything else), but once granted, publishing is zero-overhead from the agent's perspective.

Zone publishing is available on both protocol planes: via the gRPC session stream for resident agents, and via MCP tool calls (`publish_to_zone`, `list_zones`) for guest agents. MCP is the natural fit for zone publishing because both are designed for the same thing: zero-context, semantic operations that don't require scene awareness.

### Guest agents and zone leases

When a guest agent publishes to a zone via MCP, the guest does not acquire a lease. The zone's internal tile is owned by the runtime, not by the publishing agent. The guest's content is transient — it lives until the zone's timeout policy clears it, or until another publish replaces it. This is what makes zone publishing safe for guest agents: they contribute content without taking on the obligations (lease renewal, event handling, resource accounting) of residency.

### Zones manage tiles internally

A zone is implemented on top of the tile system, not beside it. When an agent publishes to the subtitle zone, the runtime creates (or reuses) a tile in the overlay layer, positions it according to the zone's geometry policy, and renders the content according to the zone's rendering policy. The agent never sees this tile directly.

This means zones inherit all the properties of the tile system: compositing, z-order, lease governance, telemetry, and headless testability. They are not a parallel rendering path — they are an opinionated wrapper around the existing one.

### Two APIs: orchestrate zones vs. publish to zones

This is an opinionated design choice that follows directly from the project's core directive of LLM interactivity.

**Zone orchestration** is the act of designing, creating, laying out, positioning, sizing, and configuring zones. It requires full scene context: display profile, tab layout, other zones, geometry constraints, media type declarations, rendering policies. Orchestration is a rich, context-heavy operation. It happens infrequently — at tab creation, layout changes, or mode switches.

**Zone publishing** is the act of pushing content to an existing zone by name. It requires almost no context: the zone name, the content (which must match the zone's declared media type), and the delivery semantics. Publishing is a lean, high-frequency operation. It happens continuously — every subtitle update, every notification, every status refresh.

These two operations are deliberately separated into different API surfaces because they have fundamentally different context requirements:

- An orchestrator agent (or the runtime's default configuration) orchestrates zones with full scene awareness. It decides: "the Security tab has a camera-pip zone in the top-right, a subtitle zone at the bottom, and an alert-banner zone at the top."
- A publisher agent publishes to zones with minimal context. It only needs: "publish this subtitle text to 'subtitle'." It does not know or care where the subtitle zone is, how large it is, or what font it uses.

This separation enables much better context management for LLMs. An orchestrating LLM needs a large context window with scene state, display capabilities, and layout logic. A publishing LLM needs a tiny context window with just the zone name and content schema. Different agents can fill different roles — or the same agent can orchestrate once (expensive, infrequent) and publish many times (cheap, continuous).

The orchestration API is transactional (zones are created/modified via atomic scene mutations). The publishing API is fire-and-forget for ephemeral content (subtitles, notifications) and acknowledged for durable content (status-bar values, background images).

### Zone definitions are part of the scene configuration

Zones can be defined statically (loaded from configuration at startup) or dynamically (created by an orchestrator agent at runtime). A "Morning" tab might ship with default zones for subtitle, notification, and ambient-background. An orchestrator agent might later add a weather-ticker zone or reconfigure the notification zone's position.

The zone registry is discoverable: an agent can query "what zones exist in the current tab, what do they accept, and do I have permission to publish to them?" This is critical for LLM agents — they can inspect the available zones and decide what to publish without hardcoding screen geometry or layout assumptions.

### Zone contention policy

When two agents publish to the same zone simultaneously, the zone's contention policy determines the outcome. Each zone type declares its policy:

- **Latest-wins** (subtitle, ambient-background): The most recent publish replaces the previous content. No queue, no merge. This is the right policy for ephemeral content where only the current value matters.
- **Stack** (notification): Publishes accumulate in a queue. Each notification renders independently and auto-dismisses after its timeout. Multiple agents can have active notifications simultaneously.
- **Merge-by-key** (status-bar): Each publish includes a key. Values with the same key are replaced; values with different keys coexist. An agent publishing `{key: "weather", value: "72°F"}` and another publishing `{key: "battery", value: "85%"}` both appear.
- **Replace** (pip): Only one occupant at a time. A new publish replaces the current one. The displaced agent receives a notification that its content was evicted.

The contention policy is part of the zone definition, not an agent choice. Agents do not need to coordinate with each other — the zone handles it.

### Zone geometry adapts to the display profile

The same zone definition produces different geometry on different display profiles. The subtitle zone on a 65" wall display renders larger text with wider margins than on a 6" phone. The picture-in-picture zone on a phone might be 30% of the screen; on a wall display, 10%. The notification zone on smart glasses might be audio-only with a minimal visual flash.

This is the mechanism by which "same scene model, different budgets" becomes concrete for content publishing. An agent publishes to "subtitle" — the runtime makes it look right on whatever display it's running on.

### Relationship to raw tiles

Zones do not replace tiles. They are the easy path for common patterns. An agent that needs custom layout, unusual geometry, or content types that no zone supports still uses the full tile API: request lease, specify geometry, create nodes, manage content.

The expectation is that most agents use zones for most of their output, and only drop to raw tiles for genuinely custom layouts. If an agent frequently needs raw tiles for something that should be a zone, that is a signal to define a new zone type — not to accept API complexity as the default.

A low-latency text interaction portal is a valid example of this escape-hatch path. If the system needs a governed surface for streamed text interaction with humans or LLMs, and no existing zone or widget contract fits, the first correct move is a raw-tile proof under lease governance. The second move, if the pattern proves stable, is a dedicated runtime contract. The incorrect move is to smuggle a generic terminal or chat app host into the compositor.

### Example zones

These are illustrative, not exhaustive. The actual zone registry is defined per deployment.

**subtitle** — Content layer. Bottom of screen, ~5% height, centered, semi-transparent backdrop. Accepts stream-text with breakpoints. Ephemeral-realtime delivery. Auto-clears after timeout. Syncs to media clock if in a sync group. Contention: latest-wins.

**notification** — Content layer. Top-right corner, stacks vertically, auto-dismisses after timeout. Accepts short text + optional icon + urgency level. Default interruption class: normal (not urgent — urgency is declared per-publish by the agent, never assumed by the zone). Adjunct: urgent notifications play a sound; critical notifications play a louder sound. Contention: stack.

**status-bar** — Chrome layer. Thin strip at top or bottom. Accepts key-value pairs rendered as a horizontal row. Coalesced updates (state-stream class). Always visible, never occluded by agent tiles. Contention: merge-by-key.

**pip** (picture-in-picture) — Content layer. Corner-anchored, draggable, resizable within bounds. Accepts a video surface reference. One pip per tab. Contention: replace (displaced agent notified).

**ambient-background** — Background layer. Full-screen behind all tiles. Accepts a static image, color, or slow-cycling gallery. Purely decorative — no input, no interaction. Contention: latest-wins.

**alert-banner** — Chrome layer. Full-width horizontal bar that pushes content down. Accepts text + severity level. Adjunct: critical alerts trigger haptic pulse if available. Used for system-level alerts, weather warnings, security events. Contention: stack by severity.

## Widgets: parameterized visual publishing

Zones accept raw content — text, key-value pairs, colors, images — and the runtime renders that content directly. This works well for information display, but it leaves a gap: there is no way for an agent to publish a *value* and have the runtime render a *visual interpretation* of that value.

Consider a temperature gauge. Without widgets, an agent wanting to display "78% capacity" as a filling bar must drop to the full tile + node tree API: create a tile, compute a `SolidColorNode` at the right height, add a `TextMarkdownNode` for the label, manage z-order and bounds arithmetic, and keep it all alive with lease renewals. This is the exact "LLM in the frame loop" pattern the architecture forbids — the agent is computing geometry, not declaring intent.

Widgets fill this gap by extending the zone pattern to parameterized visuals. A widget is a runtime-owned visual template (SVG layers with parameter bindings) that agents parameterize with simple typed values (numbers, strings, colors, enums). The runtime owns the visual assets, rasterizes at compositor frame rate, and smoothly interpolates between parameter states. The agent publishes `{ fill_level: 0.78, label: "Capacity" }` — the runtime renders a gauge.

### Widget anatomy

Widgets have the same **four-level ontology** as zones:

1. **Widget type** — the schema and visual assets: parameter declarations (name, type, constraints, default), SVG layers with parameter bindings, default geometry policy, contention policy. Widget types can be bootstrapped from asset bundles at startup and can also be registered at runtime via validated SVG upload. In both cases, registration is distinct from publication.
2. **Widget instance** — a widget type bound into a specific tab with geometry and layer attachment. Declared in configuration under `[[tabs.widgets]]`.
3. **Publication** — one publish event into a widget instance: a set of typed parameter values (f32, string, color, or enum), TTL, optional merge key (for MergeByKey contention), and optional transition duration.
4. **Occupancy** — the runtime's resolved render state: effective parameter values after contention policy application. The compositor reads occupancy to determine current visual property values and re-rasterizes only when effective parameters change.

### How agents use widgets

Publishing to a widget is a single call — the same pattern as zone publishing:

```
publish_to_widget("cpu_gauge", { fill_level: 0.72, label: "CPU", fill_color: [66, 133, 244, 255] })
publish_to_widget("status_ring", { severity: "warning" })
```

No tile creation. No geometry. No z-order. No SVG knowledge required. The widget handles all visual rendering internally.

Widget publishing is available on both protocol planes: via `WidgetPublish` (ClientMessage field 35) on the gRPC session stream for resident agents, and via the `publish_to_widget` MCP tool for guest agents. `list_widgets` (MCP) returns all available widget types and instances with their parameter schemas — agents use it for discovery without hardcoding widget names or parameter types.

### Relationship to zones and raw tiles

Widgets are the third publishing abstraction, sitting between zones (raw content rendering) and raw tiles (full compositor control):

- **Zones** — for raw content where the zone's built-in rendering policy is sufficient: text, notifications, status values, images. The agent provides content; the zone provides the visual treatment.
- **Widgets** — for parameterized visuals where the agent provides values and the runtime provides the visual template: gauges, progress bars, dials, severity indicators. The agent provides parameters; the widget's SVG layers provide the visual treatment.
- **Raw tiles** — for genuinely custom layouts, unusual geometry, or content types that no zone or widget supports. Full compositor access; full agent responsibility.

The expectation is that most display needs are covered by zones and widgets. Raw tiles are the escape hatch, not the default path.

### Widget governance

Widgets reuse the entire zone governance model without forking it:

- **Contention:** LatestWins, Stack, MergeByKey, Replace — same four policies, same semantics.
- **Geometry:** same `GeometryPolicy` types (relative or edge-anchored) and display-profile adaptation.
- **Layer attachment:** Background, Content, or Chrome — same three layers. Widget tiles use z-order `>= WIDGET_TILE_Z_MIN` (0x9000_0000), which places them above zone tiles (0x8000_0000) when they overlap spatially.
- **TTL and expiry:** publications carry `ttl_us` with the same semantics as zone publications.
- **Capability:** publishing to a widget requires `publish_widget:<widget_name>` capability, structurally identical to `publish_zone:<zone_name>`. The `publish_widget:*` wildcard grants all widgets.
- **Guest publishing:** guests do not acquire leases. Widget tiles are runtime-owned. Guest publications persist until the TTL expires or another publication replaces them.

Widget tiles default to `input_mode = Passthrough` — they are visual indicators, not interactive surfaces. Input events pass through widget tiles to the tiles beneath them.

### Widget assets: bootstrapped + runtime

Widget visual assets remain user-authorable directories on disk for bootstrap. Each bundle contains a `widget.toml` manifest (parameter schema, layer references, binding declarations) and one or more SVG files. The runtime scans configured bundle directories at startup, registers valid widget types, and logs errors for invalid or duplicate bundles without halting startup.

The runtime also supports runtime SVG registration through a separate upload/registration call path. This keeps the control model two-stage:

1. **Register asset** (startup bundle scan or runtime upload/register).
2. **Publish parameters/content** against a widget instance or SVG-capable surface.

This split is intentional: publish calls stay chatty, low-latency, and low-bandwidth because they reference already-registered assets instead of re-sending SVG markup.

Runtime asset registration is content-addressed. Upload/register calls include an expected content hash (BLAKE3) so the runtime can deduplicate by identity: if the hash already exists in the local asset store, the runtime returns the existing asset handle and skips payload transfer. Optional transport integrity checksums (for example CRC32) may be accepted as a fast corruption guard, but deduplication and identity use the strong content hash.

Runtime-uploaded SVG assets are persisted in a runtime-managed local file store and rehydrated on startup. The persistence backend is OS-specific (Linux, macOS, Windows) but semantically identical: content-addressed blobs, atomic writes, and crash-safe startup reindexing.

## Component shape language: visual identity as a swappable layer

Zones define *behavior* — where content appears, how contention resolves, what media types are accepted. Widgets define *structure* — SVG layers with parameter bindings. Neither defines *visual identity*. Without a shared vocabulary for colors, typography, outlines, and backdrops, every zone rendering policy and widget bundle is a visual island. Two independently-authored subtitle implementations will look different, and swapping one for the other requires re-authoring zone types, rendering policies, and widget bundles from scratch.

The component shape language fills this gap. It is a visual identity layer that sits above zones and widgets, giving the HUD a coherent, professional appearance while keeping every visual component modular and swappable.

### Design tokens

A **design token** is a named visual primitive: a color, a font size, a stroke width, an opacity value. The runtime loads tokens from a flat `[design_tokens]` section in the configuration file — a simple key-value map with dotted namespace conventions:

```toml
[design_tokens]
"color.text.primary" = "#FFFFFF"
"color.backdrop.default" = "#000000"
"opacity.backdrop.default" = "0.6"
"typography.subtitle.size" = "28"
"stroke.outline.width" = "2"
```

Tokens flow into two rendering paths:
- **Zone rendering** (glyphon + wgpu quads): tokens are parsed into typed `RenderingPolicy` fields at startup. The compositor reads `policy.text_color`, `policy.backdrop`, `policy.outline_width` instead of hardcoded values.
- **Widget rendering** (SVG + resvg): tokens are injected into SVG templates via `{{token.key}}` placeholders, resolved by text substitution at asset registration time (startup bundle load or runtime upload/register) before SVG parsing.

The canonical token schema defines ~28 required keys with fallback values covering colors, typography, spacing, and strokes. Operators override any token in configuration. Non-canonical keys are accepted for profile-specific use. All tokens are resolved once at startup and are immutable during the runtime lifecycle.

### Component types and profiles

A **component type** is a named contract that defines "what it means to be a subtitle" (or a notification, or an alert-banner):

- Which zone type it governs
- What readability technique is required (dual-layer, opaque backdrop, or none)
- Which specific design tokens must be resolvable
- Informal geometry expectations

V1 defines six component types matching the six built-in zone types: `subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, `pip`.

A **component profile** is a user-authored implementation of a component type. It is a directory containing:

- `profile.toml` — name, version, component type, optional token overrides
- `zones/` — rendering policy overrides for the governed zone type
- `widgets/` — optional widget bundles for enhanced visuals

Profiles are the **swappable unit**. My subtitle profile renders white-on-black outlined text with 60% backdrop opacity. Yours renders yellow-on-navy with a solid background. Both conform to the `subtitle` component type contract. The operator switches between them in configuration:

```toml
[component_profiles]
subtitle = "cinematic-subs"
notification = "clean-notifs"
```

Agents are unaffected — they still publish to `"subtitle"` by name. The visual treatment changes without any agent code change. This is the plug-and-play contract.

### Readability enforcement

The HUD renders over arbitrary window content. White text on a white background is invisible. The component shape language enforces readability structurally — at startup, not at render time:

- **Subtitle** requires dual-layer readability: a backdrop quad AND text outline (8-direction offset rendering). Both are validated on the effective `RenderingPolicy` at startup.
- **Notification** and **status-bar** require opaque backdrop (opacity >= 0.8).
- **Widget SVGs** in text-bearing profiles must use `data-role="backdrop"` and `data-role="text"` attributes with correct document order and stroke requirements.

In production builds, readability violations reject the profile. In development builds (`TZE_HUD_DEV=1`), violations are logged as warnings to enable iterative authoring.

### Extensibility principle

The component shape language is designed for ecosystem growth. The pattern is:

1. **The runtime defines the contract** (component type: governed zone, readability, required tokens).
2. **Authors implement the contract** (component profile: rendering overrides, widget bundles, token overrides).
3. **Operators select the implementation** (configuration: which profile is active per component type).

This three-layer separation means visual identity is never hardcoded in the runtime. The runtime ships with token-derived defaults that produce a clean, readable appearance out of the box. But every aspect of that appearance — colors, fonts, outlines, backdrops, transitions — is overridable through profiles without modifying runtime code or agent behavior. New component types can be added post-v1 as new zone types are introduced, following the same pattern.

---

## Presence levels

Not every agent needs the same degree of embodiment. Presence level and agent role are orthogonal axes:

- **Presence level** (guest / resident / embodied) governs trust, transport, and resource access.
- **Agent role** (publisher, orchestrator, sensor-producer, interactive-app, etc.) governs what the agent is trying to do.

A resident agent can be a zone publisher, an orchestrator, or a dashboard producer. An embodied agent might only publish to a single zone. The capability system grants permissions; presence level determines the trust ceiling and available transport. Do not conflate them.

### Guest presence

The agent performs one-off actions: show note, open tile, display image, dismiss overlay. This is the natural fit for MCP-style tool use. No persistent connection required. Minimal trust required.

### Resident presence

The agent holds a long-lived session: subscribes to scene state, receives events, updates surfaces continuously, keeps ownership of one or more regions. This requires persistent gRPC streams. Moderate trust required — the agent has ongoing resource consumption and event access.

### Embodied presence

The agent has resident presence plus timed media and bidirectional interaction: streams audio/video, receives touch or button events, aligns text or highlights to media clocks, participates as a live entity. This requires separate media and control planes. Highest trust required — the agent has real-time media access and interactive capabilities.

## Leases: presence requires governance

If an LLM is a first-class citizen, it must also be a governed citizen.

Every resident or embodied agent receives:

- a namespace
- one or more surface leases
- capability scopes (what it can do)
- TTL and renewal semantics
- resource budgets (memory, bandwidth, update rate)
- allowed z-order or overlay privileges
- event subscriptions (what it can observe)
- revocation semantics (how and when the runtime can take it back)

This prevents the system from becoming attention spam. The human must always be able to: dismiss, mute, pin, freeze, shrink, revoke. The runtime must always remain sovereign.

## Multi-agent coordination

A presence engine that hosts multiple agents is not just "multiple tenants on a shared screen." Agents may need to be aware of each other, coordinate, and interact.

### Visibility

Topology visibility is policy-driven (see privacy.md for the canonical rule). By default, agents see only their own leases and the public structure of the scene (tab names, tile geometry) — not which other agents hold which leases. Agents with an explicit topology-read capability grant can see the full scene topology including other agents' lease metadata. No agent can see the content of another agent's tiles or event streams regardless of capability level.

An agent can publish selected state to a shared namespace if it chooses. This is opt-in, not default.

### Negotiation

Agents do not negotiate territory directly with each other. All negotiation goes through the runtime. An agent requests a region; the runtime grants, denies, or counter-offers based on available space, budgets, priorities, and the current lease map.

If two agents want overlapping territory, the runtime arbitrates. Agents can express preferences (preferred region, minimum acceptable size, priority hint) but the runtime decides.

### Orchestration

The runtime supports an optional orchestrator role: a privileged agent that can manage other agents' presence. An orchestrator can:

- suggest or enforce tab layouts
- create, modify, and remove zones (zone orchestration — see "Two APIs" above)
- invite or dismiss sub-agents
- coordinate scene transitions (e.g., "switch to Security mode: bring up cameras, dismiss ambient dashboard")
- act as a meta-agent that composes agent presences into coherent experiences

An orchestrator holds elevated lease privileges but is still subject to the runtime's sovereignty. The human can always override.

### Inter-agent events

Agents can subscribe to a shared event bus for coarse-grained coordination signals: "tab switched," "new agent joined," "agent departed," "user dismissed tile," "scene entering degraded mode." These are scene-level events, not agent-to-agent messages. Direct agent-to-agent communication is out of scope for the presence engine — agents that need to talk to each other do so through their own channels outside the runtime.

## Interaction

Two-way interaction is mandatory. The system supports: touch, pointer, buttons, local keyboard/mouse, voice triggers, sensor-initiated interrupts, and app-to-agent callbacks.

The interaction model is local-first. The human should never feel like they are "clicking through a cloud roundtrip." A pressed state, focus ring, or visual acknowledgement happens locally and instantly. Remote semantics follow. Local feedback cannot wait.

### Focus model

Focus is per-tile, not per-agent. At most one tile has keyboard/text focus at any time. An agent that owns multiple tiles has focus in at most one of them. The runtime manages focus transitions — an agent cannot forcibly steal focus from another agent's tile.

Focus is also local-first: the runtime updates the visual focus indicator (ring, highlight, cursor) immediately on input, before notifying the agent. The agent learns it received focus via an event, not by observing a visual change.

### Input routing and bubbling

Input events are routed through the scene graph. A pointer event first hit-tests against the chrome layer (runtime UI always wins), then against tiles in z-order (highest first), then against nodes within the hit tile.

If a node or tile does not handle an event, it bubbles up: node → tile → tab → runtime. This means an unhandled click on a text node inside a tile can be caught by the tile's hit region, or by the runtime's default behavior (e.g., focus the tile).

### Gesture arbitration

When two tiles could plausibly claim the same gesture (a swipe that crosses a tile boundary, a pinch that starts in one tile and ends in another), the runtime arbitrates. The default policy is: the tile where the gesture started owns it. The Input RFC will define the full arbitration model, but the principle is that the runtime decides — agents do not race for gestures.

### IME, text input, and accessibility

Text input, input method editors (IME), and accessibility hooks (screen readers, switch access) are acknowledged as first-class requirements. They are not afterthoughts to be bolted on later. The Input RFC must address them. The doctrine-level commitment is: the runtime exposes accessibility metadata for the scene graph (tile names, node roles, focus state) through platform accessibility APIs. Agents declare semantic roles for their nodes; the runtime bridges to the platform.
