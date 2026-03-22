# Attention Governance

tze_hud is a presence engine. Presence is not the same as attention capture.

These two things are easy to conflate and fatal to confuse. A presence engine that exploits attention — that designs for engagement, competes for eyeballs, or escalates to stay visible — has betrayed its purpose. An agent should be able to inhabit a screen without the screen becoming a weapon against the person in the room.

This document states the philosophical stance. Implementation detail (viewer context, content classification, interruption classes, quiet hours) lives in [privacy.md](privacy.md).

## Core Principle

**The runtime governs attention. It does not exploit it.**

Attention is a finite resource belonging to the viewer, not to agents and not to the runtime. The system's job is to present information when it is wanted and stay out of the way when it is not.

Agents request presence. They do not demand attention. The viewer decides what they notice. The runtime ensures that decision is always available to them.

## Attention Budget

Every screen has finite attention capacity. Interruptions are withdrawals from that budget. A screen that interrupts constantly — even with accurate, useful information — becomes noise. The viewer stops seeing it.

This is not a hypothetical risk. It is the failure mode of every notification system that has ever existed at scale.

The runtime must treat attention budget as a real constraint:

- Not every update needs to be an interruption. Silent and gentle updates exist for a reason.
- Urgency must be earned. An agent that escalates everything is an agent that means nothing.
- Interruption rate is a signal. High interruption rate from an agent or zone is a sign of misconfiguration, not effectiveness.
- The budget resets, but slowly. A flurry of urgent interruptions does not make the next one more welcome.

The interruption class system in [privacy.md](privacy.md) is the mechanism. This is the principle that motivates it.

## Anti-Patterns of Attention

These behaviors are explicitly rejected. Any design that enables them is a design failure:

**Notification spam.** Publishing frequent, low-value notifications to stay visible or indicate activity. The screen is not a heartbeat monitor for agents. An agent that needs to interrupt every few seconds is doing it wrong.

**Escalation creep.** Marking content as urgent because it might be urgent, or because the agent wants priority. Urgency inflation destroys the signal. When everything is urgent, nothing is.

**Engagement dark patterns.** Designing for time-on-screen, maximizing user attention, or triggering emotional responses to retain engagement. The presence engine is not a social media feed. These incentives are structurally incompatible with the product's purpose.

**Cognitive overload.** Filling available screen space because it is available. Empty space is not wasted attention — it is breathing room. Agents that pack content to the edges are competing with the environment.

**Interruption escalation.** Repeatedly sending the same content with increasing urgency because the user has not responded. Silence is a response. The user is allowed to ignore an agent.

**Invisible persistence.** Holding territory or presence without rendering anything useful, purely to maintain lease priority or block other agents. Presence must justify itself with content, not with occupation.

## Design Principles for Attention

**Ambient over interruptive.** The default mode of presence is ambient — content that can be noticed when the viewer chooses to look, not content that demands to be noticed. Agents should default to the quietest interruption class that still conveys their information.

**Quiet by default.** No agent should interrupt by default. Interruption is opt-in by the viewer or explicitly configured. An unconfigured agent is a silent agent.

**Earned urgency.** Urgency is a promise: "this requires your attention right now." Breaking that promise — marking non-urgent content urgent — degrades the entire interrupt system. Urgent is not a style choice. Urgent means the viewer will regret ignoring it.

**Viewer autonomy.** The viewer can dismiss, mute, shrink, freeze, or revoke any agent at any time without friction. This is in [security.md](security.md) (human override) and in [privacy.md](privacy.md) (quiet hours). The doctrine point is that viewer autonomy is not a safety valve — it is the primary design constraint.

**Attention telemetry for governance only.** The runtime may observe attention-related signals (glance detection, interaction rate, dismissal rate) for the sole purpose of enforcing attention budgets and configuring quiet hours. This data is never shared with agents and never used to optimize for engagement. The runtime governs attention; it does not profile it.

## Relationship to Existing Doctrine

**vision.md** names the failure mode directly: "Not a notification engine. If the primary experience becomes a stream of notifications, the product has failed." This document is the principled expansion of that non-goal.

**presence.md** establishes the lease model and names "attention spam" prevention as the motivation for resource governance and revocation semantics. Leases are the mechanism; attention governance is why they exist.

**privacy.md** provides the implementation primitives: viewer context, content classification, interruption classes, and quiet hours. That document describes how the runtime enforces attention governance for household surfaces and shared viewers. Read it for the mechanism; read this document for the stance.

**security.md** establishes human override as absolute. The attention governance principle extends this: override must be frictionless because the viewer's right to control their own attention is not a feature to be gated behind complexity.

**failure.md** commits to "what the user always sees" — a responsive screen, working tab switching, always-available safe mode. These guarantees exist even under agent failure. The same commitment applies to attention overload: the screen must remain navigable even if agents are misbehaving or misconfigured.
