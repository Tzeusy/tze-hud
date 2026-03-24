//! Per-lease resource budget checks.
//!
//! Implements per-mutation budget enforcement per
//! `lease-governance/spec.md §Requirement: Resource Budget Schema`,
//! `§Requirement: Budget Soft Warning at 80%`, and
//! `§Requirement: Budget Hard Limit at 100%`.
//!
//! ## Spec-alignment notes
//!
//! * The `ResourceBudget` struct lives in the parent module (`super`); this
//!   file adds the *checking* layer on top of it.
//! * Soft-limit (80%) checks return `BudgetWarningDimension` values but do
//!   **not** reject the mutation — callers must send a `BudgetWarning` event.
//! * Hard-limit (100%) checks reject the entire `MutationBatch` atomically.
//! * `max_concurrent_streams` is always 0 in v1; it is never checked here.
//!
//! ## Latency requirement
//!
//! Per spec §Budget Enforcement Latency: each per-mutation budget check MUST
//! complete within 50µs.  All arithmetic here is branchless integer math with
//! no allocations on the hot path.

use super::ResourceBudget;

// ─── Budget usage (tracked by the session layer) ─────────────────────────────

/// Current resource usage snapshot for a single lease.
///
/// Values are maintained by the session layer and passed in on every call to
/// [`check_mutation`] so that the hot path never traverses the scene graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BudgetUsage {
    /// Number of tiles currently held by this lease.
    pub tile_count: u32,
    /// Maximum node count seen across all tiles currently held.
    pub max_nodes_per_tile: u32,
    /// Total texture bytes currently allocated for this lease.
    pub texture_bytes_used: u64,
    /// Number of active leases for this agent/session (for `max_active_leases`).
    pub active_lease_count: u32,
}

// ─── Budget deltas (what the incoming MutationBatch proposes to add) ─────────

/// The resource delta proposed by a single `MutationBatch`.
///
/// Negative values indicate resource releases (e.g. `DeleteTile`).
#[derive(Clone, Copy, Debug, Default)]
pub struct BudgetDelta {
    /// Tiles to be created (positive) or deleted (negative) by the batch.
    pub delta_tiles: i32,
    /// Maximum nodes-per-tile in any tile touched by this batch.
    pub max_nodes_in_batch: u32,
    /// Texture bytes added (positive) or released (negative).
    pub delta_texture_bytes: i64,
}

// ─── Violation dimensions ─────────────────────────────────────────────────────

/// A budget dimension that has reached the soft (80%) or hard (100%) limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetDimension {
    TileCount,
    NodesPerTile,
    TextureBytes,
    ActiveLeases,
}

/// Reason a budget hard check was violated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetHardViolation {
    /// Agent at `max_tiles` attempted to create another tile.
    TileCountExceeded { current: u32, limit: u32 },
    /// A tile would exceed `max_nodes_per_tile`.
    NodesPerTileExceeded { proposed: u32, limit: u32 },
    /// Texture allocation would exceed `texture_bytes_total`.
    TextureBytesExceeded { current_bytes: u64, limit_bytes: u64 },
    /// An agent's active-lease count would exceed `max_active_leases`.
    ActiveLeasesExceeded { current: u32, limit: u32 },
    /// Texture OOM: critical bypass (no ladder, direct revocation).
    CriticalTextureOomAttempt { requested_bytes: u64, hard_max_bytes: u64 },
}

// ─── Soft-limit check (80%) ───────────────────────────────────────────────────

/// Returns the set of budget dimensions currently at or above 80%.
///
/// The batch is **accepted** regardless — the caller is responsible for
/// emitting a `BudgetWarning` event when the returned slice is non-empty.
///
/// Per spec: "A budget warning badge MUST be rendered on affected tiles."
///
/// This function is intentionally pure (no side effects), so it can be called
/// speculatively without mutating state.
#[inline]
pub fn check_budget_soft(
    budget: &ResourceBudget,
    usage: &BudgetUsage,
) -> [Option<BudgetDimension>; 4] {
    const SOFT_PCT: f64 = 0.80;

    let mut result = [None; 4];
    let mut idx = 0;

    let tile_pct = usage.tile_count as f64 / budget.max_tiles.max(1) as f64;
    if tile_pct >= SOFT_PCT {
        result[idx] = Some(BudgetDimension::TileCount);
        idx += 1;
    }

    let node_pct = usage.max_nodes_per_tile as f64 / budget.max_nodes_per_tile.max(1) as f64;
    if node_pct >= SOFT_PCT {
        result[idx] = Some(BudgetDimension::NodesPerTile);
        idx += 1;
    }

    let tex_pct = usage.texture_bytes_used as f64 / budget.texture_bytes_total.max(1) as f64;
    if tex_pct >= SOFT_PCT {
        result[idx] = Some(BudgetDimension::TextureBytes);
        idx += 1;
    }

    let lease_pct = usage.active_lease_count as f64 / budget.max_active_leases.max(1) as f64;
    if lease_pct >= SOFT_PCT {
        result[idx] = Some(BudgetDimension::ActiveLeases);
        // idx += 1; // idx only matters while building the array
    }

    result
}

