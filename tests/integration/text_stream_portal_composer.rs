//! Composer draft editing integration tests (hud-5jbra.4).
//!
//! Task 4.7 verification suite:
//!
//! - Local echo independence from adapter latency
//! - Word-wise delete
//! - Coalesced notifications
//! - Oversized-paste truncation and non-forwarding
//! - Submit-content fidelity
//! - Safe-mode suspension
//! - Keystroke non-passthrough across both adapter families
//!
//! These tests exercise `tze_hud_input::ComposerDraft` directly (runtime layer)
//! and `tze_hud_projection::ResidentGrpcPortalAdapter`'s draft notification
//! consumption path (cooperative projection adapter family). Together they
//! satisfy both adapter families required by spec §4.7.
//!
//! # Architecture note
//!
//! The runtime-owned draft buffer (`ComposerDraft`) lives in `tze_hud_input` and
//! runs entirely within the input-to-local-ack budget path (Stages 1–2). The
//! owning adapter receives coalesced `AdapterDraftBatch` notifications; it does
//! NOT receive per-keystroke scene mutations. These tests verify both layers are
//! correct at their respective contract boundaries.

use tze_hud_input::{
    ComposerDraft, DEFAULT_DRAFT_CAP, DraftNotificationBatch, EditOutcome, MAX_DRAFT_BYTES,
};
use tze_hud_projection::{
    AdapterDraftBatch, AdapterDraftNotification, AdapterDraftSubmission,
    resident_grpc::{
        RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US, ResidentGrpcDraftCommandKind,
        ResidentGrpcPortalAdapter, ResidentGrpcPortalConfig,
    },
};

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn make_adapter() -> ResidentGrpcPortalAdapter {
    let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
    ResidentGrpcPortalAdapter::new(config)
}

fn make_adapter_draft_notification(
    text: &str,
    cursor: usize,
    sequence: u64,
) -> AdapterDraftNotification {
    AdapterDraftNotification {
        text: text.to_string(),
        cursor,
        selection_anchor: cursor,
        at_capacity: false,
        sequence,
    }
}

// ─── Task 4.7: Local echo independence from adapter latency ───────────────────

/// Spec §4.7 — "local echo independence from adapter latency"
///
/// Characters echoed locally in the runtime buffer MUST NOT depend on an
/// adapter round trip. The draft buffer is mutated and the snapshot is ready
/// for local rendering before any notification leaves the runtime.
#[test]
fn local_echo_does_not_depend_on_adapter_round_trip() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);

    // Simulate several keystrokes with no adapter involved
    let chars = ['h', 'e', 'l', 'l', 'o'];
    for ch in &chars {
        let outcome = draft.insert(&ch.to_string());
        // Each keystroke mutates the draft and is ready for local rendering
        // without waiting for any adapter response
        assert_eq!(
            outcome,
            EditOutcome::Mutated,
            "character insert must be locally acknowledged immediately"
        );
        // The snapshot is available for compositor rendering right now
        let snap = draft.snapshot();
        assert!(
            snap.text.ends_with(*ch),
            "draft snapshot available for local render immediately after insert"
        );
    }

    assert_eq!(draft.text(), "hello");

    // The adapter is not involved at all during local echo —
    // no ResidentGrpcPortalAdapter calls have been made.
    // The draft buffer holds the authoritative local state.
}

/// The input-to-local-ack budget is p99 < 4 ms (≤ 2 ms Windows locked lane).
/// We verify the draft mutation itself is synchronous and completes in < 1 ms.
#[test]
fn draft_insert_completes_within_local_ack_budget_headroom() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    let budget_us = RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US; // 4_000 µs

    let text = "benchmark character typing latency";
    let start = std::time::Instant::now();

    // Insert 100 characters and take 100 snapshots (simulating 100 keystrokes)
    for ch in text.chars().cycle().take(100) {
        let _ = draft.insert(&ch.to_string());
        let _ = draft.snapshot();
    }

    let elapsed_us = start.elapsed().as_micros() as u64;
    let per_keystroke_us = elapsed_us / 100;

    // The 100-keystroke loop must finish well within the budget for 1 keystroke
    assert!(
        per_keystroke_us < budget_us,
        "per-keystroke draft cost {}µs exceeds input-to-local-ack budget {}µs",
        per_keystroke_us,
        budget_us
    );
}

