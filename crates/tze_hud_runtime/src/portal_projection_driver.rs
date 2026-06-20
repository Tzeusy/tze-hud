//! In-process portal projection authority driver (hud-2iup7).
//!
//! This module hosts a [`ProjectionAuthority`] directly inside the runtime
//! process and drives the portal drain loop from the winit event-loop
//! `about_to_wait` callback (composer-flush pattern).
//!
//! ## Architecture decision (hud-6k06b)
//!
//! The projection authority is hosted in-process rather than as a stdio
//! subprocess. The stdio binary (`projection_authority`) and its tests remain
//! as the component harness; this driver is the production integration path.
//!
//! ## Drain loop (spec §3.2 / §3.3)
//!
//! On every `about_to_wait` the driver:
//! 1. Acquires `ProjectionAuthority::next_due_projection_id()` in round-robin order.
//! 2. Materialises the coalesced transcript window via `take_due_portal_update`.
//! 3. Builds the portal state via `projected_portal_state`.
//! 4. For `CreatePortalTile` records: creates a new tile in the scene graph,
//!    registers its scroll config, and records the tile ID in the adapter.
//! 5. For `RenderPortal` records: calls
//!    [`InputProcessor::notify_tile_content_appended`] with the append geometry
//!    so follow-tail advances (spec §3.2) or remains stable when scrolled-back
//!    (spec §3.3 — the `InputProcessor` enforces this; the driver just calls it).
//!
//! ## Lock discipline
//!
//! The `drain` method takes a direct `&mut SceneGraph` and `&mut InputProcessor`
//! reference (not locks), so it is synchronous and lock-free.  The caller
//! (windowed runtime) is responsible for acquiring the scene lock with
//! `try_lock` and deferring the call to the next `about_to_wait` if busy.
//!
//! ## Panic safety
//!
//! The `drain` method is wrapped in `std::panic::catch_unwind`. A panic resets
//! the driver's drive state and logs an error; it does NOT propagate to the
//! event loop.
//!
//! ## Hook points
//!
//! - **hud-ttq97** (submitted_at_us telemetry bucket): `submitted_at_us` from
//!   `PortalTranscriptUpdate` is consumed by the drain loop into
//!   [`InProcessPortalDriver::portal_publish_to_present_latency`], measuring
//!   the end-to-end publish→present latency for each coalesced portal update.
//! - **hud-pkg2g** (head-trim notify_head_content_removed): wired — the drain
//!   loop detects head-trim via `visible_transcript_bytes` / content-height
//!   decrease and calls `notify_head_content_removed` on the `InputProcessor`
//!   so scrolled-back viewports stay stable (spec §3.3).

use std::collections::HashMap;

use tze_hud_config::{resolve_portal_tokens, tokens::DesignTokenMap};
use tze_hud_input::{DraftNotificationBatch, InputProcessor};
pub use tze_hud_mcp::portal_op::{
    PendingInputBatch, PendingInputEntry, PortalOp, PortalOpRejection,
};
use tze_hud_projection::{
    AcknowledgeInputRequest, AdapterDraftBatch, AdapterDraftCancel, AdapterDraftNotification,
    AdapterDraftSubmission, AdapterGeometrySnapshot, AdapterPortalRect, AttachRequest,
    CleanupAuthority, CleanupRequest, ContentClassification, DetachRequest, GetPendingInputRequest,
    InputAckState, OperationEnvelope, OutputKind, PendingInputItem, PortalInputFeedback,
    ProjectedPortalPolicy, ProjectionAuthority, ProjectionBounds, ProjectionErrorCode,
    ProjectionOperation, ProviderKind, PublishOutputRequest,
    resident_grpc::{
        ResidentGrpcPortalAdapter, ResidentGrpcPortalCommandKind, ResidentGrpcPortalConfig,
        portal_visual_tokens_from_part_tokens,
    },
};
use tze_hud_scene::{
    Capability, Rect, SceneGraph,
    types::{SceneId, TileScrollConfig},
};
use tze_hud_telemetry::LatencyBucket;

/// Line-height multiplier used by the compositor's text shaper (text.rs).
///
/// `line_height_px = font_size_px * PORTAL_LINE_HEIGHT_MULTIPLIER`
///
/// Must stay in sync with `tze_hud_compositor::text` — search for `1.4` there.
const PORTAL_LINE_HEIGHT_MULTIPLIER: f32 = 1.4;

/// Namespace used by the in-process projection driver for scene mutations.
///
/// This namespace is used when creating portal tiles and publishing content.
/// It is distinct from any agent-facing namespace.
pub const PORTAL_DRIVER_NAMESPACE: &str = "tze_hud_portal_driver";

/// Default z-order for portal tiles created by the in-process driver.
const PORTAL_Z_ORDER: u32 = 160;

/// Hard upper bound on the number of portal updates processed in a single
/// `drain_inner` call (per `about_to_wait` tick).
///
/// The drain runs on the winit event-loop thread while holding the scene lock
/// under `ControlFlow::Poll`, so an unbounded inner loop wedges the entire
/// presentation pipeline (whole-HUD freeze, one core pegged ~98%). The loop is
/// normally work-conserving and terminates when the coalescer is drained, but a
/// divergence between the session map and the coalescer (e.g. an orphaned
/// coalescer entry whose session was removed by operator cleanup — hud-bsr7u)
/// could make `next_due_projection_id` return the same id forever.
///
/// This cap is a defense-in-depth backstop: even if a future divergence
/// re-introduces such an entry, the loop can never spin the event loop more than
/// this many iterations per tick. The value is far above the realistic number of
/// concurrent portals (≤ 8 in practice, each contributing at most one update per
/// drain), so it never truncates legitimate work; reaching it indicates a bug
/// and is logged at `error` level.
const MAX_PORTAL_DRAIN_ITERATIONS_PER_CYCLE: u32 = 1024;

/// Parse an optional snake_case `output_kind` string into [`OutputKind`].
///
/// `None` defaults to [`OutputKind::Assistant`] (the contract default). The
/// value is deserialized through the same serde `snake_case` representation
/// the wire contract uses, so accepted spellings match
/// `serde(rename_all = "snake_case")` exactly. An unrecognized value yields an
/// `Err` describing the rejection, which the caller forwards to the requester
/// rather than silently coercing to a default.
fn parse_output_kind(raw: Option<&str>) -> Result<OutputKind, String> {
    match raw {
        None => Ok(OutputKind::default()),
        Some(value) => serde_json::from_value(serde_json::Value::String(value.to_string()))
            .map_err(|_| {
                format!(
                    "invalid output_kind {value:?}: expected one of \
                     assistant, tool, status, error, other"
                )
            }),
    }
}

/// Parse an optional snake_case `content_classification` string into
/// [`ContentClassification`].
///
/// `None` defaults to [`ContentClassification::Private`] — privacy is
/// safe-by-default per the cooperative-hud-projection privacy governance
/// requirement. An unrecognized value yields an `Err` rather than coercing to a
/// (potentially less private) default.
fn parse_content_classification(raw: Option<&str>) -> Result<ContentClassification, String> {
    match raw {
        None => Ok(ContentClassification::default()),
        Some(value) => serde_json::from_value(serde_json::Value::String(value.to_string()))
            .map_err(|_| {
                format!(
                    "invalid content_classification {value:?}: expected one of \
                     public, household, private, sensitive"
                )
            }),
    }
}

/// Parse a snake_case `ack_state` string into [`InputAckState`].
///
/// Accepts exactly the wire spellings of `serde(rename_all = "snake_case")`
/// (`handled`, `deferred`, `rejected`). Unlike the publish-output parsers there
/// is no default: acknowledgement state is mandatory, so an empty or
/// unrecognized value yields an `Err` describing the rejection, which the
/// caller forwards to the requester.
fn parse_ack_state(raw: &str) -> Result<InputAckState, String> {
    serde_json::from_value(serde_json::Value::String(raw.to_string())).map_err(|_| {
        format!("invalid ack_state {raw:?}: expected one of handled, deferred, rejected")
    })
}

/// Parse a snake_case cleanup authority string into [`CleanupAuthority`].
///
/// Accepted spellings match the projection contract enum exactly (`owner`,
/// `operator`). Rejection happens before reaching the authority so malformed MCP
/// requests cannot be silently treated as owner or operator cleanup.
fn parse_cleanup_authority(raw: &str) -> Result<CleanupAuthority, String> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|_| format!("invalid cleanup_authority {raw:?}: expected owner or operator"))
}

fn adapter_draft_batch_from_runtime(batch: &DraftNotificationBatch) -> AdapterDraftBatch {
    AdapterDraftBatch {
        latest: batch
            .latest
            .as_ref()
            .map(|latest| AdapterDraftNotification {
                text: latest.text.clone(),
                cursor: latest.cursor,
                selection_anchor: latest.selection_anchor,
                at_capacity: latest.at_capacity,
                sequence: latest.sequence,
            }),
        submission: batch
            .submission
            .as_ref()
            .map(|submission| AdapterDraftSubmission {
                text: submission.text.clone(),
                sequence: submission.sequence,
            }),
        cancel: batch.cancel.as_ref().map(|cancel| AdapterDraftCancel {
            sequence: cancel.sequence,
        }),
    }
}

/// Map an authority [`PendingInputItem`] into the transport-layer
/// [`PendingInputEntry`] returned through the portal-op reply channel.
///
/// The enum fields (`delivery_state`, `content_classification`) are converted to
/// their snake_case wire spellings via serde so the MCP JSON-RPC response uses
/// the same vocabulary as the rest of the projection contract. The serde
/// round-trip cannot fail for these C-like enums; the `unwrap_or_else` fallback
/// is purely defensive and never taken in practice.
fn pending_input_entry_from_item(item: &PendingInputItem) -> PendingInputEntry {
    PendingInputEntry {
        input_id: item.input_id.clone(),
        projection_id: item.projection_id.clone(),
        submission_text: item.submission_text.clone(),
        submitted_at_wall_us: item.submitted_at_wall_us,
        expires_at_wall_us: item.expires_at_wall_us,
        delivery_state: serde_json::to_value(item.delivery_state)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        content_classification: serde_json::to_value(item.content_classification)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

/// Per-projection adapter state managed by the in-process driver.
struct DriveEntry {
    /// gRPC adapter that renders markdown and tracks tile state.
    adapter: ResidentGrpcPortalAdapter,
    /// Scene tile ID assigned to this portal, or `None` if not yet created.
    tile_scene_id: Option<SceneId>,
    /// Estimated total content height (px) from the last `RenderPortal` drain.
    ///
    /// A decrease in this value between consecutive drains is the authoritative
    /// signal that content was removed from the head of the visible transcript
    /// (head-trim event).  `notify_head_content_removed` is called with the
    /// difference so a scrolled-back viewport stays stable (spec §3.3 /
    /// hud-pkg2g, hud-66i1s).
    ///
    /// This is the height-only condition, mirroring the fix in
    /// `projection_authority.rs` (hud-hkaw2 / PR #779).
    prev_content_height_px: f32,
}

/// In-process state for the portal projection drive loop.
///
/// This is the runtime-side equivalent of `PortalDriveState` in the stdio
/// projection_authority binary. It holds one `ResidentGrpcPortalAdapter` per
/// attached projection session plus tile-to-scene mapping.
struct InProcessPortalDriveState {
    /// Per-projection drive entries keyed by `projection_id`.
    entries: HashMap<String, DriveEntry>,
    /// Scene tiles whose projection state has been accepted for detach/cleanup,
    /// but whose tile removal must wait until the next drain has scene access.
    pending_tile_removals: Vec<SceneId>,
    /// Current resolved design-token overrides (flat key → value strings).
    token_overrides: DesignTokenMap,
}

impl InProcessPortalDriveState {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            pending_tile_removals: Vec::new(),
            token_overrides: DesignTokenMap::new(),
        }
    }

    fn resolve_visual_tokens(&self) -> tze_hud_projection::resident_grpc::PortalVisualTokens {
        let resolved =
            tze_hud_config::tokens::resolve_tokens(&DesignTokenMap::new(), &self.token_overrides);
        portal_visual_tokens_from_part_tokens(&resolve_portal_tokens(&resolved))
    }

    fn attach(&mut self, projection_id: &str, lease_id: Vec<u8>) {
        let tokens = self.resolve_visual_tokens();
        let config = ResidentGrpcPortalConfig::new(lease_id);
        let adapter = ResidentGrpcPortalAdapter::with_tokens(config, tokens);
        self.entries.insert(
            projection_id.to_string(),
            DriveEntry {
                adapter,
                tile_scene_id: None,
                prev_content_height_px: 0.0,
            },
        );
    }

    fn detach(&mut self, projection_id: &str) {
        if let Some(entry) = self.entries.remove(projection_id)
            && let Some(tile_id) = entry.tile_scene_id
        {
            self.pending_tile_removals.push(tile_id);
        }
    }

    fn apply_token_map(&mut self, overrides: DesignTokenMap) {
        self.token_overrides = overrides;
        let tokens = self.resolve_visual_tokens();
        for entry in self.entries.values_mut() {
            entry.adapter.set_visual_tokens(tokens.clone());
        }
    }

    fn drain_pending_tile_removals(&mut self) -> Vec<SceneId> {
        self.pending_tile_removals.drain(..).collect()
    }
}

/// In-process projection authority driver.
///
/// Hosts a `ProjectionAuthority` inside the runtime process and runs the portal
/// drain loop on each `about_to_wait` call. See module-level doc for the full
/// architecture description.
pub struct InProcessPortalDriver {
    authority: ProjectionAuthority,
    drive: InProcessPortalDriveState,
    /// Scene lease ID for the driver's portal tiles.
    ///
    /// Granted once at driver construction (or lazily on first use) and renewed
    /// as needed.
    lease_id: Option<SceneId>,
    /// Publish-to-present latency bucket (hud-ttq97).
    ///
    /// Accumulates one sample per coalesced `RenderPortal` drain where
    /// `submitted_at_us > 0`.  Each sample is:
    ///
    /// ```text
    /// delta_us = now_us (drain wall-clock) − submitted_at_us (publish wall-clock)
    /// ```
    ///
    /// This measures the end-to-end time from when a portal update was first
    /// submitted (via `PublishOutput`) to when the drain loop materialises it
    /// for presentation — the primary latency signal for live task 5.7
    /// (hud-sonj6).
    ///
    /// Accessible via [`InProcessPortalDriver::portal_publish_to_present_latency`].
    portal_publish_to_present_latency: LatencyBucket,
    /// Count of drain cycles where the rate-window had not yet elapsed (hud-bq0gl.14).
    ///
    /// Incremented each time `take_due_portal_update` returns `Ok(None)` (the portal
    /// has a pending coalesced snapshot but its per-portal rate window has not elapsed
    /// yet).  A rising deferral count indicates the drain loop is being called more
    /// frequently than the rate window, or that portals are publishing faster than the
    /// rate window allows them to drain.
    ///
    /// Exposed via [`InProcessPortalDriver::drain_deferral_count`].
    drain_deferral_count: u64,
}

impl InProcessPortalDriver {
    /// Create a new in-process portal driver with default bounds.
    pub fn new() -> Self {
        let authority = ProjectionAuthority::new(ProjectionBounds::default())
            .expect("in-process projection authority initialisation must succeed");
        Self {
            authority,
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        }
    }

