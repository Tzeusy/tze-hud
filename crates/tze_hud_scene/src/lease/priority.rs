//! Lease priority assignment, sort semantics, and tile shedding order.
//!
//! Implements:
//! - Requirement: Priority Assignment (lease-governance/spec.md lines 49-60)
//! - Requirement: Priority Sort Semantics (lease-governance/spec.md lines 62-69)
//! - Requirement: Tile Shedding Order (lease-governance/spec.md lines 271-278)
//!
//! ## Priority values
//! | Value | Meaning |
//! |-------|---------|
//! | 0     | System / chrome — reserved; agents MUST NOT request this |
//! | 1     | High priority — requires `lease:priority:1` capability |
//! | 2     | Normal (default) |
//! | 3     | Low |
//! | 4+    | Background |
//!
//! Numerically lower value = higher rendering priority (0 is highest).

use crate::types::Capability;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Priority reserved for system / chrome (runtime-internal only).
pub const PRIORITY_SYSTEM: u8 = 0;
/// Priority requiring `lease:priority:1` capability.
pub const PRIORITY_HIGH: u8 = 1;
/// Default priority granted to agents (spec line 50, "Priority 2 MUST be the default").
pub const PRIORITY_DEFAULT: u8 = 2;

// ─── Priority Assignment ─────────────────────────────────────────────────────

/// Clamp a requested priority according to the spec rules:
///
/// - Priority 0 → downgraded to 2 (system-reserved).
/// - Priority 1 → downgraded to 2 unless `capabilities` contains `Capability::LeasePriority1`.
/// - All other values → passed through as-is.
///
/// From spec §Requirement: Priority Assignment (lines 49-60):
/// > "An agent requesting priority 0 MUST receive priority 2.
/// >  An agent requesting priority 1 without the capability MUST receive priority 2."
pub fn clamp_requested_priority(requested: u8, capabilities: &[Capability]) -> u8 {
    match requested {
        PRIORITY_SYSTEM => {
            // Priority 0 is reserved for system/chrome — always downgrade.
            PRIORITY_DEFAULT
        }
        PRIORITY_HIGH => {
            // Priority 1 requires the lease:priority:1 capability.
            if capabilities.contains(&Capability::LeasePriority1) {
                PRIORITY_HIGH
            } else {
                PRIORITY_DEFAULT
            }
        }
        other => other,
    }
}

// ─── Sort Key ─────────────────────────────────────────────────────────────────

/// The compositor sort key for tiles: `(lease_priority ASC, z_order DESC)`.
///
/// Lower numeric `lease_priority` = higher rendering priority.
/// Within the same priority class, higher `z_order` wins.
///
/// From spec §Requirement: Priority Sort Semantics (lines 62-69).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileSortKey {
    /// Lease priority (lower = higher priority — renders on top).
    pub lease_priority: u8,
    /// Z-order within the priority class (higher = on top of peers with same priority).
    pub z_order: u32,
}

impl TileSortKey {
    /// Construct a sort key from lease priority and tile z-order.
    pub fn new(lease_priority: u8, z_order: u32) -> Self {
        TileSortKey {
            lease_priority,
            z_order,
        }
    }

    /// Returns `true` if `self` should render *above* `other`.
    ///
    /// A tile renders above another if its `lease_priority` is numerically lower,
    /// or, when priorities are equal, if its `z_order` is higher.
    pub fn renders_above(&self, other: &TileSortKey) -> bool {
        match self.lease_priority.cmp(&other.lease_priority) {
            std::cmp::Ordering::Less => true, // lower priority number = higher rendering priority
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => self.z_order > other.z_order,
        }
    }
}

/// Ordering for use with `sort_unstable_by_key`.
///
/// Sorts tiles from *highest* rendering priority to *lowest*, i.e. the tile that
/// should appear on top comes first.
///
/// Sort key: `(lease_priority ASC, z_order DESC)` per spec lines 62-69.
///
/// To get the natural ascending-first order for shed selection (least important
/// first), reverse the result or call `shed_order_key` instead.
impl PartialOrd for TileSortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TileSortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Primary: lease_priority ASC (lower priority number = renders above = sort earlier)
        // Secondary: z_order DESC (higher z_order = renders above within same priority)
        match self.lease_priority.cmp(&other.lease_priority) {
            std::cmp::Ordering::Equal => other.z_order.cmp(&self.z_order), // DESC
            ord => ord,                                                    // ASC
        }
    }
}

