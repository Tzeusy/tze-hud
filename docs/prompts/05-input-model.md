# Epic 5: Input Model

> **Dependencies:** Epic 0 (test infrastructure), Epic 1 (scene graph, hit regions), Epic 2 (frame pipeline), Epic 3 (monotonic timestamps)
> **Depended on by:** Epic 6 (session carries input events), Epic 8 (policy evaluates input), Epic 11 (shell chrome input)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/input-model/spec.md`
> **Secondary specs:** `scene-graph/spec.md` (HitRegionNode), `timing-model/spec.md` (timestamp_mono_us)

## Prompt

Create a `/beads-writer` epic for **input model** — the interaction contract that ensures local feedback under 4ms p99 and provides the abstract command-input model for future non-pointer devices.

### Context

The input model defines the local-first interaction contract: touch/pointer acknowledgement happens locally and instantly; remote semantics follow. The existing `crates/tze_hud_input/` has `InputProcessor` with 10 tests covering hit-testing, hover, press/release, and activation. Epic 0 provides budget assertions for input-to-local-ack (< 4ms p99) and hit-test latency (< 100µs p99).

### Epic structure

Create an epic with **5 implementation beads**:

#### 1. Focus tree and management (depends on Epic 1 hit regions)
Implement the focus model per `input-model/spec.md` Requirement: Focus Tree.
- At most one tile has focus at a time; runtime manages transitions
- Agents cannot steal focus — only request it; runtime decides
- Focus rings rendered in chrome layer (never occluded by agent content)
- Tab switch clears focus
- **Acceptance:** Focus uniqueness invariant holds under property testing. Focus request/grant/deny scenarios from spec pass. `input_highlight` test scene validates focus ring rendering.
- **Spec refs:** `input-model/spec.md` Requirement: Focus Tree, Requirement: Focus Transitions

#### 2. Pointer event dispatch pipeline (depends on #1, Epic 2 frame pipeline)
Implement the 5-stage dispatch per `input-model/spec.md` Requirement: Pointer Event Dispatch Pipeline.
- Stages: capture → hit-test → route → deliver → bubble
- Chrome always wins hit-test (checked first)
- Tiles in z-order, then nodes within tile
- Pointer events: down, up, move, enter, leave, click, cancel
- All events carry `timestamp_mono_us` (monotonic domain per timing-model)
- **Acceptance:** `test_input_to_local_ack_p99_within_budget()` passes (< 4ms). `test_hit_test_p99_within_budget()` passes (< 100µs). All pointer dispatch scenarios from spec pass.
- **Spec refs:** `input-model/spec.md` Requirement: Pointer Event Dispatch Pipeline, Requirement: Pointer Event Types

#### 3. Keyboard events and command input (depends on #1)
Implement keyboard and abstract command model per `input-model/spec.md` Requirement: Keyboard Event Types, Requirement: Command Input Model.
- KeyDownEvent, KeyUpEvent with physical key_code, logical key value, modifiers, repeat flag
- Abstract commands: NAVIGATE_NEXT, NAVIGATE_PREV, ACTIVATE, CANCEL, CONTEXT, SCROLL_UP, SCROLL_DOWN
- Commands delivered via CommandInputEvent with device source (KEYBOARD, DPAD, VOICE, etc.)
- **Acceptance:** Keyboard events carry correct fields. Abstract commands mapped from keyboard shortcuts. CommandInputEvent scenarios from spec pass.
- **Spec refs:** `input-model/spec.md` Requirement: Keyboard Event Types, Requirement: Command Input Model

#### 4. Event batching and coalescing (depends on #2, #3)
Implement per-frame batching per `input-model/spec.md` Requirement: Event Serialization and Batching.
- Multiple input events for same agent within one frame batched into single delivery
- Events ordered by `timestamp_mono_us` ascending within batch
- Pointer move events coalesced (intermediate positions dropped, latest wins)
- Transactional events (down, up, click, key, command, focus, capture) never dropped
- **Acceptance:** Same-frame events delivered in single batch. Coalescing reduces pointer move count. Transactional events survive backpressure. Batching scenarios from spec pass.
- **Spec refs:** `input-model/spec.md` Requirement: Event Serialization and Batching, `session-protocol/spec.md` Requirement: EventBatch Variant Filtering

#### 5. Local feedback guarantee (depends on #2, #1)
Implement the local-first feedback contract per `input-model/spec.md` Requirement: Local Feedback Guarantee.
- Visual acknowledgement (press state, focus ring, hover highlight) happens within 4ms p99
- No remote roundtrip in the local feedback path
- Feedback is rendered by compositor, not by agent
- Input-to-scene-commit < 50ms for local mutations
- **Acceptance:** `test_input_to_local_ack_p99_within_budget()` passes. Local feedback rendered without network dependency. Feedback visible in Layer 1 pixel tests for `input_highlight` scene.
- **Spec refs:** `input-model/spec.md` Requirement: Local Feedback Guarantee, `runtime-kernel/spec.md` Requirement: Frame Time Budget

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `input-model/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference the exact spec scenarios
3. **Acceptance criteria** — which Epic 0 latency budget tests must pass
4. **Crate/file location** — `crates/tze_hud_input/src/`
5. **Latency budgets** — specific p99 targets from the spec

### Dependency chain

```
Epics 1+2+3 ──→ #1 Focus Tree ──→ #2 Pointer Dispatch ──→ #4 Batching/Coalescing
                              ──→ #3 Keyboard/Command
                              ──→ #5 Local Feedback
```
