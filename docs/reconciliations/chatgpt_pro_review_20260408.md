## 1. Executive summary

**Observed.** This is a serious Rust monorepo, not a weekend prototype. The workspace is split into focused crates for runtime, scene, protocol, MCP, config, policy, widgets, resources, validation, and the canonical app binary; it also has substantial doctrine/RFC/operations docs, a dedicated integration-test package, and a CI workflow with compile, lint, trace-regression, v1-thesis, production-boot, vocabulary, and dev-mode guards. The repo also shows real activity signals: 529 commits on `main`, 369 closed PRs, and 894 workflow runs. ([GitHub][1])

**Inference.** The project substantially achieves its *core* thesis: a local, governed, agent-facing display runtime with a pure scene model, gRPC/MCP control planes, leases/capabilities, widgets/zones, headless support, and explicit frame-budget architecture. But it does **not** cleanly achieve the public README as written today. The README still advertises WebRTC/media as a current protocol plane, while the v1 doctrine explicitly defers WebRTC and all media pipelines; the config/docs surface is also out of sync with the current runtime loader. ([GitHub][2])

The biggest strengths are architecture boundaries, spec discipline, and unusually thoughtful test intent. The biggest risks are trust erosion from doc/config drift, known instability on `main`, weak network-security defaults for anything beyond a trusted local environment, and a missing public release/contribution story. The repo’s own notes say `multi_agent.rs` has known compile failures and that `v1_thesis.rs` and some budget assertions are already unstable on `main`; the runtime also says `tze_hud_policy` is “not wired in v1,” which undercuts some governance claims. ([GitHub][3])

## 2. Scorecard

| Area                                          | Score | Confidence | Summary                                                                                                                   | Evidence                                                                                                                                                           |
| --------------------------------------------- | ----: | ---------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Goal alignment and product coherence          |     3 | Medium     | Strong core thesis, but public claims overrun implemented v1 scope and the canonical config/docs story is inconsistent.   | `README.md`, `about/heart-and-soul/v1.md`, `crates/tze_hud_config/src/loader.rs`, `app/tze_hud_app/config/production.toml` ([GitHub][2])                           |
| Architecture and modularity                   |     4 | High       | Clean crate boundaries and a well-documented authority split are real strengths.                                          | `Cargo.toml`, `crates/tze_hud_runtime/src/lib.rs`, `crates/tze_hud_scene/src/graph.rs` ([GitHub][4])                                                               |
| Code clarity and craftsmanship                |     3 | Medium     | Concepts are well documented, but some high-value files are dense and hard to review.                                     | `app/tze_hud_app/src/main.rs`, `crates/tze_hud_protocol/src/session_server.rs`, `tests/integration/v1_thesis.rs` ([GitHub][5])                                     |
| Correctness and reliability                   |     3 | Medium     | Good invariant/test intent, but known compile/test failures on `main` reduce trust.                                       | `tests/integration/trace_regression.rs`, `AGENTS.md`, `.github/workflows/ci.yml` ([GitHub][6])                                                                     |
| Error handling and failure behavior           |     3 | Medium     | Explicit state machines and fallback logic exist, but some failure paths degrade too quietly.                             | `crates/tze_hud_protocol/src/session_server.rs`, `crates/tze_hud_runtime/src/windowed.rs`, `crates/tze_hud_runtime/src/budget.rs` ([GitHub][7])                    |
| Observability, tracing, and debuggability     |     4 | Medium     | Trace replay, telemetry stages, operator docs, and structured logging give this repo unusually good debug intent.         | `crates/tze_hud_runtime/src/lib.rs`, `tests/integration/trace_regression.rs`, `about/lay-and-land/operations/OPERATOR_CHECKLIST.md` ([GitHub][8])                  |
| Testing strategy and test quality             |     3 | Medium     | The strategy is strong; current credibility is weaker because important tests are excluded or unstable.                   | `.github/workflows/ci.yml`, `examples/vertical_slice/tests/production_boot.rs`, `tests/integration/soak.rs`, `AGENTS.md` ([GitHub][9])                             |
| Tooling and engineering hygiene               |     3 | Medium     | CI is thoughtful, but public contributor ergonomics lag behind the internal process.                                      | `.github/workflows/ci.yml`, `scripts/`, missing `CONTRIBUTING.md` ([GitHub][9])                                                                                    |
| Dependency and ecosystem health               |     3 | Medium     | Modern Rust stack and workspace-managed deps; weak public release hygiene.                                                | `Cargo.toml`, Releases page, missing `CHANGELOG.md` ([GitHub][4])                                                                                                  |
| Security posture                              |     3 | Medium     | Real auth/capability machinery exists, but defaults and transport hardening are not production-grade.                     | `crates/tze_hud_protocol/src/auth.rs`, `crates/tze_hud_runtime/src/mcp.rs`, `app/tze_hud_app/src/main.rs`, `crates/tze_hud_runtime/src/windowed.rs` ([GitHub][10]) |
| Performance and scalability                   |     3 | Medium     | Explicit frame budgets and bounded channels are good signs; proof is not yet fully trustworthy.                           | `crates/tze_hud_runtime/src/lib.rs`, `crates/tze_hud_runtime/src/budget.rs`, `tests/integration/soak.rs`, CI pixel-readback note ([GitHub][8])                     |
| Data model and API design                     |     4 | High       | The pure scene graph, canonical capability vocabulary, and versioned session flow are well thought out.                   | `crates/tze_hud_scene/src/graph.rs`, `crates/tze_hud_protocol/src/auth.rs`, `crates/tze_hud_protocol/src/session_server.rs` ([GitHub][11])                         |
| Documentation and developer experience        |     3 | Medium     | Deep docs exist, but they disagree with the code in important places.                                                     | `about/heart-and-soul/`, `about/legends-and-lore/`, `about/lay-and-land/operations/*`, `README.md`, missing `CONTRIBUTING.md` ([GitHub][12])                           |
| Release, operations, and production readiness |     2 | Medium     | There is an operator-facing app and deployment guidance, but not enough release/process rigor for outside production use. | `app/tze_hud_app/Cargo.toml`, operations docs, production-boot CI, Releases page, Issues page, `AGENTS.md` ([GitHub][13])                                          |
| Maintainability and change safety             |     3 | Medium     | Specs and modularity help, but drift and unstable main-branch contracts are building maintenance drag.                    | `Cargo.toml`, `AGENTS.md`, `README.md`, `crates/tze_hud_policy/src/lib.rs`, `crates/tze_hud_runtime/src/lib.rs` ([GitHub][4])                                      |

