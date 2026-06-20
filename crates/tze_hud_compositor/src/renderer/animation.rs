//! Animation-state update methods for the compositor.
//!
//! Moved from `renderer/mod.rs` (the animation-update method cluster, approx.
//! L4288–5095 at plan date) by Step R-5 of the renderer module split
//! (hud-fgryk).  No logic was changed; only visibility modifiers were added
//! where Rust's module-privacy rules require them.
//!
//! Animation-state **types** (`ZoneAnimationState`, `PublicationAnimationState`,
//! `StreamRevealState`) remain in `draw_cmds.rs` (placed there by Step R-1).
//! This file contains only the `impl Compositor` methods that operate on those
//! types.

use std::collections::HashMap;

use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;

use super::Compositor;
use super::draw_cmds::{
    NOTIFICATION_DEFAULT_TTL_MS, NOTIFICATION_FADE_OUT_MS, PubKey, PublicationAnimationState,
    StreamRevealState, ZoneAnimationState,
};

impl Compositor {
    /// Update zone animation states before each frame.
    ///
    /// Starts fade-in animations for newly-published zones and fade-out
    /// animations for zones that just lost their last publish.
    ///
    /// Also handles zone unregistration: zones that were active and have since
    /// been removed from the registry are treated as cleared (no fade-out is
    /// possible since the zone_def is gone, so the state is simply pruned).
    ///
    /// Prunes completed transitions.
    pub fn update_zone_animations(&mut self, scene: &SceneGraph) {
        // Build current active-zone set (zone_name → has active publishes).
        let current_active: HashMap<String, bool> = scene
            .zone_registry
            .active_publishes
            .iter()
            .map(|(name, pubs)| (name.clone(), !pubs.is_empty()))
            .collect();

        // Detect publish transitions within currently-registered zones.
        for (zone_name, &is_active) in &current_active {
            let was_active = self
                .prev_active_zones
                .get(zone_name)
                .copied()
                .unwrap_or(false);

            if is_active && !was_active {
                // Zone just received its first publish — start fade-in.
                //
                // Transition interrupt semantics: if a fade-out is currently in
                // progress (target_opacity == 0.0), we MUST start the fade-in from
                // the current composite opacity rather than from 0 to prevent a
                // blank frame.  Per spec §Subtitle Contention Policy: "the fade-out
                // MUST be cancelled immediately and the new content MUST begin its
                // transition_in_ms fade-in from the current composite opacity (not
                // from zero)."
                if let Some(zone_def) = scene.zone_registry.zones.get(zone_name) {
                    if let Some(ms) = zone_def.rendering_policy.transition_in_ms {
                        if ms > 0 {
                            let new_state = if let Some(existing) =
                                self.zone_animation_states.get(zone_name)
                            {
                                if existing.target_opacity == 0.0 {
                                    // Interrupt active fade-out: begin fade-in from
                                    // current opacity so there is no blank frame.
                                    ZoneAnimationState::fade_in_from(ms, existing.current_opacity())
                                } else {
                                    ZoneAnimationState::fade_in(ms)
                                }
                            } else {
                                ZoneAnimationState::fade_in(ms)
                            };
                            self.zone_animation_states
                                .insert(zone_name.clone(), new_state);
                        }
                    }
                }
            } else if !is_active && was_active {
                // Zone just lost its last publish — start fade-out.
                if let Some(zone_def) = scene.zone_registry.zones.get(zone_name) {
                    if let Some(ms) = zone_def.rendering_policy.transition_out_ms {
                        if ms > 0 {
                            self.zone_animation_states
                                .insert(zone_name.clone(), ZoneAnimationState::fade_out(ms));
                        }
                    }
                }
            }
        }

        // Detect zone unregistration: zones that were previously tracked but
        // are now absent from active_publishes (zone was removed from registry).
        // Since zone_def is gone, no fade-out animation is possible; we simply
        // prune any in-flight animation state for that zone immediately.
        self.zone_animation_states
            .retain(|zone_name, _| current_active.contains_key(zone_name));

        // Prune completed transitions (reached target opacity).
        self.zone_animation_states
            .retain(|_, state| !state.is_complete());

        self.prev_active_zones = current_active;
    }

