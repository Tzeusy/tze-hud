//! Resident gRPC adapter for cooperative projection portal materialization.
//!
//! This module is daemon-side glue: it turns bounded projection authority state
//! into `HudSession` messages for the existing raw-tile text-stream portal path.
//! It deliberately does not expose an LLM-facing CLI, MCP surface, provider RPC,
//! PTY, terminal byte stream, or process lifecycle authority.

use std::time::Instant;

use tze_hud_protocol::proto;
use tze_hud_protocol::proto::session as session_proto;

use thiserror::Error;

use crate::{
    AdapterDraftBatch, AdapterDraftNotification, ContentClassification, OutputKind,
    PortalInputFeedback, PortalInputFeedbackState, PortalInputSubmission,
    ProjectedPortalPresentation, ProjectedPortalState, ProjectionAuthority, ProjectionErrorCode,
    ProjectionLifecycleState, TranscriptUnit,
};

/// Content-free disconnect/stale marker line rendered when the portal's driving
/// stream/session is degraded (portal-disconnect-resume-ux §2). Carries no
/// identity or transcript content, so it remains present under redaction exactly
/// like the scroll-position indicator.
const PORTAL_DISCONNECT_MARKER_LINE: &str = "⊘ disconnected — stream stale";

/// First-run empty-portal body (hud-g1ena.6, portal-chat-grade-affordances
/// §First-Run Empty Portal Treatment): quiet, inviting copy shown when a
/// *connected* portal's retained transcript window is empty, replacing the
/// literal `<empty projection stream>`. Ambient by design — a presence engine's
/// empty surface reads as calm and ready, never as an error. The portal header
/// carries the identity; this line is the "inviting copy" that redaction
/// suppresses.
const PORTAL_EMPTY_READY_LINE: &str = "Ready — waiting for the first message.";

/// Content-free empty-portal placeholder for a viewer without identity clearance
/// (§First-Run Empty Portal Treatment redaction scenario): no identity, no
/// inviting copy — only a neutral, quiet mark, mirroring how the other
/// affordances go silent under redaction.
const PORTAL_EMPTY_REDACTED_LINE: &str = "· · ·";

/// Connecting-state body shown while a portal is attached but its owning session
/// has never connected (portal-chat-grade-affordances §Connecting State
/// Distinction, `has_ever_connected == false`). A starting-up portal must not
/// read as a "ready" empty state, so the first-run treatment yields to this.
///
/// Distinct from the degraded/disconnect marker on TWO axes so a starting-up
/// portal never reads as a failing one: (1) a distinct pending glyph — `◌`
/// (dotted, "not yet solid") vs the degraded `⊘` (circled slash) — that survives
/// environments which do not inspect `color_runs`, and (2) the token-resolved
/// cool `connecting_marker_color` (vs the amber `stale_marker_color`), applied
/// via `connecting_color_runs`. Content-free: it names no identity and reveals no
/// transcript, so — like the disconnect marker — it is redaction-independent and
/// takes precedence over the redacted empty placeholder.
///
/// hud-g1ena.6 established the precedence with a minimal `"Connecting…"` string;
/// hud-g1ena.7 replaces it with this distinct connecting treatment.
const PORTAL_CONNECTING_LINE: &str = "◌ Connecting… — waiting for the session to come online.";

/// In-transcript unread divider marker (hud-g1ena.2,
/// portal-chat-grade-affordances §Unread Divider and Ambient Unread Count).
///
/// A quiet boundary label rendered before the oldest retained unseen
/// agent-authored turn when the viewer returns to a portal with unread content.
/// Ambient, not alarming (no `!`/⚠) — a presence engine marks *where* unread
/// begins without behaving like a notification; the *how many* is carried
/// separately by the ambient unread count near the header. The token-resolved
/// color rides a zero-length sentinel (`unread_divider_color_runs`); precise
/// per-line coloring is deferred to the promotion-era structured layout (like
/// the other Phase-1 single-node markers).
const PORTAL_UNREAD_DIVIDER_LINE: &str = "─── unread ───";

/// Ambient header activity marker (portal-chat-grade-affordances §Agent Activity
/// and Streaming Cue, hud-g1ena.5) shown while the owning adapter is actively
/// appending to the transcript. A compact typing-style indicator — quiet ellipsis
/// glyph, no `!`/⚠ — so continuous streaming reads as ambient presence, never a
/// notification (doctrine: not a notification engine). The token-resolved
/// `activity_cue_color` accent rides alongside via `activity_cue_color_runs`.
const PORTAL_ACTIVITY_MARKER_LINE: &str = "⋯ writing";

/// Transcript-tail streaming cursor glyph appended to the latest agent turn while
/// content is actively appending. A slim left half-block reads as a live-writing
/// caret at the tail; the token-resolved `streaming_cursor_color` accent rides
/// alongside via `streaming_cursor_color_runs`. Ambient, not alarming.
const PORTAL_STREAMING_CURSOR_GLYPH: &str = " ▍";

/// Quiescence window (µs) for the agent-activity / streaming-cursor cue.
///
/// The cue derives from the tail transcript unit's runtime-assigned
/// `appended_at_wall_us` (unforgeable — adapters cannot set arrival times)
/// compared to the render's wall-clock now: the agent is "actively appending"
/// only while the newest append is within this window. This is what makes the
/// cue quiesce promptly once appends stop, and — critically for the single-node
/// Phase-1 model — it means a portal re-rendered for an UNRELATED reason (resize,
/// draft keystroke, collapse) long after the last append shows no cursor, because
/// `now - appended_at` exceeds the window. Kept short so streaming reads as live
/// yet the cue settles quickly when the agent goes quiet (ambient, not sticky).
///
/// Phase-1 limitation: the drive loop re-renders a portal only on its own append
/// traffic (there is no autonomous heartbeat tick), so a truly idle portal holds
/// the last-rendered cursor until the next operation touches it. Any subsequent
/// render past the window quiesces it. Promotion-era work adds precise tail
/// positioning and a heartbeat repaint.
const PORTAL_ACTIVITY_QUIESCE_WINDOW_US: u64 = 2_000_000;

/// Whether the portal is in a connection-degraded presentation state.
///
/// Keyed off the redaction-independent `connection_degraded` flag (set by the
/// authority from the session lifecycle), NOT the redaction-gated
/// `lifecycle_state`, so the degraded treatment is applied even for a restricted
/// viewer.
fn is_connection_degraded(state: &ProjectedPortalState) -> bool {
    state.connection_degraded
}

/// Whether the owning adapter is actively appending to the transcript right now
/// (portal-chat-grade-affordances §Agent Activity and Streaming Cue, hud-g1ena.5).
///
/// DERIVED entirely from observed append activity — the newest visible transcript
/// unit's runtime-assigned `appended_at_wall_us` compared to the render's
/// wall-clock `now_wall_us` — never from a separate adapter "typing" protocol or
/// the adapter-declared `lifecycle_state` (which is forgeable; appended-at is
/// not). The cue fires only while ALL hold:
///
/// - **Expanded** presentation — the cursor is a transcript-tail affordance and
///   the header cue rides the expanded header; collapsed keeps its compact card.
/// - **Live** — not `connection_degraded` and `has_ever_connected`; a degraded or
///   never-connected portal must never imply an active stream (§clear live
///   signals on disconnect; connecting takes precedence over activity).
/// - **Tail is a fresh agent turn** — the last unit is agent output (`!= Viewer`)
///   and NOT `expects_reply` (a question awaiting a reply is quiescing, not
///   writing, and already carries its own awaiting-reply cue), appended within
///   [`PORTAL_ACTIVITY_QUIESCE_WINDOW_US`] of now.
///
/// Redaction falls out for free: a restricted viewer's `visible_transcript` is
/// emptied upstream, so `last()` is `None` and the cue is suppressed along with
/// transcript previews (§activity cue suppressed under redaction). Because the
/// gate is a freshness window, continuous streaming keeps re-satisfying it
/// without any attention re-escalation, and it quiesces promptly once appends
/// stop (§activity cue is not a notification).
fn agent_activity_active(state: &ProjectedPortalState, now_wall_us: u64) -> bool {
    // The cue is active exactly while `now` has not passed the newest agent
    // tail's clear-due deadline. Factored through
    // `agent_activity_clear_deadline_us` so the drive loop can schedule a
    // one-shot quiesce repaint off the same predicate (hud-kbm80).
    agent_activity_clear_deadline_us(state).is_some_and(|deadline| now_wall_us <= deadline)
}

/// The wall-clock instant (µs since epoch) past which `state`'s agent-activity /
/// streaming-cursor cue quiesces, or `None` when the state carries no cue at all
/// (wrong presentation, degraded / never-connected, or a tail that is not a
/// fresh non-question agent turn).
///
/// This is the time-independent factor of [`agent_activity_active`]: it applies
/// every structural gate documented there (Expanded, Live, tail-is-a-fresh-
/// non-question-agent-turn) and returns that tail's deadline
/// (`appended_at_wall_us + PORTAL_ACTIVITY_QUIESCE_WINDOW_US`) WITHOUT comparing
/// it to a render `now`. `agent_activity_active(state, now)` is exactly
/// `deadline.is_some_and(|d| now <= d)`.
///
/// Exposed for the drive loop (hud-kbm80): the round-robin drain only re-renders
/// a portal on a fresh coalescer update, so after the terminal append on an
/// otherwise-idle portal nothing re-evaluates the cue and it would persist past
/// its window — misrepresenting ongoing activity (§Agent Activity and Streaming
/// Cue: the cue SHALL "quiesce promptly once appends stop"). The driver reads
/// this deadline to schedule a single force-repaint past it, at which point the
/// derivation above evaluates false and the cue clears.
pub fn agent_activity_clear_deadline_us(state: &ProjectedPortalState) -> Option<u64> {
    if state.presentation != ProjectedPortalPresentation::Expanded
        || is_connection_degraded(state)
        || !state.has_ever_connected
    {
        return None;
    }
    state.visible_transcript.last().and_then(|unit| {
        (unit.output_kind != OutputKind::Viewer && !unit.expects_reply).then(|| {
            unit.appended_at_wall_us
                .saturating_add(PORTAL_ACTIVITY_QUIESCE_WINDOW_US)
        })
    })
}

/// Client-side materialization budget for one resident portal update.
///
/// The adapter must be comfortably below one 60 Hz frame budget while building
/// the outbound gRPC payload. Server-side admission and compositor budgets are
/// measured by their existing validation lanes.
pub const RESIDENT_PORTAL_UPDATE_BUILD_BUDGET_US: u64 = 16_600;

/// Local-first budget for translating HUD composer text into the semantic inbox.
pub const RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US: u64 = 4_000;

const DEFAULT_EXPANDED_W: f32 = 720.0;
const DEFAULT_EXPANDED_H: f32 = 360.0;
const DEFAULT_COMPACT_W: f32 = 420.0;
const DEFAULT_COMPACT_H: f32 = 96.0;
const DEFAULT_Z_ORDER: u32 = 160;
const MAX_PORTAL_MARKDOWN_BYTES: usize = 16_384;

// ── PortalVisualTokens ────────────────────────────────────────────────────────

/// Resolved visual values for the portal surface parts consumed by the
/// raw-tile pilot, sourced from the runtime's resolved design token set.
///
/// **Pre-promotion rule (§6.1):** the exemplar adapter MUST source every
/// published visual value from this struct. No literal colors or font sizes
/// are permitted in the adapter publish path. A profile/token change must
/// reskin the portal end-to-end with zero adapter logic changes.
///
/// Build by first calling `tze_hud_config::resolve_portal_tokens` on the
/// runtime's resolved `DesignTokenMap` to get `PortalPartTokens`, then
/// converting with
/// `tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens`.
/// Pass the result to `ResidentGrpcPortalAdapter::with_tokens`.
///
/// ## Phase-1 scope limitation
///
/// The Phase-1 raw-tile pilot publishes a **single** `TextMarkdownNodeProto`,
/// which carries only `color`, `background`, and `font_size_px`. This struct
/// therefore contains only the fields that `portal_node` actually consumes
/// (transcript, collapsed, and composer parts). The full part inventory defined in
/// `PortalPartTokens` (frame, header, divider, transitions) requires
/// a structured multi-node layout (one node per surface part) that is deferred
/// to promotion-era work (see spec §7.5 and RFC 0013 §7.2 promotion gate).
///
/// If you need the full part inventory for promotion-era work, consume
/// `PortalPartTokens` directly from `tze_hud_config::resolve_portal_tokens`.
///
/// ## Redaction safety (§6.3)
///
/// Redaction is **structural**, not time-based: the `redacted` flag on
/// `ProjectedPortalState` is computed from viewer clearance vs. content
/// classification by the `ProjectionAuthority`, independently of any transition
/// animation position. A restricted viewer therefore sees `redacted = true` in
/// every frame — expanded, collapsed, and any intermediate transition state.
///
/// ## Part inventory (Phase-1 pilot — single TextMarkdownNodeProto)
///
/// | Part | Fields |
/// |------|--------|
/// | transcript body | `transcript_background`, `transcript_text_color`, `transcript_font_size_px` |
/// | collapsed card | `collapsed_background`, `collapsed_text_color`, `collapsed_font_size_px` |
/// | composer region | `composer_background`, `composer_text_color`, `composer_font_size_px`, `composer_at_capacity_color` |
///
/// Frame, header, divider, and transition fields are omitted because
/// `TextMarkdownNodeProto` has no slots for them. They are wired in
/// `PortalPartTokens` (in `tze_hud_config`) for promotion-era structured layout.
///
/// ## Composer rendering (§4.1 / §4.8 — local feedback first)
///
/// When a draft is active, `portal_node` renders the draft text with an inline
/// `▌` caret marker at the cursor byte offset. When `at_capacity == true`, the
/// composer line receives a text-visible `[!] ` prefix. The
/// `composer_at_capacity_color` token is carried in the `color_runs` field of
/// the `TextMarkdownNodeProto` as a zero-length Phase-1 sentinel (bytes `[0..0]`)
/// for machine-readable detection; it does **not** apply a visible color to the
/// text in Phase-1 (precise per-line coloring is deferred to hud-9gyao).
/// This is entirely local — the compositor reads from the adapter's draft
/// display state without any remote roundtrip.
#[derive(Clone, Debug, PartialEq)]
pub struct PortalVisualTokens {
    // Transcript body (expanded presentation)
    pub transcript_background: proto::Rgba,
    pub transcript_text_color: proto::Rgba,
    pub transcript_font_size_px: f32,

    // Degraded / disconnect treatment (portal-disconnect-resume-ux §2/§3).
    /// Dimmed transcript text shown while the portal is disconnected/stale.
    pub transcript_dim_text_color: proto::Rgba,
    /// Dimmed transcript background shown while the portal is disconnected/stale.
    pub transcript_dim_background: proto::Rgba,
    /// Color of the content-free stale/disconnect marker (ambient, not alarming).
    pub stale_marker_color: proto::Rgba,
    /// Color of the ambient unread-output-count indicator (hud-meqet). Muted
    /// by design — a presence engine surfaces a quiet count, never a loud
    /// notification badge. Source token: `portal.unread_indicator.color`.
    pub unread_indicator_color: proto::Rgba,
    /// Color of the in-transcript unread divider (hud-g1ena.2,
    /// portal-chat-grade-affordances §Unread Divider and Ambient Unread Count).
    /// Marks the boundary before the oldest retained unseen agent-authored turn
    /// when the viewer returns to a portal with unread content. Distinct from the
    /// generic turn separator so a viewer can tell the unread boundary from an
    /// ordinary turn break, yet still ambient — a presence engine marks where
    /// unread begins without behaving like a notification. Source token:
    /// `portal.unread_divider.color`.
    pub unread_divider_color: proto::Rgba,
    /// Color of the ambient awaiting-reply (question) indicator (hud-jip0k).
    /// Set when the owning LLM's most recently published output has
    /// `expects_reply == true` — a core presence semantic signaling the
    /// output is a question awaiting a viewer reply, not a chatbot
    /// notification. Ambient by design, matching the muted-tone convention
    /// of the other quiet-signal indicators. Source token:
    /// `portal.awaiting_reply.color`.
    pub awaiting_reply_color: proto::Rgba,
    /// Color of the friendly first-run empty-portal treatment (hud-g1ena.6,
    /// portal-chat-grade-affordances §First-Run Empty Portal Treatment) that
    /// replaces the literal `<empty projection stream>` placeholder. Quiet and
    /// inviting by design — a presence engine's empty surface reads as calm and
    /// ready, never as an error. Source token: `portal.empty_state.color`.
    pub empty_state_color: proto::Rgba,
    /// Color of the connecting-state marker (portal-chat-grade-affordances
    /// §Connecting State Distinction) shown while a portal is attached but has
    /// never connected (`has_ever_connected == false`). A cool "spinning up" hue
    /// deliberately distinct from the amber `stale_marker_color` so a starting-up
    /// portal never reads as failing. Ambient by design. Source token:
    /// `portal.connecting_marker.color`.
    pub connecting_marker_color: proto::Rgba,
    /// Color of the ambient header activity cue (portal-chat-grade-affordances
    /// §Agent Activity and Streaming Cue, hud-g1ena.5) shown while the owning
    /// adapter is actively appending to the transcript. Derived from observed
    /// appends (`appended_at_wall_us` vs render-time now), never a separate
    /// "typing" protocol. Muted/ambient — continuous streaming never
    /// re-escalates attention. Source token: `portal.activity_cue.color`.
    pub activity_cue_color: proto::Rgba,
    /// Color of the ambient transcript-tail streaming cursor shown while the
    /// agent is actively appending. Same activity semantic as
    /// `activity_cue_color`; a distinct token so a profile can style the tail
    /// cursor independently. Source token: `portal.streaming_cursor.color`.
    pub streaming_cursor_color: proto::Rgba,

    // Lifecycle affordance accents (cooperative-hud-projection §lifecycle).
    //
    // Drive the viewer-facing affordance for a projection's published
    // `lifecycle_state`. Each `ProjectionLifecycleState` variant maps onto one of
    // these four ambient accents via `lifecycle_accent_color`. Source tokens:
    // `portal.lifecycle.{active,attached,attention,inactive}_color`.
    /// Accent for the actively-working state (`Active`).
    pub lifecycle_active_color: proto::Rgba,
    /// Accent for the attached/ready state (`Attached`).
    pub lifecycle_attached_color: proto::Rgba,
    /// Accent for attention states (`Degraded` / `HudUnavailable`).
    pub lifecycle_attention_color: proto::Rgba,
    /// Accent for winding-down states (`Detached` / `CleanupPending` / `Expired`).
    pub lifecycle_inactive_color: proto::Rgba,
    /// Width (px) of the left-edge lifecycle accent bar. Geometry only; sizes the
    /// `SetTileLifecycleAccent` overlay state the compositor paints from, so the
    /// adapter holds no literal accent dimension. Token: `portal.lifecycle.accent_width_px`.
    pub lifecycle_accent_width_px: f32,

    // Collapsed card (collapsed presentation)
    pub collapsed_background: proto::Rgba,
    pub collapsed_text_color: proto::Rgba,
    pub collapsed_font_size_px: f32,

    // Composer (draft input region — §4.1, §4.8)
    pub composer_background: proto::Rgba,
    pub composer_text_color: proto::Rgba,
    pub composer_font_size_px: f32,
    /// Color applied to the composer line when the draft is at its byte cap.
    /// Renders as a distinct visual signal ("limit reached") without alarming.
    /// Source token: `portal.composer.at_capacity_color`.
    pub composer_at_capacity_color: proto::Rgba,
}

