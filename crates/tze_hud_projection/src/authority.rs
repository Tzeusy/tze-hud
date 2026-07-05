//! Core authority state machine for the cooperative HUD projection contract.
//!
//! Moved from `lib.rs` in the P-4 mechanical split (hud-d570a). No logic
//! changes — byte-identical relocation with the minimal visibility adjustments
//! required by Rust's module privacy rules (`pub(super)` where a helper is
//! called from `lib.rs` across the module boundary, before the P-3 portal
//! submodule is extracted).

use crate::contract::*;
use crate::managed_session::*;
use crate::portal_cadence::PortalCadenceCoalescer;
use crate::{
    MAX_CALLER_IDENTITY_BYTES, MAX_HINT_BYTES, MAX_PORTAL_ID_BYTES, MAX_PROJECTION_ID_BYTES,
    MAX_REASON_BYTES, MAX_REQUEST_ID_BYTES, PORTAL_UPDATE_RATE_WINDOW_WALL_US,
};
use std::collections::{HashMap, HashSet, VecDeque};

impl ProjectionResponse {
    fn with_portal_update_state(mut self, session: &ProjectionSession) -> Self {
        self.portal_update_ready = session.last_publish_portal_update_ready;
        self.coalesced_output_count = session.coalesced_portal_update_count;
        self
    }
}

#[derive(Clone, Debug)]
struct ProjectionSession {
    projection_id: String,
    provider_kind: ProviderKind,
    display_name: String,
    workspace_hint: Option<String>,
    repository_hint: Option<String>,
    icon_profile_hint: Option<String>,
    portal_id: String,
    portal_presentation: ProjectedPortalPresentation,
    owner_token_verifier: String,
    owner_token_expires_at_wall_us: u64,
    lifecycle_state: ProjectionLifecycleState,
    latest_status_text: Option<String>,
    content_classification: ContentClassification,
    attach_idempotency_key: Option<String>,
    hud_connection: Option<HudConnectionMetadata>,
    advisory_lease: Option<AdvisoryLeaseIdentity>,
    reconnect: ReconnectBookkeeping,
    retained_transcript: VecDeque<TranscriptUnit>,
    retained_transcript_bytes: usize,
    next_transcript_sequence: u64,
    unread_output_count: usize,
    portal_rate_window_started_at_wall_us: u64,
    portal_updates_in_window: u32,
    coalesced_portal_update_count: usize,
    last_publish_portal_update_ready: bool,
    seen_logical_units: HashSet<String>,
    seen_logical_unit_order: VecDeque<String>,
    completed_input_ack_states: HashMap<String, InputDeliveryState>,
    completed_input_ack_order: VecDeque<String>,
    pending_input: VecDeque<PendingInputItem>,
    pending_input_bytes: usize,
    last_input_feedback: Option<PortalInputFeedback>,
    portal_update_pending: bool,
    /// Pending geometry batch produced by the window management layer.
    ///
    /// Written by [`ProjectionAuthority::push_geometry_snapshot`] when a
    /// pointer resize gesture or hotkey resize produces a new snapshot.
    /// Consumed (cloned) by [`projected_portal_state`] into
    /// `ProjectedPortalState::geometry_batch` and cleared by
    /// [`ProjectionAuthority::consume_geometry_batch`] after delivery.
    ///
    /// `None` until the first snapshot arrives (no resize has occurred).
    pending_geometry_batch: Option<AdapterGeometryBatch>,
    /// Durable latest resized geometry (hud-v4k1h follow-up). Unlike
    /// `pending_geometry_batch`, this is NOT consumed after delivery — it
    /// persists so every subsequent render sizes the portal body + composer to
    /// the resized bounds via `ProjectedPortalState::resized_bounds`. Without it
    /// the body snaps back to the fixed config size one frame after each resize,
    /// leaving the empty "shadow-body" region in the grown tile.
    latest_geometry: Option<AdapterGeometrySnapshot>,
}

struct ProjectionAuditEvent<'a> {
    envelope: &'a OperationEnvelope,
    caller_identity: &'a str,
    server_timestamp_wall_us: u64,
    accepted: bool,
    error_code: Option<ProjectionErrorCode>,
    reason: &'a str,
    category: ProjectionAuditCategory,
}

/// Minimal in-memory authority that enforces the operation contract. Production
/// daemon storage can wrap or replace this, but must preserve these semantics.
#[derive(Debug)]
pub struct ProjectionAuthority {
    bounds: ProjectionBounds,
    sessions: HashMap<String, ProjectionSession>,
    operator_authority_verifier: Option<String>,
    audit_log: Vec<ProjectionAuditRecord>,
    /// Cross-portal cadence coalescer (hud-zmt1a).
    ///
    /// Wires `PortalCadenceCoalescer` into the live streaming presentation path
    /// so that `handle_publish_output` → `take_due_portal_update` respects
    /// round-robin cross-portal fairness (tasks.md §5.1).
    ///
    /// Coalescer keys are `projection_id` values from the request envelope
    /// (i.e. `request.envelope.projection_id`), not the internal `portal_id`
    /// field on `ProjectionSession`.
    cadence_coalescer: PortalCadenceCoalescer,
}

impl ProjectionAuthority {
    pub fn new(bounds: ProjectionBounds) -> Result<Self, ProjectionContractError> {
        bounds.validate()?;
        Ok(Self {
            bounds,
            sessions: HashMap::new(),
            operator_authority_verifier: None,
            audit_log: Vec::new(),
            cadence_coalescer: PortalCadenceCoalescer::new(),
        })
    }

