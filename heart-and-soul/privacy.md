# Privacy and Attention Governance

The security model (security.md) governs agent-to-runtime trust: authentication, capabilities, isolation, budgets. That is necessary but not sufficient.

A presence engine that runs on a household surface — a wall display, a bathroom mirror, a kitchen screen — also needs viewer-centric policy. The question is not just "what can this agent do?" but "what should appear on this screen right now, given who might be looking at it?"

This is not a feature. It is part of the product's spirit. A presence engine that cannot govern attention and privacy on a shared physical surface is not trustworthy enough to put in a home.

## The problem

A smart mirror in a hallway is visible to everyone who walks by: the homeowner, their partner, their kids, guests, the plumber. Different viewers should see different things. Some information is sensitive. Some is distracting. Some is inappropriate for children. Some is simply irrelevant to someone who isn't the primary user.

The runtime must own this decision, not individual agents. An agent that shows a calendar with meeting details does not know who is standing in front of the screen. The runtime does (or at least, the runtime owns the policy for what to do when it doesn't know).

## Viewer context

The runtime maintains a viewer context that informs what the screen shows. Viewer context includes:

**Authentication state.** Is the current viewer identified? How? (Face recognition, proximity badge, phone presence, explicit login, or unknown.) The runtime does not require a specific identification mechanism — it defines a trait for viewer identity and lets the deployment plug in what's appropriate.

**Viewer class.** Based on authentication, the runtime assigns a viewer class:

- **Owner** — full access to all content, all agents, all controls.
- **Household member** — access to shared content, own agents, household-level controls. Cannot see other members' private content.
- **Known guest** — access to guest-appropriate content only. No agent controls. No private information.
- **Unknown / unauthenticated** — the screen shows only ambient, non-sensitive content. Weather, time, art, public transit. Nothing personal.
- **Nobody** — the screen detects no viewer presence. It may dim, show a clock, or go to sleep. Agents are not dismissed — their leases persist — but their content is not rendered.

The viewer class determines what the runtime allows on screen, not what agents choose to show. An agent that holds a lease for a calendar tile will have that tile blanked or redacted when the viewer class drops below the tile's required viewer level.

## Content classification

Every tile and node carries a visibility classification:

- **Public** — visible to all viewer classes. Weather, clock, ambient art, transit info.
- **Household** — visible to owner and household members. Shared calendar, family photos, grocery list.
- **Private** — visible only to the owner (or the specific household member who owns it). Personal messages, financial data, health info, work calendar details.
- **Sensitive** — visible only to the owner with explicit acknowledgement. Security camera feeds, confidential documents, anything that should not be casually glanceable.

Agents declare the visibility classification of their content when creating or updating tiles. The runtime enforces it based on viewer context. If an agent does not declare a classification, the default is **private** — fail closed, not open.

**Zone publishing and classification.** When an agent publishes to a zone, it can declare a classification per-publish. Zones also have a default classification (e.g., notification defaults to "household", ambient-background defaults to "public"). The runtime enforces the more restrictive of the zone default and the agent-declared classification. An agent cannot escalate visibility beyond the zone's ceiling — publishing "public" content to a zone with a "household" ceiling still results in "household" visibility.

## Redaction behavior

When a tile's visibility classification exceeds the current viewer's access:

- The tile is not simply hidden (that would leak the existence of hidden content through layout changes).
- The tile remains in place with its geometry preserved.
- Its content is replaced with a neutral placeholder: a subtle pattern, the agent's name, or a generic icon. The specific redaction style is configurable.
- Interactive affordances on redacted tiles are disabled.

This means the screen layout is stable regardless of who is looking at it. A guest sees the same spatial arrangement as the owner — they just can't see the content of private tiles.

## Interruption classes

Not all content is equally urgent, and not all moments are equally appropriate for interruption.

Every agent interaction that changes the screen — new tile, overlay, notification, tab switch — carries an interruption class:

- **Silent** — updates existing content without any visual disruption. Dashboard refresh, clock tick, ambient rotation.
- **Gentle** — may show a subtle indicator (badge, glow, border change) but does not reflow the screen or grab attention.
- **Normal** — may create new tiles, show overlays, or trigger transitions. Standard agent activity.
- **Urgent** — may override quiet hours, grab focus, play sounds, or expand to larger screen area. Doorbell, security alert, smoke detector.
- **Critical** — overrides everything. Fire alarm, security breach, system failure.

## Quiet hours

The runtime supports configurable quiet-hours policies:

- During quiet hours, only **urgent** and **critical** interruptions are allowed.
- **Normal** and **gentle** updates are queued and delivered when quiet hours end.
- **Silent** updates continue (they are invisible by definition).
- The screen dims or enters a low-information mode.

Quiet hours are a runtime policy, not an agent behavior. Agents do not need to know whether quiet hours are active — they submit content with an interruption class, and the runtime decides whether to present it immediately or queue it.

## Multi-viewer scenarios

When multiple viewers are present with different access levels, the runtime applies the most restrictive policy:

- If an owner and a guest are both present, the screen shows guest-level content. The owner can explicitly override this ("show my calendar anyway") but the default is restrictive.
- If the runtime cannot determine how many viewers are present, it assumes the most restrictive plausible scenario.

This is a conservative default. It can be relaxed per-deployment (a private office display may not need multi-viewer restriction), but the framework must support it.

## Topology visibility

security.md states that agents can see scene topology by default. In a household context, even topology can leak information — knowing that a "Health" tab exists, or that a specific medical agent holds a lease, is itself sensitive.

Topology visibility is therefore policy-driven:

- By default, agents see only their own leases and the public structure of the scene (tab names, tile geometry).
- Agents with explicit topology-read capability can see the full scene topology including other agents' lease metadata.
- The viewer context does not affect topology visibility (that is an agent-to-runtime concern, not a viewer concern).

## What this is not

This is not DRM. It is not access control in the enterprise sense. It is household-scale attention governance: making sure the right content appears at the right time for the right viewer, and that sensitive content does not leak to the wrong eyes on a physically shared surface.

The implementation should be simple, predictable, and overridable by the owner. Complex multi-tenant RBAC is not the goal. The goal is: a guest walks past your smart mirror and sees the weather, not your inbox.