    /// Return a reference to the publish-to-present latency bucket (hud-ttq97).
    ///
    /// Each sample is the elapsed microseconds from `submitted_at_us` (the
    /// wall-clock time when `PublishOutput` was called) to `now_us` (the
    /// wall-clock time when the drain loop materialised the coalesced update).
    ///
    /// Only `RenderPortal` drains where `submitted_at_us > 0` contribute a
    /// sample; `CreatePortalTile` drains and updates with an unset submission
    /// time (0) are excluded.
    pub fn portal_publish_to_present_latency(&self) -> &LatencyBucket {
        &self.portal_publish_to_present_latency
    }

    /// Number of drain cycles where the rate-window had not yet elapsed (hud-bq0gl.14).
    ///
    /// Incremented each time the drain loop encounters a portal whose pending
    /// coalesced snapshot cannot yet be materialised because the per-portal rate
    /// window has not elapsed.  See [`Self::portal_publish_to_present_latency`]
    /// for end-to-end latency.
    pub fn drain_deferral_count(&self) -> u64 {
        self.drain_deferral_count
    }

    /// Attach a new projection session to the driver.
    ///
    /// Called when an LLM agent attaches a projection.  The `lease_id` is
    /// passed through to the `ResidentGrpcPortalAdapter` so that the resident
    /// gRPC proto messages carry the correct lease identity.
    pub fn attach_projection(&mut self, projection_id: &str, lease_id: Vec<u8>) {
        self.drive.attach(projection_id, lease_id);
    }

    /// Detach a projection session from the driver.
    pub fn detach_projection(&mut self, projection_id: &str) {
        self.drive.detach(projection_id);
    }

    /// Get a mutable reference to the hosted [`ProjectionAuthority`].
    ///
    /// Used by the gRPC session layer to dispatch operations (Attach,
    /// PublishOutput, Detach, etc.) into the authority.
    pub fn authority_mut(&mut self) -> &mut ProjectionAuthority {
        &mut self.authority
    }

    /// Push a geometry snapshot to the projection session that owns `tile_id`.
    ///
    /// Called by the window management layer after a hotkey resize or pointer-
    /// affordance resize updates tile bounds in the scene (§6b.4: geometry-
    /// snapshot producer wiring, hud-npq6g).
    ///
    /// The drive entries are searched by `tile_scene_id` to find the owning
    /// `projection_id`, then `ProjectionAuthority::push_geometry_snapshot` is
    /// called so the geometry batch is visible to the drain loop via
    /// `projected_portal_state`.
    ///
    /// Returns `true` if a matching session was found and the snapshot was
    /// accepted (sequence is strictly newer than any existing batch entry).
    /// Returns `false` if no session owns `tile_id` or the snapshot is stale.
    pub fn push_geometry_snapshot_for_tile(
        &mut self,
        tile_id: SceneId,
        snapshot: tze_hud_input::GeometrySnapshot,
    ) -> bool {
        // Reverse-lookup: find the projection_id whose drive entry owns this tile.
        let projection_id = self
            .drive
            .entries
            .iter()
            .find(|(_, entry)| entry.tile_scene_id == Some(tile_id))
            .map(|(id, _)| id.clone());

        let Some(projection_id) = projection_id else {
            // No portal session is attached to this tile — nothing to do.
            return false;
        };

        let adapter_snapshot = AdapterGeometrySnapshot {
            rect: AdapterPortalRect::from_f32(
                snapshot.rect.x,
                snapshot.rect.y,
                snapshot.rect.width,
                snapshot.rect.height,
            ),
            gesture_active: snapshot.gesture_active,
            sequence: snapshot.sequence,
        };

        self.authority
            .push_geometry_snapshot(&projection_id, adapter_snapshot)
    }

    /// Route a focused portal composer batch into the owning projection state.
    ///
    /// The gRPC input-event broadcast is still emitted by `windowed`, but a
    /// cooperative projection owner polls input from [`ProjectionAuthority`],
    /// not from that namespace-keyed broadcast bus. This bridge is the
    /// production submit path: it resolves the focused portal tile back to the
    /// attached projection, lets the resident adapter consume the draft batch,
    /// and maps any transactional submission into the authority pending-input
    /// queue.
    ///
    /// Returns `None` when `tile_id` is not owned by an attached in-process
    /// projection, or when the batch contains no transactional submission.
    pub fn submit_composer_batch_for_tile(
        &mut self,
        tile_id: SceneId,
        batch: &DraftNotificationBatch,
        submitted_at_wall_us: u64,
        expires_at_wall_us: Option<u64>,
        content_classification: ContentClassification,
    ) -> Option<PortalInputFeedback> {
        let projection_id = self
            .drive
            .entries
            .iter()
            .find(|(_, entry)| entry.tile_scene_id == Some(tile_id))
            .map(|(projection_id, _)| projection_id.clone())?;

        let adapter_batch = adapter_draft_batch_from_runtime(batch);
        let entry = self.drive.entries.get_mut(&projection_id)?;
        entry.adapter.consume_draft_batch(&adapter_batch);

        let submission = adapter_batch.submission.as_ref()?;
        let result = entry.adapter.submit_composer_text(
            &mut self.authority,
            &projection_id,
            submission.text.clone(),
            submitted_at_wall_us,
            expires_at_wall_us,
            content_classification,
        );
        Some(result.feedback)
    }

    /// Apply a new design-token override map, propagating to all live adapters.
    pub fn apply_token_map(&mut self, overrides: DesignTokenMap) {
        self.drive.apply_token_map(overrides);
    }

    /// Dispatch one [`PortalOp`] message received from the MCP channel.
    ///
    /// Called from `windowed.rs::drain_portal_ops()` on the event-loop thread,
    /// once per message before the normal `drain()` call.  Replies are sent
    /// back through the one-shot channel embedded in each variant.
    ///
    /// ## `Attach` behaviour
    ///
    /// 1. Calls `ProjectionAuthority::handle_attach` with a generated envelope.
    /// 2. On success, calls `self.attach_projection` so the drive state is ready
    ///    for the upcoming `drain()` iteration.
    /// 3. Returns the owner token through the reply channel.
    ///
    /// ## `PublishOutput` behaviour
    ///
    /// 1. Parses the optional `output_kind` / `content_classification`
    ///    snake_case strings into the projection enums (defaulting to
    ///    `assistant` / `private` when omitted) and threads through the
    ///    optional `coalesce_key`. An unrecognized enum value is rejected via
    ///    the reply channel before the authority is called.
    /// 2. Calls `ProjectionAuthority::handle_publish_output` with a generated
    ///    envelope and the caller-supplied owner token.
    /// 3. Forwards accept/reject through the reply channel.
    ///    The cadence coalescer inside the authority accumulates the update; the
    ///    normal `drain()` call in the same `about_to_wait` iteration (or the
    ///    next one) materialises it into the scene.
    pub fn dispatch_portal_op(&mut self, op: PortalOp) {
        let now_us = now_wall_us();
        match op {
            PortalOp::Attach {
                projection_id,
                display_name,
                idempotency_key,
                reply,
            } => {
                let request_id = uuid::Uuid::now_v7().to_string();
                let req = AttachRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::Attach,
                        projection_id: projection_id.clone(),
                        request_id,
                        client_timestamp_wall_us: now_us.max(1),
                    },
                    provider_kind: ProviderKind::Other,
                    display_name,
                    workspace_hint: None,
                    repository_hint: None,
                    icon_profile_hint: None,
                    content_classification: ContentClassification::Private,
                    hud_target: None,
                    idempotency_key,
                };
                let resp = self.authority.handle_attach(req, "mcp-portal", now_us);
                if resp.accepted {
                    // Wire the drive state so the drain loop can materialise tiles.
                    //
                    // Guard against idempotent re-attach: `handle_attach` returns
                    // `accepted=true` for a replay (same projection_id + matching
                    // idempotency_key) without re-creating the session. Re-running
                    // `attach_projection` here would `insert` a fresh `DriveEntry`
                    // (adapter with `tile_id() == None`, `tile_scene_id == None`),
                    // so the next drain would treat the portal as new and create a
                    // duplicate tile — orphaning/leaking the existing one. Only wire
                    // a drive entry the first time we see this projection_id.
                    if !self.drive.entries.contains_key(&projection_id) {
                        self.attach_projection(&projection_id, Vec::new());
                    }
                    tracing::info!(
                        proj_id = %projection_id,
                        "portal_op: Attach accepted — drive entry ensured"
                    );
                    let token = resp.owner_token.unwrap_or_default();
                    let _ = reply.send(Ok(token));
                } else {
                    let error_code = resp
                        .error_code
                        .unwrap_or(ProjectionErrorCode::ProjectionInternalError);
                    tracing::warn!(
                        proj_id = %projection_id,
                        error_code = %error_code,
                        "portal_op: Attach denied"
                    );
                    let _ =
                        reply.send(Err(PortalOpRejection::new(error_code, resp.status_summary)));
                }
            }