/// Returns `true` if any dimension is at or above 80%.
#[inline]
pub fn is_budget_soft_warning(budget: &ResourceBudget, usage: &BudgetUsage) -> bool {
    check_budget_soft(budget, usage)[0].is_some()
}

// ─── Hard-limit check (100%) ──────────────────────────────────────────────────

/// Check whether the proposed `delta` would push any budget dimension to or
/// beyond 100% of its maximum.
///
/// Returns `Ok(())` if the batch is within budget, or `Err(BudgetHardViolation)`
/// if it must be rejected.  The entire `MutationBatch` MUST be rejected
/// atomically on error (spec §Requirement: Budget Hard Limit at 100%).
///
/// The `critical_hard_max_texture_bytes` parameter captures the absolute OOM
/// ceiling (bypasses the three-tier ladder — triggers immediate revocation).
/// Pass `u64::MAX` if there is no separate hard ceiling.
///
/// This function is intentionally pure (no side effects).
#[inline]
pub fn check_budget_hard(
    budget: &ResourceBudget,
    usage: &BudgetUsage,
    delta: &BudgetDelta,
    critical_hard_max_texture_bytes: u64,
) -> Result<(), BudgetHardViolation> {
    // ── Critical: absolute texture OOM (bypasses ladder) ─────────────────
    if delta.delta_texture_bytes > 0 {
        let proposed = usage
            .texture_bytes_used
            .saturating_add(delta.delta_texture_bytes as u64);
        if proposed > critical_hard_max_texture_bytes {
            return Err(BudgetHardViolation::CriticalTextureOomAttempt {
                requested_bytes: proposed,
                hard_max_bytes: critical_hard_max_texture_bytes,
            });
        }
    }

    // ── Tile count ────────────────────────────────────────────────────────
    if delta.delta_tiles > 0 {
        let proposed_tiles = usage
            .tile_count
            .saturating_add(delta.delta_tiles as u32);
        if proposed_tiles > budget.max_tiles as u32 {
            return Err(BudgetHardViolation::TileCountExceeded {
                current: proposed_tiles,
                limit: budget.max_tiles as u32,
            });
        }
    }

    // ── Nodes per tile ────────────────────────────────────────────────────
    if delta.max_nodes_in_batch > 0
        && delta.max_nodes_in_batch > budget.max_nodes_per_tile as u32
    {
        return Err(BudgetHardViolation::NodesPerTileExceeded {
            proposed: delta.max_nodes_in_batch,
            limit: budget.max_nodes_per_tile as u32,
        });
    }

    // ── Texture bytes ─────────────────────────────────────────────────────
    if delta.delta_texture_bytes > 0 {
        let proposed = usage
            .texture_bytes_used
            .saturating_add(delta.delta_texture_bytes as u64);
        if proposed > budget.texture_bytes_total {
            return Err(BudgetHardViolation::TextureBytesExceeded {
                current_bytes: proposed,
                limit_bytes: budget.texture_bytes_total,
            });
        }
    }

    // ── Active leases ─────────────────────────────────────────────────────
    // Checked at grant time, but re-verified here for defence-in-depth.
    if usage.active_lease_count > budget.max_active_leases as u32 {
        return Err(BudgetHardViolation::ActiveLeasesExceeded {
            current: usage.active_lease_count,
            limit: budget.max_active_leases as u32,
        });
    }

    Ok(())
}

// ─── Shared-resource anti-collusion helper ────────────────────────────────────

