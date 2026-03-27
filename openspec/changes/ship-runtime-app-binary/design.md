## Context

The current build surface exposes only demo binaries (`vertical_slice`, `benchmark`, `render-artifacts`) and does not provide a canonical runtime executable for operations. The active cross-machine automation path therefore targets a demo binary that does not consistently expose a live MCP HTTP surface, causing deployment-success but publish-failure outcomes.

The runtime codebase already has key primitives needed for the final architecture:
- `WindowedRuntime` and scene/compositor lifecycle
- `NetworkRuntime` scaffolding in windowed state
- MCP server implementation in `tze_hud_mcp::McpServer`

What is missing is production entrypoint composition and lifecycle wiring in the windowed app path.

## Goals / Non-Goals

**Goals:**
- Introduce a canonical, non-demo runtime application binary target.
- Ensure windowed runtime can initialize network services (including MCP HTTP) based on config.
- Provide deterministic endpoint lifecycle and shutdown behavior.
- Establish a reliable cross-machine validation flow that proves live MCP publish capability.

**Non-Goals:**
- Replacing existing demo/example binaries.
- Redesigning MCP tool semantics or capability model.
- Introducing new protocol surfaces beyond currently defined runtime endpoints.

## Decisions

### Decision 1: Add a dedicated runtime app binary target
Create a standalone runtime application binary (outside `examples/`) as the canonical executable.

Rationale:
- Prevents operational ambiguity between demo and production paths.
- Enables stable artifact naming and deployment automation.

Alternative considered:
- Continue using `vertical_slice` as operational binary with incremental patches.
- Rejected because it preserves semantic confusion and keeps demo assumptions in the deploy path.

### Decision 2: Wire MCP HTTP into windowed runtime lifecycle
Use `WindowedRuntime` startup to initialize `NetworkRuntime` tasks and MCP HTTP listener when configured.

Rationale:
- Single-process runtime behavior for actual desktop overlay use.
- Avoids requiring a separate headless process for MCP publish operations.

Alternative considered:
- Keep MCP available only in a separate process mode.
- Rejected because cross-machine operator flows require a single deployed app artifact with live publish semantics.

### Decision 3: Enforce explicit endpoint gates in validation automation
Validation scripts should hard-gate on endpoint reachability before publish checks, and emit structured failure diagnostics.

Rationale:
- Prevents false positives where deployment succeeds but runtime is not publish-ready.
- Speeds operational debugging by isolating transport vs auth vs runtime-lifecycle failures.

Alternative considered:
- Best-effort publish attempts without pre-gating.
- Rejected because timeout-driven failures are ambiguous and expensive to triage.

## Risks / Trade-offs

- **[Risk: Entry-point proliferation]** → Mitigation: explicitly designate one canonical app binary and document demo binaries as non-operational references.
- **[Risk: Network lifecycle regressions in windowed mode]** → Mitigation: add startup/shutdown tests for enabled and disabled endpoint configurations.
- **[Risk: Authentication misconfiguration in MCP path]** → Mitigation: add positive and negative auth smoke checks in cross-machine validation.
- **[Trade-off: More startup complexity in windowed path]** → Mitigation: keep endpoint wiring configuration-driven and isolated behind clear runtime initialization boundaries.

## Migration Plan

1. Introduce canonical runtime app binary target and baseline CLI/config handling.
2. Implement windowed network-service initialization and MCP listener lifecycle wiring.
3. Add endpoint lifecycle and auth tests for the new app path.
4. Update deployment automation to target canonical app artifact by default.
5. Update docs and retire demo-binary assumptions in user-test workflows.

Rollback strategy:
- If runtime startup regresses, disable canonical target rollout in automation and temporarily fall back to previous build/deploy scripts while keeping spec artifacts and tests for corrective iteration.

## Open Questions

- What canonical binary name should be standardized for artifact identity across Linux cross-build and Windows deployment tooling?
- Should gRPC and MCP endpoint defaults both be enabled in canonical windowed mode, or should MCP be opt-in by explicit config for v1?
- Which minimum logging fields are required in automation output to classify endpoint failures without manual SSH triage?