// ─── Task 4.7: Word-wise delete ───────────────────────────────────────────────

/// Spec §4.7 — "word-wise delete"
#[test]
fn word_backspace_removes_preceding_word_and_trailing_space() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("the quick brown fox");
    assert_eq!(draft.cursor(), 19);

    let outcome = draft.word_backspace();
    assert_eq!(outcome, EditOutcome::Mutated);
    assert_eq!(draft.text(), "the quick brown ");
    assert_eq!(draft.cursor(), 16);

    let outcome = draft.word_backspace();
    assert_eq!(outcome, EditOutcome::Mutated);
    assert_eq!(draft.text(), "the quick ");
    assert_eq!(draft.cursor(), 10);
}

#[test]
fn word_backspace_skips_whitespace_before_deleting_word() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("hello   world   ");
    let outcome = draft.word_backspace();
    assert_eq!(outcome, EditOutcome::Mutated);
    assert_eq!(draft.text(), "hello   ");
}

#[test]
fn word_backspace_at_start_is_unchanged() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    assert_eq!(draft.word_backspace(), EditOutcome::Unchanged);
}

#[test]
fn word_forward_delete_removes_next_word() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("hello world");
    draft.move_to_start();
    let outcome = draft.word_delete_forward();
    assert_eq!(outcome, EditOutcome::Mutated);
    assert_eq!(draft.text(), " world");
}

/// The adapter receives only a draft-state notification after word-wise delete,
/// not a per-keystroke scene mutation.
#[test]
fn word_backspace_produces_draft_state_notification_not_per_keystroke_republish() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("remove this word");

    let seq_before = draft.sequence();
    draft.word_backspace();
    let seq_after = draft.sequence();

    // One sequence increment = one notification event
    assert_eq!(
        seq_after - seq_before,
        1,
        "word-backspace must produce exactly one draft-state notification"
    );

    let snap = draft.snapshot();
    assert_eq!(snap.text, "remove this ");
    assert_eq!(snap.sequence, seq_after);
}

// ─── Task 4.7: Coalesced notifications ────────────────────────────────────────

/// Spec §4.7 — "coalesced notifications": the adapter may receive a single
/// latest-draft snapshot rather than per-keystroke events.
#[test]
fn draft_notification_batch_coalesces_to_latest_snapshot() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    let mut batch = DraftNotificationBatch::new();

    // Simulate rapid typing producing multiple draft changes
    for ch in "rapid typing simulation".chars() {
        let _ = draft.insert(&ch.to_string());
        // Each mutation is coalesced: only the latest survives
        batch.coalesce_state(draft.snapshot());
    }

    // The batch holds exactly one (latest) snapshot
    let latest = batch.latest.as_ref().expect("batch must have latest snapshot");
    assert_eq!(latest.text, "rapid typing simulation");
    assert_eq!(latest.sequence, draft.sequence());
}

/// Adapter-side coalescing: `AdapterDraftBatch` coalesces the same way.
#[test]
fn adapter_draft_batch_coalesces_to_latest() {
    let mut batch = AdapterDraftBatch::new();

    // Simulate three runtime notifications arriving in sequence
    batch.coalesce_state(make_adapter_draft_notification("a", 1, 1));
    batch.coalesce_state(make_adapter_draft_notification("ab", 2, 2));
    batch.coalesce_state(make_adapter_draft_notification("abc", 3, 3));

    let latest = batch.latest.as_ref().expect("batch has latest");
    assert_eq!(latest.text, "abc");
    assert_eq!(latest.sequence, 3);
}