            PortalOp::PublishOutput {
                projection_id,
                owner_token,
                output_text,
                logical_unit_id,
                output_kind,
                content_classification,
                coalesce_key,
                reply,
            } => {
                // Parse the optional snake_case classification strings into the
                // projection enums. Omitted fields default safely
                // (assistant / private — privacy is safe-by-default). An
                // unrecognized value is rejected before the authority sees it.
                let output_kind = match parse_output_kind(output_kind.as_deref()) {
                    Ok(kind) => kind,
                    Err(reason) => {
                        let _ = reply.send(Err(PortalOpRejection::new(
                            ProjectionErrorCode::ProjectionInvalidArgument,
                            reason,
                        )));
                        return;
                    }
                };
                let content_classification =
                    match parse_content_classification(content_classification.as_deref()) {
                        Ok(classification) => classification,
                        Err(reason) => {
                            let _ = reply.send(Err(PortalOpRejection::new(
                                ProjectionErrorCode::ProjectionInvalidArgument,
                                reason,
                            )));
                            return;
                        }
                    };
                let request_id = uuid::Uuid::now_v7().to_string();
                let req = PublishOutputRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::PublishOutput,
                        projection_id: projection_id.clone(),
                        request_id,
                        client_timestamp_wall_us: now_us.max(1),
                    },
                    owner_token,
                    output_text,
                    output_kind,
                    content_classification,
                    logical_unit_id,
                    coalesce_key,
                };
                let resp = self
                    .authority
                    .handle_publish_output(req, "mcp-portal", now_us);
                if resp.accepted {
                    tracing::debug!(
                        proj_id = %projection_id,
                        coalesced = resp.coalesced_output_count,
                        "portal_op: PublishOutput accepted"
                    );
                    let _ = reply.send(Ok(()));
                } else {
                    let error_code = resp
                        .error_code
                        .unwrap_or(ProjectionErrorCode::ProjectionInternalError);
                    tracing::warn!(
                        proj_id = %projection_id,
                        error_code = %error_code,
                        "portal_op: PublishOutput denied"
                    );
                    let _ =
                        reply.send(Err(PortalOpRejection::new(error_code, resp.status_summary)));
                }
            }

            PortalOp::GetPendingInput {
                projection_id,
                owner_token,
                max_items,
                max_bytes,
                reply,
            } => {
                let request_id = uuid::Uuid::now_v7().to_string();
                let req = GetPendingInputRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::GetPendingInput,
                        projection_id: projection_id.clone(),
                        request_id,
                        client_timestamp_wall_us: now_us.max(1),
                    },
                    owner_token,
                    max_items,
                    max_bytes,
                };
                let resp = self
                    .authority
                    .handle_get_pending_input(req, "mcp-portal", now_us);
                if resp.accepted {
                    let items = resp
                        .pending_input
                        .iter()
                        .map(pending_input_entry_from_item)
                        .collect();
                    let batch = PendingInputBatch {
                        items,
                        remaining_count: resp.pending_remaining_count,
                        remaining_bytes: resp.pending_remaining_bytes,
                    };
                    tracing::debug!(
                        proj_id = %projection_id,
                        delivered = resp.pending_input.len(),
                        remaining = resp.pending_remaining_count,
                        "portal_op: GetPendingInput accepted"
                    );
                    let _ = reply.send(Ok(batch));
                } else {
                    let error_code = resp
                        .error_code
                        .unwrap_or(ProjectionErrorCode::ProjectionInternalError);
                    tracing::warn!(
                        proj_id = %projection_id,
                        error_code = %error_code,
                        "portal_op: GetPendingInput denied"
                    );
                    let _ =
                        reply.send(Err(PortalOpRejection::new(error_code, resp.status_summary)));
                }
            }

            PortalOp::AcknowledgeInput {
                projection_id,
                owner_token,
                input_id,
                ack_state,
                ack_message,
                not_before_wall_us,
                reply,
            } => {
                let ack_state = match parse_ack_state(&ack_state) {
                    Ok(state) => state,
                    Err(reason) => {
                        let _ = reply.send(Err(PortalOpRejection::new(
                            ProjectionErrorCode::ProjectionInvalidArgument,
                            reason,
                        )));
                        return;
                    }
                };
                let request_id = uuid::Uuid::now_v7().to_string();
                let req = AcknowledgeInputRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::AcknowledgeInput,
                        projection_id: projection_id.clone(),
                        request_id,
                        client_timestamp_wall_us: now_us.max(1),
                    },
                    owner_token,
                    input_id,
                    ack_state,
                    ack_message,
                    not_before_wall_us,
                };
                let resp = self
                    .authority
                    .handle_acknowledge_input(req, "mcp-portal", now_us);
                if resp.accepted {
                    tracing::debug!(
                        proj_id = %projection_id,
                        "portal_op: AcknowledgeInput accepted"
                    );
                    let _ = reply.send(Ok(()));
                } else {
                    let error_code = resp
                        .error_code
                        .unwrap_or(ProjectionErrorCode::ProjectionInternalError);
                    tracing::warn!(
                        proj_id = %projection_id,
                        error_code = %error_code,
                        "portal_op: AcknowledgeInput denied"
                    );
                    let _ =
                        reply.send(Err(PortalOpRejection::new(error_code, resp.status_summary)));
                }
            }

            PortalOp::Detach {
                projection_id,
                owner_token,
                reason,
                reply,
            } => {
                let request_id = uuid::Uuid::now_v7().to_string();
                let req = DetachRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::Detach,
                        projection_id: projection_id.clone(),
                        request_id,
                        client_timestamp_wall_us: now_us.max(1),
                    },
                    owner_token,
                    reason,
                };
                let resp = self.authority.handle_detach(req, "mcp-portal", now_us);
                if resp.accepted {
                    // The authority purged its session + coalescer entry. Drop
                    // the driver-side drive entry / tile mapping so the next
                    // drain does not attempt to render a removed surface and the
                    // projection_id is free to be re-attached cleanly.
                    self.detach_projection(&projection_id);
                    tracing::info!(
                        proj_id = %projection_id,
                        "portal_op: Detach accepted — drive entry dropped"
                    );
                    let _ = reply.send(Ok(()));
                } else {
                    let error_code = resp
                        .error_code
                        .unwrap_or(ProjectionErrorCode::ProjectionInternalError);
                    tracing::warn!(
                        proj_id = %projection_id,
                        error_code = %error_code,
                        "portal_op: Detach denied"
                    );
                    let _ =
                        reply.send(Err(PortalOpRejection::new(error_code, resp.status_summary)));
                }
            }

            PortalOp::Cleanup {
                projection_id,
                cleanup_authority,
                owner_token,
                operator_authority,
                reason,
                reply,
            } => {
                let cleanup_authority = match parse_cleanup_authority(&cleanup_authority) {
                    Ok(authority) => authority,
                    Err(reason) => {
                        let _ = reply.send(Err(PortalOpRejection::new(
                            ProjectionErrorCode::ProjectionInvalidArgument,
                            reason,
                        )));
                        return;
                    }
                };
                let request_id = uuid::Uuid::now_v7().to_string();
                let req = CleanupRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::Cleanup,
                        projection_id: projection_id.clone(),
                        request_id,
                        client_timestamp_wall_us: now_us.max(1),
                    },
                    cleanup_authority,
                    owner_token,
                    operator_authority,
                    reason,
                };
                let resp = self.authority.handle_cleanup(req, "mcp-portal", now_us);
                if resp.accepted {
                    // Cleanup purges authority state through either owner-token or
                    // operator-authority paths. Drop driver-side state as well so
                    // stale projected-session tiles are not rendered after cleanup.
                    self.detach_projection(&projection_id);
                    tracing::info!(
                        proj_id = %projection_id,
                        "portal_op: Cleanup accepted — drive entry dropped"
                    );
                    let _ = reply.send(Ok(()));
                } else {
                    let error_code = resp
                        .error_code
                        .unwrap_or(ProjectionErrorCode::ProjectionInternalError);
                    tracing::warn!(
                        proj_id = %projection_id,
                        error_code = %error_code,
                        "portal_op: Cleanup denied"
                    );
                    let _ =
                        reply.send(Err(PortalOpRejection::new(error_code, resp.status_summary)));
                }
            }
        }
    }

    /// Run the work-conserving portal drain loop and apply results to the scene.
    ///
    /// Called from `about_to_wait` after composer-draft flush. This is the
    /// production follow-tail wiring point (hud-2iup7).
    ///
    /// # Parameters
    ///
    /// - `scene`: mutable reference to the scene graph (acquired by the caller
    ///   with `try_lock`; not acquired here to avoid blocking the main thread).
    /// - `input_processor`: mutable reference to the main-thread `InputProcessor`.
    /// - `tab_id`: the active tab in which new portal tiles should be created.
    ///   If `None` (no active tab), `CreatePortalTile` drains are skipped.
    ///
    /// # Panic safety
    ///
    /// The drain body is wrapped in `catch_unwind`. A panic resets drive state
    /// and logs an error without propagating to the event loop.
    pub fn drain(
        &mut self,
        scene: &mut SceneGraph,
        input_processor: &mut InputProcessor,
        tab_id: Option<SceneId>,
    ) {
        let now_us = now_wall_us();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.drain_inner(scene, input_processor, tab_id, now_us)
        }));
        if let Err(payload) = result {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic payload".to_string()
            };
            tracing::error!(
                error = %msg,
                "portal projection driver drain panicked — drive state reset"
            );
            // Reset all drive entries to prevent stale state accumulation.
            self.drive = InProcessPortalDriveState::new();
        }
    }

    /// Inner drain implementation.
    ///
    /// `now_us` is the caller-supplied wall-clock timestamp in microseconds
    /// since UNIX epoch.  Production callers pass `now_wall_us()`; tests pass a
    /// deterministic value so time-dependent behaviour is reproducible without
    /// wall-clock coupling.
    fn drain_inner(
        &mut self,
        scene: &mut SceneGraph,
        input_processor: &mut InputProcessor,
        tab_id: Option<SceneId>,
        now_us: u64,
    ) {
        self.drain_pending_tile_removals(scene);

        let policy = ProjectedPortalPolicy::permit_all();
        // Local counters for this drain cycle — emitted as a tracing event after
        // the loop for portal health observability (hud-bq0gl.14).
        let mut cycle_updates: u32 = 0;
        let mut cycle_deferrals: u32 = 0;
        // Per-cycle iteration guard (hud-bsr7u). Bounds the number of loop
        // iterations so a session/coalescer divergence can never busy-spin the
        // event loop (which runs this drain under the scene lock + ControlFlow::Poll).
        let mut iterations: u32 = 0;

        loop {
            // Defense-in-depth backstop: never iterate more than the cap per tick.
            // Reaching this indicates a divergence bug (the loop should normally
            // be work-conserving and terminate when the coalescer is drained).
            if iterations >= MAX_PORTAL_DRAIN_ITERATIONS_PER_CYCLE {
                tracing::error!(
                    iterations,
                    cap = MAX_PORTAL_DRAIN_ITERATIONS_PER_CYCLE,
                    "portal drain hit per-cycle iteration cap — aborting tick to \
                     avoid wedging the event loop (likely session/coalescer divergence)"
                );
                break;
            }
            iterations = iterations.saturating_add(1);

            // Round-robin fairness oracle (tasks.md §5.1 / §5.4).
            let Some(proj_id) = self.authority.next_due_projection_id() else {
                break;
            };

            // Materialise the coalesced update for this portal.
            let update = match self.authority.take_due_portal_update(&proj_id, now_us) {
                Ok(Some(update)) => update,
                Ok(None) => {
                    // Rate-window not yet elapsed — count and exit the loop.
                    self.drain_deferral_count = self.drain_deferral_count.saturating_add(1);
                    cycle_deferrals = cycle_deferrals.saturating_add(1);
                    break;
                }
                Err(_) => {
                    // Projection not found or expired. `take_due_portal_update`
                    // returns early on the session lookup BEFORE it consumes the
                    // coalescer entry, so an orphaned entry (session gone but
                    // coalescer still pending — e.g. operator cleanup, hud-bsr7u)
                    // would otherwise be returned again by the next
                    // `next_due_projection_id` and busy-spin this loop forever.
                    // Discard the coalescer entry here so it cannot recur, then
                    // clean up the adapter.
                    self.authority.discard_portal_coalescer_entry(&proj_id);
                    self.detach_projection(&proj_id);
                    continue;
                }
            };

            // Build the full projected portal state for rendering.
            let Some(state) = self.authority.projected_portal_state(&proj_id, &policy) else {
                // Session was removed between take_due and state query (race).
                self.detach_projection(&proj_id);
                continue;
            };

            // Check if the drive entry exists and what kind of command to issue.
            // We do this before taking a mutable borrow of drive.entries so that
            // `ensure_driver_lease` (which borrows `self` mutably) can be called
            // without a concurrent mutable borrow through `entry`.
            let entry_exists = self.drive.entries.contains_key(&proj_id);
            if !entry_exists {
                // No drive entry (non-portal surface). Skip silently.
                continue;
            }
            let needs_create = self
                .drive
                .entries
                .get(&proj_id)
                .map(|e| e.adapter.tile_id().is_none())
                .unwrap_or(false);

            // Determine command kind: CreatePortalTile if tile not yet created.
            let command_kind = if needs_create {
                ResidentGrpcPortalCommandKind::CreatePortalTile
            } else {
                ResidentGrpcPortalCommandKind::RenderPortal
            };

            match command_kind {
                ResidentGrpcPortalCommandKind::CreatePortalTile => {
                    // Requires an active tab to host the new tile.
                    let Some(active_tab) = tab_id else {
                        tracing::warn!(
                            proj_id = %proj_id,
                            "portal drain: no active tab — CreatePortalTile deferred"
                        );
                        continue;
                    };

                    // Ensure the driver has an active lease in the scene.
                    // Must be called BEFORE taking entry borrow to avoid borrow conflict.
                    let lease_id = match self.ensure_driver_lease(scene) {
                        Some(id) => id,
                        None => {
                            tracing::warn!(
                                proj_id = %proj_id,
                                "portal drain: could not obtain driver lease — \
                                 CreatePortalTile deferred"
                            );
                            continue;
                        }
                    };

                    // Now take the mutable entry borrow after ensure_driver_lease is done.
                    let Some(entry) = self.drive.entries.get_mut(&proj_id) else {
                        continue;
                    };

                    // Derive initial bounds from the adapter's configured viewport.
                    // Use expanded dimensions (720×360) as the default starting rect;
                    // subsequent geometry snapshots will provide accurate live dimensions.
                    let viewport_h = entry.adapter.config_viewport_height(state.presentation);
                    // Width falls back to a reasonable default (720px expanded, 420px
                    // collapsed). We derive it from the height ratio to stay consistent with
                    // the adapter defaults without exposing config internals.
                    let viewport_w = match state.presentation {
                        tze_hud_projection::ProjectedPortalPresentation::Expanded => 720.0_f32,
                        tze_hud_projection::ProjectedPortalPresentation::Collapsed => 420.0_f32,
                    };
                    let bounds = Rect::new(0.0, 0.0, viewport_w, viewport_h);

                    match scene.create_tile(
                        active_tab,
                        PORTAL_DRIVER_NAMESPACE,
                        lease_id,
                        bounds,
                        PORTAL_Z_ORDER,
                    ) {
                        Ok(tile_scene_id) => {
                            // Register scroll config so follow-tail tracking works.
                            let _ = scene.register_tile_scroll_config(
                                tile_scene_id,
                                TileScrollConfig {
                                    scrollable_x: false,
                                    scrollable_y: true,
                                    content_width: None,
                                    content_height: None,
                                },
                            );

                            // Record the tile ID in the adapter (little-endian bytes per
                            // RFC 0001 §4.1 wire encoding).
                            let tile_id_le = tile_scene_id.to_bytes_le().to_vec();
                            entry.adapter.record_created_tile(tile_id_le);
                            entry.tile_scene_id = Some(tile_scene_id);

                            tracing::debug!(
                                proj_id = %proj_id,
                                tile_id = ?tile_scene_id,
                                "portal drain: CreatePortalTile — tile created in scene"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                proj_id = %proj_id,
                                error = ?e,
                                "portal drain: CreatePortalTile scene creation failed"
                            );
                        }
                    }
                }

                ResidentGrpcPortalCommandKind::RenderPortal
                | ResidentGrpcPortalCommandKind::ReusePortalTile => {
                    // hud-2iup7: Wire notify_tile_content_appended (spec §3.2 / §3.3).
                    //
                    // The InputProcessor advances follow-tail when the tile is AtTail
                    // (spec §3.2) and preserves the offset when scrolled-back (spec §3.3).
                    // We only need to call it — the decision is fully encapsulated there.
                    let Some(entry) = self.drive.entries.get_mut(&proj_id) else {
                        continue;
                    };
                    let Some(tile_scene_id) = entry.tile_scene_id else {
                        tracing::warn!(
                            proj_id = %proj_id,
                            "portal drain: RenderPortal but tile_scene_id is None — skipping notify"
                        );
                        continue;
                    };

                    // Compute append geometry (mirrors drain_and_emit_portal_updates in
                    // projection_authority.rs §1316-1363).
                    //
                    // line_height_px  = transcript_font_size_px × PORTAL_LINE_HEIGHT_MULTIPLIER
                    // new_content_h   = total_rendered_lines × line_height_px
                    // viewport_h      = geometry_batch.latest.rect.height_px when present;
                    //                   else adapter configured bounds for current presentation
                    let line_height_px = entry.adapter.visual_tokens().transcript_font_size_px
                        * PORTAL_LINE_HEIGHT_MULTIPLIER;

                    // Count actual rendered lines across all visible units. A
                    // TranscriptUnit may contain embedded newlines, so we use
                    // `.lines().count().max(1)` per unit to avoid underestimation.
                    let total_lines: usize = update
                        .visible_transcript
                        .iter()
                        .map(|unit| unit.output_text.lines().count().max(1))
                        .sum::<usize>()
                        .max(1);

                    let new_content_height_px = total_lines as f32 * line_height_px;

                    // Prefer the live geometry snapshot (most accurate after a resize),
                    // fall back to adapter configured bounds for the current presentation.
                    let viewport_height_px = state
                        .geometry_batch
                        .as_ref()
                        .and_then(|gb| gb.latest)
                        .map(|snap| snap.rect.height_px as f32)
                        .unwrap_or_else(|| {
                            entry.adapter.config_viewport_height(state.presentation)
                        });

                    // hud-ttq97: record publish-to-present latency for this portal drain.
                    //
                    // `submitted_at_us` is the wall-clock time when `PublishOutput` was
                    // called (set by the cadence coalescer in `record_append`).  `now_us`
                    // is the drain wall-clock time supplied by the caller.  The delta is
                    // the end-to-end publish→present elapsed time for this coalesced update.
                    //
                    // Guard: skip when `submitted_at_us == 0` (coalescer returned the
                    // `unwrap_or(0)` sentinel from `peek_submitted_at` — no submission
                    // timestamp was recorded yet) or when `now_us < submitted_at_us`
                    // (should not happen in production, but guards against test fixtures
                    // that supply out-of-order timestamps).
                    if update.submitted_at_us > 0 && now_us >= update.submitted_at_us {
                        let delta_us = now_us - update.submitted_at_us;
                        self.portal_publish_to_present_latency.record(delta_us);
                        tracing::trace!(
                            proj_id = %proj_id,
                            submitted_at_us = update.submitted_at_us,
                            now_us,
                            delta_us,
                            "portal drain: publish-to-present latency recorded"
                        );
                    }

                    // hud-pkg2g / hud-66i1s: detect head-trim and call
                    // notify_head_content_removed.
                    //
                    // A content-height decrease between consecutive `RenderPortal`
                    // drains indicates that content was removed from the head of the
                    // visible transcript.  Two trim sites produce this:
                    //   1. PortalCadenceCoalescer::record_append (64 KiB cap): drops
                    //      oldest bytes from the payload before storing the snapshot.
                    //   2. visible_transcript_window (16 KiB cap): slices the retained
                    //      transcript to the newest max_visible_transcript_bytes bytes.
                    //
                    // Detection criterion: new_content_height_px < prev_content_height_px.
                    // The height is the authoritative signal because the runtime caller
                    // uses new_content_height_px (from append_geometry) as the total
                    // content height for scroll accounting.  Any decrease — regardless
                    // of whether the byte count also decreased — means the caller must
                    // receive a notify_head_content_removed call BEFORE
                    // notify_tile_content_appended, so that
                    // ScrollTileState::total_content_height_px is correct when the
                    // follow-tail bound is recomputed (spec §3.3, hud-66i1s fix).
                    //
                    // The previous dual-condition (bytes_shrank && height_shrank) was
                    // overly restrictive: it would miss a height shrink that occurred
                    // without a corresponding byte-count decrease (e.g., a many-newline
                    // unit evicted by a flat unit of equal or greater byte count).  This
                    // mirrors the fix applied to projection_authority.rs in PR #779
                    // (hud-hkaw2).
                    let prev_height = entry.prev_content_height_px;
                    if new_content_height_px < prev_height {
                        let removed_px = prev_height - new_content_height_px;
                        let trim_changed =
                            input_processor.notify_head_content_removed(tile_scene_id, removed_px);
                        tracing::debug!(
                            proj_id = %proj_id,
                            tile_id = ?tile_scene_id,
                            removed_px,
                            prev_height_px = prev_height,
                            new_height_px = new_content_height_px,
                            scroll_adjusted = trim_changed,
                            "portal drain: head-trim detected — notify_head_content_removed"
                        );
                    }
                    // Update per-portal height tracking for the next drain cycle.
                    entry.prev_content_height_px = new_content_height_px;

                    // spec §3.2 / §3.3: call notify_tile_content_appended.
                    // - AtTail  → InputProcessor advances follow-tail (§3.2).
                    // - ScrolledBack → InputProcessor is a no-op (§3.3).
                    let changed = input_processor.notify_tile_content_appended(
                        tile_scene_id,
                        new_content_height_px,
                        viewport_height_px,
                        line_height_px,
                        scene,
                    );

                    tracing::trace!(
                        proj_id = %proj_id,
                        tile_id = ?tile_scene_id,
                        new_content_height_px,
                        viewport_height_px,
                        line_height_px,
                        scroll_advanced = changed,
                        "portal drain: RenderPortal — notify_tile_content_appended"
                    );

                    // §6b.4: consume the geometry batch after delivery so that
                    // `projected_portal_state` does not re-deliver the same snapshot
                    // on the next drain cycle. The caller (push_geometry_snapshot_for_tile)
                    // writes new snapshots; old ones must be cleared after being read.
                    if state.geometry_batch.is_some() {
                        self.authority.consume_geometry_batch(&proj_id);
                    }
                }

                ResidentGrpcPortalCommandKind::ReleaseLease => {
                    // ReleaseLease: no content notification needed.
                    tracing::debug!(
                        proj_id = %proj_id,
                        "portal drain: ReleaseLease — no notify required"
                    );
                }
            }

            // Count each successfully materialised update (hud-bq0gl.14).
            cycle_updates = cycle_updates.saturating_add(1);
        }

        // Portal health snapshot — emitted at debug level after each drain cycle
        // (hud-bq0gl.14).  Carries:
        //   • per-cycle update/deferral counts
        //   • cumulative drain-deferral counter
        //   • coalescer fairness stats (portal_count, total_taken, total_coalesced)
        //   • publish-to-present latency percentiles (p50/p95/p99)
        //
        // Emit only when there was activity (at least one update or deferral) to
        // avoid spamming on idle drain cycles.
        if cycle_updates > 0 || cycle_deferrals > 0 {
            let lat = &self.portal_publish_to_present_latency;
            tracing::debug!(
                cycle_updates,
                cycle_deferrals,
                cumulative_deferrals = self.drain_deferral_count,
                coalescer_portal_count = self.authority.coalescer_portal_count(),
                coalescer_pending = self.authority.coalescer_pending_portal_count(),
                coalescer_total_taken = self.authority.coalescer_total_taken(),
                coalescer_total_coalesced = self.authority.coalescer_total_coalesced(),
                latency_p50_us = lat.p50(),
                latency_p95_us = lat.p95(),
                latency_p99_us = lat.p99(),
                latency_sample_count = lat.samples.len(),
                "portal drain: health snapshot"
            );
        }
    }

    /// Ensure the driver has an active lease in the scene graph.
    ///
    /// Returns the cached lease only if it is still **Active** (mutations
    /// allowed); otherwise grants a fresh one and caches it.
    ///
    /// The active-state check (not mere map presence) is load-bearing for the
    /// post-grace re-attach path: when a prior portal's lease orphans and its
    /// grace period elapses, `SceneGraph::expire_leases` removes the lease's
    /// tiles but leaves the lease entry resident in the map as `Expired`
    /// (terminal state is recorded in place, not pruned). A session attaching
    /// *after* that grace expiry must therefore start a FRESH portal under a new
    /// lease. If we reused the cached-but-Expired lease here, the subsequent
    /// `create_tile` would fail its `require_active_lease` check and the new
    /// session would silently get no portal — never reviving the removed surface
    /// nor presenting pre-death content as live, but also never coming back.
    fn ensure_driver_lease(&mut self, scene: &mut SceneGraph) -> Option<SceneId> {
        if let Some(lease_id) = self.lease_id {
            // Reuse only if the cached lease is still Active. A terminal
            // (Expired/Revoked) or orphaned/suspended lease must NOT be reused —
            // `lease_capabilities` would still return `Some` for an Expired
            // lease left resident by grace-period reaping, so we check liveness.
            if scene.lease_is_active(&lease_id) {
                return Some(lease_id);
            }
        }
        // Grant a new lease for the portal driver.
        // 24-hour TTL in milliseconds (long-lived resident service).
        let new_lease = scene.grant_lease(
            PORTAL_DRIVER_NAMESPACE,
            86_400_000, // 24h in ms
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        self.lease_id = Some(new_lease);
        Some(new_lease)
    }

    fn drain_pending_tile_removals(&mut self, scene: &mut SceneGraph) {
        for tile_id in self.drive.drain_pending_tile_removals() {
            match scene.delete_tile(tile_id, PORTAL_DRIVER_NAMESPACE) {
                Ok(()) => {
                    tracing::info!(
                        tile_id = ?tile_id,
                        "portal drain: removed detached projection tile"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        tile_id = ?tile_id,
                        error = ?error,
                        "portal drain: failed to remove detached projection tile"
                    );
                }
            }
        }
    }
}