## 3. README and goal-fulfillment review

**Observed explicit goals from the README.** The README says `tze_hud` is a “local, high-performance display runtime” where LLMs connect over three protocol planes—MCP, gRPC, and WebRTC—and receive governed presence through leases, capability scopes, privacy ceilings, attention budgets, graceful degradation, swappable visual identity, and multi-agent arbitration. It also presents the `tze_hud` binary as the canonical, production-ready runtime for cross-machine deployment and automated MCP publishing. ([GitHub][2])

**Observed implicit goals from code/docs.** The doctrine says v1 is meant to prove seven specific properties: tile residency at 60fps, lease governance, three-agent coexistence, measured performance budgets, five validation layers, one-call zone publishing, and full headless operation. The workspace and crate split strongly match that thesis: a pure scene graph, a runtime orchestrator, protocol and MCP planes, config validation, widget/profile loading, and integration tests around trace replay, soak, and v1 proof. ([GitHub][14])

**Achieved goals.** The repo clearly does implement a local display runtime with a real scene model, a runtime kernel, gRPC session machinery, an MCP bridge, zone/widget publishing, auth/capability gating, headless mode, and dedicated operator-facing app packaging. The scene graph is pure data with injected clocks and explicit bounds; the protocol layer has version negotiation and canonical capability validation; the runtime documents fixed thread groups, bounded channels, and a budget-enforcement ladder. ([GitHub][11])

**Partially achieved goals.** Privacy, attention, and policy/governance are present in design and partially in code, but not as cleanly as the top-level story suggests. The config/reload layer supports privacy and dynamic-policy sections, and the runtime has attention budgets and budget enforcement, but the runtime explicitly says `tze_hud_policy` is “not wired in v1,” even though the policy crate’s own docs still describe the runtime calling it. ([GitHub][15])

**Unmet goals.** The WebRTC/media plane is not a current v1 reality. The README presents WebRTC as one of the three active protocol planes, while the v1 doctrine explicitly defers WebRTC, video decode, and all media pipelines. I also did not see any WebRTC dependency in the workspace manifest I inspected. ([GitHub][2])

**Underdocumented auxiliary goals.** The repo is also trying to be a spec-driven, agent-assisted engineering system: doctrine, RFCs, ops playbooks, AI-assistant files, and internal process notes all point that way. That goal is *present*, but it is more obvious from repo structure than from the README. ([GitHub][12])

**Documentation gaps and contradictions.** The worst contradiction is the config story. The current runtime path parses `config_toml` through `TzeHudConfig`, whose loader says a minimal valid config must have `[runtime]` plus `runtime.profile`; the example `vertical_slice` production config follows that schema. But the README and operator docs still teach older `[display]` / `[network]` examples, and the committed `app/tze_hud_app/config/production.toml` also omits `[runtime]`. The runtime’s current fallback behavior on parse/validation errors is `headless_default()`, not a hard failure, which means stale canonical configs are especially dangerous because they can degrade into a different runtime mode instead of obviously exploding. ([GitHub][16])

**Unknown / cannot verify.** I did not execute the canonical app locally, so I cannot prove whether `app/tze_hud_app/config/production.toml` currently fails validation end-to-end at runtime. But based on the current loader contract and the file contents, the most likely reading is that the canonical app config/docs path is stale. ([GitHub][16])

## 4. Detailed findings by category

### 1) Goal alignment and product coherence — 3/5, confidence: Medium

**What is good.** The project has a coherent *v1* thesis and the code structure mostly follows it. `about/heart-and-soul/v1.md` defines a narrow proof target, and the workspace layout mirrors that with dedicated crates for scene, runtime, protocol, MCP, config, widgets, resources, validation, and the app. ([GitHub][14])

