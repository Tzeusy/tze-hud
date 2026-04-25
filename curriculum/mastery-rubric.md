# Mastery Rubric

## Levels

- `exposed`: I recognize the vocabulary, can define the main terms, and know roughly why the topic exists.
- `working`: I can explain the concept in my own words, connect it to the repo, and answer the sample Q&A without notes.
- `contribution-ready`: I can use the concept to predict failure modes, compatibility risks, or safe/unsafe edits in this repository.

## Checkbox Meaning

- `[ ]` means the statement is not yet true in a way I could demonstrate out loud.
- `[X]` means I can actually do what the statement says, not that I merely read the section.

## When `working` Is Enough

Most modules in this curriculum only need `working` mastery before you start reading code. That is enough to:
- keep crate responsibilities legible
- understand why tests and specs are written the way they are
- avoid obvious category errors such as confusing wall time with monotonic time

## When `contribution-ready` Is Required

Aim for `contribution-ready` before you:
- edit `.proto` files or generated-protocol-adjacent structs
- change runtime scheduling, queueing, or admission logic
- alter lease, capability, privacy, or degradation behavior
- touch upload, dedup, resource identity, zone/widget, or telemetry semantics

## Path-Level Guidance

You are ready to move from study into exploratory repo reading when:
- you can explain the six module titles without notes
- you can say why each one is part of `tze_hud` specifically
- you know which topics are active v1 behavior and which are deferred or speculative

You are ready for a first safe contribution when:
- modules 1 through 4 are at least `working`
- modules 5 and 6 are at least `working`
- you can identify one likely compatibility or invariant risk in any change you propose
