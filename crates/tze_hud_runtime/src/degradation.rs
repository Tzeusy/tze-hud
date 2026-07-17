//! 6-level degradation state machine for the tze_hud runtime.
//!
//! Implements the degradation ladder from RFC 0002 §6 as specified in
//! runtime-kernel/spec.md:
//!
//! - Requirement: Degradation Ladder (line 220) — 6 levels from Normal to Emergency
//! - Requirement: Degradation Trigger (line 237) — p95 > 14ms over 10-frame window
//! - Requirement: Degradation Hysteresis (line 250) — p95 < 12ms over 30-frame window
//! - Requirement: Tile Shedding Order (line 263) — (lease_priority ASC, z_order DESC)
//!
//! ## Performance Characteristics
//!
//! - Frame-time sample recording: O(1) amortized (ring buffer push/pop)
//! - p95 evaluation: O(N) where N ≤ 30 (window size) — well within < 100µs budget
//! - Tile shedding sort: O(n log n) where n = tile count (< 1ms p99 for v1 limits)
//!
//! ## Threading Model
//!
//! `DegradationController` is single-threaded. Only the compositor thread calls
//! it, always from within the frame loop after the frame completes.

use std::collections::VecDeque;

use tze_hud_compositor::CompositorDegradationPolicy;
use tze_hud_protocol::proto::session::{
    DegradationLevel as ProtocolDegradationLevel, DegradationNotice,
};
use tze_hud_protocol::session::RuntimeDegradationLevel;
use tze_hud_scene::types::SceneId;
use tze_hud_telemetry::{DegradationDirection, DegradationEvent};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Number of frames in the trigger rolling window (≈166ms at 60fps).
const TRIGGER_WINDOW: usize = 10;

/// Number of frames in the recovery rolling window (≈500ms at 60fps).
const RECOVERY_WINDOW: usize = 30;

/// Trigger threshold: frame_time_p95 must exceed this to advance a level (µs).
const TRIGGER_THRESHOLD_US: u64 = 14_000; // 14ms

/// Recovery threshold: frame_time_p95 must be below this to recover a level (µs).
const RECOVERY_THRESHOLD_US: u64 = 12_000; // 12ms

/// Immutable cadence-derived thresholds and elapsed windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DegradationEnvelope {
    pub effective_fps: u32,
    pub period_us: u64,
    pub entry_threshold_us: u64,
    pub recovery_threshold_us: u64,
    pub entry_duration_us: u64,
    pub recovery_duration_us: u64,
    pub entry_min_samples: usize,
    pub recovery_min_samples: usize,
}

impl DegradationEnvelope {
    /// Derive the frozen runtime envelope from the validated effective cadence.
    pub fn from_effective_fps(effective_fps: u32) -> Option<Self> {
        if effective_fps == 0 {
            return None;
        }
        let period_us = 1_000_000_u64 / u64::from(effective_fps);
        if period_us == 0 {
            return None;
        }
        let ceil_ratio = |numerator: u64, denominator: u64| {
            period_us
                .checked_mul(numerator)
                .map(|value| value.div_ceil(denominator))
        };
        Some(Self {
            effective_fps,
            period_us,
            entry_threshold_us: ceil_ratio(21, 25)?.min(TRIGGER_THRESHOLD_US),
            recovery_threshold_us: ceil_ratio(18, 25)?.min(RECOVERY_THRESHOLD_US),
            entry_duration_us: period_us.checked_mul(TRIGGER_WINDOW as u64)?,
            recovery_duration_us: period_us.checked_mul(RECOVERY_WINDOW as u64)?,
            entry_min_samples: TRIGGER_WINDOW,
            recovery_min_samples: RECOVERY_WINDOW,
        })
    }
}

/// Resolve the immutable startup cadence from the configured target and a
/// monitor refresh reported in millihertz. Unknown refresh leaves the target
/// unchanged; a known refresh caps it. Millihertz is rounded to the nearest
/// whole presentation cadence because the runtime envelope is integer-Hz.
pub(crate) fn effective_degradation_fps(
    target_fps: u32,
    monitor_refresh_millihz: Option<u32>,
) -> u32 {
    let target_fps = target_fps.max(1);
    monitor_refresh_millihz.map_or(target_fps, |refresh_millihz| {
        let refresh_fps = refresh_millihz.saturating_add(500) / 1_000;
        target_fps.min(refresh_fps.max(1))
    })
}

// ─── Degradation Level ────────────────────────────────────────────────────────

/// The 6 degradation levels from RFC 0002 §6.2.
///
/// Levels are ordered: Normal (0) is best, Emergency (5) is worst.
/// The runtime advances one level at a time (trigger) and recovers one level
/// at a time (hysteresis). To go from Level 5 to Normal requires 5 successive
/// 30-frame clean windows (~2.5 seconds at 60fps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    /// Level 0 — Full quality rendering. No restrictions.
    Normal = 0,
    /// Level 1 — Coalesce: reduce outbound SceneEvent frequency for
    /// state-stream tiles by the configured coalesce ratio (default 2×).
    Coalesce = 1,
    /// Level 2 — ReduceTextureQuality: scale down textures whose linear
    /// dimensions exceed 512px by the configured scale factor (default 50%).
    ReduceTextureQuality = 2,
    /// Level 3 — DisableTransparency: force semi-transparent tiles to opaque;
    /// skip alpha-blend render passes entirely.
    DisableTransparency = 3,
    /// Level 4 — ShedTiles: remove lowest-priority tiles from the render pass
    /// (sorted by lease_priority ASC, z_order DESC). Tiles remain in the scene
    /// graph but are not rendered.
    ShedTiles = 4,
    /// Level 5 — Emergency: render only the chrome layer plus the single
    /// highest-priority tile. All other tiles are visually suppressed.
    Emergency = 5,
}

impl DegradationLevel {
    /// Return the level one step worse, clamped at Emergency.
    fn advance(self) -> Self {
        match self {
            Self::Normal => Self::Coalesce,
            Self::Coalesce => Self::ReduceTextureQuality,
            Self::ReduceTextureQuality => Self::DisableTransparency,
            Self::DisableTransparency => Self::ShedTiles,
            Self::ShedTiles => Self::Emergency,
            Self::Emergency => Self::Emergency,
        }
    }

    /// Return the level one step better, clamped at Normal.
    fn recover(self) -> Self {
        match self {
            Self::Normal => Self::Normal,
            Self::Coalesce => Self::Normal,
            Self::ReduceTextureQuality => Self::Coalesce,
            Self::DisableTransparency => Self::ReduceTextureQuality,
            Self::ShedTiles => Self::DisableTransparency,
            Self::Emergency => Self::ShedTiles,
        }
    }