**What is weak.** Public-facing claims have outrun the implemented v1 boundary. README still says “WebRTC for media,” while v1 explicitly says “No WebRTC”; README and ops docs also teach a config schema that the current runtime loader no longer treats as canonical. ([GitHub][2])

**Observed / Inference.** Observed: core runtime goals are real. Inference: the repo is more coherent internally than externally; the doctrine is closer to truth than the README. ([GitHub][12])

**Recommended fixes.** Make the doctrine-generated v1 scope the single source of truth for README and ops docs. Delete or clearly mark future-scope claims, and ship one blessed config schema with one blessed canonical config path.

### 2) Architecture and modularity — 4/5, confidence: High

**What is good.** The crate boundaries are unusually thoughtful. `tze_hud_scene` is pure data/no I/O, `tze_hud_protocol` owns gRPC/session logic, `tze_hud_mcp` is explicitly a compatibility plane rather than the hot path, and `tze_hud_runtime` documents a concrete authority map and thread model. ([GitHub][11])

**What is weak.** One major architectural seam is aspirational rather than operational: the policy crate exists as a deep subsystem, but the runtime says it is not wired into v1. That means part of the architecture is still “reference design,” not system behavior. ([GitHub][17])

**Observed / Inference.** Observed: modularity is strong. Inference: this will scale better than a flat monolith, but only if the documented authority boundaries become *actual* runtime boundaries rather than parallel theories. ([GitHub][4])

**Recommended fixes.** Either wire `tze_hud_policy` into the mutation path or cut its public prominence until it is live. Add an architectural conformance test that proves which authority crates are actually in the request path.

### 3) Code clarity and craftsmanship — 3/5, confidence: Medium

**What is good.** Conceptual documentation inside code is strong. `session_server.rs`, `graph.rs`, `runtime/src/lib.rs`, and the config modules carry useful contract comments and state-machine descriptions. ([GitHub][7])

**What is weak.** Some important files are overly dense. `app/tze_hud_app/src/main.rs` packs substantial behavior and help text into 53 lines of long-formatted code; several integration tests are similarly compressed. That style slows review, makes diffs noisy, and raises the cost of surgical debugging. ([GitHub][5])

**Observed / Inference.** Observed: the repo is thoughtfully designed, but not consistently easy to read. Inference: maintainers already understand the system well enough to compress it; new contributors will not. ([GitHub][5])

**Recommended fixes.** Reformat the app entrypoint, test harnesses, and any protocol hot spots into normal Rust style. Treat readability in the top 20 files as a product feature.

### 4) Correctness and reliability — 3/5, confidence: Medium

**What is good.** There is real correctness intent: scene invariants and resource-registration rules live in the scene model, there are trace-replay tests, production-boot tests, soak tests, and a dedicated v1 proof test. ([GitHub][11])

**What is weak.** The repo itself documents known compile and test failures on `main`. `multi_agent.rs` is excluded from CI because of pre-existing compile failures, and `v1_thesis.rs` plus some budget assertions are already marked unstable. That is the difference between “good testing strategy” and “currently trustworthy branch.” ([GitHub][9])

**Observed / Inference.** Observed: there is good reliability machinery. Inference: correctness is probably better in the scene/runtime core than in the integration surface, because that is where breakage is explicitly admitted. ([GitHub][11])

**Recommended fixes.** Make `main` green again before adding surface area. Until then, mark broken tests as quarantined and remove broken commands from public docs.

### 5) Error handling and failure behavior — 3/5, confidence: Medium

**What is good.** The session layer has a documented lifecycle state machine, reconnect grace period, bounded queues, and traffic-class-aware handling. The runtime config path also has graceful fallback behavior, and the budget enforcer has a clear ladder from warning to throttle to revoke. ([GitHub][7])

**What is weak.** Some of the graceful behavior is *too* graceful. Invalid configs fall back to `headless_default()` instead of hard-failing, and `session_server.rs` explicitly notes that dropped transactional events should emit metrics/alerts in production but “for v1 we continue silently.” Silent degradation is survivable in tests; it is dangerous in operations. ([GitHub][18])

**Observed / Inference.** Observed: the system prefers liveness. Inference: operator mistakes can become subtle misconfiguration rather than obvious startup failures, especially around the canonical app config path. ([GitHub][18])

**Recommended fixes.** Add a strict production mode that fails on invalid config, missing secrets, or dropped transactional traffic. Keep the current forgiving behavior only in explicit dev/test modes.

### 6) Observability, tracing, and debuggability — 4/5, confidence: Medium

**What is good.** This is one of the repo’s better areas. The runtime dedicates a telemetry stage, there are trace capture/replay tests, soak runs write artifacts, and operator docs expect log files and reachability gates. ([GitHub][8])

**What is weak.** I could not verify a mature external metrics/export story from the files I inspected. The repo clearly has structured telemetry concepts, but I did not confirm a stable Prometheus/OpenTelemetry/operator dashboard path. ([GitHub][8])

