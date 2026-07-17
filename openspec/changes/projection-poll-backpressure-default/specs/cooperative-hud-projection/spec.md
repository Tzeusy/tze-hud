## MODIFIED Requirements

### Requirement: Bounded Backpressure and Expiry
Projection operations SHALL enforce configurable bounds for output payload size, retained transcript bytes, pending input count, pending input byte size, polling result size, caller-scoped list result size, and portal update rate. When bounds are exceeded, the daemon SHALL reject, truncate, coalesce, expire, or summarize according to explicit policy while preserving transactional HUD input semantics.

Source: RFC 0013 sections 2.1, 3.4, and 4.3; RFC 0005 section 2.5

Unless deployment configuration sets stricter values, v1 defaults SHALL be: `max_output_bytes_per_call = 16384`, `max_status_text_bytes = 512`, `max_retained_transcript_bytes = 262144`, `max_visible_transcript_bytes = 16384`, `max_pending_input_items = 32`, `max_pending_input_bytes_per_item = 4096`, `max_pending_input_total_bytes = 32768`, `max_poll_items = 32`, `max_poll_response_bytes = 16384`, `max_list_items = 8`, `max_portal_updates_per_second = 10`, and `owner_token_ttl_wall_us = 86_400_000_000` (24 hours; see the Projection Operation Authorization requirement). Oversized output and input submissions SHALL be rejected with stable error codes rather than truncated silently. Retained transcript overflow SHALL prune oldest non-visible logical units first while preserving coherent visible-window order. Portal update-rate overflow SHALL coalesce output by `coalesce_key` or append order into the next permitted visible-window update.

#### Scenario: oversized output is bounded
- **WHEN** an attached LLM session publishes output larger than the configured per-operation limit
- **THEN** the daemon SHALL reject the update with `PROJECTION_OUTPUT_TOO_LARGE`
- **AND** it SHALL NOT materialize the oversized payload into scene nodes

#### Scenario: pending input queue reaches limit
- **WHEN** the pending HUD input queue for a projection reaches its configured maximum
- **THEN** the daemon SHALL preserve already accepted transactional input items
- **AND** new submissions SHALL be rejected with `PROJECTION_INPUT_QUEUE_FULL` and a visible bounded-state indication rather than silently dropped

#### Scenario: retained transcript overflow prunes oldest non-visible units
- **WHEN** appending a transcript unit would exceed `max_retained_transcript_bytes`
- **THEN** the projection authority SHALL prune oldest non-visible logical units until retained bytes are within budget
- **AND** it SHALL preserve the coherent visible transcript window required by the text-stream portal contract

#### Scenario: poll item count cap binds independently of bytes
- **WHEN** an attached LLM session omits `max_items` or requests a value greater than `max_poll_items`, omits `max_bytes` or requests a value greater than `max_poll_response_bytes`, and in a configuration whose `max_pending_input_items` exceeds `max_poll_items` polls 33 eligible FIFO input items whose combined response payload fits within `max_poll_response_bytes`
- **THEN** the daemon SHALL return only the first 32 items and report the remaining pending item through compact remaining counts
- **AND** it SHALL NOT return a thirty-third item merely because response-byte budget remains

#### Scenario: poll response byte cap binds independently of item count
- **WHEN** an attached LLM session omits `max_bytes` or requests a value greater than `max_poll_response_bytes`, omits `max_items` or requests a value greater than `max_poll_items`, and has 32 or fewer equal-sized eligible FIFO input items whose cumulative response payload exceeds `max_poll_response_bytes` before the item count cap is reached
- **THEN** the daemon SHALL return only the FIFO prefix that fits within `max_poll_response_bytes` and report the undelivered items through compact remaining counts and bytes
- **AND** it SHALL NOT exceed the response-byte cap merely because fewer than `max_poll_items` items were returned