impl Default for PortalVisualTokens {
    /// Default visual tokens derived from the canonical palette in
    /// `tze_hud_config::PortalPartTokens::default()`.
    ///
    /// This is the **single source of truth** for the default portal palette.
    /// Previously this `impl` contained hard-coded float literals that diverged
    /// from the config defaults by up to 3 ULPs of rounding. The duplicate was
    /// eliminated in hud-dcynv: both sides now derive from the same source.
    ///
    /// In production these values are superseded by `with_tokens()` which
    /// accepts tokens produced by
    /// `tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens`.
    /// This default is used only in tests that do not exercise the token path,
    /// and as a fallback when no tokens are supplied to the adapter.
    fn default() -> Self {
        portal_visual_tokens_from_part_tokens(&tze_hud_config::PortalPartTokens::default())
    }
}

/// Result timing for adapter-local resident-path work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidentGrpcBudgetSample {
    pub elapsed_us: u64,
    pub budget_us: u64,
}

impl ResidentGrpcBudgetSample {
    pub fn within_budget(self) -> bool {
        self.elapsed_us <= self.budget_us
    }
}

/// Geometry and lease configuration for one projected portal tile.
#[derive(Clone, Debug)]
pub struct ResidentGrpcPortalConfig {
    pub lease_id: Vec<u8>,
    pub expanded_bounds: proto::Rect,
    pub compact_bounds: proto::Rect,
    pub z_order: u32,
}

impl ResidentGrpcPortalConfig {
    pub fn new(lease_id: Vec<u8>) -> Self {
        Self {
            lease_id,
            expanded_bounds: proto::Rect {
                x: 64.0,
                y: 180.0,
                width: DEFAULT_EXPANDED_W,
                height: DEFAULT_EXPANDED_H,
            },
            compact_bounds: proto::Rect {
                x: 64.0,
                y: 180.0,
                width: DEFAULT_COMPACT_W,
                height: DEFAULT_COMPACT_H,
            },
            z_order: DEFAULT_Z_ORDER,
        }
    }
}

/// Kind of resident operation produced by the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidentGrpcPortalCommandKind {
    CreatePortalTile,
    ReusePortalTile,
    RenderPortal,
    ReleaseLease,
}

/// One outbound `HudSession` client message plus adapter-local budget evidence.
#[derive(Debug)]
pub struct ResidentGrpcPortalCommand {
    pub kind: ResidentGrpcPortalCommandKind,
    pub message: session_proto::ClientMessage,
    pub budget: ResidentGrpcBudgetSample,
}

/// Local-first result for a HUD composer submission mapped into the semantic
/// cooperative projection inbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentGrpcPortalInputResult {
    pub feedback: PortalInputFeedback,
    pub budget: ResidentGrpcBudgetSample,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResidentGrpcAdapterError {
    #[error("resident portal tile has not been created or recorded")]
    MissingPortalTile,
}

/// Kind of draft-aware command produced by the adapter.
///
/// Used to distinguish draft-state update renders from full portal renders so
/// callers can route them to the correct tile mutation path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidentGrpcDraftCommandKind {
    /// Update the composer text/caret display to reflect a state-stream draft
    /// notification. This is a coalesced update; the adapter discards older
    /// notifications and renders only the latest snapshot.
    UpdateComposerDisplay,
    /// Process a transactional draft submission (forward to semantic inbox).
    ProcessSubmission,
    /// Process a transactional cancel (clear composer display).
    ProcessCancel,
}

/// One outbound command produced by the draft notification path.
#[derive(Debug)]
pub struct ResidentGrpcDraftCommand {
    pub kind: ResidentGrpcDraftCommandKind,
    pub budget: ResidentGrpcBudgetSample,
    /// Draft text at the time of the command (empty on cancel).
    pub draft_text: String,
    /// Cursor byte offset (for composer display).
    pub cursor: usize,
    /// Whether the draft is at capacity.
    pub at_capacity: bool,
    /// Sequence from the runtime draft buffer.
    pub sequence: u64,
}

/// Stateful daemon-side adapter for one projected session's resident portal.
///
/// ## Token-driven styling (§6.1 — no literal visual values)
///
/// Construct with `ResidentGrpcPortalAdapter::new` for default tokens, or
/// `ResidentGrpcPortalAdapter::with_tokens` to supply resolved design tokens.
///
/// Every color, font size, and transition duration in the rendered portal tile
/// comes from `visual_tokens`, never from inline literals. To reskin the portal,
/// call `set_visual_tokens` with a freshly-resolved `PortalVisualTokens` built
/// from the updated token map — no adapter logic changes required.
///
/// ## Composer draft display (§4.1 / §4.8 — local feedback first)
///
/// The adapter tracks the current draft display state in `composer_display`.
/// After `consume_draft_batch` or `apply_draft_notification` returns an
/// `UpdateComposerDisplay` command, the caller does NOT need to re-set anything:
/// the adapter updates its own `composer_display` field automatically so the
/// next `render_portal_message` / `portal_node` call immediately reflects the
/// current draft text, caret, and at-capacity state.
#[derive(Clone, Debug)]
pub struct ResidentGrpcPortalAdapter {
    config: ResidentGrpcPortalConfig,
    tile_id: Option<Vec<u8>>,
    next_input_sequence: u64,
    /// Latest draft sequence seen by this adapter — used to skip stale
    /// state-stream notifications that arrive out of order.
    last_draft_sequence: u64,
    /// Resolved visual tokens for portal part styling.
    ///
    /// All visual properties (colors, font sizes, transition durations) in the
    /// rendered portal tile MUST originate here. A profile swap updates this
    /// field; no other code in the adapter must change.
    visual_tokens: PortalVisualTokens,
    /// Current composer draft display state for local-first rendering (§4.1).
    ///
    /// Updated by `consume_draft_batch` / `apply_draft_notification` whenever
    /// an `UpdateComposerDisplay` command is produced. Reset to `None` on
    /// `ProcessCancel`. The `portal_node` render path reads this field to
    /// produce the draft text + caret + at-capacity visual without any remote
    /// roundtrip.
    composer_display: Option<ComposerDisplayState>,
}

/// Local composer draft display state cached in the adapter.
///
/// Carries only what `portal_node` needs to render the composer region:
/// the current draft text, the caret byte offset, and the at-capacity flag.
/// This is **not** a copy of `ComposerDraft` — it is the last state-stream
/// snapshot delivered to the adapter, suitable for display.
///
/// Spec: §4.1 — "local rendering of text, caret, and selection within the
/// input-to-local-ack budget."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerDisplayState {
    /// Draft text at the time of the last delivered notification.
    pub text: String,
    /// Cursor byte offset into `text` (where the caret `▌` is inserted).
    pub cursor: usize,
    /// True when the draft reached its byte cap on the last mutation.
    pub at_capacity: bool,
    /// Monotonic sequence from the originating `DraftStateNotification`.
    pub sequence: u64,
}

impl ResidentGrpcPortalAdapter {
    /// Create a new adapter with default visual tokens.
    ///
    /// In production, call `with_tokens` or `set_visual_tokens` immediately
    /// after construction to supply the runtime's resolved design token set.
    /// Using default tokens is acceptable only in tests that do not exercise
    /// the profile-swap path.
    pub fn new(config: ResidentGrpcPortalConfig) -> Self {
        Self {
            config,
            tile_id: None,
            next_input_sequence: 0,
            last_draft_sequence: 0,
            visual_tokens: PortalVisualTokens::default(),
            composer_display: None,
        }
    }

    /// Create a new adapter with the given resolved visual tokens.
    ///
    /// This is the preferred constructor for production use. Build
    /// `tokens` from the runtime's resolved `DesignTokenMap` by calling
    /// `tze_hud_config::resolve_portal_tokens` first to get `PortalPartTokens`,
    /// then converting with
    /// `tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens`
    /// to obtain the Phase-1 pilot `PortalVisualTokens`.
    pub fn with_tokens(config: ResidentGrpcPortalConfig, tokens: PortalVisualTokens) -> Self {
        Self {
            config,
            tile_id: None,
            next_input_sequence: 0,
            last_draft_sequence: 0,
            visual_tokens: tokens,
            composer_display: None,
        }
    }

    /// Update visual tokens (e.g., after a profile hot-reload).
    ///
    /// The next `render_portal_message` call after this returns will use the
    /// new token values. No other adapter state changes — this is the
    /// "profile swap without adapter logic change" contract from §6.1.
    pub fn set_visual_tokens(&mut self, tokens: PortalVisualTokens) {
        self.visual_tokens = tokens;
    }

    /// Returns the current visual tokens (for inspection / test assertions).
    pub fn visual_tokens(&self) -> &PortalVisualTokens {
        &self.visual_tokens
    }

    /// Returns the current composer display state (for inspection / test assertions).
    ///
    /// `None` when no draft is active (no `UpdateComposerDisplay` command has
    /// been produced since construction or the last `ProcessCancel`).
    pub fn composer_display(&self) -> Option<&ComposerDisplayState> {
        self.composer_display.as_ref()
    }

    pub fn tile_id(&self) -> Option<&[u8]> {
        self.tile_id.as_deref()
    }

    pub fn lease_id(&self) -> &[u8] {
        &self.config.lease_id
    }

    /// Returns the configured expanded-presentation viewport height in pixels.
    ///
    /// Equals `ResidentGrpcPortalConfig::expanded_bounds.height` as set at
    /// adapter construction time (default: `DEFAULT_EXPANDED_H`).
    pub fn config_expanded_height(&self) -> f32 {
        self.config.expanded_bounds.height
    }

    /// Returns the configured compact-presentation viewport height in pixels.
    ///
    /// Equals `ResidentGrpcPortalConfig::compact_bounds.height` as set at
    /// adapter construction time (default: `DEFAULT_COMPACT_H`).
    pub fn config_compact_height(&self) -> f32 {
        self.config.compact_bounds.height
    }

    /// Returns the configured viewport height for the given presentation mode.
    ///
    /// Used by the drain loop to populate `PortalAppendGeometry::viewport_height_px`
    /// as a fallback when no live geometry snapshot is available (hud-0528i).
    /// Expanded → `expanded_bounds.height`; Collapsed → `compact_bounds.height`.
    pub fn config_viewport_height(&self, presentation: crate::ProjectedPortalPresentation) -> f32 {
        match presentation {
            crate::ProjectedPortalPresentation::Expanded => self.config.expanded_bounds.height,
            crate::ProjectedPortalPresentation::Collapsed => self.config.compact_bounds.height,
        }
    }

    /// Record the tile ID returned by the resident `CreateTile` mutation.
    pub fn record_created_tile(&mut self, tile_id: Vec<u8>) {
        self.tile_id = Some(tile_id);
    }

    /// Render the portal markdown content for the given state without building
    /// a full gRPC proto message.
    ///
    /// Useful for stdio surfaces that need the semantic content to include in
    /// a JSON drain record, without taking a prost dependency on the binary.
    /// The output is the same string that `portal_node` places in
    /// `TextMarkdownNodeProto::content`.
    ///
    /// `now_wall_us` is the render's wall-clock reference used to derive the
    /// ambient agent-activity / streaming-cursor cue from observed appends
    /// (hud-g1ena.5); callers pass the same timestamp they hand to
    /// `render_portal_message` so the drain-record markdown matches the tile.
    pub fn render_portal_markdown(&self, state: &ProjectedPortalState, now_wall_us: u64) -> String {
        portal_markdown(state, self.composer_display.as_ref(), now_wall_us)
    }

    /// Move the compact affordance. The next collapsed render publishes this
    /// geometry through `PublishToTile`, reusing the existing content-layer tile.
    pub fn move_compact_to(&mut self, x: f32, y: f32) {
        self.config.compact_bounds.x = x;
        self.config.compact_bounds.y = y;
    }

    /// Create a content-layer portal tile if needed; otherwise publish a reuse
    /// render into the existing tile.
    pub fn ensure_portal_tile_message(
        &self,
        state: &ProjectedPortalState,
        sequence: u64,
        timestamp_wall_us: u64,
    ) -> Result<ResidentGrpcPortalCommand, ResidentGrpcAdapterError> {
        let started = Instant::now();
        let (kind, payload) = if self.tile_id.is_some() {
            (
                ResidentGrpcPortalCommandKind::ReusePortalTile,
                session_proto::client_message::Payload::MutationBatch(
                    self.render_batch(state, timestamp_wall_us)?,
                ),
            )
        } else {
            (
                ResidentGrpcPortalCommandKind::CreatePortalTile,
                session_proto::client_message::Payload::MutationBatch(
                    session_proto::MutationBatch {
                        batch_id: new_scene_id_bytes(),
                        lease_id: self.config.lease_id.clone(),
                        mutations: vec![proto::MutationProto {
                            mutation: Some(proto::mutation_proto::Mutation::CreateTile(
                                proto::CreateTileMutation {
                                    tab_id: Vec::new(),
                                    bounds: Some(self.bounds_for_state(state)),
                                    z_order: self.config.z_order,
                                },
                            )),
                        }],
                        timing: None,
                    },
                ),
            )
        };
        Ok(self.command(kind, sequence, timestamp_wall_us, payload, started))
    }

    /// Render expanded/collapsed projected state into the existing resident
    /// portal tile, including current geometry and input mode.
    pub fn render_portal_message(
        &self,
        state: &ProjectedPortalState,
        sequence: u64,
        timestamp_wall_us: u64,
    ) -> Result<ResidentGrpcPortalCommand, ResidentGrpcAdapterError> {
        let started = Instant::now();
        Ok(self.command(
            ResidentGrpcPortalCommandKind::RenderPortal,
            sequence,
            timestamp_wall_us,
            session_proto::client_message::Payload::MutationBatch(
                self.render_batch(state, timestamp_wall_us)?,
            ),
            started,
        ))
    }

    /// Release the resident lease so the runtime removes stale projected tiles
    /// through the normal lease cleanup path.
    pub fn release_lease_message(
        &self,
        sequence: u64,
        timestamp_wall_us: u64,
    ) -> ResidentGrpcPortalCommand {
        let started = Instant::now();
        self.command(
            ResidentGrpcPortalCommandKind::ReleaseLease,
            sequence,
            timestamp_wall_us,
            session_proto::client_message::Payload::LeaseRelease(session_proto::LeaseRelease {
                lease_id: self.config.lease_id.clone(),
            }),
            started,
        )
    }

    // ── Draft notification methods (hud-5jbra.4) ─────────────────────────
    //
    // These replace the per-keystroke composer-text republish pattern. Instead
    // of publishing a new TextMarkdownNode on every CharacterEvent, the adapter
    // now:
    //   1. Calls `consume_draft_batch` with the AdapterDraftBatch it receives
    //      from the runtime's ComposerDraft notification path.
    //   2. For state-stream notifications: renders a `UpdateComposerDisplay`
    //      command containing the latest draft text (for compositor display).
    //   3. For transactional submissions: calls `submit_composer_text`.
    //
    // This satisfies spec §4.6: "update the cooperative projection adapter …
    // to consume draft-state notifications instead of per-keystroke republish."

    /// Consume a `AdapterDraftBatch` and produce draft commands for the adapter
    /// to process.
    ///
    /// The batch may contain a coalesced state-stream notification, a
    /// transactional submission, or a cancel. The adapter processes them in
    /// order: state-stream notification first (for display), then
    /// submission/cancel.
    ///
    /// Returns the set of commands to dispatch. State-stream commands carry
    /// only display data (draft text, cursor); the adapter applies them locally
    /// without a semantic inbox enqueue.
    pub fn consume_draft_batch(
        &mut self,
        batch: &AdapterDraftBatch,
    ) -> Vec<ResidentGrpcDraftCommand> {
        let mut commands = Vec::new();
        let started = Instant::now();

        // State-stream notification: only process if sequence is newer
        if let Some(notification) = &batch.latest {
            if notification.sequence > self.last_draft_sequence {
                self.last_draft_sequence = notification.sequence;
                // Update local composer display state for next portal_node render.
                // This is the local-first path: the compositor reads from here
                // without any remote roundtrip (spec §4.1 — local feedback first).
                self.composer_display = Some(ComposerDisplayState {
                    text: notification.text.clone(),
                    cursor: notification.cursor,
                    at_capacity: notification.at_capacity,
                    sequence: notification.sequence,
                });
                commands.push(ResidentGrpcDraftCommand {
                    kind: ResidentGrpcDraftCommandKind::UpdateComposerDisplay,
                    budget: sample_budget(started, RESIDENT_PORTAL_UPDATE_BUILD_BUDGET_US),
                    draft_text: notification.text.clone(),
                    cursor: notification.cursor,
                    at_capacity: notification.at_capacity,
                    sequence: notification.sequence,
                });
            }
        }

        // Transactional cancel — clear composer display state.
        // Advance last_draft_sequence to the cancel's sequence so that any
        // delayed state-stream notifications that arrived before the cancel
        // event (sequence ≤ cancel.sequence) are silently dropped instead of
        // re-populating composer_display after the clear.
        if let Some(cancel) = &batch.cancel {
            self.composer_display = None;
            self.last_draft_sequence = self.last_draft_sequence.max(cancel.sequence);
            commands.push(ResidentGrpcDraftCommand {
                kind: ResidentGrpcDraftCommandKind::ProcessCancel,
                budget: sample_budget(started, RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US),
                draft_text: String::new(),
                cursor: 0,
                at_capacity: false,
                sequence: cancel.sequence,
            });
        }

        // Transactional submission (handled separately; caller should also
        // call `submit_composer_text` with the submission text).
        // Submission clears composer display state (post-submit display clear).
        // Advance last_draft_sequence so that any delayed state-stream
        // notifications with sequence ≤ submission.sequence are ignored.
        if let Some(submission) = &batch.submission {
            self.composer_display = None;
            self.last_draft_sequence = self.last_draft_sequence.max(submission.sequence);
            commands.push(ResidentGrpcDraftCommand {
                kind: ResidentGrpcDraftCommandKind::ProcessSubmission,
                budget: sample_budget(started, RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US),
                draft_text: submission.text.clone(),
                cursor: submission.text.len(),
                at_capacity: false,
                sequence: submission.sequence,
            });
        }

        commands
    }

    /// Build a draft-state notification from an `AdapterDraftNotification`
    /// without consuming a full batch (useful for direct notification delivery).
    ///
    /// Returns `None` if the notification is stale (sequence ≤ last seen).
    pub fn apply_draft_notification(
        &mut self,
        notification: &AdapterDraftNotification,
    ) -> Option<ResidentGrpcDraftCommand> {
        let started = Instant::now();
        if notification.sequence <= self.last_draft_sequence {
            return None;
        }
        self.last_draft_sequence = notification.sequence;
        // Update local composer display state for next portal_node render.
        self.composer_display = Some(ComposerDisplayState {
            text: notification.text.clone(),
            cursor: notification.cursor,
            at_capacity: notification.at_capacity,
            sequence: notification.sequence,
        });
        Some(ResidentGrpcDraftCommand {
            kind: ResidentGrpcDraftCommandKind::UpdateComposerDisplay,
            budget: sample_budget(started, RESIDENT_PORTAL_UPDATE_BUILD_BUDGET_US),
            draft_text: notification.text.clone(),
            cursor: notification.cursor,
            at_capacity: notification.at_capacity,
            sequence: notification.sequence,
        })
    }

    /// Last draft sequence seen by this adapter.
    pub fn last_draft_sequence(&self) -> u64 {
        self.last_draft_sequence
    }

    /// Map submitted HUD composer text to the cooperative semantic inbox. This
    /// is not a raw keystroke path; the active LLM session later polls the
    /// pending item through the projection operation contract.
    pub fn submit_composer_text(
        &mut self,
        authority: &mut ProjectionAuthority,
        projection_id: &str,
        text: String,
        submitted_at_wall_us: u64,
        expires_at_wall_us: Option<u64>,
        content_classification: ContentClassification,
    ) -> ResidentGrpcPortalInputResult {
        let started = Instant::now();
        self.next_input_sequence += 1;
        let feedback = authority.submit_portal_input(
            projection_id,
            PortalInputSubmission {
                input_id: format!("input-{}", self.next_input_sequence),
                submission_text: text,
                submitted_at_wall_us,
                expires_at_wall_us,
                content_classification,
            },
        );
        ResidentGrpcPortalInputResult {
            feedback,
            budget: sample_budget(started, RESIDENT_PORTAL_INPUT_FEEDBACK_BUDGET_US),
        }
    }