// ─── Shedding Order ───────────────────────────────────────────────────────────

/// A lightweight view of a tile used for shedding-order computation.
///
/// Callers pass a slice of `TileSheddingEntry` values; `shedding_order` returns
/// the indices of tiles that should be shed first (least important first).
#[derive(Clone, Debug)]
pub struct TileSheddingEntry {
    /// Unique tile identifier (opaque; passed back in the result).
    pub index: usize,
    pub key: TileSortKey,
}

impl TileSheddingEntry {
    pub fn new(index: usize, lease_priority: u8, z_order: u32) -> Self {
        TileSheddingEntry {
            index,
            key: TileSortKey::new(lease_priority, z_order),
        }
    }
}

/// Compute the shedding order for a set of active tiles at degradation Level 4.
///
/// Returns `count` tile indices in shedding order: **least important first**
/// (highest `lease_priority` value, then lowest `z_order`).
///
/// Per spec §Requirement: Tile Shedding Order (lines 271-278):
/// > "sort tiles by `(lease_priority ASC, z_order DESC)` and remove approximately
/// >  25% of active tiles per application. Shed tiles remain in the scene graph;
/// >  their leases are not revoked."
///
/// `count` is typically `ceil(tiles.len() / 4)` (≈25%) — the caller decides the
/// exact number.  This function is pure: it does **not** modify any state.
pub fn shedding_order(tiles: &[TileSheddingEntry], count: usize) -> Vec<usize> {
    // Sort ascending by (lease_priority ASC, z_order DESC): tile indices at the
    // *end* of this order are the least important and should be shed first.
    let mut sorted: Vec<&TileSheddingEntry> = tiles.iter().collect();
    // Sort so that the LEAST important tiles come FIRST.
    // Least important = highest lease_priority value (numerically), then lowest z_order.
    //   primary:   lease_priority DESC (b cmp a → DESC; highest value = least important)
    //   secondary: z_order ASC        (a cmp b → ASC;  lowest z_order = less important)
    sorted.sort_by(
        |a, b| match b.key.lease_priority.cmp(&a.key.lease_priority) {
            std::cmp::Ordering::Equal => a.key.z_order.cmp(&b.key.z_order),
            ord => ord,
        },
    );

    // Take the first `count` entries: they are the least important tiles to shed.
    sorted.iter().take(count).map(|e| e.index).collect()
}

