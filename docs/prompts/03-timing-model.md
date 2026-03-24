# Epic 3: Timing Model

> **Dependencies:** Epic 0 (test infrastructure), Epic 1 (scene types for timestamp fields)
> **Depended on by:** Epics 5, 6, 9 (input timestamps, session clock sync, event timing)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/timing-model/spec.md`
> **Secondary specs:** `session-protocol/spec.md` (clock sync), `validation-framework/spec.md`

## Prompt

Create a `/beads-writer` epic for **timing model implementation** — the temporal semantics layer that ensures "arrival time ≠ presentation time" and gives the platform deterministic, testable timing behavior.

### Context

The timing model defines clock domains, presentation scheduling, sync groups, expiration enforcement, and the injectable `Clock` trait for deterministic testing. The existing crate `crates/tze_hud_scene/src/clock.rs` already has `Clock` trait, `SystemClock`, and `TestClock`. The timing-model spec mandates that all timestamp fields encode their clock domain via `_wall_us` or `_mono_us` suffixes.

### Epic structure

Create an epic with **4 implementation beads**:

#### 1. Clock domain types and validation (depends on Epic 1 identity types)
Implement strong timestamp types per `timing-model/spec.md` Requirement: Clock Domain Naming Convention.
- `WallUs` and `MonoUs` wrapper types — prevent accidental mixing of clock domains
- All public API timestamp fields must use typed wrappers, not raw `u64`
- Conversion between domains is explicit and requires a calibration offset
- `DurationUs` for deltas and frame-relative values (no clock domain)
- **Acceptance:** Compile-time enforcement: passing `WallUs` where `MonoUs` is expected fails to compile. All existing timestamp fields in scene/protocol crates migrated to typed wrappers.
- **Spec refs:** `timing-model/spec.md` Requirement: Clock Domain Naming Convention, lines 23-30

#### 2. TimingHints and presentation scheduling (depends on #1)
Implement the TimingHints struct and no-earlier-than scheduling per `timing-model/spec.md` Requirement: TimingHints, Requirement: No-Earlier-Than Scheduling.
- TimingHints: `present_at_wall_us`, `expires_at_wall_us`, `sequence`, `priority`, `coalesce_key`, `sync_group`
- No-earlier-than: content with `present_at_wall_us` in the future is held, not displayed early
- Expiration: content past `expires_at_wall_us` is removed within one frame
- Stale/future rejection: mutations with timestamps > 60s before session open or > 10s in the future are rejected
- **Acceptance:** Presentation scheduling scenarios from spec pass. Expiration enforcement verified with `TestClock` time injection. Stale/future rejection produces structured errors.
- **Spec refs:** `timing-model/spec.md` Requirement: TimingHints, Requirement: No-Earlier-Than Scheduling, Requirement: Expiration Enforcement

#### 3. Sync groups (depends on #2)
Implement sync group coordination per `timing-model/spec.md` Requirement: Sync Groups.
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