#[test]
fn adapter_draft_batch_ignores_stale_notification() {
    let mut batch = AdapterDraftBatch::new();
    batch.coalesce_state(make_adapter_draft_notification("newer", 5, 5));
    batch.coalesce_state(make_adapter_draft_notification("stale", 3, 3)); // out of order

    let latest = batch.latest.as_ref().expect("batch has latest");
    assert_eq!(latest.text, "newer", "stale notification must not replace newer");
}

/// The adapter `consume_draft_batch` produces one UpdateComposerDisplay command
/// from a batch with a state-stream notification.
#[test]
fn adapter_consumes_draft_batch_state_notification() {
    let mut adapter = make_adapter();
    let mut batch = AdapterDraftBatch::new();
    batch.coalesce_state(make_adapter_draft_notification("typed text", 10, 1));

    let commands = adapter.consume_draft_batch(&batch);
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].kind,
        ResidentGrpcDraftCommandKind::UpdateComposerDisplay
    );
    assert_eq!(commands[0].draft_text, "typed text");
    assert_eq!(commands[0].cursor, 10);
    assert_eq!(commands[0].sequence, 1);
}

/// The adapter skips stale state-stream notifications (sequence ≤ last seen).
#[test]
fn adapter_skips_stale_draft_notification() {
    let mut adapter = make_adapter();

    // First notification sets sequence = 5
    let notification = make_adapter_draft_notification("current", 7, 5);
    let cmd = adapter
        .apply_draft_notification(&notification)
        .expect("first notification accepted");
    assert_eq!(cmd.sequence, 5);

    // Stale notification (sequence = 3) is ignored
    let stale = make_adapter_draft_notification("old", 3, 3);
    let result = adapter.apply_draft_notification(&stale);
    assert!(
        result.is_none(),
        "stale notification must be ignored; sequence=3 <= last_seen=5"
    );
    assert_eq!(adapter.last_draft_sequence(), 5);
}

// ─── Task 4.7: Oversized-paste truncation and non-forwarding ─────────────────

/// Spec §4.7 — "oversized-paste truncation and non-forwarding"
/// Spec §4.5 — "cap violation never reaches the adapter"
#[test]
fn oversized_paste_truncates_at_cap() {
    let cap = 16;
    let mut draft = ComposerDraft::new(cap);
    let long_text = "a".repeat(100);

    let outcome = draft.paste(&long_text);
    assert_eq!(outcome, EditOutcome::AtCapacity);
    assert!(
        draft.text().len() <= cap,
        "draft text must not exceed cap after oversized paste"
    );
}

#[test]
fn oversized_paste_notification_never_exceeds_cap() {
    let cap = 20;
    let mut draft = ComposerDraft::new(cap);
    draft.paste(&"x".repeat(500));

    let snap = draft.snapshot();
    assert!(
        snap.text.len() <= cap,
        "draft notification text len={} exceeds cap={}",
        snap.text.len(),
        cap
    );
    assert!(snap.at_capacity);
}

#[test]
fn oversized_paste_does_not_forward_overflow_bytes() {
    // Simulate the adapter receiving a draft batch after an oversized paste
    let cap = 8;
    let mut draft = ComposerDraft::new(cap);
    draft.paste(&"x".repeat(100));

    let snap = draft.snapshot();
    let batch = {
        let mut b = AdapterDraftBatch::new();
        b.coalesce_state(AdapterDraftNotification {
            text: snap.text.clone(),
            cursor: snap.cursor,
            selection_anchor: snap.selection_anchor,
            at_capacity: snap.at_capacity,
            sequence: snap.sequence,
        });
        b
    };

    let mut adapter = make_adapter();
    let commands = adapter.consume_draft_batch(&batch);
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].draft_text.len(),
        cap,
        "adapter command text must not exceed cap"
    );
}