    /// Whether this level is Normal.
    pub fn is_normal(self) -> bool {
        self == Self::Normal
    }

    /// Numeric representation matching the spec (0–5).
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for DegradationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Coalesce => write!(f, "Coalesce"),
            Self::ReduceTextureQuality => write!(f, "ReduceTextureQuality"),
            Self::DisableTransparency => write!(f, "DisableTransparency"),
            Self::ShedTiles => write!(f, "ShedTiles"),
            Self::Emergency => write!(f, "Emergency"),
        }
    }
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configurable parameters for the degradation controller.
///
/// All ratios/factors are validated defaults from spec Implementation Notes
/// (line 415–417). Operators may override via runtime configuration.
#[derive(Clone, Debug)]
pub struct DegradationConfig {
    /// Level 1: reduce outbound SceneEvent frequency by this divisor.
    /// Default: 2 (one notification per two frames).
    pub coalesce_ratio: u32,

    /// Level 2: textures with linear dimensions exceeding this value (pixels)
    /// are scaled down by `texture_scale_factor`.
    /// Default: 512.
    pub texture_quality_threshold_px: u32,

    /// Level 2: scale factor applied to large textures (as a fraction of 1.0).
    /// Default: 0.5 (50% reduction).
    pub texture_scale_factor: f32,
}

impl Default for DegradationConfig {
    fn default() -> Self {
        Self {
            coalesce_ratio: 2,
            texture_quality_threshold_px: 512,
            texture_scale_factor: 0.5,
        }
    }
}

// ─── Tile descriptor ──────────────────────────────────────────────────────────

/// A tile descriptor used for shedding decisions.
///
/// `lease_priority`: 0 = highest priority (never shed first).
/// `z_order`: higher value = painted on top = more important.
///
/// Shedding order: (lease_priority DESC numerically, z_order ASC) — i.e. we
/// shed the tile with the *largest* lease_priority number first, and within
/// the same priority class we shed the one with the *smallest* z_order first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileDescriptor {
    pub tile_id: SceneId,
    pub lease_priority: u32,
    pub z_order: u32,
}

// ─── Degradation Controller ───────────────────────────────────────────────────

/// Rolling-window degradation state machine.
///
/// Call [`DegradationController::record_frame`] once per frame after the frame
/// completes. The controller evaluates trigger and recovery conditions and
/// advances or recovers the degradation level as needed.
///
/// Query [`DegradationController::level`] to determine what restrictions should
/// be applied to the current frame.
pub struct DegradationController {
    /// Current degradation level.
    level: DegradationLevel,

    /// Ring buffer of recent frame times (µs), capacity = RECOVERY_WINDOW.
    ///
    /// We keep the longer window (30 frames) because it subsumes the shorter
    /// (10 frames). The p95 over the last N entries gives the rolling window.
    frame_times: VecDeque<(u64, u64)>,

    /// Number of consecutive 30-frame rolling windows where p95 < 12ms.
    /// Used for the full-recovery path from Level 5.
    clean_recovery_windows: u32,

    /// Configuration.
    config: DegradationConfig,

    /// Monotonically increasing frame counter for telemetry.
    frame_number: u64,

    /// Frozen startup thresholds and elapsed windows.
    envelope: DegradationEnvelope,

    /// Deterministic clock used by the compatibility `record_frame` API.
    virtual_now_us: u64,

    /// First instant at which the scheduler proved there was no render deadline.
    quiescent_since_us: Option<u64>,
}

impl DegradationController {
    /// Create a new controller starting at Normal.
    pub fn new(config: DegradationConfig) -> Self {
        Self::with_envelope(
            config,
            DegradationEnvelope::from_effective_fps(60).expect("60 Hz envelope is valid"),
        )
    }

    /// Create a controller with a cadence envelope frozen by startup resolution.
    pub fn with_envelope(config: DegradationConfig, envelope: DegradationEnvelope) -> Self {
        Self {
            level: DegradationLevel::Normal,
            frame_times: VecDeque::with_capacity(RECOVERY_WINDOW),
            clean_recovery_windows: 0,
            config,
            frame_number: 0,
            envelope,
            virtual_now_us: 0,
            quiescent_since_us: None,
        }
    }

