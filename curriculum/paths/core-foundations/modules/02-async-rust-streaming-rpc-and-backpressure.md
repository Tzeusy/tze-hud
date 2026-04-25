# Async Rust, Streaming RPC, and Backpressure

- Estimated smart-human study time: 8 hours
- Keep every module at or below 10 hours.

## Why This Module Matters

The repo’s resident control path is not “some API layer.” It is a single multiplexed bidirectional session stream with strict handshake, ordering, and backpressure rules. If you do not understand async Rust, bounded concurrency, and schema/codegen boundaries, the protocol code will look noisier than it really is and you will underestimate compatibility risk.

## Learning Goals

- Explain why the resident control plane uses one gRPC stream per agent.
- Understand how protobuf layout and generated code constrain safe edits.
- Recognize transactional, state-stream, and ephemeral message behavior under pressure.

## Subsection: Streaming Sessions and Wire Compatibility

### Why This Matters Here

`tze_hud` uses one stream to carry mutations, heartbeats, leases, subscriptions, uploads, and telemetry-related traffic. That only works if you understand envelope protocols, ordering, sequence numbers, and what counts as a wire break versus an internal refactor.

### Technical Deep Dive

The foundational idea is transport multiplexing: instead of many long-lived streams per concern, one session stream carries different message classes inside typed envelopes. That reduces coordination overhead but increases the importance of clear message design.

With protobuf-based protocols, field numbers and message structure are part of the public contract. Generated Rust types are rebuild artifacts, but the `.proto` files define compatibility. Renaming or renumbering a field is not “cleanup” if remote agents, tests, or persisted expectations depend on it.

Async Rust adds another layer: bounded channels, explicit concurrency, and non-blocking state machines are the mechanism that keeps one noisy path from stalling the runtime. Under pressure, transactional traffic must preserve atomicity, state-stream traffic can coalesce, and ephemeral traffic can be dropped. Those are transport semantics, not implementation accidents.

### Where It Appears In The Repo

- `openspec/specs/session-protocol/spec.md`
- `crates/tze_hud_protocol/proto/`
- `crates/tze_hud_protocol/build.rs`
- `crates/tze_hud_protocol/src/session_server.rs`
- `crates/tze_hud_protocol/tests/session_fsm.rs`
- `crates/tze_hud_protocol/tests/backpressure.rs`

### Sample Q&A

- Q: Why is “one bidirectional stream per agent” a deliberate rule here?
  A: Because the system wants few fat streams with explicit message classes, not many thin streams that hit HTTP/2 concurrency limits and complicate coordination.
- Q: Why is renumbering a protobuf field risky even if local Rust code compiles?
  A: Because field numbers define the wire contract; changing them can silently break compatibility with clients, tests, and generated code expectations.

### Progress

- [ ] Exposed: I can define session stream, envelope, backpressure, and codegen boundary
- [ ] Working: I can explain why this repo uses one stream per agent
- [ ] Working: I can answer the sample Q&A without looking
- [ ] Contribution-ready: I can name at least one change to a `.proto` file that would be wire-safe and one that would not

### Mastery Check

Target level: `working`

You should be able to explain how protocol messages flow through a single agent session and why queue semantics matter to correctness.

## Module Mastery Gate

- [ ] I can summarize the stream/session model without notes
- [ ] I can explain why protobuf layout is a compatibility concern
- [ ] I can point to the main protocol handler and at least one backpressure test
- [ ] I can distinguish transactional, state-stream, and ephemeral behavior

## What This Module Unlocks Next

It makes the timing module legible, because scheduling and expiry fields only make sense once the session transport and envelope model are clear.