    /// Configure a separate operator authority credential for operator cleanup.
    pub fn set_operator_authority(
        &mut self,
        credential: &str,
    ) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded("operator_authority", credential, MAX_HINT_BYTES)?;
        self.operator_authority_verifier = Some(verifier_for_secret(credential));
        Ok(())
    }

    pub fn bounds(&self) -> &ProjectionBounds {
        &self.bounds
    }

    pub fn audit_log(&self) -> &[ProjectionAuditRecord] {
        &self.audit_log
    }

    pub fn has_projection(&self, projection_id: &str) -> bool {
        self.sessions.contains_key(projection_id)
    }

    pub fn projection_identity(&self, projection_id: &str) -> Option<ProjectionIdentitySummary> {
        self.sessions
            .get(projection_id)
            .map(|session| ProjectionIdentitySummary {
                provider_kind: session.provider_kind.clone(),
                display_name: session.display_name.clone(),
                content_classification: session.content_classification,
                lifecycle_state: session.lifecycle_state,
            })
    }

    pub fn state_summary(&self, projection_id: &str) -> Option<ProjectionStateSummary> {
        self.sessions.get(projection_id).map(|session| {
            let visible_transcript_bytes =
                visible_transcript_window(session, self.bounds.max_visible_transcript_bytes)
                    .iter()
                    .map(TranscriptUnit::byte_len)
                    .sum();
            ProjectionStateSummary {
                projection_id: projection_id.to_string(),
                lifecycle_state: session.lifecycle_state,
                content_classification: session.content_classification,
                has_hud_connection: session.hud_connection.is_some(),
                has_advisory_lease: session.advisory_lease.is_some(),
                retained_transcript_bytes: session.retained_transcript_bytes,
                visible_transcript_bytes,
                retained_transcript_units: session.retained_transcript.len(),
                pending_input_count: session
                    .pending_input
                    .iter()
                    .filter(|item| !item.delivery_state.is_terminal())
                    .count(),
                pending_input_bytes: session.pending_input_bytes,
                unread_output_count: session.unread_output_count,
                reconnect: session.reconnect,
            }
        })
    }

    pub fn visible_transcript_window(&self, projection_id: &str) -> Option<Vec<TranscriptUnit>> {
        self.sessions.get(projection_id).map(|session| {
            visible_transcript_window(session, self.bounds.max_visible_transcript_bytes)
        })
    }

    /// Materialize the bounded text-stream portal state for a projected
    /// session. This returns data for an external daemon/resident-session
    /// adapter; it does not expose runtime scene state or process authority.
    pub fn projected_portal_state(
        &self,
        projection_id: &str,
        policy: &ProjectedPortalPolicy,
    ) -> Option<ProjectedPortalState> {
        self.sessions.get(projection_id).map(|session| {
            projected_portal_state(session, policy, self.bounds.max_visible_transcript_bytes)
        })
    }

    /// Deliver a coalescible portal geometry snapshot from the window management
    /// layer (§6b.4).
    ///
    /// Called by the windowed runtime when a pointer resize gesture step or
    /// hotkey resize produces a new `GeometrySnapshot`. The snapshot is stored
    /// in the session's `pending_geometry_batch`; on the next call to
    /// `projected_portal_state` the batch is included for adapter delivery.
    ///
    /// Returns `true` if the snapshot was accepted (session exists and the new
    /// sequence is strictly greater than the current latest, or there is no
    /// current snapshot). Returns `false` if the session is not found or the
    /// snapshot is stale.
    ///
    /// Callers MUST call [`consume_geometry_batch`] after reading the state to
    /// clear the pending batch; otherwise the same snapshot will be re-delivered
    /// on every subsequent `projected_portal_state` call.
    pub fn push_geometry_snapshot(
        &mut self,
        projection_id: &str,
        snapshot: AdapterGeometrySnapshot,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(projection_id) else {
            return false;
        };
        // Persist the durable resized geometry (latest-wins by sequence) so the
        // rendered body follows the resize on EVERY subsequent frame, not just
        // the one delivery the transient `pending_geometry_batch` survives
        // (hud-v4k1h follow-up — fixes the "shadow-body" left in the grown tile).
        if session
            .latest_geometry
            .is_none_or(|existing| snapshot.sequence > existing.sequence)
        {
            session.latest_geometry = Some(snapshot);
        }
        match &mut session.pending_geometry_batch {
            Some(batch) => {
                // Coalesce: only accept if sequence is strictly newer.
                if snapshot.sequence > batch.latest.map_or(0, |s| s.sequence) {
                    batch.coalesce(snapshot);
                    true
                } else {
                    false
                }
            }
            None => {
                let mut batch = AdapterGeometryBatch::default();
                batch.coalesce(snapshot);
                session.pending_geometry_batch = Some(batch);
                true
            }
        }
    }

    /// Consume (clear) the pending geometry batch for a session after delivery.
    ///
    /// The window management layer calls this after the adapter has been notified
    /// of the geometry change so that the same snapshot is not re-delivered on
    /// the next `projected_portal_state` call.
    ///
    /// No-op if the session does not exist or has no pending batch.
    pub fn consume_geometry_batch(&mut self, projection_id: &str) {
        if let Some(session) = self.sessions.get_mut(projection_id) {
            session.pending_geometry_batch = None;
        }
    }

    /// Collapse a projected portal into its compact content-layer surface.
    pub fn collapse_projected_portal(
        &mut self,
        projection_id: &str,
    ) -> Result<(), ProjectionErrorCode> {
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        session.portal_presentation = ProjectedPortalPresentation::Collapsed;
        Ok(())
    }

    /// Expand a projected portal back to its transcript/composer surface.
    pub fn expand_projected_portal(
        &mut self,
        projection_id: &str,
    ) -> Result<(), ProjectionErrorCode> {
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        session.portal_presentation = ProjectedPortalPresentation::Expanded;
        Ok(())
    }

    pub fn record_hud_connection(
        &mut self,
        projection_id: &str,
        metadata: HudConnectionMetadata,
    ) -> Result<(), ProjectionErrorCode> {
        metadata.validate().map_err(|error| error.code())?;
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let is_reconnect = session
            .reconnect
            .last_reconnect_wall_us
            .is_some_and(|last| last < metadata.last_reconnect_wall_us);
        let connection_changed = session.hud_connection.as_ref().is_some_and(|connection| {
            connection.connection_id != metadata.connection_id
                || connection.authenticated_session_id != metadata.authenticated_session_id
        });
        if is_reconnect {
            session.reconnect.reconnect_count += 1;
        }
        session.reconnect.last_reconnect_wall_us = Some(metadata.last_reconnect_wall_us);
        if is_reconnect || connection_changed {
            session.advisory_lease = None;
        }
        session.hud_connection = Some(metadata);
        promote_to_active_if_recovering(session);
        Ok(())
    }

    pub fn mark_hud_disconnected(
        &mut self,
        projection_id: &str,
        disconnected_at_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        if disconnected_at_wall_us == 0 {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        }
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        session.hud_connection = None;
        session.advisory_lease = None;
        session.reconnect.last_disconnect_wall_us = Some(disconnected_at_wall_us);
        session.lifecycle_state = ProjectionLifecycleState::HudUnavailable;
        Ok(())
    }

    pub fn record_heartbeat(
        &mut self,
        projection_id: &str,
        heartbeat_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        if heartbeat_wall_us == 0 {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        }
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        if session.hud_connection.is_none() {
            return Err(ProjectionErrorCode::ProjectionHudUnavailable);
        }
        if session
            .reconnect
            .last_heartbeat_wall_us
            .is_some_and(|last| heartbeat_wall_us < last)
        {
            return Err(ProjectionErrorCode::ProjectionStateConflict);
        }
        session.reconnect.last_heartbeat_wall_us = Some(heartbeat_wall_us);
        promote_to_active_if_recovering(session);
        Ok(())
    }

    pub fn record_advisory_lease(
        &mut self,
        projection_id: &str,
        lease: AdvisoryLeaseIdentity,
        server_timestamp_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        lease.validate().map_err(|error| error.code())?;
        if server_timestamp_wall_us >= lease.expires_at_wall_us {
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let Some(connection) = session.hud_connection.as_ref() else {
            return Err(ProjectionErrorCode::ProjectionHudUnavailable);
        };
        if !capabilities_are_subset(&lease.capabilities, &connection.granted_capabilities) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        session.advisory_lease = Some(lease);
        Ok(())
    }

    pub fn authorize_portal_republish(
        &mut self,
        projection_id: &str,
        lease_id: &str,
        requested_capabilities: &[String],
        server_timestamp_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        validate_non_empty_bounded("lease_id", lease_id, MAX_HINT_BYTES)
            .map_err(|error| error.code())?;
        for capability in requested_capabilities {
            validate_non_empty_bounded("requested_capability", capability, MAX_HINT_BYTES)
                .map_err(|error| error.code())?;
        }

        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let Some(connection) = session.hud_connection.as_ref() else {
            session.advisory_lease = None;
            return Err(ProjectionErrorCode::ProjectionHudUnavailable);
        };
        if !capabilities_are_subset(requested_capabilities, &connection.granted_capabilities) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        let Some(lease) = session.advisory_lease.as_ref() else {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        };
        if server_timestamp_wall_us >= lease.expires_at_wall_us {
            session.advisory_lease = None;
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
        if lease.lease_id != lease_id {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        if !capabilities_are_subset(requested_capabilities, &lease.capabilities) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        Ok(())
    }

    pub fn take_due_portal_update(
        &mut self,
        projection_id: &str,
        server_timestamp_wall_us: u64,
    ) -> Result<Option<PortalTranscriptUpdate>, ProjectionErrorCode> {
        let max_updates = self.bounds.max_portal_updates_per_second;
        let max_visible = self.bounds.max_visible_transcript_bytes;

        // Peek the submission timestamp from the cadence coalescer before
        // consuming the session borrow. This captures the arrival time of the
        // append for arrival→present latency measurement (hud-zmt1a, tasks.md §5.7).
        let submitted_at_us = self
            .cadence_coalescer
            .peek_submitted_at(projection_id)
            .unwrap_or(0);

        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        if session.unread_output_count == 0 && !session.portal_update_pending {
            return Ok(None);
        }
        if !session.portal_update_pending {
            if !portal_update_allowed(session, server_timestamp_wall_us, max_updates) {
                return Ok(None);
            }
            session.portal_update_pending = true;
        }
        let visible_transcript = visible_transcript_window(session, max_visible);
        let visible_transcript_bytes = visible_transcript
            .iter()
            .map(TranscriptUnit::byte_len)
            .sum();
        let coalesced_output_count = session.coalesced_portal_update_count;
        let unread_output_count = session.unread_output_count;
        session.coalesced_portal_update_count = 0;
        session.unread_output_count = 0;
        session.portal_update_pending = false;
        session.last_publish_portal_update_ready = false;

        // Drain the coalescer entry for this portal (marks it as served).
        // The coalescer snapshot payload is unused here; transcript content
        // comes from the session. The drain records the sequence for the
        // post-drain stale-sequence guard.
        let _ = self.cadence_coalescer.take_snapshot(projection_id);

        Ok(Some(PortalTranscriptUpdate {
            projection_id: projection_id.to_string(),
            visible_transcript,
            visible_transcript_bytes,
            coalesced_output_count,
            unread_output_count,
            submitted_at_us,
        }))
    }

    /// Return the next projection ID that has a pending portal update, in
    /// round-robin fairness order (cross-portal fairness, tasks.md §5.1).
    ///
    /// Returns `None` if no portal has a pending update in the coalescer.
    ///
    /// Use this when driving multiple concurrent portals to ensure no portal
    /// is starved: call `next_due_projection_id()` to pick the portal, then
    /// `take_due_portal_update(id, ...)` to materialize the update.
    ///
    /// ## Rate-limit caution
    ///
    /// This method returns a portal key whenever the coalescer has a **pending
    /// snapshot** for it — it does not check whether the portal's rate window has
    /// elapsed.  A subsequent `take_due_portal_update` call may return `Ok(None)`
    /// because the rate window has not elapsed yet.
    ///
    /// A naive `while let Some(id) = next_due_projection_id()` loop will
    /// busy-spin in that case because the coalescer entry is never consumed on the
    /// `Ok(None)` path.  Use [`portal_next_due_at_us`] to obtain a wait hint and
    /// break/sleep until the window elapses (defect fix: hud-endkj).
    pub fn next_due_projection_id(&mut self) -> Option<String> {
        self.cadence_coalescer.next_ready_portal()
    }

    /// Return the earliest wall-clock time (µs) at which `projection_id` will
    /// be serviced by `take_due_portal_update`.
    ///
    /// Returns `Some(next_due_at_us)` when the portal has a pending coalescer
    /// entry but its per-portal rate window has not yet elapsed.  Returns `None`
    /// when the portal is either unknown, has no pending entry, or is already
    /// within its rate window (i.e. `take_due_portal_update` will not be blocked
    /// by rate-limiting right now).
    ///
    /// Callers that drive the drain loop with [`next_due_projection_id`] should
    /// call this on an `Ok(None)` result from `take_due_portal_update` to avoid
    /// busy-spinning.  Sleep / yield until `server_timestamp_wall_us >= next_due`,
    /// then retry (defect fix: hud-endkj).
    pub fn portal_next_due_at_us(
        &self,
        projection_id: &str,
        server_timestamp_wall_us: u64,
    ) -> Option<u64> {
        // Only meaningful when the coalescer has a pending entry (otherwise
        // take_due_portal_update would have returned Ok(None) for a different reason).
        self.cadence_coalescer.peek_submitted_at(projection_id)?;
        let session = self.sessions.get(projection_id)?;
        let window_start = session.portal_rate_window_started_at_wall_us;
        if window_start == 0 {
            // Rate window never started — portal is immediately serviceable.
            return None;
        }
        let next_due = window_start.checked_add(PORTAL_UPDATE_RATE_WINDOW_WALL_US)?;
        if server_timestamp_wall_us >= next_due {
            // Rate window already elapsed — no wait needed.
            None
        } else {
            Some(next_due)
        }
    }

    /// Peek the submission timestamp (µs) of the pending coalescer entry for
    /// `projection_id`. Returns `None` if no pending entry exists.
    ///
    /// Used by the cadence harness to measure per-append arrival→present elapsed
    /// before consuming the entry with `take_due_portal_update`.
    pub fn peek_portal_submitted_at(&self, projection_id: &str) -> Option<u64> {
        self.cadence_coalescer.peek_submitted_at(projection_id)
    }

    pub fn expire_projection(&mut self, projection_id: &str) -> bool {
        self.cadence_coalescer.remove_portal(projection_id);
        self.sessions.remove(projection_id).is_some()
    }

    /// Discard any pending coalescer entry for `projection_id` without touching
    /// the session map.
    ///
    /// This is the targeted backstop for the drain-loop `Err` arm (hud-bsr7u):
    /// when `take_due_portal_update` returns an error (e.g.
    /// `ProjectionNotFound`) because the session is gone but the coalescer still
    /// holds a pending entry, the caller MUST consume that entry so
    /// `next_due_projection_id` cannot return the same orphaned id again and
    /// busy-spin the event loop. `take_due_portal_update` only consumes the
    /// coalescer entry on its `Ok(Some(_))` path (after the session lookup
    /// succeeds), so the error path leaves the entry stranded.
    ///
    /// Idempotent: a no-op when no pending entry exists for `projection_id`.
    pub fn discard_portal_coalescer_entry(&mut self, projection_id: &str) {
        self.cadence_coalescer.remove_portal(projection_id);
    }

    pub fn expire_token_expired_projections(&mut self, server_timestamp_wall_us: u64) -> usize {
        let before = self.sessions.len();
        // Collect expired IDs first so we can clean up the coalescer.
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                server_timestamp_wall_us >= session.owner_token_expires_at_wall_us
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            self.cadence_coalescer.remove_portal(id);
        }
        self.sessions
            .retain(|_, session| server_timestamp_wall_us < session.owner_token_expires_at_wall_us);
        before - self.sessions.len()
    }

    #[cfg(test)]
    pub fn owner_token_verifier_for_test(&self, projection_id: &str) -> Option<&str> {
        self.sessions
            .get(projection_id)
            .map(|session| session.owner_token_verifier.as_str())
    }

    /// Test-only: inject a coalescer entry for `projection_id` without an
    /// associated session entry.
    ///
    /// This puts the authority into an abnormal state that would not arise in
    /// production (every `record_append` is paired with a live session under
    /// normal operation), but is needed to regression-test the drain loop's
    /// `Err` branch (hud-hkaw2): if `take_due_portal_update` returns
    /// `Err(ProjectionNotFound)` and the coalescer entry is not cleared, the
    /// drain loop spins forever.
    pub fn inject_orphan_coalescer_entry_for_test(&mut self, projection_id: &str) {
        self.cadence_coalescer
            .record_append(projection_id, b"test-orphan".to_vec(), 1, 0);
    }

    // ── Coalescer diagnostic accessors (hud-bq0gl.14) ─────────────────────────

    /// Number of projection IDs currently registered in the cadence coalescer.
    ///
    /// This counts both portals with pending updates and those that are registered
    /// but have already been drained.  Use [`coalescer_pending_portal_count`] for
    /// the subset with a pending coalesced snapshot waiting to be drained.
    pub fn coalescer_portal_count(&self) -> usize {
        self.cadence_coalescer.portal_count()
    }

    /// Number of portals with a pending coalesced snapshot not yet drained.
    pub fn coalescer_pending_portal_count(&self) -> usize {
        self.cadence_coalescer.pending_portal_count()
    }

    /// Total number of coalesced snapshots taken by the drain loop since
    /// `ProjectionAuthority` was constructed.
    ///
    /// Each call to `take_due_portal_update` that returns `Ok(Some(_))` increments
    /// this counter by 1, regardless of how many `PublishOutput` calls were
    /// coalesced into that snapshot.
    pub fn coalescer_total_taken(&self) -> u64 {
        self.cadence_coalescer.total_taken()
    }

    /// Total number of coalesced append operations since `ProjectionAuthority`
    /// was constructed.
    ///
    /// Incremented each time a new `PublishOutput` append supersedes an existing
    /// pending snapshot for a portal (latest-wins coalescing).  A high ratio of
    /// `coalescer_total_coalesced` to `coalescer_total_taken` indicates heavy
    /// burst traffic being efficiently collapsed.
    pub fn coalescer_total_coalesced(&self) -> u64 {
        self.cadence_coalescer.total_coalesced()
    }

    pub fn handle_attach(
        &mut self,
        request: AttachRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        if let Err(error) = validate_non_empty_bounded(
            "caller_identity",
            caller_identity,
            MAX_CALLER_IDENTITY_BYTES,
        ) {
            return self.validation_denial(
                &request.envelope,
                "invalid-caller",
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::AuthDenied,
            );
        }

        if let Some(existing) = self.sessions.get(&request.envelope.projection_id) {
            if request.idempotency_key.is_some()
                && request.idempotency_key == existing.attach_idempotency_key
            {
                let mut response = ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "projection already attached for matching idempotency key",
                );
                response.lifecycle_state = Some(existing.lifecycle_state);
                self.audit(ProjectionAuditEvent {
                    envelope: &request.envelope,
                    caller_identity,
                    server_timestamp_wall_us,
                    accepted: true,
                    error_code: None,
                    reason: "idempotent attach replay",
                    category: ProjectionAuditCategory::Attach,
                });
                return response;
            }
            let response = ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                ProjectionErrorCode::ProjectionAlreadyAttached,
                "projection_id is already attached",
            );
            self.audit(ProjectionAuditEvent {
                envelope: &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                accepted: false,
                error_code: Some(ProjectionErrorCode::ProjectionAlreadyAttached),
                reason: "attach conflict",
                category: ProjectionAuditCategory::ConflictDenied,
            });
            return response;
        }

        let owner_token = match generate_owner_token() {
            Ok(token) => token,
            Err(error) => {
                return self.validation_denial(
                    &request.envelope,
                    caller_identity,
                    server_timestamp_wall_us,
                    error,
                    ProjectionAuditCategory::AuthDenied,
                );
            }
        };
        let owner_token_verifier = verifier_for_secret(&owner_token);
        self.sessions.insert(
            request.envelope.projection_id.clone(),
            ProjectionSession {
                projection_id: request.envelope.projection_id.clone(),
                provider_kind: request.provider_kind,
                display_name: request.display_name,
                workspace_hint: request.workspace_hint,
                repository_hint: request.repository_hint,
                icon_profile_hint: request.icon_profile_hint,
                portal_id: portal_id_for_projection(
                    PortalSurfaceKind::TextStreamRawTile,
                    &request.envelope.projection_id,
                ),
                portal_presentation: ProjectedPortalPresentation::Expanded,
                owner_token_verifier,
                owner_token_expires_at_wall_us: server_timestamp_wall_us
                    + self.bounds.owner_token_ttl_wall_us,
                lifecycle_state: ProjectionLifecycleState::Attached,
                latest_status_text: None,
                content_classification: request.content_classification,
                attach_idempotency_key: request.idempotency_key,
                hud_connection: None,
                advisory_lease: None,
                reconnect: ReconnectBookkeeping::default(),
                retained_transcript: VecDeque::new(),
                retained_transcript_bytes: 0,
                next_transcript_sequence: 0,
                unread_output_count: 0,
                portal_rate_window_started_at_wall_us: 0,
                portal_updates_in_window: 0,
                coalesced_portal_update_count: 0,
                last_publish_portal_update_ready: false,
                seen_logical_units: HashSet::new(),
                seen_logical_unit_order: VecDeque::new(),
                completed_input_ack_states: HashMap::new(),
                completed_input_ack_order: VecDeque::new(),
                pending_input: VecDeque::new(),
                pending_input_bytes: 0,
                last_input_feedback: None,
                portal_update_pending: false,
                pending_geometry_batch: None,
                latest_geometry: None,
            },
        );

        let mut response = ProjectionResponse::accepted(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            "projection attached",
        );
        response.owner_token = Some(owner_token);
        response.lifecycle_state = Some(ProjectionLifecycleState::Attached);
        self.audit(ProjectionAuditEvent {
            envelope: &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            accepted: true,
            error_code: None,
            reason: "attach accepted",
            category: ProjectionAuditCategory::Attach,
        });
        response
    }

    pub fn handle_publish_output(
        &mut self,
        request: PublishOutputRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate(&self.bounds) {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let max_retained_transcript_bytes = self.bounds.max_retained_transcript_bytes;
        let max_seen_logical_units = self.bounds.max_seen_logical_units;
        let max_visible_transcript_bytes = self.bounds.max_visible_transcript_bytes;
        let max_portal_updates_per_second = self.bounds.max_portal_updates_per_second;

        // Collect (projection_id, sequence) for cadence coalescer wiring after the
        // session borrow is released. Set to `Some(...)` only when an append is
        // actually stored (not on idempotent duplicate or auth failure).
        let mut cadence_append: Option<(String, u64)> = None;

        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerPublish,
        ) {
            Ok(session) => {
                // A publish with a `logical_unit_id` we have already seen is an
                // idempotent duplicate (drop it). Any publish without an id, or
                // with a not-yet-seen id, is a fresh append. `is_some_and` only
                // calls `remember_logical_unit` (which records the id) when an id
                // is present, preserving the original side-effect ordering.
                let is_duplicate = request
                    .logical_unit_id
                    .as_ref()
                    .is_some_and(|id| remember_logical_unit(session, id, max_seen_logical_units));
                if is_duplicate {
                    ProjectionResponse::accepted(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        "duplicate logical_unit_id accepted idempotently",
                    )
                } else {
                    let coalescer_seq = append_transcript_unit(
                        session,
                        &request,
                        server_timestamp_wall_us,
                        max_retained_transcript_bytes,
                        max_visible_transcript_bytes,
                        max_portal_updates_per_second,
                    );
                    // Capture for cadence coalescer. `append_transcript_unit`
                    // returns the coalescer sequence — strictly greater than any
                    // previously drained or pending sequence — so `record_append`
                    // always clears the post-drain stale-sequence guard even on the
                    // coalesce-key in-place update path (defect fix: hud-endkj).
                    cadence_append = Some((request.envelope.projection_id.clone(), coalescer_seq));
                    let mut response = ProjectionResponse::accepted(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        "output accepted",
                    )
                    .with_portal_update_state(session);
                    response.status_summary = if response.portal_update_ready {
                        "output accepted".to_string()
                    } else {
                        "output accepted and coalesced for next portal update".to_string()
                    };
                    response
                }
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };

        // Wire cadence coalescer: register the append for cross-portal fairness
        // scheduling. The payload is empty — the coalescer is used as a scheduling
        // oracle only; transcript content is fetched from the session on drain.
        // submitted_at_us is recorded for arrival→present latency measurement.
        if let Some((projection_id, sequence)) = cadence_append {
            self.cadence_coalescer.record_append(
                &projection_id,
                Vec::new(),
                sequence,
                server_timestamp_wall_us,
            );
        }

        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerPublish
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_publish_status(
        &mut self,
        request: PublishStatusRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate(&self.bounds) {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        // Collect (projection_id, sequence, submitted_at) for cadence coalescer
        // wiring after the session borrow is released. Set to `Some(...)` only on
        // an accepted status update so the portal is marked due and the drain loop
        // re-materialises the viewer-facing lifecycle/status — a status-only
        // publish carries no transcript content, so without this the new state
        // would stay invisible until some unrelated publish/input made the portal
        // due (mirrors the content-less refresh in `handle_input_ack`).
        let mut cadence_append: Option<(String, u64, u64)> = None;
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerStatus,
        ) {
            Ok(session) => {
                session.lifecycle_state = request.lifecycle_state;
                session.latest_status_text = request.status_text;
                cadence_append = Some((
                    request.envelope.projection_id.clone(),
                    schedule_portal_state_update(session),
                    server_timestamp_wall_us,
                ));
                let mut response = ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "status accepted",
                );
                response.lifecycle_state = Some(session.lifecycle_state);
                response
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        // Register the status refresh for cross-portal fairness scheduling. The
        // payload is empty — the coalescer is a scheduling oracle only; the
        // lifecycle/status is read from the session on drain. submitted_at_us is
        // recorded for publish→present latency measurement.
        if let Some((projection_id, sequence, submitted_at_wall_us)) = cadence_append {
            self.cadence_coalescer.record_append(
                &projection_id,
                Vec::new(),
                sequence,
                submitted_at_wall_us,
            );
        }
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerStatus
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn enqueue_input(
        &mut self,
        projection_id: &str,
        input_id: &str,
        submission_text: String,
        submitted_at_wall_us: u64,
        expires_at_wall_us: u64,
        content_classification: Option<ContentClassification>,
    ) -> Result<(), ProjectionErrorCode> {
        let item = PendingInputItem {
            input_id: input_id.to_string(),
            projection_id: projection_id.to_string(),
            submission_text,
            submitted_at_wall_us,
            expires_at_wall_us,
            delivery_state: InputDeliveryState::Pending,
            delivered_at_wall_us: None,
            not_before_wall_us: None,
            content_classification: content_classification.unwrap_or_default(),
        };
        self.enqueue_input_item(projection_id, item)
    }

    /// Submit HUD composer text into the cooperative pending-input inbox and
    /// return bounded local-first feedback for the portal surface.
    ///
    /// On success, the submission text is also echoed into the session
    /// transcript as an [`OutputKind::Viewer`] unit so the portal surface
    /// renders a complete conversation rather than a structurally one-sided
    /// agent-only view.
    pub fn submit_portal_input(
        &mut self,
        projection_id: &str,
        submission: PortalInputSubmission,
    ) -> PortalInputFeedback {
        let input_id = submission.input_id.clone();
        let submitted_at_wall_us = submission.submitted_at_wall_us;
        // Capture before submission fields are consumed by the PendingInputItem move.
        let submission_text = submission.submission_text.clone();
        let content_classification = submission.content_classification;
        let result = match submission.effective_expires_at_wall_us() {
            Ok(expires_at_wall_us) => self.enqueue_input_item(
                projection_id,
                PendingInputItem {
                    input_id: submission.input_id,
                    projection_id: projection_id.to_string(),
                    submission_text: submission.submission_text,
                    submitted_at_wall_us: submission.submitted_at_wall_us,
                    expires_at_wall_us,
                    delivery_state: InputDeliveryState::Pending,
                    delivered_at_wall_us: None,
                    not_before_wall_us: None,
                    content_classification: submission.content_classification,
                },
            ),
            Err(code) => Err(code),
        };
        // On success, echo the viewer's text into the transcript via the same
        // append path used by `handle_publish_output`.
        let max_retained_transcript_bytes = self.bounds.max_retained_transcript_bytes;
        let max_visible_transcript_bytes = self.bounds.max_visible_transcript_bytes;
        let max_portal_updates_per_second = self.bounds.max_portal_updates_per_second;
        let cadence_append = if result.is_ok() {
            self.sessions.get_mut(projection_id).map(|session| {
                let viewer_request = PublishOutputRequest {
                    envelope: OperationEnvelope {
                        operation: ProjectionOperation::PublishOutput,
                        projection_id: projection_id.to_string(),
                        request_id: "viewer-echo".to_string(),
                        client_timestamp_wall_us: submitted_at_wall_us,
                    },
                    owner_token: String::new(),
                    output_text: submission_text,
                    output_kind: OutputKind::Viewer,
                    content_classification,
                    logical_unit_id: None,
                    coalesce_key: None,
                    // A viewer's own echoed reply is never itself a pending
                    // question — this is what clears the awaiting-reply cue
                    // once the viewer responds (hud-jip0k).
                    expects_reply: false,
                };
                let sequence = append_transcript_unit(
                    session,
                    &viewer_request,
                    submitted_at_wall_us,
                    max_retained_transcript_bytes,
                    max_visible_transcript_bytes,
                    max_portal_updates_per_second,
                );
                (projection_id.to_string(), sequence, submitted_at_wall_us)
            })
        } else {
            None
        };

        let (pending_input_count, pending_input_bytes) = self
            .state_summary(projection_id)
            .map(|summary| (summary.pending_input_count, summary.pending_input_bytes))
            .unwrap_or_default();
        let feedback = match result {
            Ok(()) => PortalInputFeedback {
                projection_id: projection_id.to_string(),
                input_id,
                feedback_state: PortalInputFeedbackState::Accepted,
                error_code: None,
                pending_input_count,
                pending_input_bytes,
                status_summary: "portal input accepted".to_string(),
            },
            Err(code) => PortalInputFeedback {
                projection_id: projection_id.to_string(),
                input_id,
                feedback_state: PortalInputFeedbackState::Rejected,
                error_code: Some(code),
                pending_input_count,
                pending_input_bytes,
                status_summary: format!("{code}: portal input rejected"),
            },
        };
        if let Some(session) = self.sessions.get_mut(projection_id) {
            session.last_input_feedback = Some(feedback.clone());
        }
        if let Some((projection_id, sequence, submitted_at_wall_us)) = cadence_append {
            self.cadence_coalescer.record_append(
                &projection_id,
                Vec::new(),
                sequence,
                submitted_at_wall_us,
            );
        }
        feedback
    }

    fn enqueue_input_item(
        &mut self,
        projection_id: &str,
        item: PendingInputItem,
    ) -> Result<(), ProjectionErrorCode> {
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        validate_pending_input_item(&item, &self.bounds)?;
        prune_terminal_pending_input(session, self.bounds.max_pending_input_items);
        if session
            .pending_input
            .iter()
            .any(|pending| pending.input_id == item.input_id)
            || session
                .completed_input_ack_states
                .contains_key(&item.input_id)
        {
            return Err(ProjectionErrorCode::ProjectionStateConflict);
        }
        if item.submission_text.len() > self.bounds.max_pending_input_bytes_per_item {
            return Err(ProjectionErrorCode::ProjectionInputTooLarge);
        }
        if session.pending_input.len() >= self.bounds.max_pending_input_items {
            return Err(ProjectionErrorCode::ProjectionInputQueueFull);
        }
        if session.pending_input_bytes + item.submission_text.len()
            > self.bounds.max_pending_input_total_bytes
        {
            return Err(ProjectionErrorCode::ProjectionInputQueueFull);
        }
        session.pending_input_bytes += item.submission_text.len();
        session.pending_input.push_back(item);
        Ok(())
    }

    pub fn handle_get_pending_input(
        &mut self,
        request: GetPendingInputRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let max_items = request
            .max_items
            .unwrap_or(self.bounds.max_poll_items)
            .min(self.bounds.max_poll_items);
        let max_bytes = request
            .max_bytes
            .unwrap_or(self.bounds.max_poll_response_bytes)
            .min(self.bounds.max_poll_response_bytes);
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerInputRead,
        ) {
            Ok(session) => {
                expire_pending(session, server_timestamp_wall_us);
                let mut used_bytes = 0usize;
                let mut returned = Vec::new();
                let mut remaining_count = 0usize;
                let mut remaining_bytes = 0usize;
                for item in session.pending_input.iter_mut() {
                    if !matches!(
                        item.delivery_state,
                        InputDeliveryState::Pending | InputDeliveryState::Deferred
                    ) {
                        continue;
                    }
                    if item.delivery_state == InputDeliveryState::Deferred
                        && item
                            .not_before_wall_us
                            .is_some_and(|not_before| server_timestamp_wall_us < not_before)
                    {
                        continue;
                    }
                    let item_bytes = item.submission_text.len();
                    if returned.len() < max_items && used_bytes + item_bytes <= max_bytes {
                        item.delivery_state = InputDeliveryState::Delivered;
                        item.delivered_at_wall_us = Some(server_timestamp_wall_us);
                        used_bytes += item_bytes;
                        returned.push(item.clone());
                    } else {
                        remaining_count += 1;
                        remaining_bytes += item_bytes;
                    }
                }
                let mut response = ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "pending input returned",
                );
                response.pending_input = returned;
                response.pending_remaining_count = remaining_count;
                response.pending_remaining_bytes = remaining_bytes;
                response
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerInputRead
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_acknowledge_input(
        &mut self,
        request: AcknowledgeInputRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let mut cadence_append: Option<(String, u64, u64)> = None;
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerInputAck,
        ) {
            Ok(session) => {
                let (response, state_changed) =
                    acknowledge_input(session, &request, server_timestamp_wall_us);
                if state_changed {
                    cadence_append = Some((
                        request.envelope.projection_id.clone(),
                        schedule_portal_state_update(session),
                        server_timestamp_wall_us,
                    ));
                }
                response
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        if let Some((projection_id, sequence, submitted_at_wall_us)) = cadence_append {
            self.cadence_coalescer.record_append(
                &projection_id,
                Vec::new(),
                sequence,
                submitted_at_wall_us,
            );
        }
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerInputAck
            } else if response.error_code == Some(ProjectionErrorCode::ProjectionStateConflict) {
                ProjectionAuditCategory::ConflictDenied
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_detach(
        &mut self,
        request: DetachRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerDetach,
        ) {
            Ok(_) => {
                self.cadence_coalescer
                    .remove_portal(&request.envelope.projection_id);
                self.sessions.remove(&request.envelope.projection_id);
                ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "projection detached and private state purged",
                )
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerDetach
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_cleanup(
        &mut self,
        request: CleanupRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }

        let response = match request.cleanup_authority {
            CleanupAuthority::Owner => {
                let owner_token = request.owner_token.as_deref().unwrap_or_default();
                match self.authorize_owner(
                    &request.envelope,
                    owner_token,
                    server_timestamp_wall_us,
                    ProjectionAuditCategory::OwnerCleanup,
                ) {
                    Ok(_) => {
                        self.cadence_coalescer
                            .remove_portal(&request.envelope.projection_id);
                        self.sessions.remove(&request.envelope.projection_id);
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "owner cleanup purged projection state",
                        )
                    }
                    Err(code) => ProjectionResponse::denied(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        code,
                        "owner authorization failed",
                    ),
                }
            }
            CleanupAuthority::Operator => {
                let credential = request.operator_authority.as_deref().unwrap_or_default();
                if self
                    .operator_authority_verifier
                    .as_deref()
                    .is_some_and(|verifier| {
                        constant_time_eq(verifier, &verifier_for_secret(credential))
                    })
                {
                    // Purge the coalescer entry BEFORE removing the session so
                    // both maps stay consistent regardless of which branch is
                    // taken. The owner-cleanup branch (above), `handle_detach`,
                    // `expire_projection`, and `expire_token_expired_projections`
                    // all purge both maps; this operator branch previously purged
                    // only the session, leaving an orphaned coalescer entry that
                    // busy-spun the drain loop (hud-bsr7u).
                    self.cadence_coalescer
                        .remove_portal(&request.envelope.projection_id);
                    if self
                        .sessions
                        .remove(&request.envelope.projection_id)
                        .is_some()
                    {
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "operator cleanup purged projection state",
                        )
                    } else {
                        ProjectionResponse::denied(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            ProjectionErrorCode::ProjectionNotFound,
                            "projection not found",
                        )
                    }
                } else {
                    ProjectionResponse::denied(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        ProjectionErrorCode::ProjectionUnauthorized,
                        "operator authority failed",
                    )
                }
            }
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            match (response.accepted, request.cleanup_authority) {
                (true, CleanupAuthority::Owner) => ProjectionAuditCategory::OwnerCleanup,
                (true, CleanupAuthority::Operator) => ProjectionAuditCategory::OperatorCleanup,
                (false, _) => ProjectionAuditCategory::AuthDenied,
            },
        );
        response
    }

    fn authorize_owner(
        &mut self,
        envelope: &OperationEnvelope,
        owner_token: &str,
        server_timestamp_wall_us: u64,
        _category: ProjectionAuditCategory,
    ) -> Result<&mut ProjectionSession, ProjectionErrorCode> {
        if self
            .sessions
            .get(&envelope.projection_id)
            .is_some_and(|session| {
                server_timestamp_wall_us >= session.owner_token_expires_at_wall_us
            })
        {
            self.cadence_coalescer
                .remove_portal(&envelope.projection_id);
            self.sessions.remove(&envelope.projection_id);
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
        let session = self
            .sessions
            .get_mut(&envelope.projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let presented = verifier_for_secret(owner_token);
        if !constant_time_eq(&session.owner_token_verifier, &presented) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        Ok(session)
    }

    fn validation_denial(
        &mut self,
        envelope: &OperationEnvelope,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
        error: ProjectionContractError,
        category: ProjectionAuditCategory,
    ) -> ProjectionResponse {
        let code = error.code();
        let response = ProjectionResponse::denied(
            &envelope.request_id,
            &envelope.projection_id,
            server_timestamp_wall_us,
            code,
            error.to_string(),
        );
        self.audit_from_response(
            envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            category,
        );
        response
    }

    fn audit_from_response(
        &mut self,
        envelope: &OperationEnvelope,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
        response: &ProjectionResponse,
        category: ProjectionAuditCategory,
    ) {
        self.audit(ProjectionAuditEvent {
            envelope,
            caller_identity,
            server_timestamp_wall_us,
            accepted: response.accepted,
            error_code: response.error_code,
            reason: &response.status_summary,
            category,
        });
    }

    fn audit(&mut self, event: ProjectionAuditEvent<'_>) {
        self.audit_log.push(ProjectionAuditRecord {
            timestamp_wall_us: event.server_timestamp_wall_us,
            operation: event.envelope.operation,
            projection_id: event.envelope.projection_id.clone(),
            caller_identity: bounded_copy(
                event.caller_identity.to_string(),
                MAX_CALLER_IDENTITY_BYTES,
            ),
            request_id: event.envelope.request_id.clone(),
            accepted: event.accepted,
            error_code: event.error_code,
            reason: bounded_copy(event.reason.to_string(), MAX_REASON_BYTES),
            category: event.category,
        });
        if self.audit_log.len() > self.bounds.max_audit_records {
            let overflow = self.audit_log.len() - self.bounds.max_audit_records;
            self.audit_log.drain(0..overflow);
        }
    }
}

impl Default for ProjectionAuthority {
    fn default() -> Self {
        Self::new(ProjectionBounds::default()).expect("default projection bounds are valid")
    }
}

pub(crate) fn route_plan_for_request(
    request: &ManagedSessionRequest,
    target: &WindowsHudTarget,
) -> ManagedSessionRoutePlan {
    let agent_id = format!("projection:{}", request.projection_id);
    let surface_command = match &request.surface_route {
        PresenceSurfaceRoute::Zone {
            zone_name,
            content_kind,
            ttl_ms,
        } => HudSurfaceCommandPlan::ZonePublish {
            zone_name: zone_name.clone(),
            content_kind: content_kind.clone(),
            ttl_ms: *ttl_ms,
            agent_id,
        },
        PresenceSurfaceRoute::Widget {
            widget_name,
            parameters,
            ttl_ms,
        } => HudSurfaceCommandPlan::WidgetPublish {
            widget_name: widget_name.clone(),
            parameters: parameters.clone(),
            ttl_ms: *ttl_ms,
            agent_id,
        },
        PresenceSurfaceRoute::Portal {
            portal_surface,
            requested_capabilities,
            lease_ttl_ms,
        } => HudSurfaceCommandPlan::PortalLease {
            portal_surface: *portal_surface,
            portal_id: portal_id_for_projection(*portal_surface, &request.projection_id),
            requested_capabilities: requested_capabilities.clone(),
            lease_ttl_ms: *lease_ttl_ms,
            agent_id,
        },
    };

    ManagedSessionRoutePlan {
        projection_id: request.projection_id.clone(),
        provider_kind: request.provider_kind.clone(),
        display_name: request.display_name.clone(),
        origin: request.origin.clone(),
        hud_target_id: target.target_id.clone(),
        runtime_audience: target.runtime_audience.clone(),
        credential_redacted: target.credential_source.redacted_marker(),
        lifecycle_state: ProjectionLifecycleState::Attached,
        content_classification: request.content_classification,
        attention_intent: request.attention_intent,
        surface_command,
        cleanup_on_detach: true,
    }
}

fn projected_portal_state(
    session: &ProjectionSession,
    policy: &ProjectedPortalPolicy,
    max_visible_transcript_bytes: usize,
) -> ProjectedPortalState {
    let projection_visible = policy.permits(session.content_classification);
    let expanded = session.portal_presentation == ProjectedPortalPresentation::Expanded;
    let identity_visible = projection_visible && policy.reveal_identity;
    let lifecycle_visible = projection_visible && policy.reveal_lifecycle;
    let transcript_visible = expanded && projection_visible && policy.reveal_transcript;
    let unread_visible = projection_visible && policy.reveal_unread;
    let pending_visible = projection_visible && policy.reveal_pending_input;
    let redacted = !identity_visible || !lifecycle_visible || (expanded && !transcript_visible);
    // Content-free connection-degraded signal (portal-disconnect-resume-ux §2/§3).
    // Latched on the actual HUD connection bookkeeping, independent of viewer
    // redaction, so it survives redaction exactly like the scroll-position
    // indicator: a restricted viewer still learns the portal is disconnected
    // without any transcript/identity leak. The richer `lifecycle_state` below
    // stays redaction-gated.
    //
    // The signal is keyed off `hud_connection`/`last_disconnect_wall_us`, NOT
    // `lifecycle_state`, because owner traffic clears the lifecycle latch but not
    // the connection: `handle_publish_output` accepts owner publishes while the
    // HUD is gone (`hud_connection == None`) and `append_transcript_unit`
    // promotes `HudUnavailable` back to `Active`. Deriving from `lifecycle_state`
    // would silently drop the stale treatment during the orphan/grace window even
    // though `authorize_portal_republish` still fails — the surface would un-dim
    // and re-enable input without the HUD ever reconnecting. Only
    // `record_hud_connection` restores `hud_connection = Some`, so this latch
    // clears exactly on a genuine reconnect. `last_disconnect_wall_us.is_some()`
    // gates out a freshly-attached, never-connected portal (which is "connecting",
    // not "disconnected").
    let connection_degraded =
        session.hud_connection.is_none() && session.reconnect.last_disconnect_wall_us.is_some();
    // Content-free "ever connected" signal (portal-chat-grade-affordances
    // §Connecting State Distinction). True once the session has a live HUD
    // connection OR has recorded a disconnect (connected-then-dropped still
    // counts as ever-connected). It is false only for a freshly-attached,
    // never-connected portal — exactly the "connecting" case the disconnect
    // comment above gates out of `connection_degraded`. Redaction-independent,
    // like `connection_degraded`: connection state only, no content leak.
    let has_ever_connected =
        session.hud_connection.is_some() || session.reconnect.last_disconnect_wall_us.is_some();
    let interaction_enabled = session.portal_presentation == ProjectedPortalPresentation::Expanded
        && projection_visible
        && policy.allow_input
        && !redacted
        && !connection_degraded
        && !policy.safe_mode_active
        && !policy.frozen
        && !policy.dismissed;
    let visible_transcript: Vec<TranscriptUnit> = if transcript_visible {
        visible_transcript_window(session, max_visible_transcript_bytes)
            .into_iter()
            .filter(|unit| policy.permits(unit.content_classification))
            .collect()
    } else {
        Vec::new()
    };
    let visible_transcript_bytes = visible_transcript
        .iter()
        .map(TranscriptUnit::byte_len)
        .sum();
    // Clearance-corrected unread count for the in-transcript unread divider
    // (hud-g1ena.2): of the last-N non-viewer retained units (the unread suffix,
    // N = session.unread_output_count), how many survive THIS viewer's clearance
    // filter. The divider is placed with this count so a higher-classification
    // unread turn filtered out of `visible_transcript` cannot push the divider
    // onto an already-seen visible turn. `unread_output_count` stays the aggregate
    // for the ambient count, which MAY exceed the units below the divider. Viewer
    // echoes are never unread, so they are skipped, matching the append-side
    // `unread_output_count` bookkeeping.
    let visible_unread_output_count = {
        let unread_suffix_len = session.unread_output_count;
        let mut nonviewer_seen = 0usize;
        let mut visible_unread = 0usize;
        for unit in session.retained_transcript.iter().rev() {
            if unit.output_kind == OutputKind::Viewer {
                continue;
            }
            nonviewer_seen += 1;
            if nonviewer_seen > unread_suffix_len {
                break;
            }
            if policy.permits(unit.content_classification) {
                visible_unread += 1;
            }
        }
        visible_unread
    };
    let pending_input_count = session
        .pending_input
        .iter()
        .filter(|item| !item.delivery_state.is_terminal())
        .count();

    ProjectedPortalState {
        projection_id: session.projection_id.clone(),
        portal_id: session.portal_id.clone(),
        adapter_family: ProjectedPortalAdapterFamily::CooperativeProjection,
        runtime_authority: ProjectedPortalRuntimeAuthority::ResidentSessionLease,
        layer: ProjectedPortalLayer::Content,
        presentation: session.portal_presentation,
        preserve_geometry: true,
        redacted,
        connection_degraded,
        has_ever_connected,
        interaction_enabled,
        attention: ProjectedPortalAttention::Ambient,
        provider_kind: identity_visible.then(|| session.provider_kind.clone()),
        display_name: identity_visible.then(|| session.display_name.clone()),
        workspace_hint: identity_visible
            .then(|| session.workspace_hint.clone())
            .flatten(),
        repository_hint: identity_visible
            .then(|| session.repository_hint.clone())
            .flatten(),
        icon_profile_hint: identity_visible
            .then(|| session.icon_profile_hint.clone())
            .flatten(),
        lifecycle_state: lifecycle_visible.then_some(session.lifecycle_state),
        status_text: lifecycle_visible
            .then(|| session.latest_status_text.clone())
            .flatten(),
        visible_transcript,
        visible_transcript_bytes,
        unread_output_count: unread_visible.then_some(session.unread_output_count),
        visible_unread_output_count: unread_visible.then_some(visible_unread_output_count),
        pending_input_count: pending_visible.then_some(pending_input_count),
        pending_input_bytes: pending_visible.then_some(session.pending_input_bytes),
        last_input_feedback: session.last_input_feedback.as_ref().and_then(|feedback| {
            pending_visible.then(|| {
                if redacted {
                    redacted_feedback(feedback)
                } else {
                    feedback.clone()
                }
            })
        }),
        // draft_batch is populated externally by the adapter (daemon side) when
        // the runtime delivers draft notifications. The authority does not own the
        // draft buffer state — the runtime's ComposerDraft does.
        draft_batch: None,
        // geometry_batch: include any pending snapshot that was pushed by the
        // window management layer via `push_geometry_snapshot`. The batch is
        // cloned here so it survives serialization to the adapter; the caller
        // MUST call `consume_geometry_batch` after delivery to clear it.
        // `None` until the first resize gesture or hotkey resize occurs.
        geometry_batch: session.pending_geometry_batch.clone(),
        // Durable resized bounds: drives the rendered body/composer size every
        // frame (preserve_geometry is always true on this path), so the portal
        // body grows with the tile instead of leaving a "shadow" (hud-v4k1h).
        resized_bounds: session.latest_geometry.map(|snapshot| snapshot.rect),
    }
}

fn redacted_feedback(feedback: &PortalInputFeedback) -> PortalInputFeedback {
    PortalInputFeedback {
        projection_id: feedback.projection_id.clone(),
        input_id: String::new(),
        feedback_state: feedback.feedback_state,
        error_code: feedback.error_code,
        pending_input_count: feedback.pending_input_count,
        pending_input_bytes: feedback.pending_input_bytes,
        status_summary: feedback.status_summary.clone(),
    }
}

fn portal_id_for_projection(portal_surface: PortalSurfaceKind, projection_id: &str) -> String {
    let prefix = match portal_surface {
        PortalSurfaceKind::TextStreamRawTile => "text-stream://projection/",
    };
    let mut portal_id = String::with_capacity(prefix.len() + projection_id.len());
    portal_id.push_str(prefix);
    portal_id.push_str(projection_id);
    bounded_copy(portal_id, MAX_PORTAL_ID_BYTES)
}

fn validate_pending_input_item(
    item: &PendingInputItem,
    bounds: &ProjectionBounds,
) -> Result<(), ProjectionErrorCode> {
    validate_non_empty_bounded("input_id", &item.input_id, MAX_REQUEST_ID_BYTES)
        .map_err(|error| error.code())?;
    validate_non_empty_bounded(
        "projection_id",
        &item.projection_id,
        MAX_PROJECTION_ID_BYTES,
    )
    .map_err(|error| error.code())?;
    if item.submitted_at_wall_us == 0
        || item.expires_at_wall_us == 0
        || item.submitted_at_wall_us >= item.expires_at_wall_us
    {
        return Err(ProjectionErrorCode::ProjectionInvalidArgument);
    }
    if item.submission_text.len() > bounds.max_pending_input_bytes_per_item {
        return Err(ProjectionErrorCode::ProjectionInputTooLarge);
    }
    Ok(())
}

/// Append or coalesce a transcript unit for `session`.
///
/// Returns the **coalescer sequence** — a monotonically increasing value that
/// callers must pass to [`PortalCadenceCoalescer::record_append`].
///
/// ## Why a separate return value?
///
/// Two distinct code paths exist inside this function:
///
/// 1. **New-unit path**: a new [`TranscriptUnit`] is pushed with the current
///    `next_transcript_sequence`, which is then incremented.  The coalescer
///    receives the newly allocated sequence.
///
/// 2. **Coalesce-key in-place update path**: an existing unit is mutated in-place
///    and `next_transcript_sequence` is NOT incremented (the transcript unit keeps
///    its original sequence).  If the caller naively passed
///    `next_transcript_sequence - 1` to `record_append`, it would be the same
///    sequence that was already drained — the post-drain stale-sequence guard in
///    [`PortalCadenceCoalescer`] would drop it as stale, so the coalesced
///    final-state value would never be presented.
///
///    Fix: on the in-place path, bump `next_transcript_sequence` and return the
///    new value so the coalescer always receives a strictly-increasing sequence
///    that clears the stale guard.  `next_transcript_sequence` is an internal
///    counter used by both the transcript and the coalescer; bumping it here is
///    safe because it is never used to infer the number of stored units.
fn append_transcript_unit(
    session: &mut ProjectionSession,
    request: &PublishOutputRequest,
    server_timestamp_wall_us: u64,
    max_retained_transcript_bytes: usize,
    max_visible_transcript_bytes: usize,
    max_portal_updates_per_second: u32,
) -> u64 {
    let portal_update_ready = portal_update_allowed(
        session,
        server_timestamp_wall_us,
        max_portal_updates_per_second,
    );
    session.last_publish_portal_update_ready = portal_update_ready;
    if portal_update_ready {
        session.portal_update_pending = true;
    }

    if !portal_update_ready {
        session.coalesced_portal_update_count += 1;
        if let Some(coalesce_key) = &request.coalesce_key {
            if let Some(existing) = session
                .retained_transcript
                .iter_mut()
                .rev()
                .find(|unit| unit.coalesce_key.as_ref() == Some(coalesce_key))
            {
                session.retained_transcript_bytes = session
                    .retained_transcript_bytes
                    .saturating_sub(existing.byte_len());
                existing.output_text = request.output_text.clone();
                existing.output_kind = request.output_kind;
                existing.content_classification = request.content_classification;
                existing.logical_unit_id = request.logical_unit_id.clone();
                existing.expects_reply = request.expects_reply;
                existing.appended_at_wall_us = server_timestamp_wall_us;
                session.retained_transcript_bytes += existing.byte_len();
                prune_retained_transcript(
                    session,
                    max_retained_transcript_bytes,
                    max_visible_transcript_bytes,
                );
                promote_to_active_if_recovering(session);
                // Viewer echoes are the viewer's own already-seen text, so they do
                // not raise unread/attention (text-stream-portals "Viewer Reply Echo").
                // They must still be drainable, though: flag the update pending so a
                // rate-limited (coalesced) viewer echo is not stranded by
                // take_due_portal_update's `unread==0 && !pending` early return.
                if request.output_kind != OutputKind::Viewer {
                    session.unread_output_count += 1;
                } else {
                    session.portal_update_pending = true;
                }
                // Bump next_transcript_sequence so the coalescer receives a
                // strictly-increasing sequence that clears the post-drain
                // stale-sequence guard (defect fix: hud-endkj).
                session.next_transcript_sequence += 1;
                return session.next_transcript_sequence - 1;
            }
        }
    }

    let unit = TranscriptUnit {
        sequence: session.next_transcript_sequence,
        output_text: request.output_text.clone(),
        output_kind: request.output_kind,
        content_classification: request.content_classification,
        logical_unit_id: request.logical_unit_id.clone(),
        coalesce_key: request.coalesce_key.clone(),
        expects_reply: request.expects_reply,
        appended_at_wall_us: server_timestamp_wall_us,
    };
    session.next_transcript_sequence += 1;
    session.retained_transcript_bytes += unit.byte_len();
    session.retained_transcript.push_back(unit);
    prune_retained_transcript(
        session,
        max_retained_transcript_bytes,
        max_visible_transcript_bytes,
    );
    promote_to_active_if_recovering(session);
    // Viewer echoes are the viewer's own already-seen text, so they do not raise
    // unread/attention (text-stream-portals "Viewer Reply Echo"). They must still
    // be drainable: flag the update pending so a rate-limited viewer echo is not
    // stranded by take_due_portal_update's `unread==0 && !pending` early return.
    if request.output_kind != OutputKind::Viewer {
        session.unread_output_count += 1;
    } else {
        session.portal_update_pending = true;
    }
    session.next_transcript_sequence - 1
}

fn schedule_portal_state_update(session: &mut ProjectionSession) -> u64 {
    session.portal_update_pending = true;
    let sequence = session.next_transcript_sequence;
    session.next_transcript_sequence += 1;
    sequence
}

fn promote_to_active_if_recovering(session: &mut ProjectionSession) {
    if matches!(
        session.lifecycle_state,
        ProjectionLifecycleState::Attached | ProjectionLifecycleState::HudUnavailable
    ) {
        session.lifecycle_state = ProjectionLifecycleState::Active;
    }
}

fn portal_update_allowed(
    session: &mut ProjectionSession,
    server_timestamp_wall_us: u64,
    max_portal_updates_per_second: u32,
) -> bool {
    if session.portal_rate_window_started_at_wall_us == 0
        || server_timestamp_wall_us
            >= session.portal_rate_window_started_at_wall_us + PORTAL_UPDATE_RATE_WINDOW_WALL_US
    {
        session.portal_rate_window_started_at_wall_us = server_timestamp_wall_us;
        session.portal_updates_in_window = 0;
    }
    if session.portal_updates_in_window < max_portal_updates_per_second {
        session.portal_updates_in_window += 1;
        true
    } else {
        false
    }
}

fn prune_retained_transcript(
    session: &mut ProjectionSession,
    max_retained_transcript_bytes: usize,
    max_visible_transcript_bytes: usize,
) {
    let mut visible_bytes = 0usize;
    let mut oldest_visible_sequence = None;
    for unit in session.retained_transcript.iter().rev() {
        let next_visible_bytes = visible_bytes.saturating_add(unit.byte_len());
        if next_visible_bytes > max_visible_transcript_bytes {
            break;
        }
        visible_bytes = next_visible_bytes;
        oldest_visible_sequence = Some(unit.sequence);
    }
    while session.retained_transcript_bytes > max_retained_transcript_bytes {
        let Some(front) = session.retained_transcript.front() else {
            session.retained_transcript_bytes = 0;
            break;
        };
        if oldest_visible_sequence.is_some_and(|sequence| front.sequence >= sequence)
            && session.retained_transcript.len() == 1
        {
            break;
        }
        let Some(pruned) = session.retained_transcript.pop_front() else {
            break;
        };
        session.retained_transcript_bytes = session
            .retained_transcript_bytes
            .saturating_sub(pruned.byte_len());
    }
}

fn visible_transcript_window(
    session: &ProjectionSession,
    max_visible_transcript_bytes: usize,
) -> Vec<TranscriptUnit> {
    let mut visible = Vec::new();
    let mut visible_bytes = 0usize;
    for unit in session.retained_transcript.iter().rev() {
        let unit_bytes = unit.byte_len();
        if visible_bytes + unit_bytes > max_visible_transcript_bytes {
            break;
        }
        visible_bytes += unit_bytes;
        visible.push(unit.clone());
    }
    visible.reverse();
    visible
}

fn capabilities_are_subset(requested: &[String], granted: &[String]) -> bool {
    requested
        .iter()
        .all(|capability| granted.iter().any(|granted| granted == capability))
}

fn remember_logical_unit(
    session: &mut ProjectionSession,
    logical_unit_id: &str,
    max_seen_logical_units: usize,
) -> bool {
    if session.seen_logical_units.contains(logical_unit_id) {
        return true;
    }
    session
        .seen_logical_units
        .insert(logical_unit_id.to_string());
    session
        .seen_logical_unit_order
        .push_back(logical_unit_id.to_string());
    while session.seen_logical_unit_order.len() > max_seen_logical_units {
        if let Some(evicted) = session.seen_logical_unit_order.pop_front() {
            session.seen_logical_units.remove(&evicted);
        }
    }
    false
}

fn requested_delivery_state(ack_state: InputAckState) -> InputDeliveryState {
    match ack_state {
        InputAckState::Handled => InputDeliveryState::Handled,
        InputAckState::Rejected => InputDeliveryState::Rejected,
        InputAckState::Deferred => InputDeliveryState::Deferred,
    }
}

fn terminal_ack_replay_response(
    terminal_state: InputDeliveryState,
    request: &AcknowledgeInputRequest,
    server_timestamp_wall_us: u64,
) -> ProjectionResponse {
    if terminal_state == requested_delivery_state(request.ack_state) {
        return ProjectionResponse::accepted(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            "terminal acknowledgement replay accepted idempotently",
        );
    }
    ProjectionResponse::denied(
        &request.envelope.request_id,
        &request.envelope.projection_id,
        server_timestamp_wall_us,
        ProjectionErrorCode::ProjectionStateConflict,
        "conflicting acknowledgement for terminal input",
    )
}

fn remember_terminal_input(
    session: &mut ProjectionSession,
    input_id: &str,
    delivery_state: InputDeliveryState,
    max_completed_input_tombstones: usize,
) {
    if !delivery_state.is_terminal() {
        return;
    }
    if session
        .completed_input_ack_states
        .insert(input_id.to_string(), delivery_state)
        .is_none()
    {
        session
            .completed_input_ack_order
            .push_back(input_id.to_string());
    }
    while session.completed_input_ack_order.len() > max_completed_input_tombstones {
        if let Some(evicted) = session.completed_input_ack_order.pop_front() {
            session.completed_input_ack_states.remove(&evicted);
        }
    }
}

fn prune_terminal_pending_input(
    session: &mut ProjectionSession,
    max_completed_input_tombstones: usize,
) {
    let mut retained = VecDeque::with_capacity(session.pending_input.len());
    while let Some(item) = session.pending_input.pop_front() {
        if item.delivery_state.is_terminal() {
            remember_terminal_input(
                session,
                &item.input_id,
                item.delivery_state,
                max_completed_input_tombstones,
            );
        } else {
            retained.push_back(item);
        }
    }
    session.pending_input = retained;
}

fn acknowledge_input(
    session: &mut ProjectionSession,
    request: &AcknowledgeInputRequest,
    server_timestamp_wall_us: u64,
) -> (ProjectionResponse, bool) {
    expire_pending(session, server_timestamp_wall_us);
    let Some(item) = session
        .pending_input
        .iter_mut()
        .find(|item| item.input_id == request.input_id)
    else {
        if let Some(terminal_state) = session.completed_input_ack_states.get(&request.input_id) {
            return (
                terminal_ack_replay_response(*terminal_state, request, server_timestamp_wall_us),
                false,
            );
        }
        return (
            ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                ProjectionErrorCode::ProjectionNotFound,
                "input_id not found",
            ),
            false,
        );
    };

    if request.ack_state != InputAckState::Deferred && request.not_before_wall_us.is_some() {
        return (
            ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                ProjectionErrorCode::ProjectionInvalidArgument,
                "not_before_wall_us is only valid for deferred acknowledgements",
            ),
            false,
        );
    }

    if let Some(not_before_wall_us) = request.not_before_wall_us {
        if not_before_wall_us >= item.expires_at_wall_us {
            return (
                ProjectionResponse::denied(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    ProjectionErrorCode::ProjectionInvalidArgument,
                    "not_before_wall_us must be before expires_at_wall_us",
                ),
                false,
            );
        }
    }

    if item.delivery_state.is_terminal() {
        return (
            terminal_ack_replay_response(item.delivery_state, request, server_timestamp_wall_us),
            false,
        );
    }

    match request.ack_state {
        InputAckState::Handled => {
            session.pending_input_bytes = session
                .pending_input_bytes
                .saturating_sub(item.submission_text.len());
            item.delivery_state = InputDeliveryState::Handled;
        }
        InputAckState::Rejected => {
            session.pending_input_bytes = session
                .pending_input_bytes
                .saturating_sub(item.submission_text.len());
            item.delivery_state = InputDeliveryState::Rejected;
        }
        InputAckState::Deferred => {
            if item.delivery_state != InputDeliveryState::Delivered {
                return (
                    ProjectionResponse::denied(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        ProjectionErrorCode::ProjectionStateConflict,
                        "only delivered input can be deferred",
                    ),
                    false,
                );
            }
            item.delivery_state = InputDeliveryState::Deferred;
            item.not_before_wall_us = request.not_before_wall_us;
        }
    }

    (
        ProjectionResponse::accepted(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            "acknowledgement accepted",
        ),
        true,
    )
}

fn expire_pending(session: &mut ProjectionSession, server_timestamp_wall_us: u64) {
    for item in &mut session.pending_input {
        if !item.delivery_state.is_terminal() && server_timestamp_wall_us >= item.expires_at_wall_us
        {
            session.pending_input_bytes = session
                .pending_input_bytes
                .saturating_sub(item.submission_text.len());
            item.delivery_state = InputDeliveryState::Expired;
        }
    }
}
