# Heart and Soul

This folder contains the doctrine of **tze_hud** — an agent-native presence engine. These documents define what the project is, why it exists, how it works, how it is built, and what it ships first.

This is doctrine, not documentation. It does not describe the code — it describes the principles the code must embody. Implementation details, API references, and how-to guides belong elsewhere.

## Why this folder exists

A project without doctrine is a bag of iron filings — agents pull in their own directions, design decisions contradict each other, testing validates the wrong invariants, and planning drifts from the actual goal. Every new contributor (human or model) adds another random vector. Energy is spent. Nothing compounds.

Doctrine is the magnet.

Drop a magnet into a field of iron filings and every particle snaps to the same orientation. The filings don't lose their individual character — they gain coherence. That's what these documents do. They establish a single, strong field that aligns every chaotic element of the project:

- **Agents** know what presence means, what the screen contract is, what they can and cannot do — without re-deriving it from code or guessing from context.
- **Architecture decisions** resolve against shared principles instead of individual taste. When two reasonable designs conflict, doctrine breaks the tie.
- **Work planning** inherits direction from the goal rather than inventing its own. Tasks trace back to doctrine; orphan work is visible.
- **Testing** validates the invariants that actually matter — the ones doctrine names — instead of chasing coverage for its own sake.
- **Design** stays coherent across ten documents because every document points at the same north.
- **New contributors** align in minutes. Read the doctrine, absorb the field, start pulling in the same direction.

Without the magnet, you get a flux diagram where every arrow points somewhere different — maximum entropy, minimum progress. With it, every arrow points the same way. The project moves.

## Reading order

1. **[vision.md](vision.md)** — Why this exists. The core thesis. What presence means. Start here.

2. **[architecture.md](architecture.md)** — How the system is structured. Protocol planes, message classes, timing model, rendering/media/language choices.

3. **[presence.md](presence.md)** — The presence model. Tabs, tiles, leases, presence levels, multi-agent coordination, interaction.

4. **[security.md](security.md)** — Trust and governance. Authentication, capability scopes, agent isolation, resource governance, human override.

5. **[privacy.md](privacy.md)** — Attention governance for household surfaces. Viewer context, content classification, redaction, interruption classes, quiet hours.

6. **[attention.md](attention.md)** — The philosophical stance on attention, attention budget, and the anti-patterns of attention exploitation. Read after privacy.md.

7. **[failure.md](failure.md)** — What happens when things break. Agent failure modes, recovery, degradation, persistence, reconnection.

8. **[mobile.md](mobile.md)** — Mobile and smart-glasses profile. Same model, different budgets, degradation axes, upstream composition.

9. **[validation.md](validation.md)** — How we test. Testing doctrine, five validation layers, LLM development loop, fuzzing, chaos testing, developer visibility artifacts.

10. **[development.md](development.md)** — How we build. Spec-driven workflow, task management, execution loop, development principles.

11. **[v1.md](v1.md)** — What v1 ships and what it defers. Scope boundary for the first working system.

Note: The component shape language (design tokens, component profiles, visual extensibility) is documented in **presence.md** §"Component shape language" and **architecture.md** §"Text rendering policy and design tokens". The detailed specification lives in `openspec/changes/component-shape-language/`. The corresponding RFC is 0012 in `about/law-and-lore/`.
