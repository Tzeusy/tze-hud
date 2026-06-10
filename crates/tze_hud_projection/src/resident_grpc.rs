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
    AdapterDraftBatch, AdapterDraftNotification, ContentClassification, PortalInputFeedback,
    PortalInputSubmission, ProjectedPortalPresentation, ProjectedPortalState, ProjectionAuthority,
    TranscriptUnit,
};

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
/// Build from the runtime's resolved `DesignTokenMap` using
/// `tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens`,
/// then pass to `ResidentGrpcPortalAdapter::with_tokens`.
///
/// ## Phase-1 scope limitation
///
/// The Phase-1 raw-tile pilot publishes a **single** `TextMarkdownNodeProto`,
/// which carries only `color`, `background`, and `font_size_px`. This struct
/// therefore contains only the six fields that `portal_node` actually consumes
/// (transcript and collapsed parts). The full part inventory defined in
/// `PortalPartTokens` (frame, header, composer, divider, transitions) requires
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
///
/// Frame, header, composer, divider, and transition fields are omitted because
/// `TextMarkdownNodeProto` has no slots for them. They are wired in
/// `PortalPartTokens` (in `tze_hud_config`) for promotion-era structured layout.
#[derive(Clone, Debug, PartialEq)]
pub struct PortalVisualTokens {
    // Transcript body (expanded presentation)
    pub transcript_background: proto::Rgba,
    pub transcript_text_color: proto::Rgba,
    pub transcript_font_size_px: f32,

    // Collapsed card (collapsed presentation)
    pub collapsed_background: proto::Rgba,
    pub collapsed_text_color: proto::Rgba,
    pub collapsed_font_size_px: f32,
}