    /// Create a controller with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(DegradationConfig::default())
    }

    /// The current degradation level.
    pub fn level(&self) -> DegradationLevel {
        self.level
    }

    /// The current configuration.
    pub fn config(&self) -> &DegradationConfig {
        &self.config
    }

    pub fn envelope(&self) -> DegradationEnvelope {
        self.envelope
    }

    /// Record a completed frame's time (in microseconds) and evaluate
    /// trigger / recovery conditions.
    ///
    /// Returns `Some(DegradationEvent)` if the level changed this frame, or
    /// `None` if the level is unchanged.
    ///
    /// This method MUST be called exactly once after every frame completes,
    /// so that the rolling windows advance correctly.
    ///
    /// ## Window semantics
    ///
    /// The controller maintains a single ring buffer of recent frame times
    /// (capacity = RECOVERY_WINDOW = 30). After any level change (advance OR
    /// recover) the buffer is cleared so that the next evaluation window
    /// starts with fresh observations. This prevents a single burst of high-
    /// latency frames from causing cascading multi-level advances in a single
    /// cycle, and ensures the recovery window only counts frames observed
    /// AFTER the most recent level change.
    ///
    /// Trigger evaluation uses a true rolling 10-frame window: checked every
    /// frame once at least 10 samples exist. Recovery evaluation uses a true
    /// rolling 30-frame window: checked every frame once at least 30 samples
    /// exist, matching the spec ("30-frame rolling window").
    pub fn record_frame(&mut self, frame_time_us: u64) -> Option<DegradationEvent> {
        self.virtual_now_us = self.virtual_now_us.saturating_add(self.envelope.period_us);
        self.record_frame_at(frame_time_us, self.virtual_now_us)
    }

    /// Record a successful active frame at an injected monotonic completion time.
    pub fn record_frame_at(
        &mut self,
        frame_time_us: u64,
        completed_at_us: u64,
    ) -> Option<DegradationEvent> {
        self.frame_number += 1;
        self.virtual_now_us = self.virtual_now_us.max(completed_at_us);
        self.quiescent_since_us = None;

        self.frame_times.push_back((completed_at_us, frame_time_us));
        let oldest = completed_at_us.saturating_sub(self.envelope.recovery_duration_us);
        while self.frame_times.front().is_some_and(|(at, _)| *at < oldest) {
            self.frame_times.pop_front();
        }

        let old_level = self.level;

        // ── Trigger: advance one level ────────────────────────────────────────
        //
        // Spec: trigger fires when p95 > 14ms over the rolling 10-frame window.
        // Evaluated on every frame once at least TRIGGER_WINDOW samples exist.
        let entry_samples = samples_for_window(
            &self.frame_times,
            completed_at_us,
            self.envelope.entry_duration_us,
        );
        if self.level < DegradationLevel::Emergency
            && entry_samples.len() >= self.envelope.entry_min_samples
            && window_coverage_us(
                &self.frame_times,
                completed_at_us,
                self.envelope.entry_duration_us,
                self.envelope.period_us,
            ) >= self.envelope.entry_duration_us
        {
            let p95_trigger = p95(&entry_samples);
            if p95_trigger > self.envelope.entry_threshold_us {
                self.level = self.level.advance();
                // Clear the ring buffer after a level change.
                // New observations start from scratch, preventing the same
                // high-latency frames from causing cascading advances or
                // polluting the recovery window.
                self.frame_times.clear();
                self.clean_recovery_windows = 0;
                return Some(DegradationEvent {
                    frame_number: self.frame_number,
                    previous_level: old_level.as_u8(),
                    new_level: self.level.as_u8(),
                    frame_time_p95_us: p95_trigger,
                    direction: DegradationDirection::Advance,
                    sample_count: entry_samples.len() as u32,
                    window_duration_us: self.envelope.entry_duration_us,
                    effective_cadence_hz: self.envelope.effective_fps,
                    entry_threshold_us: self.envelope.entry_threshold_us,
                    recovery_threshold_us: self.envelope.recovery_threshold_us,
                    recovery_source: tze_hud_telemetry::DegradationRecoverySource::ActiveFrames,
                });
            }
        }

        // ── Recovery: recover one level ───────────────────────────────────────
        //
        // Spec: recovery requires p95 < 12ms sustained over a rolling 30-frame
        // window (spec line 256). Evaluated on every frame once at least
        // RECOVERY_WINDOW samples exist. For Level 5, full recovery to Normal
        // requires 5 such successive clean windows (~2.5 seconds at 60fps).
        let recovery_samples = samples_for_window(
            &self.frame_times,
            completed_at_us,
            self.envelope.recovery_duration_us,
        );
        if self.level > DegradationLevel::Normal
            && recovery_samples.len() >= self.envelope.recovery_min_samples
            && window_coverage_us(
                &self.frame_times,
                completed_at_us,
                self.envelope.recovery_duration_us,
                self.envelope.period_us,
            ) >= self.envelope.recovery_duration_us
        {
            let p95_recovery = p95(&recovery_samples);

            if p95_recovery < self.envelope.recovery_threshold_us {
                self.clean_recovery_windows += 1;
                // Always recover one level per clean window.
                self.level = self.level.recover();
                // Clear buffer after level change so the next recovery window
                // starts with fresh post-transition observations.
                self.frame_times.clear();
                return Some(DegradationEvent {
                    frame_number: self.frame_number,
                    previous_level: old_level.as_u8(),
                    new_level: self.level.as_u8(),
                    frame_time_p95_us: p95_recovery,
                    direction: DegradationDirection::Recover,
                    sample_count: recovery_samples.len() as u32,
                    window_duration_us: self.envelope.recovery_duration_us,
                    effective_cadence_hz: self.envelope.effective_fps,
                    entry_threshold_us: self.envelope.entry_threshold_us,
                    recovery_threshold_us: self.envelope.recovery_threshold_us,
                    recovery_source: tze_hud_telemetry::DegradationRecoverySource::ActiveFrames,
                });
            } else {
                // Dirty window — reset clean window counter.
                // The ring buffer keeps sliding; no reset needed.
                self.clean_recovery_windows = 0;
            }
        }

        None
    }

    /// Report a scheduler tick whose canonical predicate proved true quiescence.
    pub fn record_quiescent_at(&mut self, now_us: u64) -> Option<DegradationEvent> {
        self.virtual_now_us = self.virtual_now_us.max(now_us);
        if self.level == DegradationLevel::Normal {
            self.quiescent_since_us = Some(now_us);
            return None;
        }
        let since = *self.quiescent_since_us.get_or_insert(now_us);
        if now_us.saturating_sub(since) < self.envelope.recovery_duration_us {
            return None;
        }
        let old_level = self.level;
        self.level = self.level.recover();
        self.quiescent_since_us = Some(now_us);
        self.frame_times.clear();
        Some(DegradationEvent {
            frame_number: self.frame_number,
            previous_level: old_level.as_u8(),
            new_level: self.level.as_u8(),
            frame_time_p95_us: 0,
            direction: DegradationDirection::Recover,
            sample_count: 0,
            window_duration_us: self.envelope.recovery_duration_us,
            effective_cadence_hz: self.envelope.effective_fps,
            entry_threshold_us: self.envelope.entry_threshold_us,
            recovery_threshold_us: self.envelope.recovery_threshold_us,
            recovery_source: tze_hud_telemetry::DegradationRecoverySource::Quiescent,
        })
    }

    /// Next monotonic instant at which quiescent recovery can advance one
    /// level. `None` means either normal operation or quiescence has not yet
    /// been observed by the scheduler.
    pub fn next_quiescent_recovery_at_us(&self) -> Option<u64> {
        (self.level != DegradationLevel::Normal)
            .then(|| {
                self.quiescent_since_us
                    .map(|since| since.saturating_add(self.envelope.recovery_duration_us))
            })
            .flatten()
    }

    /// Determine which tiles to suppress in the render pass at Level 4+ shedding.
    ///
    /// Returns the tile descriptors for tiles that should be excluded from the
    /// render pass. Callers can extract `.tile_id` from each returned descriptor.
    /// Tiles remain in the scene graph; they are simply not presented.
    ///
    /// At Level 4 (ShedTiles), returns the lowest-priority tiles to suppress.
    /// The spec preservation order is `(lease_priority ASC, z_order DESC)`:
    /// lower `lease_priority` values and higher `z_order` values are kept.
    /// Equivalently, tiles are shed in `(lease_priority DESC, z_order ASC)`
    /// order — highest `lease_priority` value is shed first; within the same
    /// priority class, lowest `z_order` is shed first.
    ///
    /// At Level 5 (Emergency), returns all tiles except the single tile with
    /// the lowest lease_priority value (and, as a tiebreaker, the highest
    /// z_order within that priority class).
    ///
    /// For all other levels, returns an empty vec.
    ///
    /// The chrome layer is always preserved (it is never in `tiles`).
    ///
    /// # Complexity
    ///
    /// O(n log n) where n = tile count. Satisfies the < 1ms p99 budget for
    /// v1 tile limits (max 64 tiles per agent × number of agents).
    pub fn shed_tiles<'a>(&self, tiles: &'a [TileDescriptor]) -> Vec<&'a TileDescriptor> {
        match self.level {
            DegradationLevel::Normal
            | DegradationLevel::Coalesce
            | DegradationLevel::ReduceTextureQuality
            | DegradationLevel::DisableTransparency => {
                // No shedding at levels 0–3.
                vec![]
            }
            DegradationLevel::ShedTiles => {
                // Sort tiles by shedding priority: shed highest-lease_priority first.
                // Within a priority class, shed lowest z_order first.
                //
                // Result ordering (front = shed first):
                //   sort key: (lease_priority DESC, z_order ASC)
                let mut sorted: Vec<&TileDescriptor> = tiles.iter().collect();
                sorted.sort_by(|a, b| {
                    // Higher lease_priority number = lower importance = shed first.
                    b.lease_priority
                        .cmp(&a.lease_priority)
                        .then(a.z_order.cmp(&b.z_order))
                });
                // At Level 4, shed the lowest-priority quartile (at least 1).
                // For v1, we shed 25% consistent with the frame-time guardian.
                // Integer ceiling: ceil(len * 0.25) == (len + 3) / 4.
                let shed_count = tiles.len().div_ceil(4).max(1);
                sorted.into_iter().take(shed_count).collect()
            }
            DegradationLevel::Emergency => {
                // Keep only the single highest-priority tile (lease_priority=0
                // and highest z_order within that class). Suppress everything else.
                if tiles.is_empty() {
                    return vec![];
                }
                // Find the "best" tile: smallest lease_priority, then largest z_order.
                let keep = tiles
                    .iter()
                    .min_by(|a, b| {
                        a.lease_priority
                            .cmp(&b.lease_priority)
                            .then(b.z_order.cmp(&a.z_order)) // higher z_order wins
                    })
                    .expect("non-empty slice always has a min");

                // Everything except `keep` is shed.
                tiles.iter().filter(|t| t.tile_id != keep.tile_id).collect()
            }
        }
    }

    /// Compute the Level 1 coalesce: whether a notification should be sent for
    /// the current frame.
    ///
    /// Returns `true` if a state-stream notification should be emitted this
    /// frame (i.e., this frame is "on beat"). Returns `false` if the notification
    /// should be suppressed.
    ///
    /// At Level 0, always returns `true`. At Level 1+, returns `true` every
    /// `coalesce_ratio` frames.
    pub fn should_emit_state_stream(&self) -> bool {
        if self.level < DegradationLevel::Coalesce {
            return true;
        }
        let ratio = self.config.coalesce_ratio.max(1) as u64;
        self.frame_number.is_multiple_of(ratio)
    }

    /// Whether textures exceeding the threshold should be downscaled.
    ///
    /// Returns `true` at Level 2+.
    pub fn should_reduce_texture_quality(&self) -> bool {
        self.level >= DegradationLevel::ReduceTextureQuality
    }

    /// Whether alpha-blending should be disabled (all semi-transparent tiles
    /// forced to opaque).
    ///
    /// Returns `true` at Level 3+.
    pub fn should_disable_transparency(&self) -> bool {
        self.level >= DegradationLevel::DisableTransparency
    }

    /// The texture scale factor to apply when [`should_reduce_texture_quality`]
    /// is true. Dimensions exceeding `texture_quality_threshold_px` are scaled
    /// by this factor.
    pub fn texture_scale_factor(&self) -> f32 {
        self.config.texture_scale_factor
    }

    /// The texture dimension threshold (pixels) above which scaling is applied.
    pub fn texture_quality_threshold_px(&self) -> u32 {
        self.config.texture_quality_threshold_px
    }

    /// Number of consecutive frames evaluated so far (for testing / telemetry).
    pub fn frame_number(&self) -> u64 {
        self.frame_number
    }

    /// Number of successive clean 30-frame recovery windows counted since the
    /// last advance. Useful for telemetry and testing.
    pub fn clean_recovery_windows(&self) -> u32 {
        self.clean_recovery_windows
    }

    /// Build the complete compositor policy from inputs snapshotted under the
    /// scene lock for the frame being built.
    pub fn compositor_policy(&self, tiles: &[TileDescriptor]) -> CompositorDegradationPolicy {
        let level = match self.level {
            DegradationLevel::Normal => tze_hud_scene::DegradationLevel::Nominal,
            DegradationLevel::Coalesce => tze_hud_scene::DegradationLevel::Minor,
            DegradationLevel::ReduceTextureQuality => tze_hud_scene::DegradationLevel::Moderate,
            DegradationLevel::DisableTransparency => tze_hud_scene::DegradationLevel::Significant,
            DegradationLevel::ShedTiles => tze_hud_scene::DegradationLevel::ShedTiles,
            DegradationLevel::Emergency => tze_hud_scene::DegradationLevel::Emergency,
        };
        CompositorDegradationPolicy {
            level,
            suppressed_tiles: self
                .shed_tiles(tiles)
                .into_iter()
                .map(|tile| tile.tile_id)
                .collect(),
            texture_quality_threshold_px: self.config.texture_quality_threshold_px,
            texture_scale_factor: self.config.texture_scale_factor,
        }
    }

    /// Exhaustive append-only runtime-to-wire mapping.
    pub fn protocol_level(&self) -> (RuntimeDegradationLevel, ProtocolDegradationLevel) {
        match self.level {
            DegradationLevel::Normal => (
                RuntimeDegradationLevel::Normal,
                ProtocolDegradationLevel::Normal,
            ),
            DegradationLevel::Coalesce => (
                RuntimeDegradationLevel::CoalescingMore,
                ProtocolDegradationLevel::CoalescingMore,
            ),
            DegradationLevel::ReduceTextureQuality => (
                RuntimeDegradationLevel::TextureQualityReduced,
                ProtocolDegradationLevel::TextureQualityReduced,
            ),
            DegradationLevel::DisableTransparency => (
                RuntimeDegradationLevel::RenderingSimplified,
                ProtocolDegradationLevel::RenderingSimplified,
            ),
            DegradationLevel::ShedTiles => (
                RuntimeDegradationLevel::SheddingTiles,
                ProtocolDegradationLevel::SheddingTiles,
            ),
            DegradationLevel::Emergency => (
                RuntimeDegradationLevel::EmergencyRendering,
                ProtocolDegradationLevel::EmergencyRendering,
            ),
        }
    }

    pub fn protocol_notice(&self, timestamp_wall_us: u64) -> DegradationNotice {
        let (_, level) = self.protocol_level();
        let affected_capabilities = match self.level {
            DegradationLevel::Coalesce => vec!["state_stream".to_string()],
            DegradationLevel::Normal
            | DegradationLevel::ReduceTextureQuality
            | DegradationLevel::DisableTransparency
            | DegradationLevel::ShedTiles
            | DegradationLevel::Emergency => Vec::new(),
        };
        DegradationNotice {
            level: level as i32,
            reason: format!("runtime degradation level changed to {}", self.level),
            affected_capabilities,
            timestamp_wall_us,
        }
    }
}