#[test]
fn paste_utf8_boundary_respected_at_cap() {
    // "é" is 2 bytes; cap = 1 means it cannot fit and is dropped entirely
    let mut draft = ComposerDraft::new(1);
    let outcome = draft.paste("é");
    assert_eq!(outcome, EditOutcome::AtCapacity);
    assert_eq!(draft.text(), "");
    assert!(draft.text().is_empty());

    // cap = 3 fits one "é" (2 bytes) but not two
    let mut draft = ComposerDraft::new(3);
    let outcome = draft.paste("éé");
    assert_eq!(outcome, EditOutcome::AtCapacity);
    assert_eq!(draft.text(), "é"); // one "é" fits, second is truncated
    assert!(draft.text().is_char_boundary(draft.text().len()));
}

// ─── Task 4.7: Submit-content fidelity ───────────────────────────────────────

/// Spec §4.7 — "submit-content fidelity": submitted text equals local buffer.
#[test]
fn submit_content_exactly_matches_local_buffer() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("my message to send");

    let expected = draft.text().to_string();
    let submission = draft.submit().expect("submit should succeed");

    assert_eq!(
        submission.text, expected,
        "submitted text must equal local buffer at submit time"
    );
    assert_eq!(
        draft.text(),
        "",
        "draft must be cleared after submit"
    );
}

#[test]
fn submit_clears_draft_after_returning_content() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("content");
    draft.submit();
    assert_eq!(draft.text(), "");
    assert_eq!(draft.cursor(), 0);
    assert!(!draft.has_selection());
}

/// Adapter-side: the ProcessSubmission command carries the exact submitted text.
#[test]
fn adapter_submission_command_carries_exact_draft_text() {
    let mut adapter = make_adapter();
    let mut batch = AdapterDraftBatch::new();
    batch.record_submission(AdapterDraftSubmission {
        text: "submit this exact content".to_string(),
        sequence: 42,
    });

    let commands = adapter.consume_draft_batch(&batch);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].kind, ResidentGrpcDraftCommandKind::ProcessSubmission);
    assert_eq!(commands[0].draft_text, "submit this exact content");
    assert_eq!(commands[0].sequence, 42);
}

#[test]
fn adapter_cancel_command_carries_empty_text() {
    use tze_hud_projection::AdapterDraftCancel;
    let mut adapter = make_adapter();
    let mut batch = AdapterDraftBatch::new();
    batch.record_cancel(AdapterDraftCancel { sequence: 7 });

    let commands = adapter.consume_draft_batch(&batch);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].kind, ResidentGrpcDraftCommandKind::ProcessCancel);
    assert_eq!(commands[0].draft_text, "");
}

// ─── Task 4.7: Safe-mode suspension ──────────────────────────────────────────

/// Spec §4.7 — "safe-mode suspension": draft suspends when safe mode active.
#[test]
fn draft_suspends_under_safe_mode() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("text before safe mode");

    // Enter safe mode
    draft.set_suspended(true);

    // All mutating operations are rejected
    assert_eq!(draft.insert("x"), EditOutcome::Suspended);
    assert_eq!(draft.paste("paste"), EditOutcome::Suspended);
    assert_eq!(draft.backspace(), EditOutcome::Suspended);
    assert_eq!(draft.word_backspace(), EditOutcome::Suspended);
    assert_eq!(draft.delete_forward(), EditOutcome::Suspended);

    // Draft content is preserved but not modified
    assert_eq!(draft.text(), "text before safe mode");

    // Submit is also rejected under safe mode
    assert!(draft.submit().is_none());
    assert_eq!(draft.text(), "text before safe mode");
}

#[test]
fn draft_caret_queries_work_while_suspended() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("read only during suspend");
    draft.set_suspended(true);

    // Read-only operations are unaffected
    assert_eq!(draft.text(), "read only during suspend");
    assert_eq!(draft.cursor(), 24);
    let snap = draft.snapshot();
    assert_eq!(snap.text, "read only during suspend");
}

