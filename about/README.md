# About tze_hud

This directory contains the project's self-knowledge — the structured understanding that tells you *what* tze_hud is, *why* it exists, *how* it works, and *where* everything lives.

## Four Pillars

| Pillar | Folder | Question | Content |
|--------|--------|----------|---------|
| **Doctrine** | `heart-and-soul/` | **WHY** does this exist? | Vision, principles, non-negotiables, scope boundaries |
| **Design Contracts** | `law-and-lore/` | **HOW** will it work? | RFCs, wire contracts, state machines, reviews |
| **Capability Specs** | `../openspec/` | **WHAT** must be built? | Normative requirements, WHEN/THEN scenarios |
| **Topology** | `lay-and-land/` | **WHERE** does everything live? | Component maps, data flow, deployment topology |

Three pillars live here under `about/`. Capability specs (`openspec/`) stay at project root — they're a product with their own structure and conventions.

## Traceability Chain

Every implementation decision traces back through: **Doctrine principle → RFC design decision → Spec requirement → Code → Test**

## Navigation

Start with `heart-and-soul/README.md` for the doctrine reading order. Use the local skills (`/heart-and-soul`, `/law-and-lore`, `/spec-and-spine`, `/lay-and-land`) for selective, context-appropriate loading.
