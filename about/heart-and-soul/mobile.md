# Mobile and Smart Glasses

> **DEFERRED INDEFINITELY (2026-05-09).** Mobile and smart-glasses deployment profiles are parked. The project has refocused on a performant single-device Rust HUD runtime for Windows. Mobile capability negotiation hooks remain in the schema as documented in `v1.md`'s deferral list, but **no mobile- or glasses-specific implementation, doctrine evolution, or beads** are admitted until the single-Windows runtime is done. The "first-class from the start" framing below is superseded; treat it as historical context, not as an active requirement. See `openspec/changes/windows-first-performant-runtime/` and epic `hud-9wljr`.
>
> Original mobile doctrine follows.

---

The system must support a mobile deployment profile from the start. Not as an afterthought. Not as a separate product. As a first-class requirement.

Mobile presence is not "desktop presence on a smaller screen." It is a degraded but compatible operating profile within the same architecture.

## Two profiles, one model

The system supports two deployment profiles from day one:

**Full Display Node.** A high-end local display appliance: powerful local GPU, large always-on screen (mirror, monitor, wall display), local or near-local networking, up to 10 Gbps links to media/agent producers, multiple concurrent live feeds, rich multimodal scenes, long-lived resident agents.

**Mobile Presence Node.** A degraded but compatible mobile target: high-end phone or smart-glasses-class device, variable 5G network conditions, smaller display and tighter interaction budget, tighter thermal/battery/memory/decoder limits, reduced concurrent media capacity, narrower feature surface.

The system must not fork into a "desktop architecture" and a "mobile architecture." One scene model, one API. Differences are negotiated capabilities and budgets, not separate codepaths.

## Mobile design principles

### Same scene model, different budgets

Do not invent a separate API for mobile. Keep one scene model and negotiate:

- max concurrent streams
- max update rate
- max texture/surface count
- allowed node types
- allowed resolution tiers
- input capabilities
- local cache capacity
- background/foreground behavior

### Overlay-first

A glasses or phone surface should prioritize:

- one primary live element
- a few small glanceable companions
- short-lived overlays
- compact transcript strips
- minimal chrome
- low cognitive load

Mobile presence is about immediacy, not density.

### Zones as the mobile-first publishing primitive

Zones (see presence.md) are the ideal mobile surface. An agent publishes to "subtitle" — the mobile runtime's zone geometry policy handles the reduced screen size, larger relative text, and tighter margins automatically. The agent's code is identical on desktop and mobile; only the zone's geometry policy differs.

This is the concrete mechanism by which "same scene model, different budgets" works for content publishing. On a 65" wall display, the notification zone stacks in the top-right. On a phone, it may be a full-width banner at the top. On smart glasses, it may be an audio cue with a minimal visual flash. The agent publishes the same notification to the same zone name — the runtime adapts.

Mobile deployments should prefer zones over raw tiles for most content. Raw tiles require the agent to know geometry, which varies dramatically across mobile devices. Zones abstract that away.

### Semantic updates over visual churn

On mobile, the system should prefer:

- scene diffs over full re-renders
- coalesced state over raw event floods
- stable layout over frequent reflow
- timed cues over chatty text mutation
- pre-baked overlays when necessary

### Explicit degradation

The runtime degrades along known axes (see failure.md for the full degradation ladder):

- fewer simultaneous video feeds
- lower frame rate and resolution
- reduced dashboard update cadence
- reduced visual effects
- fewer browser surfaces
- simplified transitions
- audio-first fallback
- server-side or upstream precomposition when justified

This is not failure. This is the product behaving correctly under a different operating envelope.

## Mobile protocol requirements

The mobile profile keeps the same three-plane architecture, with stricter policies.

**MCP.** Still valid for compatibility and semantic tool entry points. No changes.

**gRPC.** Still the resident control plane. Mobile sessions add: aggressive coalescing, subscription scoping, interest management, backpressure, resumable state sync, and lightweight diffs instead of verbose churn.

**WebRTC.** Still the media plane. Mobile defaults to: one primary live stream, optional thumbnail/auxiliary streams, adaptive quality, reduced parallel decodes, low-latency interaction over maximal fidelity.

## Optional upstream composition

On the full display node, composition happens locally.

On mobile, the system allows an optional mode where upstream services or the paired home display node precompose certain layers or reduce the scene before delivery. This is especially important for glasses-class devices where display area, battery, and decoder count are all constrained.

This is a capability negotiation, not a separate architecture. The mobile node advertises its constraints; the upstream service adapts its output accordingly.