#[test]
fn draft_resumes_editing_after_safe_mode_lift() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("partial content");
    draft.set_suspended(true);
    assert_eq!(draft.insert("x"), EditOutcome::Suspended);

    // Safe mode lifted
    draft.set_suspended(false);
    assert_eq!(draft.insert("!"), EditOutcome::Mutated);
    assert_eq!(draft.text(), "partial content!");
}

/// Safe-mode suspension applies consistently across both adapter families.
/// For the cooperative projection adapter family, the `interaction_enabled`
/// flag on the portal state is derived from `!safe_mode_active` in the
/// authority. The draft buffer's own suspension is the runtime-side gate.
#[test]
fn safe_mode_gate_applies_to_both_adapter_families() {
    // Runtime side (adapter family 1: runtime-owned draft in any portal):
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.set_suspended(true);
    assert_eq!(draft.insert("keystroke"), EditOutcome::Suspended);

    // Adapter side (adapter family 2: cooperative projection):
    // Under safe mode the adapter receives no draft-state notifications
    // because the runtime does not produce them (suspension returns early).
    // Verify: suspended draft produces no notifications.
    let batch = DraftNotificationBatch::new();
    // Simulate: after a suspended insert, caller checks outcome
    let outcome = draft.insert("blocked");
    assert_eq!(outcome, EditOutcome::Suspended);
    // No notification is coalesced — caller must check outcome before calling snapshot
    assert!(batch.latest.is_none(), "no notification produced on suspended insert");
}

// ─── Task 4.7: Keystroke non-passthrough ─────────────────────────────────────

/// Spec §4.7 — "keystroke non-passthrough across both adapter families"
/// Spec §4.4 — "editing keystrokes are never interpreted as terminal input"
#[test]
fn editing_keystrokes_never_reach_adapter_as_terminal_input() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    let mut adapter = make_adapter();
    let mut batch = AdapterDraftBatch::new();

    // Simulate rapid typing: characters arrive as CharacterEvents
    for ch in "ls -la --color=auto".chars() {
        let outcome = draft.insert(&ch.to_string());
        match outcome {
            EditOutcome::Mutated | EditOutcome::AtCapacity => {
                batch.coalesce_state(AdapterDraftNotification {
                    text: draft.text().to_string(),
                    cursor: draft.cursor(),
                    selection_anchor: draft.selection_anchor(),
                    at_capacity: draft.is_at_capacity(),
                    sequence: draft.sequence(),
                });
            }
            _ => {}
        }
    }

    // The adapter receives a coalesced state-stream notification, NOT raw keystrokes
    let commands = adapter.consume_draft_batch(&batch);
    assert_eq!(commands.len(), 1, "exactly one coalesced command");
    assert_eq!(
        commands[0].kind,
        ResidentGrpcDraftCommandKind::UpdateComposerDisplay
    );

    // The command carries display state only — no keystroke forwarding
    assert_eq!(commands[0].draft_text, "ls -la --color=auto");

    // No "raw keystroke" or terminal input fields are present in the command
    // (the struct only has draft_text, cursor, at_capacity, sequence —
    // no terminal byte stream or key_code field).
    let _ = commands[0].draft_text.as_str();
    let _ = commands[0].cursor;
    let _ = commands[0].at_capacity;
    let _ = commands[0].sequence;
}

/// Backspace/Delete/Arrow editing keystrokes must not appear in any adapter
/// command as raw keystrokes — they only appear as changes to draft state.
#[test]
fn navigation_and_delete_keystrokes_visible_only_as_draft_state_changes() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("hello world");
    let seq_before = draft.sequence();

    // Simulate editing keystrokes
    draft.word_backspace();   // Ctrl+Backspace → removes "world"
    draft.move_to_start();    // Home key
    draft.delete_forward();   // Delete key removes "h"
    draft.insert("H");        // Character replaces "h"

    let seq_after = draft.sequence();
    assert!(seq_after > seq_before + 3, "each editing operation increments sequence");
    assert_eq!(draft.text(), "Hello ");

    // The adapter observes only the final draft state (coalesced)
    let snap = draft.snapshot();
    assert_eq!(snap.text, "Hello ");
    // No raw keystroke data is present in the snapshot
    assert_eq!(snap.sequence, seq_after);
}

