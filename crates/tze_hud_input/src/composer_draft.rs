//! Runtime-owned bounded plain-text draft buffer for portal composer regions.
//!
//! # Design (spec §4, hud-5jbra.4)
//!
//! The draft buffer is runtime-owned. Local echo happens within the
//! input-to-local-ack budget; the owning adapter receives coalesced
//! **state-stream** draft-state notifications and one final **transactional**
//! submission. The adapter is never on the echo path.
//!
//! ## Editing primitives
//!
//! - Character insert at cursor (from CharacterEvent).
//! - Caret movement: character, word, line-start/end.
//! - Selection: keyboard (shift + movement) and pointer.
//! - Backspace / Delete: character-wise and word-wise (Ctrl/Alt + Backspace,
//!   Ctrl/Alt + Delete).
//! - Paste with UTF-8-boundary truncation at the cap; at-capacity feedback.
//!
//! ## Excluded operations (normative §4.4)
//!
//! - No IME composition (v1-reserved).
//! - No undo/redo.
//! - No rich text.
//! - No multi-caret.
//! - Editing keystrokes are never terminal input.
//!
//! ## Governance (§4.5)
//!
//! Under safe mode the draft suspends: `insert`, `paste`, and submit are
//! rejected while `suspended == true`. Caret and selection queries still work
//! (read-only) but mutations are no-ops. Draft content is subject to portal
//! redaction policy (tracked by caller via `suspended`).
//!
//! ## Notification classes (§4.3, four-message taxonomy)
//!
//! | Path | Class |
//! |------|-------|
//! | Local echo | not a message — never leaves the runtime |
//! | `DraftStateNotification` | state-stream, coalescible to latest snapshot |
//! | `DraftSubmission` | transactional |
//!
//! # Size cap
//!
//! `ComposerDraft::new(cap)` enforces `cap ≤ MAX_DRAFT_BYTES` (65535). Paste
//! and insert that would exceed the cap are truncated at a UTF-8 character
//! boundary; `EditOutcome::AtCapacity` is returned and no notification leaves
//! the runtime with content exceeding the cap.

use unicode_segmentation::UnicodeSegmentation;

/// Hard ceiling matching the TextMarkdownNode content limit (spec §4.3, §4.5).
pub const MAX_DRAFT_BYTES: usize = 65_535;

/// Default lower draft cap (suitable for typical composer inputs).
pub const DEFAULT_DRAFT_CAP: usize = 4_096;

// ─── Outcome types ────────────────────────────────────────────────────────────

/// Outcome of a mutating draft operation.
///
/// `Mutated` — draft content or caret changed; emit a
/// [`DraftStateNotification`] and re-render locally.
///
/// `Unchanged` — operation was a no-op (e.g. backspace at offset 0, move at
/// boundary). No notification needed.
///
/// `AtCapacity` — a paste or insert was truncated at the byte cap. Draft
/// contains the truncated prefix. Caller MUST render at-capacity feedback.
/// A notification is still emitted (with the truncated content).
///
/// `Suspended` — the draft is suspended under safe mode; the operation was
/// rejected without side effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// Draft content or caret moved; emit notification.
    Mutated,
    /// No change; no notification needed.
    Unchanged,
    /// Paste/insert exceeded cap; content truncated to cap boundary.
    /// Emit notification and render at-capacity feedback.
    AtCapacity,
    /// Draft suspended under safe mode; operation rejected.
    Suspended,
}

// ─── Draft state notification (state-stream) ──────────────────────────────────

/// Coalescible draft-state notification delivered to the owning adapter.
///
/// Message class: **state-stream**. The adapter MAY receive a single
/// latest-snapshot rather than per-keystroke events when the runtime coalesces
/// under delivery pressure. The submitted text delivered at submit time MUST
/// reflect the local buffer at the moment of submission, not any notification
/// snapshot.
///
/// Spec: §4.3, §4.7 (coalesced notifications scenario).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftStateNotification {
    /// Current draft text. Never exceeds the configured byte cap.
    pub text: String,
    /// Cursor byte offset into `text` (start of caret / anchor of selection).
    pub cursor: usize,
    /// Selection anchor byte offset. When equal to `cursor` there is no
    /// active selection.
    pub selection_anchor: usize,
    /// True if the draft is currently at or over its byte cap (i.e., the last
    /// operation was truncated).
    pub at_capacity: bool,
    /// Monotonic sequence counter, incremented on every mutation.
    /// Allows the adapter to detect skipped notifications.
    pub sequence: u64,
}

// ─── Draft submission (transactional) ─────────────────────────────────────────

/// Transactional submission event produced on Enter / explicit submit.
///
/// Message class: **transactional**. The submitted text is exactly the
/// local draft buffer at the moment of submission, capped and UTF-8-valid.
///
/// Spec: §4.3 (submission transactional), §4.7 (submit-content fidelity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftSubmission {
    /// Draft text at submit time. Never exceeds the configured byte cap.
    pub text: String,
    /// Sequence number at submission time.
    pub sequence: u64,
}

