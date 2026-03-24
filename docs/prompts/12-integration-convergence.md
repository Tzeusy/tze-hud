# Epic 12: Integration and Convergence

> **Dependencies:** Epics 1–11 (all subsystem implementations)
> **Depended on by:** None — this is the final epic
> **Primary spec:** `openspec/changes/v1-mvp-standards/tasks.md` §13, `validation-framework/spec.md` (Layers 2-4)
> **Secondary specs:** All 12 subsystem specs (cross-subsystem integration contracts)

## Prompt

> **Before starting:** Read `docs/prompts/PREAMBLE.md` for authority rules, doctrine guardrails, and v1 scope tagging requirements that apply to every bead.

Create a `/beads-writer` epic for **integration and convergence** — the final pass that verifies all subsystems work together, realizes the remaining validation layers, and proves the v1 thesis.

### Context

This epic runs after all subsystem epics are substantially complete. It focuses on cross-subsystem integration contracts, the higher validation layers (Layer 2 SSIM, Layer 3 performance, Layer 4 artifacts), and the v1 success criteria from `heart-and-soul/v1.md`: three resident agents coexisting, lease model working, zones as LLM-first surface, p99 latencies measured, headless fully functional, and validation architecture operational.

### Epic structure

Create an epic with **6 convergence beads**:

#### 1. Multi-agent integration test (depends on Epics 1, 4, 6)
End-to-end test: three resident agents connect via gRPC, acquire leases, publish to zones, create tiles, and coexist without interference.
- Agent A: weather dashboard (create tiles, update content at 1Hz)
- Agent B: notification agent (publish to notification zone, stack contention)
- Agent C: media agent (publish to subtitle zone, latest-wins contention)
- Verify: namespace isolation, lease priority shedding, zone contention resolution, no cross-agent content leakage
- **Acceptance:** `three_agents_contention` test scene passes end-to-end. All three agents render independently. Zone contention resolves per policy. Namespace isolation holds.
- **Spec refs:** `scene-graph/spec.md` (namespace isolation), `lease-governance/spec.md` (priority), `scene-events/spec.md` (subscription filtering)

#### 2. Session lifecycle integration (depends on Epics 6, 4, 11)
End-to-end session test: connect → auth → lease → mutate → disconnect → reconnect → resume → safe mode → freeze → close.
- Full session lifecycle exercised
- Reconnection delivers full SceneSnapshot
- Safe mode suspends all leases, cancels freeze
- After safe mode exit, leases resume, mutations accepted
- **Acceptance:** All session state transitions verified end-to-end. Reconnection within grace succeeds. Safe mode/freeze interaction correct. `disconnect_reclaim_multiagent` test scene passes.
- **Spec refs:** `session-protocol/spec.md` (full lifecycle), `system-shell/spec.md` (safe mode), `lease-governance/spec.md` (disconnect grace)

#### 3. Layer 2: SSIM visual regression (depends on Epic 2 compositor, Epic 0 scene registry)
Implement Layer 2 validation per `validation-framework/spec.md` Requirement: Layer 2 - Visual Regression.
- Generate golden images for all 25 test scenes
- SSIM comparison: structural similarity > 0.99 for identical scenes
- Detect regressions: layout shifts, color drift, z-order errors, missing content
- CI integration: fail on SSIM drop below threshold
- **Acceptance:** Golden images generated for all 25 scenes. SSIM comparison operational. Intentional regression detected and flagged.
- **Spec refs:** `validation-framework/spec.md` Requirement: Layer 2 - Visual Regression via SSIM

#### 4. Layer 3: Performance validation with calibration (depends on Epic 2 runtime)
Implement Layer 3 validation per `validation-framework/spec.md` Requirement: Layer 3 - Performance Validation.
- Hardware-normalized calibration harness: three-dimensional calibration (CPU, GPU, memory bandwidth)
- Uncalibrated results are warnings, not pass/fail
- Budget assertions for all quantitative targets from all subsystem specs
- CI retention: performance history tracked across branches
- **Acceptance:** Calibration harness produces calibration vector. All budget assertions pass on calibrated hardware. Uncalibrated runs produce warnings, not failures.
- **Spec refs:** `validation-framework/spec.md` Requirement: Layer 3 - Performance Validation, Requirement: Hardware-Normalized Calibration

#### 5. Layer 4: Developer visibility artifacts (depends on #3, #4)
Implement Layer 4 per `validation-framework/spec.md` Requirement: Layer 4 - Developer Visibility Artifacts.
- Artifact schema: `test_results/{timestamp}-{branch}/` with `index.html` and `manifest.json`
- Each run produces: spec IDs covered, benchmark context, scene images, diff summaries, latency quantiles, protocol traces
- LLM-readable: structured JSON output suitable for automated analysis
- **Acceptance:** Artifact directory produced on test run. Manifest contains all required fields. Index navigable. JSON output parseable.
- **Spec refs:** `validation-framework/spec.md` Requirement: Layer 4 - Developer Visibility Artifacts

#### 6. V1 thesis proof (depends on all above)
Final validation that the v1 success criteria from `heart-and-soul/v1.md` are met:
- An LLM can hold a tile and have it render at 60fps
- The lease model works (auth, capabilities, TTL, revocation)
- Multiple agents coexist without interference
- Performance is real (p99 latencies measured and tested)
- The validation architecture works (5 layers operational)
- Zones work as LLM-first surface (single MCP call, no scene context needed)
- Headless mode fully functional (no display server, CI on software GPU)
- **Acceptance:** All 7 v1 success criteria demonstrated. All 25 test scenes pass all applicable layers. All budget assertions pass on calibrated hardware.
- **Spec refs:** `heart-and-soul/v1.md` success criteria, all 12 subsystem specs

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite specific spec files and requirements for each integration contract
2. **Cross-epic references** — which epic's implementation this bead validates
3. **Acceptance criteria** — concrete end-to-end test scenarios
4. **Artifact expectations** — what each test produces (images, traces, JSON reports)
5. **V1 success criteria mapping** — which of the 7 thesis points this bead proves

### Dependency chain

```
Epics 1–11 ──→ #1 Multi-Agent Integration
           ──→ #2 Session Lifecycle Integration
           ──→ #3 Layer 2 SSIM ──→ #5 Layer 4 Artifacts ──→ #6 V1 Thesis Proof
           ──→ #4 Layer 3 Perf ──→ #5
```