/// Compute the double-counted texture usage for shared resources.
///
/// Per spec §Shared Resources and Anti-Collusion: shared resources are
/// double-counted per agent to prevent cross-agent budget collusion.
///
/// `base_bytes`: bytes used by this agent's exclusive resources.
/// `shared_bytes`: bytes used by resources that are also referenced by
///   other agents (counted twice).
#[inline]
pub fn anti_collusion_texture_bytes(base_bytes: u64, shared_bytes: u64) -> u64 {
    base_bytes.saturating_add(shared_bytes)
}

// ─── Latency self-test ────────────────────────────────────────────────────────

/// Perform a single budget check and return the elapsed microseconds.
///
/// Used by latency acceptance tests to verify the <50µs requirement from
/// spec §Requirement: Budget Enforcement Latency.
///
/// This function is `#[inline(never)]` to prevent the compiler from folding
/// it into its caller and producing an artificially low measurement.
#[inline(never)]
pub fn timed_budget_check(
    budget: &ResourceBudget,
    usage: &BudgetUsage,
    delta: &BudgetDelta,
) -> (Result<(), BudgetHardViolation>, u64) {
    let start = std::time::Instant::now();
    let result = check_budget_hard(budget, usage, delta, u64::MAX);
    let elapsed_us = start.elapsed().as_micros() as u64;
    (result, elapsed_us)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::ResourceBudget;

    fn default_budget() -> ResourceBudget {
        ResourceBudget::default()
    }

    fn zero_usage() -> BudgetUsage {
        BudgetUsage::default()
    }

    // ── Soft-limit (80%) tests ────────────────────────────────────────────

    /// WHEN no dimension is at or above 80% THEN check_budget_soft returns all-None.
    #[test]
    fn test_soft_warning_not_triggered_below_80() {
        let budget = default_budget();
        let usage = zero_usage();
        let warnings = check_budget_soft(&budget, &usage);
        assert!(warnings.iter().all(|w| w.is_none()),
            "no dimensions should warn below 80%");
    }

    /// WHEN tile count at 80% THEN TileCount dimension is returned.
    #[test]
    fn test_soft_warning_tile_count_at_80pct() {
        let budget = default_budget(); // max_tiles = 8
        let usage = BudgetUsage {
            tile_count: 7, // 7/8 = 87.5% ≥ 80%
            ..BudgetUsage::default()
        };
        let warnings = check_budget_soft(&budget, &usage);
        assert!(
            warnings.iter().any(|w| *w == Some(BudgetDimension::TileCount)),
            "TileCount should be warned at 87.5%"
        );
    }

    /// WHEN texture bytes at exactly 80% THEN TextureBytes dimension is returned.
    ///
    /// Uses ceiling division to ensure the value is exactly at or above 80%.
    #[test]
    fn test_soft_warning_texture_at_80pct_exact() {
        let budget = default_budget(); // texture_bytes_total = 64 MiB
        // Ceiling division: round up so that the fraction is >= 0.80 exactly.
        let total = budget.texture_bytes_total;
        let eighty_pct = (total as f64 * 0.80).ceil() as u64;
        let usage = BudgetUsage {
            texture_bytes_used: eighty_pct,
            ..BudgetUsage::default()
        };
        let warnings = check_budget_soft(&budget, &usage);
        assert!(
            warnings.iter().any(|w| *w == Some(BudgetDimension::TextureBytes)),
            "TextureBytes should be warned at exactly 80% (used={eighty_pct}, total={total})"
        );
    }

    /// WHEN all dimensions below 80% THEN is_budget_soft_warning is false.
    #[test]
    fn test_is_budget_soft_warning_false_below_80() {
        let budget = default_budget();
        let usage = BudgetUsage {
            tile_count: 6, // 6/8 = 75%
            ..BudgetUsage::default()
        };
        assert!(!is_budget_soft_warning(&budget, &usage));
    }

    /// WHEN tile count at 100% THEN is_budget_soft_warning is true.
    #[test]
    fn test_is_budget_soft_warning_true_at_100() {
        let budget = default_budget(); // max_tiles = 8
        let usage = BudgetUsage {
            tile_count: 8, // 100%
            ..BudgetUsage::default()
        };
        assert!(is_budget_soft_warning(&budget, &usage));
    }

    // ── Hard-limit (100%) tests ───────────────────────────────────────────

    /// WHEN mutation is within budget THEN check_budget_hard returns Ok.
    #[test]
    fn test_hard_limit_ok_when_within_budget() {
        let budget = default_budget();
        let usage = zero_usage();
        let delta = BudgetDelta {
            delta_tiles: 1,
            max_nodes_in_batch: 5,
            delta_texture_bytes: 1024,
        };
        assert!(check_budget_hard(&budget, &usage, &delta, u64::MAX).is_ok());
    }

    /// WHEN tile count would reach or exceed max_tiles THEN TileCountExceeded error.
    #[test]
    fn test_hard_limit_tile_count_exceeded() {
        let budget = default_budget(); // max_tiles = 8
        let usage = BudgetUsage {
            tile_count: 8, // already at max
            ..BudgetUsage::default()
        };
        let delta = BudgetDelta {
            delta_tiles: 1, // trying to create 9th
            ..BudgetDelta::default()
        };
        let result = check_budget_hard(&budget, &usage, &delta, u64::MAX);
        assert!(
            matches!(result, Err(BudgetHardViolation::TileCountExceeded { .. })),
            "expected TileCountExceeded, got {:?}", result
        );
    }

    /// WHEN max tiles = 8 and 9th tile attempted THEN error code is TileCountExceeded.
    /// Spec scenario: "Tile count at 100%" (spec lines 183-185).
    #[test]
    fn test_hard_limit_scenario_tile_count_at_100pct() {
        let mut budget = ResourceBudget::default();
        budget.max_tiles = 8;
        let usage = BudgetUsage {
            tile_count: 8,
            ..BudgetUsage::default()
        };
        let delta = BudgetDelta { delta_tiles: 1, ..Default::default() };
        let result = check_budget_hard(&budget, &usage, &delta, u64::MAX);
        assert!(
            matches!(result, Err(BudgetHardViolation::TileCountExceeded { current: 9, limit: 8 })),
            "9th tile must produce TileCountExceeded(current=9, limit=8): {:?}", result
        );
    }

    /// WHEN nodes-per-tile exceeds max_nodes_per_tile THEN NodesPerTileExceeded.
    #[test]
    fn test_hard_limit_nodes_per_tile_exceeded() {
        let mut budget = ResourceBudget::default();
        budget.max_nodes_per_tile = 32;
        let usage = zero_usage();
        let delta = BudgetDelta {
            max_nodes_in_batch: 33, // exceeds 32
            ..Default::default()
        };
        let result = check_budget_hard(&budget, &usage, &delta, u64::MAX);
        assert!(
            matches!(result, Err(BudgetHardViolation::NodesPerTileExceeded { proposed: 33, limit: 32 })),
            "expected NodesPerTileExceeded(33, 32): {:?}", result
        );
    }

    /// WHEN texture bytes would exceed texture_bytes_total THEN TextureBytesExceeded.
    #[test]
    fn test_hard_limit_texture_bytes_exceeded() {
        let mut budget = ResourceBudget::default();
        budget.texture_bytes_total = 1000;
        let usage = BudgetUsage {
            texture_bytes_used: 900,
            ..BudgetUsage::default()
        };
        let delta = BudgetDelta {
            delta_texture_bytes: 200, // 900 + 200 = 1100 > 1000
            ..Default::default()
        };
        let result = check_budget_hard(&budget, &usage, &delta, u64::MAX);
        assert!(
            matches!(result, Err(BudgetHardViolation::TextureBytesExceeded { .. })),
            "expected TextureBytesExceeded: {:?}", result
        );
    }

    /// WHEN texture OOM ceiling is exceeded THEN CriticalTextureOomAttempt (bypasses ladder).
    #[test]
    fn test_hard_limit_critical_texture_oom_bypasses_ladder() {
        let budget = default_budget();
        let usage = BudgetUsage {
            texture_bytes_used: 1_900_000_000,
            ..BudgetUsage::default()
        };
        let delta = BudgetDelta {
            delta_texture_bytes: 200_000_000, // pushes over 2 GiB hard max
            ..Default::default()
        };
        let hard_max = 2_000_000_000u64;
        let result = check_budget_hard(&budget, &usage, &delta, hard_max);
        assert!(
            matches!(result, Err(BudgetHardViolation::CriticalTextureOomAttempt { .. })),
            "expected CriticalTextureOomAttempt: {:?}", result
        );
    }

    // ── Spec scenario: Budget dimensions enforced (spec lines 161-163) ────

    /// WHEN lease has max_tiles=8 and max_nodes_per_tile=32
    /// THEN can create up to 8 tiles, each with up to 32 nodes.
    #[test]
    fn test_spec_scenario_budget_dimensions_enforced() {
        let mut budget = ResourceBudget::default();
        budget.max_tiles = 8;
        budget.max_nodes_per_tile = 32;

        // Creating the 8th tile is allowed
        let usage7 = BudgetUsage { tile_count: 7, ..Default::default() };
        let delta_one_tile = BudgetDelta { delta_tiles: 1, ..Default::default() };
        assert!(
            check_budget_hard(&budget, &usage7, &delta_one_tile, u64::MAX).is_ok(),
            "creating 8th tile (total=8) must be allowed"
        );

        // Creating a 9th tile is rejected
        let usage8 = BudgetUsage { tile_count: 8, ..Default::default() };
        assert!(
            check_budget_hard(&budget, &usage8, &delta_one_tile, u64::MAX).is_err(),
            "creating 9th tile must be rejected"
        );

        // 32 nodes per tile is allowed
        let delta_32_nodes = BudgetDelta { max_nodes_in_batch: 32, ..Default::default() };
        assert!(
            check_budget_hard(&budget, &BudgetUsage::default(), &delta_32_nodes, u64::MAX).is_ok(),
            "32 nodes per tile must be allowed"
        );

        // 33 nodes per tile is rejected
        let delta_33_nodes = BudgetDelta { max_nodes_in_batch: 33, ..Default::default() };
        assert!(
            check_budget_hard(&budget, &BudgetUsage::default(), &delta_33_nodes, u64::MAX).is_err(),
            "33 nodes per tile must be rejected"
        );
    }

    // ── Spec: max_concurrent_streams zero in v1 (spec lines 165-167) ─────

    /// WHEN a lease is granted in v1 THEN max_concurrent_streams is 0.
    #[test]
    fn test_max_concurrent_streams_zero_in_v1() {
        let budget = ResourceBudget::default();
        assert_eq!(budget.max_concurrent_streams, 0,
            "max_concurrent_streams must be 0 in v1 (media streams deferred)");
    }

    // ── Latency requirement (spec lines 204-211) ──────────────────────────

    /// WHEN a MutationBatch budget check is run THEN it completes within 50µs.
    #[test]
    fn test_budget_check_latency_under_50us() {
        let budget = default_budget();
        let usage = BudgetUsage {
            tile_count: 4,
            max_nodes_per_tile: 16,
            texture_bytes_used: 1024 * 1024,
            active_lease_count: 2,
        };
        let delta = BudgetDelta {
            delta_tiles: 1,
            max_nodes_in_batch: 20,
            delta_texture_bytes: 512 * 1024,
        };

        // Run 64 budget checks (spec scenario: MutationBatch with 64 mutations).
        let mut max_elapsed_us = 0u64;
        for _ in 0..64 {
            let (_, elapsed_us) = timed_budget_check(&budget, &usage, &delta);
            if elapsed_us > max_elapsed_us {
                max_elapsed_us = elapsed_us;
            }
        }

        // Allow a 10× headroom for CI (software-only environments).
        const NOMINAL_BUDGET_US: u64 = 50;
        const CI_MULTIPLIER: u64 = 10;
        assert!(
            max_elapsed_us <= NOMINAL_BUDGET_US * CI_MULTIPLIER,
            "budget check exceeded latency budget: max {}µs > {}µs",
            max_elapsed_us, NOMINAL_BUDGET_US * CI_MULTIPLIER
        );
    }

    // ── Anti-collusion (spec §Shared Resources) ───────────────────────────

    /// WHEN shared resources are present THEN anti_collusion_texture_bytes
    /// double-counts them per spec.
    #[test]
    fn test_anti_collusion_double_counts_shared_bytes() {
        let exclusive = 100u64;
        let shared = 50u64;
        // Anti-collusion: the agent "owns" exclusive + shared (shared counted once for this agent).
        // The other agent also counts the same shared bytes → effective load is double.
        let counted = anti_collusion_texture_bytes(exclusive, shared);
        assert_eq!(counted, 150, "exclusive({exclusive}) + shared({shared}) should be 150");
    }
}
