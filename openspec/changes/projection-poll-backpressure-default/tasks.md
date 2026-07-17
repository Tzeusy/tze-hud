## 1. Projection Bounds Implementation

- [ ] 1.1 Update `DEFAULT_MAX_POLL_ITEMS` in `crates/tze_hud_projection/src/lib.rs` to `32`, while preserving the `32` pending-input-item limit and the `16_384` response-byte limit in `ProjectionBounds::default()`.
- [ ] 1.2 Add a deterministic authority regression test that a default poll returns the first 32 of 33 small FIFO inputs and reports the remaining item without exceeding the item cap.
- [ ] 1.3 Add a deterministic authority regression test that a default poll returns only the FIFO prefix fitting the response-byte cap when the item cap has not been reached, and leaves undelivered items accounted for.

## 2. Verification

- [ ] 2.1 Run the focused default-count and byte-cap regressions with `cargo test -p tze_hud_projection --features resident-grpc`.
- [ ] 2.2 Run `cargo clippy -p tze_hud_projection --all-targets --features resident-grpc -- -D warnings` and record the results in the implementation handoff.
