//! Resource budget enforcement ladder for agent sessions.
//!
//! Implements the three-tier enforcement system from RFC 0002 §5:
//!   Normal → Warning (5s grace) → Throttle (30s grace) → Revoke
//!
//! Critical violations (OOM attempt, repeated invariant violations) bypass the
//! ladder and go directly to revocation.
//!
//! The frame-time guardian is also implemented here: if Stage 3–5 cumulative
//! time exceeds 3ms, the lowest-priority tiles are shed for that frame.
//!
//! # Authority Boundary
//!
//! This module is the **resource budget enforcement authority** for `tze_hud_runtime`.
//!
//! Responsibility split:
//!
//! - **This module (`BudgetEnforcer`)**: owns the per-agent enforcement state machine
//!   (Normal/Warning/Throttled/Revoked), the enforcement ladder tick, the
//!   frame-time guardian, and the per-mutation admission gate. This is stateful —
//!   it tracks resource counters over time and escalates/de-escalates accordingly.
//!
//! - **`tze_hud_policy::resource`**: a stateless pure evaluator. It receives a
//!   `ResourceContext` snapshot (populated from this module's state) and returns a
//!   `ResourceDecision`. It does NOT own any resource state or counters.
//!
//! - **`tze_hud_resource::budget::BudgetRegistry`**: owns decoded-byte accounting
//!   for uploaded resources (textures). This module tracks tile counts, update rate,
//!   and the enforcement ladder; `BudgetRegistry` tracks raw decoded byte totals.
//!   They are complementary, not competing.
//!
//! **Do not duplicate budget enforcement logic in `tze_hud_policy`.** Policy only
//! evaluates snapshots; it never advances the enforcement ladder.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use tze_hud_scene::types::{BudgetViolation, ResourceBudget, SceneId};
use tze_hud_telemetry::{
    BudgetTier, BudgetViolationEvent, BudgetViolationKind, FrameTimeShedEvent,
};

// ─── Grace periods ───────────────────────────────────────────────────────────

/// Time in Warning state before escalating to Throttle.
const WARNING_GRACE: Duration = Duration::from_secs(5);

/// Time in Throttle state before escalating to Revoke.
const THROTTLE_GRACE: Duration = Duration::from_secs(30);

/// Number of invariant violations before triggering critical revocation.
const CRITICAL_INVARIANT_VIOLATION_THRESHOLD: u32 = 10;

/// Frame-time guardian threshold: if stages 3–5 cumulative time exceeds this,
/// shed lowest-priority tiles.
const FRAME_GUARDIAN_STAGE5_THRESHOLD_US: u64 = 3_000; // 3ms

// ─── Budget state per agent ───────────────────────────────────────────────────

/// Runtime state of an agent's budget position.
#[derive(Clone, Debug)]
pub enum BudgetState {
    /// All limits respected.
    Normal,
    /// At least one limit is exceeded. If not resolved within WARNING_GRACE,
    /// the agent is throttled.
    Warning { first_exceeded: Instant },
    /// Budget exceeded and grace period elapsed. Updates are coalesced more
    /// aggressively and effective `max_update_rate_hz` is halved. If not
    /// resolved within THROTTLE_GRACE, the session is revoked.
    Throttled { throttled_since: Instant },
    /// Session has been revoked. This is a terminal state — the agent must
    /// be disconnected and its leases reclaimed.
    Revoked,
}

impl BudgetState {
    pub fn tier(&self) -> BudgetTier {
        match self {
            BudgetState::Normal => BudgetTier::Normal,
            BudgetState::Warning { .. } => BudgetTier::Warning,
            BudgetState::Throttled { .. } => BudgetTier::Throttled,
            BudgetState::Revoked => BudgetTier::Revoked,
        }
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, BudgetState::Revoked)
    }
}

// ─── Per-agent resource counters ─────────────────────────────────────────────

/// Per-agent resource state maintained by the compositor thread.
///
/// Updated on every mutation intake pass (Stage 3). All arithmetic is integer
/// to avoid floating-point non-determinism (§5.3).
#[derive(Clone, Debug)]
pub struct AgentResourceState {
    pub session_id: SceneId,
    pub namespace: String,

    // ── Cumulative counts ──────────────────────────────────────────────────
    pub tile_count: u32,
    pub texture_bytes_used: u64,
    pub node_count_total: u32,
    pub lease_count: u32,

    // ── Rate tracking (sliding window over last 1 second) ─────────────────
    /// Timestamps of recent mutation events used to compute the sliding-window Hz.
    update_timestamps: VecDeque<Instant>,

