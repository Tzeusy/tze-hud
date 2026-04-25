# Repository Thesis

`tze_hud` is a local, high-performance display runtime that gives LLM agents governed presence on a real screen. The runtime owns pixels, timing, input routing, policy enforcement, resource budgets, and composition. Agents do not render frames directly; they negotiate for presence through leases, sessions, zones, widgets, and atomic scene mutations.

This curriculum exists because the repository assumes a compound mental model that most application codebases do not: native GPU compositor constraints, a pure scene graph, one multiplexed streaming gRPC session per agent, explicit clock-domain semantics, capability and privacy policy, content-addressed resources, and machine-readable validation for LLM-driven development. Without that background, the repo can look like “a lot of Rust crates plus docs,” while the important invariants remain invisible.

The major technical domains a learner will encounter are:
- Native compositor/runtime architecture (`wgpu`, `winit`, thread ownership, frame budgets)
- Scene-graph data modeling and transactional mutation semantics
- Streaming protocol design (`prost`, `tonic`, protobuf layout, backpressure, session lifecycle)
- Timing and synchronization (wall vs monotonic clocks, scheduling, sync groups)
- Governance (leases, capabilities, privacy, attention, degradation, human override)
- Content-addressed assets and managed publishing surfaces (zones, widgets, runtime-owned rendering)
- Validation architecture (headless rendering, SSIM, telemetry, benchmark normalization, artifacts)

The key mental-model gaps are mostly systems gaps, not syntax gaps. The most dangerous misunderstandings are:
- treating the runtime like an app UI instead of a sovereign compositor
- treating protobuf/schema changes like local refactors
- treating arrival time as presentation time
- treating policy as “auth only” instead of a full privacy/attention/resource stack
- treating tests as pass/fail checks instead of the main observability surface

This curriculum is mostly evidence-backed rather than inference-heavy. The active v1 path is documented directly in `about/`, `openspec/`, crate docs, and tests. The one major inference-heavy area is how much post-v1 media knowledge a contributor needs now; that is kept explicitly secondary.

Repo orientation appears here only to justify why each concept matters. The teaching target is the transferable concept, not the local file tree.