    /// Build the portal-content `MutationBatch` for the given projected state.
    ///
    /// This is the single render path shared by both adapter families: the
    /// gRPC/wire family wraps it in a `ClientMessage` (`ensure_portal_tile_message`
    /// / `render_portal_message`) and sends it over the session stream, while the
    /// in-process cooperative driver applies it directly to the `SceneGraph` via
    /// `tze_hud_protocol::convert::apply_portal_render_batch_to_scene`. It is
    /// `pub` so the runtime driver can ask the adapter to render content rather
    /// than only counting transcript lines for geometry (the cooperative
    /// grey-tile fix, hud-utbiy).
    ///
    /// Requires `record_created_tile` to have been called first (returns
    /// [`ResidentGrpcAdapterError::MissingPortalTile`] otherwise).
    ///
    /// `now_wall_us` is the render's wall-clock reference (the same timestamp the
    /// caller stamps on the outbound message). It is used only to derive the
    /// ambient agent-activity / streaming-cursor cue from observed appends
    /// (hud-g1ena.5) — a portal re-rendered long after its last append shows no
    /// cursor because `now - appended_at` exceeds the quiesce window.
    pub fn render_batch(
        &self,
        state: &ProjectedPortalState,
        now_wall_us: u64,
    ) -> Result<session_proto::MutationBatch, ResidentGrpcAdapterError> {
        let tile_id = self
            .tile_id
            .clone()
            .ok_or(ResidentGrpcAdapterError::MissingPortalTile)?;

        // Generate an explicit root node ID so we can reference it as parent_id
        // in the composer hit-region AddNodeMutation within the same batch.
        //
        // NodeProto.id wire encoding: little-endian UUID bytes (per RFC 0001 §4.1).
        // AddNodeMutation.parent_id wire encoding: big-endian RFC 4122 bytes.
        // These two encodings reference the same underlying UUID.
        let root_uuid = uuid::Uuid::now_v7();
        let root_id_le = root_uuid.to_bytes_le().to_vec();
        let root_id_be = root_uuid.as_bytes().to_vec();

        let mut mutations = vec![
            proto::MutationProto {
                mutation: Some(proto::mutation_proto::Mutation::PublishToTile(
                    proto::PublishToTileMutation {
                        element_id: tile_id.clone(),
                        bounds: Some(self.bounds_for_state(state)),
                        node: Some(self.portal_node(state, root_id_le, now_wall_us)),
                    },
                )),
            },
            proto::MutationProto {
                mutation: Some(proto::mutation_proto::Mutation::UpdateTileInputMode(
                    proto::UpdateTileInputModeMutation {
                        tile_id: tile_id.clone(),
                        input_mode: if state.interaction_enabled {
                            proto::TileInputModeProto::TileInputModeCapture as i32
                        } else {
                            proto::TileInputModeProto::TileInputModeLocalOnly as i32
                        },
                    },
                )),
            },
        ];

        // Lifecycle affordance accent (hud-m48i0): a coalescible StateStream
        // tile-update carrying the token-resolved accent color + width. The
        // runtime stores it as per-tile overlay state and the compositor paints a
        // left-edge bar from it. This is deliberately NOT an AddNode: a
        // per-republish AddNode classifies the whole batch Transactional and flips
        // non-interactive lifecycle-visible portals off the coalescible path
        // (hud-mzk74). Stored as overlay state, the accent also survives the
        // PublishToTile content republish above (which replaces the node tree).
        //
        // Redaction-gated: when lifecycle_state is None (authority redacted),
        // color is None / width 0 → the mutation CLEARS the accent, exactly like
        // the redaction-gated `status:` line. Emitted every render so the latest
        // lifecycle color coalesces; the markdown node keeps only its zero-length
        // lifecycle sentinel run, so the cached markdown path is untouched (#947).
        let (accent_color, accent_width) = match state.lifecycle_state {
            Some(lifecycle) => (
                Some(lifecycle_accent_color(lifecycle, &self.visual_tokens)),
                self.visual_tokens.lifecycle_accent_width_px,
            ),
            None => (None, 0.0),
        };
        mutations.push(proto::MutationProto {
            mutation: Some(proto::mutation_proto::Mutation::SetTileLifecycleAccent(
                proto::SetTileLifecycleAccentMutation {
                    tile_id: tile_id.clone(),
                    color: accent_color,
                    width_px: accent_width,
                },
            )),
        });

        // When interaction is enabled, publish a composer hit region as a child
        // of the portal root so the runtime's ComposerDraftManager can activate.
        // Without this AddNodeMutation, accepts_composer_input is never true in
        // any wire-driven scene (is_composer_active() always returns false).
        if state.interaction_enabled {
            let composer_bounds = self.local_bounds_for_state(state);
            let composer_interaction_id = format!("{}-composer", state.portal_id);
            mutations.push(proto::MutationProto {
                mutation: Some(proto::mutation_proto::Mutation::AddNode(
                    proto::AddNodeMutation {
                        tile_id,
                        parent_id: root_id_be,
                        node: Some(proto::NodeProto {
                            id: Vec::new(),
                            data: Some(proto::node_proto::Data::HitRegion(
                                proto::HitRegionNodeProto {
                                    bounds: Some(composer_bounds),
                                    interaction_id: composer_interaction_id,
                                    accepts_focus: true,
                                    // accepts_pointer MUST be true for click-to-focus
                                    // (hud-v4k1h). SceneGraph::hit_test only returns
                                    // HitResult::NodeHit for HitRegion nodes with
                                    // accepts_pointer = true; InputProcessor::process_with_focus
                                    // only acquires keyboard focus on a NodeHit. With this
                                    // false, a pointer-down on the composer fell through to a
                                    // bare TileHit, so the portal never gained focus and every
                                    // keystroke / Ctrl+= resize chord was silently dropped even
                                    // though the OS delivered it. The three local-render composer
                                    // sites in windowed/portal.rs already set this true; the
                                    // wire-driven projection path had diverged.
                                    accepts_pointer: true,
                                    auto_capture: false,
                                    release_on_up: false,
                                    accepts_composer_input: true,
                                },
                            )),
                        }),
                    },
                )),
            });
        }

        Ok(session_proto::MutationBatch {
            batch_id: new_scene_id_bytes(),
            lease_id: self.config.lease_id.clone(),
            mutations,
            timing: None,
        })
    }

    fn portal_node(
        &self,
        state: &ProjectedPortalState,
        root_id_le: Vec<u8>,
        now_wall_us: u64,
    ) -> proto::NodeProto {
        // §6.1 enforcement: every visual value sourced from self.visual_tokens —
        // no literal colors, font sizes, or opacities permitted here.
        let bounds = self.local_bounds_for_state(state);
        let (mut text_color, mut background_color, font_size_px) = match state.presentation {
            ProjectedPortalPresentation::Expanded => (
                self.visual_tokens.transcript_text_color,
                self.visual_tokens.transcript_background,
                self.visual_tokens.transcript_font_size_px,
            ),
            ProjectedPortalPresentation::Collapsed => (
                self.visual_tokens.collapsed_text_color,
                self.visual_tokens.collapsed_background,
                self.visual_tokens.collapsed_font_size_px,
            ),
        };
        // §2: when the driving stream is disconnected/stale, dim the retained
        // transcript window using token-resolved colors (never hardcoded) so the
        // viewer reads it as inactive rather than blanked or faking liveness. The
        // dim treatment applies to the expanded transcript surface; the collapsed
        // card keeps its own palette.
        if is_connection_degraded(state)
            && state.presentation == ProjectedPortalPresentation::Expanded
        {
            text_color = self.visual_tokens.transcript_dim_text_color;
            background_color = self.visual_tokens.transcript_dim_background;
        }
        proto::NodeProto {
            // Explicit root ID (little-endian UUID bytes per RFC 0001 §4.1) so
            // render_batch can reference it as AddNodeMutation.parent_id in the
            // same batch when adding the composer hit region.
            id: root_id_le,
            data: Some(proto::node_proto::Data::TextMarkdown(
                proto::TextMarkdownNodeProto {
                    content: portal_markdown(state, self.composer_display.as_ref(), now_wall_us),
                    bounds: Some(bounds),
                    font_size_px,
                    color: Some(text_color),
                    background: Some(background_color),
                    // color_runs carry the composer at-capacity indicator, the
                    // disconnect/stale marker color, and the lifecycle-affordance
                    // accent when active. Each is a zero-length sentinel run
                    // carrying the token color so the visual token drives the
                    // display without any literal color in the render path
                    // (§2/§6.1: token-resolved, never hardcoded).
                    color_runs: {
                        let mut runs =
                            stale_marker_color_runs(state, self.visual_tokens.stale_marker_color);
                        // Lifecycle affordance: token-resolved accent reflecting the
                        // published lifecycle_state (active/attached/attention/inactive).
                        // Redaction-gated via state.lifecycle_state being None.
                        runs.extend(lifecycle_marker_color_runs(state, &self.visual_tokens));
                        runs.extend(composer_color_runs(
                            state,
                            self.composer_display.as_ref(),
                            self.visual_tokens.composer_at_capacity_color,
                        ));
                        // Ambient unread-output-count indicator (hud-meqet). Absent
                        // when there is nothing unread or the count is redacted.
                        runs.extend(unread_indicator_color_runs(
                            state,
                            self.visual_tokens.unread_indicator_color,
                        ));
                        // In-transcript unread divider (hud-g1ena.2). Absent unless
                        // an Expanded portal's non-empty retained window contains an
                        // unread agent-authored turn; clears with the count on tail
                        // view. Distinct token from the ambient count above so the
                        // boundary rule and the count can be reskinned separately.
                        runs.extend(unread_divider_color_runs(
                            state,
                            self.visual_tokens.unread_divider_color,
                        ));
                        // Ambient awaiting-reply (question) indicator
                        // (hud-jip0k). Absent unless the most recently
                        // published output is a pending question.
                        runs.extend(awaiting_reply_color_runs(
                            state,
                            self.visual_tokens.awaiting_reply_color,
                        ));
                        // First-run empty-state treatment (hud-g1ena.6). Absent
                        // unless the retained transcript is empty on a portal that
                        // has connected; the never-connected case is handled by
                        // connecting_color_runs below (mutually exclusive gates).
                        runs.extend(empty_state_color_runs(
                            state,
                            self.visual_tokens.empty_state_color,
                        ));
                        // Connecting-state treatment (hud-g1ena.7,
                        // §Connecting State Distinction). Absent unless the portal
                        // is attached-but-never-connected with an empty transcript;
                        // its cool token is distinct from the amber stale marker so
                        // a starting-up portal never reads as failing.
                        runs.extend(connecting_color_runs(
                            state,
                            self.visual_tokens.connecting_marker_color,
                        ));
                        // Ambient agent-activity header cue + transcript-tail
                        // streaming cursor (hud-g1ena.5, §Agent Activity and
                        // Streaming Cue). Both derive from observed appends
                        // (`appended_at_wall_us` vs `now_wall_us`) via
                        // `agent_activity_active`; absent unless the tail is a
                        // fresh agent turn on a live expanded portal, so they
                        // quiesce promptly and suppress under redaction. Header +
                        // cursor take distinct tokens so a profile can style them
                        // apart. Kept in the HEADER + latest-turn-cursor region.
                        runs.extend(activity_cue_color_runs(
                            state,
                            self.visual_tokens.activity_cue_color,
                            now_wall_us,
                        ));
                        runs.extend(streaming_cursor_color_runs(
                            state,
                            self.visual_tokens.streaming_cursor_color,
                            now_wall_us,
                        ));
                        runs
                    },
                    // Transcript panes use Ellipsis to engage the TruncationCache
                    // contract (word-boundary ellipsis, tail-anchored follow-tail).
                    // This prevents partially-clipped final glyphs under
                    // non-line-multiple viewports.
                    overflow: proto::TextOverflowProto::Ellipsis as i32,
                },
            )),
        }
    }

    fn bounds_for_state(&self, state: &ProjectedPortalState) -> proto::Rect {
        match state.presentation {
            ProjectedPortalPresentation::Expanded => {
                // hud-v4k1h: an Expanded portal that has been resized renders its
                // body + composer at the durable resized size, not the fixed
                // config size. The runtime grows the tile bounds locally on
                // resize; without honoring that here, the body stayed config-
                // sized and the grown tile area showed an empty "shadow-body".
                // Collapsed always uses compact_bounds (resize is an
                // Expanded-only affordance), so the override is Expanded-scoped.
                if let Some(resized) = state.resized_bounds {
                    if resized.width_px > 0 && resized.height_px > 0 {
                        // Preserve the resized ORIGIN as well as the size: a
                        // left/top edge pointer-drag keeps the opposite edge
                        // stationary (DeviceResizeState::compute_rect), so the
                        // snapshot carries a shifted x/y. bounds_for_state feeds
                        // the per-render PublishToTile bounds, so returning the
                        // static config origin here would snap the tile back to
                        // the configured position on the next publish while
                        // keeping the new size. local_bounds_for_state still
                        // zeroes x/y for the tile-local node bounds.
                        return proto::Rect {
                            x: resized.x_px as f32,
                            y: resized.y_px as f32,
                            width: resized.width_px as f32,
                            height: resized.height_px as f32,
                        };
                    }
                }
                self.config.expanded_bounds
            }
            ProjectedPortalPresentation::Collapsed => self.config.compact_bounds,
        }
    }

    fn local_bounds_for_state(&self, state: &ProjectedPortalState) -> proto::Rect {
        let source = self.bounds_for_state(state);
        proto::Rect {
            x: 0.0,
            y: 0.0,
            width: source.width,
            height: source.height,
        }
    }

    fn command(
        &self,
        kind: ResidentGrpcPortalCommandKind,
        sequence: u64,
        timestamp_wall_us: u64,
        payload: session_proto::client_message::Payload,
        started: Instant,
    ) -> ResidentGrpcPortalCommand {
        ResidentGrpcPortalCommand {
            kind,
            message: session_proto::ClientMessage {
                sequence,
                timestamp_wall_us,
                payload: Some(payload),
            },
            budget: sample_budget(started, RESIDENT_PORTAL_UPDATE_BUILD_BUDGET_US),
        }
    }
}

fn portal_markdown(
    state: &ProjectedPortalState,
    composer_display: Option<&ComposerDisplayState>,
    now_wall_us: u64,
) -> String {
    let mut result = String::new();
    let title = state.display_name.as_deref().unwrap_or("Projected session");
    push_line(&mut result, &format!("**{title}**"));
    push_line(
        &mut result,
        &format!(
            "`{}` · {:?} · {:?}",
            state.portal_id, state.presentation, state.attention
        ),
    );
    // Ambient unread-output-count indicator (hud-meqet). `unread_output_count`
    // is `None` when redacted (`reveal_unread` policy gate) and `Some(0)` when
    // there is nothing unread — both render nothing. This is a presence engine,
    // not a notification engine: a quiet count near the header, never a loud
    // badge (CLAUDE.md doctrine).
    if let Some(line) = unread_indicator_line(state.unread_output_count) {
        push_line(&mut result, &line);
    }
    // Ambient awaiting-reply (question) indicator (hud-jip0k). Present only
    // when the most recently published visible transcript unit is a question
    // awaiting a viewer reply (`expects_reply == true`). A redacted viewer
    // naturally gets nothing here — `visible_transcript` is already empty
    // under redaction — and a viewer's own echoed reply becomes the new last
    // unit (with `expects_reply == false`), which is exactly what clears the
    // cue once answered. Minimal and text-visible so the signal survives
    // environments that don't inspect color_runs; kept quiet (no `!`/⚠) to
    // stay ambient per presence-engine doctrine.
    if awaiting_reply(state) {
        push_line(&mut result, "? awaiting reply");
    }
    // Ambient agent-activity header cue (hud-g1ena.5, §Agent Activity and
    // Streaming Cue). A compact typing-style indicator shown while the owning
    // adapter is actively appending — derived purely from observed appends via
    // `agent_activity_active` (the tail is a fresh, non-question agent turn on a
    // live expanded portal). Mutually exclusive with the awaiting-reply cue above
    // (that requires `expects_reply`; this requires `!expects_reply`). Text-
    // visible so the signal survives environments that don't inspect color_runs;
    // the token-driven `activity_cue_color` rides alongside via
    // `activity_cue_color_runs`. Kept quiet (no `!`/⚠) — continuous streaming
    // must never escalate attention (doctrine: not a notification engine).
    if agent_activity_active(state, now_wall_us) {
        push_line(&mut result, PORTAL_ACTIVITY_MARKER_LINE);
    }
    // §2: content-free disconnect/stale marker. Emitted from the
    // redaction-independent `connection_degraded` flag and BEFORE the
    // redaction-gated lifecycle line, so it remains present (and reveals nothing)
    // even for a restricted viewer — like the scroll-position indicator. It
    // carries connection state only, never identity or transcript content.
    if is_connection_degraded(state) {
        push_line(&mut result, PORTAL_DISCONNECT_MARKER_LINE);
    }
    if let Some(lifecycle) = state.lifecycle_state {
        // Ambient, content-free glyph + exact spelling. The glyph groups states
        // for at-a-glance scanning (active / ready / attention / inactive); the
        // accent color is carried separately via `lifecycle_marker_color_runs`.
        push_line(
            &mut result,
            &format!("status: {} {lifecycle:?}", lifecycle_glyph(lifecycle)),
        );
    }
    if let Some(status_text) = state.status_text.as_deref() {
        push_line(&mut result, &format!("note: {status_text}"));
    }

    match state.presentation {
        ProjectedPortalPresentation::Expanded => {
            push_line(&mut result, "");
            // §First-Run Empty Portal Treatment (hud-g1ena.6): an empty retained
            // transcript renders the friendly, token-styled empty/connecting body
            // instead of the literal `<empty projection stream>`. The first
            // appended unit replaces it (this branch is empty-only).
            if state.visible_transcript.is_empty() {
                push_line(&mut result, empty_portal_markdown(state));
            } else {
                // Transcript body carries the in-transcript unread divider
                // (hud-g1ena.2, via `unread_divider_boundary`) plus, when the agent
                // is actively appending, an ambient streaming cursor at the tail of
                // the latest turn (hud-g1ena.5). The cursor is appended AFTER
                // `visible_transcript_markdown` (not inside it) so this stays clear
                // of the transcript-body region and the glyph reads as a
                // live-writing caret at the very tail. The token-driven
                // `streaming_cursor_color` rides alongside via
                // `streaming_cursor_color_runs`; `agent_activity_active` is the
                // single gate for both the glyph and the sentinel run.
                let mut body = visible_transcript_markdown(
                    &state.visible_transcript,
                    unread_divider_boundary(state),
                );
                if agent_activity_active(state, now_wall_us) {
                    body.push_str(PORTAL_STREAMING_CURSOR_GLYPH);
                }
                push_line(&mut result, &body);
            }
            push_line(&mut result, "");
            if is_connection_degraded(state) {
                // §2: clear live activity/typing/composer-ready signals on
                // disconnect — the surface must not imply an active stream. The
                // authority also forces `interaction_enabled = false` when
                // degraded; this is the redundant presentation-layer guard.
                push_line(&mut result, "composer: unavailable");
            } else if state.interaction_enabled {
                // Render composer region with draft text + caret (§4.1).
                // Local-first: no remote roundtrip — reads from adapter's cached
                // ComposerDisplayState which is updated by consume_draft_batch /
                // apply_draft_notification on every delivered notification.
                let composer_line = composer_line(composer_display, state.interaction_enabled);
                push_line(&mut result, &composer_line);
            } else {
                push_line(&mut result, "composer: unavailable");
            }
        }
        ProjectedPortalPresentation::Collapsed => {
            let preview = state
                .visible_transcript
                .last()
                .map(|unit| unit.output_text.as_str())
                .unwrap_or("compact projection affordance");
            push_line(&mut result, &clamp_one_line(preview, 160));
        }
    }

    if let Some(pending) = state.pending_input_count {
        push_line(&mut result, &format!("pending HUD input: {pending}"));
    }
    if let Some(feedback) = &state.last_input_feedback {
        push_line(&mut result, &composer_feedback_line(feedback));
    }
    truncate_utf8(result, MAX_PORTAL_MARKDOWN_BYTES)
}

