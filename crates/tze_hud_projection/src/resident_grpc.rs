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
    ContentClassification, PortalInputFeedback, PortalInputSubmission, ProjectedPortalPresentation,
    ProjectedPortalState, ProjectionAuthority, TranscriptUnit,
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

/// Stateful daemon-side adapter for one projected session's resident portal.
#[derive(Clone, Debug)]
pub struct ResidentGrpcPortalAdapter {
    config: ResidentGrpcPortalConfig,
    tile_id: Option<Vec<u8>>,
    next_input_sequence: u64,
}

impl ResidentGrpcPortalAdapter {
    pub fn new(config: ResidentGrpcPortalConfig) -> Self {
        Self {
            config,
            tile_id: None,
            next_input_sequence: 0,
        }
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
        let bounds = self.local_bounds_for_state(state);
        proto::NodeProto {
            id: Vec::new(),
            data: Some(proto::node_proto::Data::TextMarkdown(
                proto::TextMarkdownNodeProto {
                    content: portal_markdown(state),
                    bounds: Some(bounds),
                    font_size_px: if state.presentation == ProjectedPortalPresentation::Expanded {
                        14.0
                    } else {
                        12.0
                    },
                    color: Some(proto::Rgba {
                        r: 0.94,
                        g: 0.97,
                        b: 1.0,
                        a: 1.0,
                    }),
                    background: Some(proto::Rgba {
                        r: 0.06,
                        g: 0.08,
                        b: 0.11,
                        a: 0.90,
                    }),
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
    let mut lines = Vec::new();
    let title = state.display_name.as_deref().unwrap_or("Projected session");
    lines.push(format!("**{title}**"));
    lines.push(format!(
        "`{}` · {:?} · {:?}",
        state.portal_id, state.presentation, state.attention
    ));
    if let Some(lifecycle) = state.lifecycle_state {
        lines.push(format!("status: {lifecycle:?}"));
    }
    if let Some(status_text) = state.status_text.as_deref() {
        lines.push(format!("note: {status_text}"));
    }

    match state.presentation {
        ProjectedPortalPresentation::Expanded => {
            lines.push(String::new());
            lines.push(visible_transcript_markdown(&state.visible_transcript));
            if state.interaction_enabled {
                lines.push(String::new());
                lines.push("composer: ready".to_string());
            } else {
                lines.push(String::new());
                lines.push("composer: unavailable".to_string());
            }
        }
        ProjectedPortalPresentation::Collapsed => {
            let preview = state
                .visible_transcript
                .last()
                .map(|unit| unit.output_text.as_str())
                .unwrap_or("compact projection affordance");
            lines.push(clamp_one_line(preview, 160));
        }
    }

    if let Some(pending) = state.pending_input_count {
        lines.push(format!("pending HUD input: {pending}"));
    }
    if let Some(feedback) = &state.last_input_feedback {
        lines.push(format!("last composer: {:?}", feedback.feedback_state));
    }
    clamp_utf8(lines.join("\n"), MAX_PORTAL_MARKDOWN_BYTES)
}

fn visible_transcript_markdown(units: &[TranscriptUnit]) -> String {
    if units.is_empty() {
        return "<empty projection stream>".to_string();
    }
    units
        .iter()
        .map(|unit| clamp_utf8(unit.output_text.clone(), 4_096))
        .collect::<Vec<_>>()
        .join("\n")
}

fn clamp_one_line(text: &str, max_bytes: usize) -> String {
    clamp_utf8(
        text.lines().next().unwrap_or_default().to_string(),
        max_bytes,
    )
}

fn clamp_utf8(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text
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
