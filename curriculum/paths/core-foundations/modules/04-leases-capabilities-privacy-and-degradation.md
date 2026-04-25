# Leases, Capabilities, Privacy, and Degradation

- Estimated smart-human study time: 7 hours
- Keep every module at or below 10 hours.

## Why This Module Matters

This repo’s governance model is more than auth. Safe changes require understanding who is allowed to do what, what happens when that authority changes, how viewer privacy affects rendering, and how the runtime degrades under stress without giving up sovereignty.

## Learning Goals

- Explain lease and capability semantics well enough to follow admission code.
- Distinguish agent authorization from viewer-facing privacy policy.
- Understand human override, quiet hours, redaction, and degradation ordering.

## Subsection: Governance as Runtime Behavior

### Why This Matters Here

Many codebases treat security, privacy, and resource pressure as separate concerns. `tze_hud` treats them as one ordered governance surface because all three affect what may appear on screen and what the runtime must do under pressure.

### Technical Deep Dive

The core concepts are:
- authorization: whether an agent may perform an action
- tenancy/isolation: whether one actor may affect another actor’s state
- privacy: whether content may be shown to the current viewer
- attention management: whether an update should interrupt now
- degradation: how the system reduces cost without losing control

Leases encode governed occupancy over time. Capabilities are additive grants and can be revoked. Privacy is separate from agent authorization because a fully authorized agent can still have its content redacted for the current viewer. Attention and degradation add two more runtime-owned decisions: even valid content may be delayed, simplified, or suppressed according to policy.

The most transferable systems lesson is that governance layers need a clear precedence order. Without that, behavior under conflict becomes ad hoc and unsafe. Here, human override sits at the top, and runtime-owned responses win over agent preferences.

### Where It Appears In The Repo

- `about/heart-and-soul/security.md`
- `about/heart-and-soul/privacy.md`
- `about/heart-and-soul/failure.md`
- `openspec/specs/lease-governance/spec.md`
- `openspec/specs/policy-arbitration/spec.md`
- `crates/tze_hud_protocol/src/session_server.rs`

### Sample Q&A

- Q: Why is privacy not the same as agent authorization?
  A: Authorization decides whether the agent may publish or mutate; privacy decides whether the current viewer may see the resulting content.
- Q: Why does the repo insist that human override always wins?
  A: Because the system is meant to remain trustworthy on a shared physical screen; no agent may block dismissal, safe mode, or revocation.

### Progress

- [ ] Exposed: I can define lease, capability, viewer class, redaction, and degradation
- [ ] Working: I can explain how authorization and privacy differ
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can name one policy interaction that must have a fixed precedence order

### Mastery Check

Target level: `working`

You should be able to explain why policy in this repo means more than “security checks” and identify the main runtime-owned governance surfaces.

## Module Mastery Gate

- [ ] I can summarize lease and capability behavior without notes
- [ ] I can explain redaction and viewer-class logic
- [ ] I can explain what degradation is trying to preserve
- [ ] I can point to the main policy and lease specs

## What This Module Unlocks Next

It makes zone/widget publishing and resource ownership understandable, because those surfaces are governed by the same capability, privacy, and lifecycle rules.