/// Human-legible one-line status for the last composer submission.
///
/// Replaces the old `Debug`-formatted enum (`last composer: Rejected`) with a
/// reason the viewer can act on. On rejection we map the machine error code to
/// a friendly sentence, falling back to the authority's `status_summary`
/// (already populated, e.g. "PROJECTION_HUD_UNAVAILABLE: portal input rejected")
/// when the code is absent (hud-phdkd).
fn composer_feedback_line(feedback: &PortalInputFeedback) -> String {
    match feedback.feedback_state {
        PortalInputFeedbackState::Accepted => "✓ sent".to_string(),
        PortalInputFeedbackState::Rejected => {
            let reason = match feedback.error_code {
                Some(ProjectionErrorCode::ProjectionHudUnavailable) => {
                    "the agent is disconnected".to_string()
                }
                Some(ProjectionErrorCode::ProjectionInputQueueFull) => {
                    "too many replies waiting — try again shortly".to_string()
                }
                Some(ProjectionErrorCode::ProjectionInputTooLarge) => {
                    "message too long".to_string()
                }
                Some(ProjectionErrorCode::ProjectionRateLimited) => {
                    "sending too fast — try again shortly".to_string()
                }
                Some(
                    ProjectionErrorCode::ProjectionTokenExpired
                    | ProjectionErrorCode::ProjectionUnauthorized,
                ) => "the projection session expired".to_string(),
                _ => clamp_one_line(&feedback.status_summary, 160),
            };
            format!("⚠ not sent — {reason}")
        }
    }
}

/// Build the composer status line for the expanded portal node.
///
/// This line is a **content-free status affordance**, not the draft surface. The
/// live draft text + caret are rendered exclusively by the compositor's
/// bottom-pinned input strip, which is the single source of truth for the draft
/// (hud-2zsbf: `Compositor::composer_input_box` confines the echo to a one-line
/// strip at the portal bottom). Embedding the draft here as well produced a
/// SECOND copy in the transcript flow — mid-portal, right after the transcript —
/// at a different Y than the bottom strip, reading live as a double / misaligned
/// composer (hud-f6zfa). So this line never carries the draft glyphs; it only
/// reflects composer availability.
///
/// States:
/// - `interaction_enabled == false` → `composer: unavailable`.
/// - no active draft (`composer_display` is None) → `composer: ready`.
/// - active draft → `composer: composing` (the draft itself is in the bottom
///   strip). When the draft is at capacity the line becomes
///   `composer: [!] at capacity`, keeping the at-capacity state text-visible for
///   environments without color runs; `composer_color_runs` still emits the
///   token-driven at-capacity color independently.
///
/// The promotion-era structured composer node (a dedicated footer input box with
/// its own bounds) will let the echo, chrome, and status share one geometry and
/// retire this interim dedup.
fn composer_line(
    composer_display: Option<&ComposerDisplayState>,
    interaction_enabled: bool,
) -> String {
    if !interaction_enabled {
        return "composer: unavailable".to_string();
    }
    let Some(display) = composer_display else {
        return "composer: ready".to_string();
    };

    if display.at_capacity {
        // Text-visible at-capacity marker WITHOUT the draft text (the bottom
        // strip owns the draft). The color_runs path applies the token-driven
        // color independently.
        "composer: [!] at capacity".to_string()
    } else {
        "composer: composing".to_string()
    }
}

/// Build `TextColorRunProto` entries for the at-capacity composer indicator.
///
/// When the draft is at capacity and the portal is in expanded mode, emits a
/// single `TextColorRunProto` covering bytes `[0..0]` (a zero-length sentinel)
/// carrying `composer_at_capacity_color`. This is the token-driven color path
/// (§6.1): no literal colors in the render code.
///
/// When the draft is not at capacity or the presentation is not expanded,
/// returns an empty vec.
///
/// # Phase-1 scope note
///
/// In the Phase-1 raw-tile pilot, `color_runs` affect the full content of the
/// single `TextMarkdownNodeProto`. An accurate per-line color run requires
/// knowing the byte offset of the composer line in the rendered content. That
/// calculation is fragile in the raw-tile single-node model and would couple
/// `portal_node` to the exact output of `portal_markdown`. For Phase-1, the
/// at-capacity indicator is therefore expressed as a zero-length sentinel run
/// at byte 0 carrying the token color rather than a precise line-level run.
/// Callers can check that `color_runs` is non-empty and inspect the color to
/// detect the at-capacity state. The text-visible `[!]` prefix in
/// `composer_line` additionally signals the state for environments where
/// color runs are not inspected.
///
/// The zero-length run (`start_byte == end_byte == 0`) has no pixel coverage;
/// `from_text_markdown_node` treats it as a pure sentinel and does **not**
/// suppress Markdown stripping for it (only non-empty runs require that).
///
/// Promotion-era structured multi-node layout (one node per surface part) will
/// allow a precise, isolated composer-region color run without this limitation.
fn composer_color_runs(
    state: &ProjectedPortalState,
    composer_display: Option<&ComposerDisplayState>,
    at_capacity_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    if state.presentation != ProjectedPortalPresentation::Expanded {
        return Vec::new();
    }
    let Some(display) = composer_display else {
        return Vec::new();
    };
    if !display.at_capacity {
        return Vec::new();
    }
    // Zero-length sentinel run carrying the token-derived at-capacity color.
    // start_byte == end_byte == 0 → no pixel coloring by the compositor;
    // presence of this run + its color value is the machine-readable signal.
    vec![proto::TextColorRunProto {
        start_byte: 0,
        end_byte: 0,
        color: Some(at_capacity_color),
    }]
}

/// Build a `TextColorRunProto` for the disconnect/stale marker.
///
/// When the portal is connection-degraded, emits a single zero-length sentinel
/// run (`[0..0]`) carrying `stale_marker_color`. This mirrors the Phase-1
/// at-capacity sentinel mechanism (`composer_color_runs`): the run has no pixel
/// coverage; its presence + color is the machine-readable signal that the
/// token-driven stale marker is active. Precise per-line coloring is deferred to
/// the promotion-era structured layout. Returns an empty vec when not degraded.
///
/// §2: the marker color is token-driven — no literal color in the render path.
fn stale_marker_color_runs(
    state: &ProjectedPortalState,
    stale_marker_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    if !is_connection_degraded(state) {
        return Vec::new();
    }
    vec![proto::TextColorRunProto {
        start_byte: 0,
        end_byte: 0,
        color: Some(stale_marker_color),
    }]
}

/// Ambient one-line unread-output-count affordance, e.g. `"3 unread"`.
///
/// Returns `None` when the count is redacted (`unread_output_count == None`,
/// gated by the authority's `reveal_unread` policy) or when there is nothing
/// unread (`Some(0)`) — a presence engine renders nothing rather than a "0
/// unread" line. Never a loud/alarming marker: just a quiet count, styled
/// separately via `unread_indicator_color_runs` (§6.1: token-resolved, never
/// hardcoded).
fn unread_indicator_line(unread_output_count: Option<usize>) -> Option<String> {
    match unread_output_count {
        Some(count) if count > 0 => Some(format!("{count} unread")),
        _ => None,
    }
}

/// Build a `TextColorRunProto` for the ambient unread-output-count indicator.
///
/// Mirrors `stale_marker_color_runs`/`composer_color_runs`: a zero-length
/// sentinel run (`[0..0]`) carrying the token-resolved `unread_indicator_color`
/// so the token drives the signal without any literal color in the render path
/// (§6.1). Empty when there is nothing unread or the count is redacted —
/// matching `unread_indicator_line`'s gating exactly.
fn unread_indicator_color_runs(
    state: &ProjectedPortalState,
    unread_indicator_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    match state.unread_output_count {
        Some(count) if count > 0 => vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(unread_indicator_color),
        }],
        _ => Vec::new(),
    }
}

/// Index into `visible_transcript` **before** which the in-transcript unread
/// divider is inserted: the oldest retained *unseen agent-authored* turn
/// (hud-g1ena.2, §Unread Divider and Ambient Unread Count).
///
/// Returns `None` when there is nothing unread visible to this viewer
/// (`visible_unread_output_count` is `None` under the `reveal_unread` redaction
/// gate, or `Some(0)`), or when the retained window holds no agent-authored unit —
/// in those cases no divider renders, which is exactly how the divider **clears
/// locally when the viewer views the tail**: the authority resets the count to `0`
/// on tail view, so this yields `None` with no adapter round trip.
///
/// The count driving placement is `visible_unread_output_count` — the number of
/// unread units that survive this viewer's clearance filter — **not** the
/// aggregate `unread_output_count`. A higher-classification unread turn filtered
/// out of `visible_transcript` must not push the divider onto an already-seen
/// visible turn; the aggregate would over-count the units below the divider and do
/// exactly that. The ambient count near the header still shows the aggregate,
/// which MAY legitimately exceed the units below the divider.
///
/// The boundary is found by walking the retained window from the tail and
/// counting only agent-authored units — **echoed viewer turns are never unread**
/// per the Viewer Reply Echo requirement, so `OutputKind::Viewer` units are
/// skipped. When the (visible) unread count exceeds the agent-authored units still
/// retained in the bounded window (older unread turns coalesced or scrolled out),
/// the walk exhausts and the divider sits at the **oldest retained unseen unit** —
/// the first retained agent-authored turn — matching the spec's "the count MAY
/// exceed the units visibly below the divider".
fn unread_divider_boundary(state: &ProjectedPortalState) -> Option<usize> {
    let count = match state.visible_unread_output_count {
        Some(count) if count > 0 => count,
        _ => return None,
    };
    let mut agent_seen = 0usize;
    let mut boundary = None;
    for (index, unit) in state.visible_transcript.iter().enumerate().rev() {
        if unit.output_kind != OutputKind::Viewer {
            agent_seen += 1;
            boundary = Some(index);
            if agent_seen == count {
                break;
            }
        }
    }
    boundary
}

/// Build the token-styled sentinel color run for the in-transcript unread divider
/// (hud-g1ena.2, §Unread Divider and Ambient Unread Count).
///
/// Mirrors the other Phase-1 sentinels (`unread_indicator_color_runs`,
/// `stale_marker_color_runs`): a zero-length run (`[0..0]`) carrying the
/// token-resolved `unread_divider_color` so the visual token drives the divider
/// treatment without any literal color in the render path (§6.1). Emitted exactly
/// when the divider line is actually rendered — an Expanded portal with a
/// non-empty retained transcript whose retained window contains an unread
/// agent-authored turn (`unread_divider_boundary` is `Some`). Empty otherwise, so
/// it disappears the moment the count clears on tail view.
fn unread_divider_color_runs(
    state: &ProjectedPortalState,
    unread_divider_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    if state.presentation == ProjectedPortalPresentation::Expanded
        && !state.visible_transcript.is_empty()
        && unread_divider_boundary(state).is_some()
    {
        vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(unread_divider_color),
        }]
    } else {
        Vec::new()
    }
}

/// `true` when the most recently appended visible transcript unit is a
/// question awaiting a viewer reply (`expects_reply == true`, hud-jip0k).
///
/// Gating on `visible_transcript.last()` gets two things for free:
/// - **Redaction**: a restricted viewer's `visible_transcript` is already
///   emptied by the authority (`projected_portal_state`), so this returns
///   `false` without any separate redaction check.
/// - **Answered-question clearing**: a viewer's own echoed reply
///   (`OutputKind::Viewer`) is appended with `expects_reply == false`
///   (`submit_portal_input`), so once the viewer responds it becomes the new
///   last unit and the cue clears — no separate "answered" bookkeeping needed.
fn awaiting_reply(state: &ProjectedPortalState) -> bool {
    state
        .visible_transcript
        .last()
        .is_some_and(|unit| unit.expects_reply)
}

/// Build a `TextColorRunProto` for the ambient awaiting-reply (question)
/// indicator.
///
/// Mirrors `unread_indicator_color_runs`/`stale_marker_color_runs`: a
/// zero-length sentinel run (`[0..0]`) carrying the token-resolved
/// `awaiting_reply_color` so the token drives the signal without any literal
/// color in the render path (§6.1). Empty when nothing is awaiting reply.
fn awaiting_reply_color_runs(
    state: &ProjectedPortalState,
    awaiting_reply_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    if awaiting_reply(state) {
        vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(awaiting_reply_color),
        }]
    } else {
        Vec::new()
    }
}

/// Map a published `ProjectionLifecycleState` onto its ambient affordance accent.
///
/// The seven contract variants collapse into four viewer-facing semantic groups,
/// each with its own token-resolved accent (no literal color here — §6.1):
///
/// | Lifecycle state | Group | Accent token |
/// |---|---|---|
/// | `Active` | actively working | `lifecycle_active_color` |
/// | `Attached` | attached / ready | `lifecycle_attached_color` |
/// | `Degraded`, `HudUnavailable` | needs attention | `lifecycle_attention_color` |
/// | `Detached`, `CleanupPending`, `Expired` | winding down / gone | `lifecycle_inactive_color` |
///
/// The grouping keeps the palette tasteful and ambient (the cooperative-projection
/// and text-stream-portal specs forbid self-escalating interruption class); the
/// exact lifecycle spelling still rides the redaction-gated `status:` text line.
/// Ambient glyph for the lifecycle status line, grouped to match the accent
/// categories in [`lifecycle_accent_color`]. Content-free: conveys session
/// state only, never identity or transcript content. The glyphs are deliberately
/// quiet (no `!`/⚠) so the affordance stays ambient and never self-escalates
/// interruption class (text-stream-portals / cooperative-hud-projection specs).
fn lifecycle_glyph(lifecycle: ProjectionLifecycleState) -> char {
    match lifecycle {
        ProjectionLifecycleState::Active => '◆',
        ProjectionLifecycleState::Attached => '◇',
        ProjectionLifecycleState::Degraded | ProjectionLifecycleState::HudUnavailable => '◈',
        ProjectionLifecycleState::Detached
        | ProjectionLifecycleState::CleanupPending
        | ProjectionLifecycleState::Expired => '○',
    }
}

fn lifecycle_accent_color(
    lifecycle: ProjectionLifecycleState,
    tokens: &PortalVisualTokens,
) -> proto::Rgba {
    match lifecycle {
        ProjectionLifecycleState::Active => tokens.lifecycle_active_color,
        ProjectionLifecycleState::Attached => tokens.lifecycle_attached_color,
        ProjectionLifecycleState::Degraded | ProjectionLifecycleState::HudUnavailable => {
            tokens.lifecycle_attention_color
        }
        ProjectionLifecycleState::Detached
        | ProjectionLifecycleState::CleanupPending
        | ProjectionLifecycleState::Expired => tokens.lifecycle_inactive_color,
    }
}

/// Build the lifecycle-affordance color run for the portal node.
///
/// When the viewer is permitted to see lifecycle state (`state.lifecycle_state`
/// is `Some` — the authority sets it to `None` under redaction), emits a single
/// zero-length sentinel run (`[0..0]`) carrying the token-resolved accent for the
/// current lifecycle group. This mirrors the Phase-1 `stale_marker_color_runs` /
/// `composer_color_runs` mechanism: the run has no pixel coverage (a non-empty
/// run would suppress Markdown stripping for the whole single-node portal); its
/// presence + color is the machine-readable, token-driven signal that the viewer
/// affordance is active, while the text-visible `status:` line carries the exact
/// spelling. Precise per-line coloring is deferred to the promotion-era
/// structured multi-node layout.
///
/// Returns an empty vec when lifecycle state is redacted/absent — so a restricted
/// viewer gets no lifecycle affordance, exactly like the redaction-gated
/// `status:` line.
fn lifecycle_marker_color_runs(
    state: &ProjectedPortalState,
    tokens: &PortalVisualTokens,
) -> Vec<proto::TextColorRunProto> {
    let Some(lifecycle) = state.lifecycle_state else {
        return Vec::new();
    };
    vec![proto::TextColorRunProto {
        start_byte: 0,
        end_byte: 0,
        color: Some(lifecycle_accent_color(lifecycle, tokens)),
    }]
}

/// Render the friendly, token-styled empty/first-run portal body (hud-g1ena.6,
/// §First-Run Empty Portal Treatment) shown when the retained transcript window
/// is empty. Replaces the literal `<empty projection stream>` placeholder.
///
/// Three cases, in precedence order:
/// 1. **Connecting takes precedence** (§Connecting State Distinction): an
///    attached-but-never-connected portal (`!has_ever_connected`) shows the
///    distinct connecting line (`PORTAL_CONNECTING_LINE`) — never the "ready"
///    invite — so a starting-up portal never reads as a ready-and-idle empty
///    state. Its distinct pending glyph + token-resolved `connecting_marker_color`
///    (via `connecting_color_runs`) keep it visually distinct from the degraded
///    treatment, so a starting-up portal never reads as failing (hud-g1ena.7).
///    Connecting is content-free, so it also precedes the redacted placeholder.
/// 2. **Redacted**: a restricted viewer's `visible_transcript` is emptied
///    upstream, so this path is reached for them too. Under redaction identity
///    and inviting copy are suppressed — only a content-free placeholder shows.
/// 3. **Connected + empty**: the inviting ready line. Identity is carried by the
///    portal header (`**title**`), so this stays a single, uncluttered line.
///
/// Yields immediately to real content: the caller only renders this when
/// `visible_transcript.is_empty()`, so the first appended unit replaces it.
fn empty_portal_markdown(state: &ProjectedPortalState) -> &'static str {
    if !state.has_ever_connected {
        return PORTAL_CONNECTING_LINE;
    }
    if state.redacted {
        return PORTAL_EMPTY_REDACTED_LINE;
    }
    PORTAL_EMPTY_READY_LINE
}

/// Build the token-styled sentinel color run for the first-run empty-state body.
///
/// Mirrors the other Phase-1 sentinels (`stale_marker_color_runs`,
/// `unread_indicator_color_runs`): a zero-length run (`[0..0]`) carrying the
/// token-resolved `empty_state_color` so the visual token drives the treatment
/// without any literal color in the render path (§6.1). Emitted only when the
/// empty-state body is actually rendered — an Expanded portal with an empty
/// retained transcript that has connected. The connecting case
/// (`!has_ever_connected`) is intentionally excluded: it carries its own distinct
/// token via `connecting_color_runs` (hud-g1ena.7).
fn empty_state_color_runs(
    state: &ProjectedPortalState,
    empty_state_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    if state.presentation == ProjectedPortalPresentation::Expanded
        && state.visible_transcript.is_empty()
        && state.has_ever_connected
    {
        vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(empty_state_color),
        }]
    } else {
        Vec::new()
    }
}