impl Default for PortalVisualTokens {
    /// Default visual tokens — same palette as the Phase-0 exemplar literals,
    /// expressed as resolved token defaults, scoped to the fields consumed by
    /// the raw-tile pilot's single `TextMarkdownNodeProto`.
    ///
    /// In production these values are superseded by `with_tokens()` which
    /// accepts tokens produced by
    /// `tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens`.
    /// This default is used only in tests that do not exercise the token path,
    /// and as a fallback when no tokens are supplied to the adapter.
    ///
    /// NOTE: The numeric defaults here must match the string defaults in
    /// `tze_hud_config::portal_tokens::defaults` for the corresponding keys.
    /// There is no compile-time link; update both sides together.
    fn default() -> Self {
        Self {
            transcript_background: proto::Rgba {
                r: 0.039,
                g: 0.051,
                b: 0.067,
                a: 0.90,
            },
            transcript_text_color: proto::Rgba {
                r: 0.90,
                g: 0.933,
                b: 0.980,
                a: 1.0,
            },
            transcript_font_size_px: 13.0,
            collapsed_background: proto::Rgba {
                r: 0.102,
                g: 0.122,
                b: 0.157,
                a: 0.88,
            },
            collapsed_text_color: proto::Rgba {
                r: 0.784,
                g: 0.839,
                b: 0.910,
                a: 1.0,
            },
            collapsed_font_size_px: 12.0,
        }
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
        }
    }

    /// Create a new adapter with the given resolved visual tokens.
    ///
    /// This is the preferred constructor for production use. Build
    /// `tokens` from the runtime's resolved `DesignTokenMap` using
    /// `tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens`
    /// (which calls `tze_hud_config::resolve_portal_tokens` and converts
    /// the result to `PortalVisualTokens` for the Phase-1 pilot).
    pub fn with_tokens(config: ResidentGrpcPortalConfig, tokens: PortalVisualTokens) -> Self {
        Self {
            config,
            tile_id: None,
            next_input_sequence: 0,
            last_draft_sequence: 0,
            visual_tokens: tokens,
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

    pub fn tile_id(&self) -> Option<&[u8]> {
        self.tile_id.as_deref()
    }

    pub fn lease_id(&self) -> &[u8] {
        &self.config.lease_id
    }

    /// Record the tile ID returned by the resident `CreateTile` mutation.
    pub fn record_created_tile(&mut self, tile_id: Vec<u8>) {
        self.tile_id = Some(tile_id);
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
                session_proto::client_message::Payload::MutationBatch(self.render_batch(state)?),
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
            session_proto::client_message::Payload::MutationBatch(self.render_batch(state)?),
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

        // Transactional cancel
        if let Some(cancel) = &batch.cancel {
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
        // call `submit_composer_text` with the submission text)
        if let Some(submission) = &batch.submission {
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

    fn render_batch(
        &self,
        state: &ProjectedPortalState,
    ) -> Result<session_proto::MutationBatch, ResidentGrpcAdapterError> {
        let tile_id = self
            .tile_id
            .clone()
            .ok_or(ResidentGrpcAdapterError::MissingPortalTile)?;
        Ok(session_proto::MutationBatch {
            batch_id: new_scene_id_bytes(),
            lease_id: self.config.lease_id.clone(),
            mutations: vec![
                proto::MutationProto {
                    mutation: Some(proto::mutation_proto::Mutation::PublishToTile(
                        proto::PublishToTileMutation {
                            element_id: tile_id.clone(),
                            bounds: Some(self.bounds_for_state(state)),
                            node: Some(self.portal_node(state)),
                        },
                    )),
                },
                proto::MutationProto {
                    mutation: Some(proto::mutation_proto::Mutation::UpdateTileInputMode(
                        proto::UpdateTileInputModeMutation {
                            tile_id,
                            input_mode: if state.interaction_enabled {
                                proto::TileInputModeProto::TileInputModeCapture as i32
                            } else {
                                proto::TileInputModeProto::TileInputModeLocalOnly as i32
                            },
                        },
                    )),
                },
            ],
            timing: None,
        })
    }

    fn portal_node(&self, state: &ProjectedPortalState) -> proto::NodeProto {
        // §6.1 enforcement: every visual value sourced from self.visual_tokens —
        // no literal colors, font sizes, or opacities permitted here.
        let bounds = self.local_bounds_for_state(state);
        let (text_color, background_color, font_size_px) = match state.presentation {
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
        proto::NodeProto {
            id: Vec::new(),
            data: Some(proto::node_proto::Data::TextMarkdown(
                proto::TextMarkdownNodeProto {
                    content: portal_markdown(state),
                    bounds: Some(bounds),
                    font_size_px,
                    color: Some(text_color),
                    background: Some(background_color),
                    color_runs: Vec::new(),
                },
            )),
        }
    }

    fn bounds_for_state(&self, state: &ProjectedPortalState) -> proto::Rect {
        match state.presentation {
            ProjectedPortalPresentation::Expanded => self.config.expanded_bounds,
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

fn portal_markdown(state: &ProjectedPortalState) -> String {
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
    if let Some(lifecycle) = state.lifecycle_state {
        push_line(&mut result, &format!("status: {lifecycle:?}"));
    }
    if let Some(status_text) = state.status_text.as_deref() {
        push_line(&mut result, &format!("note: {status_text}"));
    }

    match state.presentation {
        ProjectedPortalPresentation::Expanded => {
            push_line(&mut result, "");
            push_line(
                &mut result,
                &visible_transcript_markdown(&state.visible_transcript),
            );
            if state.interaction_enabled {
                push_line(&mut result, "");
                push_line(&mut result, "composer: ready");
            } else {
                push_line(&mut result, "");
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
        push_line(
            &mut result,
            &format!("last composer: {:?}", feedback.feedback_state),
        );
    }
    truncate_utf8(result, MAX_PORTAL_MARKDOWN_BYTES)
}

fn visible_transcript_markdown(units: &[TranscriptUnit]) -> String {
    if units.is_empty() {
        return "<empty projection stream>".to_string();
    }
    let mut result = String::new();
    for (index, unit) in units.iter().enumerate() {
        if index > 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputKind;

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
                appended_at_wall_us: 1,
            },
            TranscriptUnit {
                sequence: 2,
                output_text: "é".repeat(3_000),
                output_kind: OutputKind::Assistant,
                content_classification: ContentClassification::Private,
                logical_unit_id: None,
                coalesce_key: None,
                appended_at_wall_us: 2,
            },
        ];

        let markdown = visible_transcript_markdown(&units);

        assert!(markdown.starts_with("first\n"));
        assert!(markdown.is_char_boundary(markdown.len()));
        assert!(markdown.len() <= "first\n".len() + 4_096);
    }
}
