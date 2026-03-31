---
name: lay-and-land
description: >
  Load the project's topology maps to understand where components live, how they connect,
  and what boundaries exist. The about/lay-and-land/ directory contains component inventories,
  data flow diagrams, dependency maps, deployment topology, and operational docs. Consult
  before adding new components, modifying integration points, changing deployment, or when
  unsure where something lives in the system.
---

# System Topology — Lay and Land

The `about/lay-and-land/` directory contains the spatial understanding of this project — where components live, how data flows, what boundaries exist, and how the system is deployed.

**Consult topology maps before:**
- Adding or restructuring components
- Modifying integration points or APIs between subsystems
- Changing deployment targets or infrastructure
- Working on something that crosses component boundaries

**Do NOT load all maps at once.** Select by what you need to understand.

## Map Index

| Map | Read when... | Key content |
|-----|-------------|-------------|
| `about/lay-and-land/README.md` | Quick orientation | Directory overview, what's available |
| `about/lay-and-land/operations/DEPLOYMENT.md` | Deployment topology | Cross-machine deployment, targets, environments |
| `about/lay-and-land/operations/OPERATOR_CHECKLIST.md` | Operational procedures | Runbooks, playbooks, checklists |
| `about/lay-and-land/operations/RUNTIME_APP_BINARY.md` | Binary specification | Canonical application binary structure |

## Crate/Component Layout

The Rust workspace at project root contains:

| Directory | Purpose |
|-----------|---------|
| `crates/` | Core library crates (scene, runtime, protocol, etc.) |
| `app/` | Application binaries |
| `examples/` | Demo and benchmark binaries |
| `tests/` | Integration tests |

## Key Boundaries

1. **Three protocol planes** — MCP (compatibility), gRPC (control), WebRTC (media). Do not collapse.
2. **Runtime vs agent** — The runtime owns pixels and timing. Agents request via leases.
3. **Hot path vs cold path** — Frame pipeline (hot) uses zero-copy, binary formats. Management (cold) uses protobuf.

## Quick Reference

| Need | Skill |
|------|-------|
| Why a boundary exists | `/heart-and-soul` |
| How a boundary communicates | `/law-and-lore` |
| What a component must do | `/spec-and-spine` |