/// Build the token-styled sentinel color run for the connecting-state body
/// (portal-chat-grade-affordances §Connecting State Distinction, hud-g1ena.7).
///
/// The mirror image of `empty_state_color_runs`: emitted exactly when the
/// connecting line (`PORTAL_CONNECTING_LINE`) is rendered — an Expanded portal
/// whose retained transcript is empty AND that has never connected
/// (`!has_ever_connected`). Carries the token-resolved `connecting_marker_color`
/// as a zero-length sentinel run so the connecting hue is token-driven, never a
/// literal in the render path (§6.1). Because this fires precisely when
/// `empty_state_color_runs` does NOT (the `has_ever_connected` gate is inverted),
/// the two treatments are mutually exclusive — a portal is either connecting or
/// empty-ready, never both.
///
/// Redaction-independent, like the connecting line itself and the stale marker:
/// the connecting hue reveals only connection state, no identity or content, so
/// it is emitted even for a restricted viewer.
fn connecting_color_runs(
    state: &ProjectedPortalState,
    connecting_marker_color: proto::Rgba,
) -> Vec<proto::TextColorRunProto> {
    if state.presentation == ProjectedPortalPresentation::Expanded
        && state.visible_transcript.is_empty()
        && !state.has_ever_connected
    {
        vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(connecting_marker_color),
        }]
    } else {
        Vec::new()
    }
}

/// Build the token-styled sentinel color run for the ambient header activity cue
/// (hud-g1ena.5, §Agent Activity and Streaming Cue).
///
/// Mirrors the other Phase-1 sentinels (`stale_marker_color_runs`,
/// `awaiting_reply_color_runs`): a zero-length run (`[0..0]`) carrying the
/// token-resolved `activity_cue_color` so the token drives the header cue without
/// any literal color in the render path (§6.1). Emitted exactly when
/// [`agent_activity_active`] holds — the agent is actively appending on a live
/// expanded portal — matching the `PORTAL_ACTIVITY_MARKER_LINE` header text so
/// the text-visible and machine-readable signals agree. Absent (empty vec)
/// otherwise, so it quiesces with the append cadence and suppresses under
/// redaction exactly like the header line.
fn activity_cue_color_runs(
    state: &ProjectedPortalState,
    activity_cue_color: proto::Rgba,
    now_wall_us: u64,
) -> Vec<proto::TextColorRunProto> {
    if agent_activity_active(state, now_wall_us) {
        vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(activity_cue_color),
        }]
    } else {
        Vec::new()
    }
}

/// Build the token-styled sentinel color run for the ambient transcript-tail
/// streaming cursor (hud-g1ena.5, §Agent Activity and Streaming Cue).
///
/// The mirror of `activity_cue_color_runs` for the tail cursor: a zero-length
/// sentinel run (`[0..0]`) carrying the token-resolved `streaming_cursor_color`,
/// gated by the same [`agent_activity_active`] predicate that appends the
/// `PORTAL_STREAMING_CURSOR_GLYPH` to the transcript body. Kept a distinct run
/// (and token) from the header cue so a profile can style the tail cursor apart,
/// while both quiesce together when appends stop.
///
/// ## Phase-1 scope note
///
/// Precise per-glyph coloring of the tail cursor requires the byte offset of the
/// appended glyph in the single-node content, which is fragile in the raw-tile
/// model (see `composer_color_runs`). For Phase-1 the cursor is a zero-length
/// sentinel at byte 0 carrying the token color; the text-visible glyph marks the
/// tail position. Promotion-era structured multi-node layout will position the
/// cursor precisely.
fn streaming_cursor_color_runs(
    state: &ProjectedPortalState,
    streaming_cursor_color: proto::Rgba,
    now_wall_us: u64,
) -> Vec<proto::TextColorRunProto> {
    if agent_activity_active(state, now_wall_us) {
        vec![proto::TextColorRunProto {
            start_byte: 0,
            end_byte: 0,
            color: Some(streaming_cursor_color),
        }]
    } else {
        Vec::new()
    }
}

/// Render the retained transcript window to markdown, inserting the in-transcript
/// unread divider (hud-g1ena.2) before the oldest retained unseen agent-authored
/// turn when `unread_boundary` names its index.
///
/// The unread divider is a distinct labeled marker (`PORTAL_UNREAD_DIVIDER_LINE`),
/// not a bare `---` thematic break, so it reads as the unread boundary rather than
/// an ordinary turn separator (the compositor styles all `---` breaks with the
/// single global `portal.divider.color`, so a bare break could not be told apart
/// in this single-node markdown path). Its token-resolved color rides a separate
/// zero-length sentinel (`unread_divider_color_runs`).
fn visible_transcript_markdown(units: &[TranscriptUnit], unread_boundary: Option<usize>) -> String {
    let mut result = String::new();
    for (index, unit) in units.iter().enumerate() {
        if index > 0 {
            // Turn separator between adjacent transcript entries (hud-nx7yq.4): a
            // thematic break on its own line, which the compositor renders as a
            // token-styled divider (`portal.divider.color`). This re-encodes the
            // `Vec<TranscriptUnit>` boundary — lost in the single-`\n` flatten —
            // so the history reads as discrete turns. Content-free geometry: under
            // redaction `units` is emptied upstream, so no separators are emitted.
            result.push_str("\n---\n");
        }
        if unread_boundary == Some(index) {
            // Unread divider before the oldest retained unseen agent-authored turn.
            // A quiet labeled marker (not a bare `---`) so it is distinguishable
            // from the ordinary turn separator above. Own line, then the turn text.
            result.push_str(PORTAL_UNREAD_DIVIDER_LINE);
            result.push('\n');
        }
        result.push_str(clamp_utf8(&unit.output_text, 4_096));
    }
    result
}

fn clamp_one_line(text: &str, max_bytes: usize) -> String {
    clamp_utf8(text.lines().next().unwrap_or_default(), max_bytes).to_string()
}

fn clamp_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

fn truncate_utf8(mut text: String, max_bytes: usize) -> String {
    let cut = clamp_utf8(&text, max_bytes).len();
    text.truncate(cut);
    text
}

fn push_line(result: &mut String, line: &str) {
    if !result.is_empty() {
        result.push('\n');
    }
    result.push_str(line);
}

fn sample_budget(started: Instant, budget_us: u64) -> ResidentGrpcBudgetSample {
    ResidentGrpcBudgetSample {
        elapsed_us: started.elapsed().as_micros() as u64,
        budget_us,
    }
}

fn new_scene_id_bytes() -> Vec<u8> {
    uuid::Uuid::now_v7().as_bytes().to_vec()
}

// ── Token bridge (tze_hud_config feature) ─────────────────────────────────────

