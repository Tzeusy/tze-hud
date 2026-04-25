# Time, Clocks, and Sync-Safe Scheduling

- Estimated smart-human study time: 6 hours
- Keep every module at or below 10 hours.

## Why This Module Matters

This repository treats time as a first-class API concept. If you confuse wall clock with monotonic time, or arrival time with presentation time, you can make changes that appear to work locally but violate the spec and break sync-sensitive behavior.

## Learning Goals

- Distinguish wall, monotonic, display, and media clock domains.
- Explain what `present_at_wall_us`, `expires_at_wall_us`, and sync groups mean.
- Understand why no-earlier-than scheduling matters in a compositor.

## Subsection: Clock Domains, Scheduling, and Sync Groups

### Why This Matters Here

Many systems can get away with “just apply the update when it arrives.” A compositor that cares about responsiveness, expiry, and coordinated presentation cannot. `tze_hud` relies on explicit timing fields and frame-boundary evaluation, so timing rules are part of correctness, not just performance tuning.

### Technical Deep Dive

The broad concept is clock-domain separation. Wall time is for externally meaningful schedules. Monotonic time is for internal latency and TTL calculations because it does not jump backward. The display clock sets actual presentation cadence. Media clocks exist for synchronized AV systems, even when v1 keeps them mostly deferred.

In systems like this, `present_at_wall_us` means “not before this time,” not “this would be nice if possible.” The compositor quantizes decisions to frames, so a scheduled mutation is held until the first frame whose presentation point is at or after the target time. That preserves temporal intent and avoids racey best-effort behavior.

Sync groups build on that idea by letting related updates coordinate under an explicit policy. The underlying transferable concept is that time-aware systems often need structured grouping and controlled deferral rather than independent immediate application.

### Where It Appears In The Repo

- `openspec/specs/timing-model/spec.md`
- `about/legends-and-lore/rfcs/0003-timing.md`
- `openspec/specs/session-protocol/spec.md`
- `crates/tze_hud_protocol/tests/clock_domain.rs`
- `tests/integration/soak.rs`

### Sample Q&A

- Q: Why can’t the runtime use arrival time as presentation time?
  A: Because network arrival is not the same as intended presentation; the system needs explicit timing semantics and frame-quantized scheduling.
- Q: Why is monotonic time preferred for internal latency measurement?
  A: Because it does not jump backward with wall-clock corrections, making budgets and expiry logic stable.

### Progress

- [ ] Exposed: I can define wall clock, monotonic clock, display clock, and sync group
- [ ] Working: I can explain the meaning of a no-earlier-than schedule
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can describe one bug that would result from mixing wall and monotonic timestamps

### Mastery Check

Target level: `working`

You should be able to explain how a timed mutation moves from session input to a frame boundary and why the repo names timestamp fields by clock domain.

## Module Mastery Gate

- [ ] I can summarize the four clock domains in plain language
- [ ] I can explain `present_at_wall_us` and `expires_at_wall_us`
- [ ] I can point to where clock-domain rules are specified and tested
- [ ] I can explain why sync groups exist

## What This Module Unlocks Next

It prepares you for the governance module, where leases, expiry, redaction timing, attention, and degradation all depend on explicit temporal reasoning.

