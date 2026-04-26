# Lay and Land — System Topology

Maps of where components live, how they connect, and what boundaries exist.

One topology seam worth tracking explicitly is low-latency streamed text interaction. The intended use case is a governed text portal backed by authenticated runtime sessions and external adapters. Those adapters may represent anything from human chat transports to LLM interactions, but the runtime boundary remains transport-agnostic text input/output streams rather than tmux- or provider-specific semantics.

## Maps

| Map | Description |
|-----|-------------|
| `components.md` | Component inventory: crates, binaries, and their boundaries |
| `data-flow.md` | How data moves through the three protocol planes, including the text stream portal pilot flow |
| `runtime-widget-asset-topology.md` | Runtime widget SVG register/upload ingress, durable store topology, startup re-index path, and budget hooks |
| `operations/` | Deployment topology, operator checklists, runtime binary spec |

Diagrams live in `assets/`.

## Operations

Operational docs migrated from `docs/`:

| File | Purpose |
|------|---------|
| `operations/DEPLOYMENT.md` | Cross-machine deployment topology |
| `operations/OPERATOR_CHECKLIST.md` | Operational playbooks and procedures |
| `operations/RUNTIME_APP_BINARY.md` | Canonical application binary specification |