    // ── Invariant violation counter ───────────────────────────────────────
    pub invariant_violation_count: u32,

    // ── Budget enforcement state ───────────────────────────────────────────
    pub budget_state: BudgetState,

    // ── Assigned budget ────────────────────────────────────────────────────
    pub budget: ResourceBudget,

    // ── Hard caps (absolute maximums from RFC 0002 §4.3) ──────────────────
    /// Absolute hard maximum for texture memory. OOM attempt triggers critical revocation.
    pub hard_max_texture_bytes: u64,
}

impl AgentResourceState {
    pub fn new(session_id: SceneId, namespace: String, budget: ResourceBudget) -> Self {
        Self {
            session_id,
            namespace,
            tile_count: 0,
            texture_bytes_used: 0,
            node_count_total: 0,
            lease_count: 0,
            update_timestamps: VecDeque::new(),
            invariant_violation_count: 0,
            budget_state: BudgetState::Normal,
            hard_max_texture_bytes: 2 * 1024 * 1024 * 1024, // 2 GiB hard max
            budget,
        }
    }

    /// Record a mutation event (for Hz tracking).
    pub fn record_update(&mut self, now: Instant) {
        // Sliding window: drop entries older than 1 second
        let cutoff = now - Duration::from_secs(1);
        while self
            .update_timestamps
            .front()
            .map(|t| *t <= cutoff)
            .unwrap_or(false)
        {
            self.update_timestamps.pop_front();
        }
        self.update_timestamps.push_back(now);
    }

    /// Current update rate in Hz (count of events in the last 1 second).
    pub fn current_update_rate_hz(&self) -> f32 {
        self.update_timestamps.len() as f32
    }

    /// Report an invariant violation. Returns true if this is a critical count.
    pub fn record_invariant_violation(&mut self) -> bool {
        self.invariant_violation_count += 1;
        self.invariant_violation_count > CRITICAL_INVARIANT_VIOLATION_THRESHOLD
    }

    /// Effective `max_update_rate_hz` after throttle penalty (50% reduction when throttled).
    pub fn effective_max_update_rate_hz(&self) -> f32 {
        match &self.budget_state {
            BudgetState::Throttled { .. } => self.budget.max_update_rate_hz * 0.5,
            _ => self.budget.max_update_rate_hz,
        }
    }
}

// ─── Enforcement result ───────────────────────────────────────────────────────

/// Outcome of a budget check for a pending mutation batch.
#[derive(Debug)]
pub enum BudgetCheckOutcome {
    /// Mutation batch is within budget; proceed.
    Allow,
    /// Mutation batch would exceed budget; reject the entire batch.
    Reject(BudgetViolation),
    /// Session must be revoked immediately (critical violation).
    Revoke(BudgetViolation),
}

// ─── Telemetry sink trait ─────────────────────────────────────────────────────

/// Minimal sink for budget-related telemetry events.
/// Production use will wire this to the TelemetryCollector channel.
pub trait BudgetTelemetrySink: Send {
    fn emit_violation(&mut self, event: BudgetViolationEvent);
    fn emit_shed(&mut self, event: FrameTimeShedEvent);
}

/// A no-op telemetry sink useful for tests and benchmarks.
pub struct NoopTelemetrySink;

impl BudgetTelemetrySink for NoopTelemetrySink {
    fn emit_violation(&mut self, _event: BudgetViolationEvent) {}
    fn emit_shed(&mut self, _event: FrameTimeShedEvent) {}
}

/// A collecting telemetry sink that stores events in-memory (for tests).
#[derive(Default)]
pub struct CollectingTelemetrySink {
    pub violations: Vec<BudgetViolationEvent>,
    pub shed_events: Vec<FrameTimeShedEvent>,
}

impl BudgetTelemetrySink for CollectingTelemetrySink {
    fn emit_violation(&mut self, event: BudgetViolationEvent) {
        self.violations.push(event);
    }
    fn emit_shed(&mut self, event: FrameTimeShedEvent) {
        self.shed_events.push(event);
    }
}

// ─── BudgetEnforcer ───────────────────────────────────────────────────────────

