## 1. Projection Bounds Implementation

- [ ] 1.1 Update `DEFAULT_MAX_POLL_ITEMS` in `crates/tze_hud_projection/src/lib.rs` to `32`, while preserving the `32` pending-input-item limit and the `16_384` response-byte limit in `ProjectionBounds::default()`.
- [ ] 1.2 Add deterministic authority regressions using a test configuration whose `max_pending_input_items` exceeds `max_poll_items`, proving that both omitted and oversized `max_items` requests return the first 32 of 33 small FIFO inputs and report the remaining item without exceeding the item cap.
- [ ] 1.3 Add deterministic authority regressions proving that both omitted and oversized `max_bytes` requests return only the FIFO prefix fitting the response-byte cap when the item cap has not been reached, and leave undelivered items accounted for.

## 2. Verification

- [ ] 2.1 Run the focused default-count and byte-cap regressions with `cargo test -p tze_hud_projection --features resident-grpc`.
- [ ] 2.2 Run `cargo clippy -p tze_hud_projection --all-targets --features resident-grpc -- -D warnings` and record the results in the implementation handoff.