**Unknown / cannot verify.** I did not inspect every telemetry-related crate/file or run the runtime to confirm emitted schemas.

**Recommended fixes.** Publish one operator-facing observability contract: core counters, health signals, trace artifact locations, and escalation rules.

### 7) Testing strategy and test quality — 3/5, confidence: Medium

**What is good.** The testing *shape* is excellent: unit/workspace tests, trace regression, v1-thesis proof, production-boot, vocabulary lint, dev-mode guard, soak, and GPU pixel-readback. Very few repos in this maturity band think this broadly. ([GitHub][9])

**What is weak.** Quality is constrained by branch health. The integration package is partially excluded, the v1-thesis proof is unstable on `main`, and the GPU pixel-readback job is explicitly non-blocking. Also, the “production config boot” test covers the `vertical_slice` example config, not the canonical app’s committed config. ([GitHub][9])

**Observed / Inference.** Observed: the author knows what should be tested. Inference: the repo is closer to “strong verification culture, weak release discipline” than to “poorly tested.” ([GitHub][9])

**Recommended fixes.** Add a blocking smoke test for the canonical app config and get the v1-thesis lane green before widening coverage further.

### 8) Tooling and engineering hygiene — 3/5, confidence: Medium

**What is good.** `cargo fmt --check`, `clippy -D warnings`, dev-mode guards, vocabulary lint, and protobuf dependency setup are all documented in CI. Scripts exist for vocabulary checks, MCP reachability, Windows build, and full-app smoke flow. ([GitHub][9])

**What is weak.** External contributor ergonomics are thin. There is no `CONTRIBUTING.md`, no public changelog, and the live process appears to run through internal beads/agent notes rather than standard public GitHub workflow. GitHub Issues are empty, but `AGENTS.md` clearly references internal issue IDs like `hud-3m8h`. 

**Observed / Inference.** Observed: internal hygiene is better than public hygiene. Inference: this is optimized for one maintainer plus AI assistants, not for a growing external contributor base. ([GitHub][1])

**Recommended fixes.** Add `CONTRIBUTING.md`, `CHANGELOG.md`, and a short “how work is tracked” note so outsiders understand beads vs GitHub Issues.

### 9) Dependency and ecosystem health — 3/5, confidence: Medium

**What is good.** The dependency stack is sensible and mainstream for the problem: Tokio, Tonic, Prost, WGPU, Winit, tracing, schemars, resvg. Workspace-level dependency management and a single toolchain floor (`rust-version = 1.88`) help consistency. ([GitHub][4])

**What is weak.** Public ecosystem hygiene is weak. There are no visible GitHub releases, and I did not find a changelog. That makes external adoption, semver expectations, and upgrade confidence much worse than the code quality alone would suggest. ([GitHub][19])

**Unknown / cannot verify.** I did not run `cargo audit` or inspect advisories, so this is not a vulnerability audit.

**Recommended fixes.** Start shipping tagged releases with upgrade notes, even if they are explicitly “alpha.”

### 10) Security posture — 3/5, confidence: Medium

**What is good.** Security is not ignored. The protocol layer has structured credentials, canonical capability validation, guest-vs-registered policy paths, and constant-time-ish PSK comparison; MCP authentication is described as always enforced. ([GitHub][10])

**What is weak.** The defaults are not safe enough for anything beyond trusted local/dev use. The canonical app defaults to PSK `tze-hud-key`; the MCP HTTP server is a minimal custom handler with no TLS and is documented as something to replace with a production-grade server; the no-config path in the windowed runtime falls back to `headless_default()` with `fallback_unrestricted = true`, meaning any PSK-authenticated agent gets unrestricted capabilities. OAuth and mTLS are schema-defined but unimplemented; local-socket auth is accepted unconditionally without peer verification. ([GitHub][5])

**Observed / Inference.** Observed: there is a real auth model. Inference: the security model is currently “controlled operator environment,” not “internet-exposed service.” ([GitHub][10])

**Recommended fixes.** Make missing config and default PSKs fatal in release builds, verify local-socket peers, and either terminate TLS upstream or move MCP onto a hardened server stack.

### 11) Performance and scalability — 3/5, confidence: Medium

**What is good.** The runtime is designed around explicit p99 budgets, fixed thread groups, bounded channels, a compositor-owned GPU path, and a budget-enforcement ladder. The v1 doctrine also makes performance a first-class acceptance criterion rather than a vague aspiration. ([GitHub][8])

**What is weak.** The proof surface is not fully trustworthy yet. The repo says the v1-thesis proof and some budget assertions are unstable on `main`, and the GPU pixel-readback test is still informational only. I also did not find published benchmark outputs in the materials inspected. ([GitHub][3])

**Observed / Inference.** Observed: the architecture is performance-aware. Inference: the bottleneck over the next year is more likely to be ops/config/documentation trust than raw frame time—unless the performance proof keeps drifting red. ([GitHub][8])

**Recommended fixes.** Publish benchmark baselines and make at least one canonical perf lane blocking.

### 12) Data model and API design — 4/5, confidence: High