/// Cancel event: draft cleared without submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftCancel {
    pub sequence: u64,
}

// ─── Selection ────────────────────────────────────────────────────────────────

/// An active selection range `[start, end)` over byte offsets in the draft.
/// Both offsets are valid UTF-8 boundaries; `start <= end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: usize,
    pub end: usize,
}

impl Selection {
    /// True when the selection is empty (caret position only).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Cursor position when there is no selection (normalized anchor offset).
    pub fn cursor(&self) -> usize {
        self.start
    }
}

// ─── ComposerDraft ────────────────────────────────────────────────────────────

/// Runtime-owned bounded plain-text draft buffer.
///
/// One `ComposerDraft` per focused composer region. The runtime creates it on
/// focus-in and destroys it on focus-out (or on submit/cancel).
///
/// All mutating methods return an [`EditOutcome`]; callers SHOULD emit a
/// [`DraftStateNotification`] via [`snapshot`] whenever `Mutated` or
/// `AtCapacity` is returned, and SHOULD render at-capacity feedback on
/// `AtCapacity`.
///
/// # Invariants
///
/// - `text.len() <= cap <= MAX_DRAFT_BYTES`
/// - `cursor` and `selection_anchor` are valid byte offsets into `text`
/// - `cursor <= text.len()` and `selection_anchor <= text.len()`
///
/// [`snapshot`]: ComposerDraft::snapshot
#[derive(Clone, Debug)]
pub struct ComposerDraft {
    text: String,
    /// Cursor position in bytes (insert/caret position).
    cursor: usize,
    /// Selection anchor in bytes. Equal to `cursor` when no selection.
    selection_anchor: usize,
    /// Byte cap (≤ MAX_DRAFT_BYTES).
    cap: usize,
    /// Monotonic mutation sequence.
    sequence: u64,
    /// True when the draft is suspended under safe mode (§4.5).
    suspended: bool,
}

impl ComposerDraft {
    /// Create a new draft buffer with the given byte `cap`.
    ///
    /// `cap` is clamped to `[1, MAX_DRAFT_BYTES]`.
    pub fn new(cap: usize) -> Self {
        let cap = cap.clamp(1, MAX_DRAFT_BYTES);
        Self {
            text: String::new(),
            cursor: 0,
            selection_anchor: 0,
            cap,
            sequence: 0,
            suspended: false,
        }
    }

    // ── Read-only accessors ───────────────────────────────────────────────

    /// Current draft text. Never exceeds the byte cap.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Cursor byte offset.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Selection anchor byte offset (equal to cursor when no selection active).
    pub fn selection_anchor(&self) -> usize {
        self.selection_anchor
    }

    /// Active selection `[start, end)`, or a zero-width cursor position.
    pub fn selection(&self) -> Selection {
        let (start, end) = if self.selection_anchor <= self.cursor {
            (self.selection_anchor, self.cursor)
        } else {
            (self.cursor, self.selection_anchor)
        };
        Selection { start, end }
    }

    /// True when an active selection is present (start != end).
    pub fn has_selection(&self) -> bool {
        self.cursor != self.selection_anchor
    }

    /// Whether the draft is at or exceeds the byte cap.
    pub fn is_at_capacity(&self) -> bool {
        self.text.len() >= self.cap
    }

    /// Configured byte cap.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Whether the draft is suspended under safe mode.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Mutation sequence counter. Incremented on every successful mutation.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Build a [`DraftStateNotification`] snapshot of current state.
    pub fn snapshot(&self) -> DraftStateNotification {
        DraftStateNotification {
            text: self.text.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
            at_capacity: self.is_at_capacity(),
            sequence: self.sequence,
        }
    }

    // ── Governance ────────────────────────────────────────────────────────

    /// Suspend the draft under safe mode (§4.5).
    ///
    /// While suspended, mutating operations return `EditOutcome::Suspended`.
    /// The draft text is preserved but not modified. Submission is also
    /// rejected.
    pub fn set_suspended(&mut self, suspended: bool) {
        self.suspended = suspended;
    }

    // ── Editing operations ────────────────────────────────────────────────

    /// Insert `character` at the cursor position (from a CharacterEvent).
    ///
    /// If a selection is active, the selected range is first deleted, then the
    /// character is inserted. Control characters and `\r`/`\n` are rejected as
    /// no-ops (newlines end editing via submit).
    ///
    /// Returns `AtCapacity` if the inserted text was truncated at the byte cap.
    pub fn insert(&mut self, character: &str) -> EditOutcome {
        if self.suspended {
            return EditOutcome::Suspended;
        }
        // Reject control characters and line endings (spec §4.4: no newlines in draft)
        if character
            .chars()
            .any(|c| c == '\r' || c == '\n' || c.is_control())
        {
            return EditOutcome::Unchanged;
        }
        // Delete selection first if active
        if self.has_selection() {
            let sel = self.selection();
            self.text.replace_range(sel.start..sel.end, "");
            self.cursor = sel.start;
            self.selection_anchor = sel.start;
        }
        let remaining = self.cap.saturating_sub(self.text.len());
        if remaining == 0 {
            // Nothing was mutated; do not bump sequence to avoid redundant notifications.
            return EditOutcome::AtCapacity;
        }
        // Truncate to remaining byte capacity at UTF-8 boundary
        let truncated = truncate_at_utf8_boundary(character, remaining);
        let at_cap = truncated.len() < character.len();
        self.text.insert_str(self.cursor, truncated);
        self.cursor += truncated.len();
        self.selection_anchor = self.cursor;
        self.bump_sequence();
        if at_cap {
            EditOutcome::AtCapacity
        } else {
            EditOutcome::Mutated
        }
    }