    /// Update per-portal-tile fade animation state (§6.3 — transition tokens).
    ///
    /// Runs the same appear/disappear transition logic as [`update_zone_animations`]
    /// but for portal tiles (scrollable tiles identified by a registered
    /// [`TileScrollConfig`]).  Durations are sourced from the
    /// `portal.transition.in_ms` and `portal.transition.out_ms` design tokens
    /// in `self.token_map`; no literal durations appear here.
    ///
    /// A tile is considered "has content" when `tile.root_node.is_some()`.
    ///
    /// - **Appear**: a scrollable tile whose `root_node` just became `Some`
    ///   starts a `fade_in(transition_in_ms)` animation (if `> 0`).
    /// - **Disappear**: a scrollable tile whose `root_node` just became `None`
    ///   starts a `fade_out(transition_out_ms)` animation (if `> 0`).
    ///
    /// Completed transitions are pruned after each update.
    ///
    /// Must be called once per frame alongside `update_zone_animations`.
    pub fn update_portal_tile_animations(&mut self, scene: &SceneGraph) {
        // Resolve transition durations from design tokens (§6.1 — no literals).
        let transition_in_ms: u32 = self
            .token_map
            .get("portal.transition.in_ms")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(120); // matches PortalPartTokens::defaults::TRANSITION_IN_MS
        let transition_out_ms: u32 = self
            .token_map
            .get("portal.transition.out_ms")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(80); // matches PortalPartTokens::defaults::TRANSITION_OUT_MS

        // Build current content state for all scrollable tiles.
        let current: HashMap<SceneId, bool> = scene
            .tiles
            .values()
            .filter(|tile| scene.tile_scroll_config(tile.id).is_some())
            .map(|tile| (tile.id, tile.root_node.is_some()))
            .collect();

        for (&tile_id, &has_content) in &current {
            let had_content = self
                .prev_portal_tile_has_content
                .get(&tile_id)
                .copied()
                .unwrap_or(false);

            if has_content && !had_content {
                // Tile just received content — start fade-in.
                // Interrupt semantics mirror zone animation: start from the
                // current opacity if a fade-out is in progress (no blank frame).
                //
                // Portal tiles are *displayed* through the EaseInOut curve (see
                // `portal_tile_anim_opacity`), so the interrupted fade-in MUST be
                // seeded from the eased (on-screen) opacity rather than the linear
                // `current_opacity()`. Seeding from the linear value would make the
                // new fade-in start at a different opacity than the frame just
                // rendered, producing a visible jump (hud-uir0w). Reusing
                // `portal_tile_anim_opacity` keeps the seed tied to the exact
                // displayed value, so the easing curve can never drift between the
                // two paths.
                if transition_in_ms > 0 {
                    let new_state =
                        if let Some(existing) = self.portal_tile_anim_states.get(&tile_id) {
                            if existing.target_opacity == 0.0 {
                                ZoneAnimationState::fade_in_from(
                                    transition_in_ms,
                                    self.portal_tile_anim_opacity(tile_id),
                                )
                            } else {
                                ZoneAnimationState::fade_in(transition_in_ms)
                            }
                        } else {
                            ZoneAnimationState::fade_in(transition_in_ms)
                        };
                    self.portal_tile_anim_states.insert(tile_id, new_state);
                }
            } else if !has_content && had_content {
                // Tile just lost content — start fade-out.
                if transition_out_ms > 0 {
                    self.portal_tile_anim_states
                        .insert(tile_id, ZoneAnimationState::fade_out(transition_out_ms));
                }
            }
        }

        // Prune animation states for tiles that are no longer scrollable.
        self.portal_tile_anim_states
            .retain(|tile_id, _| current.contains_key(tile_id));

        // Prune completed transitions.
        self.portal_tile_anim_states
            .retain(|_, state| !state.is_complete());

        self.prev_portal_tile_has_content = current;
    }

    /// Return the current portal tile animation opacity for `tile_id`.
    ///
    /// Returns `1.0` when no animation is in progress (fully visible).
    /// Used by tile text collection and background rendering to fade portal
    /// tiles in/out using the token-configured duration.
    ///
    /// The fade is shaped by [`Easing::EaseInOut`] (hud-bq0gl.10) so portal
    /// collapse/expand transitions accelerate and decelerate rather than ramping
    /// linearly. Endpoints (`t=0`, `t=1`) are unchanged, so this is a motion-only
    /// refinement: a freshly-appearing tile still starts fully transparent and a
    /// completed transition still rests at full opacity.
    #[inline]
    pub(crate) fn portal_tile_anim_opacity(&self, tile_id: SceneId) -> f32 {
        self.portal_tile_anim_states
            .get(&tile_id)
            .map(|s| s.current_opacity_eased(super::easing::Easing::EaseInOut))
            .unwrap_or(1.0)
    }