**What is good.** This is one of the strongest areas. The scene graph is pure, explicit, bounded, and clock-injectable; resource registration has an upload-before-use invariant; the session layer has protocol version negotiation and a documented lifecycle; capability names are canonicalized rather than ad hoc. ([GitHub][11])

**What is weak.** The API surface is still moving. `AGENTS.md` says `multi_agent.rs` fails to compile because `SessionInit` and `MutationBatch` gained fields. That is good evidence of schema churn and weak integration change safety, even if the new schema itself is better. Session resume tokens are also in-memory only and not persisted across process restarts. ([GitHub][3])

**Recommended fixes.** Add backward-compat contract tests for protobuf and integration builders, and define a versioning/deprecation policy for wire changes.

### 13) Documentation and developer experience — 3/5, confidence: Medium

**What is good.** The docs are unusually rich: doctrine (`heart-and-soul`), RFCs (`legends-and-lore`), topology/ops (`lay-and-land`), operator playbooks, and a command-heavy README. For internal alignment, this is a major asset. ([GitHub][12])

**What is weak.** The docs disagree with the code in user-important places: WebRTC scope, config schema, and canonical-app readiness. There is also no public `CONTRIBUTING.md`, no public changelog, and no visible release stream. ([GitHub][2])

**Observed / Inference.** Observed: internal documentation effort is high. Inference: external DX is worse than internal DX because the docs have not been pruned around the current truth. ([GitHub][12])

**Recommended fixes.** Designate one public truth set: README + canonical app spec + canonical config example + CI smoke. Everything else should derive from that.

### 14) Release, operations, and production readiness — 2/5, confidence: Medium

**What is good.** There is a real operator-facing binary (`tze_hud`), a Windows application manifest, cross-machine deployment docs, operator checklists, and a production-boot smoke test for the example path. This is more operational thought than many repos show at this stage. ([GitHub][13])

**What is weak.** Outside the maintainer’s environment, this is not yet production-ready in the normal open-source sense. There are no visible releases, no public changelog, no public contribution guide, GitHub Issues are empty while backlog appears elsewhere, the canonical app config path is not what CI boots, and the network edge is still a minimal custom HTTP server. ([GitHub][19])

**Unknown / cannot verify.** I did not verify containerization, service manifests, or real-world deployment success beyond the docs.

**Recommended fixes.** Ship one tested, packaged, documented canonical operator path and stop calling the broader story production-ready until it is.

### 15) Maintainability and change safety — 3/5, confidence: Medium

**What is good.** The repo has the bones of a maintainable system: modular crates, a pure scene core, explicit specs, dedicated integration tests, and vocabulary/feature guards to prevent accidental regressions. ([GitHub][4])

**What is weak.** Maintainability is being taxed by divergence: doctrine says one thing, README another, ops docs a third, and the runtime behavior a fourth. Add known failing tests on `main`, a non-public issue process, and some dense source formatting, and you have a system that is intellectually organized but operationally drift-prone. ([GitHub][2])

**Recommended fixes.** Reduce drift before adding features. The next maintainability win is not another subsystem; it is source-of-truth convergence.

## 5. Feature gap analysis

### Blockers

| Gap                                 | Why it matters                                                                                                                         | Type    | Evidence it is intended/expected                                                                                                                                          | Effort |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -----: |
| Canonical config/schema convergence | The public “canonical app” path appears stale against the current loader. That can silently downgrade governance or startup semantics. | Blocker | Loader requires `[runtime] profile`; app config and docs still use older schema; runtime falls back on validation errors; CI boots example config instead. ([GitHub][16]) |      M |
| Trustworthy green `main`            | If the repo itself says key tests are unstable or excluded, every README claim becomes harder to trust.                                | Blocker | `multi_agent.rs` compile failures; unstable `v1_thesis.rs` and budget assertions; CI exclusions. ([GitHub][9])                                                            |      M |
| Secure production defaults          | Default PSK, no-config unrestricted fallback, and minimal HTTP serving are fine for local dev, not for real ops.                       | Blocker | Default `tze-hud-key`; minimal MCP HTTP server; no-config unrestricted fallback; OAuth/mTLS unimplemented. ([GitHub][5])                                                  |      M |
| Policy crate actually enforced      | The repo’s governance story is stronger than the runtime path it currently proves.                                                     | Blocker | Policy crate says runtime builds/evaluates context; runtime says policy is not wired in v1. ([GitHub][17])                                                                |      M |
| Public release/process surface      | Outside adopters need releases, upgrade notes, and contributor guidance.                                                               | Blocker | No releases visible, no changelog, no `CONTRIBUTING.md`, internal tracking in `AGENTS.md`. ([GitHub][19])                                                                 |      S |

### Enhancements

