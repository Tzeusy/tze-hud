//! Versioned, fail-closed evidence for the quiescent runtime efficiency gate.
//!
//! This schema intentionally lives beside frame telemetry rather than inside it:
//! a quiescent runtime emits no frames, so per-frame records cannot prove the
//! absence of submissions, acquisitions, presents, or timer-driven wakeups.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const QUIESCENT_EFFICIENCY_SCHEMA_VERSION: u32 = 1;
pub const QUIESCENT_SCENARIO_NAME: &str = "quiescent_static_scene";
pub const QUIESCENT_SCENARIO_VERSION: u32 = 1;
pub const QUIESCENT_SETTLING_MIN_MS: u64 = 5_000;
pub const QUIESCENT_INTERVAL_MIN_MS: u64 = 60_000;
pub const QUIESCENT_RUNTIME_WAKEUP_CEILING: u64 = 120;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EfficiencyPacingMode {
    EventDriven,
    FixedCadence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuiescentMeasurementStatus {
    Complete,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EfficiencyWindowMode {
    Headless,
    Overlay,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyScenarioIdentity {
    pub name: String,
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyRuntimeIdentity {
    pub build: String,
    pub window_mode: EfficiencyWindowMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyPacingIdentity {
    pub mode: EfficiencyPacingMode,
    pub requested_cadence_hz: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyRendererIdentity {
    pub backend: String,
    pub adapter: String,
    pub software: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedProfileIdentity {
    pub operating_system: String,
    pub cpu_model: String,
    pub logical_cpu_limit: u32,
    pub cpu_limit_enforcement: String,
    pub memory_limit_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyWakeupCounters {
    pub combined_runtime_driven: u64,
    pub main_loop: u64,
    pub compositor_loop: u64,
    pub sources: BTreeMap<String, u64>,
    pub excluded_sampler: u64,
    pub excluded_operating_system: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyGpuCounters {
    pub queue_submissions: u64,
    pub surface_acquisitions: u64,
    pub presents: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuiescentEfficiencyArtifact {
    pub schema_version: u32,
    pub scenario: EfficiencyScenarioIdentity,
    pub runtime: EfficiencyRuntimeIdentity,
    pub pacing: EfficiencyPacingIdentity,
    pub renderer: EfficiencyRendererIdentity,
    pub viewport: EfficiencyViewport,
    pub constrained_profile: Option<ConstrainedProfileIdentity>,
    pub settling_duration_ms: u64,
    pub interval_duration_ms: u64,
    pub status: QuiescentMeasurementStatus,
    pub wakeups: EfficiencyWakeupCounters,
    pub gpu: EfficiencyGpuCounters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfficiencyBudgetResult {
    pub actual: u64,
    pub ceiling: u64,
    pub passed: bool,
}

impl EfficiencyBudgetResult {
    fn at_most(actual: u64, ceiling: u64) -> Self {
        Self {
            actual,
            ceiling,
            passed: actual <= ceiling,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuiescentEfficiencyValidation {
    pub passed: bool,
    pub combined_runtime_wakeups: EfficiencyBudgetResult,
    pub gpu_queue_submissions: EfficiencyBudgetResult,
    pub surface_acquisitions: EfficiencyBudgetResult,
    pub presents: EfficiencyBudgetResult,
    pub violations: Vec<String>,
}

impl QuiescentEfficiencyArtifact {
    /// Validate the normative idle budgets and identity invariants.
    ///
    /// `require_constrained_profile` is set by the llvmpipe/WARP CI lane. The
    /// ordinary schema can also carry manual windowed evidence where CPU
    /// affinity is not part of the invocation.
    pub fn validate(&self, require_constrained_profile: bool) -> QuiescentEfficiencyValidation {
        let mut violations = Vec::new();

        if self.schema_version != QUIESCENT_EFFICIENCY_SCHEMA_VERSION {
            violations.push(format!(
                "schema_version must be {QUIESCENT_EFFICIENCY_SCHEMA_VERSION}, got {}",
                self.schema_version
            ));
        }
        if self.scenario.name != QUIESCENT_SCENARIO_NAME
            || self.scenario.version != QUIESCENT_SCENARIO_VERSION
        {
            violations.push(format!(
                "scenario must be {QUIESCENT_SCENARIO_NAME} v{QUIESCENT_SCENARIO_VERSION}"
            ));
        }
        if self.status != QuiescentMeasurementStatus::Complete {
            violations.push("measurement status must be complete".into());
        }
        if self.runtime.build.trim().is_empty() {
            violations.push("runtime build identity must be non-empty".into());
        }
        if self.pacing.mode != EfficiencyPacingMode::EventDriven
            || self.pacing.requested_cadence_hz.is_some()
        {
            violations.push(
                "quiescent evidence must use event-driven pacing with no requested cadence".into(),
            );
        }
        if self.renderer.backend.trim().is_empty() || self.renderer.adapter.trim().is_empty() {
            violations.push("renderer backend and adapter identities must be non-empty".into());
        }
        if self.viewport.width == 0 || self.viewport.height == 0 {
            violations.push("viewport dimensions must be non-zero".into());
        }
        if self.settling_duration_ms < QUIESCENT_SETTLING_MIN_MS {
            violations.push(format!(
                "settling interval must be at least {QUIESCENT_SETTLING_MIN_MS}ms"
            ));
        }
        if self.interval_duration_ms < QUIESCENT_INTERVAL_MIN_MS {
            violations.push(format!(
                "measurement interval must be at least {QUIESCENT_INTERVAL_MIN_MS}ms"
            ));
        }

        let per_loop_total = self
            .wakeups
            .main_loop
            .saturating_add(self.wakeups.compositor_loop);
        if self.wakeups.combined_runtime_driven != per_loop_total {
            violations.push(format!(
                "combined runtime wakeups {} do not equal main+compositor total {per_loop_total}",
                self.wakeups.combined_runtime_driven
            ));
        }
        let attributed_total = self.wakeups.sources.values().copied().sum::<u64>();
        if self.wakeups.combined_runtime_driven != attributed_total {
            violations.push(format!(
                "combined runtime wakeups {} do not equal attributed source total {attributed_total}",
                self.wakeups.combined_runtime_driven
            ));
        }

        if require_constrained_profile {
            match &self.constrained_profile {
                Some(profile) => {
                    if profile.logical_cpu_limit != 2 {
                        violations.push(format!(
                            "constrained_profile logical_cpu_limit must be 2, got {}",
                            profile.logical_cpu_limit
                        ));
                    }
                    if profile.operating_system.trim().is_empty()
                        || profile.cpu_model.trim().is_empty()
                        || profile.cpu_limit_enforcement.trim().is_empty()
                    {
                        violations.push(
                            "constrained_profile OS, CPU, and enforcement identities must be non-empty"
                                .into(),
                        );
                    }
                    if !self.renderer.software {
                        violations.push(
                            "constrained_profile requires an explicitly identified software renderer"
                                .into(),
                        );
                    }
                }
                None => violations.push(
                    "constrained_profile is required for the constrained efficiency lane".into(),
                ),
            }
        }

        let combined_runtime_wakeups = EfficiencyBudgetResult::at_most(
            self.wakeups.combined_runtime_driven,
            QUIESCENT_RUNTIME_WAKEUP_CEILING,
        );
        let gpu_queue_submissions = EfficiencyBudgetResult::at_most(self.gpu.queue_submissions, 0);
        let surface_acquisitions =
            EfficiencyBudgetResult::at_most(self.gpu.surface_acquisitions, 0);
        let presents = EfficiencyBudgetResult::at_most(self.gpu.presents, 0);

        if !combined_runtime_wakeups.passed {
            violations.push(format!(
                "runtime-driven wakeups {} exceed ceiling {}",
                combined_runtime_wakeups.actual, combined_runtime_wakeups.ceiling
            ));
        }
        if !gpu_queue_submissions.passed {
            violations.push(format!(
                "GPU queue submissions must be zero, got {}",
                gpu_queue_submissions.actual
            ));
        }
        if !surface_acquisitions.passed {
            violations.push(format!(
                "surface acquisitions must be zero, got {}",
                surface_acquisitions.actual
            ));
        }
        if !presents.passed {
            violations.push(format!("presents must be zero, got {}", presents.actual));
        }

        QuiescentEfficiencyValidation {
            passed: violations.is_empty(),
            combined_runtime_wakeups,
            gpu_queue_submissions,
            surface_acquisitions,
            presents,
            violations,
        }
    }
}