// ─── p95 helper ───────────────────────────────────────────────────────────────

/// Compute the p95 of the last `n` values in a ring buffer (VecDeque).
///
/// Panics if `n > deque.len()` — callers must guard with `len() >= n`.
///
/// Uses the nearest-rank method (consistent with [`LatencyBucket::percentile`]).
#[cfg(test)]
fn p95_of_last_n(deque: &VecDeque<(u64, u64)>, n: usize) -> u64 {
    debug_assert!(deque.len() >= n, "caller must ensure len() >= n");
    let samples: Vec<u64> = deque
        .iter()
        .rev()
        .take(n)
        .map(|(_, value)| *value)
        .collect();
    p95(&samples)
}

fn samples_for_window(deque: &VecDeque<(u64, u64)>, now_us: u64, duration_us: u64) -> Vec<u64> {
    let oldest = now_us.saturating_sub(duration_us);
    deque
        .iter()
        .filter(|(at, _)| *at >= oldest && *at <= now_us)
        .map(|(_, value)| *value)
        .collect()
}

fn window_coverage_us(
    deque: &VecDeque<(u64, u64)>,
    now_us: u64,
    duration_us: u64,
    period_us: u64,
) -> u64 {
    let oldest = now_us.saturating_sub(duration_us);
    deque
        .iter()
        .find(|(at, _)| *at >= oldest && *at <= now_us)
        .map_or(0, |(first_at, _)| {
            now_us.saturating_sub(*first_at).saturating_add(period_us)
        })
}