| Gap                               | Why it matters                                                                                                    | Type        | Evidence it is intended/expected                                                                                                                  | Effort |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | -----: |
| Durable state / persistent resume | In-memory-only resume and no durable scene persistence limit recovery and operational resilience.                 | Enhancement | V1 doctrine explicitly defers durable snapshots/persistence; session resume tokens are not persisted across restarts. ([GitHub][14])              |      L |
| Media / WebRTC plane              | This is part of the long-term vision, but not v1 reality yet.                                                     | Enhancement | README advertises WebRTC; v1 doctrine explicitly defers it and says media is first post-v1 priority. ([GitHub][2])                                |     XL |
| Operator/admin tooling            | Deep systems like this need inspection and control surfaces that are friendlier than raw gRPC/MCP/log spelunking. | Enhancement | V1 doctrine says there is no TypeScript inspector/admin panel yet. ([GitHub][14])                                                                 |      L |
| Published benchmark baselines     | Performance claims need stable, comparable output artifacts.                                                      | Enhancement | V1 doctrine emphasizes normalized benchmarks and measured proof; I did not find published benchmark output in inspected materials. ([GitHub][14]) |      M |
| Public agent SDK/examples         | The repo is rich for maintainers, thinner for external agent authors.                                             | Enhancement | MCP and gRPC surfaces exist, but contributor/release surface is sparse. ([GitHub][20])                                                            |      M |

## 6. Scale and long-horizon analysis

**10x usage/data/traffic/contributors — Inference.** Technically, the core may handle a 10x increase in controlled single-node usage reasonably well because the runtime has fixed thread groups, bounded channels, a dedicated budget-enforcement ladder, and a pure scene graph model. The bigger pain at 10x is likely to be operator error, config sprawl, and security posture, not raw architecture. The current session defaults also show a system still sized for modest concurrency, not a large multitenant fleet. ([GitHub][8])

**100x usage/data/traffic/contributors — Inference.** At 100x, the weak points become organizational and operational. A minimal custom MCP HTTP server, PSK-centric auth, no TLS story in the inspected runtime edge, no visible release stream, and a docs/config surface that already drifts on `main` will fail before the scene graph design does. Contributor onboarding will also get harder because the repo uses rich internal doctrine and beads-style process, but not the standard public artifacts outsiders expect. ([GitHub][21])

**1-year horizon — Inference.** This could become a very strong niche platform if the next year is spent converging truth, not widening scope: green `main`, fix canonical config/docs, harden defaults, and decide whether `tze_hud_policy` is live or aspirational. The foundation is strong enough that this is plausible. ([GitHub][8])

**3-year horizon — Inference.** Without semver/release discipline and compatibility testing, the protocol/config surface will keep shifting under integrators. The compile-failure note about added fields on `SessionInit` and `MutationBatch` is exactly the kind of small break that becomes ecosystem poison over time. ([GitHub][3])

**5-year horizon — Inference.** The main calcification risk is split-brain architecture: doctrine and policy documents becoming “how the system should work,” while the runtime and ops path continue doing something narrower. The second risk is bus factor: no public release/contribution story, empty GitHub Issues, and internal notes/process files suggest heavy maintainer-specific context. ([GitHub][17])

## 7. Risk register

| Title                                                         | Severity | Likelihood | Impact | Confidence | Evidence                                                                                                                            | Why it matters                                                                          | Suggested fix                                                                                  | Effort |
| ------------------------------------------------------------- | -------- | ---------- | ------ | ---------- | ----------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | -----: |
| Canonical app config/docs are out of sync with runtime loader | Critical | High       | High   | High       | `README.md`, ops docs, app config, loader, windowed runtime fallback ([GitHub][2])                                                  | The user-facing “canonical path” may not configure the runtime the way docs imply.      | Replace all stale examples, validate canonical app config in CI, fail hard in production mode. |      M |
| No-config path grants unrestricted capabilities               | Critical | Medium     | High   | High       | `windowed.rs` says no config ⇒ `headless_default()` + `fallback_unrestricted = true`; app treats config as optional. ([GitHub][18]) | An operator omission can weaken governance instead of safely failing.                   | Require explicit `--allow-unconfigured` or dev flag in release builds.                         |      S |
| Main-branch test credibility gap                              | High     | High       | High   | High       | `multi_agent.rs` compile failures; unstable `v1_thesis`; CI exclusions. ([GitHub][9])                                               | Users cannot trust README claims or merge confidence when `main` is knowingly unstable. | Green the broken lanes or quarantine them transparently.                                       |      M |
| Security defaults are too weak for broader deployment         | High     | High       | High   | High       | Default PSK, minimal HTTP server, no TLS, unimplemented OAuth/mTLS. ([GitHub][5])                                                   | The code invites misuse as “production-ready” before the edge is hardened.              | Harden defaults, add TLS guidance/termination, verify local peers, remove default PSK.         |      M |
| Policy architecture is partly fictional at runtime            | High     | Medium     | Medium | High       | Policy crate says runtime evaluates it; runtime says policy is not wired in v1. ([GitHub][17])                                      | Governance claims lose credibility when the named authority is not in-path.             | Either wire it in or de-scope the claims/docs.                                                 |      M |
| Public release/contrib surface is missing                     | Medium   | High       | Medium | High       | No releases, no changelog, no `CONTRIBUTING.md`, internal issue IDs in `AGENTS.md`. ([GitHub][19])                                  | Limits adoption, onboarding, and long-term ecosystem health.                            | Add release cadence, changelog, contribution guide, public work-tracking note.                 |      S |
| Protocol/config churn can break integrators                   | Medium   | Medium     | Medium | High       | `multi_agent.rs` compile failures due added fields on wire structs. ([GitHub][3])                                                   | This becomes an ecosystem tax over time.                                                | Add compatibility tests and deprecation policy.                                                |      M |
| Resume state is not durable across restarts                   | Medium   | Medium     | Medium | High       | Session resume tokens are not persisted across process restarts. ([GitHub][7])                                                      | Limits resilience and smooth recovery.                                                  | Add optional persistent session state/store.                                                   |      L |