/// Convert a fully-resolved `PortalPartTokens` (from `tze_hud_config`) into
/// the Phase-1 pilot's `PortalVisualTokens`.
///
/// This is the **canonical production-path conversion**. Any code that constructs
/// a `ResidentGrpcPortalAdapter` MUST use this function instead of
/// hand-constructing `PortalVisualTokens`. When a token-map swap occurs (e.g.
/// profile hot-reload), call `tze_hud_config::resolve_portal_tokens` on the new
/// `DesignTokenMap` to get `PortalPartTokens`, pass the result here, and forward
/// the `PortalVisualTokens` to `adapter.set_visual_tokens(...)`.
///
/// Maps transcript, collapsed, and composer parts. The remaining parts of
/// `PortalPartTokens` (frame, header, divider, transitions) require a structured
/// multi-node layout deferred to promotion-era work.
///
/// ## Usage
///
/// ```rust,ignore
/// use tze_hud_config::{resolve_portal_tokens, tokens::DesignTokenMap};
/// use tze_hud_projection::resident_grpc::{
///     ResidentGrpcPortalAdapter, ResidentGrpcPortalConfig,
///     portal_visual_tokens_from_part_tokens,
/// };
///
/// // At adapter construction:
/// let part_tokens = resolve_portal_tokens(&resolved_token_map);
/// let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);
/// let adapter = ResidentGrpcPortalAdapter::with_tokens(config, visual_tokens);
///
/// // On profile hot-reload / token-map swap:
/// let new_part_tokens = resolve_portal_tokens(&new_token_map);
/// adapter.set_visual_tokens(portal_visual_tokens_from_part_tokens(&new_part_tokens));
/// ```
pub fn portal_visual_tokens_from_part_tokens(
    part: &tze_hud_config::PortalPartTokens,
) -> PortalVisualTokens {
    PortalVisualTokens {
        transcript_background: proto::Rgba {
            r: part.transcript_background.r,
            g: part.transcript_background.g,
            b: part.transcript_background.b,
            a: part.transcript_background.a,
        },
        transcript_text_color: proto::Rgba {
            r: part.transcript_text_color.r,
            g: part.transcript_text_color.g,
            b: part.transcript_text_color.b,
            a: part.transcript_text_color.a,
        },
        transcript_font_size_px: part.transcript_font_size_px,
        transcript_dim_text_color: proto::Rgba {
            r: part.transcript_dim_text_color.r,
            g: part.transcript_dim_text_color.g,
            b: part.transcript_dim_text_color.b,
            a: part.transcript_dim_text_color.a,
        },
        transcript_dim_background: proto::Rgba {
            r: part.transcript_dim_background.r,
            g: part.transcript_dim_background.g,
            b: part.transcript_dim_background.b,
            a: part.transcript_dim_background.a,
        },
        stale_marker_color: proto::Rgba {
            r: part.stale_marker_color.r,
            g: part.stale_marker_color.g,
            b: part.stale_marker_color.b,
            a: part.stale_marker_color.a,
        },
        unread_indicator_color: proto::Rgba {
            r: part.unread_indicator_color.r,
            g: part.unread_indicator_color.g,
            b: part.unread_indicator_color.b,
            a: part.unread_indicator_color.a,
        },
        unread_divider_color: proto::Rgba {
            r: part.unread_divider_color.r,
            g: part.unread_divider_color.g,
            b: part.unread_divider_color.b,
            a: part.unread_divider_color.a,
        },
        awaiting_reply_color: proto::Rgba {
            r: part.awaiting_reply_color.r,
            g: part.awaiting_reply_color.g,
            b: part.awaiting_reply_color.b,
            a: part.awaiting_reply_color.a,
        },
        empty_state_color: proto::Rgba {
            r: part.empty_state_color.r,
            g: part.empty_state_color.g,
            b: part.empty_state_color.b,
            a: part.empty_state_color.a,
        },
        connecting_marker_color: proto::Rgba {
            r: part.connecting_marker_color.r,
            g: part.connecting_marker_color.g,
            b: part.connecting_marker_color.b,
            a: part.connecting_marker_color.a,
        },
        activity_cue_color: proto::Rgba {
            r: part.activity_cue_color.r,
            g: part.activity_cue_color.g,
            b: part.activity_cue_color.b,
            a: part.activity_cue_color.a,
        },
        streaming_cursor_color: proto::Rgba {
            r: part.streaming_cursor_color.r,
            g: part.streaming_cursor_color.g,
            b: part.streaming_cursor_color.b,
            a: part.streaming_cursor_color.a,
        },
        lifecycle_active_color: proto::Rgba {
            r: part.lifecycle_active_color.r,
            g: part.lifecycle_active_color.g,
            b: part.lifecycle_active_color.b,
            a: part.lifecycle_active_color.a,
        },
        lifecycle_attached_color: proto::Rgba {
            r: part.lifecycle_attached_color.r,
            g: part.lifecycle_attached_color.g,
            b: part.lifecycle_attached_color.b,
            a: part.lifecycle_attached_color.a,
        },
        lifecycle_attention_color: proto::Rgba {
            r: part.lifecycle_attention_color.r,
            g: part.lifecycle_attention_color.g,
            b: part.lifecycle_attention_color.b,
            a: part.lifecycle_attention_color.a,
        },
        lifecycle_inactive_color: proto::Rgba {
            r: part.lifecycle_inactive_color.r,
            g: part.lifecycle_inactive_color.g,
            b: part.lifecycle_inactive_color.b,
            a: part.lifecycle_inactive_color.a,
        },
        lifecycle_accent_width_px: part.lifecycle_accent_width_px,
        collapsed_background: proto::Rgba {
            r: part.collapsed_background.r,
            g: part.collapsed_background.g,
            b: part.collapsed_background.b,
            a: part.collapsed_background.a,
        },
        collapsed_text_color: proto::Rgba {
            r: part.collapsed_text_color.r,
            g: part.collapsed_text_color.g,
            b: part.collapsed_text_color.b,
            a: part.collapsed_text_color.a,
        },
        collapsed_font_size_px: part.collapsed_font_size_px,
        composer_background: proto::Rgba {
            r: part.composer_background.r,
            g: part.composer_background.g,
            b: part.composer_background.b,
            a: part.composer_background.a,
        },
        composer_text_color: proto::Rgba {
            r: part.composer_text_color.r,
            g: part.composer_text_color.g,
            b: part.composer_text_color.b,
            a: part.composer_text_color.a,
        },
        composer_font_size_px: part.composer_font_size_px,
        composer_at_capacity_color: proto::Rgba {
            r: part.composer_at_capacity_color.r,
            g: part.composer_at_capacity_color.g,
            b: part.composer_at_capacity_color.b,
            a: part.composer_at_capacity_color.a,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContentClassification, OutputKind, ProjectedPortalAdapterFamily, ProjectedPortalAttention,
        ProjectedPortalLayer, ProjectedPortalPresentation, ProjectedPortalRuntimeAuthority,
        ProjectedPortalState, ProjectionLifecycleState,
    };

    /// Build a minimal interaction-enabled expanded portal state for adapter tests.
    fn make_expanded_interaction_state(portal_id: &str) -> ProjectedPortalState {
        ProjectedPortalState {
            projection_id: "test-proj-1".to_string(),
            portal_id: portal_id.to_string(),
            adapter_family: ProjectedPortalAdapterFamily::CooperativeProjection,
            runtime_authority: ProjectedPortalRuntimeAuthority::ResidentSessionLease,
            layer: ProjectedPortalLayer::Content,
            presentation: ProjectedPortalPresentation::Expanded,
            preserve_geometry: false,
            redacted: false,
            connection_degraded: false,
            // Live, interactive fixture: the session has connected (so the
            // first-run empty state, not the connecting placeholder, applies).
            has_ever_connected: true,
            interaction_enabled: true,
            attention: ProjectedPortalAttention::Ambient,
            provider_kind: None,
            display_name: Some("Test Session".to_string()),
            workspace_hint: None,
            repository_hint: None,
            icon_profile_hint: None,
            lifecycle_state: None,
            status_text: None,
            visible_transcript: vec![],
            visible_transcript_bytes: 0,
            unread_output_count: None,
            visible_unread_output_count: None,
            pending_input_count: None,
            pending_input_bytes: None,
            last_input_feedback: None,
            draft_batch: None,
            geometry_batch: None,
            resized_bounds: None,
        }
    }

    // ── Lifecycle affordance accent (hud-m48i0) ──────────────────────────────

    /// hud-m48i0 acceptance #1/#2/#3/#4/#5: a **non-interactive**
    /// lifecycle-visible portal paints its accent via the coalescible StateStream
    /// `SetTileLifecycleAccent` tile-update — NOT a per-republish `AddNode`:
    ///
    /// 1. each lifecycle group emits its distinct token-resolved accent color and
    ///    the token-resolved width (no literal visual constants);
    /// 2. the batch contains NO `AddNode` mutation (so `classify_inbound_batch`
    ///    keeps it StateStream — the hud-mzk74 regression cannot return);
    /// 3. the markdown node carries only zero-length sentinel `color_runs` (no
    ///    pixel-bearing runs), so the cached/styled markdown path is preserved
    ///    (#947 must not regress);
    /// 4. a redacted viewer (`lifecycle_state = None`) emits a *clearing* accent
    ///    mutation (color None / width 0) and still no `AddNode`.
    #[test]
    fn lifecycle_accent_rides_state_stream_tile_update_not_add_node() {
        fn accent_of(
            batch: &session_proto::MutationBatch,
        ) -> Option<proto::SetTileLifecycleAccentMutation> {
            batch.mutations.iter().find_map(|m| match &m.mutation {
                Some(proto::mutation_proto::Mutation::SetTileLifecycleAccent(a)) => Some(a.clone()),
                _ => None,
            })
        }
        fn has_add_node(batch: &session_proto::MutationBatch) -> bool {
            batch.mutations.iter().any(|m| {
                matches!(
                    &m.mutation,
                    Some(proto::mutation_proto::Mutation::AddNode(_))
                )
            })
        }
        fn markdown_of(batch: &session_proto::MutationBatch) -> proto::TextMarkdownNodeProto {
            for m in &batch.mutations {
                if let Some(proto::mutation_proto::Mutation::PublishToTile(p)) = &m.mutation {
                    if let Some(proto::node_proto::Data::TextMarkdown(tm)) =
                        p.node.as_ref().and_then(|n| n.data.as_ref())
                    {
                        return tm.clone();
                    }
                }
            }
            panic!("batch must publish a markdown portal node");
        }

        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let mut adapter = ResidentGrpcPortalAdapter::new(config);
        adapter.record_created_tile(vec![0u8; 16]);
        let tokens = adapter.visual_tokens().clone();

        let cases = [
            (
                ProjectionLifecycleState::Active,
                tokens.lifecycle_active_color,
            ),
            (
                ProjectionLifecycleState::Attached,
                tokens.lifecycle_attached_color,
            ),
            (
                ProjectionLifecycleState::Degraded,
                tokens.lifecycle_attention_color,
            ),
            (
                ProjectionLifecycleState::HudUnavailable,
                tokens.lifecycle_attention_color,
            ),
            (
                ProjectionLifecycleState::Detached,
                tokens.lifecycle_inactive_color,
            ),
            (
                ProjectionLifecycleState::CleanupPending,
                tokens.lifecycle_inactive_color,
            ),
            (
                ProjectionLifecycleState::Expired,
                tokens.lifecycle_inactive_color,
            ),
        ];
        for (lifecycle, expected) in cases {
            // Non-interactive: this is the portal class hud-mzk74 protected — it
            // must NOT be flipped Transactional by the accent.
            let mut state = make_expanded_interaction_state("portal-accent");
            state.interaction_enabled = false;
            state.lifecycle_state = Some(lifecycle);
            let batch = adapter
                .render_batch(&state, 0)
                .expect("render_batch must succeed");

            assert!(
                !has_add_node(&batch),
                "lifecycle {lifecycle:?}: non-interactive portal must emit NO AddNode \
                 (would flip the batch Transactional — hud-mzk74)"
            );

            let accent = accent_of(&batch).unwrap_or_else(|| {
                panic!("lifecycle {lifecycle:?} must emit a SetTileLifecycleAccent mutation")
            });
            assert_eq!(
                accent
                    .color
                    .expect("permitted lifecycle must carry a color"),
                expected,
                "lifecycle {lifecycle:?} accent must use its token-resolved color"
            );
            assert!(
                (accent.width_px - tokens.lifecycle_accent_width_px).abs() < 1e-4,
                "accent width must come from the token (no literal dimension)"
            );

            // #947 guard: the markdown node carries only zero-length sentinels.
            let tm = markdown_of(&batch);
            assert!(
                tm.color_runs.iter().all(|r| r.start_byte >= r.end_byte),
                "markdown node must carry only zero-length sentinels (no pixel runs) \
                 for lifecycle {lifecycle:?} — cached markdown path must be preserved"
            );
        }

        // The four affordance groups paint mutually distinct accents.
        let groups = [
            tokens.lifecycle_active_color,
            tokens.lifecycle_attached_color,
            tokens.lifecycle_attention_color,
            tokens.lifecycle_inactive_color,
        ];
        for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                assert_ne!(
                    groups[i], groups[j],
                    "lifecycle accent groups {i} and {j} must be visually distinct"
                );
            }
        }

        // Redaction-gated: lifecycle_state = None emits a CLEARING accent (no
        // color, zero width) and still no AddNode.
        let mut redacted = make_expanded_interaction_state("portal-accent-redacted");
        redacted.interaction_enabled = false;
        redacted.lifecycle_state = None;
        let batch = adapter
            .render_batch(&redacted, 0)
            .expect("render_batch must succeed");
        assert!(!has_add_node(&batch));
        let accent = accent_of(&batch).expect("a clearing accent mutation is still emitted");
        assert!(
            accent.color.is_none() && accent.width_px <= 0.0,
            "redacted lifecycle must clear the accent (no color / zero width)"
        );
    }

    // ── Composer hit region activation (hud-hxe91) ────────────────────────────

    /// render_batch must emit an AddNodeMutation with a HitRegionNodeProto
    /// carrying accepts_composer_input=true when interaction_enabled is true.
    /// This is the production path that unblocks is_composer_active() in
    /// wire-driven scenes.
    #[test]
    fn render_batch_emits_composer_hit_region_when_interaction_enabled() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let mut adapter = ResidentGrpcPortalAdapter::new(config);
        // Record a fake tile_id so render_batch doesn't return MissingPortalTile.
        adapter.record_created_tile(vec![0u8; 16]);

        let state = make_expanded_interaction_state("portal-composer-test");

        let batch = adapter
            .render_batch(&state, 0)
            .expect("render_batch must succeed with interaction_enabled");

        // Should be 4 mutations: PublishToTile, UpdateTileInputMode,
        // SetTileLifecycleAccent (always emitted, hud-m48i0), AddNode.
        assert_eq!(
            batch.mutations.len(),
            4,
            "interaction_enabled=true must produce PublishToTile + UpdateTileInputMode + \
             SetTileLifecycleAccent + AddNode (composer hit region)"
        );

        // The fourth mutation must be AddNode with accepts_composer_input=true.
        let add_node_mutation = &batch.mutations[3];
        match &add_node_mutation.mutation {
            Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(an)) => {
                assert_eq!(
                    an.parent_id.len(),
                    16,
                    "parent_id must be 16 bytes (big-endian UUID)"
                );
                let node = an.node.as_ref().expect("AddNode must carry a NodeProto");
                match &node.data {
                    Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(hr)) => {
                        assert!(
                            hr.accepts_composer_input,
                            "composer hit region must have accepts_composer_input=true"
                        );
                        assert!(
                            hr.accepts_focus,
                            "composer hit region must have accepts_focus=true"
                        );
                        // hud-v4k1h: click-to-focus requires accepts_pointer=true so
                        // SceneGraph::hit_test yields a NodeHit that process_with_focus
                        // can focus. A false here silently breaks pointer focus + typing.
                        assert!(
                            hr.accepts_pointer,
                            "composer hit region must have accepts_pointer=true (click-to-focus)"
                        );
                        assert!(
                            hr.interaction_id.contains("portal-composer-test"),
                            "interaction_id must contain the portal_id: got '{}'",
                            hr.interaction_id
                        );
                        assert!(
                            hr.interaction_id.ends_with("-composer"),
                            "interaction_id must end with '-composer': got '{}'",
                            hr.interaction_id
                        );
                    }
                    other => panic!("AddNode node data must be HitRegion, got {other:?}"),
                }
            }
            other => panic!("Fourth mutation must be AddNode (composer hit region), got {other:?}"),
        }
    }

    /// hud-v4k1h resize follow-up: once a portal has been resized, the rendered
    /// body + composer hit region must size to the durable resized bounds, not
    /// the fixed config bounds. Without this the body kept rendering at the
    /// config size while the runtime grew the tile, leaving an empty
    /// "shadow-body" region in the grown tile.
    #[test]
    fn render_batch_sizes_body_to_resized_bounds_when_present() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let mut adapter = ResidentGrpcPortalAdapter::new(config);
        adapter.record_created_tile(vec![0u8; 16]);

        // A portal grown taller than the default expanded height AND moved to a
        // non-config origin (e.g. left/top edge drag keeps the opposite edge
        // stationary, shifting x/y).
        let grown_h = DEFAULT_EXPANDED_H + 240.0;
        let moved_x = 120.0;
        let moved_y = 80.0;
        let mut state = make_expanded_interaction_state("portal-resize-test");
        state.resized_bounds = Some(crate::AdapterPortalRect::from_f32(
            moved_x,
            moved_y,
            DEFAULT_EXPANDED_W,
            grown_h,
        ));

        let batch = adapter
            .render_batch(&state, 0)
            .expect("render_batch must succeed with resized bounds");

        // The PublishToTile bounds (1st mutation) must carry the resized ORIGIN
        // and size, not the static config origin — otherwise the tile snaps back
        // to the configured position on the next publish.
        match &batch.mutations[0].mutation {
            Some(tze_hud_protocol::proto::mutation_proto::Mutation::PublishToTile(pt)) => {
                let b = pt.bounds.as_ref().expect("PublishToTile must carry bounds");
                assert_eq!(b.x, moved_x, "tile bounds must keep the resized x origin");
                assert_eq!(b.y, moved_y, "tile bounds must keep the resized y origin");
                assert_eq!(b.height, grown_h, "tile bounds must use the resized height");
            }
            other => panic!("First mutation must be PublishToTile, got {other:?}"),
        }

        // The composer hit region (4th mutation) must cover the grown height —
        // proving the body bounds followed the resize, not the config size.
        let add_node_mutation = &batch.mutations[3];
        match &add_node_mutation.mutation {
            Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(an)) => {
                let node = an.node.as_ref().expect("AddNode must carry a NodeProto");
                match &node.data {
                    Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(hr)) => {
                        let bounds = hr.bounds.as_ref().expect("composer must carry bounds");
                        assert_eq!(
                            bounds.height, grown_h,
                            "composer/body must size to the resized height {grown_h}, got {}",
                            bounds.height
                        );
                    }
                    other => panic!("AddNode node data must be HitRegion, got {other:?}"),
                }
            }
            other => panic!("Fourth mutation must be AddNode (composer hit region), got {other:?}"),
        }

        // Sanity: an Expanded state with NO resized_bounds still uses config height.
        let mut plain = make_expanded_interaction_state("portal-noresize-test");
        plain.resized_bounds = None;
        let plain_batch = adapter
            .render_batch(&plain, 0)
            .expect("render_batch must succeed without resized bounds");
        if let Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(an)) =
            &plain_batch.mutations[3].mutation
        {
            let hr = match &an.node.as_ref().unwrap().data {
                Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(hr)) => hr,
                other => panic!("expected HitRegion, got {other:?}"),
            };
            assert_eq!(
                hr.bounds.as_ref().unwrap().height,
                DEFAULT_EXPANDED_H,
                "without resized_bounds the body must keep the config height"
            );
        } else {
            panic!("expected AddNode composer mutation");
        }
    }

    /// When interaction_enabled is false, render_batch must NOT emit an
    /// AddNodeMutation for the composer hit region.
    #[test]
    fn render_batch_does_not_emit_composer_hit_region_when_interaction_disabled() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let mut adapter = ResidentGrpcPortalAdapter::new(config);
        adapter.record_created_tile(vec![0u8; 16]);

        let mut state = make_expanded_interaction_state("portal-no-input");
        state.interaction_enabled = false;

        let batch = adapter
            .render_batch(&state, 0)
            .expect("render_batch must succeed");

        // Should be 3 mutations: PublishToTile + UpdateTileInputMode +
        // SetTileLifecycleAccent (always emitted, hud-m48i0). No AddNode.
        assert_eq!(
            batch.mutations.len(),
            3,
            "interaction_enabled=false must produce exactly 3 mutations \
             (PublishToTile + UpdateTileInputMode + SetTileLifecycleAccent, no AddNode)"
        );
    }

    /// The portal root node must carry an explicit (non-empty) ID so the
    /// AddNodeMutation can reference it as parent_id in the same batch.
    #[test]
    fn portal_node_carries_explicit_root_id() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let mut adapter = ResidentGrpcPortalAdapter::new(config);
        adapter.record_created_tile(vec![0u8; 16]);

        let state = make_expanded_interaction_state("portal-root-id");

        let batch = adapter
            .render_batch(&state, 0)
            .expect("render_batch must succeed");

        // The PublishToTile mutation must carry a NodeProto with a non-empty ID.
        match &batch.mutations[0].mutation {
            Some(tze_hud_protocol::proto::mutation_proto::Mutation::PublishToTile(p)) => {
                let root = p.node.as_ref().expect("PublishToTile must carry a node");
                assert_eq!(
                    root.id.len(),
                    16,
                    "portal root NodeProto.id must be 16 bytes (explicit little-endian UUID)"
                );
                // The AddNodeMutation's parent_id must match the same UUID (different encoding).
                // Both are 16 bytes; the relationship is verified by the runtime.
                // Index 3: SetTileLifecycleAccent occupies index 2 (hud-m48i0).
                match &batch.mutations[3].mutation {
                    Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(an)) => {
                        assert_eq!(an.parent_id.len(), 16, "parent_id must be 16 bytes");
                        // Verify they're NOT byte-equal (they encode the same UUID
                        // in different endian orders).
                        assert_ne!(
                            root.id, an.parent_id,
                            "NodeProto.id (little-endian) and parent_id (big-endian) must differ \
                             for the same UUID — same UUID, different wire encodings"
                        );
                    }
                    other => panic!("Fourth mutation must be AddNode, got {other:?}"),
                }
            }
            other => panic!("First mutation must be PublishToTile, got {other:?}"),
        }
    }

    #[test]
    fn clamp_utf8_borrows_at_valid_character_boundary() {
        let text = "alpha éé omega";

        assert_eq!(clamp_utf8(text, text.len()), text);
        assert_eq!(clamp_utf8(text, 7), "alpha ");
    }

    #[test]
    fn visible_transcript_markdown_clamps_each_unit_without_collecting_lines() {
        let units = vec![
            TranscriptUnit {
                sequence: 1,
                output_text: "first".to_string(),
                output_kind: OutputKind::Assistant,
                content_classification: ContentClassification::Private,
                logical_unit_id: None,
                coalesce_key: None,
                expects_reply: false,
                appended_at_wall_us: 1,
            },
            TranscriptUnit {
                sequence: 2,
                output_text: "é".repeat(3_000),
                output_kind: OutputKind::Assistant,
                content_classification: ContentClassification::Private,
                logical_unit_id: None,
                coalesce_key: None,
                expects_reply: false,
                appended_at_wall_us: 2,
            },
        ];

        let markdown = visible_transcript_markdown(&units, None);

        // First entry, then a turn separator, then the clamped second entry.
        assert!(markdown.starts_with("first\n---\n"));
        assert!(markdown.is_char_boundary(markdown.len()));
        // Each unit is still clamped to 4096 bytes; the separator adds a bounded
        // constant between entries.
        assert!(markdown.len() <= "first".len() + "\n---\n".len() + 4_096);
    }

    // ── §2/§3 disconnect + stale-content treatment ───────────────────────────

    /// Extract the single `TextMarkdownNodeProto` produced by `portal_node`.
    fn text_markdown_node(node: &proto::NodeProto) -> &proto::TextMarkdownNodeProto {
        match node.data.as_ref().expect("node must carry data") {
            proto::node_proto::Data::TextMarkdown(tm) => tm,
            other => panic!("portal_node must produce TextMarkdown, got {other:?}"),
        }
    }

    /// §2: when the portal is connection-degraded, the expanded transcript
    /// surface uses the token-resolved DIM colors, not the live transcript
    /// colors. A control case with `connection_degraded = false` proves the live
    /// colors are used otherwise — so the difference is driven solely by the
    /// degraded flag, not by a hardcoded constant.
    #[test]
    fn degraded_state_uses_dim_transcript_colors() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let tokens = adapter.visual_tokens().clone();

        let mut live = make_expanded_interaction_state("portal-degraded-colors");
        live.connection_degraded = false;
        let live_node = adapter.portal_node(&live, vec![0u8; 16], 0);
        let live_tm = text_markdown_node(&live_node);
        assert_eq!(
            live_tm.color.unwrap(),
            tokens.transcript_text_color,
            "live portal must use the live transcript text color"
        );
        assert_eq!(
            live_tm.background.unwrap(),
            tokens.transcript_background,
            "live portal must use the live transcript background"
        );

        let mut degraded = make_expanded_interaction_state("portal-degraded-colors");
        degraded.connection_degraded = true;
        let degraded_node = adapter.portal_node(&degraded, vec![0u8; 16], 0);
        let degraded_tm = text_markdown_node(&degraded_node);
        assert_eq!(
            degraded_tm.color.unwrap(),
            tokens.transcript_dim_text_color,
            "§2: degraded portal must use the DIM transcript text token"
        );
        assert_eq!(
            degraded_tm.background.unwrap(),
            tokens.transcript_dim_background,
            "§2: degraded portal must use the DIM transcript background token"
        );
    }

    /// §2: a degraded portal emits a content-free disconnect marker line. The
    /// marker survives redaction (lifecycle_state = None, redacted = true) and
    /// reveals neither transcript content nor a lifecycle spelling.
    #[test]
    fn degraded_state_emits_content_free_disconnect_marker_under_redaction() {
        let mut state = make_expanded_interaction_state("portal-marker");
        state.connection_degraded = true;
        // Restricted viewer: identity/transcript/lifecycle all redacted.
        state.redacted = true;
        state.lifecycle_state = None;
        state.display_name = None;
        state.visible_transcript = vec![];

        let markdown = portal_markdown(&state, None, 0);

        assert!(
            markdown.contains(PORTAL_DISCONNECT_MARKER_LINE),
            "§2: disconnect marker must remain present under redaction"
        );
        // No lifecycle spelling leaks (the `status:` line is redaction-gated).
        assert!(
            !markdown.contains("status:"),
            "redaction-gated lifecycle spelling must NOT appear"
        );

        // A non-degraded state must NOT show the marker.
        let mut live = make_expanded_interaction_state("portal-marker");
        live.connection_degraded = false;
        let live_md = portal_markdown(&live, None, 0);
        assert!(
            !live_md.contains(PORTAL_DISCONNECT_MARKER_LINE),
            "live portal must not show the disconnect marker"
        );
    }

    /// §2: the stale marker color is carried as a token-driven sentinel color run
    /// when degraded (no literal color in the render path), and absent otherwise.
    #[test]
    fn degraded_state_emits_stale_marker_color_run() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let stale = adapter.visual_tokens().stale_marker_color;

        let mut degraded = make_expanded_interaction_state("portal-stale-run");
        degraded.connection_degraded = true;
        let runs = stale_marker_color_runs(&degraded, stale);
        assert_eq!(
            runs.len(),
            1,
            "degraded state must emit one stale color run"
        );
        assert_eq!(runs[0].color.unwrap(), stale, "run must carry stale token");
        assert_eq!(runs[0].start_byte, 0);
        assert_eq!(runs[0].end_byte, 0);

        let mut live = make_expanded_interaction_state("portal-stale-run");
        live.connection_degraded = false;
        assert!(
            stale_marker_color_runs(&live, stale).is_empty(),
            "live state must emit no stale color run"
        );
    }

    /// hud-meqet: `unread_output_count` was already threaded (un-nulled)
    /// through to `ProjectedPortalState` by the authority, but the render
    /// path never read it, so it was never drawn. A nonzero, visible count
    /// must emit an ambient text-item line ("N unread") plus a token-driven
    /// sentinel color run (mirroring the stale-marker/composer Phase-1
    /// convention); zero or redacted (`None`) must emit neither — a presence
    /// engine renders nothing rather than a "0 unread" line or a loud badge.
    #[test]
    fn unread_output_count_renders_ambient_indicator_only_when_nonzero() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let unread_color = adapter.visual_tokens().unread_indicator_color;

        let mut present = make_expanded_interaction_state("portal-badge");
        present.unread_output_count = Some(3);
        let markdown = portal_markdown(&present, None, 0);
        assert!(
            markdown.contains("3 unread"),
            "nonzero unread count must render an ambient text-item indicator: {markdown}"
        );
        let runs = unread_indicator_color_runs(&present, unread_color);
        assert_eq!(
            runs.len(),
            1,
            "nonzero unread count must emit one token-driven color run"
        );
        assert_eq!(
            runs[0].color.unwrap(),
            unread_color,
            "run must carry the unread_indicator_color token, never a literal color"
        );
        assert_eq!(runs[0].start_byte, 0);
        assert_eq!(runs[0].end_byte, 0);

        let mut zero = make_expanded_interaction_state("portal-badge");
        zero.unread_output_count = Some(0);
        let zero_markdown = portal_markdown(&zero, None, 0);
        assert!(
            !zero_markdown.contains("unread"),
            "zero unread count must render nothing: {zero_markdown}"
        );
        assert!(
            unread_indicator_color_runs(&zero, unread_color).is_empty(),
            "zero unread count must emit no color run"
        );

        let mut redacted = make_expanded_interaction_state("portal-badge");
        redacted.unread_output_count = None;
        let redacted_markdown = portal_markdown(&redacted, None, 0);
        assert!(
            !redacted_markdown.contains("unread"),
            "redacted (None) unread count must render nothing: {redacted_markdown}"
        );
        assert!(
            unread_indicator_color_runs(&redacted, unread_color).is_empty(),
            "redacted (None) unread count must emit no color run"
        );
    }

    /// The `unread_indicator_color` `PortalVisualTokens` field maps 1:1 from
    /// the source `PortalPartTokens` channel (single-source-of-truth
    /// invariant, same as the other token-mapping tests in this module).
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_unread_indicator_color() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(
            visual.unread_indicator_color.r,
            part.unread_indicator_color.r
        );
        assert_eq!(
            visual.unread_indicator_color.a,
            part.unread_indicator_color.a
        );
    }

    /// The `awaiting_reply_color` `PortalVisualTokens` field maps 1:1 from the
    /// source `PortalPartTokens` channel (hud-jip0k), same single-source-of-truth
    /// invariant as the other token-mapping tests in this module.
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_awaiting_reply_color() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(visual.awaiting_reply_color.r, part.awaiting_reply_color.r);
        assert_eq!(visual.awaiting_reply_color.a, part.awaiting_reply_color.a);
    }

    // ── Unread divider (hud-g1ena.2, §Unread Divider and Ambient Unread Count) ──

    /// A `TranscriptUnit` with explicit text + kind for divider-placement tests.
    fn transcript_unit_text(sequence: u64, output_kind: OutputKind, text: &str) -> TranscriptUnit {
        TranscriptUnit {
            sequence,
            output_text: text.to_string(),
            output_kind,
            content_classification: ContentClassification::Private,
            logical_unit_id: None,
            coalesce_key: None,
            expects_reply: false,
            appended_at_wall_us: sequence,
        }
    }

    /// No unread content → no divider boundary. Covers both the redacted /
    /// not-revealed count (`None`) and the nothing-unread count (`Some(0)`), which
    /// is exactly how the divider clears locally when the viewer views the tail.
    #[test]
    fn unread_divider_boundary_is_none_without_unread() {
        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![
            transcript_unit(1, OutputKind::Assistant, false),
            transcript_unit(2, OutputKind::Assistant, false),
        ];
        state.visible_unread_output_count = None;
        assert_eq!(unread_divider_boundary(&state), None);
        state.visible_unread_output_count = Some(0);
        assert_eq!(unread_divider_boundary(&state), None);
    }

    /// The divider sits before the oldest retained unseen agent-authored turn.
    #[test]
    fn unread_divider_boundary_marks_oldest_unseen_agent_turn() {
        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![
            transcript_unit(1, OutputKind::Assistant, false), // seen
            transcript_unit(2, OutputKind::Assistant, false), // unseen
            transcript_unit(3, OutputKind::Assistant, false), // unseen
        ];
        state.visible_unread_output_count = Some(2);
        assert_eq!(unread_divider_boundary(&state), Some(1));
    }

    /// Echoed viewer turns are never unread (Viewer Reply Echo): a viewer echo
    /// interleaved with agent turns is skipped when counting the boundary.
    #[test]
    fn unread_divider_boundary_skips_viewer_echo() {
        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![
            transcript_unit(1, OutputKind::Assistant, false),
            transcript_unit(2, OutputKind::Viewer, false),
            transcript_unit(3, OutputKind::Assistant, false),
        ];
        state.visible_unread_output_count = Some(1);
        // Only the newest agent turn (index 2) is unread; the viewer echo at
        // index 1 is skipped, so the divider does not land on it.
        assert_eq!(unread_divider_boundary(&state), Some(2));
    }

    /// When older unread turns have coalesced or scrolled out of the bounded
    /// window, the count MAY exceed the retained agent turns; the divider then
    /// sits at the oldest RETAINED unseen (agent) unit, not on a viewer echo.
    #[test]
    fn unread_divider_boundary_clamps_to_oldest_retained_when_count_exceeds_window() {
        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![
            transcript_unit(1, OutputKind::Viewer, false),
            transcript_unit(2, OutputKind::Assistant, false),
            transcript_unit(3, OutputKind::Assistant, false),
        ];
        state.visible_unread_output_count = Some(9);
        assert_eq!(unread_divider_boundary(&state), Some(1));
    }

    /// Only viewer echoes retained → nothing is ever unread, so no divider.
    #[test]
    fn unread_divider_boundary_is_none_without_agent_turn() {
        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![transcript_unit(1, OutputKind::Viewer, false)];
        state.visible_unread_output_count = Some(3);
        assert_eq!(unread_divider_boundary(&state), None);
    }

    /// The divider marker line is inserted before the boundary turn, distinct
    /// from the ordinary `---` turn separator, and never appears without a
    /// boundary.
    #[test]
    fn visible_transcript_markdown_inserts_unread_divider_before_boundary() {
        let units = vec![
            transcript_unit_text(1, OutputKind::Assistant, "alpha"),
            transcript_unit_text(2, OutputKind::Assistant, "bravo"),
        ];
        let md = visible_transcript_markdown(&units, Some(1));
        assert_eq!(
            md,
            format!("alpha\n---\n{PORTAL_UNREAD_DIVIDER_LINE}\nbravo")
        );
        let plain = visible_transcript_markdown(&units, None);
        assert!(!plain.contains(PORTAL_UNREAD_DIVIDER_LINE));
    }

    /// The divider color run is a zero-length token sentinel (§6.1: token-driven,
    /// no literal color) and clears when the count clears on tail view.
    #[test]
    fn unread_divider_color_runs_emit_zero_length_token_sentinel() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let divider_color = adapter.visual_tokens().unread_divider_color;

        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![
            transcript_unit(1, OutputKind::Assistant, false),
            transcript_unit(2, OutputKind::Assistant, false),
        ];
        state.visible_unread_output_count = Some(1);

        let runs = unread_divider_color_runs(&state, divider_color);
        assert_eq!(
            runs.len(),
            1,
            "an unread agent turn must emit one divider run"
        );
        assert_eq!(
            runs[0].start_byte, 0,
            "divider run must be a zero-length sentinel"
        );
        assert_eq!(
            runs[0].end_byte, 0,
            "divider run must be a zero-length sentinel"
        );
        assert_eq!(
            runs[0].color.unwrap(),
            divider_color,
            "run must carry the unread_divider_color token, never a literal color"
        );

        state.visible_unread_output_count = None;
        assert!(
            unread_divider_color_runs(&state, divider_color).is_empty(),
            "the divider run must clear when the count clears on tail view"
        );
    }

    /// The in-transcript divider is an expanded-only affordance and needs
    /// transcript content to divide: absent in collapsed and on an empty window.
    #[test]
    fn unread_divider_color_runs_absent_in_collapsed_and_empty() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let divider_color = adapter.visual_tokens().unread_divider_color;

        let mut collapsed = make_expanded_interaction_state("portal-unread");
        collapsed.presentation = ProjectedPortalPresentation::Collapsed;
        collapsed.visible_transcript = vec![transcript_unit(1, OutputKind::Assistant, false)];
        collapsed.visible_unread_output_count = Some(1);
        assert!(unread_divider_color_runs(&collapsed, divider_color).is_empty());

        let mut empty = make_expanded_interaction_state("portal-unread");
        empty.visible_unread_output_count = Some(1);
        assert!(unread_divider_color_runs(&empty, divider_color).is_empty());
    }

    /// End-to-end through `portal_markdown`: an unread agent turn renders the
    /// divider, and viewing the tail (count → `None`) clears it locally with no
    /// adapter round trip.
    #[test]
    fn portal_markdown_renders_unread_divider_and_clears_on_tail_view() {
        let mut state = make_expanded_interaction_state("portal-unread");
        state.visible_transcript = vec![
            transcript_unit_text(1, OutputKind::Assistant, "old turn"),
            transcript_unit_text(2, OutputKind::Assistant, "new turn"),
        ];
        // Non-redacted viewer: the aggregate and clearance-corrected counts match.
        state.unread_output_count = Some(1);
        state.visible_unread_output_count = Some(1);
        // Render past the agent-activity quiesce window (units appended at 1..=2)
        // so this divider assertion is isolated from the streaming-cursor cue.
        let now = PORTAL_ACTIVITY_QUIESCE_WINDOW_US + 10;
        let md = portal_markdown(&state, None, now);
        assert!(
            md.contains(PORTAL_UNREAD_DIVIDER_LINE),
            "an unread agent turn must render the in-transcript divider: {md}"
        );

        state.unread_output_count = None;
        state.visible_unread_output_count = None;
        let cleared = portal_markdown(&state, None, now);
        assert!(
            !cleared.contains(PORTAL_UNREAD_DIVIDER_LINE),
            "the divider must clear locally when the viewer views the tail: {cleared}"
        );
    }

    /// Regression (hud-g1ena.2 review): when the viewer's clearance filters some
    /// unread turns out of `visible_transcript`, the aggregate `unread_output_count`
    /// exceeds the unread units the viewer can actually see. The divider must be
    /// placed with the clearance-corrected `visible_unread_output_count`, never the
    /// aggregate — otherwise the walk exhausts against the aggregate and clamps the
    /// divider onto an already-seen visible turn, marking seen text as unread.
    #[test]
    fn unread_divider_boundary_uses_visible_count_not_aggregate() {
        let mut state = make_expanded_interaction_state("portal-unread");
        // Two agent turns survive clearance: an older SEEN turn and one unread
        // turn. Two further unread turns were higher-classification and filtered
        // out upstream, so they never reach `visible_transcript`.
        state.visible_transcript = vec![
            transcript_unit(1, OutputKind::Assistant, false), // seen, visible
            transcript_unit(4, OutputKind::Assistant, false), // unread, visible
        ];
        // Aggregate says 3 unread; only 1 of them is visible to this viewer.
        state.unread_output_count = Some(3);
        state.visible_unread_output_count = Some(1);
        // The divider sits before the single visible unread turn (index 1), NOT
        // clamped onto the seen turn at index 0 (which the aggregate count would do).
        assert_eq!(unread_divider_boundary(&state), Some(1));
    }

    /// The `unread_divider_color` `PortalVisualTokens` field maps 1:1 from the
    /// source `PortalPartTokens` channel and is distinct from the ambient unread
    /// count token so the two affordances can be reskinned separately.
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_unread_divider_color() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(visual.unread_divider_color.r, part.unread_divider_color.r);
        assert_eq!(visual.unread_divider_color.g, part.unread_divider_color.g);
        assert_eq!(visual.unread_divider_color.b, part.unread_divider_color.b);
        assert_eq!(visual.unread_divider_color.a, part.unread_divider_color.a);
        assert_ne!(
            visual.unread_divider_color, visual.unread_indicator_color,
            "unread divider and ambient count must be distinct tokens"
        );
    }

    /// Build a minimal `TranscriptUnit` for awaiting-reply tests, varying only
    /// `expects_reply` and `output_kind`.
    fn transcript_unit(
        sequence: u64,
        output_kind: OutputKind,
        expects_reply: bool,
    ) -> TranscriptUnit {
        TranscriptUnit {
            sequence,
            output_text: "example output".to_string(),
            output_kind,
            content_classification: ContentClassification::Private,
            logical_unit_id: None,
            coalesce_key: None,
            expects_reply,
            appended_at_wall_us: sequence,
        }
    }

    /// hud-jip0k acceptance: the ambient awaiting-reply cue — both the
    /// text-visible `"? awaiting reply"` line and the token-driven sentinel
    /// color run — is present exactly when the most recently published
    /// visible transcript unit has `expects_reply == true`, and ABSENT for
    /// every backward-compat case: no transcript, `expects_reply == false`
    /// (the default/omitted case), and once a viewer's echoed reply
    /// (`OutputKind::Viewer`, `expects_reply == false` by construction)
    /// becomes the new last unit.
    #[test]
    fn awaiting_reply_cue_gates_on_last_unit_expects_reply() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let awaiting_color = adapter.visual_tokens().awaiting_reply_color;

        // Backward-compat baseline: no transcript at all — absent, exactly
        // like every portal published before this field existed.
        let empty = make_expanded_interaction_state("portal-question");
        assert!(
            !portal_markdown(&empty, None, 0).contains("awaiting reply"),
            "no transcript must render no awaiting-reply cue"
        );
        assert!(
            awaiting_reply_color_runs(&empty, awaiting_color).is_empty(),
            "no transcript must emit no awaiting-reply color run"
        );

        // The default/omitted case: expects_reply == false on the last unit.
        let mut unset = make_expanded_interaction_state("portal-question");
        unset.visible_transcript = vec![transcript_unit(1, OutputKind::Assistant, false)];
        assert!(
            !portal_markdown(&unset, None, 0).contains("awaiting reply"),
            "expects_reply == false must render no awaiting-reply cue"
        );
        assert!(
            awaiting_reply_color_runs(&unset, awaiting_color).is_empty(),
            "expects_reply == false must emit no awaiting-reply color run"
        );

        // The opt-in case: expects_reply == true on the last unit.
        let mut question = make_expanded_interaction_state("portal-question");
        question.visible_transcript = vec![
            transcript_unit(1, OutputKind::Assistant, false),
            transcript_unit(2, OutputKind::Assistant, true),
        ];
        let markdown = portal_markdown(&question, None, 0);
        assert!(
            markdown.contains("? awaiting reply"),
            "expects_reply == true on the last unit must render the ambient cue: {markdown}"
        );
        let runs = awaiting_reply_color_runs(&question, awaiting_color);
        assert_eq!(
            runs.len(),
            1,
            "expects_reply == true must emit exactly one token-driven color run"
        );
        assert_eq!(
            runs[0].color.unwrap(),
            awaiting_color,
            "run must carry the awaiting_reply_color token, never a literal color"
        );
        assert_eq!(runs[0].start_byte, 0);
        assert_eq!(runs[0].end_byte, 0);

        // Answered: a viewer's echoed reply becomes the new last unit and
        // clears the cue, with no separate "answered" bookkeeping required.
        let mut answered = question.clone();
        answered
            .visible_transcript
            .push(transcript_unit(3, OutputKind::Viewer, false));
        assert!(
            !portal_markdown(&answered, None, 0).contains("awaiting reply"),
            "a viewer's echoed reply must clear the awaiting-reply cue"
        );
        assert!(
            awaiting_reply_color_runs(&answered, awaiting_color).is_empty(),
            "a viewer's echoed reply must clear the awaiting-reply color run"
        );

        // Redaction: visible_transcript is already emptied by the authority
        // for a restricted viewer, so the cue is absent with no separate check.
        let mut redacted = make_expanded_interaction_state("portal-question");
        redacted.redacted = true;
        redacted.visible_transcript = vec![];
        assert!(
            !portal_markdown(&redacted, None, 0).contains("awaiting reply"),
            "a redacted (empty transcript) viewer must render no awaiting-reply cue"
        );
    }

    /// hud-g1ena.5 acceptance: build a live expanded portal whose tail is a fresh
    /// agent turn and assert the ambient activity cue is fully present — the
    /// text-visible header marker, the tail streaming cursor glyph, and BOTH
    /// token-driven zero-length sentinel color runs (header + cursor). Then assert
    /// it QUIESCES once the newest append ages past the window: no marker, no
    /// cursor, no runs. Derivation is purely from the observed
    /// `appended_at_wall_us` vs the render `now` — no separate typing signal.
    #[test]
    fn agent_activity_cue_present_while_appending_and_quiesces_after_window() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let activity_color = adapter.visual_tokens().activity_cue_color;
        let cursor_color = adapter.visual_tokens().streaming_cursor_color;

        let appended_at = 1_000_000;
        let now_fresh = appended_at + 500_000; // within the quiesce window
        let now_quiesced = appended_at + PORTAL_ACTIVITY_QUIESCE_WINDOW_US + 1;

        let mut state = make_expanded_interaction_state("portal-activity");
        state.visible_transcript = vec![TranscriptUnit {
            appended_at_wall_us: appended_at,
            ..transcript_unit(1, OutputKind::Assistant, false)
        }];

        // Actively appending: header marker + tail cursor glyph + both runs.
        assert!(agent_activity_active(&state, now_fresh));
        let fresh_md = portal_markdown(&state, None, now_fresh);
        assert!(
            fresh_md.contains(PORTAL_ACTIVITY_MARKER_LINE),
            "active append must render the ambient header activity marker: {fresh_md}"
        );
        assert!(
            fresh_md.contains(PORTAL_STREAMING_CURSOR_GLYPH),
            "active append must render the tail streaming cursor glyph: {fresh_md}"
        );
        let header_runs = activity_cue_color_runs(&state, activity_color, now_fresh);
        let cursor_runs = streaming_cursor_color_runs(&state, cursor_color, now_fresh);
        assert_eq!(header_runs.len(), 1, "one activity header sentinel run");
        assert_eq!(cursor_runs.len(), 1, "one streaming cursor sentinel run");
        for (run, expected) in [
            (&header_runs[0], activity_color),
            (&cursor_runs[0], cursor_color),
        ] {
            assert_eq!(
                run.color.unwrap(),
                expected,
                "run carries token color, never a literal"
            );
            assert_eq!(run.start_byte, 0, "sentinel run is zero-length");
            assert_eq!(run.end_byte, 0, "sentinel run is zero-length");
        }

        // Quiesced: the same tail is now stale relative to `now`.
        assert!(!agent_activity_active(&state, now_quiesced));
        let quiesced_md = portal_markdown(&state, None, now_quiesced);
        assert!(
            !quiesced_md.contains(PORTAL_ACTIVITY_MARKER_LINE),
            "cue must quiesce once appends stop: {quiesced_md}"
        );
        assert!(
            !quiesced_md.contains(PORTAL_STREAMING_CURSOR_GLYPH),
            "cursor must quiesce once appends stop: {quiesced_md}"
        );
        assert!(activity_cue_color_runs(&state, activity_color, now_quiesced).is_empty());
        assert!(streaming_cursor_color_runs(&state, cursor_color, now_quiesced).is_empty());
    }

    /// hud-kbm80: `agent_activity_clear_deadline_us` is the time-independent
    /// factor of `agent_activity_active` — it returns the tail's clear-due
    /// deadline (`appended_at + PORTAL_ACTIVITY_QUIESCE_WINDOW_US`) exactly when a
    /// cue is present, and `agent_activity_active(state, now)` is
    /// `now <= deadline`. The drive loop schedules its one-shot quiesce repaint
    /// off this deadline (there is otherwise no re-render to clear an idle cue).
    #[test]
    fn agent_activity_clear_deadline_matches_active_predicate() {
        let appended_at = 1_000_000;
        let deadline = appended_at + PORTAL_ACTIVITY_QUIESCE_WINDOW_US;

        let mut state = make_expanded_interaction_state("portal-deadline");
        state.visible_transcript = vec![TranscriptUnit {
            appended_at_wall_us: appended_at,
            ..transcript_unit(1, OutputKind::Assistant, false)
        }];

        // A live fresh agent tail exposes its deadline.
        assert_eq!(
            agent_activity_clear_deadline_us(&state),
            Some(deadline),
            "the deadline is appended_at + the quiesce window"
        );
        // The active predicate is exactly `now <= deadline` at the boundary.
        assert!(agent_activity_active(&state, deadline), "active at the deadline");
        assert!(
            !agent_activity_active(&state, deadline + 1),
            "quiesced one µs past the deadline"
        );

        // No cue → no deadline (each structural guard removes it). A degraded
        // portal is the representative case; the other guards are covered by
        // `agent_activity_cue_suppressed_in_non_writing_states`.
        let mut degraded = state.clone();
        degraded.connection_degraded = true;
        assert_eq!(
            agent_activity_clear_deadline_us(&degraded),
            None,
            "a degraded portal carries no active cue, so no deadline"
        );
    }

    /// The activity cue is suppressed in every non-writing case, all evaluated at
    /// a `now` that WOULD be fresh for an agent tail — proving each guard, not the
    /// window: a viewer-authored tail (the agent is not writing), a question
    /// awaiting a reply (quiescing, and its own cue owns that state), an empty
    /// transcript (redaction / first-run), a connection-degraded portal, a
    /// never-connected ("connecting") portal, and the collapsed presentation.
    #[test]
    fn agent_activity_cue_suppressed_in_non_writing_states() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let activity_color = adapter.visual_tokens().activity_cue_color;
        let cursor_color = adapter.visual_tokens().streaming_cursor_color;
        let appended_at = 1_000_000;
        let now = appended_at + 500_000; // fresh — so only the guards can suppress

        let assert_suppressed = |state: &ProjectedPortalState, case: &str| {
            assert!(
                !agent_activity_active(state, now),
                "{case}: predicate must be false"
            );
            let md = portal_markdown(state, None, now);
            assert!(
                !md.contains(PORTAL_ACTIVITY_MARKER_LINE),
                "{case}: no header marker: {md}"
            );
            assert!(
                !md.contains(PORTAL_STREAMING_CURSOR_GLYPH),
                "{case}: no cursor glyph: {md}"
            );
            assert!(
                activity_cue_color_runs(state, activity_color, now).is_empty(),
                "{case}: no header run"
            );
            assert!(
                streaming_cursor_color_runs(state, cursor_color, now).is_empty(),
                "{case}: no cursor run"
            );
        };

        let agent_tail = || TranscriptUnit {
            appended_at_wall_us: appended_at,
            ..transcript_unit(1, OutputKind::Assistant, false)
        };

        // Viewer-authored tail: the on-screen viewer, not the agent, is the last
        // author — the agent is not appending.
        let mut viewer_tail = make_expanded_interaction_state("portal-activity");
        viewer_tail.visible_transcript = vec![TranscriptUnit {
            appended_at_wall_us: appended_at,
            ..transcript_unit(1, OutputKind::Viewer, false)
        }];
        assert_suppressed(&viewer_tail, "viewer tail");

        // Question awaiting a reply: quiescing, and the awaiting-reply cue owns it.
        let mut awaiting = make_expanded_interaction_state("portal-activity");
        awaiting.visible_transcript = vec![TranscriptUnit {
            appended_at_wall_us: appended_at,
            ..transcript_unit(1, OutputKind::Assistant, true)
        }];
        assert_suppressed(&awaiting, "awaiting reply");
        assert!(
            portal_markdown(&awaiting, None, now).contains("awaiting reply"),
            "awaiting-reply cue owns this state instead of the activity cue"
        );

        // Empty transcript (redaction / first-run): no tail to derive from.
        let empty = make_expanded_interaction_state("portal-activity");
        assert_suppressed(&empty, "empty transcript");

        // Connection-degraded: the surface must not imply an active stream.
        let mut degraded = make_expanded_interaction_state("portal-activity");
        degraded.visible_transcript = vec![agent_tail()];
        degraded.connection_degraded = true;
        assert_suppressed(&degraded, "degraded");

        // Never connected ("connecting"): connecting takes precedence over activity.
        let mut connecting = make_expanded_interaction_state("portal-activity");
        connecting.visible_transcript = vec![agent_tail()];
        connecting.has_ever_connected = false;
        assert_suppressed(&connecting, "connecting");

        // Collapsed presentation: the cursor is an expanded-transcript affordance.
        let mut collapsed = make_expanded_interaction_state("portal-activity");
        collapsed.visible_transcript = vec![agent_tail()];
        collapsed.presentation = ProjectedPortalPresentation::Collapsed;
        assert_suppressed(&collapsed, "collapsed");
    }

    /// The `activity_cue_color` / `streaming_cursor_color` `PortalVisualTokens`
    /// fields map 1:1 from the source `PortalPartTokens` channels
    /// (single-source-of-truth invariant, matching the other token-mapping tests).
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_activity_and_cursor_fields() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(visual.activity_cue_color.r, part.activity_cue_color.r);
        assert_eq!(visual.activity_cue_color.a, part.activity_cue_color.a);
        assert_eq!(
            visual.streaming_cursor_color.r,
            part.streaming_cursor_color.r
        );
        assert_eq!(
            visual.streaming_cursor_color.a,
            part.streaming_cursor_color.a
        );
    }

    /// §lifecycle: a permitted viewer's published lifecycle_state drives a
    /// token-resolved accent color run, distinct per affordance group, and absent
    /// when lifecycle is redacted (`lifecycle_state = None`). No literal color
    /// appears in the render path — every accent is sourced from `visual_tokens`.
    #[test]
    fn lifecycle_state_drives_distinct_token_accent_runs() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let tokens = adapter.visual_tokens().clone();

        // Each variant maps onto its documented token accent.
        let cases = [
            (
                ProjectionLifecycleState::Active,
                tokens.lifecycle_active_color,
            ),
            (
                ProjectionLifecycleState::Attached,
                tokens.lifecycle_attached_color,
            ),
            (
                ProjectionLifecycleState::Degraded,
                tokens.lifecycle_attention_color,
            ),
            (
                ProjectionLifecycleState::HudUnavailable,
                tokens.lifecycle_attention_color,
            ),
            (
                ProjectionLifecycleState::Detached,
                tokens.lifecycle_inactive_color,
            ),
            (
                ProjectionLifecycleState::CleanupPending,
                tokens.lifecycle_inactive_color,
            ),
            (
                ProjectionLifecycleState::Expired,
                tokens.lifecycle_inactive_color,
            ),
        ];
        for (lifecycle, expected) in cases {
            let mut state = make_expanded_interaction_state("portal-lifecycle");
            state.lifecycle_state = Some(lifecycle);
            let runs = lifecycle_marker_color_runs(&state, &tokens);
            assert_eq!(
                runs.len(),
                1,
                "lifecycle {lifecycle:?} must emit exactly one accent run"
            );
            assert_eq!(
                runs[0].color.unwrap(),
                expected,
                "lifecycle {lifecycle:?} must carry its token-resolved accent"
            );
            assert_eq!(runs[0].start_byte, 0, "Phase-1 sentinel run is zero-length");
            assert_eq!(runs[0].end_byte, 0, "Phase-1 sentinel run is zero-length");
        }

        // The four affordance groups are mutually distinct so each reads as a
        // different viewer-facing state.
        let groups = [
            tokens.lifecycle_active_color,
            tokens.lifecycle_attached_color,
            tokens.lifecycle_attention_color,
            tokens.lifecycle_inactive_color,
        ];
        for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                assert_ne!(
                    groups[i], groups[j],
                    "lifecycle affordance accents {i} and {j} must be visually distinct"
                );
            }
        }

        // Redaction-gated: a viewer without lifecycle clearance gets no affordance.
        let mut redacted = make_expanded_interaction_state("portal-lifecycle");
        redacted.lifecycle_state = None;
        assert!(
            lifecycle_marker_color_runs(&redacted, &tokens).is_empty(),
            "redacted lifecycle must emit no accent run"
        );
    }

    /// §lifecycle (acceptance #3/#4): a lifecycle_state transition changes the
    /// rendered portal node — both the color_runs and the text-visible `status:`
    /// line differ per state. This is the scene change that marks the tile dirty
    /// (it rides the normal PublishToTile mutation), so the affordance renders
    /// rather than being swallowed by idle present-gating.
    #[test]
    fn portal_node_reflects_lifecycle_transition() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);

        let mut active = make_expanded_interaction_state("portal-lc-transition");
        active.lifecycle_state = Some(ProjectionLifecycleState::Active);
        let active_node = adapter.portal_node(&active, vec![0u8; 16], 0);
        let active_tm = text_markdown_node(&active_node);

        let mut detached = make_expanded_interaction_state("portal-lc-transition");
        detached.lifecycle_state = Some(ProjectionLifecycleState::Detached);
        let detached_node = adapter.portal_node(&detached, vec![0u8; 16], 0);
        let detached_tm = text_markdown_node(&detached_node);

        // The render path reflects each state distinctly.
        assert_ne!(
            active_tm.color_runs, detached_tm.color_runs,
            "lifecycle transition must change the node's color runs (render path reflects state)"
        );
        assert_ne!(
            active_tm.content, detached_tm.content,
            "lifecycle transition must change the text-visible status line"
        );
        assert!(
            active_tm.content.contains("status:"),
            "permitted viewer must see the lifecycle status line"
        );
    }

    /// The lifecycle `PortalVisualTokens` fields map 1:1 from the source
    /// `PortalPartTokens` channels (single-source-of-truth invariant).
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_lifecycle_fields() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(
            visual.lifecycle_active_color.r,
            part.lifecycle_active_color.r
        );
        assert_eq!(
            visual.lifecycle_attached_color.g,
            part.lifecycle_attached_color.g
        );
        assert_eq!(
            visual.lifecycle_attention_color.b,
            part.lifecycle_attention_color.b
        );
        assert_eq!(
            visual.lifecycle_inactive_color.a,
            part.lifecycle_inactive_color.a
        );
        // All four must be visible (non-zero alpha) so the affordance shows.
        for c in [
            visual.lifecycle_active_color,
            visual.lifecycle_attached_color,
            visual.lifecycle_attention_color,
            visual.lifecycle_inactive_color,
        ] {
            assert!(c.a > 0.0, "lifecycle accent must have non-zero alpha");
        }
    }

    /// §2: live activity/composer signals clear on disconnect — a degraded
    /// portal must not present `composer: ready` (which implies an active
    /// interactive stream).
    #[test]
    fn degraded_state_clears_composer_ready_signal() {
        let mut state = make_expanded_interaction_state("portal-composer-clear");
        state.connection_degraded = true;
        // interaction_enabled would normally render a composer line; the degraded
        // guard must take precedence.
        state.interaction_enabled = true;

        let markdown = portal_markdown(&state, None, 0);
        assert!(
            !markdown.contains("composer: ready"),
            "§2: degraded portal must not imply an active composer stream"
        );
        assert!(
            markdown.contains("composer: unavailable"),
            "§2: degraded portal must show composer as unavailable"
        );
    }

    /// hud-f6zfa: with an active draft the compositor's bottom-pinned input
    /// strip is the single source of truth for the live draft. The markdown
    /// `composer:` line MUST NOT embed the draft text or caret glyph — doing so
    /// rendered a SECOND copy mid-transcript at a different Y than the bottom
    /// strip (a double / misaligned composer). The line stays a content-free
    /// status affordance.
    #[test]
    fn active_draft_not_duplicated_in_markdown_composer_line() {
        let state = make_expanded_interaction_state("portal-composer-dedup");
        let display = ComposerDisplayState {
            text: "hello world draft".to_string(),
            cursor: 5,
            at_capacity: false,
            sequence: 1,
        };

        let markdown = portal_markdown(&state, Some(&display), 0);

        // The draft text + caret live ONLY in the bottom input strip, never in
        // the transcript-flow markdown — otherwise the draft appears twice at
        // different Y positions.
        assert!(
            !markdown.contains("hello world draft"),
            "draft text must not be embedded in the markdown composer line: {markdown}"
        );
        assert!(
            !markdown.contains('▌'),
            "caret glyph must not appear in the markdown (bottom strip owns it): {markdown}"
        );
        // A content-free composer affordance is still present so the surface
        // reflects that the composer is active.
        assert!(
            markdown.contains("composer: composing"),
            "composer status affordance should be present: {markdown}"
        );
    }

    /// hud-f6zfa: at-capacity stays text-visible on the markdown composer status
    /// line even though the draft glyphs themselves are owned by the bottom
    /// strip — the `[!]` marker must not disappear with the draft dedup.
    #[test]
    fn at_capacity_draft_marks_capacity_without_draft_text() {
        let state = make_expanded_interaction_state("portal-composer-cap");
        let display = ComposerDisplayState {
            text: "some capped draft text".to_string(),
            cursor: 4,
            at_capacity: true,
            sequence: 2,
        };

        let markdown = portal_markdown(&state, Some(&display), 0);

        assert!(
            !markdown.contains("some capped draft text"),
            "draft text must not be duplicated in markdown: {markdown}"
        );
        assert!(
            !markdown.contains('▌'),
            "caret glyph must not appear in the markdown: {markdown}"
        );
        assert!(
            markdown.contains("[!]"),
            "at-capacity must remain text-visible on the status line: {markdown}"
        );
    }

    /// The degraded `PortalVisualTokens` fields map 1:1 from the source
    /// `PortalPartTokens` channels (single-source-of-truth invariant).
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_degraded_fields() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(
            visual.transcript_dim_text_color.r,
            part.transcript_dim_text_color.r
        );
        assert_eq!(
            visual.transcript_dim_background.g,
            part.transcript_dim_background.g
        );
        assert_eq!(visual.stale_marker_color.b, part.stale_marker_color.b);
        assert_eq!(visual.stale_marker_color.a, part.stale_marker_color.a);
    }

    #[test]
    fn default_projected_portal_font_sizes_are_readable() {
        let visual = PortalVisualTokens::default();

        assert!(
            visual.transcript_font_size_px >= 16.0,
            "projected portal transcript default font should be readable without resize; got {}px",
            visual.transcript_font_size_px
        );
        assert!(
            visual.composer_font_size_px >= 16.0,
            "projected portal composer default font should be readable without resize; got {}px",
            visual.composer_font_size_px
        );
        assert!(
            visual.collapsed_font_size_px >= 14.0,
            "projected portal collapsed default font should remain readable; got {}px",
            visual.collapsed_font_size_px
        );
    }

    // ── §First-Run Empty Portal Treatment (hud-g1ena.6) ──────────────────────

    /// A connected portal with an empty retained transcript renders the friendly,
    /// token-styled empty state — NOT the literal `<empty projection stream>` —
    /// and emits exactly one token-driven sentinel color run.
    #[test]
    fn empty_state_replaces_literal_placeholder_with_token_styled_ready_line() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let empty_color = adapter.visual_tokens().empty_state_color;

        // Fixture: connected (has_ever_connected == true), empty transcript.
        let state = make_expanded_interaction_state("portal-empty");
        assert!(state.visible_transcript.is_empty(), "precondition: empty");

        let markdown = portal_markdown(&state, None, 0);
        assert!(
            !markdown.contains("<empty projection stream>"),
            "the literal placeholder must be gone: {markdown}"
        );
        assert!(
            markdown.contains(PORTAL_EMPTY_READY_LINE),
            "connected + empty must render the inviting ready line: {markdown}"
        );

        let runs = empty_state_color_runs(&state, empty_color);
        assert_eq!(runs.len(), 1, "empty state must emit one token-driven run");
        assert_eq!(
            runs[0].color.unwrap(),
            empty_color,
            "run must carry the empty_state_color token, never a literal color"
        );
        assert_eq!(runs[0].start_byte, 0);
        assert_eq!(runs[0].end_byte, 0);
    }

    /// §Connecting State Distinction precedence: an attached-but-never-connected
    /// portal (`has_ever_connected == false`) shows the distinct connecting line —
    /// never the "ready" invite — and emits the connecting token run but NO
    /// empty-state run (the two treatments are mutually exclusive, hud-g1ena.7).
    #[test]
    fn empty_state_yields_to_connecting_when_never_connected() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let empty_color = adapter.visual_tokens().empty_state_color;
        let connecting_color = adapter.visual_tokens().connecting_marker_color;

        let mut state = make_expanded_interaction_state("portal-connecting");
        state.has_ever_connected = false;

        let markdown = portal_markdown(&state, None, 0);
        assert!(
            markdown.contains(PORTAL_CONNECTING_LINE),
            "never-connected must render the connecting line: {markdown}"
        );
        assert!(
            !markdown.contains(PORTAL_EMPTY_READY_LINE),
            "a starting-up portal must NOT read as a ready empty state: {markdown}"
        );
        assert!(
            !markdown.contains("<empty projection stream>"),
            "the literal placeholder must be gone even while connecting: {markdown}"
        );
        // The connecting treatment emits its own token run and suppresses the
        // empty-ready run — the two gates are inverted, so exactly one fires.
        let connecting_runs = connecting_color_runs(&state, connecting_color);
        assert_eq!(
            connecting_runs.len(),
            1,
            "connecting state must emit one token-driven run"
        );
        assert_eq!(
            connecting_runs[0].color.unwrap(),
            connecting_color,
            "run must carry the connecting_marker_color token, never a literal color"
        );
        assert!(
            empty_state_color_runs(&state, empty_color).is_empty(),
            "the connecting case must emit no empty-ready run (mutually exclusive)"
        );
    }

    /// §Connecting State Distinction core requirement: the connecting treatment is
    /// visually distinct from the degraded/disconnected treatment on BOTH the
    /// text (distinct glyph + copy) and the token color — a starting-up portal
    /// must not read as a failing one.
    #[test]
    fn connecting_treatment_is_distinct_from_degraded() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let tokens = adapter.visual_tokens();

        // Text axis: the connecting line and the degraded marker share no glyph
        // or copy, so environments that ignore color_runs still tell them apart.
        assert_ne!(
            PORTAL_CONNECTING_LINE, PORTAL_DISCONNECT_MARKER_LINE,
            "connecting and degraded lines must differ"
        );
        assert!(
            !PORTAL_CONNECTING_LINE.contains("disconnected")
                && !PORTAL_CONNECTING_LINE.contains("stale"),
            "connecting copy must not read as disconnected/stale: {PORTAL_CONNECTING_LINE}"
        );

        // Color axis: the connecting hue differs from the degraded/stale marker.
        assert_ne!(
            tokens.connecting_marker_color, tokens.stale_marker_color,
            "connecting hue must be distinct from the degraded/stale marker"
        );

        // A never-connected portal renders connecting, NOT the degraded marker,
        // and does not dim the transcript or disable the composer for a failure.
        let mut connecting = make_expanded_interaction_state("portal-connecting-vs-degraded");
        connecting.has_ever_connected = false;
        let connecting_md = portal_markdown(&connecting, None, 0);
        assert!(
            connecting_md.contains(PORTAL_CONNECTING_LINE),
            "never-connected renders connecting: {connecting_md}"
        );
        assert!(
            !connecting_md.contains(PORTAL_DISCONNECT_MARKER_LINE),
            "never-connected must NOT render the degraded/disconnect marker: {connecting_md}"
        );

        // A previously-connected-now-dropped portal renders the degraded marker,
        // NOT the connecting line — the inverse case, proving the split.
        let mut degraded = make_expanded_interaction_state("portal-degraded-not-connecting");
        degraded.connection_degraded = true;
        let degraded_md = portal_markdown(&degraded, None, 0);
        assert!(
            degraded_md.contains(PORTAL_DISCONNECT_MARKER_LINE),
            "dropped portal renders the degraded marker: {degraded_md}"
        );
        assert!(
            !degraded_md.contains(PORTAL_CONNECTING_LINE),
            "a dropped (previously-connected) portal must NOT read as connecting: {degraded_md}"
        );
    }

    /// The connecting treatment is content-free, so — like the degraded marker —
    /// it is redaction-independent: a restricted viewer of a never-connected
    /// portal still sees connecting (revealing only connection state, no content),
    /// taking precedence over the redacted empty placeholder.
    #[test]
    fn connecting_treatment_is_redaction_independent() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let connecting_color = adapter.visual_tokens().connecting_marker_color;

        let mut state = make_expanded_interaction_state("portal-connecting-redacted");
        state.has_ever_connected = false;
        state.redacted = true;

        let markdown = portal_markdown(&state, None, 0);
        assert!(
            markdown.contains(PORTAL_CONNECTING_LINE),
            "a redacted never-connected portal still shows connecting: {markdown}"
        );
        assert!(
            !markdown.contains(PORTAL_EMPTY_REDACTED_LINE),
            "connecting takes precedence over the redacted empty placeholder: {markdown}"
        );
        assert_eq!(
            connecting_color_runs(&state, connecting_color).len(),
            1,
            "the connecting token run is emitted even under redaction"
        );
    }

    /// §First-Run Empty Portal Treatment redaction scenario: a restricted viewer
    /// (whose `visible_transcript` is emptied upstream, so this path is reached)
    /// sees a content-free placeholder with NO inviting copy. The empty-state
    /// treatment itself is still active, so the token-driven run is still emitted.
    #[test]
    fn empty_state_suppresses_inviting_copy_under_redaction() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let empty_color = adapter.visual_tokens().empty_state_color;

        let mut state = make_expanded_interaction_state("portal-redacted-empty");
        state.redacted = true;

        let markdown = portal_markdown(&state, None, 0);
        assert!(
            markdown.contains(PORTAL_EMPTY_REDACTED_LINE),
            "redacted empty portal must render the content-free placeholder: {markdown}"
        );
        assert!(
            !markdown.contains(PORTAL_EMPTY_READY_LINE),
            "redaction must suppress the inviting copy: {markdown}"
        );
        assert_eq!(
            empty_state_color_runs(&state, empty_color).len(),
            1,
            "the redacted empty-state treatment is still active → one token run"
        );
    }

    /// The empty state yields immediately to real content: the first appended
    /// transcript unit replaces it, and no empty-state marker or run remains.
    #[test]
    fn empty_state_yields_to_first_content() {
        let config = ResidentGrpcPortalConfig::new(vec![0u8; 16]);
        let adapter = ResidentGrpcPortalAdapter::new(config);
        let empty_color = adapter.visual_tokens().empty_state_color;

        let mut state = make_expanded_interaction_state("portal-first-content");
        state.visible_transcript = vec![transcript_unit(1, OutputKind::Assistant, false)];

        let markdown = portal_markdown(&state, None, 0);
        assert!(
            markdown.contains("example output"),
            "real content must render: {markdown}"
        );
        assert!(
            !markdown.contains(PORTAL_EMPTY_READY_LINE)
                && !markdown.contains(PORTAL_CONNECTING_LINE)
                && !markdown.contains(PORTAL_EMPTY_REDACTED_LINE),
            "no empty/connecting placeholder once content exists: {markdown}"
        );
        assert!(
            empty_state_color_runs(&state, empty_color).is_empty(),
            "a non-empty transcript must emit no empty-state run"
        );
    }

    /// The `empty_state_color` `PortalVisualTokens` field maps 1:1 from the
    /// source `PortalPartTokens` channel (hud-g1ena.6), same single-source-of-
    /// truth invariant as the other token-mapping tests in this module.
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_empty_state_color() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(visual.empty_state_color.r, part.empty_state_color.r);
        assert_eq!(visual.empty_state_color.a, part.empty_state_color.a);
    }

    /// The `connecting_marker_color` `PortalVisualTokens` field maps 1:1 from the
    /// source `PortalPartTokens` channel (hud-g1ena.7), same single-source-of-
    /// truth invariant as the other token-mapping tests in this module.
    #[test]
    fn portal_visual_tokens_from_part_tokens_maps_connecting_marker_color() {
        let part = tze_hud_config::PortalPartTokens::default();
        let visual = portal_visual_tokens_from_part_tokens(&part);
        assert_eq!(
            visual.connecting_marker_color.r,
            part.connecting_marker_color.r
        );
        assert_eq!(
            visual.connecting_marker_color.a,
            part.connecting_marker_color.a
        );
        // The default connecting hue must not collide with the degraded/stale
        // marker — the §Connecting State Distinction invariant, enforced at the
        // mapping boundary too.
        assert_ne!(visual.connecting_marker_color, visual.stale_marker_color);
    }
}
