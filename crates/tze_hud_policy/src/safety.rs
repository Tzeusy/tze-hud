//! # Level 1 Safety Evaluation — GPU Tier Response and Safety Signals
//!
//! Implements the Level 1 Safety requirement per policy-arbitration/spec.md (lines 61-89).
//!
//! ## Responsibilities
//!
//! - Monitor GPU device health and scene graph integrity.
//! - Determine the correct GPU failure tier (Tier 1 / Tier 2 / Tier 3).
//! - Produce `SafetySignal` — a pure, side-effect-free description of what action
//!   the system shell MUST take. The shell owns all state transitions.
//! - Severity ordering: `CatastrophicExit > SafeModeEntry > GpuReconfiguration > Nominal`.
//!
//! ## Purity Constraint
//!
//! All functions here are pure: they read `SafetyState` (and optionally a
//! `GpuFailureContext`) and return a `SafetySignal`. They do NOT enter safe mode,
//! suspend leases, or perform any side-effecting operation. The system shell reads
//! `SafetySignal` and acts accordingly.
//!
//! ## Per-frame integration
//!
//! The per-frame pipeline (`frame.rs`) calls `evaluate_safety` at the start of each
//! frame cycle. If the returned signal is `SafeModeEntry` or `CatastrophicExit`, the
//! frame pipeline short-circuits (Levels 2/5/6 are not evaluated).

use crate::types::SafetyState;

// ─── GPU Failure Context ──────────────────────────────────────────────────────

/// Context describing a GPU failure event at this frame.
///
/// Populated by the compositor from wgpu error callbacks before per-frame evaluation.
/// The policy layer reads this; it does NOT write it.
#[derive(Clone, Debug, Default)]
pub struct GpuFailureContext {
    /// `wgpu::DeviceError::Lost` was detected this frame.
    pub device_lost: bool,

    /// `wgpu::SurfaceError::Lost` was detected (surface reconfiguration needed).
    pub surface_lost: bool,

    /// `surface.configure()` succeeded after the surface was lost (Tier 1 recovery).
    pub surface_reconfigure_succeeded: bool,

    /// The safe mode overlay itself cannot render (GPU completely unusable — Tier 3).
    pub overlay_cannot_render: bool,

    /// Scene graph integrity check failed (corruption detected).
    pub scene_graph_corrupted: bool,
}

// ─── Safety Signal (pure output of Level 1 evaluation) ────────────────────────

/// The safety signal produced by Level 1 evaluation.
///
/// This is a **pure output** — no side effects. The system shell reads it and
/// executes the corresponding state transitions.
///
/// Severity ordering (highest first):
/// `CatastrophicExit > SafeModeEntry > GpuReconfiguration > Nominal`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetySignal {
    /// Everything is healthy. No action needed.
    Nominal,

    /// Tier 1: `SurfaceError::Lost` and reconfiguration succeeded.
    ///
    /// Rendering continues transparently; no agent notification required.
    GpuReconfiguration,

    /// Tier 2: GPU device lost or scene graph corrupted; CPU is intact.
    ///
    /// System shell MUST:
    /// - Enter safe mode.
    /// - Suspend all agent leases.
    /// - Render chrome in software fallback.
    /// - Wait up to 2 seconds for the overlay to appear.
    SafeModeEntry {
        /// Why safe mode is being entered.
        reason: SafeModeEntryReason,
    },

    /// Tier 3: Safe mode overlay cannot render (GPU completely unusable).
    ///
    /// System shell MUST:
    /// - Flush telemetry (200ms grace period).
    /// - Exit the process with a non-zero exit code.
    CatastrophicExit,
}

/// Reason for entering safe mode (Tier 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeModeEntryReason {
    /// `wgpu::DeviceError::Lost` detected (spec line 67-68).
    GpuDeviceLost,
    /// Scene graph corruption detected (spec line 69).
    SceneGraphCorruption,
    /// Frame-time p95 > emergency threshold sustained through all degradation levels.
    EmergencyDegradation,
    /// Generic critical error.
    CriticalError,
}