## 8. Recommendations and roadmap

### 5 quick wins (1–3 days)

1. **Unify the public truth.** Update `README.md`, `about/lay-and-land/operations/*`, and `app/tze_hud_app/config/production.toml` to the *actual* `TzeHudConfig` schema used by `WindowedRuntime`. ([GitHub][2])
2. **Add a canonical app boot test.** Mirror the `vertical_slice` production-boot test for `app/tze_hud_app/config/production.toml`, and fail CI if it falls back to headless/default behavior. ([GitHub][22])
3. **Kill unsafe defaults in release builds.** Refuse to start with the default PSK or with no config unless an explicit dev/override flag is present. ([GitHub][5])
4. **Stop advertising broken test commands.** Remove or annotate README commands for `multi_agent` until that lane compiles and passes again. ([GitHub][2])
5. **Add `CONTRIBUTING.md` and `CHANGELOG.md`.** Even minimal versions would materially improve external trust. 

### 5 medium improvements (1–3 weeks)

1. **Replace the MCP edge server.** Move from the minimal custom HTTP handler to a hardened server stack, then document TLS/rate-limiting expectations. ([GitHub][21])
2. **Make `main` honest.** Either fix `multi_agent`, `v1_thesis`, and budget assertions, or quarantine them in a clearly non-blocking lane with a public status note. ([GitHub][3])
3. **Wire policy or shrink claims.** Choose one: integrate `tze_hud_policy` into runtime mutation decisions, or rewrite docs to reflect the real enforcement path. ([GitHub][17])
4. **Add compatibility tests for wire/config evolution.** Protect `SessionInit`, `MutationBatch`, and canonical config examples from silent breaking changes. ([GitHub][3])
5. **Publish benchmark artifacts.** Turn the repo’s performance doctrine into a consumable baseline, not just internal intent. ([GitHub][14])

### 3 strategic investments (1–3 months+)

1. **Build a real release model.** Tagged versions, release notes, upgrade guidance, and packaged canonical assets/configs. This is the highest-leverage move for external trust. ([GitHub][19])
2. **Create an operator-facing production surface.** Metrics/export contract, service packaging, hardened transport, inspection tooling, and runbooks that match runtime reality. ([GitHub][23])
3. **Define the post-v1 contract before adding surface area.** Especially around media/WebRTC, persistence, and policy wiring, so the README stops acting like a future press release. ([GitHub][2])

## 9. Strengths worth preserving

* The **doctrine → RFC → code/test** mindset is a real competitive advantage here. Most repos at this stage do not have this level of conceptual alignment tooling. ([GitHub][24])
* The **pure scene graph** is the right foundation. Keeping rendering, I/O, and policy out of the data model pays long-term maintainability dividends. ([GitHub][11])
* The **runtime authority map** is the kind of architectural honesty that prevents future mud. Keep that. ([GitHub][8])
* The **testing imagination**—trace replay, soak, production-boot, dev-mode guard, vocabulary lint—is better than average and worth preserving even while branch health is repaired. ([GitHub][9])
* The **widget/component-profile approach** is a strong extensibility story that separates runtime mechanics from visual identity. ([GitHub][14])

## 10. Appendix

### Repository map

* **Canonical app:** `app/tze_hud_app` → binary `tze_hud`, described as the production entrypoint. ([GitHub][13])
* **Core crates:** `tze_hud_runtime`, `tze_hud_scene`, `tze_hud_protocol`, `tze_hud_mcp`, `tze_hud_config`, `tze_hud_policy`, `tze_hud_widget`, `tze_hud_resource`, `tze_hud_validation`, plus compositor/input/a11y/telemetry crates. ([GitHub][4])
* **Examples and test packages:** `examples/vertical_slice`, benchmark/render-artifact examples, and a dedicated `tests/integration` crate. ([GitHub][4])
* **Docs/specs:** `about/heart-and-soul` (doctrine), `about/legends-and-lore` (RFCs), `about/lay-and-land` (topology/ops), `openspec` (spec/change scaffolding). ([GitHub][12])
* **CI/scripts:** `.github/workflows/ci.yml`, plus scripts for vocabulary lint, MCP reachability, Windows build, and smoke flows. ([GitHub][9])

### Critical user flows