    /// Paste `text` at the cursor position, truncating at the UTF-8 boundary
    /// at the remaining byte cap.
    ///
    /// Oversized content is truncated and `AtCapacity` is returned. No
    /// untruncated bytes are stored or forwarded to any notification.
    ///
    /// Spec: §4.5 (Composer Draft Bounds and Paste Caps).
    pub fn paste(&mut self, text: &str) -> EditOutcome {
        if self.suspended {
            return EditOutcome::Suspended;
        }
        if text.is_empty() {
            return EditOutcome::Unchanged;
        }
        // Delete selection first if active
        if self.has_selection() {
            let sel = self.selection();
            self.text.replace_range(sel.start..sel.end, "");
            self.cursor = sel.start;
            self.selection_anchor = sel.start;
        }
        let remaining = self.cap.saturating_sub(self.text.len());
        if remaining == 0 {
            // Nothing was mutated; do not bump sequence to avoid redundant notifications.
            return EditOutcome::AtCapacity;
        }
        let truncated = truncate_at_utf8_boundary(text, remaining);
        let at_cap = truncated.len() < text.len();
        self.text.insert_str(self.cursor, truncated);
        self.cursor += truncated.len();
        self.selection_anchor = self.cursor;
        self.bump_sequence();
        if at_cap {
            EditOutcome::AtCapacity
        } else {
            EditOutcome::Mutated
        }
    }