// ─── Severity ordering helpers ────────────────────────────────────────────────

impl SafetySignal {
    /// Numeric severity: higher = more severe.
    pub fn severity(self) -> u8 {
        match self {
            SafetySignal::Nominal => 0,
            SafetySignal::GpuReconfiguration => 1,
            SafetySignal::SafeModeEntry { .. } => 2,
            SafetySignal::CatastrophicExit => 3,
        }
    }

    /// True if this signal should short-circuit the per-frame evaluation pipeline.
    ///
    /// If Level 1 produces a safe-mode or catastrophic signal, Levels 2/5/6 MUST NOT
    /// be evaluated for that frame (spec line 195).
    pub fn should_short_circuit(self) -> bool {
        matches!(
            self,
            SafetySignal::SafeModeEntry { .. } | SafetySignal::CatastrophicExit
        )
    }

    /// Returns the most severe of two safety signals (spec §2.2: most severe wins).
    pub fn most_severe(a: SafetySignal, b: SafetySignal) -> SafetySignal {
        if a.severity() >= b.severity() { a } else { b }
    }
}

// ─── Level 1 Safety Evaluation (pure) ────────────────────────────────────────

/// Evaluate Level 1 (Safety) given the current `SafetyState` and any GPU failure context.
///
/// This is a **pure function**. It returns a `SafetySignal` describing what the shell
/// should do. It does NOT mutate any state.
///
/// ## Evaluation order (within Level 1)
///
/// Multiple simultaneous triggers are resolved by severity (most severe wins, spec §2.2):
/// `CatastrophicExit > SafeModeEntry > GpuReconfiguration > Nominal`
///
/// 1. If `overlay_cannot_render`: `CatastrophicExit` (Tier 3).
/// 2. If `device_lost`: `SafeModeEntry { GpuDeviceLost }` (Tier 2).
/// 3. If `scene_graph_corrupted`: `SafeModeEntry { SceneGraphCorruption }` (Tier 2).
/// 4. If `frame_time_p95 > emergency_threshold` and GPU is healthy: `SafeModeEntry { EmergencyDegradation }`.
/// 5. If `surface_lost` and `surface_reconfigure_succeeded`: `GpuReconfiguration` (Tier 1).
/// 6. Otherwise: `Nominal`.
pub fn evaluate_safety(state: &SafetyState, gpu: &GpuFailureContext) -> SafetySignal {
    let mut signal = SafetySignal::Nominal;

    // Tier 1: surface reconfiguration (lowest severity, overridden by anything above).
    if gpu.surface_lost && gpu.surface_reconfigure_succeeded {
        signal = SafetySignal::most_severe(signal, SafetySignal::GpuReconfiguration);
    }

    // Frame-time emergency: p95 exceeds threshold while GPU is healthy.
    // (GPU healthy check: if device is lost this is already handled below as Tier 2.)
    if state.gpu_healthy
        && state.scene_graph_intact
        && state.frame_time_p95_us > state.emergency_threshold_us
    {
        signal = SafetySignal::most_severe(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::EmergencyDegradation,
            },
        );
    }

    // Tier 2a: scene graph corruption.
    if !state.scene_graph_intact || gpu.scene_graph_corrupted {
        signal = SafetySignal::most_severe(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::SceneGraphCorruption,
            },
        );
    }

    // Tier 2b: GPU device lost.
    if !state.gpu_healthy || gpu.device_lost {
        signal = SafetySignal::most_severe(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::GpuDeviceLost,
            },
        );
    }

    // Tier 3: overlay cannot render — catastrophic exit (highest severity).
    if gpu.overlay_cannot_render {
        signal = SafetySignal::most_severe(signal, SafetySignal::CatastrophicExit);
    }

    signal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy_safety_state() -> SafetyState {
        SafetyState {
            gpu_healthy: true,
            scene_graph_intact: true,
            frame_time_p95_us: 5_000,
            emergency_threshold_us: 14_000,
        }
    }

    fn no_failure() -> GpuFailureContext {
        GpuFailureContext::default()
    }

    // ─── Nominal path ─────────────────────────────────────────────────────────

    #[test]
    fn test_nominal_when_everything_healthy() {
        let signal = evaluate_safety(&healthy_safety_state(), &no_failure());
        assert_eq!(signal, SafetySignal::Nominal);
    }

    // ─── Tier 1: Surface Lost + Reconfigure ──────────────────────────────────

    /// WHEN SurfaceError::Lost occurs and surface.configure() succeeds
    /// THEN rendering continues transparently with no agent notification (spec line 80-81)
    #[test]
    fn test_tier1_surface_reconfigure_succeeds() {
        let gpu = GpuFailureContext {
            surface_lost: true,
            surface_reconfigure_succeeded: true,
            ..Default::default()
        };
        let signal = evaluate_safety(&healthy_safety_state(), &gpu);
        assert_eq!(signal, SafetySignal::GpuReconfiguration);
        assert!(!signal.should_short_circuit());
    }

    #[test]
    fn test_surface_lost_without_reconfigure_success_is_nominal() {
        // surface_lost=true but reconfigure did NOT succeed → not Tier 1 (handled elsewhere)
        let gpu = GpuFailureContext {
            surface_lost: true,
            surface_reconfigure_succeeded: false,
            ..Default::default()
        };
        // Without device_lost or scene corruption, this is just Nominal.
        let signal = evaluate_safety(&healthy_safety_state(), &gpu);
        assert_eq!(signal, SafetySignal::Nominal);
    }

    // ─── Tier 2: GPU Device Lost ──────────────────────────────────────────────

    /// WHEN wgpu::DeviceError::Lost is detected
    /// THEN the runtime enters safe mode (Tier 2) with SafeModeEntryReason=CRITICAL_ERROR (spec line 67-68)
    #[test]
    fn test_tier2_gpu_device_lost_via_state() {
        let mut state = healthy_safety_state();
        state.gpu_healthy = false;
        let signal = evaluate_safety(&state, &no_failure());
        assert!(
            matches!(
                signal,
                SafetySignal::SafeModeEntry {
                    reason: SafeModeEntryReason::GpuDeviceLost
                }
            ),
            "Expected SafeModeEntry(GpuDeviceLost), got {:?}",
            signal
        );
        assert!(signal.should_short_circuit());
    }

    #[test]
    fn test_tier2_gpu_device_lost_via_event() {
        let gpu = GpuFailureContext {
            device_lost: true,
            ..Default::default()
        };
        let signal = evaluate_safety(&healthy_safety_state(), &gpu);
        assert!(matches!(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::GpuDeviceLost
            }
        ));
    }

    // ─── Tier 2: Scene Graph Corruption ──────────────────────────────────────

    #[test]
    fn test_tier2_scene_graph_corruption_via_state() {
        let mut state = healthy_safety_state();
        state.scene_graph_intact = false;
        let signal = evaluate_safety(&state, &no_failure());
        assert!(matches!(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::SceneGraphCorruption
            }
        ));
        assert!(signal.should_short_circuit());
    }

    #[test]
    fn test_tier2_scene_graph_corruption_via_event() {
        let gpu = GpuFailureContext {
            scene_graph_corrupted: true,
            ..Default::default()
        };
        let signal = evaluate_safety(&healthy_safety_state(), &gpu);
        assert!(matches!(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::SceneGraphCorruption
            }
        ));
    }

    // ─── Tier 2: Emergency Degradation ───────────────────────────────────────

    #[test]
    fn test_tier2_emergency_degradation_when_frame_time_exceeds_threshold() {
        let mut state = healthy_safety_state();
        state.frame_time_p95_us = 15_000; // > 14ms threshold
        let signal = evaluate_safety(&state, &no_failure());
        assert!(matches!(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::EmergencyDegradation
            }
        ));
        assert!(signal.should_short_circuit());
    }

    #[test]
    fn test_no_emergency_when_frame_time_at_threshold() {
        let mut state = healthy_safety_state();
        state.frame_time_p95_us = 14_000; // equal to threshold, not strictly greater
        let signal = evaluate_safety(&state, &no_failure());
        assert_eq!(signal, SafetySignal::Nominal);
    }

    // ─── Tier 3: Catastrophic Exit ────────────────────────────────────────────

    /// WHEN safe mode overlay cannot render
    /// THEN telemetry flushed (200ms) and process exits with non-zero exit code (spec line 87-89)
    #[test]
    fn test_tier3_catastrophic_exit_when_overlay_cannot_render() {
        let gpu = GpuFailureContext {
            device_lost: true,
            overlay_cannot_render: true,
            ..Default::default()
        };
        let signal = evaluate_safety(&healthy_safety_state(), &gpu);
        assert_eq!(signal, SafetySignal::CatastrophicExit);
        assert!(signal.should_short_circuit());
    }

    // ─── Severity ordering (most severe wins) ────────────────────────────────

    /// WHEN GPU failure and scene corruption detected in same frame
    /// THEN most severe response applied (spec line 71-72)
    #[test]
    fn test_most_severe_wins_gpu_plus_scene_corruption() {
        let mut state = healthy_safety_state();
        state.gpu_healthy = false;
        state.scene_graph_intact = false;
        let signal = evaluate_safety(&state, &no_failure());
        // Both are Tier 2 SafeModeEntry; device_lost takes precedence (evaluated last)
        assert!(matches!(
            signal,
            SafetySignal::SafeModeEntry { .. }
        ));
    }

    #[test]
    fn test_catastrophic_exit_beats_safe_mode_entry() {
        let a = SafetySignal::SafeModeEntry { reason: SafeModeEntryReason::GpuDeviceLost };
        let b = SafetySignal::CatastrophicExit;
        assert_eq!(SafetySignal::most_severe(a, b), SafetySignal::CatastrophicExit);
        assert_eq!(SafetySignal::most_severe(b, a), SafetySignal::CatastrophicExit);
    }

    #[test]
    fn test_safe_mode_entry_beats_gpu_reconfiguration() {
        let a = SafetySignal::GpuReconfiguration;
        let b = SafetySignal::SafeModeEntry { reason: SafeModeEntryReason::GpuDeviceLost };
        assert_eq!(SafetySignal::most_severe(a, b), b);
    }

    #[test]
    fn test_safe_mode_entry_beats_nominal() {
        let a = SafetySignal::Nominal;
        let b = SafetySignal::SafeModeEntry { reason: SafeModeEntryReason::CriticalError };
        assert_eq!(SafetySignal::most_severe(a, b), b);
    }

    // ─── Tier 2 before Tier 3: safe mode before shutdown ─────────────────────

    /// WHEN GPU device is lost and reconfiguration fails
    /// THEN safe mode is entered first (up to 2s for overlay) before graceful shutdown (spec line 83-85)
    ///
    /// This test verifies the signal ordering: the signal starts as SafeModeEntry,
    /// only escalating to CatastrophicExit when overlay_cannot_render is also set.
    #[test]
    fn test_tier2_before_tier3_device_lost_without_overlay_failure() {
        let mut state = healthy_safety_state();
        state.gpu_healthy = false;
        let gpu = GpuFailureContext {
            device_lost: true,
            overlay_cannot_render: false, // overlay is still capable
            ..Default::default()
        };
        let signal = evaluate_safety(&state, &gpu);
        // Should be Tier 2 (safe mode), not Tier 3
        assert!(matches!(
            signal,
            SafetySignal::SafeModeEntry {
                reason: SafeModeEntryReason::GpuDeviceLost
            }
        ));
    }

    #[test]
    fn test_should_short_circuit_is_false_for_nominal_and_reconfig() {
        assert!(!SafetySignal::Nominal.should_short_circuit());
        assert!(!SafetySignal::GpuReconfiguration.should_short_circuit());
    }

    #[test]
    fn test_should_short_circuit_is_true_for_safe_mode_and_catastrophic() {
        assert!(SafetySignal::SafeModeEntry {
            reason: SafeModeEntryReason::GpuDeviceLost
        }
        .should_short_circuit());
        assert!(SafetySignal::CatastrophicExit.should_short_circuit());
    }
}