/// Compute the shed count for ~25% of `total` tiles (rounded up, min 0).
pub fn shed_count_for_level4(total: usize) -> usize {
    (total + 3) / 4
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Capability;

    // ── Priority clamping ────────────────────────────────────────────────────

    /// WHEN an agent requests priority 0 THEN it receives priority 2.
    #[test]
    fn priority_0_downgraded_to_2() {
        assert_eq!(clamp_requested_priority(0, &[]), PRIORITY_DEFAULT);
        // Even with the high-priority capability, 0 is always downgraded.
        assert_eq!(
            clamp_requested_priority(0, &[Capability::LeasePriority1]),
            PRIORITY_DEFAULT
        );
    }

    /// WHEN an agent requests priority 1 without `lease:priority:1` THEN it receives priority 2.
    #[test]
    fn priority_1_without_capability_downgraded_to_2() {
        assert_eq!(clamp_requested_priority(1, &[]), PRIORITY_DEFAULT);
        assert_eq!(
            clamp_requested_priority(1, &[Capability::CreateTiles]),
            PRIORITY_DEFAULT
        );
    }

    /// WHEN an agent requests priority 1 WITH `lease:priority:1` THEN it receives priority 1.
    #[test]
    fn priority_1_with_capability_granted() {
        assert_eq!(
            clamp_requested_priority(1, &[Capability::LeasePriority1]),
            PRIORITY_HIGH
        );
    }

    /// WHEN an agent requests priority 2 THEN it receives priority 2 (no change).
    #[test]
    fn priority_2_passes_through() {
        assert_eq!(clamp_requested_priority(2, &[]), PRIORITY_DEFAULT);
    }

    /// WHEN an agent requests priority 3 THEN it receives priority 3 (no change).
    #[test]
    fn priority_3_passes_through() {
        assert_eq!(clamp_requested_priority(3, &[]), 3u8);
    }

    // ── Sort key ordering ────────────────────────────────────────────────────

    /// Lower lease_priority renders above higher lease_priority.
    #[test]
    fn sort_key_lower_priority_renders_above() {
        let high = TileSortKey::new(1, 5);
        let normal = TileSortKey::new(2, 5);
        assert!(high.renders_above(&normal));
        assert!(!normal.renders_above(&high));
    }

    /// Same priority: higher z_order renders above.
    #[test]
    fn sort_key_same_priority_higher_z_wins() {
        let top = TileSortKey::new(2, 10);
        let bottom = TileSortKey::new(2, 5);
        assert!(top.renders_above(&bottom));
        assert!(!bottom.renders_above(&top));
    }

    /// Ord: keys sort ascending by (lease_priority ASC, z_order DESC).
    #[test]
    fn sort_key_ord_ascending() {
        let mut keys = vec![
            TileSortKey::new(3, 1),  // least important
            TileSortKey::new(1, 10), // most important
            TileSortKey::new(2, 5),  // middle
        ];
        keys.sort();
        assert_eq!(keys[0], TileSortKey::new(1, 10)); // highest-priority tile first
        assert_eq!(keys[2], TileSortKey::new(3, 1)); // lowest-priority tile last
    }

    // ── Shedding order ───────────────────────────────────────────────────────

    /// WHEN degradation requires tile shedding THEN least-important tiles shed first.
    ///
    /// Spec scenario (lines 67-69):
    /// "WHEN the degradation ladder requires tile shedding
    ///  THEN tiles with the highest lease_priority values (least important) and
    ///  lowest z_order values are shed first."
    #[test]
    fn shedding_order_least_important_first() {
        let tiles = vec![
            TileSheddingEntry::new(0, 1, 10), // high-prio, high-z — most important
            TileSheddingEntry::new(1, 2, 5),  // normal prio, mid-z
            TileSheddingEntry::new(2, 3, 1),  // low-prio, low-z — least important
        ];
        let shed = shedding_order(&tiles, 1);
        assert_eq!(
            shed,
            vec![2],
            "tile index 2 (priority=3, z=1) should shed first"
        );
    }

    /// With equal priorities, lower z_order is shed first.
    #[test]
    fn shedding_order_equal_priority_lower_z_first() {
        let tiles = vec![
            TileSheddingEntry::new(0, 2, 10),
            TileSheddingEntry::new(1, 2, 1),
        ];
        let shed = shedding_order(&tiles, 1);
        assert_eq!(shed, vec![1], "tile with z=1 should shed before z=10");
    }

    /// shed_count_for_level4: approximately 25% rounded up.
    #[test]
    fn shed_count_25_percent() {
        assert_eq!(shed_count_for_level4(4), 1);
        assert_eq!(shed_count_for_level4(8), 2);
        assert_eq!(shed_count_for_level4(3), 1);
        assert_eq!(shed_count_for_level4(0), 0);
    }

    /// Shedding empty list returns empty.
    #[test]
    fn shedding_order_empty() {
        let shed = shedding_order(&[], 0);
        assert!(shed.is_empty());
    }

    /// Three-agent contention scenario from the spec (lines 67-69):
    /// agents at priority 1/2/3, z-orders 10/5/1.
    /// Priority-1 tile (high-prio, z=10) must be the LAST shed.
    #[test]
    fn shedding_order_three_agents_contention() {
        let tiles = vec![
            TileSheddingEntry::new(0, 1, 10), // agent.high_prio
            TileSheddingEntry::new(1, 2, 5),  // agent.normal_prio
            TileSheddingEntry::new(2, 3, 1),  // agent.low_prio
        ];
        // Shed 1 tile (~ 33% of 3) — least important is priority=3, z=1.
        let shed = shedding_order(&tiles, 1);
        assert_eq!(shed, vec![2], "low_prio tile (priority=3, z=1) sheds first");

        // Shed 2 tiles — after low_prio, normal_prio sheds next.
        let shed2 = shedding_order(&tiles, 2);
        assert_eq!(shed2[0], 2, "low_prio sheds first");
        assert_eq!(shed2[1], 1, "normal_prio sheds second");

        // High-prio tile (index 0) must never be in a 2-tile shed from 3.
        assert!(
            !shed2.contains(&0),
            "high_prio tile must not shed before low and normal"
        );
    }
}