    /// Backspace: delete the character before the cursor (or the selection).
    ///
    /// If a selection is active, deletes the selection. Otherwise removes the
    /// grapheme cluster immediately before the cursor.
    pub fn backspace(&mut self) -> EditOutcome {
        if self.suspended {
            return EditOutcome::Suspended;
        }
        if self.has_selection() {
            return self.delete_selection();
        }
        if self.cursor == 0 {
            return EditOutcome::Unchanged;
        }
        let prev = prev_grapheme_boundary(&self.text, self.cursor);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.selection_anchor = prev;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Delete: delete the character after the cursor (or the selection).
    pub fn delete_forward(&mut self) -> EditOutcome {
        if self.suspended {
            return EditOutcome::Suspended;
        }
        if self.has_selection() {
            return self.delete_selection();
        }
        if self.cursor == self.text.len() {
            return EditOutcome::Unchanged;
        }
        let next = next_grapheme_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..next, "");
        // cursor stays at same position
        self.selection_anchor = self.cursor;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Word-wise backspace: delete from cursor to the start of the preceding word.
    ///
    /// Mirrors the behaviour of Ctrl/Alt + Backspace in the exemplar adapter.
    /// Spec: §4.2, §4.7 (word-wise delete scenario).
    pub fn word_backspace(&mut self) -> EditOutcome {
        if self.suspended {
            return EditOutcome::Suspended;
        }
        if self.has_selection() {
            return self.delete_selection();
        }
        let word_start = word_delete_start(&self.text, self.cursor);
        if word_start == self.cursor {
            return EditOutcome::Unchanged;
        }
        self.text.replace_range(word_start..self.cursor, "");
        self.cursor = word_start;
        self.selection_anchor = word_start;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Word-wise forward delete: delete from cursor to the end of the next word.
    pub fn word_delete_forward(&mut self) -> EditOutcome {
        if self.suspended {
            return EditOutcome::Suspended;
        }
        if self.has_selection() {
            return self.delete_selection();
        }
        let word_end = word_delete_end(&self.text, self.cursor);
        if word_end == self.cursor {
            return EditOutcome::Unchanged;
        }
        self.text.replace_range(self.cursor..word_end, "");
        self.selection_anchor = self.cursor;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Move the cursor one grapheme cluster to the left, collapsing any selection.
    pub fn move_left(&mut self) -> EditOutcome {
        if self.has_selection() {
            let start = self.selection().start;
            self.cursor = start;
            self.selection_anchor = start;
            self.bump_sequence();
            return EditOutcome::Mutated;
        }
        if self.cursor == 0 {
            return EditOutcome::Unchanged;
        }
        let prev = prev_grapheme_boundary(&self.text, self.cursor);
        self.cursor = prev;
        self.selection_anchor = prev;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Move the cursor one grapheme cluster to the right, collapsing any selection.
    pub fn move_right(&mut self) -> EditOutcome {
        if self.has_selection() {
            let end = self.selection().end;
            self.cursor = end;
            self.selection_anchor = end;
            self.bump_sequence();
            return EditOutcome::Mutated;
        }
        if self.cursor == self.text.len() {
            return EditOutcome::Unchanged;
        }
        let next = next_grapheme_boundary(&self.text, self.cursor);
        self.cursor = next;
        self.selection_anchor = next;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Move the cursor to the start of the preceding word (collapsing selection).
    pub fn move_word_left(&mut self) -> EditOutcome {
        let target = word_delete_start(&self.text, self.cursor);
        if target == self.cursor && !self.has_selection() {
            return EditOutcome::Unchanged;
        }
        self.cursor = target;
        self.selection_anchor = target;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Move the cursor to the end of the next word (collapsing selection).
    pub fn move_word_right(&mut self) -> EditOutcome {
        let target = word_delete_end(&self.text, self.cursor);
        if target == self.cursor && !self.has_selection() {
            return EditOutcome::Unchanged;
        }
        self.cursor = target;
        self.selection_anchor = target;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Move the cursor to the start of the text (collapsing selection).
    pub fn move_to_start(&mut self) -> EditOutcome {
        if self.cursor == 0 && !self.has_selection() {
            return EditOutcome::Unchanged;
        }
        self.cursor = 0;
        self.selection_anchor = 0;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Move the cursor to the end of the text (collapsing selection).
    pub fn move_to_end(&mut self) -> EditOutcome {
        let end = self.text.len();
        if self.cursor == end && !self.has_selection() {
            return EditOutcome::Unchanged;
        }
        self.cursor = end;
        self.selection_anchor = end;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    // ── Selection editing operations (shift + movement) ───────────────────

    /// Extend the selection one grapheme cluster to the left (shift + Left).
    pub fn select_left(&mut self) -> EditOutcome {
        if self.cursor == 0 {
            return EditOutcome::Unchanged;
        }
        let prev = prev_grapheme_boundary(&self.text, self.cursor);
        self.cursor = prev;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Extend the selection one grapheme cluster to the right (shift + Right).
    pub fn select_right(&mut self) -> EditOutcome {
        if self.cursor == self.text.len() {
            return EditOutcome::Unchanged;
        }
        let next = next_grapheme_boundary(&self.text, self.cursor);
        self.cursor = next;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Extend the selection to the start of the text (shift + Home).
    pub fn select_to_start(&mut self) -> EditOutcome {
        if self.cursor == 0 {
            return EditOutcome::Unchanged;
        }
        self.cursor = 0;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Extend the selection to the end of the text (shift + End).
    pub fn select_to_end(&mut self) -> EditOutcome {
        let end = self.text.len();
        if self.cursor == end {
            return EditOutcome::Unchanged;
        }
        self.cursor = end;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    /// Set pointer selection: `anchor` is the click position, `cursor` is the
    /// drag position. Both are raw byte offsets from the UI layer and are
    /// snapped down to the nearest valid UTF-8 character boundary before
    /// storage, preventing panics in subsequent `replace_range`/`insert_str`
    /// calls on multi-byte characters.
    pub fn set_pointer_selection(&mut self, anchor: usize, cursor: usize) -> EditOutcome {
        debug_assert!(anchor <= self.text.len(), "anchor out of bounds");
        debug_assert!(cursor <= self.text.len(), "cursor out of bounds");
        let mut anchor = anchor.min(self.text.len());
        while !self.text.is_char_boundary(anchor) {
            anchor -= 1;
        }
        let mut cursor = cursor.min(self.text.len());
        while !self.text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        if self.selection_anchor == anchor && self.cursor == cursor {
            return EditOutcome::Unchanged;
        }
        self.selection_anchor = anchor;
        self.cursor = cursor;
        self.bump_sequence();
        EditOutcome::Mutated
    }

    // ── Submit / Cancel ───────────────────────────────────────────────────

    /// Submit the draft: returns a [`DraftSubmission`] with the current buffer
    /// content and resets the draft to empty.
    ///
    /// Returns `None` if the draft is suspended (§4.5) or if the buffer is
    /// empty. Callers that want to allow explicit empty-submit must check
    /// `draft.text().is_empty()` first and decide their own policy.
    ///
    /// Spec: §4.3 (submission is transactional, content == local buffer).
    /// Spec: §4.7 (submit-content fidelity).
    pub fn submit(&mut self) -> Option<DraftSubmission> {
        if self.suspended {
            return None;
        }
        if self.text.is_empty() {
            return None;
        }
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.selection_anchor = 0;
        self.bump_sequence();
        Some(DraftSubmission {
            text,
            sequence: self.sequence,
        })
    }

    /// Cancel the draft: clear without submission.
    ///
    /// Returns `None` if the draft is suspended.
    pub fn cancel(&mut self) -> Option<DraftCancel> {
        if self.suspended {
            return None;
        }
        self.text.clear();
        self.cursor = 0;
        self.selection_anchor = 0;
        self.bump_sequence();
        Some(DraftCancel {
            sequence: self.sequence,
        })
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn bump_sequence(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
    }

    fn delete_selection(&mut self) -> EditOutcome {
        if !self.has_selection() {
            return EditOutcome::Unchanged;
        }
        let sel = self.selection();
        self.text.replace_range(sel.start..sel.end, "");
        self.cursor = sel.start;
        self.selection_anchor = sel.start;
        self.bump_sequence();
        EditOutcome::Mutated
    }
}

impl Default for ComposerDraft {
    /// Create a draft buffer with the default cap (`DEFAULT_DRAFT_CAP`).
    fn default() -> Self {
        Self::new(DEFAULT_DRAFT_CAP)
    }
}

// ─── Unicode helpers ──────────────────────────────────────────────────────────

/// Truncate `s` to at most `max_bytes`, cutting on a UTF-8 char boundary.
///
/// Spec: §4.5 — paste truncated at a UTF-8 character boundary at the cap.
pub fn truncate_at_utf8_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk back from max_bytes until we land on a valid char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Byte offset of the grapheme boundary immediately before `byte_pos`.
fn prev_grapheme_boundary(text: &str, byte_pos: usize) -> usize {
    if byte_pos == 0 {
        return 0;
    }
    let before = &text[..byte_pos];
    let mut clusters = before.grapheme_indices(true);
    // Find the last cluster start
    let mut last_start = 0;
    for (idx, _) in &mut clusters {
        last_start = idx;
    }
    last_start
}

/// Byte offset of the grapheme boundary immediately after `byte_pos`.
fn next_grapheme_boundary(text: &str, byte_pos: usize) -> usize {
    if byte_pos == text.len() {
        return byte_pos;
    }
    let after = &text[byte_pos..];
    let mut clusters = after.grapheme_indices(true);
    match clusters.next() {
        Some((_, gc)) => byte_pos + gc.len(),
        None => byte_pos,
    }
}

/// Byte offset of the start of the word to delete on word-backspace.
///
/// Mirrors `composer_word_delete_start` from the exemplar adapter (Python):
/// skip trailing whitespace, then skip the non-whitespace word.
pub fn word_delete_start(text: &str, cursor: usize) -> usize {
    let before = &text[..cursor];
    let bytes = before.as_bytes();
    let mut pos = cursor;
    // Skip trailing whitespace
    while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    // Skip the preceding word (non-whitespace)
    while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    pos
}

/// Byte offset of the end of the word after cursor for word-forward-delete.
///
/// Skip leading whitespace, then skip the non-whitespace word.
pub fn word_delete_end(text: &str, cursor: usize) -> usize {
    let after = &text[cursor..];
    let bytes = after.as_bytes();
    let mut pos = 0;
    // Skip leading whitespace
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    // Skip the word
    while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    cursor + pos
}

// ─── DraftNotificationBatch ───────────────────────────────────────────────────

/// Batched adapter notification: holds the latest draft snapshot for coalesced
/// delivery to the owning adapter. Older notifications within the same batch
/// window are superseded.
///
/// Spec: §4.3 — "adapter MAY receive a single latest-draft snapshot rather than
/// per-keystroke events."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftNotificationBatch {
    /// Latest draft snapshot (coalescible — later snapshots replace earlier ones).
    pub latest: Option<DraftStateNotification>,
    /// Pending submission (transactional — NOT coalescible).
    pub submission: Option<DraftSubmission>,
    /// Pending cancel (transactional — NOT coalescible).
    pub cancel: Option<DraftCancel>,
}

impl DraftNotificationBatch {
    /// Empty batch.
    pub fn new() -> Self {
        Self {
            latest: None,
            submission: None,
            cancel: None,
        }
    }

    /// Coalesce a state-stream notification into the batch.
    /// Later snapshots replace earlier ones (latest-wins coalescing).
    pub fn coalesce_state(&mut self, notification: DraftStateNotification) {
        match &self.latest {
            Some(existing) if notification.sequence > existing.sequence => {
                self.latest = Some(notification);
            }
            None => {
                self.latest = Some(notification);
            }
            _ => {}
        }
    }

    /// Record a transactional submission (first wins; clears any pending cancel
    /// to enforce submit-XOR-cancel semantics).
    pub fn record_submission(&mut self, submission: DraftSubmission) {
        if self.submission.is_none() {
            self.cancel = None;
            self.submission = Some(submission);
        }
    }

    /// Record a transactional cancel (first wins; clears any pending submission
    /// to enforce submit-XOR-cancel semantics).
    pub fn record_cancel(&mut self, cancel: DraftCancel) {
        if self.cancel.is_none() {
            self.submission = None;
            self.cancel = Some(cancel);
        }
    }

    /// True if the batch contains anything to deliver.
    pub fn is_empty(&self) -> bool {
        self.latest.is_none() && self.submission.is_none() && self.cancel.is_none()
    }
}

impl Default for DraftNotificationBatch {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    // ─── Basic insert / backspace ─────────────────────────────────────────

    #[test]
    fn insert_and_backspace_round_trip() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        assert_eq!(draft.insert("hello"), EditOutcome::Mutated);
        assert_eq!(draft.text(), "hello");
        assert_eq!(draft.cursor(), 5);
        assert_eq!(draft.backspace(), EditOutcome::Mutated);
        assert_eq!(draft.text(), "hell");
        assert_eq!(draft.cursor(), 4);
    }

    #[test]
    fn backspace_at_start_is_unchanged() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        assert_eq!(draft.backspace(), EditOutcome::Unchanged);
    }

    #[test]
    fn insert_mid_text() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("ab");
        draft.move_left();
        draft.insert("X");
        assert_eq!(draft.text(), "aXb");
        assert_eq!(draft.cursor(), 2);
    }

    // ─── Newline / control rejection ──────────────────────────────────────

    #[test]
    fn insert_newline_rejected() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let outcome = draft.insert("\n");
        assert_eq!(outcome, EditOutcome::Unchanged);
        assert!(draft.text().is_empty());
    }

    #[test]
    fn insert_cr_rejected() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        assert_eq!(draft.insert("\r"), EditOutcome::Unchanged);
    }

    // ─── Word-wise delete ─────────────────────────────────────────────────

    /// Spec scenario 4.7: word-wise backspace removes the preceding word and
    /// the adapter observes only a draft-state notification, not per-keystroke
    /// traffic.
    #[test]
    fn word_backspace_removes_preceding_word() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("hello world");
        assert_eq!(draft.cursor(), 11);
        let seq_before = draft.sequence();
        let outcome = draft.word_backspace();
        assert_eq!(outcome, EditOutcome::Mutated);
        assert_eq!(draft.text(), "hello ");
        assert_eq!(draft.cursor(), 6);
        assert!(draft.sequence() > seq_before);
    }

    #[test]
    fn word_backspace_skips_trailing_whitespace_then_deletes_word() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("foo bar   ");
        let outcome = draft.word_backspace();
        assert_eq!(outcome, EditOutcome::Mutated);
        assert_eq!(draft.text(), "foo ");
    }

    #[test]
    fn word_backspace_at_start_is_unchanged() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        assert_eq!(draft.word_backspace(), EditOutcome::Unchanged);
    }

    #[test]
    fn word_delete_forward_removes_next_word() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("hello world");
        draft.move_to_start();
        let outcome = draft.word_delete_forward();
        assert_eq!(outcome, EditOutcome::Mutated);
        assert_eq!(draft.text(), " world");
    }

    // ─── Caret movement ───────────────────────────────────────────────────

    #[test]
    fn move_left_right_around_text() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("abc");
        assert_eq!(draft.move_left(), EditOutcome::Mutated);
        assert_eq!(draft.cursor(), 2);
        assert_eq!(draft.move_right(), EditOutcome::Mutated);
        assert_eq!(draft.cursor(), 3);
        assert_eq!(draft.move_right(), EditOutcome::Unchanged);
    }

    #[test]
    fn move_to_start_and_end() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("hello");
        assert_eq!(draft.move_to_start(), EditOutcome::Mutated);
        assert_eq!(draft.cursor(), 0);
        assert_eq!(draft.move_to_start(), EditOutcome::Unchanged);
        assert_eq!(draft.move_to_end(), EditOutcome::Mutated);
        assert_eq!(draft.cursor(), 5);
        assert_eq!(draft.move_to_end(), EditOutcome::Unchanged);
    }

    // ─── Selection ────────────────────────────────────────────────────────

    #[test]
    fn selection_left_right() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("abcde");
        draft.move_to_start();
        draft.select_right();
        draft.select_right();
        let sel = draft.selection();
        assert_eq!(sel.start, 0);
        assert_eq!(sel.end, 2);
        assert!(draft.has_selection());
    }

    #[test]
    fn move_collapses_selection_to_start() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("abcde");
        draft.move_to_start();
        draft.select_to_end();
        assert!(draft.has_selection());
        draft.move_left();
        assert!(!draft.has_selection());
        assert_eq!(draft.cursor(), 0);
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("abcde");
        draft.move_to_start();
        draft.select_right();
        draft.select_right();
        draft.select_right(); // select "abc"
        let outcome = draft.backspace();
        assert_eq!(outcome, EditOutcome::Mutated);
        assert_eq!(draft.text(), "de");
        assert_eq!(draft.cursor(), 0);
    }

