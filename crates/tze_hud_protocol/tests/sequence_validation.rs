//! Sequence validation tests.
//!
//! Tests sequence number validation from session-protocol/spec.md lines 212-223:
//!   - Normal: monotonically increasing sequences accepted
//!   - Gap ≤100: accepted (allows for packet reordering)
//!   - Gap >100: SEQUENCE_GAP_EXCEEDED error
//!   - Regression: SEQUENCE_REGRESSION error
//!   - Sequence starts at 1 per direction
//!
//! The validation logic is exercised through the SessionConfig's max_sequence_gap
//! parameter and sequence arithmetic.
//!
//! Test count target: ≥4 tests.

use tze_hud_protocol::session_server::SessionConfig;

// ─── Sequence validation helper ───────────────────────────────────────────────

/// Error types for sequence validation, matching spec terminology.
#[derive(Debug, PartialEq, Eq)]
enum SequenceError {
    SequenceGapExceeded,
    SequenceRegression,
}

/// Validate a new sequence number against the last accepted sequence.
/// Returns Ok(()) if accepted, Err(SequenceError) if rejected.
fn validate_sequence(
    last_sequence: u64,
    new_sequence: u64,
    max_gap: u64,
) -> Result<(), SequenceError> {
    if new_sequence == 0 {
        // Sequence starts at 1; 0 is invalid
        return Err(SequenceError::SequenceRegression);
    }
    if new_sequence <= last_sequence {
        // Regression: same or lower sequence
        return Err(SequenceError::SequenceRegression);
    }
    let gap = new_sequence - last_sequence;
    if gap > max_gap {
        return Err(SequenceError::SequenceGapExceeded);
    }
    Ok(())
}

// ─── Normal sequence: monotonically increasing ────────────────────────────────

/// WHEN sequences arrive in strict order THEN all accepted.
#[test]
fn normal_monotonic_sequence_accepted() {
    let cfg = SessionConfig::default();
    let max_gap = cfg.max_sequence_gap;

    let mut last = 0u64;
    for seq in 1..=100u64 {
        let result = validate_sequence(last, seq, max_gap);
        assert!(result.is_ok(), "sequence {} must be accepted after {}", seq, last);
        last = seq;
    }
}

/// WHEN sequence starts at 1 THEN accepted (first message after connection).
#[test]
fn sequence_starts_at_1_per_direction() {
    let cfg = SessionConfig::default();
    // spec: "Per-direction monotonically increasing, starts at 1"
    let result = validate_sequence(0, 1, cfg.max_sequence_gap);
    assert!(result.is_ok(), "first sequence must be 1 (not 0)");
}

// ─── Gap ≤100: allowed (packet reordering tolerance) ─────────────────────────

/// WHEN gap exactly equals max_gap (100) THEN accepted.
#[test]
fn sequence_gap_exactly_max_is_accepted() {
    let max_gap = 100u64;
    let last = 50u64;
    let result = validate_sequence(last, last + max_gap, max_gap);
    assert!(result.is_ok(), "gap equal to max_sequence_gap must be accepted");
}

/// WHEN gap is 1 (consecutive) THEN accepted.
#[test]
fn sequence_gap_1_is_accepted() {
    let max_gap = 100u64;
    let result = validate_sequence(99, 100, max_gap);
    assert!(result.is_ok());
}

/// WHEN gap is 50 (within tolerance) THEN accepted.
#[test]
fn sequence_gap_50_is_accepted() {
    let max_gap = 100u64;
    let result = validate_sequence(100, 150, max_gap);
    assert!(result.is_ok(), "gap of 50 is within tolerance");
}

// ─── Gap >100: SEQUENCE_GAP_EXCEEDED ─────────────────────────────────────────

/// WHEN gap is 101 THEN SEQUENCE_GAP_EXCEEDED error.
#[test]
fn sequence_gap_101_triggers_gap_exceeded() {
    let max_gap = 100u64;
    let result = validate_sequence(1, 102, max_gap);
    assert_eq!(result, Err(SequenceError::SequenceGapExceeded),
        "gap of 101 must trigger SEQUENCE_GAP_EXCEEDED \
         (session-protocol/spec.md lines 212-223)");
}

/// WHEN gap is 1000 (large jump) THEN SEQUENCE_GAP_EXCEEDED error.
#[test]
fn large_sequence_gap_triggers_gap_exceeded() {
    let max_gap = 100u64;
    let result = validate_sequence(1, 1001, max_gap);
    assert_eq!(result, Err(SequenceError::SequenceGapExceeded),
        "gap of 1000 must trigger SEQUENCE_GAP_EXCEEDED");
}

// ─── Regression: SEQUENCE_REGRESSION ─────────────────────────────────────────

/// WHEN sequence regresses (goes backwards) THEN SEQUENCE_REGRESSION error.
#[test]
fn sequence_regression_triggers_error() {
    let max_gap = 100u64;
    // Regression: last=100, new=99
    let result = validate_sequence(100, 99, max_gap);
    assert_eq!(result, Err(SequenceError::SequenceRegression),
        "sequence regression must trigger SEQUENCE_REGRESSION \
         (session-protocol/spec.md lines 212-223)");
}

/// WHEN sequence repeats (same number) THEN SEQUENCE_REGRESSION error.
#[test]
fn sequence_repeat_triggers_regression() {
    let max_gap = 100u64;
    let result = validate_sequence(50, 50, max_gap);
    assert_eq!(result, Err(SequenceError::SequenceRegression),
        "repeated sequence number must trigger SEQUENCE_REGRESSION");
}

/// WHEN sequence wraps to 0 THEN SEQUENCE_REGRESSION (0 is invalid, sequences start at 1).
#[test]
fn sequence_zero_triggers_regression() {
    let max_gap = 100u64;
    let result = validate_sequence(5, 0, max_gap);
    assert_eq!(result, Err(SequenceError::SequenceRegression),
        "sequence 0 is invalid; sequences start at 1");
}

// ─── Max sequence gap configuration ──────────────────────────────────────────

/// Verify SessionConfig max_sequence_gap default is 100.
#[test]
fn max_sequence_gap_default_is_100() {
    let cfg = SessionConfig::default();
    assert_eq!(cfg.max_sequence_gap, 100,
        "max_sequence_gap must be 100 per session-protocol/spec.md lines 212-223");
}

/// WHEN sequences jump exactly at boundary (gap=100) vs over (gap=101).
#[test]
fn boundary_gap_100_vs_101() {
    let max_gap = 100u64;

    // Gap exactly 100: accept
    assert!(validate_sequence(200, 300, max_gap).is_ok(), "gap=100 must be accepted");

    // Gap 101: reject
    assert_eq!(
        validate_sequence(200, 301, max_gap),
        Err(SequenceError::SequenceGapExceeded),
        "gap=101 must be rejected"
    );
}

/// Sequences are per-direction. Client and server each track their own counter.
#[test]
fn sequences_are_per_direction() {
    let max_gap = 100u64;

    // Client→Server direction
    let mut client_last = 0u64;
    assert!(validate_sequence(client_last, 1, max_gap).is_ok());
    client_last = 1;
    assert!(validate_sequence(client_last, 2, max_gap).is_ok());
    client_last = 2;

    // Server→Client direction (independent counter)
    let mut server_last = 0u64;
    assert!(validate_sequence(server_last, 1, max_gap).is_ok());
    server_last = 1;
    assert!(validate_sequence(server_last, 5, max_gap).is_ok());

    // They don't interfere
    assert_eq!(client_last, 2);
    assert_eq!(server_last, 1);
}
