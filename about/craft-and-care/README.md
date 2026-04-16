# Craft and Care

This pillar answers **WHO ARE WE WHEN WE BUILD?** It defines the engineering quality bar for tze_hud: what "good enough" means, what is non-negotiable, and what every contributor (human or model) must uphold before code lands. Where the doctrine in `heart-and-soul/` says *what* to build and *why*, this pillar says *how well*.

## Documents

| Document | When to read |
|----------|-------------|
| [engineering-bar.md](engineering-bar.md) | Before writing, reviewing, or merging any code. The unified quality bar. |

## Relationship to Other Pillars

| Pillar | Relationship |
|--------|-------------|
| `heart-and-soul/` | Doctrine defines the invariants. This pillar defines how to enforce them in practice. |
| `heart-and-soul/validation.md` | The authoritative testing architecture. This pillar references it, does not duplicate it. |
| `heart-and-soul/development.md` | The development workflow and role separation. This pillar adds the quality gate each role must pass. |
| `legends-and-lore/` | RFCs define quantitative budgets. This pillar consolidates them into one reference table. |
| `../openspec/` | Capability specs define *what* must be built. This pillar defines the quality bar for *how*. |