    /// Update per-zone streaming word-by-word reveal state.
    ///
    /// Must be called once per frame (after `update_zone_animations`).
    ///
    /// For each zone with a `LatestWins` or `Replace` publication that has
    /// non-empty breakpoints:
    /// - If no reveal state exists or the current pub key doesn't match, start
    ///   a fresh reveal from segment 0 (latest-wins cancels previous streaming).
    /// - If reveal state exists for the current pub key, advance by one frame.
    /// - Zones with empty breakpoints (or no StreamText) have their reveal state
    ///   pruned so text renders at full immediately.
    ///
    /// Per spec §Subtitle Streaming Word-by-Word Reveal:
    /// - Breakpoints identify byte offsets for progressive reveal.
    /// - Empty breakpoints → reveal all at once.
    /// - Replacement during streaming → cancel old reveal, start new.
    pub fn update_stream_reveals(&mut self, scene: &SceneGraph) {
        // Collect zones whose latest publish has breakpoints.
        let mut active_keys: HashMap<String, PubKey> = HashMap::new();

        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Only LatestWins/Replace zones get streaming reveal (single occupant).
            if !matches!(
                zone_def.contention_policy,
                ContentionPolicy::LatestWins | ContentionPolicy::Replace
            ) {
                continue;
            }
            let latest = &publishes[publishes.len() - 1];
            // Only StreamText with non-empty breakpoints gets progressive reveal.
            if !matches!(&latest.content, ZoneContent::StreamText(_))
                || latest.breakpoints.is_empty()
            {
                continue;
            }
            let pub_key: PubKey = (
                latest.published_at_wall_us,
                latest.publisher_namespace.clone(),
            );
            active_keys.insert(zone_name.clone(), pub_key);
        }

        // Prune reveal states for zones no longer streaming.
        self.stream_reveal_states
            .retain(|zone_name, _| active_keys.contains_key(zone_name));