fn p95(samples: &[u64]) -> u64 {
    debug_assert!(!samples.is_empty());
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    let rank = (95 * samples.len()).div_ceil(100);
    let idx = rank.saturating_sub(1).min(samples.len() - 1);
    samples[idx]
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::SceneId;

    fn controller() -> DegradationController {
        DegradationController::with_defaults()
    }

    #[test]
    fn cadence_envelope_preserves_60hz_calibration_and_tightens_faster_periods() {
        let sixty = DegradationEnvelope::from_effective_fps(60).expect("valid cadence");
        assert_eq!(sixty.entry_threshold_us, 14_000);
        assert_eq!(sixty.recovery_threshold_us, 12_000);
        assert_eq!(sixty.entry_min_samples, 10);
        assert_eq!(sixty.recovery_min_samples, 30);

        let faster = DegradationEnvelope::from_effective_fps(75).expect("valid cadence");
        assert!(faster.entry_threshold_us < sixty.entry_threshold_us);
        assert!(faster.recovery_threshold_us < sixty.recovery_threshold_us);
    }

    #[test]
    fn effective_cadence_is_capped_only_when_monitor_refresh_is_known() {
        assert_eq!(effective_degradation_fps(120, None), 120);
        assert_eq!(effective_degradation_fps(120, Some(60_000)), 60);
        assert_eq!(effective_degradation_fps(120, Some(59_940)), 60);
        assert_eq!(effective_degradation_fps(30, Some(60_000)), 30);
    }

    #[test]
    fn elapsed_window_blocks_burst_samples_and_quiescence_recovers_without_frames() {
        let envelope = DegradationEnvelope::from_effective_fps(60).expect("valid cadence");
        let mut ctrl = DegradationController::with_envelope(DegradationConfig::default(), envelope);
        for i in 0..10 {
            assert!(ctrl.record_frame_at(20_000, 1_000 + i).is_none());
        }
        assert_eq!(ctrl.level(), DegradationLevel::Normal);

        let mut now = 0;
        for _ in 0..10 {
            now += envelope.period_us;
            let _ = ctrl.record_frame_at(20_000, now);
        }
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
        assert!(ctrl.record_quiescent_at(now).is_none());
        assert_eq!(
            ctrl.next_quiescent_recovery_at_us(),
            Some(now + envelope.recovery_duration_us),
            "quiescent recovery must expose its monotonic wake deadline"
        );
        assert!(
            ctrl.record_quiescent_at(now + envelope.recovery_duration_us - 1)
                .is_none()
        );
        let recovered = ctrl
            .record_quiescent_at(now + envelope.recovery_duration_us)
            .expect("one quiescent recovery step");
        assert_eq!(recovered.new_level, DegradationLevel::Normal.as_u8());
        assert_eq!(ctrl.next_quiescent_recovery_at_us(), None);
    }

    #[test]
    fn production_degradation_sustained_payload_emits_machine_readable_deadline_evidence() {
        let started = std::time::Instant::now();
        let envelope = DegradationEnvelope::from_effective_fps(60).expect("valid cadence");
        let mut ctrl = DegradationController::with_envelope(DegradationConfig::default(), envelope);
        let mut selected_at_us = 0;
        let mut transition = None;

        for sample_index in 1..=envelope.entry_min_samples {
            selected_at_us = sample_index as u64 * envelope.period_us;
            transition = ctrl.record_frame_at(envelope.entry_threshold_us + 1, selected_at_us);
        }

        let transition = transition.expect("sustained over-budget load must select Level 1");
        assert!(
            selected_at_us <= envelope.entry_duration_us,
            "transition must be selected within the cadence-derived deadline"
        );
        assert_eq!(transition.previous_level, DegradationLevel::Normal.as_u8());
        assert_eq!(transition.new_level, DegradationLevel::Coalesce.as_u8());

        println!(
            "{}",
            serde_json::json!({
                "artifact": "production_degradation_sustained_payload",
                "status": "pass",
                "effective_cadence_hz": envelope.effective_fps,
                "entry_threshold_us": envelope.entry_threshold_us,
                "entry_deadline_us": envelope.entry_duration_us,
                "selected_at_us": selected_at_us,
                "sample_count": transition.sample_count,
                "from_level": transition.previous_level,
                "to_level": transition.new_level,
                "validation_wall_time_us": started.elapsed().as_micros() as u64,
            })
        );
    }

    /// Push `n` frames of `frame_time_us` through the controller without
    /// caring about transition events.
    fn push_frames(ctrl: &mut DegradationController, frame_time_us: u64, n: usize) {
        for _ in 0..n {
            ctrl.record_frame(frame_time_us);
        }
    }

    // ── Level invariants ──────────────────────────────────────────────────────

    #[test]
    fn test_starts_at_normal() {
        let ctrl = controller();
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_level_advance_chain() {
        assert_eq!(
            DegradationLevel::Normal.advance(),
            DegradationLevel::Coalesce
        );
        assert_eq!(
            DegradationLevel::Coalesce.advance(),
            DegradationLevel::ReduceTextureQuality
        );
        assert_eq!(
            DegradationLevel::ReduceTextureQuality.advance(),
            DegradationLevel::DisableTransparency
        );
        assert_eq!(
            DegradationLevel::DisableTransparency.advance(),
            DegradationLevel::ShedTiles
        );
        assert_eq!(
            DegradationLevel::ShedTiles.advance(),
            DegradationLevel::Emergency
        );
        assert_eq!(
            DegradationLevel::Emergency.advance(),
            DegradationLevel::Emergency
        );
    }

    #[test]
    fn test_level_recover_chain() {
        assert_eq!(DegradationLevel::Normal.recover(), DegradationLevel::Normal);
        assert_eq!(
            DegradationLevel::Coalesce.recover(),
            DegradationLevel::Normal
        );
        assert_eq!(
            DegradationLevel::ReduceTextureQuality.recover(),
            DegradationLevel::Coalesce
        );
        assert_eq!(
            DegradationLevel::DisableTransparency.recover(),
            DegradationLevel::ReduceTextureQuality
        );
        assert_eq!(
            DegradationLevel::ShedTiles.recover(),
            DegradationLevel::DisableTransparency
        );
        assert_eq!(
            DegradationLevel::Emergency.recover(),
            DegradationLevel::ShedTiles
        );
    }

    // ── Trigger: sustained overbudget → advance ───────────────────────────────

    #[test]
    fn test_trigger_advances_level_after_10_frames_over_14ms() {
        let mut ctrl = controller();
        // 10 frames all at 20ms — p95 = 20ms > 14ms → must advance.
        let event = push_and_get_last_event(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert!(event.is_some(), "Expected a degradation advance event");
        let ev = event.unwrap();
        assert_eq!(ev.previous_level, 0); // Normal
        assert_eq!(ev.new_level, 1); // Coalesce
        assert_eq!(ev.direction, DegradationDirection::Advance);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    #[test]
    fn test_trigger_requires_full_10_frame_window() {
        let mut ctrl = controller();
        // Only 9 frames — must NOT trigger yet.
        for _ in 0..(TRIGGER_WINDOW - 1) {
            let ev = ctrl.record_frame(20_000);
            assert!(
                ev.is_none(),
                "Should not trigger before full 10-frame window"
            );
        }
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
        // 10th frame — now should trigger.
        let ev = ctrl.record_frame(20_000);
        assert!(ev.is_some());
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    // ── Transient spike tolerance ─────────────────────────────────────────────

    #[test]
    fn test_partial_window_does_not_trigger_degradation() {
        // Spec: trigger requires p95 > 14ms over the 10-frame ROLLING WINDOW.
        // Before accumulating 10 frames, the system MUST NOT trigger,
        // regardless of how large individual frames are.
        let mut ctrl = controller();
        // Push 9 frames well above the threshold — but no trigger yet (window not full).
        for i in 0..9 {
            let ev = ctrl.record_frame(50_000);
            assert!(
                ev.is_none(),
                "Frame {i}: must not trigger before 10-frame window is full"
            );
        }
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_10th_frame_first_possible_trigger() {
        let mut ctrl = controller();
        push_frames(&mut ctrl, 50_000, 9);
        // The 10th frame is the FIRST point at which the trigger can fire.
        let ev = ctrl.record_frame(50_000);
        assert!(
            ev.is_some(),
            "10th frame above threshold should trigger degradation"
        );
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    #[test]
    fn test_p95_boundary_does_not_trigger_at_exactly_14ms() {
        let mut ctrl = controller();
        // All 10 frames at exactly 14ms — p95 = 14ms, NOT > 14ms. No trigger.
        push_frames(&mut ctrl, TRIGGER_THRESHOLD_US, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    // ── Hysteresis / recovery ─────────────────────────────────────────────────

    #[test]
    fn test_recovery_requires_30_frames_under_12ms() {
        let mut ctrl = controller();
        // Force to Level 1 by triggering.
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // 29 clean frames — must NOT recover yet.
        for i in 0..(RECOVERY_WINDOW - 1) {
            let ev = ctrl.record_frame(5_000);
            assert!(
                ev.is_none(),
                "Frame {i}: should not recover before 30 frames"
            );
        }
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // 30th clean frame — should trigger recovery.
        let ev = ctrl.record_frame(5_000);
        assert!(ev.is_some(), "Should recover after 30 clean frames");
        let ev = ev.unwrap();
        assert_eq!(ev.previous_level, 1); // Coalesce
        assert_eq!(ev.new_level, 0); // Normal
        assert_eq!(ev.direction, DegradationDirection::Recover);
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_recovery_one_level_at_a_time() {
        let mut ctrl = controller();
        // Advance twice (to Level 2 = ReduceTextureQuality).
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::ReduceTextureQuality);

        // First clean window of 30 frames → recover to Level 1.
        push_frames(&mut ctrl, 5_000, RECOVERY_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // Second clean window of 30 frames → recover to Level 0.
        push_frames(&mut ctrl, 5_000, RECOVERY_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_recovery_threshold_exactly_12ms_does_not_recover() {
        let mut ctrl = controller();
        // Force to Level 1.
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // 30 frames at exactly 12ms — p95 = 12ms, NOT < 12ms. Must not recover.
        push_frames(&mut ctrl, RECOVERY_THRESHOLD_US, RECOVERY_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    #[test]
    fn test_level5_to_normal_requires_5_successive_clean_windows() {
        let mut ctrl = controller();
        // Force to Emergency (Level 5) by advancing 5 times.
        for _ in 0..5 {
            push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        }
        assert_eq!(ctrl.level(), DegradationLevel::Emergency);

        // 5 clean windows (each 30 frames) recovers one level per window.
        let expected = [
            DegradationLevel::ShedTiles,
            DegradationLevel::DisableTransparency,
            DegradationLevel::ReduceTextureQuality,
            DegradationLevel::Coalesce,
            DegradationLevel::Normal,
        ];
        for (i, expected_level) in expected.iter().enumerate() {
            push_frames(&mut ctrl, 5_000, RECOVERY_WINDOW);
            assert_eq!(
                ctrl.level(),
                *expected_level,
                "After {} clean windows, expected {:?}",
                i + 1,
                expected_level
            );
        }
    }

    // ── Tile shedding order ───────────────────────────────────────────────────

    #[test]
    fn test_no_shedding_at_level0_through_level3() {
        for level in [
            DegradationLevel::Normal,
            DegradationLevel::Coalesce,
            DegradationLevel::ReduceTextureQuality,
            DegradationLevel::DisableTransparency,
        ] {
            let mut ctrl = controller();
            // Force to specific level by poking internal state for simplicity.
            ctrl.level = level;
            let tiles = vec![
                TileDescriptor {
                    tile_id: SceneId::new(),
                    lease_priority: 0,
                    z_order: 1,
                },
                TileDescriptor {
                    tile_id: SceneId::new(),
                    lease_priority: 1,
                    z_order: 0,
                },
            ];
            let shed = ctrl.shed_tiles(&tiles);
            assert!(shed.is_empty(), "Level {level} must not shed any tiles");
        }
    }

    #[test]
    fn test_level4_sheds_lowest_priority_tiles_first() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::ShedTiles;

        let id_p0 = SceneId::new();
        let id_p1 = SceneId::new();
        let id_p2a = SceneId::new();
        let id_p2b = SceneId::new();

        // 4 tiles: priorities 0, 1, 2, 2 (two at priority 2)
        let tiles = vec![
            TileDescriptor {
                tile_id: id_p0,
                lease_priority: 0,
                z_order: 10,
            },
            TileDescriptor {
                tile_id: id_p1,
                lease_priority: 1,
                z_order: 5,
            },
            TileDescriptor {
                tile_id: id_p2a,
                lease_priority: 2,
                z_order: 3,
            },
            TileDescriptor {
                tile_id: id_p2b,
                lease_priority: 2,
                z_order: 1,
            },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        // 25% of 4 = 1 tile shed. Should be the priority-2/lowest-z_order tile.
        assert_eq!(shed.len(), 1);
        assert_eq!(
            shed[0].tile_id, id_p2b,
            "Should shed p2/z1 (lowest z_order in highest priority)"
        );
    }

    #[test]
    fn test_level4_spec_scenario_priority_0_1_2_sheds_priority2_first() {
        // Spec scenario (line 269): when entering Level 4 with tiles at
        // priority 0, 1, and 2 → priority-2 tiles MUST be shed first.
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::ShedTiles;

        let id_p0 = SceneId::new();
        let id_p1 = SceneId::new();
        let id_p2 = SceneId::new();

        let tiles = vec![
            TileDescriptor {
                tile_id: id_p0,
                lease_priority: 0,
                z_order: 5,
            },
            TileDescriptor {
                tile_id: id_p1,
                lease_priority: 1,
                z_order: 3,
            },
            TileDescriptor {
                tile_id: id_p2,
                lease_priority: 2,
                z_order: 1,
            },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        assert!(!shed.is_empty());
        // At least id_p2 must be in shed; id_p0 must NOT be.
        let shed_ids: Vec<SceneId> = shed.iter().map(|t| t.tile_id).collect();
        assert!(
            shed_ids.contains(&id_p2),
            "Priority-2 tile must be shed first"
        );
        assert!(
            !shed_ids.contains(&id_p0),
            "Priority-0 tile must be preserved"
        );
    }

    #[test]
    fn test_level5_keeps_only_highest_priority_tile() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::Emergency;

        let id_high = SceneId::new();
        let id_med = SceneId::new();
        let id_low = SceneId::new();

        let tiles = vec![
            TileDescriptor {
                tile_id: id_high,
                lease_priority: 0,
                z_order: 10,
            },
            TileDescriptor {
                tile_id: id_med,
                lease_priority: 1,
                z_order: 5,
            },
            TileDescriptor {
                tile_id: id_low,
                lease_priority: 2,
                z_order: 1,
            },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        let shed_ids: Vec<SceneId> = shed.iter().map(|t| t.tile_id).collect();

        // id_high (priority=0, z_order=10) must be kept; everything else shed.
        assert!(
            !shed_ids.contains(&id_high),
            "Highest-priority tile must not be shed"
        );
        assert!(
            shed_ids.contains(&id_med),
            "Medium-priority tile must be shed at Level 5"
        );
        assert!(
            shed_ids.contains(&id_low),
            "Lowest-priority tile must be shed at Level 5"
        );
        assert_eq!(shed.len(), 2);
    }

    #[test]
    fn test_level5_empty_tiles_returns_empty() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::Emergency;
        let shed = ctrl.shed_tiles(&[]);
        assert!(shed.is_empty());
    }

    #[test]
    fn test_shedding_order_same_priority_higher_z_order_wins() {
        // Spec: within same priority class, higher z_order wins (preserved),
        // lower z_order is shed first.
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::ShedTiles;

        let id_high_z = SceneId::new();
        let id_low_z = SceneId::new();

        let tiles = vec![
            TileDescriptor {
                tile_id: id_high_z,
                lease_priority: 1,
                z_order: 100,
            },
            TileDescriptor {
                tile_id: id_low_z,
                lease_priority: 1,
                z_order: 1,
            },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        assert_eq!(shed.len(), 1);
        assert_eq!(
            shed[0].tile_id, id_low_z,
            "Lower z_order must be shed first within same priority"
        );
    }

    // ── Level 1 coalescing ────────────────────────────────────────────────────

    #[test]
    fn test_coalesce_level0_always_emits() {
        let ctrl = controller();
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
        // At Level 0, should always emit state-stream.
        for _ in 0..20 {
            assert!(ctrl.should_emit_state_stream());
        }
    }

    #[test]
    fn test_coalesce_level1_emits_every_ratio_frames() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::Coalesce;
        // Default ratio = 2: emit on even frame_number, suppress on odd.
        // frame_number starts at 0 and increments each record_frame call.
        // We test should_emit_state_stream() directly against the frame counter.

        // frame_number = 0: 0 % 2 == 0 → emit
        // frame_number = 1: 1 % 2 == 1 → suppress
        // etc.
        let mut emit_count = 0;
        let total = 20;
        for _ in 0..total {
            // Simulate frame advance (record_frame updates frame_number).
            ctrl.frame_number += 1;
            if ctrl.should_emit_state_stream() {
                emit_count += 1;
            }
        }
        // With ratio=2, expect exactly 10 emissions in 20 frames.
        assert_eq!(
            emit_count, 10,
            "Coalesce ratio=2 should emit every other frame"
        );
    }

    // ── Level 2/3 flags ───────────────────────────────────────────────────────

    #[test]
    fn test_reduce_texture_quality_flag() {
        let mut ctrl = controller();
        assert!(!ctrl.should_reduce_texture_quality());
        ctrl.level = DegradationLevel::ReduceTextureQuality;
        assert!(ctrl.should_reduce_texture_quality());
        ctrl.level = DegradationLevel::Emergency;
        assert!(ctrl.should_reduce_texture_quality());
    }

    #[test]
    fn test_disable_transparency_flag() {
        let mut ctrl = controller();
        assert!(!ctrl.should_disable_transparency());
        ctrl.level = DegradationLevel::DisableTransparency;
        assert!(ctrl.should_disable_transparency());
        ctrl.level = DegradationLevel::Emergency;
        assert!(ctrl.should_disable_transparency());
    }

    #[test]
    fn production_policy_mapping_is_exhaustive_and_suppression_uses_scene_ids() {
        let mut ctrl = controller();
        let tiles = vec![
            TileDescriptor {
                tile_id: SceneId::new(),
                lease_priority: 3,
                z_order: 1,
            },
            TileDescriptor {
                tile_id: SceneId::new(),
                lease_priority: 1,
                z_order: 9,
            },
            TileDescriptor {
                tile_id: SceneId::new(),
                lease_priority: 2,
                z_order: 4,
            },
            TileDescriptor {
                tile_id: SceneId::new(),
                lease_priority: 2,
                z_order: 8,
            },
        ];

        ctrl.level = DegradationLevel::ShedTiles;
        let policy = ctrl.compositor_policy(&tiles);
        assert_eq!(policy.level, tze_hud_scene::DegradationLevel::ShedTiles);
        assert_eq!(policy.suppressed_tiles.len(), 1);
        assert!(policy.suppressed_tiles.contains(&tiles[0].tile_id));

        ctrl.level = DegradationLevel::Emergency;
        let policy = ctrl.compositor_policy(&tiles);
        assert_eq!(policy.suppressed_tiles.len(), 3);
        assert!(!policy.suppressed_tiles.contains(&tiles[1].tile_id));
    }

    #[test]
    fn runtime_levels_have_exact_append_only_protocol_mapping() {
        let mut ctrl = controller();
        let expected = [
            (DegradationLevel::Normal, ProtocolDegradationLevel::Normal),
            (
                DegradationLevel::Coalesce,
                ProtocolDegradationLevel::CoalescingMore,
            ),
            (
                DegradationLevel::ReduceTextureQuality,
                ProtocolDegradationLevel::TextureQualityReduced,
            ),
            (
                DegradationLevel::DisableTransparency,
                ProtocolDegradationLevel::RenderingSimplified,
            ),
            (
                DegradationLevel::ShedTiles,
                ProtocolDegradationLevel::SheddingTiles,
            ),
            (
                DegradationLevel::Emergency,
                ProtocolDegradationLevel::EmergencyRendering,
            ),
        ];
        for (runtime, protocol) in expected {
            ctrl.level = runtime;
            assert_eq!(ctrl.protocol_level().1, protocol);
        }
    }

    // ── Telemetry events ──────────────────────────────────────────────────────

    #[test]
    fn test_advance_event_has_correct_fields() {
        let mut ctrl = controller();
        let ev = push_and_get_last_event(&mut ctrl, 20_000, TRIGGER_WINDOW).unwrap();
        assert_eq!(ev.previous_level, 0);
        assert_eq!(ev.new_level, 1);
        assert_eq!(ev.direction, DegradationDirection::Advance);
        assert!(
            ev.frame_time_p95_us > TRIGGER_THRESHOLD_US,
            "p95 should exceed trigger threshold"
        );
    }

    #[test]
    fn render_only_protocol_notices_do_not_claim_capability_reduction() {
        let mut ctrl = controller();
        for level in [
            DegradationLevel::ReduceTextureQuality,
            DegradationLevel::DisableTransparency,
            DegradationLevel::ShedTiles,
            DegradationLevel::Emergency,
        ] {
            ctrl.level = level;
            assert!(
                ctrl.protocol_notice(1).affected_capabilities.is_empty(),
                "{level} changes rendering only"
            );
        }

        ctrl.level = DegradationLevel::Coalesce;
        assert_eq!(
            ctrl.protocol_notice(1).affected_capabilities,
            vec!["state_stream"]
        );
    }

    #[test]
    fn test_recover_event_has_correct_fields() {
        let mut ctrl = controller();
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW); // advance to Coalesce
        let ev = push_and_get_last_event(&mut ctrl, 5_000, RECOVERY_WINDOW).unwrap();
        assert_eq!(ev.previous_level, 1);
        assert_eq!(ev.new_level, 0);
        assert_eq!(ev.direction, DegradationDirection::Recover);
        assert!(
            ev.frame_time_p95_us < RECOVERY_THRESHOLD_US,
            "p95 should be below recovery threshold"
        );
    }

    // ── p95 helper ────────────────────────────────────────────────────────────

    #[test]
    fn test_p95_helper_correctness() {
        let mut deque: VecDeque<(u64, u64)> = VecDeque::new();
        for i in 1..=10u64 {
            deque.push_back((i, i * 1000));
        }
        // Values: [1000, 2000, ..., 10000]
        // p95 nearest-rank: ceil(0.95*10) = 10 → index 9 → 10000
        assert_eq!(p95_of_last_n(&deque, 10), 10_000);
        // p95 of last 5: [6000, 7000, 8000, 9000, 10000]
        // ceil(0.95*5) = 5 → index 4 → 10000
        assert_eq!(p95_of_last_n(&deque, 5), 10_000);
    }

    // ── Helper ────────────────────────────────────────────────────────────────

    /// Push n frames and return the last non-None event (or None if no events).
    fn push_and_get_last_event(
        ctrl: &mut DegradationController,
        frame_time_us: u64,
        n: usize,
    ) -> Option<DegradationEvent> {
        let mut last = None;
        for _ in 0..n {
            if let Some(ev) = ctrl.record_frame(frame_time_us) {
                last = Some(ev);
            }
        }
        last
    }
}
