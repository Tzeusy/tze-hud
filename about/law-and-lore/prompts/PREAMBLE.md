# Common Preamble for All Epic Prompts

> Include this preamble at the start of every `/beads-writer` epic creation session.

## Authority and Conflict Resolution

- **Specs are authoritative.** Treat `openspec/changes/v1-mvp-standards/specs/<subsystem>/spec.md` as the source of truth over this prompt text and over summary text in `tasks.md`. If prompt, task, and spec disagree, **do not normalize silently** — add a conflict note in the bead description.
- **Preserve exact canonical names.** Do not invent aliases, synonyms, or paraphrases for state names, message types, capability identifiers, error codes, or event type names that the spec already defines. Use the spec's exact terminology.
- **Doctrine is non-negotiable.** The principles in `about/heart-and-soul/` are architectural invariants, not aspirational goals. Every bead must be compatible with them.

## Doctrine Guardrails

Every bead in every epic must be compatible with these load-bearing principles from `about/heart-and-soul/`:

- **LLMs must never sit in the frame loop.** Models drive the scene; the runtime composits. (architecture.md)
- **Arrival time ≠ presentation time.** All payloads carry timing semantics. (architecture.md, timing-model spec)
- **Local feedback first.** Touch/interaction acknowledgement happens locally and instantly; remote semantics follow. (architecture.md, input-model spec)
- **Screen is sovereign.** The runtime owns pixels, timing, composition, permissions, arbitration. Models request via leases. (architecture.md)
- **Human override always wins.** Dismiss, revoke, safe mode, freeze — local, instant, cannot be intercepted. (security.md, policy-arbitration spec)
- **One scene model, two profiles.** Desktop and mobile share the same API; differences are negotiated capabilities/budgets, not separate architectures. (mobile.md, configuration spec)
- **Presence requires governance.** Every agent gets namespace, leases, capabilities, TTL, budgets. No unbounded screen territory. (presence.md, lease-governance spec)
- **Errors are teaching surfaces.** Every error must be structured, machine-readable, with correction hints. (development.md)

## V1 Scope Tagging

Every bead description MUST include:

1. **v1-mandatory requirements** — what this bead implements
2. **v1-reserved exclusions** — schema defined but implementation deferred (e.g., mobile profile, WebRTC, IME composition)
3. **post-v1 exclusions** — explicitly out of scope (e.g., persistent resource store, delta resume, GStreamer media)

Use the spec's own `Scope:` tags (v1-mandatory, v1-reserved, post-v1) as the source of truth.

## Completeness Contract

Each bead must:
- Reference specific requirement names and line numbers from the subsystem spec
- Quote or cite the WHEN/THEN scenarios that define acceptance criteria
- State which test scenes from the validation-framework's 25-scene registry are relevant
- State the crate/file location where implementation belongs
- State what the bead does NOT include (scope boundary)