1. **gRPC session handshake:** auth → version negotiation → capability validation → session established → resume token. `session_server.rs` documents this explicitly. ([GitHub][7])
2. **MCP publishing path:** JSON-RPC tool call → scene mutation / zone-or-widget publish → shared scene visible to both MCP and gRPC consumers. ([GitHub][20])
3. **Config-driven startup:** app resolves config path, reads TOML, passes it into `WindowedRuntime`; runtime parses via `TzeHudConfig`, applies grants/hot sections, or falls back to headless default. ([GitHub][5])
4. **Production-boot validation path:** example production config is parsed and booted in CI; registered vs guest policy is asserted there. ([GitHub][22])

### Key hotspots

* `README.md` — biggest source of overclaim and stale config guidance. ([GitHub][2])
* `app/tze_hud_app/config/production.toml` — likely stale relative to the current loader. ([GitHub][25])
* `crates/tze_hud_runtime/src/lib.rs` — best single source for actual runtime authority and limits. ([GitHub][8])
* `crates/tze_hud_runtime/src/windowed.rs` — current truth for config loading, fallback behavior, and network startup semantics. ([GitHub][18])
* `crates/tze_hud_protocol/src/session_server.rs` — session complexity, resume behavior, and protocol reliability live here. ([GitHub][7])
* `.github/workflows/ci.yml` and `AGENTS.md` — current truth for branch health. ([GitHub][9])

### Evidence index

Most load-bearing evidence came from:
`README.md`; `about/heart-and-soul/v1.md`; `Cargo.toml`; `app/tze_hud_app/src/main.rs`; `app/tze_hud_app/config/production.toml`; `crates/tze_hud_runtime/src/lib.rs`; `crates/tze_hud_runtime/src/windowed.rs`; `crates/tze_hud_runtime/src/mcp.rs`; `crates/tze_hud_scene/src/graph.rs`; `crates/tze_hud_protocol/src/auth.rs`; `crates/tze_hud_protocol/src/session_server.rs`; `crates/tze_hud_config/src/loader.rs`; `crates/tze_hud_config/src/reload.rs`; `crates/tze_hud_policy/src/lib.rs`; `examples/vertical_slice/config/production.toml`; `examples/vertical_slice/tests/production_boot.rs`; `tests/integration/trace_regression.rs`; `tests/integration/soak.rs`; `.github/workflows/ci.yml`; `AGENTS.md`; `about/lay-and-land/operations/*`; plus repo-level GitHub pages for releases, issues, PRs, and Actions. ([GitHub][2])

## Verdict

Functional but accumulating debt

This repo already has the architecture, testing intent, and conceptual rigor of something substantial. But `main` currently shows a dangerous pattern: the doctrine is sharper than the public README, the canonical app/docs path appears out of sync with the actual config loader, and the repo openly acknowledges unstable tests on the main branch. That does not make the codebase weak; it makes trust in the *surface* weaker than trust in the *core*. If the next wave of work is spent converging truth and hardening defaults rather than adding more scope, this can become a very strong system.

[1]: https://github.com/Tzeusy/tze-hud/tree/main "https://github.com/Tzeusy/tze-hud/tree/main"
[2]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/README.md "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/README.md"
[3]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/AGENTS.md "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/AGENTS.md"
[4]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/Cargo.toml "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/Cargo.toml"
[5]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/app/tze_hud_app/src/main.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/app/tze_hud_app/src/main.rs"
[6]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/tests/integration/trace_regression.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/tests/integration/trace_regression.rs"
[7]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_protocol/src/session_server.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_protocol/src/session_server.rs"
[8]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_runtime/src/lib.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_runtime/src/lib.rs"
[9]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/.github/workflows/ci.yml "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/.github/workflows/ci.yml"
[10]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_protocol/src/auth.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_protocol/src/auth.rs"
[11]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_scene/src/graph.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_scene/src/graph.rs"
[12]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/about/heart-and-soul/README.md "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/about/heart-and-soul/README.md"
[13]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/app/tze_hud_app/Cargo.toml "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/app/tze_hud_app/Cargo.toml"
[14]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/about/heart-and-soul/v1.md "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/about/heart-and-soul/v1.md"
[15]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_config/src/reload.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_config/src/reload.rs"
[16]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_config/src/loader.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_config/src/loader.rs"
[17]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_policy/src/lib.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_policy/src/lib.rs"
[18]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_runtime/src/windowed.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_runtime/src/windowed.rs"
[19]: https://github.com/Tzeusy/tze-hud/releases "https://github.com/Tzeusy/tze-hud/releases"
[20]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_mcp/src/lib.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_mcp/src/lib.rs"
[21]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_runtime/src/mcp.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/crates/tze_hud_runtime/src/mcp.rs"
[22]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/examples/vertical_slice/tests/production_boot.rs "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/examples/vertical_slice/tests/production_boot.rs"
[23]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/about/lay-and-land/operations/OPERATOR_CHECKLIST.md "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/about/lay-and-land/operations/OPERATOR_CHECKLIST.md"
[24]: https://github.com/Tzeusy/tze-hud/tree/main/about "https://github.com/Tzeusy/tze-hud/tree/main/about"
[25]: https://raw.githubusercontent.com/Tzeusy/tze-hud/main/app/tze_hud_app/config/production.toml "https://raw.githubusercontent.com/Tzeusy/tze-hud/main/app/tze_hud_app/config/production.toml"
