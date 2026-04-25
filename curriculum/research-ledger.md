# Research Ledger

## Pass 1

- Angle: Surface and topology pass using top-level docs, doctrine, topology maps, specs, workspace manifests, and app/deployment entrypoints.
- Major concept clusters surfaced:
  - sovereign compositor/runtime model
  - protocol planes and generated protobuf boundary
  - lease/capability governance
  - canonical app binary, config, and deployment workflow
  - validation and telemetry as first-class architecture
  - doctrine/RFC/OpenSpec split
- Evidence notes:
  - Strongest signals came from `README.md`, `about/heart-and-soul/architecture.md`, `about/heart-and-soul/v1.md`, `about/lay-and-land/components.md`, and the root workspace manifest.
  - This pass also surfaced the important doc-topology prerequisite: understanding which documents are normative versus explanatory is itself part of safe contribution.

## Pass 2

- Angle: Runtime and failure-mode pass using app startup code, session handlers, tests, config/deployment docs, and failure/validation doctrine.
- Major concept clusters surfaced:
  - thread ownership and main/compositor/network responsibilities
  - fail-closed startup, config precedence, endpoint lifecycle
  - one-stream-per-agent gRPC semantics
  - lease handling, safe mode, backpressure, freeze queues
  - resource upload lifecycle and content-addressed dedup
  - observability and calibration discipline
- Evidence notes:
  - `app/tze_hud_app/src/main.rs`, `crates/tze_hud_protocol/src/session_server.rs`, `openspec/specs/runtime-kernel/spec.md`, `openspec/specs/resource-store/spec.md`, and protocol tests were the highest-signal surfaces.
  - This pass raised the required depth for async transport, backpressure, and startup/config knowledge from “nice background” to “must know before changing behavior safely.”

## Pass 3

- Angle: Contribution-hazard pass focused on invariants a newcomer could violate by making plausible but wrong edits.
- Major concept clusters surfaced:
  - protobuf/wire compatibility hazards
  - scene-graph atomicity and namespace isolation
  - clock-domain mistakes and scheduling bugs
  - privacy/redaction/attention/resource policy interactions
  - resource identity and asset-ingress distinctions
  - zones/widgets as runtime-owned abstractions
  - validation as an invariant surface rather than a cosmetic layer
- Evidence notes:
  - The strongest hazard evidence came from `openspec/specs/session-protocol/spec.md`, `openspec/specs/scene-graph/spec.md`, `openspec/specs/timing-model/spec.md`, `openspec/specs/policy-arbitration/spec.md`, `tests/integration/v1_thesis.rs`, and `tests/integration/disconnect_orphan.rs`.
  - This pass specifically changed the curriculum shape by elevating policy/resource/validation from side topics into core prerequisites.

## Reconciliation

- Concepts that appeared across multiple passes:
  - sovereign runtime/compositor architecture
  - scene graph and atomic mutation semantics
  - single-stream gRPC/protobuf session model
  - lease/capability/resource governance
  - validation/telemetry as part of the product
  - config/deployment/startup gating
- Concepts that surfaced late and changed the curriculum:
  - backpressure/freeze-queue behavior and interleaved session replies
  - content-addressed resource identity and its distinction from transport checksums
  - redaction/layout-preservation and attention/degradation ordering
  - the importance of document authority boundaries (`about/` vs RFCs vs OpenSpec)
- Additional passes run beyond the minimum:
  - None. Three isolated passes were sufficient because the same main clusters converged independently and no major new topic family emerged during reconciliation.

The reconciled result is one curriculum path rather than multiple split tracks. The active v1 prerequisite surface is broad but still compact enough to fit inside a single 39-hour path when grouped into six modules. Deferred media work stays in glossary/open-question territory instead of distorting the main learning path.
