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
//!   `PortalTranscriptUpdate` is available here; a structured bucket should be
//!   added once hud-ttq97 lands.
//! - **hud-pkg2g** (head-trim notify_head_content_removed): wired — the drain
//!   loop detects head-trim via `visible_transcript_bytes` / content-height
//!   decrease and calls `notify_head_content_removed` on the `InputProcessor`
//!   so scrolled-back viewports stay stable (spec §3.3).

use std::collections::HashMap;

use tze_hud_config::{resolve_portal_tokens, tokens::DesignTokenMap};
use tze_hud_input::InputProcessor;
use tze_hud_projection::{
    ProjectedPortalPolicy, ProjectionAuthority, ProjectionBounds,
    resident_grpc::{
        ResidentGrpcPortalAdapter, ResidentGrpcPortalCommandKind, ResidentGrpcPortalConfig,
        portal_visual_tokens_from_part_tokens,
    },
};
use tze_hud_scene::{
    Capability, Rect, SceneGraph,
    types::{SceneId, TileScrollConfig},
};

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

/// Per-projection adapter state managed by the in-process driver.
struct DriveEntry {
    /// gRPC adapter that renders markdown and tracks tile state.
    adapter: ResidentGrpcPortalAdapter,
    /// Scene tile ID assigned to this portal, or `None` if not yet created.
    tile_scene_id: Option<SceneId>,
    /// Estimated total content height (px) from the last `RenderPortal` drain.
    ///
    /// Used to detect head-trim: when `new_content_height_px` on the next
    /// drain is less than this value while `visible_transcript_bytes` also
    /// decreased, the coalescer (64 KiB cap) or visible-window (16 KiB cap)
    /// trimmed head content.  `notify_head_content_removed` is called with the
    /// difference so a scrolled-back viewport stays stable (spec §3.3 / hud-pkg2g).
    prev_content_height_px: f32,
    /// Visible-transcript byte count from the last `RenderPortal` drain.
    ///
    /// A decrease in this value on the next drain is the observable signal that
    /// a head-trim occurred (either the 64 KiB coalescer cap in
    /// `PortalCadenceCoalescer::record_append` or the 16 KiB visible-window cap
    /// in `visible_transcript_window`).
    prev_visible_bytes: usize,
}

/// In-process state for the portal projection drive loop.
///
/// This is the runtime-side equivalent of `PortalDriveState` in the stdio
/// projection_authority binary. It holds one `ResidentGrpcPortalAdapter` per
/// attached projection session plus tile-to-scene mapping.
struct InProcessPortalDriveState {
    /// Per-projection drive entries keyed by `projection_id`.
    entries: HashMap<String, DriveEntry>,
    /// Current resolved design-token overrides (flat key → value strings).
    token_overrides: DesignTokenMap,
}

impl InProcessPortalDriveState {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
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
                prev_visible_bytes: 0,
            },
        );
    }

    fn detach(&mut self, projection_id: &str) {
        self.entries.remove(projection_id);
    }

    fn apply_token_map(&mut self, overrides: DesignTokenMap) {
        self.token_overrides = overrides;
        let tokens = self.resolve_visual_tokens();
        for entry in self.entries.values_mut() {
            entry.adapter.set_visual_tokens(tokens.clone());
        }
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
        }
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

    /// Apply a new design-token override map, propagating to all live adapters.
    pub fn apply_token_map(&mut self, overrides: DesignTokenMap) {
        self.drive.apply_token_map(overrides);
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
        let policy = ProjectedPortalPolicy::permit_all();

        loop {
            // Round-robin fairness oracle (tasks.md §5.1 / §5.4).
            let Some(proj_id) = self.authority.next_due_projection_id() else {
                break;
            };

            // Materialise the coalesced update for this portal.
            let update = match self.authority.take_due_portal_update(&proj_id, now_us) {
                Ok(Some(update)) => update,
                Ok(None) => break, // Rate-window not yet elapsed.
                Err(_) => {
                    // Projection not found or expired — clean up adapter.
                    self.drive.detach(&proj_id);
                    continue;
                }
            };

            // Build the full projected portal state for rendering.
            let Some(state) = self.authority.projected_portal_state(&proj_id, &policy) else {
                // Session was removed between take_due and state query (race).
                self.drive.detach(&proj_id);
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

                    // TODO(hud-ttq97): emit structured latency telemetry for
                    //   update.submitted_at_us → now_us (arrival→present latency).

                    // hud-pkg2g: detect head-trim and call notify_head_content_removed.
                    //
                    // A head-trim has occurred when visible_transcript_bytes decreased
                    // AND new_content_height_px is less than the previous value.
                    // Two trim sites surface this signal:
                    //   1. PortalCadenceCoalescer::record_append (64 KiB cap): drops
                    //      oldest bytes from the payload before storing the snapshot.
                    //   2. visible_transcript_window (16 KiB cap): slices the retained
                    //      transcript to the newest max_visible_transcript_bytes.
                    // In both cases the observable effect is a reduction in
                    // visible_transcript_bytes + a reduction in new_content_height_px.
                    //
                    // We adjust the scroll offset BEFORE calling notify_tile_content_appended
                    // so that the content-height fields inside ScrollTileState are up to
                    // date when notify_content_appended recomputes the follow-tail bound.
                    let prev_height = entry.prev_content_height_px;
                    let prev_bytes = entry.prev_visible_bytes;
                    if update.visible_transcript_bytes < prev_bytes
                        && new_content_height_px < prev_height
                    {
                        let removed_px = prev_height - new_content_height_px;
                        let trim_changed =
                            input_processor.notify_head_content_removed(tile_scene_id, removed_px);
                        tracing::debug!(
                            proj_id = %proj_id,
                            tile_id = ?tile_scene_id,
                            removed_px,
                            prev_bytes,
                            new_bytes = update.visible_transcript_bytes,
                            scroll_adjusted = trim_changed,
                            "portal drain: head-trim detected — notify_head_content_removed"
                        );
                    }
                    // Update per-portal tracking for the next drain cycle.
                    entry.prev_content_height_px = new_content_height_px;
                    entry.prev_visible_bytes = update.visible_transcript_bytes;

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
                }

                ResidentGrpcPortalCommandKind::ReleaseLease => {
                    // ReleaseLease: no content notification needed.
                    tracing::debug!(
                        proj_id = %proj_id,
                        "portal drain: ReleaseLease — no notify required"
                    );
                }
            }
        }
    }

    /// Ensure the driver has an active lease in the scene graph.
    ///
    /// Returns the existing lease if still valid (capabilities can be retrieved),
    /// or grants a new one.
    fn ensure_driver_lease(&mut self, scene: &mut SceneGraph) -> Option<SceneId> {
        if let Some(lease_id) = self.lease_id {
            // Check if still valid by attempting to read capabilities.
            if scene.lease_capabilities(&lease_id).is_some() {
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
    use tze_hud_input::InputProcessor;
    use tze_hud_projection::{
        AttachRequest, ContentClassification, OperationEnvelope, OutputKind,
        PORTAL_UPDATE_RATE_WINDOW_WALL_US, ProjectionBounds, ProjectionOperation, ProviderKind,
        PublishOutputRequest,
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
            vec![Capability::CreateTile, Capability::ModifyOwnTiles],
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
}