impl Default for InProcessPortalDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current wall-clock timestamp in microseconds since UNIX epoch.
fn now_wall_us() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_input::{DraftNotificationBatch, DraftSubmission, InputProcessor};
    use tze_hud_projection::{
        AcknowledgeInputRequest, AttachRequest, ContentClassification, GetPendingInputRequest,
        InputAckState, OperationEnvelope, OutputKind, PORTAL_UPDATE_RATE_WINDOW_WALL_US,
        ProjectionBounds, ProjectionOperation, ProviderKind, PublishOutputRequest,
    };
    use tze_hud_scene::SceneGraph;

    fn test_envelope(
        operation: ProjectionOperation,
        projection_id: &str,
        request_id: &str,
    ) -> OperationEnvelope {
        OperationEnvelope {
            operation,
            projection_id: projection_id.to_string(),
            request_id: request_id.to_string(),
            client_timestamp_wall_us: 1,
        }
    }

    fn attach_and_get_token(driver: &mut InProcessPortalDriver, projection_id: &str) -> String {
        let resp = driver.authority_mut().handle_attach(
            AttachRequest {
                envelope: test_envelope(
                    ProjectionOperation::Attach,
                    projection_id,
                    &format!("attach-{projection_id}"),
                ),
                provider_kind: ProviderKind::Claude,
                display_name: format!("Test {projection_id}"),
                workspace_hint: None,
                repository_hint: None,
                icon_profile_hint: None,
                content_classification: ContentClassification::Private,
                hud_target: None,
                idempotency_key: None,
            },
            "test-caller",
            1000,
        );
        assert!(resp.accepted, "attach must be accepted");
        resp.owner_token
            .expect("owner_token must be present after attach")
    }

    fn publish(
        driver: &mut InProcessPortalDriver,
        projection_id: &str,
        owner_token: &str,
        text: &str,
        ts: u64,
    ) {
        let resp = driver.authority_mut().handle_publish_output(
            PublishOutputRequest {
                envelope: test_envelope(
                    ProjectionOperation::PublishOutput,
                    projection_id,
                    &format!("pub-{projection_id}-{ts}"),
                ),
                owner_token: owner_token.to_string(),
                output_text: text.to_string(),
                output_kind: OutputKind::Assistant,
                content_classification: ContentClassification::Private,
                logical_unit_id: Some(format!("unit-{ts}")),
                coalesce_key: None,
            },
            "test-caller",
            ts,
        );
        assert!(resp.accepted, "publish_output must be accepted");
    }

    #[test]
    fn portal_composer_submission_enters_pending_input_queue_and_can_be_acknowledged() {
        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        };

        let projection_id = "proj-composer-return";
        let token = attach_and_get_token(&mut driver, projection_id);
        driver.attach_projection(projection_id, Vec::new());

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        publish(
            &mut driver,
            projection_id,
            &token,
            "assistant is ready",
            100,
        );
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);
        let tile_id = driver
            .drive
            .entries
            .get(projection_id)
            .expect("drive entry must exist")
            .tile_scene_id
            .expect("drain must create a portal tile");

        let mut batch = DraftNotificationBatch::new();
        batch.record_submission(DraftSubmission {
            text: "please summarize the current diff".to_string(),
            sequence: 7,
        });

        let feedback = driver
            .submit_composer_batch_for_tile(
                tile_id,
                &batch,
                1_000,
                Some(2_000),
                ContentClassification::Private,
            )
            .expect("focused portal tile must map to an attached projection");
        assert_eq!(
            feedback.pending_input_count, 1,
            "submission must create one semantic pending-input item"
        );

        let poll = driver.authority_mut().handle_get_pending_input(
            GetPendingInputRequest {
                envelope: test_envelope(
                    ProjectionOperation::GetPendingInput,
                    projection_id,
                    "poll-composer-input",
                ),
                owner_token: token.clone(),
                max_items: Some(1),
                max_bytes: Some(4_096),
            },
            "codex-session",
            1_100,
        );
        assert!(poll.accepted, "pending-input poll must be accepted");
        assert_eq!(poll.pending_input.len(), 1);
        assert_eq!(
            poll.pending_input[0].submission_text,
            "please summarize the current diff"
        );

        let input_id = poll.pending_input[0].input_id.clone();
        let ack = driver.authority_mut().handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: test_envelope(
                    ProjectionOperation::AcknowledgeInput,
                    projection_id,
                    "ack-composer-input",
                ),
                owner_token: token,
                input_id,
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "codex-session",
            1_200,
        );
        assert!(ack.accepted, "handled acknowledgement must be accepted");

        let state = driver
            .authority_mut()
            .projected_portal_state(projection_id, &ProjectedPortalPolicy::permit_all())
            .expect("projection state must still exist");
        assert_eq!(
            state.pending_input_count,
            Some(0),
            "handled input must no longer count as waiting for the agent"
        );
    }

    /// Regression guard for the `notify_tile_content_appended` wiring at the
    /// RenderPortal drain site (portal_projection_driver.rs, spec §3.2).
    ///
    /// After a RenderPortal drain with content that overflows the viewport the
    /// `InputProcessor` must have advanced follow-tail and recorded the tile as
    /// at-tail in the scene graph.  This assertion only passes when the driver
    /// actually calls `notify_tile_content_appended`; removing that call causes
    /// `tile_follow_tail_at_tail` to remain `false` and the test to fail.
    ///
    /// Uses a deterministic `now_us` via `drain_inner` so the rate-window
    /// comparison is clock-independent.
    #[test]
    fn drain_render_portal_notify_tile_content_appended_wiring() {
        use tze_hud_projection::AdapterGeometrySnapshot;
        use tze_hud_projection::AdapterPortalRect;

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        };

        let token = attach_and_get_token(&mut driver, "proj-a");
        driver.attach_projection("proj-a", Vec::new());

        // Derive line height from the adapter's font token so the geometry is
        // consistent with whatever font size the design tokens resolve to.
        let font_size_px = driver
            .drive
            .entries
            .get("proj-a")
            .unwrap()
            .adapter
            .visual_tokens()
            .transcript_font_size_px;
        let line_h = font_size_px * PORTAL_LINE_HEIGHT_MULTIPLIER;
        // One-line viewport: content with 10 units overflows and forces scroll.
        let viewport_h = (1.0 * line_h).ceil();

        // Push a geometry snapshot so the RenderPortal path uses the small
        // viewport — otherwise it falls back to the adapter configured height
        // which may be large enough that content never overflows.
        driver.authority_mut().push_geometry_snapshot(
            "proj-a",
            AdapterGeometrySnapshot {
                rect: AdapterPortalRect {
                    x_px: 0,
                    y_px: 0,
                    width_px: 600,
                    height_px: viewport_h as i32,
                },
                gesture_active: false,
                sequence: 1,
            },
        );

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // Drain 1 (CreatePortalTile): publish at t=100 µs, drain at t=200 µs.
        publish(&mut driver, "proj-a", &token, "line-0", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let tile_id = driver
            .drive
            .entries
            .get("proj-a")
            .expect("drive entry must exist")
            .tile_scene_id
            .expect("tile must be created after first drain");

        // Confirm scroll config was registered.
        assert!(
            scene.tile_scroll_config(tile_id).is_some(),
            "tile must have scroll config after CreatePortalTile drain"
        );

        // Resize the tile to the small viewport so overflow is detectable.
        let _ = scene.update_tile_bounds(
            tile_id,
            Rect::new(0.0, 0.0, 600.0, viewport_h),
            PORTAL_DRIVER_NAMESPACE,
        );

        // Drain 2 (RenderPortal): publish 9 more lines at timestamps inside the
        // rate window, then drain at a time well past PORTAL_UPDATE_RATE_WINDOW_WALL_US
        // so the coalescer releases the update.
        let base_ts = PORTAL_UPDATE_RATE_WINDOW_WALL_US;
        for i in 1..=9_u64 {
            publish(
                &mut driver,
                "proj-a",
                &token,
                &format!("line-{i}"),
                base_ts + i * 5,
            );
        }
        // now_us is past the rate window — the authority will release the update.
        let drain2_now_us = base_ts + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain2_now_us);

        // Primary regression assertion: notify_tile_content_appended must have been
        // called — it is the only code path that sets tile_follow_tail_at_tail.
        // If the call at the RenderPortal drain site is removed, this assertion fails.
        assert!(
            scene.tile_follow_tail_at_tail(tile_id),
            "spec §3.2 regression: tile must be at-tail after RenderPortal drain \
             with overflowing content — removing notify_tile_content_appended wiring \
             at the drain site causes this assertion to fail"
        );

        // Secondary: the tile must still exist (no panic / state reset).
        assert!(
            driver
                .drive
                .entries
                .get("proj-a")
                .unwrap()
                .tile_scene_id
                .is_some(),
            "tile must persist after RenderPortal drain"
        );
    }

    /// Follow-tail (spec §3.2): after a RenderPortal drain on an at-tail tile,
    /// the scroll offset advances and `tile_follow_tail_at_tail` is still true.
    ///
    /// Uses deterministic time injection via `drain_inner` (injectable clock seam
    /// introduced by hud-28j7v).
    #[test]
    fn follow_tail_advances_on_append_at_tail_tile() {
        use tze_hud_projection::AdapterGeometrySnapshot;
        use tze_hud_projection::AdapterPortalRect;

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        };

        let token = attach_and_get_token(&mut driver, "proj-b");
        driver.attach_projection("proj-b", Vec::new());

        // Derive line height from the adapter's font token.
        let font_size_px = driver
            .drive
            .entries
            .get("proj-b")
            .unwrap()
            .adapter
            .visual_tokens()
            .transcript_font_size_px;
        let line_h = font_size_px * PORTAL_LINE_HEIGHT_MULTIPLIER;
        let viewport_h = (1.0 * line_h).ceil();

        // Push a geometry snapshot so the drain uses the small viewport.
        driver.authority_mut().push_geometry_snapshot(
            "proj-b",
            AdapterGeometrySnapshot {
                rect: AdapterPortalRect {
                    x_px: 0,
                    y_px: 0,
                    width_px: 600,
                    height_px: viewport_h as i32,
                },
                gesture_active: false,
                sequence: 1,
            },
        );

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // Drain 1 (CreatePortalTile): deterministic t=200 µs.
        publish(&mut driver, "proj-b", &token, "unit-0", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let tile_id = driver
            .drive
            .entries
            .get("proj-b")
            .unwrap()
            .tile_scene_id
            .expect("tile must be created");

        // Resize tile to the 1-line viewport used by the geometry snapshot.
        let _ = scene.update_tile_bounds(
            tile_id,
            Rect::new(0.0, 0.0, 600.0, viewport_h),
            PORTAL_DRIVER_NAMESPACE,
        );

        // The tile's scroll config was registered during CreatePortalTile drain.
        // Confirm it's present before draining the RenderPortal step.
        assert!(
            scene.tile_scroll_config(tile_id).is_some(),
            "tile must have scroll config before RenderPortal drain"
        );

        // Publish 9 more units with timestamps inside the first rate window so the
        // coalescer accumulates them, then drain at a time past the window.
        let base_ts = PORTAL_UPDATE_RATE_WINDOW_WALL_US;
        for i in 1..=9_u64 {
            publish(
                &mut driver,
                "proj-b",
                &token,
                &format!("unit-{i}"),
                base_ts + i * 10 + 5,
            );
        }

        // Drain 2 (RenderPortal): now_us is past two rate windows so the update
        // is guaranteed due.  Deterministic timestamp — no wall-clock dependency.
        let drain2_now_us = base_ts + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain2_now_us);

        // The driver must have called notify_tile_content_appended: with 10 lines
        // in a 1-line viewport the content overflows and at-tail stays true.
        assert!(
            scene.tile_follow_tail_at_tail(tile_id),
            "spec §3.2: tile must be at-tail after RenderPortal drain with overflow"
        );

        // The scroll offset must be positive: follow-tail advanced to show the tail.
        let (_, scroll_y) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            scroll_y > 0.0,
            "spec §3.2: scroll offset must advance for at-tail tile after overflow \
             (got scroll_y={scroll_y})"
        );

        // Drive entry must survive (no panic / state reset).
        assert!(
            driver
                .drive
                .entries
                .get("proj-b")
                .unwrap()
                .tile_scene_id
                .is_some(),
            "tile must persist after second drain"
        );
    }

    /// spec §3.3 — `notify_tile_content_appended` on a scrolled-back tile must
    /// NOT change the scroll offset (InputProcessor enforces this; driver just calls it).
    #[test]
    fn scrolled_back_tile_offset_is_stable_on_append() {
        use tze_hud_input::ScrollEvent;
        use tze_hud_scene::TileScrollConfig;

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let viewport_h = 200.0_f32;
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(0.0, 0.0, 600.0, viewport_h),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(
                tile_id,
                TileScrollConfig {
                    scrollable_x: false,
                    scrollable_y: true,
                    content_width: None,
                    content_height: None,
                },
            )
            .unwrap();

        let mut processor = InputProcessor::new();

        // Prime at-tail with large content.
        let line_h = 18.2_f32;
        let large_content = viewport_h * 5.0;
        processor.notify_tile_content_appended(
            tile_id,
            large_content,
            viewport_h,
            line_h,
            &mut scene,
        );

        // User scrolls back.  The tile occupies (0,0)→(600,200); use (300,100)
        // as the event coordinate so hit-testing resolves to this tile.
        let scroll_ev = ScrollEvent {
            x: 300.0,
            y: 100.0,
            delta_x: 0.0,
            delta_y: -50.0,
        };
        processor.process_scroll_event(&scroll_ev, &mut scene);

        let (_, pre_y) = scene.tile_scroll_offset_local(tile_id);

        // spec §3.3: another append must NOT change the scrolled-back position.
        let changed = processor.notify_tile_content_appended(
            tile_id,
            large_content + line_h,
            viewport_h,
            line_h,
            &mut scene,
        );

        let (_, post_y) = scene.tile_scroll_offset_local(tile_id);

        assert!(
            !changed,
            "spec §3.3: scrolled-back tile must not advance after append"
        );
        assert!(
            (post_y - pre_y).abs() < f32::EPSILON,
            "spec §3.3: scroll offset must be unchanged; pre={pre_y} post={post_y}"
        );
    }

    /// Regression guard for the §6b.4 geometry-snapshot producer wiring (hud-npq6g).
    ///
    /// `push_geometry_snapshot_for_tile` must forward a resize geometry snapshot
    /// into the `ProjectionAuthority` so that the drain loop consumer (the
    /// `geometry_batch` field on `ProjectedPortalState`) reads it on the next
    /// drain cycle.
    ///
    /// Precondition: `push_geometry_snapshot_for_tile` had zero production callers
    /// before hud-npq6g.  This test fails on the pre-wiring code where the only
    /// caller was a bin test (the authority snapshot therefore stays `None` in
    /// production, and the drain always falls back to the adapter-configured height).
    ///
    /// The test drives the full producer → consumer path:
    ///   1. Attach a projection session and drain to create the portal tile in
    ///      the scene.
    ///   2. Call `push_geometry_snapshot_for_tile` with a tiny (1-px) viewport
    ///      height — this is what the window management layer now does after a
    ///      hotkey resize.
    ///   3. Drain again with overflowing content and confirm that:
    ///      (a) the drain used the geometry-batch viewport — scroll advanced and
    ///      `tile_follow_tail_at_tail` is true, and
    ///      (b) the authority's pending batch was consumed so it is `None`
    ///      after the drain (no stale re-delivery).
    ///
    /// Removing the `push_geometry_snapshot_for_tile` call (or the wiring in
    /// windowed.rs) causes the drain to fall back to the adapter-configured height
    /// (typically 360 px).  With a 10-line transcript the content height is much
    /// less than 360 px, so `tile_follow_tail_at_tail` stays `false` and the
    /// first assertion fails.
    #[test]
    fn hotkey_resize_geometry_reaches_drain_consumer() {
        use tze_hud_input::{GeometrySnapshot, PortalRect};
        use tze_hud_projection::ProjectedPortalPolicy;

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        };

        let token = attach_and_get_token(&mut driver, "proj-resize");
        driver.attach_projection("proj-resize", Vec::new());

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // Drain 1 (CreatePortalTile): publish one unit, drain to create the tile.
        publish(&mut driver, "proj-resize", &token, "initial-line", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let tile_id = driver
            .drive
            .entries
            .get("proj-resize")
            .expect("drive entry must exist")
            .tile_scene_id
            .expect("tile must be created after first drain");

        // Resize the scene tile to a tiny height so that overflow is detectable.
        // This simulates the hotkey-resize scene mutation in windowed.rs.
        let tiny_h = 1.0_f32;
        let _ = scene.update_tile_bounds(
            tile_id,
            tze_hud_scene::Rect::new(0.0, 0.0, 600.0, tiny_h),
            PORTAL_DRIVER_NAMESPACE,
        );

        // Producer wiring (hud-npq6g): push the geometry snapshot that the hotkey
        // resize path now emits. The sequence counter starts at 1.
        let geo_snapshot = GeometrySnapshot {
            portal_id_hash: 0,
            rect: PortalRect {
                x: 0.0,
                y: 0.0,
                width: 600.0,
                height: tiny_h,
            },
            gesture_active: false,
            sequence: 1,
        };
        let pushed = driver.push_geometry_snapshot_for_tile(tile_id, geo_snapshot);
        assert!(
            pushed,
            "push_geometry_snapshot_for_tile must return true for a known tile \
             — the producer wiring is broken if this fails"
        );

        // Verify the authority now holds the pending batch before the drain consumes it.
        {
            let state = driver
                .authority
                .projected_portal_state("proj-resize", &ProjectedPortalPolicy::permit_all())
                .expect("session must exist");
            let batch = state.geometry_batch.expect(
                "geometry_batch must be Some after push_geometry_snapshot_for_tile \
                 — the consumer path is broken if this fails",
            );
            assert_eq!(
                batch.latest.map(|s| s.sequence),
                Some(1),
                "pending batch must carry sequence 1 before drain"
            );
        }

        // Drain 2 (RenderPortal): publish overflowing content, drain past the rate window.
        let base_ts = PORTAL_UPDATE_RATE_WINDOW_WALL_US;
        for i in 1..=9_u64 {
            publish(
                &mut driver,
                "proj-resize",
                &token,
                &format!("overflow-line-{i}"),
                base_ts + i * 5,
            );
        }
        let drain2_now_us = base_ts + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain2_now_us);

        // (a) The drain must have used the geometry-batch viewport (tiny_h = 1 px).
        // With a 1-px viewport and 10 lines of content the tile is deep in overflow,
        // so notify_tile_content_appended must have set the at-tail flag.
        assert!(
            scene.tile_follow_tail_at_tail(tile_id),
            "spec §6b.4 regression (hud-npq6g): drain must use the geometry-batch \
             viewport height from push_geometry_snapshot_for_tile. \
             Removing the push_geometry_snapshot_for_tile call (or wiring in windowed.rs) \
             causes the drain to fall back to the adapter-configured height (~360 px), \
             which is too large for 10 lines to overflow → tile_follow_tail_at_tail stays false."
        );

        // (b) The pending batch must be cleared after the drain (consume wiring).
        let state_after = driver
            .authority
            .projected_portal_state("proj-resize", &ProjectedPortalPolicy::permit_all())
            .expect("session must still exist");
        assert!(
            state_after.geometry_batch.is_none(),
            "geometry_batch must be None after the drain consumed it — \
             removing consume_geometry_batch from the drain causes stale re-delivery"
        );
    }

    /// hud-ttq97: verify that a drained `RenderPortal` update with a known
    /// `submitted_at_us` records the expected publish-to-present delta into the
    /// `portal_publish_to_present_latency` bucket.
    ///
    /// Test shape:
    /// 1. Attach a projection session and drain (CreatePortalTile) at `t=200 µs`.
    ///    The bucket must have zero samples after the create drain (only `RenderPortal`
    ///    drains contribute samples).
    /// 2. Publish content with a known `submitted_at_us = 1_000 µs` and drain at
    ///    `now_us = 6_000 µs`.  Expected delta: `6_000 − 1_000 = 5_000 µs`.
    /// 3. Assert the bucket has exactly one sample with value `5_000 µs`.
    ///
    /// Removing the `self.portal_publish_to_present_latency.record(delta_us)` call
    /// at the `RenderPortal` drain site causes the bucket to remain empty and the
    /// final assertion to fail.
    #[test]
    fn render_portal_drain_records_publish_to_present_latency() {
        use tze_hud_projection::AdapterGeometrySnapshot;
        use tze_hud_projection::AdapterPortalRect;

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        };

        let token = attach_and_get_token(&mut driver, "proj-lat");
        driver.attach_projection("proj-lat", Vec::new());

        // Push a geometry snapshot so the drain has a valid viewport.
        driver.authority_mut().push_geometry_snapshot(
            "proj-lat",
            AdapterGeometrySnapshot {
                rect: AdapterPortalRect {
                    x_px: 0,
                    y_px: 0,
                    width_px: 600,
                    height_px: 360,
                },
                gesture_active: false,
                sequence: 1,
            },
        );

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // Drain 1 (CreatePortalTile): publish at t=100 µs, drain at t=200 µs.
        // The bucket must be empty after a CreatePortalTile drain.
        publish(&mut driver, "proj-lat", &token, "initial-line", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        assert_eq!(
            driver.portal_publish_to_present_latency().samples.len(),
            0,
            "CreatePortalTile drain must not record a latency sample — only \
             RenderPortal drains contribute to the bucket"
        );

        // Drain 2 (RenderPortal): publish at submitted_at_us = 1_000 µs, drain
        // at now_us = 6_000 µs.  Expected delta: 6_000 − 1_000 = 5_000 µs.
        //
        // Use `base_ts` past the initial rate window so the coalescer releases the update.
        let submitted_at_us = PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1_000;
        let drain2_now_us = submitted_at_us + 5_000;
        publish(
            &mut driver,
            "proj-lat",
            &token,
            "content-line",
            submitted_at_us,
        );
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain2_now_us);

        // Assert exactly one sample was recorded with the expected delta.
        let bucket = driver.portal_publish_to_present_latency();
        assert_eq!(
            bucket.samples.len(),
            1,
            "RenderPortal drain with submitted_at_us > 0 must record exactly one \
             latency sample; removing the record() call at the drain site causes \
             this to fail with 0 samples"
        );
        assert_eq!(
            bucket.samples[0], 5_000,
            "publish-to-present latency must be drain_now_us − submitted_at_us = \
             {drain2_now_us} − {submitted_at_us} = 5_000 µs"
        );
    }

    /// Regression guard for the `apply_token_map` → drive state propagation
    /// wiring (hud-be6ee acceptance criterion: token wiring reaches live adapters).
    ///
    /// The test verifies that:
    /// 1. A token map applied *before* an adapter attaches is inherited by the
    ///    new adapter's `PortalVisualTokens` at attach time.
    /// 2. A token map applied *after* an adapter attaches propagates immediately
    ///    to all live adapters' `PortalVisualTokens`.
    ///
    /// Removing the `self.drive.apply_token_map(overrides)` call in
    /// `InProcessPortalDriver::apply_token_map` causes the post-attach assertion
    /// to fail because `entry.adapter.visual_tokens()` still reflects the
    /// default (empty) token map rather than the overridden values.
    #[test]
    fn apply_token_map_propagates_to_drive_state() {
        let mut driver = InProcessPortalDriver::new();

        // Build a token map with a clearly non-default transcript font size.
        // The canonical key is `portal.transcript.font_size` (no `_px` suffix)
        // as declared by `PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE` in portal_tokens.rs.
        let mut tokens = DesignTokenMap::new();
        tokens.insert("portal.transcript.font_size".to_string(), "32".to_string());

        // --- Part 1: pre-attach token map propagates to new adapters -------
        driver.apply_token_map(tokens.clone());

        // Attach *after* the token map is applied.
        attach_and_get_token(&mut driver, "proj-token-pre");
        driver.attach_projection("proj-token-pre", Vec::new());

        let font_size_pre = driver
            .drive
            .entries
            .get("proj-token-pre")
            .expect("drive entry must exist after attach")
            .adapter
            .visual_tokens()
            .transcript_font_size_px;

        // The driver's `token_overrides` map was set before attach, so
        // `InProcessPortalDriveState::attach` calls `resolve_visual_tokens()`
        // using those overrides. The adapter must reflect the exact custom value
        // rather than the default (13.0).  Using `assert_eq!` here ensures the
        // test fails if the wrong key is looked up or the map is not propagated.
        assert_eq!(
            font_size_pre, 32.0,
            "adapter font size must match the pre-attach overridden value (32.0)"
        );

        // --- Part 2: post-attach token map propagates to live adapters -----
        // Reset to empty map so we can observe the change clearly.
        driver.apply_token_map(DesignTokenMap::new());

        // Attach a second adapter before applying the custom map.
        attach_and_get_token(&mut driver, "proj-token-post");
        driver.attach_projection("proj-token-post", Vec::new());

        // Now apply a custom token map — must propagate to *both* live adapters.
        let mut tokens2 = DesignTokenMap::new();
        tokens2.insert("portal.transcript.font_size".to_string(), "48".to_string());
        driver.apply_token_map(tokens2.clone());

        // Removing the `set_visual_tokens` loop inside `apply_token_map`
        // causes the post-apply assertion to fail (value stays at the default,
        // 13.0, rather than the overridden 48.0).
        let font_size_post = driver
            .drive
            .entries
            .get("proj-token-post")
            .expect("second drive entry must exist")
            .adapter
            .visual_tokens()
            .transcript_font_size_px;

        assert_eq!(
            font_size_post, 48.0,
            "adapter font size must match the post-attach overridden value (48.0); \
             removing the set_visual_tokens loop in apply_token_map causes this to fail"
        );

        // Both entries must have been updated (not just the most recently attached).
        let font_size_pre_after = driver
            .drive
            .entries
            .get("proj-token-pre")
            .expect("first drive entry must still exist")
            .adapter
            .visual_tokens()
            .transcript_font_size_px;

        assert_eq!(
            font_size_pre_after, 48.0,
            "pre-existing adapter must also be updated to 48.0 by apply_token_map; \
             only the new-entry path in attach() being correct is not sufficient"
        );
    }

    /// Regression guard for the `dispatch_portal_op` → `drain_inner` wiring
    /// introduced by hud-bq0gl.2.
    ///
    /// Exercises the full MCP channel path:
    ///
    /// 1. `dispatch_portal_op(PortalOp::Attach)` — must create a drive entry
    ///    and return an owner token through the one-shot reply channel.
    /// 2. `dispatch_portal_op(PortalOp::PublishOutput)` — must feed content into
    ///    the cadence coalescer.
    /// 3. `drain_inner` — must materialise the coalesced update into the scene and
    ///    call `notify_tile_content_appended` so `tile_follow_tail_at_tail` is
    ///    true (spec §3.2).
    ///
    /// If `dispatch_portal_op` is removed or the authority calls are deleted, the
    /// drive entry is never created, `drain_inner` finds nothing to do, and the
    /// `tile_follow_tail_at_tail` assertion fails.
    #[test]
    fn dispatch_portal_op_attach_publish_drain_wiring() {
        use tze_hud_projection::{AdapterGeometrySnapshot, AdapterPortalRect, ProjectionBounds};

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("test_dispatch_portal_op"),
            drain_deferral_count: 0,
        };

        // ── Step 1: Attach via dispatch_portal_op ──────────────────────────────
        let (attach_tx, mut attach_rx) =
            tokio::sync::oneshot::channel::<Result<String, PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: "test-proj".to_string(),
            display_name: "Test Projection".to_string(),
            idempotency_key: None,
            reply: attach_tx,
        });

        // The reply channel must carry the owner token — no async runtime needed
        // because dispatch_portal_op is synchronous.
        let owner_token = attach_rx
            .try_recv()
            .expect("reply must be sent synchronously")
            .expect("Attach must be accepted");
        assert!(
            !owner_token.is_empty(),
            "owner token must be non-empty on successful attach"
        );

        // Drive entry must exist after Attach.
        assert!(
            driver.drive.entries.contains_key("test-proj"),
            "dispatch_portal_op(Attach) must create a drive entry; \
             removing the attach_projection call causes this to fail"
        );

        // ── Step 2: PublishOutput via dispatch_portal_op ───────────────────────
        let (pub_tx, mut pub_rx) = tokio::sync::oneshot::channel::<Result<(), PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::PublishOutput {
            projection_id: "test-proj".to_string(),
            owner_token: owner_token.clone(),
            output_text: "Hello from MCP portal".to_string(),
            logical_unit_id: None,
            output_kind: None,
            content_classification: None,
            coalesce_key: None,
            reply: pub_tx,
        });

        pub_rx
            .try_recv()
            .expect("publish reply must be sent synchronously")
            .expect("PublishOutput must be accepted");

        // ── Step 3: drain_inner materialises content into scene ────────────────
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // Supply a geometry snapshot so the adapter has valid bounds.
        driver.authority_mut().push_geometry_snapshot(
            "test-proj",
            AdapterGeometrySnapshot {
                rect: AdapterPortalRect {
                    x_px: 0,
                    y_px: 0,
                    width_px: 600,
                    height_px: 200,
                },
                gesture_active: false,
                sequence: 1,
            },
        );

        // First drain: CreatePortalTile.
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let tile_id = driver
            .drive
            .entries
            .get("test-proj")
            .expect("drive entry must still exist after drain")
            .tile_scene_id
            .expect("drain must create a portal tile in the scene");

        assert!(
            scene.tile_scroll_config(tile_id).is_some(),
            "portal tile must have a scroll config after CreatePortalTile drain"
        );

        // Resize the tile to a 1-line viewport so overflow and follow-tail are
        // detectable.
        let font_size_px = driver
            .drive
            .entries
            .get("test-proj")
            .unwrap()
            .adapter
            .visual_tokens()
            .transcript_font_size_px;
        let line_h = font_size_px * PORTAL_LINE_HEIGHT_MULTIPLIER;
        let viewport_h = (1.0 * line_h).ceil();
        let _ = scene.update_tile_bounds(
            tile_id,
            Rect::new(0.0, 0.0, 600.0, viewport_h),
            PORTAL_DRIVER_NAMESPACE,
        );

        // Publish 9 more lines, then drain past the rate window.
        for i in 1..=9_u64 {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            driver.dispatch_portal_op(PortalOp::PublishOutput {
                projection_id: "test-proj".to_string(),
                owner_token: owner_token.clone(),
                output_text: format!("line {i}"),
                logical_unit_id: Some(format!("u{i}")),
                output_kind: None,
                content_classification: None,
                coalesce_key: None,
                reply: tx,
            });
        }

        let drain2_now_us = PORTAL_UPDATE_RATE_WINDOW_WALL_US * 2 + 1;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain2_now_us);

        // Primary regression assertion (hud-bq0gl.2):
        // The MCP-channel wiring must result in notify_tile_content_appended
        // being called for overflowing content, setting tile_follow_tail_at_tail.
        // Removing dispatch_portal_op or the drain_inner call causes this to fail.
        assert!(
            scene.tile_follow_tail_at_tail(tile_id),
            "hud-bq0gl.2 regression: portal content published via dispatch_portal_op \
             must flow through drain_inner and set tile_follow_tail_at_tail (spec §3.2); \
             removing the dispatch_portal_op → authority wiring causes this to fail"
        );
    }

    /// Regression guard: an idempotent re-attach (same `projection_id` +
    /// matching `idempotency_key`) must NOT reset the drive entry, otherwise
    /// the next drain creates a duplicate tile and orphans the original
    /// (gemini PR #765 HIGH finding).
    ///
    /// The authority returns `accepted=true` for the replay without recreating
    /// the session; `dispatch_portal_op` must therefore preserve the existing
    /// `DriveEntry` (with its live `tile_scene_id`) instead of inserting a
    /// fresh one.
    #[test]
    fn idempotent_reattach_does_not_duplicate_tile() {
        use tze_hud_projection::{AdapterGeometrySnapshot, AdapterPortalRect, ProjectionBounds};

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("test_reattach"),
            drain_deferral_count: 0,
        };

        // ── Step 1: First attach (with an idempotency key) ─────────────────────
        let (attach_tx, mut attach_rx) =
            tokio::sync::oneshot::channel::<Result<String, PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: "reattach-proj".to_string(),
            display_name: "Reattach Projection".to_string(),
            idempotency_key: Some("key-1".to_string()),
            reply: attach_tx,
        });
        let owner_token = attach_rx
            .try_recv()
            .expect("reply must be sent synchronously")
            .expect("first Attach must be accepted");

        // ── Step 2: Publish + drain so a tile is materialised ──────────────────
        let (pub_tx, mut pub_rx) = tokio::sync::oneshot::channel::<Result<(), PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::PublishOutput {
            projection_id: "reattach-proj".to_string(),
            owner_token: owner_token.clone(),
            output_text: "first line".to_string(),
            logical_unit_id: None,
            output_kind: None,
            content_classification: None,
            coalesce_key: None,
            reply: pub_tx,
        });
        pub_rx
            .try_recv()
            .expect("publish reply must be sent synchronously")
            .expect("PublishOutput must be accepted");

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();
        driver.authority_mut().push_geometry_snapshot(
            "reattach-proj",
            AdapterGeometrySnapshot {
                rect: AdapterPortalRect {
                    x_px: 0,
                    y_px: 0,
                    width_px: 600,
                    height_px: 200,
                },
                gesture_active: false,
                sequence: 1,
            },
        );
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let original_tile = driver
            .drive
            .entries
            .get("reattach-proj")
            .expect("drive entry must exist after first attach+drain")
            .tile_scene_id
            .expect("first drain must create a portal tile");
        let tile_count_after_first = scene.tile_count();

        // ── Step 3: Idempotent re-attach (same id + same key) ──────────────────
        let (re_tx, mut re_rx) =
            tokio::sync::oneshot::channel::<Result<String, PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: "reattach-proj".to_string(),
            display_name: "Reattach Projection".to_string(),
            idempotency_key: Some("key-1".to_string()),
            reply: re_tx,
        });
        re_rx
            .try_recv()
            .expect("reply must be sent synchronously")
            .expect("idempotent re-attach must be accepted");

        // The drive entry must still point at the SAME tile — not reset to None.
        let tile_after_reattach = driver
            .drive
            .entries
            .get("reattach-proj")
            .expect("drive entry must survive idempotent re-attach")
            .tile_scene_id
            .expect("idempotent re-attach must preserve the existing tile_scene_id");
        assert_eq!(
            tile_after_reattach, original_tile,
            "idempotent re-attach must NOT reset tile_scene_id; resetting it makes \
             the next drain create a duplicate tile and orphan the original"
        );

        // ── Step 4: Another drain must NOT create a second tile ────────────────
        driver.drain_inner(
            &mut scene,
            &mut processor,
            Some(tab_id),
            PORTAL_UPDATE_RATE_WINDOW_WALL_US * 2 + 1,
        );
        assert_eq!(
            scene.tile_count(),
            tile_count_after_first,
            "idempotent re-attach + drain must not create a duplicate portal tile"
        );
    }

    // ── hud-66i1s: height-only head-trim condition (runtime parity) ───────────

    /// Regression test for hud-66i1s: the in-process runtime drain must call
    /// `notify_head_content_removed` when `new_content_height_px` shrinks between
    /// consecutive `RenderPortal` drains, even when `visible_transcript_bytes` does
    /// NOT shrink (or grows).
    ///
    /// This is the runtime-path counterpart to the CLI-path test
    /// `head_trim_geometry_emitted_when_height_shrinks_without_byte_shrink` in
    /// `projection_authority.rs` (PR #779 / hud-hkaw2).
    ///
    /// The old dual-condition (`bytes_shrank && height_shrank`) would miss the
    /// case where a many-newline unit is evicted by a flat unit of equal or
    /// greater byte count.  The fixed height-only condition fires correctly.
    ///
    /// Byte/line layout (max_vis=30B, 1-line viewport):
    ///
    ///   drain1 (CreatePortalTile): publish seed unit "S" (1B, 1 line).
    ///   drain2 (RenderPortal): publish "\n" × 24 = 24B, 25 lines.
    ///     Visible window: 1B + 24B = 25B ≤ 30B → both visible.
    ///     Total height: 26 lines × line_h. Tile is AtTail; scroll_y = 25 × line_h.
    ///   [user scrolls back 1 px: tile becomes ScrolledBack, scroll_y = 25*line_h - 1]
    ///   drain3 (RenderPortal): publish flat "B" × 30 = 30B, 1 line.
    ///     Visible window from tail: 30B ≤ 30B → fits; + 24B = 54B > 30 → evicted.
    ///     visible_bytes = 30 (GREW from 25), height = 1 × line_h (SHRANK from 26).
    ///     height-only condition: height_shrank → notify_head_content_removed fires.
    ///     Removed height = (26-1)*line_h; ScrolledBack offset decreases to 0 (clamped).
    ///     Old dual-condition: bytes_shrank=(30<25)=false → would NOT fire (bug).
    #[test]
    fn head_trim_fires_on_height_shrink_without_byte_shrink_runtime_path() {
        use tze_hud_input::ScrollEvent;
        use tze_hud_scene::TileScrollConfig;

        // max_vis = 30B so that:
        //   seed(1B) + many-lines(24B) = 25B ≤ 30B → both fit in drain2;
        //   flat(30B) alone ≤ 30B → fits; flat + many-lines = 54B > 30 → evicts many-lines.
        let max_vis: usize = 30;
        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                max_visible_transcript_bytes: max_vis,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("test_htrim_runtime"),
            drain_deferral_count: 0,
        };

        let token = attach_and_get_token(&mut driver, "proj-htrim");
        driver.attach_projection("proj-htrim", Vec::new());

        // Derive line height from the adapter's font token.
        let font_size_px = driver
            .drive
            .entries
            .get("proj-htrim")
            .unwrap()
            .adapter
            .visual_tokens()
            .transcript_font_size_px;
        let line_h = font_size_px * PORTAL_LINE_HEIGHT_MULTIPLIER;

        // Viewport = 1 line so that after drain2 (26 lines) there is real overflow
        // and the tile is AtTail with scroll_y = 25 × line_h.
        let viewport_h = (1.0_f32 * line_h).ceil();

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // drain1 (CreatePortalTile): seed unit "S" = 1B, 1 line.
        publish(&mut driver, "proj-htrim", &token, "S", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let tile_id = driver
            .drive
            .entries
            .get("proj-htrim")
            .expect("drive entry must exist after drain1")
            .tile_scene_id
            .expect("tile must be created after drain1 (CreatePortalTile)");

        // Register scroll config for hit-test (needed for process_scroll_event).
        scene
            .register_tile_scroll_config(
                tile_id,
                TileScrollConfig {
                    scrollable_x: false,
                    scrollable_y: true,
                    content_width: None,
                    content_height: None,
                },
            )
            .unwrap();

        // Resize tile to the 1-line viewport so overflow and follow-tail are
        // detectable.
        let _ = scene.update_tile_bounds(
            tile_id,
            Rect::new(0.0, 0.0, 600.0, viewport_h),
            PORTAL_DRIVER_NAMESPACE,
        );

        // drain2 (RenderPortal): many-lines unit "\n" × 24 = 24B, 25 lines.
        // After drain2: visible = [seed(1B) + many-lines(24B)] = 25B ≤ 30B;
        // total height = 26 × line_h; tile is AtTail with scroll_y = 25 × line_h.
        let many_lines = "\n".repeat(24);
        assert_eq!(many_lines.len(), 24, "many_lines must be exactly 24B");

        let ts2 = PORTAL_UPDATE_RATE_WINDOW_WALL_US + 25;
        publish(&mut driver, "proj-htrim", &token, &many_lines, ts2);
        let drain2_now = ts2 + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain2_now);

        // After drain2 the tile must be at-tail (26 lines in 1-line viewport).
        assert!(
            scene.tile_follow_tail_at_tail(tile_id),
            "tile must be at-tail after drain2 (26-line content in 1-line viewport)"
        );

        // Scroll back by 1 px to make the tile ScrolledBack.
        // The tile occupies (0,0)→(600, viewport_h); use the tile centre for hit-testing.
        let centre_y = viewport_h / 2.0;
        let scroll_ev = ScrollEvent {
            x: 300.0,
            y: centre_y,
            delta_x: 0.0,
            delta_y: -1.0, // negative = scroll up (move content down = scroll back toward head)
        };
        processor.process_scroll_event(&scroll_ev, &mut scene);

        // Tile must now be ScrolledBack (no longer at-tail).
        assert!(
            !scene.tile_follow_tail_at_tail(tile_id),
            "tile must be ScrolledBack after user scroll-back event"
        );

        // Record pre-drain3 scroll offset.
        let (_, pre_y) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            pre_y > 0.0,
            "scroll offset must be positive before drain3 (got {pre_y})"
        );

        // drain3 (RenderPortal): flat unit "B" × 30 = 30B, 1 line.
        // Visible from tail: flat(30B) ≤ 30B → fits; flat + many-lines = 54B > 30 → evicted.
        // visible_bytes = 30 (GREW from 25); total height = 1 × line_h (SHRANK from 26×line_h).
        //
        // KEY: visible_transcript_bytes grew (25 → 30), so the old dual-condition
        // (bytes_shrank && height_shrank) would NOT fire — the bug.
        // The fixed height-only condition (height_shrank) MUST fire.
        let flat_unit = "B".repeat(30);
        assert_eq!(flat_unit.len(), 30, "flat_unit must be exactly 30B");

        let ts3 = drain2_now + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1;
        publish(&mut driver, "proj-htrim", &token, &flat_unit, ts3);
        let drain3_now = ts3 + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), drain3_now);

        // Flush the dirty scroll state to the scene (notify_head_content_removed
        // marks the tile dirty for ScrolledBack tiles; commit_scroll_updates
        // applies the updated offset to the scene graph).
        processor.commit_scroll_updates(&mut scene);

        // KEY ASSERTION (hud-66i1s fix):
        // The scroll offset must have decreased because head-trim fired.
        // notify_head_content_removed(removed_px = 25 × line_h) shifts the
        // ScrolledBack offset down by 25 × line_h → clamped to 0 (since
        // new max_scroll_offset = 1×line_h - 1×line_h = 0).
        //
        // If the old dual-condition were still in place:
        //   bytes_shrank = (30 < 25) = false → no head-trim call → offset unchanged → BUG.
        // With the height-only fix:
        //   height_shrank = (1×line_h < 26×line_h) = true → head-trim fires → offset = 0.
        let (_, post_y) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            post_y < pre_y,
            "hud-66i1s regression: scroll offset must decrease after height-only head-trim; \
             pre_y={pre_y} post_y={post_y} line_h={line_h}. \
             If this fails, notify_head_content_removed was not called — the old dual-condition \
             (bytes_shrank && height_shrank) is still in place at the runtime drain site."
        );
        // The new content has the same height as the viewport so max_scroll = 0.
        // After head-trim adjustment the offset must reach 0.
        assert!(
            post_y <= f32::EPSILON,
            "hud-66i1s: after head-trim the max scroll offset is 0 (1-line content in 1-line \
             viewport); offset must clamp to 0, got post_y={post_y}"
        );
    }

    // ── Portal observability surface tests (hud-bq0gl.14) ────────────────────

    /// Regression guard: `drain_deferral_count` must be incremented when the
    /// drain loop encounters a portal whose rate window has not yet elapsed
    /// (i.e. `take_due_portal_update` returns `Ok(None)`).
    ///
    /// The rate window opens on the FIRST successful drain (at `now_us = T1`).
    /// A second publish + drain within `T1 + PORTAL_UPDATE_RATE_WINDOW_WALL_US`
    /// triggers the `Ok(None)` path because `portal_update_allowed` returns
    /// `false` once `max_portal_updates_per_second` is exhausted.
    ///
    /// We use `max_portal_updates_per_second: 1` (one update allowed per window)
    /// so a single publish + drain pair saturates the window, and the next
    /// drain in the same window is deferred.
    #[test]
    fn drain_deferral_count_increments_on_rate_window_not_elapsed() {
        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 1,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("test_deferral"),
            drain_deferral_count: 0,
        };

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut processor = InputProcessor::new();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        let token = attach_and_get_token(&mut driver, "proj-defer");
        driver.attach_projection("proj-defer", Vec::new());

        assert_eq!(
            driver.drain_deferral_count(),
            0,
            "deferral count must be 0 before any drain"
        );

        // First publish + drain at T1: opens the rate window, returns Ok(Some(_)).
        // max_portal_updates_per_second = 1, so the window is now saturated.
        let t1 = PORTAL_UPDATE_RATE_WINDOW_WALL_US;
        publish(&mut driver, "proj-defer", &token, "hello", t1);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), t1);

        assert_eq!(
            driver.drain_deferral_count(),
            0,
            "deferral count must still be 0 after successful drain"
        );

        // Second publish within the same rate window (T1 + 1, window end = T1 + 1_000_000).
        // The next drain at T1 + 2 is within the window; take_due_portal_update
        // returns Ok(None) because max_portal_updates_per_second = 1 is saturated.
        let t2 = t1 + 1;
        publish(&mut driver, "proj-defer", &token, "world", t2);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), t2);

        assert_eq!(
            driver.drain_deferral_count(),
            1,
            "deferral count must be 1 after a drain where rate-window has not elapsed"
        );

        // A second deferred drain must increment again.
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), t2 + 1);
        assert_eq!(
            driver.drain_deferral_count(),
            2,
            "deferral count must increment on each rate-limited drain cycle"
        );
    }

    /// `drain_deferral_count` must NOT increment when a drain successfully
    /// materialises an update (rate window has elapsed).
    #[test]
    fn drain_deferral_count_unchanged_on_successful_drain() {
        let mut driver = InProcessPortalDriver::new();
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut processor = InputProcessor::new();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        let token = attach_and_get_token(&mut driver, "proj-ok");
        driver.attach_projection("proj-ok", Vec::new());

        publish(&mut driver, "proj-ok", &token, "content", 1);

        // Drain past the rate window — update is materialised successfully.
        let now = PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1_000_000;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), now);

        assert_eq!(
            driver.drain_deferral_count(),
            0,
            "deferral count must remain 0 after a successful drain"
        );
    }

    /// `coalescer_portal_count` on `ProjectionAuthority` must return the number
    /// of portals registered in the cadence coalescer after attach.
    #[test]
    fn coalescer_portal_count_reflects_attached_portals() {
        let mut driver = InProcessPortalDriver::new();

        assert_eq!(
            driver.authority_mut().coalescer_portal_count(),
            0,
            "coalescer_portal_count must be 0 before any portal is attached"
        );

        attach_and_get_token(&mut driver, "proj-c1");
        let _ = driver.authority_mut().coalescer_portal_count(); // idempotent reads

        // After a publish the coalescer registers the portal.
        let token = attach_and_get_token(&mut driver, "proj-c2");
        publish(&mut driver, "proj-c2", &token, "hi", 1);

        assert_eq!(
            driver.authority_mut().coalescer_portal_count(),
            1,
            "coalescer_portal_count must be 1 after one portal publishes"
        );
    }

    /// `coalescer_total_taken` must increment each time a successful drain
    /// consumes a pending coalescer snapshot.
    #[test]
    fn coalescer_total_taken_increments_on_drain() {
        let mut driver = InProcessPortalDriver::new();
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut processor = InputProcessor::new();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        let token = attach_and_get_token(&mut driver, "proj-tk");
        driver.attach_projection("proj-tk", Vec::new());

        assert_eq!(
            driver.authority_mut().coalescer_total_taken(),
            0,
            "total_taken must be 0 before any drain"
        );

        // First publish + drain.
        publish(&mut driver, "proj-tk", &token, "a", 1);
        let now1 = PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1_000_000;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), now1);

        assert_eq!(
            driver.authority_mut().coalescer_total_taken(),
            1,
            "total_taken must be 1 after one successful drain"
        );

        // Second publish + drain.
        let ts2 = now1 + 1;
        publish(&mut driver, "proj-tk", &token, "b", ts2);
        let now2 = ts2 + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1_000_000;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), now2);

        assert_eq!(
            driver.authority_mut().coalescer_total_taken(),
            2,
            "total_taken must be 2 after two successful drains"
        );
    }

    /// Regression guard (hud-bsr7u): the drain loop must TERMINATE when the
    /// coalescer holds a pending entry for a projection whose session is gone
    /// (session/coalescer divergence — e.g. operator cleanup that purged only
    /// the session).
    ///
    /// Mechanism of the original freeze:
    ///   1. `next_due_projection_id` returns the orphaned id (pending entry set).
    ///   2. `take_due_portal_update` fails the session lookup and returns
    ///      `Err(ProjectionNotFound)` BEFORE it can consume the coalescer entry.
    ///   3. The `Err` arm previously did `detach + continue` without touching the
    ///      coalescer → `next_due_projection_id` returns the SAME id again →
    ///      unbounded loop that wedges the event loop under `ControlFlow::Poll`.
    ///
    /// The fix discards the coalescer entry in the `Err` arm. This test injects
    /// an orphaned coalescer entry (no session), runs a drain, and asserts the
    /// drain RETURNS (does not hang) and that the orphaned entry was consumed so
    /// it cannot be returned again. The bounded `#[test]` harness will hang and
    /// time out if the loop spins — termination IS the assertion.
    #[test]
    fn drain_terminates_on_orphaned_coalescer_entry() {
        let mut driver = InProcessPortalDriver::new();
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut processor = InputProcessor::new();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // Inject an orphaned coalescer entry: a pending snapshot with NO session
        // backing it (the abnormal divergence state operator-cleanup used to leave).
        driver
            .authority_mut()
            .inject_orphan_coalescer_entry_for_test("proj-orphan");
        assert_eq!(
            driver.authority_mut().coalescer_pending_portal_count(),
            1,
            "precondition: orphaned coalescer entry must be pending"
        );

        // This must NOT hang. The Err arm of the drain loop consumes the
        // orphaned entry; the loop then sees an empty coalescer and breaks.
        let now = PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1_000_000;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), now);

        // The orphaned entry must be gone so `next_due_projection_id` cannot
        // return it again on the next tick.
        assert_eq!(
            driver.authority_mut().coalescer_pending_portal_count(),
            0,
            "hud-bsr7u: drain Err-arm must consume the orphaned coalescer entry"
        );
        assert!(
            driver.authority_mut().next_due_projection_id().is_none(),
            "hud-bsr7u: no portal may be reported due after the orphaned entry is consumed"
        );

        // A second drain is a clean no-op (idempotent termination).
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), now + 1);
        assert_eq!(
            driver.authority_mut().coalescer_pending_portal_count(),
            0,
            "second drain must remain idle"
        );
    }

    /// Regression guard (hud-bsr7u): operator cleanup performed through the
    /// driver's hosted authority leaves the coalescer with no pending entry, and
    /// a subsequent drain terminates cleanly.
    ///
    /// This exercises the full path the live freeze took: cooperative attach →
    /// publish_output (seeds a coalescer pending entry) → operator cleanup →
    /// drain. With the layer-1 fix the coalescer is already clear at cleanup
    /// time; with the layer-2 fix the drain would still terminate even if it
    /// weren't. Together they guarantee no busy-spin.
    #[test]
    fn operator_cleanup_then_drain_does_not_spin() {
        use tze_hud_projection::{CleanupAuthority, CleanupRequest};

        let mut driver = InProcessPortalDriver::new();
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut processor = InputProcessor::new();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        driver
            .authority_mut()
            .set_operator_authority("operator-secret")
            .unwrap();

        let token = attach_and_get_token(&mut driver, "proj-op");
        driver.attach_projection("proj-op", Vec::new());

        // Seed a pending coalescer entry without draining it.
        publish(&mut driver, "proj-op", &token, "live transcript", 100);
        assert_eq!(
            driver.authority_mut().coalescer_pending_portal_count(),
            1,
            "precondition: publish seeds a pending coalescer entry"
        );

        // Operator cleanup — the live freeze trigger.
        let resp = driver.authority_mut().handle_cleanup(
            CleanupRequest {
                envelope: OperationEnvelope {
                    operation: ProjectionOperation::Cleanup,
                    projection_id: "proj-op".to_string(),
                    request_id: "req-op-cleanup".to_string(),
                    client_timestamp_wall_us: 1,
                },
                cleanup_authority: CleanupAuthority::Operator,
                owner_token: None,
                operator_authority: Some("operator-secret".to_string()),
                reason: "operator override".to_string(),
            },
            "operator",
            200,
        );
        assert!(resp.accepted, "operator cleanup must be accepted");

        // Layer-1: coalescer already purged at cleanup time.
        assert_eq!(
            driver.authority_mut().coalescer_pending_portal_count(),
            0,
            "hud-bsr7u layer-1: operator cleanup must purge the coalescer entry"
        );

        // Drain must terminate (no busy-spin) and remain idle.
        let now = 200 + PORTAL_UPDATE_RATE_WINDOW_WALL_US + 1_000_000;
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), now);
        assert!(
            driver.authority_mut().next_due_projection_id().is_none(),
            "hud-bsr7u: no portal may remain due after operator cleanup + drain"
        );
    }

    // ── hud-m7w3g: publish classification + coalesce_key threading ────────────

    #[test]
    fn parse_output_kind_defaults_and_variants() {
        assert_eq!(parse_output_kind(None), Ok(OutputKind::Assistant));
        assert_eq!(
            parse_output_kind(Some("assistant")),
            Ok(OutputKind::Assistant)
        );
        assert_eq!(parse_output_kind(Some("tool")), Ok(OutputKind::Tool));
        assert_eq!(parse_output_kind(Some("status")), Ok(OutputKind::Status));
        assert_eq!(parse_output_kind(Some("error")), Ok(OutputKind::Error));
        assert_eq!(parse_output_kind(Some("other")), Ok(OutputKind::Other));
    }

    #[test]
    fn parse_output_kind_rejects_unknown() {
        let err = parse_output_kind(Some("Assistant")).unwrap_err();
        assert!(err.contains("invalid output_kind"), "got: {err}");
        // Camel-case / unknown spellings are rejected, not silently defaulted.
        assert!(parse_output_kind(Some("bogus")).is_err());
    }

    #[test]
    fn parse_content_classification_defaults_safe_to_private() {
        // Omitted classification must default to Private (privacy safe-by-default).
        assert_eq!(
            parse_content_classification(None),
            Ok(ContentClassification::Private)
        );
        assert_eq!(
            parse_content_classification(Some("public")),
            Ok(ContentClassification::Public)
        );
        assert_eq!(
            parse_content_classification(Some("household")),
            Ok(ContentClassification::Household)
        );
        assert_eq!(
            parse_content_classification(Some("private")),
            Ok(ContentClassification::Private)
        );
        assert_eq!(
            parse_content_classification(Some("sensitive")),
            Ok(ContentClassification::Sensitive)
        );
    }

    #[test]
    fn parse_content_classification_rejects_unknown() {
        let err = parse_content_classification(Some("secret")).unwrap_err();
        assert!(err.contains("invalid content_classification"), "got: {err}");
    }

    /// An explicit, valid `output_kind` / `content_classification` plus a
    /// `coalesce_key` must thread through `dispatch_portal_op(PublishOutput)`
    /// into the authority and be accepted (hud-m7w3g).
    #[test]
    fn dispatch_publish_threads_classification_and_coalesce_key() {
        use tze_hud_projection::ProjectionBounds;

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("test_publish_classification"),
            drain_deferral_count: 0,
        };

        let (attach_tx, mut attach_rx) =
            tokio::sync::oneshot::channel::<Result<String, PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: "classify-proj".to_string(),
            display_name: "Classify Projection".to_string(),
            idempotency_key: None,
            reply: attach_tx,
        });
        let owner_token = attach_rx
            .try_recv()
            .expect("reply sent synchronously")
            .expect("attach accepted");

        let (pub_tx, mut pub_rx) = tokio::sync::oneshot::channel::<Result<(), PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::PublishOutput {
            projection_id: "classify-proj".to_string(),
            owner_token,
            output_text: "status line".to_string(),
            logical_unit_id: None,
            output_kind: Some("status".to_string()),
            content_classification: Some("public".to_string()),
            coalesce_key: Some("ck-1".to_string()),
            reply: pub_tx,
        });
        pub_rx
            .try_recv()
            .expect("reply sent synchronously")
            .expect("publish with explicit classification + coalesce_key must be accepted");
    }

    /// An unrecognized `content_classification` must be rejected at the driver
    /// before reaching the authority, surfaced through the reply channel —
    /// never silently coerced to a less-private default (hud-m7w3g).
    #[test]
    fn dispatch_publish_rejects_invalid_classification() {
        use tze_hud_projection::ProjectionBounds;

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds::default()).unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("test_publish_invalid"),
            drain_deferral_count: 0,
        };

        let (attach_tx, mut attach_rx) =
            tokio::sync::oneshot::channel::<Result<String, PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: "reject-proj".to_string(),
            display_name: "Reject Projection".to_string(),
            idempotency_key: None,
            reply: attach_tx,
        });
        let owner_token = attach_rx
            .try_recv()
            .expect("reply sent synchronously")
            .expect("attach accepted");

        let (pub_tx, mut pub_rx) = tokio::sync::oneshot::channel::<Result<(), PortalOpRejection>>();
        driver.dispatch_portal_op(PortalOp::PublishOutput {
            projection_id: "reject-proj".to_string(),
            owner_token,
            output_text: "secret payload".to_string(),
            logical_unit_id: None,
            output_kind: None,
            content_classification: Some("top-secret".to_string()),
            coalesce_key: None,
            reply: pub_tx,
        });
        let result = pub_rx
            .try_recv()
            .expect("reply sent synchronously even on rejection");
        let err = result.expect_err("invalid content_classification must be rejected");
        assert_eq!(
            err.error_code,
            ProjectionErrorCode::ProjectionInvalidArgument,
            "a driver-side validation failure carries the stable INVALID_ARGUMENT code"
        );
        assert!(
            err.message.contains("invalid content_classification"),
            "got: {}",
            err.message
        );
    }

    /// hud-pk9pz (task 4.6): a session attaching AFTER the prior portal's lease
    /// grace period has already expired must start a FRESH portal under a NEW
    /// lease — it must never revive the removed surface nor reuse the dead lease.
    ///
    /// Structural reason this needs a guard: `expire_leases` reaps an orphaned
    /// lease's tiles on grace expiry but leaves the lease entry resident in the
    /// map as `Expired` (terminal state recorded in place). The driver caches
    /// its lease id in `self.lease_id`. The OLD reuse check
    /// (`lease_capabilities(..).is_some()`) returned `Some` for that dead-but-
    /// resident lease, so the driver would reuse it; the next `create_tile` then
    /// fails its `require_active_lease` check and the re-attaching session
    /// silently gets NO portal. The fix gates reuse on `lease_is_active`, so a
    /// post-grace re-attach grants a fresh lease and creates a fresh tile.
    #[test]
    fn reattach_after_grace_expiry_starts_fresh_portal_under_new_lease() {
        use std::sync::Arc;
        use tze_hud_scene::{Clock, TestClock};

        let mut driver = InProcessPortalDriver {
            authority: ProjectionAuthority::new(ProjectionBounds {
                max_portal_updates_per_second: 100,
                ..ProjectionBounds::default()
            })
            .unwrap(),
            drive: InProcessPortalDriveState::new(),
            lease_id: None,
            portal_publish_to_present_latency: LatencyBucket::new("portal_publish_to_present"),
            drain_deferral_count: 0,
        };

        // Scene backed by a TestClock so we can drive lease grace expiry
        // deterministically. Scene clock (ms) is independent of the drain
        // rate-window clock (`now_us`) passed to `drain_inner`.
        let clock = TestClock::new(1_000);
        let mut scene = SceneGraph::new_with_clock(1920.0, 1080.0, Arc::new(clock.clone()));
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        // --- First session: attach + drain → CreatePortalTile under lease L1. ---
        let token1 = attach_and_get_token(&mut driver, "proj-1");
        driver.attach_projection("proj-1", Vec::new());
        publish(&mut driver, "proj-1", &token1, "session-one content", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);

        let lease1 = driver
            .lease_id
            .expect("driver must hold a lease after the first CreatePortalTile drain");
        let tile1 = driver
            .drive
            .entries
            .get("proj-1")
            .expect("first drive entry must exist")
            .tile_scene_id
            .expect("first tile must be created");
        assert_eq!(scene.tile_count(), 1, "first portal tile is live");
        assert!(
            scene.lease_is_active(&lease1),
            "the first lease is active while the portal is live"
        );

        // --- First session drops; grace period elapses; orphan path reaps it. ---
        scene
            .disconnect_lease(&lease1, clock.now_millis())
            .expect("disconnect orphans the lease (degraded surface, grace begins)");
        // Advance well past the 30s grace period, then run expiry.
        clock.advance(SceneGraph::DEFAULT_GRACE_PERIOD_MS + 1_000);
        let expiries = scene.expire_leases();
        assert_eq!(expiries.len(), 1, "the grace-expired lease must be reaped");
        assert!(
            expiries[0].removed_tiles.contains(&tile1),
            "the dead session's surface is removed on grace expiry"
        );
        assert_eq!(
            scene.tile_count(),
            0,
            "no surface survives past grace — nothing to revive"
        );
        // The lease entry is left resident as Expired (terminal recorded in
        // place) — this is exactly the trap the fix guards against.
        assert!(
            !scene.lease_is_active(&lease1),
            "L1 is no longer active after grace expiry"
        );

        // --- Second session attaches AFTER grace expiry. It must get a FRESH ---
        // --- portal under a NEW lease, not a revived surface or reused lease.  --
        let token2 = attach_and_get_token(&mut driver, "proj-2");
        driver.attach_projection("proj-2", Vec::new());
        publish(&mut driver, "proj-2", &token2, "session-two content", 300);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 400);

        let lease2 = driver
            .lease_id
            .expect("driver must hold a lease after the re-attach CreatePortalTile drain");
        assert_ne!(
            lease2, lease1,
            "post-grace re-attach must obtain a FRESH lease, not reuse the dead L1"
        );
        assert!(
            scene.lease_is_active(&lease2),
            "the fresh lease must be Active so create_tile succeeds"
        );

        let tile2 = driver
            .drive
            .entries
            .get("proj-2")
            .expect("second drive entry must exist")
            .tile_scene_id
            .expect(
                "a fresh portal tile MUST be created for the re-attaching session — \
                 if this is None, the driver reused the dead lease and create_tile \
                 silently failed (the bug this test guards)",
            );
        assert_ne!(
            tile2, tile1,
            "the new portal is a fresh surface, not a revival of the removed one"
        );
        assert_eq!(
            scene.tile_count(),
            1,
            "exactly one live portal tile — the fresh one"
        );
    }

    /// End-to-end ingress lifecycle through `dispatch_portal_op` (hud-bq0gl.1).
    ///
    /// This is the production MCP ingress path: each `PortalOp` is the exact
    /// message the MCP tool handlers send over the portal-op channel. The test
    /// drives Attach → (seed HUD input) → GetPendingInput → AcknowledgeInput →
    /// Detach and asserts the authority state transitions at each step, plus the
    /// owner-token authorization gate.
    ///
    /// No GPU / scene drain is involved — this exercises only the op-dispatch
    /// → authority wiring that the new MCP surface depends on.
    #[test]
    fn dispatch_portal_op_input_lifecycle_end_to_end() {
        use tze_hud_projection::ContentClassification;

        let mut driver = InProcessPortalDriver::new();
        let proj = "proj-ingress";

        // 1. Attach via the op channel — yields the owner token.
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: proj.to_string(),
            display_name: "Ingress Test".to_string(),
            idempotency_key: None,
            reply: tx,
        });
        let token = rx
            .blocking_recv()
            .expect("attach reply must arrive")
            .expect("attach must be accepted");
        assert!(!token.is_empty(), "attach must return a non-empty token");

        // Seed a HUD-originated input item directly on the authority (the
        // operator-input producer side is out of scope for this ingress test).
        let now = now_wall_us().max(1);
        driver
            .authority_mut()
            .enqueue_input(
                proj,
                "input-1",
                "hello from the HUD".to_string(),
                now,
                now + 60_000_000,
                Some(ContentClassification::Household),
            )
            .expect("enqueue_input must succeed for an attached projection");

        // 2. GetPendingInput via the op channel — returns the delivered item.
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::GetPendingInput {
            projection_id: proj.to_string(),
            owner_token: token.clone(),
            max_items: None,
            max_bytes: None,
            reply: tx,
        });
        let batch = rx
            .blocking_recv()
            .expect("get_pending_input reply must arrive")
            .expect("get_pending_input must be accepted");
        assert_eq!(batch.items.len(), 1, "exactly one pending item delivered");
        let item = &batch.items[0];
        assert_eq!(item.input_id, "input-1");
        assert_eq!(item.projection_id, proj);
        assert_eq!(item.submission_text, "hello from the HUD");
        assert_eq!(
            item.delivery_state, "delivered",
            "item must be transitioned to delivered by the poll"
        );
        assert_eq!(
            item.content_classification, "household",
            "classification must be mapped to its snake_case wire spelling"
        );
        assert_eq!(batch.remaining_count, 0);

        // Auth gate: a wrong token must be rejected (security regression guard).
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::GetPendingInput {
            projection_id: proj.to_string(),
            owner_token: "not-the-real-token".to_string(),
            max_items: None,
            max_bytes: None,
            reply: tx,
        });
        let denied = rx
            .blocking_recv()
            .expect("get_pending_input reply must arrive");
        assert!(
            denied.is_err(),
            "get_pending_input with a bad owner token must be denied"
        );

        // 3. AcknowledgeInput (handled) via the op channel.
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::AcknowledgeInput {
            projection_id: proj.to_string(),
            owner_token: token.clone(),
            input_id: "input-1".to_string(),
            ack_state: "handled".to_string(),
            ack_message: Some("done".to_string()),
            not_before_wall_us: None,
            reply: tx,
        });
        rx.blocking_recv()
            .expect("acknowledge reply must arrive")
            .expect("acknowledge_input (handled) must be accepted");

        // After a terminal ack, the item is no longer pending.
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::GetPendingInput {
            projection_id: proj.to_string(),
            owner_token: token.clone(),
            max_items: None,
            max_bytes: None,
            reply: tx,
        });
        let batch = rx
            .blocking_recv()
            .expect("get_pending_input reply must arrive")
            .expect("get_pending_input must be accepted");
        assert!(
            batch.items.is_empty(),
            "no pending input should remain after a terminal acknowledgement"
        );

        // Unrecognized ack_state is rejected before the authority is called.
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::AcknowledgeInput {
            projection_id: proj.to_string(),
            owner_token: token.clone(),
            input_id: "input-1".to_string(),
            ack_state: "bogus".to_string(),
            ack_message: None,
            not_before_wall_us: None,
            reply: tx,
        });
        let bad_ack = rx.blocking_recv().expect("acknowledge reply must arrive");
        assert!(
            bad_ack.is_err(),
            "an unrecognized ack_state must be rejected"
        );

        // 4. Detach via the op channel — purges authority + drive state.
        assert!(
            driver.authority_mut().has_projection(proj),
            "projection must exist before detach"
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::Detach {
            projection_id: proj.to_string(),
            owner_token: token.clone(),
            reason: "session ending".to_string(),
            reply: tx,
        });
        rx.blocking_recv()
            .expect("detach reply must arrive")
            .expect("detach must be accepted");
        assert!(
            !driver.authority_mut().has_projection(proj),
            "projection state must be purged after detach"
        );

        // Post-detach, a publish with the old token must fail (session gone).
        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::PublishOutput {
            projection_id: proj.to_string(),
            owner_token: token,
            output_text: "after detach".to_string(),
            logical_unit_id: None,
            output_kind: None,
            content_classification: None,
            coalesce_key: None,
            reply: tx,
        });
        let after = rx.blocking_recv().expect("publish reply must arrive");
        assert!(
            after.is_err(),
            "publishing to a detached projection must be denied"
        );
    }

    #[test]
    fn dispatch_portal_op_operator_cleanup_purges_authority_and_drive_state() {
        let mut driver = InProcessPortalDriver::new();
        driver
            .authority_mut()
            .set_operator_authority("operator-secret")
            .expect("operator credential must configure");
        let proj = "proj-cleanup";

        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::Attach {
            projection_id: proj.to_string(),
            display_name: "Cleanup Test".to_string(),
            idempotency_key: None,
            reply: tx,
        });
        rx.blocking_recv()
            .expect("attach reply must arrive")
            .expect("attach must be accepted");
        assert!(
            driver.authority_mut().has_projection(proj),
            "projection must exist before cleanup"
        );
        assert!(
            driver.drive.entries.contains_key(proj),
            "dispatch attach must create a driver entry"
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::Cleanup {
            projection_id: proj.to_string(),
            cleanup_authority: "operator".to_string(),
            owner_token: None,
            operator_authority: Some("operator-secret".to_string()),
            reason: "operator override".to_string(),
            reply: tx,
        });
        rx.blocking_recv()
            .expect("cleanup reply must arrive")
            .expect("operator cleanup must be accepted");

        assert!(
            !driver.authority_mut().has_projection(proj),
            "operator cleanup must purge authority state"
        );
        assert!(
            !driver.drive.entries.contains_key(proj),
            "operator cleanup must drop the driver entry"
        );
    }

    #[test]
    fn dispatch_portal_op_operator_cleanup_removes_projection_tile_on_next_drain() {
        let mut driver = InProcessPortalDriver::new();
        driver
            .authority_mut()
            .set_operator_authority("operator-secret")
            .expect("operator credential must configure");
        let proj = "proj-cleanup-tile";
        let token = attach_and_get_token(&mut driver, proj);
        driver.attach_projection(proj, Vec::new());

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let mut processor = InputProcessor::new();

        publish(&mut driver, proj, &token, "line before cleanup", 100);
        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 200);
        let tile_id = driver
            .drive
            .entries
            .get(proj)
            .expect("drive entry must exist")
            .tile_scene_id
            .expect("drain must create a portal tile");
        assert!(
            scene.tiles.contains_key(&tile_id),
            "precondition: projected tile must exist before cleanup"
        );
        assert!(
            scene.tile_scroll_config(tile_id).is_some(),
            "precondition: projected tile must have scroll state before cleanup"
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        driver.dispatch_portal_op(PortalOp::Cleanup {
            projection_id: proj.to_string(),
            cleanup_authority: "operator".to_string(),
            owner_token: None,
            operator_authority: Some("operator-secret".to_string()),
            reason: "operator override".to_string(),
            reply: tx,
        });
        rx.blocking_recv()
            .expect("cleanup reply must arrive")
            .expect("operator cleanup must be accepted");
        assert!(
            !driver.drive.entries.contains_key(proj),
            "accepted cleanup must drop the drive entry immediately"
        );
        assert!(
            scene.tiles.contains_key(&tile_id),
            "tile removal is queued until the next drain has scene access"
        );

        driver.drain_inner(&mut scene, &mut processor, Some(tab_id), 300);

        assert!(
            !scene.tiles.contains_key(&tile_id),
            "accepted cleanup must remove the projection tile on the next drain"
        );
        assert!(
            scene.tile_scroll_config(tile_id).is_none(),
            "accepted cleanup must clear scene scroll config for the projection tile"
        );
        assert!(
            !scene.tile_follow_tail_at_tail(tile_id),
            "accepted cleanup must clear follow-tail scene state for the projection tile"
        );
        assert!(
            scene.drain_removed_tile_ids().contains(&tile_id),
            "accepted cleanup must publish a removed-tile notification for external state pruning"
        );
    }
}