    #[test]
    fn pointer_selection() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("hello");
        draft.set_pointer_selection(1, 4);
        let sel = draft.selection();
        assert_eq!(sel.start, 1);
        assert_eq!(sel.end, 4);
    }

    // ─── Paste and cap ────────────────────────────────────────────────────

    /// Spec §4.5 + §4.7: oversized paste truncates at UTF-8 boundary; at-capacity
    /// result returned; no overflow bytes in the notification or submission.
    #[test]
    fn paste_truncates_at_cap() {
        let cap = 10;
        let mut draft = ComposerDraft::new(cap);
        let big = "a".repeat(20);
        let outcome = draft.paste(&big);
        assert_eq!(outcome, EditOutcome::AtCapacity);
        assert_eq!(draft.text().len(), 10);
        assert!(draft.is_at_capacity());
        let snap = draft.snapshot();
        assert!(snap.at_capacity);
        assert_eq!(snap.text.len(), 10);
    }

    #[test]
    fn paste_utf8_boundary_preserved() {
        // "é" is 2 bytes; cap at 1 byte means the é gets dropped, not split.
        let mut draft = ComposerDraft::new(1);
        let outcome = draft.paste("é");
        assert_eq!(outcome, EditOutcome::AtCapacity);
        assert_eq!(draft.text(), "");
    }

    #[test]
    fn paste_within_cap_is_mutated() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        assert_eq!(draft.paste("hello"), EditOutcome::Mutated);
        assert_eq!(draft.text(), "hello");
    }

    /// Spec §4.7: cap violation never reaches the adapter (notification contains
    /// at most the capped draft content).
    #[test]
    fn oversized_paste_notification_does_not_exceed_cap() {
        let cap = 8;
        let mut draft = ComposerDraft::new(cap);
        draft.paste(&"x".repeat(100));
        let snap = draft.snapshot();
        assert!(snap.text.len() <= cap, "notification text exceeds cap");
    }

    #[test]
    fn insert_at_cap_returns_at_capacity() {
        let mut draft = ComposerDraft::new(5);
        draft.insert("hello");
        assert!(draft.is_at_capacity());
        let outcome = draft.insert("z");
        assert_eq!(outcome, EditOutcome::AtCapacity);
        assert_eq!(draft.text(), "hello");
    }

    // ─── Submit / Cancel ──────────────────────────────────────────────────

    /// Spec §4.7: submit-content fidelity — submitted text equals local buffer.
    #[test]
    fn submit_returns_exact_buffer_and_clears() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("send me");
        let sub = draft.submit().expect("submit should succeed");
        assert_eq!(sub.text, "send me");
        assert_eq!(draft.text(), "");
        assert_eq!(draft.cursor(), 0);
    }

    #[test]
    fn cancel_clears_without_submission() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("typed stuff");
        let cancel = draft.cancel().expect("cancel succeeds");
        let _ = cancel;
        assert_eq!(draft.text(), "");
    }

    // ─── Governance: safe-mode suspension ────────────────────────────────

    /// Spec §4.5 + §4.7: draft suspends under safe mode; insert is rejected.
    #[test]
    fn suspended_draft_rejects_insert() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.set_suspended(true);
        let outcome = draft.insert("x");
        assert_eq!(outcome, EditOutcome::Suspended);
        assert_eq!(draft.text(), "");
    }

    #[test]
    fn suspended_draft_rejects_paste() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.set_suspended(true);
        assert_eq!(draft.paste("hello"), EditOutcome::Suspended);
    }

    #[test]
    fn suspended_draft_rejects_backspace() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("hello");
        draft.set_suspended(true);
        assert_eq!(draft.backspace(), EditOutcome::Suspended);
        assert_eq!(draft.text(), "hello"); // text preserved
    }

    #[test]
    fn suspended_submit_returns_none() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("pending text");
        draft.set_suspended(true);
        assert!(draft.submit().is_none());
        assert_eq!(draft.text(), "pending text"); // not cleared
    }

    /// Safe-mode: content preserved while suspended, editable again after lift.
    #[test]
    fn draft_resumes_after_unsuspend() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("partial");
        draft.set_suspended(true);
        assert_eq!(draft.insert("x"), EditOutcome::Suspended);
        draft.set_suspended(false);
        assert_eq!(draft.insert("!"), EditOutcome::Mutated);
        assert_eq!(draft.text(), "partial!");
    }

    // ─── Keystroke non-passthrough ────────────────────────────────────────

    /// Spec §4.7: editing keystrokes are never terminal input.
    /// This is enforced by design — the draft buffer never routes to
    /// a terminal / provider byte stream. The test confirms that after
    /// editing operations, the draft contains only the locally rendered
    /// content and nothing has been "forwarded."
    ///
    /// Insert operations reject any string containing control characters
    /// (including `\r`/`\n`). The caller (input pipeline) receives individual
    /// CharacterEvents that do not include line endings — those arrive as
    /// KeyDown(Enter) and are handled as submit, never as character insert.
    #[test]
    fn editing_keystrokes_only_affect_local_draft() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        // Characters arrive as individual CharacterEvents from the runtime;
        // control characters are rejected at the insert boundary.
        draft.insert("l");
        draft.insert("s");
        draft.insert(" ");
        draft.insert("-");
        draft.insert("l");
        draft.insert("a");
        assert_eq!(draft.text(), "ls -la");
        // No external queue is modified — callers receive only draft state
        let snap = draft.snapshot();
        assert_eq!(snap.text, "ls -la");

        // Ensure control chars are rejected
        let newline_outcome = draft.insert("\n");
        assert_eq!(newline_outcome, EditOutcome::Unchanged);
        assert_eq!(draft.text(), "ls -la");
    }

    // ─── Notification coalescing ──────────────────────────────────────────

    /// Spec §4.7: adapter may receive a single latest-draft snapshot rather than
    /// per-keystroke events.
    #[test]
    fn notification_batch_coalesces_to_latest_snapshot() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let mut batch = DraftNotificationBatch::new();

        draft.insert("a");
        batch.coalesce_state(draft.snapshot());

        draft.insert("b");
        batch.coalesce_state(draft.snapshot());

        draft.insert("c");
        batch.coalesce_state(draft.snapshot());

        // Batch holds only the latest
        let snap = batch.latest.as_ref().expect("batch has latest");
        assert_eq!(snap.text, "abc");
        assert_eq!(snap.sequence, draft.sequence());
    }

    #[test]
    fn submission_recorded_not_coalesced() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("final text");
        let mut batch = DraftNotificationBatch::new();
        let sub = draft.submit().expect("submit");
        batch.record_submission(sub);
        // A second submit (hypothetically) does not overwrite
        let dummy = DraftSubmission {
            text: "other".to_string(),
            sequence: 99,
        };
        batch.record_submission(dummy);
        assert_eq!(batch.submission.as_ref().unwrap().text, "final text");
    }

    #[test]
    fn cancel_in_batch_is_transactional() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.insert("something");
        let mut batch = DraftNotificationBatch::new();
        let cancel = draft.cancel().expect("cancel");
        batch.record_cancel(cancel);
        assert!(batch.cancel.is_some());
    }

    // ─── Sequence monotonicity ────────────────────────────────────────────

    #[test]
    fn sequence_increments_on_each_mutation() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let s0 = draft.sequence();
        draft.insert("a");
        let s1 = draft.sequence();
        draft.insert("b");
        let s2 = draft.sequence();
        assert!(s1 > s0);
        assert!(s2 > s1);
    }

    #[test]
    fn unchanged_operations_do_not_increment_sequence() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let s0 = draft.sequence();
        draft.backspace(); // no-op at start
        assert_eq!(draft.sequence(), s0);
        draft.move_left(); // no-op at start
        assert_eq!(draft.sequence(), s0);
    }

    // ─── Unicode correctness (property-based) ────────────────────────────

    proptest! {
        /// Invariant: text.len() always ≤ cap.
        #[test]
        fn prop_text_never_exceeds_cap(
            cap in 1usize..=256,
            ops in proptest::collection::vec(0u8..10, 0..50),
        ) {
            let mut draft = ComposerDraft::new(cap);
            for op in ops {
                match op % 6 {
                    0 => { let _ = draft.insert("abc"); }
                    1 => { let _ = draft.paste("hello world"); }
                    2 => { let _ = draft.backspace(); }
                    3 => { let _ = draft.delete_forward(); }
                    4 => { let _ = draft.word_backspace(); }
                    _ => { let _ = draft.move_left(); }
                }
                prop_assert!(draft.text().len() <= cap,
                    "text.len()={} > cap={}", draft.text().len(), cap);
            }
        }

        /// Invariant: cursor is always a valid UTF-8 boundary.
        #[test]
        fn prop_cursor_is_valid_utf8_boundary(
            text in ".*",
            cursor_ops in proptest::collection::vec(0u8..8, 0..20),
        ) {
            let mut draft = ComposerDraft::new(MAX_DRAFT_BYTES);
            let _ = draft.paste(&text);
            for op in cursor_ops {
                match op % 8 {
                    0 => { let _ = draft.move_left(); }
                    1 => { let _ = draft.move_right(); }
                    2 => { let _ = draft.move_to_start(); }
                    3 => { let _ = draft.move_to_end(); }
                    4 => { let _ = draft.move_word_left(); }
                    5 => { let _ = draft.move_word_right(); }
                    6 => { let _ = draft.backspace(); }
                    _ => { let _ = draft.delete_forward(); }
                }
                let pos = draft.cursor();
                prop_assert!(
                    draft.text().is_char_boundary(pos),
                    "cursor={} is not a char boundary in {:?}",
                    pos, draft.text()
                );
            }
        }

        /// Invariant: submitted text always equals the local buffer at submit time.
        #[test]
        fn prop_submitted_text_equals_local_buffer(
            chars in proptest::collection::vec("[a-z ]", 0..30),
        ) {
            let mut draft = ComposerDraft::new(MAX_DRAFT_BYTES);
            for ch in &chars {
                let _ = draft.insert(ch);
            }
            let expected = draft.text().to_string();
            if let Some(sub) = draft.submit() {
                prop_assert_eq!(sub.text, expected,
                    "submitted text must equal buffer before submit");
            }
        }
    }
}