        // Update or create reveal states.
        for (zone_name, pub_key) in &active_keys {
            let publishes = match scene.zone_registry.active_publishes.get(zone_name) {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };
            let latest = &publishes[publishes.len() - 1];

            let state = self.stream_reveal_states.get(zone_name);
            let need_reset = state.map(|s| &s.pub_key != pub_key).unwrap_or(true);

            if need_reset {
                // New publication (latest-wins replaced) or first reveal — start fresh.
                let new_state = StreamRevealState::new(pub_key.clone(), latest.breakpoints.clone());
                self.stream_reveal_states
                    .insert(zone_name.clone(), new_state);
            } else if let Some(state) = self.stream_reveal_states.get_mut(zone_name) {
                // Advance existing reveal by one frame.
                state.advance();
            }
        }
    }

    /// Update per-publication fade-out animation state for Stack zone publications.
    ///
    /// For each active publication in a Stack zone:
    ///
    /// 1. If it is new (not in `pub_animation_states`), insert a fresh
    ///    [`PublicationAnimationState`] using the effective TTL from
    ///    [`Compositor::publication_ttl_ms`]: `expires_at_wall_us` (urgency-derived)
    ///    takes highest priority, then `NotificationPayload.ttl_ms`, then the zone's
    ///    `auto_clear_ms`, then `NOTIFICATION_DEFAULT_TTL_MS` (8 000 ms).
    /// 2. Call `tick()` to check whether the TTL has expired and start the fade if so.
    ///
    /// Stale entries (publications no longer present in `active_publishes`) are
    /// pruned from `pub_animation_states` by this method.
    ///
    /// After this call, use [`Compositor::prune_faded_publications`] to remove
    /// publications whose fade-out has fully completed from the scene graph.
    ///
    /// Call order per frame: `update_zone_animations` → `update_publication_animations`
    /// → `prune_faded_publications(scene)` → render.
    pub fn update_publication_animations(&mut self, scene: &SceneGraph) {
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Only Stack zones get per-publication TTL fade-out.
            if !matches!(zone_def.contention_policy, ContentionPolicy::Stack { .. }) {
                continue;
            }
            let zone_auto_clear_ms = zone_def
                .auto_clear_ms
                .unwrap_or(NOTIFICATION_DEFAULT_TTL_MS);

            let zone_states = self
                .pub_animation_states
                .entry(zone_name.clone())
                .or_default();

            // Build the set of currently-active pub keys for this zone.
            let active_keys: std::collections::HashSet<PubKey> = publishes
                .iter()
                .map(|r| (r.published_at_wall_us, r.publisher_namespace.clone()))
                .collect();

            // Prune stale entries (publications removed from active_publishes).
            zone_states.retain(|k, _| active_keys.contains(k));

            // Ensure every active publication has an animation state; tick existing ones.
            for record in publishes {
                let ttl_ms = Self::publication_ttl_ms(record, zone_auto_clear_ms);
                let key: PubKey = (
                    record.published_at_wall_us,
                    record.publisher_namespace.clone(),
                );
                zone_states
                    .entry(key)
                    .or_insert_with(|| PublicationAnimationState::new(ttl_ms))
                    .tick();
            }
        }

        // Prune zones no longer present in active_publishes.
        self.pub_animation_states
            .retain(|zone_name, _| scene.zone_registry.active_publishes.contains_key(zone_name));
    }

    /// Determine the effective TTL (ms) for a single publication.
    ///
    /// `ttl_ms` is the delay **until the fade-out animation begins**; the fade
    /// itself then lasts `NOTIFICATION_FADE_OUT_MS` ms.  Total visible duration
    /// is therefore `ttl_ms + NOTIFICATION_FADE_OUT_MS`.
    ///
    /// Priority (highest to lowest):
    /// 1. `ZonePublishRecord.expires_at_wall_us` — urgency-derived absolute expiry
    ///    set by the publishing path.  TTL is derived so the fade-out **starts**
    ///    `NOTIFICATION_FADE_OUT_MS` before the drain deadline:
    ///    `((expires_at_wall_us - published_at_wall_us) / 1_000)
    ///        .saturating_sub(NOTIFICATION_FADE_OUT_MS as u64)`.
    ///    If `expires_at_wall_us <= published_at_wall_us` (already expired or
    ///    invalid), the TTL is `0` (immediate fade-out).
    ///    This ensures the visual fade-out completes before `drain_expired_zone_publications`
    ///    removes the record (e.g., ~14 850 ms TTL for a 15 s warning).
    /// 2. `NotificationPayload.ttl_ms` — per-notification override.
    /// 3. Zone `auto_clear_ms` fallback (supplied by the caller).
    pub(super) fn publication_ttl_ms(record: &ZonePublishRecord, zone_default_ttl_ms: u64) -> u64 {
        // Highest priority: absolute wall-clock expiry on the record.
        // Derive TTL so the fade starts NOTIFICATION_FADE_OUT_MS before the drain boundary.
        if let Some(exp_us) = record.expires_at_wall_us {
            let duration_ms = if exp_us > record.published_at_wall_us {
                (exp_us - record.published_at_wall_us) / 1_000
            } else {
                // Already expired or invalid: immediate fade-out.
                0
            };
            return duration_ms.saturating_sub(NOTIFICATION_FADE_OUT_MS as u64);
        }
        // Next: per-notification explicit TTL.
        if let ZoneContent::Notification(n) = &record.content {
            if let Some(ttl) = n.ttl_ms {
                return ttl;
            }
        }
        zone_default_ttl_ms
    }

    /// Look up the current opacity for a publication in `pub_animation_states`.
    ///
    /// Returns 1.0 if no animation state is found (publication is fully visible).
    pub(crate) fn pub_opacity(&self, zone_name: &str, record: &ZonePublishRecord) -> f32 {
        let key: PubKey = (
            record.published_at_wall_us,
            record.publisher_namespace.clone(),
        );
        self.pub_animation_states
            .get(zone_name)
            .and_then(|zone_states| zone_states.get(&key))
            .map(|s| s.current_opacity())
            .unwrap_or(1.0)
    }

    /// Remove publications from the scene whose fade-out animation has completed.
    ///
    /// This method MUST be called before rendering so that fully-faded publications
    /// are absent from `active_publishes` during the frame.  After removal,
    /// remaining notifications reflow naturally in the next frame (slot positions
    /// are recalculated from the updated `active_publishes` slice each frame).
    ///
    /// Intended call site: runtime frame loop, between scene commit and render,
    /// alongside `SceneGraph::drain_expired_zone_publications`.
    pub fn prune_faded_publications(&mut self, scene: &mut SceneGraph) {
        for (zone_name, zone_states) in &self.pub_animation_states {
            let publishes = match scene.zone_registry.active_publishes.get_mut(zone_name) {
                Some(p) => p,
                None => continue,
            };
            let before = publishes.len();
            publishes.retain(|record| {
                let key: PubKey = (
                    record.published_at_wall_us,
                    record.publisher_namespace.clone(),
                );
                !zone_states
                    .get(&key)
                    .map(|s| s.is_fade_complete())
                    .unwrap_or(false)
            });
            if publishes.len() < before {
                scene.version += 1;
            }
        }
        // Remove empty active_publishes entries.
        scene
            .zone_registry
            .active_publishes
            .retain(|_, v| !v.is_empty());
    }

    /// Advance per-portal-tile scroll smoothing toward the scene's authoritative
    /// scroll targets (smooth scroll / animated follow-tail, hud-bq0gl.10).
    ///
    /// Must be called once per frame, before any render pass reads a tile's
    /// displayed scroll offset via [`Compositor::display_tile_scroll_offset`].
    /// The shared frame body ([`Compositor::build_frame_vertices`]) drives it so
    /// all three render entry points advance the smoothers exactly once.
    ///
    /// No-op when [`scroll_smoothing_enabled`](Compositor::scroll_smoothing_enabled)
    /// is `false` (headless): those paths read the raw scene offset directly so
    /// deterministic golden tests are unaffected.
    ///
    /// A newly-observed tile starts *settled* on its current offset (no initial
    /// jump); only subsequent target changes — user scroll or follow-tail
    /// content appends — animate. User scroll stays authoritative (RFC 0013
    /// §3.2): only the visual catch-up is eased, never the target.
    pub fn update_scroll_smoothing(&mut self, scene: &SceneGraph) {
        if !self.scroll_smoothing_enabled {
            return;
        }

        let now = std::time::Instant::now();
        let dt_ms = self
            .last_scroll_smooth_at
            .map(|t| now.duration_since(t).as_secs_f32() * 1_000.0)
            .unwrap_or(0.0);
        self.last_scroll_smooth_at = Some(now);

        for tile in scene.tiles.values() {
            // Only portal (scrollable) tiles smooth their scroll offset.
            if scene.tile_scroll_config(tile.id).is_none() {
                continue;
            }
            let (target_x, target_y) = scene.tile_scroll_offset_local(tile.id);
            self.scroll_smoothers
                .entry(tile.id)
                .or_insert_with(|| super::easing::ScrollSmoother::new(target_x, target_y))
                .advance(target_x, target_y, dt_ms);
        }

        // Prune smoothers for tiles that are no longer scrollable / present.
        self.scroll_smoothers
            .retain(|id, _| scene.tile_scroll_config(*id).is_some());
    }

    /// Return the *displayed* (smoothed) scroll offset for a tile.
    ///
    /// When smoothing is enabled (windowed) and a smoother exists for the tile,
    /// returns the eased displayed offset; otherwise returns the scene's raw
    /// authoritative offset. The fallback keeps non-portal tiles and the
    /// headless path exact.
    #[inline]
    pub(crate) fn display_tile_scroll_offset(
        &self,
        scene: &SceneGraph,
        tile_id: SceneId,
    ) -> (f32, f32) {
        if self.scroll_smoothing_enabled {
            if let Some(smoother) = self.scroll_smoothers.get(&tile_id) {
                return smoother.displayed();
            }
        }
        scene.tile_scroll_offset_local(tile_id)
    }

    /// Publish the per-tile *displayed* (smoothed/lagged) scroll offsets into the
    /// scene so the live hit-test path maps pointer coordinates against the same
    /// offset the renderer drew with (hud-3lynp).
    ///
    /// Must be called once per frame, after [`update_scroll_smoothing`] has
    /// advanced the smoothers (so the published offsets reflect this frame's
    /// displayed state) and with `&mut SceneGraph` available.
    ///
    /// When smoothing is disabled (headless/snap) this clears any published
    /// overrides so hit-testing falls back to the authoritative offset and
    /// deterministic golden tests are unaffected (acceptance: behavior unchanged
    /// when no smoothing is active).
    ///
    /// [`update_scroll_smoothing`]: Compositor::update_scroll_smoothing
    pub(crate) fn publish_displayed_scroll_offsets(&self, scene: &mut SceneGraph) {
        if !self.scroll_smoothing_enabled {
            scene.clear_displayed_tile_scroll_offsets();
            return;
        }
        // Drop overrides for tiles that no longer smooth (no longer scrollable /
        // removed) so a stale displayed offset can never outlive its smoother.
        scene.retain_displayed_tile_scroll_offsets(|id| self.scroll_smoothers.contains_key(&id));
        for (tile_id, smoother) in &self.scroll_smoothers {
            let (x, y) = smoother.displayed();
            scene.set_displayed_tile_scroll_offset(*tile_id, x, y);
        }
    }
}
