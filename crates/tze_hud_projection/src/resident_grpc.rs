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
    pub fn render_portal_markdown(&self, state: &ProjectedPortalState) -> String {
        portal_markdown(state, self.composer_display.as_ref())
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

    fn render_batch(
        &self,
        state: &ProjectedPortalState,
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
                        node: Some(self.portal_node(state, root_id_le)),
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
                                    accepts_pointer: false,
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

    fn portal_node(&self, state: &ProjectedPortalState, root_id_le: Vec<u8>) -> proto::NodeProto {
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
            // Explicit root ID (little-endian UUID bytes per RFC 0001 §4.1) so
            // render_batch can reference it as AddNodeMutation.parent_id in the
            // same batch when adding the composer hit region.
            id: root_id_le,
            data: Some(proto::node_proto::Data::TextMarkdown(
                proto::TextMarkdownNodeProto {
                    content: portal_markdown(state, self.composer_display.as_ref()),
                    bounds: Some(bounds),
                    font_size_px,
                    color: Some(text_color),
                    background: Some(background_color),
                    // color_runs carry the composer at-capacity indicator when active.
                    // The at-capacity run covers the composer line with
                    // `composer_at_capacity_color` so the visual token drives the
                    // display without any literal color in the render path.
                    color_runs: composer_color_runs(
                        state,
                        self.composer_display.as_ref(),
                        self.visual_tokens.composer_at_capacity_color,
                    ),
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

fn portal_markdown(
    state: &ProjectedPortalState,
    composer_display: Option<&ComposerDisplayState>,
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
            push_line(&mut result, "");
            if state.interaction_enabled {
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
        push_line(
            &mut result,
            &format!("last composer: {:?}", feedback.feedback_state),
        );
    }
    truncate_utf8(result, MAX_PORTAL_MARKDOWN_BYTES)
}

/// Build the composer region line for the expanded portal node.
///
/// Renders the current draft text with an inline `▌` caret marker inserted at
/// the cursor byte offset. The caret marker is chosen because it is a Unicode
/// block character that is visually distinct in monospace/proportional fonts
/// and does not require compositor-level cursor blinking support.
///
/// When the draft is at capacity, the prefix `[!] ` is prepended to make the
/// at-capacity state text-visible. The color_run path (via `composer_color_runs`)
/// additionally applies `composer_at_capacity_color` to the line for the
/// token-driven visual indicator. Both mechanisms are redundant by design:
/// the text prefix is readable in environments where color runs are unavailable.
///
/// When no draft is active (composer_display is None), returns "composer: ready"
/// to indicate the composer is available for input.
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

    let text = &display.text;
    let cursor = display.cursor.min(text.len());

    // Insert the caret marker (▌ U+258C LEFT HALF BLOCK) at the cursor position.
    // The cursor is a byte offset; we snap backward to the nearest valid char
    // boundary at or before the offset to avoid panics on multi-byte characters.
    let mut snap = cursor;
    while snap > 0 && !text.is_char_boundary(snap) {
        snap -= 1;
    }
    let (before, after) = text.split_at(snap);
    let with_caret = format!("{before}▌{after}");

    if display.at_capacity {
        // Text-visible at-capacity prefix + draft with caret.
        // The color_runs path applies the token-driven color independently.
        format!("[!] {with_caret}")
    } else {
        with_caret
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
        ProjectedPortalState,
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
            pending_input_count: None,
            pending_input_bytes: None,
            last_input_feedback: None,
            draft_batch: None,
            geometry_batch: None,
        }
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
            .render_batch(&state)
            .expect("render_batch must succeed with interaction_enabled");

        // Should be 3 mutations: PublishToTile, UpdateTileInputMode, AddNode.
        assert_eq!(
            batch.mutations.len(),
            3,
            "interaction_enabled=true must produce PublishToTile + UpdateTileInputMode + AddNode \
             (composer hit region)"
        );

        // The third mutation must be AddNode with accepts_composer_input=true.
        let add_node_mutation = &batch.mutations[2];
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
            other => panic!("Third mutation must be AddNode (composer hit region), got {other:?}"),
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
            .render_batch(&state)
            .expect("render_batch must succeed");

        // Should be 2 mutations only: PublishToTile + UpdateTileInputMode.
        assert_eq!(
            batch.mutations.len(),
            2,
            "interaction_enabled=false must produce exactly 2 mutations (no AddNode)"
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
            .render_batch(&state)
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
                match &batch.mutations[2].mutation {
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
                    other => panic!("Third mutation must be AddNode, got {other:?}"),
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