/// Per-session resource budget enforcer.
///
/// Maintains a registry of all active agent sessions and their resource state.
/// Called by the compositor thread at Stage 3 (Mutation Intake) each frame.
///
/// Thread model: single-threaded — only the compositor thread calls this.
pub struct BudgetEnforcer {
    /// Per-agent resource state keyed by namespace.
    agents: HashMap<String, AgentResourceState>,
    /// Monotonically increasing frame counter (for telemetry events).
    frame_number: u64,
    /// Consecutive frames where the frame-time guardian had to shed tiles.
    consecutive_shed_frames: u32,
    /// Epoch reference for converting Instant to microseconds in telemetry events.
    epoch: Instant,
}

impl BudgetEnforcer {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            frame_number: 0,
            consecutive_shed_frames: 0,
            epoch: Instant::now(),
        }
    }

    /// Register a new agent session with the given resource budget.
    pub fn register_session(
        &mut self,
        session_id: SceneId,
        namespace: String,
        budget: ResourceBudget,
    ) {
        let state = AgentResourceState::new(session_id, namespace.clone(), budget);
        self.agents.insert(namespace, state);
    }

    /// Remove an agent session (on disconnect or revocation).
    pub fn remove_session(&mut self, namespace: &str) {
        self.agents.remove(namespace);
    }

    /// Get a reference to an agent's current resource state.
    pub fn agent_state(&self, namespace: &str) -> Option<&AgentResourceState> {
        self.agents.get(namespace)
    }

    /// Get a mutable reference to an agent's resource state.
    pub fn agent_state_mut(&mut self, namespace: &str) -> Option<&mut AgentResourceState> {
        self.agents.get_mut(namespace)
    }

    // ── Budget checking ────────────────────────────────────────────────────

    /// Check whether a proposed mutation batch from `namespace` is within budget.
    ///
    /// `delta_tiles`: change in tile count (can be negative for deletes).
    /// `delta_texture_bytes`: change in texture memory (can be negative).
    /// `delta_nodes_per_tile`: proposed node count for the affected tile.
    /// `now`: current monotonic timestamp.
    ///
    /// Returns `BudgetCheckOutcome::Allow` if the batch should proceed,
    /// `Reject` if it should be dropped, or `Revoke` if the session must end.
    pub fn check_mutation(
        &mut self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
        max_nodes_in_batch: u32,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> BudgetCheckOutcome {
        let state = match self.agents.get_mut(namespace) {
            Some(s) => s,
            None => return BudgetCheckOutcome::Allow, // unknown session — let upstream handle
        };

        // ── Critical: hard texture OOM check (bypasses ladder) ────────────
        if delta_texture_bytes > 0 {
            let proposed_bytes = state
                .texture_bytes_used
                .saturating_add(delta_texture_bytes as u64);
            if proposed_bytes > state.hard_max_texture_bytes {
                let violation = BudgetViolation::CriticalTextureOomAttempt {
                    requested_bytes: proposed_bytes,
                    hard_max_bytes: state.hard_max_texture_bytes,
                };
                Self::transition_to_revoked(state, &violation, now, sink, &self.epoch);
                return BudgetCheckOutcome::Revoke(violation);
            }
        }

        // ── Tile count check ──────────────────────────────────────────────
        if delta_tiles > 0 {
            let proposed_tiles = state.tile_count.saturating_add(delta_tiles as u32);
            if proposed_tiles > state.budget.max_tiles {
                let violation = BudgetViolation::TileCountExceeded {
                    current: proposed_tiles,
                    limit: state.budget.max_tiles,
                };
                return BudgetCheckOutcome::Reject(violation);
            }
        }

        // ── Texture memory check ───────────────────────────────────────────
        if delta_texture_bytes > 0 {
            let proposed_bytes = state
                .texture_bytes_used
                .saturating_add(delta_texture_bytes as u64);
            if proposed_bytes > state.budget.max_texture_bytes {
                let violation = BudgetViolation::TextureMemoryExceeded {
                    current_bytes: proposed_bytes,
                    limit_bytes: state.budget.max_texture_bytes,
                };
                return BudgetCheckOutcome::Reject(violation);
            }
        }

        // ── Update rate check (sliding window Hz) ─────────────────────────
        state.record_update(now);
        let current_hz = state.current_update_rate_hz();
        let effective_limit = state.effective_max_update_rate_hz();
        if current_hz > effective_limit {
            let violation = BudgetViolation::UpdateRateExceeded {
                current_hz,
                limit_hz: effective_limit,
            };
            return BudgetCheckOutcome::Reject(violation);
        }

        // ── Nodes-per-tile check ───────────────────────────────────────────
        if max_nodes_in_batch > state.budget.max_nodes_per_tile {
            let violation = BudgetViolation::NodeCountPerTileExceeded {
                tile_id_hint: "batch".to_string(),
                current: max_nodes_in_batch,
                limit: state.budget.max_nodes_per_tile,
            };
            return BudgetCheckOutcome::Reject(violation);
        }

        BudgetCheckOutcome::Allow
    }

    /// Apply a successful mutation delta to the agent's tracked counters.
    ///
    /// Call only after `check_mutation` returns `Allow`.
    pub fn apply_mutation_delta(
        &mut self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
    ) {
        if let Some(state) = self.agents.get_mut(namespace) {
            if delta_tiles >= 0 {
                state.tile_count = state.tile_count.saturating_add(delta_tiles as u32);
            } else {
                state.tile_count = state.tile_count.saturating_sub((-delta_tiles) as u32);
            }
            if delta_texture_bytes >= 0 {
                state.texture_bytes_used = state
                    .texture_bytes_used
                    .saturating_add(delta_texture_bytes as u64);
            } else {
                state.texture_bytes_used = state
                    .texture_bytes_used
                    .saturating_sub((-delta_texture_bytes) as u64);
            }
        }
    }

    // ── Enforcement ladder tick ────────────────────────────────────────────

    /// Advance the enforcement ladder for all agents.
    ///
    /// Call once per frame after mutation intake. Returns the list of namespaces
    /// that have been revoked this tick (the caller must tear down those sessions).
    pub fn tick(
        &mut self,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> Vec<String> {
        self.frame_number += 1;
        let mut revoked = Vec::new();

        for (namespace, state) in &mut self.agents {
            // Skip sessions that are already revoked.
            if state.budget_state.is_revoked() {
                revoked.push(namespace.clone());
                continue;
            }

            // Determine whether the agent is currently in violation.
            let currently_violated = Self::is_violated(state);

            match &state.budget_state.clone() {
                BudgetState::Normal => {
                    if currently_violated {
                        let violation_kind = Self::primary_violation_kind(state);
                        state.budget_state = BudgetState::Warning { first_exceeded: now };
                        sink.emit_violation(BudgetViolationEvent {
                            namespace: namespace.clone(),
                            new_tier: BudgetTier::Warning,
                            violation_kind,
                            timestamp_us: now
                                .duration_since(self.epoch)
                                .as_micros() as u64,
                            detail: format!(
                                "Agent '{}' has exceeded resource budget; 5s grace period before throttle",
                                namespace
                            ),
                        });
                    }
                }
                BudgetState::Warning { first_exceeded } => {
                    if !currently_violated {
                        // Resolved during grace period — back to Normal.
                        state.budget_state = BudgetState::Normal;
                    } else if now.duration_since(*first_exceeded) >= WARNING_GRACE {
                        // Grace period elapsed without resolution → Throttle.
                        let throttled_since = now;
                        state.budget_state = BudgetState::Throttled { throttled_since };
                        sink.emit_violation(BudgetViolationEvent {
                            namespace: namespace.clone(),
                            new_tier: BudgetTier::Throttled,
                            violation_kind: Self::primary_violation_kind(state),
                            timestamp_us: throttled_since
                                .duration_since(self.epoch)
                                .as_micros() as u64,
                            detail: format!(
                                "Agent '{}' throttled after 5s warning grace; update rate halved",
                                namespace
                            ),
                        });
                    }
                }
                BudgetState::Throttled { throttled_since } => {
                    if !currently_violated {
                        // Resolved — back to Normal.
                        state.budget_state = BudgetState::Normal;
                    } else if now.duration_since(*throttled_since) >= THROTTLE_GRACE {
                        // 30s throttle sustained without resolution → Revoke.
                        // Build the violation from the actual dimension that is still exceeded.
                        let violation = Self::primary_revocation_violation(state);
                        Self::transition_to_revoked(state, &violation, now, sink, &self.epoch);
                        revoked.push(namespace.clone());
                    }
                }
                BudgetState::Revoked => {
                    revoked.push(namespace.clone());
                }
            }
        }

        revoked.sort();
        revoked.dedup();
        revoked
    }

    // ── Frame-time guardian ────────────────────────────────────────────────

    /// Frame-time guardian: if stages 3–5 cumulative elapsed time (µs) exceeds
    /// the 3ms threshold, returns shed advice.
    ///
    /// `tile_priorities`: slice of `(namespace, tile_id, lease_priority, z_order)` tuples
    /// representing all tiles to be rendered this frame, in arbitrary order.
    /// `elapsed_us`: cumulative µs consumed by stages 3–5 so far this frame.
    ///
    /// Returns the list of tile IDs to skip for this frame (lowest-priority first),
    /// or an empty vec if no shedding is needed.
    pub fn frame_guardian_shed(
        &mut self,
        tile_priorities: &[(&str, SceneId, u32, u32)],
        elapsed_us: u64,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> Vec<SceneId> {
        if elapsed_us <= FRAME_GUARDIAN_STAGE5_THRESHOLD_US || tile_priorities.is_empty() {
            self.consecutive_shed_frames = 0;
            return Vec::new();
        }

        // Sort tiles by (lease_priority DESC, z_order ASC) — numerically higher
        // lease_priority = lower importance (0 = highest priority per RFC convention).
        // z_order is a secondary tiebreaker: lower z_order = lower importance → shed first.
        // We shed from the front of this sorted list (lowest importance) using .take().
        let mut sorted: Vec<(SceneId, u32, u32)> = tile_priorities
            .iter()
            .map(|(_, tile_id, lp, zo)| (*tile_id, *lp, *zo))
            .collect();

        // Sort ascending by importance: highest lease_priority first (lowest importance),
        // then lowest z_order as tiebreaker.
        sorted.sort_by(|a, b| {
            b.1.cmp(&a.1) // higher lease_priority number = lower importance → shed first
                .then(a.2.cmp(&b.2)) // lower z_order = lower importance → shed first
        });

        // Shed approximately 25% of tiles (at least 1).
        let shed_count = ((sorted.len() as f32 * 0.25).ceil() as usize).max(1);
        let shed_ids: Vec<SceneId> = sorted.iter().take(shed_count).map(|(id, _, _)| *id).collect();

        self.consecutive_shed_frames += 1;

        sink.emit_shed(FrameTimeShedEvent {
            frame_number: self.frame_number,
            tiles_shed: shed_count as u32,
            elapsed_us_at_stage5: elapsed_us,
            consecutive_shed_frames: self.consecutive_shed_frames,
        });

        shed_ids
    }

    /// Called when the frame completes without shedding.
    pub fn frame_no_shed(&mut self) {
        self.consecutive_shed_frames = 0;
    }

    /// Number of consecutive frames where the guardian shed tiles.
    pub fn consecutive_shed_frames(&self) -> u32 {
        self.consecutive_shed_frames
    }

    // ── Invariant violation reporting ──────────────────────────────────────

    /// Report an invariant violation for an agent. Critical violations trigger
    /// immediate revocation; returns true if the session was revoked.
    pub fn report_invariant_violation(
        &mut self,
        namespace: &str,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> bool {
        let state = match self.agents.get_mut(namespace) {
            Some(s) => s,
            None => return false,
        };
        let is_critical = state.record_invariant_violation();
        if is_critical {
            let violation = BudgetViolation::RepeatedInvariantViolations {
                count: state.invariant_violation_count,
            };
            Self::transition_to_revoked(state, &violation, now, sink, &self.epoch);
            return true;
        }
        false
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    /// Returns true if the agent is currently exceeding any budget dimension.
    ///
    /// Uses `effective_max_update_rate_hz()` so that a throttled agent is only
    /// considered resolved once it drops below the *halved* rate limit, not the
    /// original one — otherwise an agent could escape Throttled state while still
    /// violating the reduced limit.
    fn is_violated(state: &AgentResourceState) -> bool {
        state.tile_count > state.budget.max_tiles
            || state.texture_bytes_used > state.budget.max_texture_bytes
            || state.current_update_rate_hz() > state.effective_max_update_rate_hz()
    }

    /// Determine the primary violation kind for telemetry (first exceeded dimension).
    fn primary_violation_kind(state: &AgentResourceState) -> BudgetViolationKind {
        if state.tile_count > state.budget.max_tiles {
            BudgetViolationKind::TileCountExceeded
        } else if state.texture_bytes_used > state.budget.max_texture_bytes {
            BudgetViolationKind::TextureMemoryExceeded
        } else {
            BudgetViolationKind::UpdateRateExceeded
        }
    }

    /// Build a `BudgetViolation` reflecting the actual exceeded dimension for use
    /// in revocation events (prevents the Throttled→Revoke path from always
    /// emitting a misleading `TileCountExceeded` reason).
    fn primary_revocation_violation(state: &AgentResourceState) -> BudgetViolation {
        if state.tile_count > state.budget.max_tiles {
            BudgetViolation::TileCountExceeded {
                current: state.tile_count,
                limit: state.budget.max_tiles,
            }
        } else if state.texture_bytes_used > state.budget.max_texture_bytes {
            BudgetViolation::TextureMemoryExceeded {
                current_bytes: state.texture_bytes_used,
                limit_bytes: state.budget.max_texture_bytes,
            }
        } else {
            BudgetViolation::UpdateRateExceeded {
                current_hz: state.current_update_rate_hz(),
                limit_hz: state.budget.max_update_rate_hz,
            }
        }
    }

    fn transition_to_revoked(
        state: &mut AgentResourceState,
        violation: &BudgetViolation,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
        epoch: &Instant,
    ) {
        state.budget_state = BudgetState::Revoked;
        let kind = match violation {
            BudgetViolation::TileCountExceeded { .. } => BudgetViolationKind::TileCountExceeded,
            BudgetViolation::TextureMemoryExceeded { .. } => {
                BudgetViolationKind::TextureMemoryExceeded
            }
            BudgetViolation::UpdateRateExceeded { .. } => BudgetViolationKind::UpdateRateExceeded,
            BudgetViolation::NodeCountPerTileExceeded { .. } => {
                BudgetViolationKind::NodeCountPerTileExceeded
            }
            BudgetViolation::CriticalTextureOomAttempt { .. } => {
                BudgetViolationKind::CriticalTextureOomAttempt
            }
            BudgetViolation::RepeatedInvariantViolations { .. } => {
                BudgetViolationKind::RepeatedInvariantViolations
            }
        };
        sink.emit_violation(BudgetViolationEvent {
            namespace: state.namespace.clone(),
            new_tier: BudgetTier::Revoked,
            violation_kind: kind,
            timestamp_us: now.duration_since(*epoch).as_micros() as u64,
            detail: format!("Agent '{}' session revoked", state.namespace),
        });
    }
}

impl AgentResourceState {
    /// Helper: when entering Warning, use `now` as the entered-state instant.
    /// We just need this to avoid a borrow conflict in tick(); not stored.
    #[allow(dead_code)]
    fn budget_state_entered_instant(&self, now: Instant) -> Instant {
        match &self.budget_state {
            BudgetState::Warning { first_exceeded } => *first_exceeded,
            BudgetState::Throttled { throttled_since } => *throttled_since,
            _ => now,
        }
    }
}

impl Default for BudgetEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tze_hud_scene::types::ResourceBudget;

    fn make_enforcer() -> (BudgetEnforcer, CollectingTelemetrySink) {
        (BudgetEnforcer::new(), CollectingTelemetrySink::default())
    }

    fn tight_budget() -> ResourceBudget {
        ResourceBudget {
            max_tiles: 2,
            max_texture_bytes: 1024,
            max_update_rate_hz: 5.0,
            max_nodes_per_tile: 4,
        }
    }

    fn register(enforcer: &mut BudgetEnforcer, ns: &str) {
        enforcer.register_session(SceneId::new(), ns.to_string(), tight_budget());
    }

    // ─── Warning escalation ────────────────────────────────────────────────

    #[test]
    fn test_warning_on_tile_count_exceed() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-a");

        let state = enforcer.agent_state_mut("agent-a").unwrap();
        // Manually push tile count over budget
        state.tile_count = 3; // limit is 2

        enforcer.tick(Instant::now(), &mut sink);

        assert!(
            !sink.violations.is_empty(),
            "Expected a warning violation event"
        );
        assert_eq!(sink.violations[0].new_tier, BudgetTier::Warning);

        let state = enforcer.agent_state("agent-a").unwrap();
        assert!(matches!(state.budget_state, BudgetState::Warning { .. }));
    }

    #[test]
    fn test_warning_resolves_when_budget_drops() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-b");

        let state = enforcer.agent_state_mut("agent-b").unwrap();
        state.tile_count = 3; // over budget

        enforcer.tick(Instant::now(), &mut sink);
        assert!(matches!(
            enforcer.agent_state("agent-b").unwrap().budget_state,
            BudgetState::Warning { .. }
        ));

        // Fix the violation
        let state = enforcer.agent_state_mut("agent-b").unwrap();
        state.tile_count = 1; // back within budget

        enforcer.tick(Instant::now(), &mut sink);
        assert!(
            matches!(
                enforcer.agent_state("agent-b").unwrap().budget_state,
                BudgetState::Normal
            ),
            "Expected return to Normal after violation resolved"
        );
    }

    // ─── Throttle activation ───────────────────────────────────────────────

    #[test]
    fn test_throttle_after_warning_grace() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-c");

        let state = enforcer.agent_state_mut("agent-c").unwrap();
        state.tile_count = 3; // over budget

        // Simulate Warning state that is already past the grace period
        let past = Instant::now() - Duration::from_secs(6);
        state.budget_state = BudgetState::Warning { first_exceeded: past };

        enforcer.tick(Instant::now(), &mut sink);

        assert!(
            matches!(
                enforcer.agent_state("agent-c").unwrap().budget_state,
                BudgetState::Throttled { .. }
            ),
            "Expected escalation to Throttled after 5s grace"
        );

        let throttle_events: Vec<_> = sink
            .violations
            .iter()
            .filter(|e| e.new_tier == BudgetTier::Throttled)
            .collect();
        assert_eq!(throttle_events.len(), 1, "Expected exactly one Throttle event");
    }

    #[test]
    fn test_throttle_halves_effective_update_rate() {
        let (mut enforcer, _) = make_enforcer();
        register(&mut enforcer, "agent-d");

        let past = Instant::now() - Duration::from_secs(6);
        let state = enforcer.agent_state_mut("agent-d").unwrap();
        state.tile_count = 3;
        state.budget_state = BudgetState::Throttled {
            throttled_since: past,
        };

        let effective = state.effective_max_update_rate_hz();
        assert_eq!(
            effective,
            tight_budget().max_update_rate_hz * 0.5,
            "Throttled agent should have halved update rate"
        );
    }

    // ─── Revocation trigger ────────────────────────────────────────────────

    #[test]
    fn test_revoke_after_throttle_grace() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-e");

        let far_past = Instant::now() - Duration::from_secs(35);
        let state = enforcer.agent_state_mut("agent-e").unwrap();
        state.tile_count = 3;
        state.budget_state = BudgetState::Throttled {
            throttled_since: far_past,
        };

        let revoked = enforcer.tick(Instant::now(), &mut sink);

        assert!(
            revoked.contains(&"agent-e".to_string()),
            "Expected agent-e to be revoked"
        );
        assert!(
            matches!(
                enforcer.agent_state("agent-e").unwrap().budget_state,
                BudgetState::Revoked
            ),
            "Expected state to be Revoked"
        );

        let revoke_events: Vec<_> = sink
            .violations
            .iter()
            .filter(|e| e.new_tier == BudgetTier::Revoked)
            .collect();
        assert!(!revoke_events.is_empty(), "Expected a Revoked telemetry event");
    }

    #[test]
    fn test_critical_oom_triggers_immediate_revocation() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-f");

        // Make the agent try to allocate far beyond the hard max
        let outcome = enforcer.check_mutation(
            "agent-f",
            0,
            3 * 1024 * 1024 * 1024_i64, // 3 GiB > 2 GiB hard max
            0,
            Instant::now(),
            &mut sink,
        );

        assert!(
            matches!(outcome, BudgetCheckOutcome::Revoke(_)),
            "Expected Revoke outcome for OOM attempt"
        );

        let revoke_events: Vec<_> = sink
            .violations
            .iter()
            .filter(|e| e.new_tier == BudgetTier::Revoked)
            .collect();
        assert!(!revoke_events.is_empty(), "Expected revocation telemetry event");
    }

    #[test]
    fn test_invariant_violations_trigger_revocation() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-g");

        let mut revoked = false;
        for _ in 0..=CRITICAL_INVARIANT_VIOLATION_THRESHOLD {
            revoked = enforcer.report_invariant_violation("agent-g", Instant::now(), &mut sink);
        }

        assert!(revoked, "Expected revocation after too many invariant violations");
        assert!(
            matches!(
                enforcer.agent_state("agent-g").unwrap().budget_state,
                BudgetState::Revoked
            ),
            "Expected agent state to be Revoked"
        );
    }

    // ─── Frame-time guardian ───────────────────────────────────────────────

    #[test]
    fn test_frame_guardian_no_shed_under_threshold() {
        let (mut enforcer, mut sink) = make_enforcer();

        let tiles = vec![
            ("ns", SceneId::new(), 0u32, 1u32),
            ("ns", SceneId::new(), 1u32, 0u32),
        ];
        let refs: Vec<(&str, SceneId, u32, u32)> =
            tiles.iter().map(|(ns, id, lp, zo)| (*ns, *id, *lp, *zo)).collect();

        let shed = enforcer.frame_guardian_shed(&refs, 2_000, &mut sink); // 2ms < 3ms threshold
        assert!(shed.is_empty(), "Should not shed under threshold");
        assert!(sink.shed_events.is_empty());
    }

    #[test]
    fn test_frame_guardian_sheds_lowest_priority() {
        let (mut enforcer, mut sink) = make_enforcer();

        let id_high = SceneId::new();
        let id_low = SceneId::new();

        // id_high: lease_priority=0 (most important), z_order=10
        // id_low: lease_priority=5 (least important), z_order=1
        let tiles = vec![
            ("ns", id_high, 0u32, 10u32),
            ("ns", id_low, 5u32, 1u32),
        ];
        let refs: Vec<(&str, SceneId, u32, u32)> =
            tiles.iter().map(|(ns, id, lp, zo)| (*ns, *id, *lp, *zo)).collect();

        let shed = enforcer.frame_guardian_shed(&refs, 4_000, &mut sink); // 4ms > 3ms threshold

        assert!(!shed.is_empty(), "Should shed at least one tile");
        assert!(
            shed.contains(&id_low),
            "id_low (priority=5) should be shed first"
        );
        assert!(
            !shed.contains(&id_high),
            "id_high (priority=0) should not be shed"
        );
        assert_eq!(sink.shed_events.len(), 1);
        assert_eq!(sink.shed_events[0].tiles_shed, 1);
    }

    #[test]
    fn test_frame_guardian_consecutive_shed_tracking() {
        let (mut enforcer, mut sink) = make_enforcer();

        let tiles = vec![
            ("ns", SceneId::new(), 0u32, 1u32),
            ("ns", SceneId::new(), 1u32, 0u32),
            ("ns", SceneId::new(), 2u32, 0u32),
            ("ns", SceneId::new(), 3u32, 0u32),
        ];
        let refs: Vec<(&str, SceneId, u32, u32)> =
            tiles.iter().map(|(ns, id, lp, zo)| (*ns, *id, *lp, *zo)).collect();

        for _ in 0..3 {
            enforcer.frame_guardian_shed(&refs, 5_000, &mut sink);
        }

        assert_eq!(enforcer.consecutive_shed_frames(), 3);
        assert_eq!(sink.shed_events.len(), 3);
        assert_eq!(sink.shed_events[2].consecutive_shed_frames, 3);
    }

    #[test]
    fn test_frame_no_shed_resets_consecutive_count() {
        let (mut enforcer, mut sink) = make_enforcer();

        let tiles = vec![("ns", SceneId::new(), 0u32, 1u32)];
        let refs: Vec<(&str, SceneId, u32, u32)> =
            tiles.iter().map(|(ns, id, lp, zo)| (*ns, *id, *lp, *zo)).collect();

        enforcer.frame_guardian_shed(&refs, 5_000, &mut sink);
        assert_eq!(enforcer.consecutive_shed_frames(), 1);

        enforcer.frame_no_shed();
        assert_eq!(enforcer.consecutive_shed_frames(), 0);
    }

    // ─── check_mutation integration ────────────────────────────────────────

    #[test]
    fn test_check_mutation_allows_within_budget() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-h");

        let outcome = enforcer.check_mutation("agent-h", 1, 512, 2, Instant::now(), &mut sink);
        assert!(
            matches!(outcome, BudgetCheckOutcome::Allow),
            "Should allow within budget"
        );
    }

    #[test]
    fn test_check_mutation_rejects_over_tile_limit() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-i");

        // max_tiles = 2; requesting 3 new tiles
        let outcome = enforcer.check_mutation("agent-i", 3, 0, 0, Instant::now(), &mut sink);
        assert!(
            matches!(outcome, BudgetCheckOutcome::Reject(BudgetViolation::TileCountExceeded { .. })),
            "Should reject over tile limit"
        );
    }

    #[test]
    fn test_check_mutation_rejects_over_node_per_tile_limit() {
        let (mut enforcer, mut sink) = make_enforcer();
        register(&mut enforcer, "agent-j");

        // max_nodes_per_tile = 4; batch has 5 nodes
        let outcome = enforcer.check_mutation("agent-j", 0, 0, 5, Instant::now(), &mut sink);
        assert!(
            matches!(
                outcome,
                BudgetCheckOutcome::Reject(BudgetViolation::NodeCountPerTileExceeded { .. })
            ),
            "Should reject over nodes-per-tile limit"
        );
    }
}
