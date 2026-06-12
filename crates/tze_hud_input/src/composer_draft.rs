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
//! and insert that would exceed the cap are truncated at a grapheme-cluster
//! boundary; `EditOutcome::AtCapacity` is returned and no notification leaves
//! the runtime with content exceeding the cap.

use tze_hud_scene::SceneId;
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

/// Truncate `s` to at most `max_bytes`, cutting on a **grapheme-cluster boundary**.
///
/// # Why grapheme clusters and not char boundaries
///
/// Truncating at a bare char boundary can split multi-codepoint grapheme clusters
/// (e.g. NFD `"e\u{0301}"`, emoji ZWJ sequences, or skin-tone modifiers). The
/// result is a string that is valid UTF-8 but visually incomplete / semantically
/// wrong. Since the composer cap guards *visible* content, we must truncate so that
/// the last retained grapheme is whole.
///
/// Algorithm: iterate grapheme clusters from the front, accumulate byte lengths,
/// stop when the next cluster would push us past `max_bytes`. Returns the largest
/// grapheme-aligned prefix that fits within `max_bytes`.
///
/// Spec: §4.5 — paste truncated at a UTF-8 grapheme-cluster boundary at the cap.
pub fn truncate_at_utf8_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk grapheme clusters and collect the largest prefix ≤ max_bytes.
    let mut end = 0usize;
    for cluster in s.graphemes(true) {
        let next = end + cluster.len();
        if next > max_bytes {
            break;
        }
        end = next;
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

// ─── DraftScheduler ──────────────────────────────────────────────────────────

/// Coalesced-delivery scheduler with a **flush guarantee**.
///
/// # Spec (§4.3)
///
/// Draft-state notifications are **state-stream** (coalescible). However,
/// the spec requires that the settled/terminal draft state is *always*
/// delivered to the adapter regardless of coalescing. This struct enforces
/// that contract:
///
/// - `push_notification` coalesces rapid state-stream notifications into a
///   single latest-snapshot pending slot.
/// - `flush` must be called on idle/settle, on blur, on submit, or on cancel
///   to guarantee the terminal state reaches the adapter.
/// - `take_batch` drains everything accumulated since the last call.
///
/// # Flush guarantee for submit (§4.3 / §4.e)
///
/// On `flush_submit`, the scheduler additionally enqueues a **post-submit
/// clear notification** (`text=""`, `cursor=0`) with an incremented sequence
/// number after the submission. This ensures the adapter view resets after
/// submit without relying on the next keystroke to update the display.
///
/// # Usage
///
/// ```ignore
/// // Per-keystroke (Stage 1):
/// scheduler.push_notification(draft.snapshot());
///
/// // On idle, blur, or any settle point:
/// scheduler.flush();
///
/// // On submit (produces DraftSubmission + clear notification):
/// if let Some(sub) = draft.submit() {
///     scheduler.flush_submit(sub);
/// }
///
/// // Per-frame delivery loop:
/// if let Some(batch) = scheduler.take_batch() {
///     deliver_to_adapter(batch);
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct DraftScheduler {
    /// Coalesced latest-snapshot slot (state-stream).
    pending_notification: Option<DraftStateNotification>,
    /// Pending transactional submission.
    pending_submission: Option<DraftSubmission>,
    /// Pending transactional cancel.
    pending_cancel: Option<DraftCancel>,
    /// True when `flush` has been requested and the pending notification (if any)
    /// should be delivered immediately on the next `take_batch` call.
    flush_pending: bool,
}

impl DraftScheduler {
    /// Create a new scheduler with no pending state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Coalesce a new draft-state notification into the pending slot.
    ///
    /// Later snapshots replace earlier ones (latest-wins, state-stream semantics).
    /// The notification is NOT delivered until `flush` is called or a transactional
    /// event forces delivery.
    pub fn push_notification(&mut self, notification: DraftStateNotification) {
        match &self.pending_notification {
            Some(existing) if notification.sequence <= existing.sequence => {}
            _ => {
                self.pending_notification = Some(notification);
            }
        }
    }

    /// Request a flush: the pending notification will be included in the next
    /// `take_batch` even if it was coalesced.
    ///
    /// Call this on idle/settle, on blur, and on any natural batch boundary.
    /// The scheduler guarantees the terminal draft state is delivered whenever
    /// `flush` is called — even if it was the only notification in the window.
    pub fn flush(&mut self) {
        self.flush_pending = true;
    }

    /// Record a transactional submit and schedule the post-submit display clear.
    ///
    /// This sets `flush_pending = true` so the clear notification is picked up
    /// by the next `take_batch`. The clear uses `sequence + 1` to ensure adapters
    /// that check sequence ordering treat it as newer than the submission.
    ///
    /// # Post-submit clear (spec §4.e / hud-qwqxy)
    ///
    /// `submit()` on `ComposerDraft` clears the buffer locally, but no
    /// `DraftStateNotification` with `text=""` is emitted automatically.
    /// Without this clear, adapters retain the submitted text in their display
    /// until the next keystroke. `flush_submit` emits that clear.
    pub fn flush_submit(&mut self, submission: DraftSubmission) {
        // Post-submit display clear: empty text, cursor at 0, sequence after submission.
        let clear_sequence = submission.sequence.wrapping_add(1);
        let clear = DraftStateNotification {
            text: String::new(),
            cursor: 0,
            selection_anchor: 0,
            at_capacity: false,
            sequence: clear_sequence,
        };
        // Record the transactional submission first.
        if self.pending_submission.is_none() {
            self.pending_cancel = None;
            self.pending_submission = Some(submission);
        }
        // Install the clear as the pending notification so it is delivered in
        // the same batch as the submission. The clear's sequence is strictly
        // greater than the submission's, allowing adapters that check sequence
        // ordering to treat it as newer regardless of batch processing order.
        self.pending_notification = Some(clear);
        self.flush_pending = true;
    }

    /// Record a transactional cancel.
    ///
    /// Forces a flush so the cancel is delivered. Any coalesced state-stream
    /// notification pending before the cancel is discarded: it represents an
    /// intermediate state that is now superseded by the cancel itself, and
    /// delivering it alongside the cancel would produce a contradictory pair
    /// (the display shows the pre-cancel text, then immediately a cancel event).
    pub fn flush_cancel(&mut self, cancel: DraftCancel) {
        if self.pending_cancel.is_none() {
            self.pending_submission = None;
            self.pending_cancel = Some(cancel);
        }
        self.pending_notification = None;
        self.flush_pending = true;
    }

    /// Drain the pending batch if there is anything to deliver.
    ///
    /// Returns `Some(batch)` when:
    /// - A transactional event (submission / cancel) is pending, OR
    /// - `flush` has been called and a notification is pending.
    ///
    /// Returns `None` when there is nothing to deliver (rapid mid-typing coalescing).
    ///
    /// Callers should call this once per frame (or once per settle point) and
    /// forward the batch to the owning adapter.
    pub fn take_batch(&mut self) -> Option<DraftNotificationBatch> {
        let has_transactional = self.pending_submission.is_some() || self.pending_cancel.is_some();
        let has_deliverable_notification =
            self.pending_notification.is_some() && self.flush_pending;

        if !has_transactional && !has_deliverable_notification {
            // If flush was requested but there is nothing to deliver (e.g.
            // `on_focus_lost` with no edits since last drain), clear the sticky
            // flush flag so it does not prematurely collapse the next coalescing
            // window after new edits arrive.
            if self.flush_pending {
                self.flush_pending = false;
            }
            return None;
        }

        let mut batch = DraftNotificationBatch::new();

        // State-stream notification (only on flush or when forced by transactional).
        if self.flush_pending {
            if let Some(n) = self.pending_notification.take() {
                batch.coalesce_state(n);
            }
            self.flush_pending = false;
        }

        // Transactional cancel.
        if let Some(cancel) = self.pending_cancel.take() {
            batch.record_cancel(cancel);
        }

        // Transactional submission.
        if let Some(sub) = self.pending_submission.take() {
            batch.record_submission(sub);
        }

        if batch.is_empty() { None } else { Some(batch) }
    }

    /// True when there is any pending state that has not been drained.
    pub fn has_pending(&self) -> bool {
        self.pending_notification.is_some()
            || self.pending_submission.is_some()
            || self.pending_cancel.is_some()
    }
}

// ─── ComposerDraftManager ─────────────────────────────────────────────────────

/// Per-tab runtime manager for `ComposerDraft` buffers.
///
/// One `ComposerDraftManager` per tab (or per runtime). Owns the `ComposerDraft`
/// for the currently focused composer region (at most one per tab at a time).
///
/// # Keystroke routing
///
/// When a focused node has `accepts_composer_input = true`, the runtime passes
/// keystroke events through `route_key_down` and `route_character` instead of
/// forwarding them to the agent as raw `KeyDownEvent` / `CharacterEvent`. The
/// draft buffer is mutated locally; the adapter receives coalesced
/// `DraftStateNotification` state-stream events (not per-keystroke events).
///
/// # Focus lifecycle
///
/// - `on_focus_gained` — creates or reuses a draft buffer for the focused node.
/// - `on_focus_lost` — flushes pending state and destroys the buffer.
/// - `on_submit` — submits the draft, schedules post-submit clear.
/// - `on_cancel` — cancels the draft, schedules cancel event.
///
/// # Safe-mode governance (§4.5)
///
/// Call `set_suspended(true)` when safe mode activates. The draft rejects all
/// mutating operations while suspended but preserves its content.
///
/// # Spec refs
///
/// - §4.1: runtime-owned draft attached to focused composer regions.
/// - §4.3: coalesced state-stream notifications + transactional submit/cancel.
/// - §4.5: governance — draft suspends under safe mode.
#[derive(Debug, Default)]
pub struct ComposerDraftManager {
    /// Active draft buffer, present while a composer region is focused.
    draft: Option<ComposerDraft>,
    /// Coalesced delivery scheduler.
    scheduler: DraftScheduler,
    /// The node_id of the currently focused composer region, if any.
    focused_node: Option<SceneId>,
}

impl ComposerDraftManager {
    /// Create a new manager with no active draft.
    pub fn new() -> Self {
        Self::default()
    }

    /// Called when a node with `accepts_composer_input = true` gains focus.
    ///
    /// Creates a new `ComposerDraft` (with `DEFAULT_DRAFT_CAP`) for the region.
    /// Any previous draft from a stale focus is discarded (focus is exclusive).
    /// The scheduler is also reset to ensure no pending state from the previous
    /// node leaks into the new focus window (state hygiene).
    pub fn on_focus_gained(&mut self, node_id: SceneId, suspended: bool) {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        draft.set_suspended(suspended);
        self.draft = Some(draft);
        self.focused_node = Some(node_id);
        self.scheduler = DraftScheduler::new();
    }

    /// Called when the focused composer region loses focus.
    ///
    /// Flushes any pending notification (blur is a settle point per §4.3 flush
    /// guarantee) then discards the draft buffer.
    ///
    /// Returns the drained batch, if any (caller delivers to adapter).
    pub fn on_focus_lost(&mut self) -> Option<DraftNotificationBatch> {
        self.scheduler.flush();
        let batch = self.scheduler.take_batch();
        self.draft = None;
        self.focused_node = None;
        batch
    }

    /// Route a character event into the active draft buffer.
    ///
    /// Returns `(EditOutcome, Option<DraftNotificationBatch>)` where the batch is
    /// `Some` only when a flush point is reached (transactional event or explicit
    /// settle — callers should call `try_flush` at frame end for the normal
    /// coalesced delivery path).
    ///
    /// If no composer is focused, returns `(EditOutcome::Unchanged, None)`.
    ///
    /// # Paste / multiline text (§4.5)
    ///
    /// Text that contains CR, LF, or other control characters (e.g. clipboard
    /// paste via Ctrl+V) is routed through `draft.paste()` after stripping
    /// newlines and control characters, rather than `draft.insert()`.
    /// `insert()` returns `Unchanged` for such text, which would cause
    /// `dispatch_character_event` to fall through and forward the raw text to
    /// the agent stream — violating the invariant that editing keystrokes are
    /// never terminal input while a composer is focused (spec §4.4).
    ///
    /// The sanitised text has all `\r`, `\n`, and other Unicode control
    /// characters removed before insertion so the single-line draft constraint
    /// (spec §4.4: no newlines in draft) is preserved.
    pub fn route_character(
        &mut self,
        character: &str,
    ) -> (EditOutcome, Option<DraftNotificationBatch>) {
        let Some(draft) = self.draft.as_mut() else {
            return (EditOutcome::Unchanged, None);
        };
        // Route multiline/control text through paste() after sanitisation so
        // it is consumed by the composer rather than falling through to the
        // agent's raw character stream (§4.4 / hud-083az).
        let outcome = if character
            .chars()
            .any(|c| c == '\r' || c == '\n' || c.is_control())
        {
            let sanitised: String = character
                .chars()
                .filter(|c| *c != '\r' && *c != '\n' && !c.is_control())
                .collect();
            draft.paste(&sanitised)
        } else {
            draft.insert(character)
        };
        if matches!(outcome, EditOutcome::Mutated | EditOutcome::AtCapacity) {
            self.scheduler.push_notification(draft.snapshot());
        }
        (outcome, None)
    }

    /// Route a pointer-down event to set the cursor position in the active
    /// draft buffer.
    ///
    /// `anchor` and `cursor` are byte offsets into the draft text; callers are
    /// responsible for computing them from the pointer position (e.g.
    /// proportional to local_x / node width, scaled by `draft.text().len()`).
    /// Both values are clamped and snapped to UTF-8 boundaries inside
    /// `ComposerDraft::set_pointer_selection`.
    ///
    /// Returns `EditOutcome::Unchanged` when no composer is focused.
    ///
    /// Spec: §4.1 — pointer selection into the runtime-owned draft buffer
    /// (hud-083az).
    pub fn route_pointer_selection(&mut self, anchor: usize, cursor: usize) -> EditOutcome {
        let Some(draft) = self.draft.as_mut() else {
            return EditOutcome::Unchanged;
        };
        let outcome = draft.set_pointer_selection(anchor, cursor);
        if outcome == EditOutcome::Mutated {
            self.scheduler.push_notification(draft.snapshot());
        }
        outcome
    }

    /// Route a key-down event into the active draft buffer.
    ///
    /// Interprets:
    /// - `Backspace` → `draft.backspace()`
    /// - `Delete` → `draft.delete_forward()`
    /// - `Ctrl+Backspace` / `Alt+Backspace` → `draft.word_backspace()`
    /// - `Ctrl+Delete` / `Alt+Delete` → `draft.word_delete_forward()`
    /// - `ArrowLeft` → `draft.move_left()` (or `select_left` with Shift)
    /// - `ArrowRight` → `draft.move_right()` (or `select_right` with Shift)
    /// - `Home` → `draft.move_to_start()` (or `select_to_start` with Shift)
    /// - `End` → `draft.move_to_end()` (or `select_to_end` with Shift)
    /// - `Ctrl+ArrowLeft` → `draft.move_word_left()`
    /// - `Ctrl+ArrowRight` → `draft.move_word_right()`
    /// - `Enter` / `NumpadEnter` → submit (returns batch with submission + clear)
    /// - `Escape` → cancel (returns batch with cancel)
    ///
    /// Returns `(consumed, Option<DraftNotificationBatch>)`.
    /// `consumed = true` means the keystroke was handled by the draft (do NOT
    /// forward to the agent as a raw `KeyDownEvent`).
    pub fn route_key_down(
        &mut self,
        key_code: &str,
        _key: &str,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> (bool, Option<DraftNotificationBatch>) {
        let Some(draft) = self.draft.as_mut() else {
            return (false, None);
        };

        match key_code {
            "Backspace" => {
                let o = if ctrl || alt {
                    draft.word_backspace()
                } else {
                    draft.backspace()
                };
                if matches!(o, EditOutcome::Mutated | EditOutcome::AtCapacity) {
                    self.scheduler.push_notification(draft.snapshot());
                }
                (true, None)
            }
            "Delete" => {
                let o = if ctrl || alt {
                    draft.word_delete_forward()
                } else {
                    draft.delete_forward()
                };
                if matches!(o, EditOutcome::Mutated | EditOutcome::AtCapacity) {
                    self.scheduler.push_notification(draft.snapshot());
                }
                (true, None)
            }
            "ArrowLeft" => {
                let o = if shift {
                    draft.select_left()
                } else if ctrl || alt {
                    draft.move_word_left()
                } else {
                    draft.move_left()
                };
                if o == EditOutcome::Mutated {
                    self.scheduler.push_notification(draft.snapshot());
                }
                (true, None)
            }
            "ArrowRight" => {
                let o = if shift {
                    draft.select_right()
                } else if ctrl || alt {
                    draft.move_word_right()
                } else {
                    draft.move_right()
                };
                if o == EditOutcome::Mutated {
                    self.scheduler.push_notification(draft.snapshot());
                }
                (true, None)
            }
            "Home" => {
                let o = if shift {
                    draft.select_to_start()
                } else {
                    draft.move_to_start()
                };
                if o == EditOutcome::Mutated {
                    self.scheduler.push_notification(draft.snapshot());
                }
                (true, None)
            }
            "End" => {
                let o = if shift {
                    draft.select_to_end()
                } else {
                    draft.move_to_end()
                };
                if o == EditOutcome::Mutated {
                    self.scheduler.push_notification(draft.snapshot());
                }
                (true, None)
            }
            "Enter" | "NumpadEnter" => {
                if let Some(sub) = draft.submit() {
                    self.scheduler.flush_submit(sub);
                    let b = self.scheduler.take_batch();
                    (true, b)
                } else {
                    // Empty draft — consume but nothing to deliver.
                    (true, None)
                }
            }
            "Escape" => {
                if let Some(cancel) = draft.cancel() {
                    self.scheduler.flush_cancel(cancel);
                    let b = self.scheduler.take_batch();
                    (true, b)
                } else {
                    (true, None)
                }
            }
            "KeyV" if ctrl => {
                // Ctrl+V (paste shortcut): consume the KeyDown so it is never
                // forwarded to the agent as a raw KeyDownEvent while a composer
                // is focused.  The actual paste content arrives separately via
                // `route_character` (dispatch_character_event reads the
                // clipboard and fires a RawCharacterEvent immediately after the
                // KeyDown — hud-083az).
                (true, None)
            }
            _ => {
                // Not a handled editing key; do not consume (let caller forward
                // to agent if needed).
                (false, None)
            }
        }
    }

    /// Flush pending coalesced notifications at a settle point (e.g. frame end).
    ///
    /// Must be called periodically to guarantee the terminal draft state is
    /// delivered. The `DraftScheduler` guarantees that at least one notification
    /// is delivered after any sequence of `push_notification` + `flush` calls.
    ///
    /// Callers should call this once per frame or once after a burst of keystrokes.
    pub fn try_flush(&mut self) -> Option<DraftNotificationBatch> {
        self.scheduler.flush();
        self.scheduler.take_batch()
    }

    /// Called when safe mode activates or deactivates.
    ///
    /// Propagates the suspension state to the active draft buffer.
    pub fn set_suspended(&mut self, suspended: bool) {
        if let Some(draft) = self.draft.as_mut() {
            draft.set_suspended(suspended);
        }
    }

    /// Returns a reference to the active draft buffer, if any.
    pub fn draft(&self) -> Option<&ComposerDraft> {
        self.draft.as_ref()
    }

    /// Returns the node_id of the currently focused composer region, if any.
    pub fn focused_node(&self) -> Option<SceneId> {
        self.focused_node
    }

    /// True when a composer region is currently focused (draft buffer present).
    pub fn is_active(&self) -> bool {
        self.draft.is_some()
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

    // ─── DraftScheduler: coalesced delivery + flush guarantee ────────────────

    /// Spec §4.c: rapid coalesced edits must still deliver the final state.
    ///
    /// If 100 keystrokes arrive in one scheduler window, the adapter sees one
    /// batch containing the terminal snapshot (not 100 batches). The flush call
    /// on idle/settle guarantees the terminal state is delivered.
    #[test]
    fn scheduler_coalesces_rapid_edits_but_guarantees_final_delivery() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let mut sched = DraftScheduler::new();

        // Simulate 10 rapid keystrokes coalesced into one scheduler window.
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l"] {
            let _ = draft.insert(ch);
            sched.push_notification(draft.snapshot());
        }

        // No flush yet — mid-window, nothing should be delivered.
        // (take_batch returns None when flush has not been requested)
        assert!(sched.take_batch().is_none(), "no delivery before flush");

        // Flush (settle/idle point) — terminal state must be delivered.
        sched.flush();
        let batch = sched
            .take_batch()
            .expect("terminal state must be delivered after flush");

        let snap = batch
            .latest
            .as_ref()
            .expect("batch must have a state notification");
        assert_eq!(
            snap.text, "hello worl",
            "delivered snapshot must equal the terminal draft state"
        );
        // Verify coalescing: the delivered sequence matches the draft's current sequence.
        assert_eq!(
            snap.sequence,
            draft.sequence(),
            "sequence must match draft sequence"
        );
    }

    /// After `flush` + `take_batch`, subsequent `take_batch` returns None (no double-delivery).
    #[test]
    fn scheduler_does_not_double_deliver_after_flush() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let mut sched = DraftScheduler::new();

        draft.insert("hello");
        sched.push_notification(draft.snapshot());
        sched.flush();

        let first = sched.take_batch();
        assert!(first.is_some(), "first take_batch returns the batch");

        let second = sched.take_batch();
        assert!(
            second.is_none(),
            "second take_batch after drain returns None"
        );
    }

    /// Rapid edit → flush → more edits: each settle point gets exactly the
    /// state at that point, not stale state from a previous window.
    #[test]
    fn scheduler_delivers_correct_state_across_multiple_flush_windows() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let mut sched = DraftScheduler::new();

        draft.insert("hello");
        sched.push_notification(draft.snapshot());
        sched.flush();
        let batch1 = sched.take_batch().expect("first flush delivers");
        assert_eq!(batch1.latest.as_ref().unwrap().text, "hello");

        draft.insert(" world");
        sched.push_notification(draft.snapshot());
        sched.flush();
        let batch2 = sched.take_batch().expect("second flush delivers");
        assert_eq!(batch2.latest.as_ref().unwrap().text, "hello world");
    }

    // ─── Post-submit display clear (spec §4.e) ───────────────────────────────

    /// Spec §4.e: `flush_submit` emits an `UpdateComposerDisplay('')` clear so
    /// the adapter view resets after submit without relying on the next keystroke.
    ///
    /// The clear notification must have `text=""`, `cursor=0`, and a sequence
    /// number strictly greater than the submission sequence so adapters that
    /// check ordering treat it as newer.
    #[test]
    fn flush_submit_emits_post_submit_display_clear() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let mut sched = DraftScheduler::new();

        draft.insert("send this");
        let sub = draft.submit().expect("submit should succeed");
        let sub_sequence = sub.sequence;

        sched.flush_submit(sub);
        let batch = sched
            .take_batch()
            .expect("batch must be non-empty after flush_submit");

        // The batch must contain the transactional submission.
        let submission = batch
            .submission
            .as_ref()
            .expect("submission must be in batch");
        assert_eq!(
            submission.text, "send this",
            "submission text must be exact buffer"
        );

        // The batch must also contain the post-submit clear notification.
        let clear = batch
            .latest
            .as_ref()
            .expect("clear notification must be in batch");
        assert!(
            clear.text.is_empty(),
            "post-submit clear notification must have empty text"
        );
        assert_eq!(clear.cursor, 0, "post-submit clear cursor must be 0");
        assert!(
            clear.sequence > sub_sequence,
            "clear sequence ({}) must be > submission sequence ({})",
            clear.sequence,
            sub_sequence,
        );
    }

    /// Submit with no prior draft-state notification also produces the clear.
    #[test]
    fn flush_submit_produces_clear_even_with_no_prior_notifications() {
        let mut draft = ComposerDraft::new(DEFAULT_DRAFT_CAP);
        let mut sched = DraftScheduler::new();

        draft.insert("x");
        let sub = draft.submit().expect("submit");

        // No push_notification before flush_submit.
        sched.flush_submit(sub);
        let batch = sched.take_batch().expect("batch after flush_submit");
        assert!(batch.submission.is_some(), "submission present");
        let clear = batch.latest.as_ref().expect("clear notification present");
        assert!(clear.text.is_empty(), "post-submit clear is empty");
    }

    // ─── Grapheme-cluster cap boundary fix ──────────────────────────────────

    /// Truncation at the byte cap must preserve whole grapheme clusters.
    ///
    /// NFD `"e\u{0301}"` (LATIN SMALL LETTER E + COMBINING ACUTE ACCENT) is a
    /// 2-codepoint grapheme cluster (3 bytes: `0x65` + `0xcc` `0x81`). If the cap
    /// falls in the middle of the combining accent, a naive char-boundary
    /// truncation would produce `"e"` (dropping only the accent half) while a
    /// grapheme-boundary truncation correctly drops the entire cluster.
    ///
    /// This test places the cap at exactly 2 bytes — the `e` (1 byte) and the
    /// first byte of the combining accent — to verify the cluster is not split.
    #[test]
    fn truncate_at_grapheme_boundary_nfd_combining_accent() {
        // "e\u{0301}" is U+0065 (1 byte) + U+0301 (2 bytes) = 3 bytes total.
        // The grapheme cluster boundary falls at 0 or 3; 2 is in the middle.
        let s = "e\u{0301}"; // NFD: e + combining acute accent
        assert_eq!(s.len(), 3);

        // Cap = 2: cannot fit the full cluster (3 bytes). Must return "".
        let truncated = truncate_at_utf8_boundary(s, 2);
        assert!(
            truncated.is_empty(),
            "a 3-byte grapheme cluster must not be split at byte 2; got {truncated:?}",
        );

        // Cap = 3: fits exactly.
        let full = truncate_at_utf8_boundary(s, 3);
        assert_eq!(full, s);

        // Cap = 0: always empty.
        let empty = truncate_at_utf8_boundary(s, 0);
        assert_eq!(empty, "");
    }

    /// Insert with a cap that falls inside a multi-codepoint grapheme cluster
    /// does not split the cluster (uses `truncate_at_utf8_boundary` internally).
    #[test]
    fn insert_does_not_split_grapheme_cluster_at_cap() {
        // NFD "e\u{0301}" = 3 bytes. Cap = 2 → the cluster does not fit.
        let mut draft = ComposerDraft::new(2);
        let outcome = draft.insert("e\u{0301}");
        assert_eq!(
            outcome,
            EditOutcome::AtCapacity,
            "inserting a cluster that exceeds the cap must return AtCapacity"
        );
        assert_eq!(
            draft.text(),
            "",
            "the cluster must not be partially inserted"
        );
    }

    /// Paste with a ZWJ emoji sequence at the cap boundary must not split it.
    ///
    /// A ZWJ sequence like "👨\u{200D}👩" is a 3-element grapheme cluster
    /// (4 + 3 + 4 = 11 bytes).  Prepend 10 'a's (cap=15 → 5 bytes remaining for
    /// the cluster) to verify the whole cluster is either kept or dropped, never
    /// split.
    #[test]
    fn paste_does_not_split_zwj_emoji_at_cap() {
        // ZWJ sequence: 👨 (U+1F468, 4 bytes) + ZWJ (U+200D, 3 bytes) + 👩 (U+1F469, 4 bytes)
        // = 11 bytes as one grapheme cluster.
        let zwj = "\u{1F468}\u{200D}\u{1F469}";
        assert_eq!(zwj.len(), 11);

        // Cap = 14 bytes: 10 'a's (10 bytes) + 4 remaining → not enough for the cluster (11 bytes).
        let cap = 14;
        let mut draft = ComposerDraft::new(cap);
        draft.paste(&"a".repeat(10));
        assert_eq!(draft.text().len(), 10);

        // Paste the ZWJ sequence. Only 4 bytes remain; cluster needs 11.
        // The cluster must be dropped entirely (not split).
        let outcome = draft.paste(zwj);
        assert_eq!(
            outcome,
            EditOutcome::AtCapacity,
            "ZWJ cluster exceeding cap must return AtCapacity"
        );
        // Text must contain only the 10 'a's — no partial emoji bytes.
        assert_eq!(
            draft.text(),
            "aaaaaaaaaa",
            "draft must not contain a partial ZWJ cluster; got {:?}",
            draft.text()
        );
    }

    // ─── ComposerDraftManager: keystroke routing ─────────────────────────────

    /// When a composer is focused, character events are routed into the draft.
    #[test]
    fn manager_routes_character_to_draft() {
        let mut mgr = ComposerDraftManager::new();
        let node_id = tze_hud_scene::SceneId::new();
        mgr.on_focus_gained(node_id, false);

        let (outcome, batch) = mgr.route_character("h");
        assert_eq!(outcome, EditOutcome::Mutated);
        assert!(
            batch.is_none(),
            "character routes to coalesced path, not immediate batch"
        );

        let (_, _) = mgr.route_character("i");
        let flush_batch = mgr.try_flush().expect("flush delivers accumulated state");
        let snap = flush_batch.latest.as_ref().unwrap();
        assert_eq!(snap.text, "hi", "draft must contain routed characters");
    }

    /// Enter key submits the draft and emits a clear notification.
    #[test]
    fn manager_enter_submits_and_clears() {
        let mut mgr = ComposerDraftManager::new();
        let node_id = tze_hud_scene::SceneId::new();
        mgr.on_focus_gained(node_id, false);

        mgr.route_character("h");
        mgr.route_character("i");

        let (consumed, batch) = mgr.route_key_down("Enter", "", false, false, false);
        assert!(consumed, "Enter must be consumed by the draft manager");
        let batch = batch.expect("Enter must emit a batch");

        let sub = batch
            .submission
            .as_ref()
            .expect("submission must be present");
        assert_eq!(sub.text, "hi");

        let clear = batch
            .latest
            .as_ref()
            .expect("post-submit clear must be present");
        assert!(
            clear.text.is_empty(),
            "clear notification must have empty text"
        );
        assert!(
            clear.sequence > sub.sequence,
            "clear must have higher sequence than submission"
        );
    }

    /// Backspace is consumed and routed to word_backspace when Ctrl is held.
    #[test]
    fn manager_ctrl_backspace_word_delete() {
        let mut mgr = ComposerDraftManager::new();
        let node_id = tze_hud_scene::SceneId::new();
        mgr.on_focus_gained(node_id, false);

        // Insert "hello world"
        for ch in "hello world".chars() {
            mgr.route_character(&ch.to_string());
        }

        // Ctrl+Backspace should delete "world"
        let (consumed, _) = mgr.route_key_down("Backspace", "", false, true, false);
        assert!(consumed, "Ctrl+Backspace must be consumed");

        let batch = mgr.try_flush().expect("flush after delete");
        let snap = batch.latest.as_ref().unwrap();
        assert_eq!(snap.text, "hello ", "word backspace must delete 'world'");
    }

    /// on_focus_lost flushes pending state.
    #[test]
    fn manager_focus_lost_flushes_pending_state() {
        let mut mgr = ComposerDraftManager::new();
        let node_id = tze_hud_scene::SceneId::new();
        mgr.on_focus_gained(node_id, false);

        mgr.route_character("x");
        // Do NOT call try_flush — simulate mid-window blur.
        let batch = mgr.on_focus_lost().expect("blur must flush pending state");
        let snap = batch.latest.as_ref().unwrap();
        assert_eq!(
            snap.text, "x",
            "blur flush must deliver pending draft state"
        );
        assert!(!mgr.is_active(), "draft must be cleared after focus lost");
    }

    /// Unknown key codes are not consumed (caller forwards to agent).
    #[test]
    fn manager_unknown_key_not_consumed() {
        let mut mgr = ComposerDraftManager::new();
        let node_id = tze_hud_scene::SceneId::new();
        mgr.on_focus_gained(node_id, false);

        let (consumed, batch) = mgr.route_key_down("F5", "", false, false, false);
        assert!(!consumed, "unknown key must not be consumed");
        assert!(batch.is_none());
    }

    /// Suspended draft rejects character routing but allows key navigation.
    #[test]
    fn manager_suspended_rejects_character() {
        let mut mgr = ComposerDraftManager::new();
        let node_id = tze_hud_scene::SceneId::new();
        mgr.on_focus_gained(node_id, true); // suspended = true

        let (outcome, _) = mgr.route_character("x");
        assert_eq!(outcome, EditOutcome::Suspended);
    }
}
