# Epic 9: Scene Events

> **Dependencies:** Epic 0 (EventRouter trait contract), Epic 6 (session subscriptions), Epic 8 (policy filters events)
> **Depended on by:** Epic 11 (shell responds to events), Epic 12 (integration tests)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/scene-events/spec.md`
> **Secondary specs:** `session-protocol/spec.md` (subscription categories), `policy-arbitration/spec.md` (interruption classes)

## Prompt

Create a `/beads-writer` epic for **scene events** — the event taxonomy, dispatch, and delivery system that makes the platform observable and interactive for agents.

### Context

Scene events cover three categories (input, scene, system), carry interruption classification, respect quiet hours, and use a dotted namespace naming convention. The naming grammar is: `scene.<domain>.<verb>` for runtime scene events, `system.<domain>.<verb>` for system events, and `agent.<namespace>.<bare_name>` for delivered agent events (agents emit bare names like `doorbell.ring`; runtime prefixes with `agent.<namespace>.`). Epic 0 provides the `EventRouter` trait contract.

### Epic structure

Create an epic with **4 implementation beads**:

#### 1. Event envelope and naming convention (depends on Epic 0 EventRouter contract)
Implement the event envelope and naming per `scene-events/spec.md` Requirement: SceneEvent Envelope, Requirement: Event Type Naming Convention.
- SceneEvent envelope: event_type (string), timestamp_wall_us, timestamp_mono_us, payload, source namespace, interruption_class
- Naming: `scene.<domain>.<verb>`, `system.<domain>.<verb>`, `agent.<namespace>.<bare_name>`
- Agents emit bare names; runtime prefixes with `agent.<namespace>.`
- Reserved prefixes (`system.`, `scene.`) rejected for agent-emitted events
- **Acceptance:** EventRouter trait tests from Epic 0 pass. Naming convention validated for all event types. Reserved prefix rejection verified.
- **Spec refs:** `scene-events/spec.md` Requirement: SceneEvent Envelope, Requirement: Event Type Naming Convention, lines 35-47

#### 2. Subscription filtering and delivery (depends on #1, Epic 6 subscriptions)
Implement subscription-based event delivery per `scene-events/spec.md` Requirement: Subscription Model.
- 9 subscription categories: SCENE_TOPOLOGY, INPUT_EVENTS, FOCUS_EVENTS, DEGRADATION_NOTICES, LEASE_CHANGES, ZONE_EVENTS, TELEMETRY_FRAMES, ATTENTION_EVENTS, AGENT_EVENTS
- DEGRADATION_NOTICES and LEASE_CHANGES always subscribed (not filterable)
- Other categories filtered by agent's granted capabilities
- Self-event suppression: agents do not receive their own emitted events
- **Acceptance:** Category-to-capability mapping matches spec. Mandatory categories always delivered. Self-suppression verified. Unsubscribed categories not delivered.
- **Spec refs:** `scene-events/spec.md` Requirement: Subscription Model, `session-protocol/spec.md` Requirement: Subscription Management

#### 3. Interruption classification and quiet hours (depends on #1, Epic 8 attention)
Implement interruption handling per `scene-events/spec.md` Requirement: Interruption Classification, Requirement: Quiet Hours.
- 5 classes: CRITICAL, HIGH, NORMAL, LOW, SILENT
- Zone ceiling: runtime applies the more restrictive of agent-declared class and zone's ceiling
- Quiet hours: CRITICAL/HIGH pass through; NORMAL/LOW queued until quiet hours end
- Queued events delivered in original order when quiet hours end
- **Acceptance:** Classification matches zone ceiling. Quiet hours queue/deliver behavior verified. CRITICAL bypasses everything.
- **Spec refs:** `scene-events/spec.md` Requirement: Interruption Classification, Requirement: Quiet Hours

#### 4. tab_switch_on_event and agent event emission (depends on #1, #2)
Implement agent event capabilities per `scene-events/spec.md` Requirement: Agent Event Emission, Requirement: tab_switch_on_event.
- Agent emission requires `emit_scene_event:<event_name>` capability
- `tab_switch_on_event` matches the bare emitted agent event name (before namespace prefixing)
- Regex validation: bare names must match `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+`
- Rate limiting per agent for event emission
- **Acceptance:** Capability-gated emission verified. tab_switch_on_event triggers tab switch on matching event. Regex validation rejects malformed names.
- **Spec refs:** `scene-events/spec.md` Requirement: Agent Event Emission, Requirement: tab_switch_on_event Contract

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `scene-events/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference exact spec scenarios
3. **Acceptance criteria** — which Epic 0 EventRouter tests must pass
4. **Crate/file location** — event system module in scene or runtime crate
5. **Naming grammar** — use concrete examples from spec scenarios, not abstract templates

### Dependency chain

```
Epics 0+6+8 ──→ #1 Envelope/Naming ──→ #2 Subscription Filtering
                                    ──→ #3 Interruption/Quiet Hours
                                    ──→ #4 Agent Emission/Tab Switch
```
