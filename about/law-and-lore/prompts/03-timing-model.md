# Epic 3: Timing Model

> **Dependencies:** Epic 0 (test infrastructure), Epic 1 (scene types for timestamp fields)
> **Depended on by:** Epics 5, 6, 9 (input timestamps, session clock sync, event timing)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/timing-model/spec.md`
> **Secondary specs:** `session-protocol/spec.md` (clock sync), `validation-framework/spec.md`

## Prompt

> **Before starting:** Read `docs/prompts/PREAMBLE.md` for authority rules, doctrine guardrails, and v1 scope tagging requirements that apply to every bead.

Create a `/beads-writer` epic for **timing model implementation** — the temporal semantics layer that ensures "arrival time ≠ presentation time" and gives the platform deterministic, testable timing behavior.

### Context

The timing model defines clock domains, presentation scheduling, sync groups, expiration enforcement, and the injectable `Clock` trait for deterministic testing. The existing crate `crates/tze_hud_scene/src/clock.rs` already has `Clock` trait, `SystemClock`, and `TestClock`. The timing-model spec mandates that all timestamp fields encode their clock domain via `_wall_us` or `_mono_us` suffixes.

### Epic structure

Create an epic with **4 implementation beads**:

#### 1. Clock domain types and validation (depends on Epic 1 identity types)
Implement the clock domain naming convention per `timing-model/spec.md` Requirement: Clock Domain Naming Convention.
- The spec mandates: all timestamp fields MUST encode their clock domain via `_wall_us` (UTC wall clock) or `_mono_us` (monotonic system clock) suffixes. Fields without a clock-domain suffix are deltas or frame-relative values, not timestamps.
- Recommended implementation: `WallUs` and `MonoUs` newtype wrappers over `u64` to enforce the convention at compile time; `DurationUs` for deltas. This goes beyond what the spec literally requires (naming convention only) but prevents the class of bugs the convention targets.
- Conversion between domains is explicit and requires a calibration offset
- **Acceptance:** All timestamp fields in scene/protocol/session crates use correct `_wall_us`/`_mono_us` suffixes. If wrapper types are used: passing `WallUs` where `MonoUs` is expected fails to compile.
- **Spec refs:** `timing-model/spec.md` Requirement: Clock Domain Naming Convention, lines 23-30

#### 2. TimingHints and presentation scheduling (depends on #1)
Implement the TimingHints struct and presentation scheduling per `timing-model/spec.md` Requirement: Timing Fields on Payloads, Requirement: Frame Quantization.
- TimingHints: `present_at_wall_us`, `expires_at_wall_us`, `sequence`, `priority`, `coalesce_key`, `sync_group`
- No-earlier-than (doctrine: "arrival time ≠ presentation time"): content with `present_at_wall_us` in the future is held, not displayed early
- Expiration: content past `expires_at_wall_us` is removed within one frame
- Stale/future rejection: mutations with `present_at_wall_us` > 60s before session open are rejected with TIMESTAMP_TOO_OLD; mutations > 5 minutes (300,000,000µs, the `max_future_schedule_us` default) in the future are rejected with TIMESTAMP_TOO_FUTURE
- **Acceptance:** Presentation scheduling scenarios from spec pass. Expiration enforcement verified with `TestClock` time injection. Stale/future rejection produces structured errors with correct codes.
- **Spec refs:** `timing-model/spec.md` Requirement: Timing Fields on Payloads, Requirement: Arrival Time Is Not Presentation Time, Requirement: Expiration Enforcement

#### 3. Sync groups (depends on #2)
Implement sync group coordination per `timing-model/spec.md` Requirement: Sync Group Membership and Lifecycle, Requirement: Sync Group Commit Policies.
- Sync group: named set of tiles that present together
- Policies: AllOrDefer (all members ready or none present), AvailableMembers (present what's ready)
- Force-commit after sync_group_timeout to prevent indefinite blocking
- Sync drift budget: < 500µs between members in the same group
- **Acceptance:** `sync_group_media` test scene passes. AllOrDefer and AvailableMembers policies verified. Drift budget assertion passes.
- **Spec refs:** `timing-model/spec.md` Requirement: Sync Groups, Requirement: Sync Drift Budget

#### 4. Relative scheduling primitives (depends on #2)
Implement relative timing per `timing-model/spec.md` Requirement: Relative Scheduling.
- `after(duration)`: present N microseconds after the previous content in the same tile
- `with(tile_id)`: present simultaneously with content in another tile
- These compose with absolute `present_at_wall_us` — relative is sugar over absolute
- **Acceptance:** Relative scheduling resolves to correct absolute timestamps. Composition with absolute timing verified.
- **Spec refs:** `timing-model/spec.md` Requirement: Relative Scheduling

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `timing-model/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference the exact spec scenarios
3. **Acceptance criteria** — which Epic 0 tests must pass, plus type-safety compile checks
4. **Crate/file location** — primarily `crates/tze_hud_scene/src/clock.rs` and new timing module
5. **Cross-epic contracts** — how timing types are consumed by session-protocol (Epic 6) and input-model (Epic 5)

### Dependency chain

```
Epic 1 ──→ #1 Clock Domain Types ──→ #2 TimingHints ──→ #3 Sync Groups
                                                     ──→ #4 Relative Scheduling
```