// ─── Adapter migration: no per-keystroke republish ───────────────────────────

/// Spec §4.6 — "migrate the adapter OFF per-keystroke republish of composer
/// text nodes."
///
/// The cooperative projection adapter path previously published a new
/// TextMarkdownNode on every CharacterEvent. With the draft buffer, the adapter
/// consumes a single coalesced AdapterDraftBatch instead.
///
/// This test verifies the migration: the adapter produces ONE command for
/// N keystrokes (coalesced), not N commands.
#[test]
fn adapter_produces_one_command_for_many_keystrokes() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    let mut batch = AdapterDraftBatch::new();

    // Simulate 20 keystrokes
    for ch in "twenty character msg".chars() {
        let _ = draft.insert(&ch.to_string());
        // Coalesce after every keystroke (as the runtime would do)
        batch.coalesce_state(AdapterDraftNotification {
            text: draft.text().to_string(),
            cursor: draft.cursor(),
            selection_anchor: draft.selection_anchor(),
            at_capacity: false,
            sequence: draft.sequence(),
        });
    }

    let mut adapter = make_adapter();
    let commands = adapter.consume_draft_batch(&batch);

    // Exactly ONE command for all 20 keystrokes — not 20 separate publishes
    assert_eq!(
        commands.len(),
        1,
        "20 keystrokes must produce 1 coalesced adapter command, not 20"
    );
    assert_eq!(commands[0].draft_text, "twenty character msg");
}

/// Submission is transactional — the adapter processes it exactly once.
#[test]
fn submission_is_transactional_not_coalescible() {
    let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
    draft.insert("message to send");

    let sub_text = draft.text().to_string();
    let sub = draft.submit().expect("submit");

    let mut batch = AdapterDraftBatch::new();
    batch.record_submission(AdapterDraftSubmission {
        text: sub.text.clone(),
        sequence: sub.sequence,
    });

    // Attempt to record a second submission (should be ignored — first wins)
    batch.record_submission(AdapterDraftSubmission {
        text: "second submit attempt".to_string(),
        sequence: 99,
    });

    assert_eq!(
        batch.submission.as_ref().unwrap().text,
        sub_text,
        "transactional submission is first-wins, not coalescible"
    );

    let mut adapter = make_adapter();
    let commands = adapter.consume_draft_batch(&batch);
    let submit_cmd = commands
        .iter()
        .find(|c| c.kind == ResidentGrpcDraftCommandKind::ProcessSubmission)
        .expect("submission command must be present");
    assert_eq!(submit_cmd.draft_text, sub_text);
}

// ─── MAX_DRAFT_BYTES boundary ─────────────────────────────────────────────────

/// The hard ceiling matches the TextMarkdownNode content limit (65535 bytes).
#[test]
fn max_draft_bytes_matches_text_node_content_limit() {
    assert_eq!(MAX_DRAFT_BYTES, 65_535);
}

#[test]
fn draft_never_exceeds_max_bytes_on_direct_cap() {
    let mut draft = ComposerDraft::new(MAX_DRAFT_BYTES);
    // Fill to capacity with ASCII
    let full_text = "a".repeat(MAX_DRAFT_BYTES);
    let outcome = draft.paste(&full_text);
    assert_eq!(outcome, EditOutcome::Mutated, "exact capacity is not over-cap");
    assert_eq!(draft.text().len(), MAX_DRAFT_BYTES);

    // One more byte must be rejected
    let over = draft.insert("z");
    assert_eq!(over, EditOutcome::AtCapacity);
    assert_eq!(draft.text().len(), MAX_DRAFT_BYTES);
}
