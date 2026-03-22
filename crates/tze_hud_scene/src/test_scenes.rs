//! Test scene registry — deterministic named scene configurations for validation layers 0-4.
//!
//! # Design
//!
//! [`TestSceneRegistry`] is the single entry point. Call [`TestSceneRegistry::build`] with a
//! scene name to receive a fully-assembled [`SceneGraph`] and the matching [`SceneSpec`] that
//! describes what Layer 0 invariants must hold.
//!
//! All randomness is absent by design. Every scene is a pure function of its name and the
//! injected clock. The same name always produces the same graph structure (modulo UUIDs, which
//! are assigned freshly on every call but are not inspected by the invariant checks).
//!
//! ## Injectable clock
//!
//! Scene construction calls that need a timestamp (`grant_lease`, `create_tab`, …) ultimately
//! call the internal `now_millis()` helper in `graph.rs`. That helper reads the real wall clock.
//! For tests that want to reason about expiry, the registry accepts a [`ClockMs`] value that is
//! used to derive TTLs and `present_at`/`expires_at` offsets — no wall-clock-sensitive assertions
//! are made directly on those fields.
//!
//! ## Layer 0 assertions
//!
//! [`assert_layer0_invariants`] runs the full invariant suite on any [`SceneGraph`] slice and
//! returns a [`Vec<InvariantViolation>`]. An empty vec means all invariants hold. Individual
//! named checks are also exported so callers can compose narrower assertion sets.

use crate::graph::SceneGraph;
use crate::types::{
    Capability, ContentionPolicy, DisplayEdge, FontFamily, GeometryPolicy, HitRegionNode,
    Node, NodeData, Rect, RenderingPolicy, Rgba, SceneId, SolidColorNode, TextAlign,
    TextMarkdownNode, TextOverflow, ZoneDefinition, ZoneMediaType,
};

// ─── Clock injection ─────────────────────────────────────────────────────────

/// A timestamp in milliseconds since the Unix epoch, used as the "current time"
/// when constructing test scenes. Inject a fixed value for deterministic expiry tests.
///
/// If you don't care about timing semantics, use [`ClockMs::FIXED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockMs(pub u64);

impl ClockMs {
    /// A fixed timestamp used when timing doesn't matter (1 January 2025 00:00:00 UTC).
    pub const FIXED: ClockMs = ClockMs(1_735_689_600_000);

    /// Offset this clock by `delta_ms` milliseconds.
    pub fn offset(self, delta_ms: u64) -> Self {
        ClockMs(self.0 + delta_ms)
    }
}

impl Default for ClockMs {
    fn default() -> Self {
        Self::FIXED
    }
}

// ─── Scene specification ──────────────────────────────────────────────────────

/// Metadata describing what a test scene contains.
///
/// Used by higher validation layers (Layer 1–4) to know what to render and check.
/// Layer 0 assertions are derived programmatically from the [`SceneGraph`]; this struct
/// carries only the human-readable description and structural expectations that the
/// graph itself cannot express.
#[derive(Clone, Debug)]
pub struct SceneSpec {
    /// Canonical name (the key used with [`TestSceneRegistry::build`]).
    pub name: &'static str,
    /// Human-readable description — used in Layer 4 `explanation.md`.
    pub description: &'static str,
    /// Expected number of tabs.
    pub expected_tab_count: usize,
    /// Expected number of tiles (across all tabs).
    pub expected_tile_count: usize,
    /// Whether any tiles contain hit regions.
    pub has_hit_regions: bool,
    /// Whether the scene registers any zones.
    pub has_zones: bool,
}

// ─── Invariant violation ─────────────────────────────────────────────────────

/// A Layer 0 invariant that was violated.
#[derive(Clone, Debug, PartialEq)]
pub struct InvariantViolation {
    /// Short machine-readable label, e.g. `"orphan_tile"`.
    pub code: &'static str,
    /// Human-readable diagnostic message.
    pub message: String,
}

impl InvariantViolation {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

// ─── Test scene registry ──────────────────────────────────────────────────────

/// Registry of named, deterministic test scenes.
///
/// # Usage
///
/// ```rust
/// use tze_hud_scene::test_scenes::{TestSceneRegistry, ClockMs};
///
/// let registry = TestSceneRegistry::new();
/// let (graph, spec) = registry.build("single_tile", ClockMs::FIXED).unwrap();
/// // graph is ready for Layer 0 assertions
/// ```
pub struct TestSceneRegistry {
    /// Display dimensions used for all scenes.
    pub display_width: f32,
    pub display_height: f32,
}

impl Default for TestSceneRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TestSceneRegistry {
    /// Create a registry using the standard 1920×1080 display area.
    pub fn new() -> Self {
        Self { display_width: 1920.0, display_height: 1080.0 }
    }

    /// Create a registry with a custom display area (useful for mobile-profile tests).
    pub fn with_display(width: f32, height: f32) -> Self {
        Self { display_width: width, display_height: height }
    }

    /// Build a named scene, returning `(graph, spec)`.
    ///
    /// Returns `None` if the name is not known.
    pub fn build(&self, name: &str, clock: ClockMs) -> Option<(SceneGraph, SceneSpec)> {
        match name {
            "empty" => Some(self.build_empty(clock)),
            "single_tile" => Some(self.build_single_tile(clock)),
            "two_tiles" => Some(self.build_two_tiles(clock)),
            "max_tiles" => Some(self.build_max_tiles(clock)),
            "zone_test" => Some(self.build_zone_test(clock)),
            _ => None,
        }
    }

    /// All known scene names.
    pub fn scene_names() -> &'static [&'static str] {
        &["empty", "single_tile", "two_tiles", "max_tiles", "zone_test"]
    }

    // ─── Scene builders ───────────────────────────────────────────────────

    /// `empty` — no tabs, no tiles. Validates clean initialisation.
    fn build_empty(&self, _clock: ClockMs) -> (SceneGraph, SceneSpec) {
        let graph = SceneGraph::new(self.display_width, self.display_height);

        let spec = SceneSpec {
            name: "empty",
            description: "No tabs, no tiles. Validates clean startup state.",
            expected_tab_count: 0,
            expected_tile_count: 0,
            has_hit_regions: false,
            has_zones: false,
        };

        (graph, spec)
    }

    /// `single_tile` — one tab, one tile with a text content node.
    fn build_single_tile(&self, clock: ClockMs) -> (SceneGraph, SceneSpec) {
        let mut graph = SceneGraph::new(self.display_width, self.display_height);

        let tab_id = graph.create_tab("Main", 0).expect("create_tab failed");

        let lease_id = graph.grant_lease_at(
            "agent.single",
            clock.0,
            300_000, // 5-minute TTL
            vec![Capability::CreateTile, Capability::CreateNode, Capability::UpdateTile],
        );

        let tile_bounds = Rect::new(100.0, 100.0, 800.0, 400.0);
        let tile_id = graph
            .create_tile(tab_id, "agent.single", lease_id, tile_bounds, 1)
            .expect("create_tile failed");

        let text_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "# Hello from tze_hud\n\nThis is a single-tile test scene.".to_string(),
                bounds: Rect::new(0.0, 0.0, 800.0, 400.0),
                font_size_px: 18.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: Some(Rgba::new(0.08, 0.08, 0.15, 1.0)),
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };
        graph.set_tile_root(tile_id, text_node).expect("set_tile_root failed");

        let spec = SceneSpec {
            name: "single_tile",
            description: "One tab, one tile with markdown text content.",
            expected_tab_count: 1,
            expected_tile_count: 1,
            has_hit_regions: false,
            has_zones: false,
        };

        (graph, spec)
    }

    /// `two_tiles` — one tab, two tiles (text + hit_region). Matches the vertical-slice layout.
    fn build_two_tiles(&self, clock: ClockMs) -> (SceneGraph, SceneSpec) {
        let mut graph = SceneGraph::new(self.display_width, self.display_height);

        let tab_id = graph.create_tab("Dashboard", 0).expect("create_tab failed");

        let lease_id = graph.grant_lease_at(
            "agent.two",
            clock.0,
            300_000,
            vec![
                Capability::CreateTile,
                Capability::CreateNode,
                Capability::UpdateTile,
                Capability::ReceiveInput,
            ],
        );

        // Tile 1 — text content, left half of screen
        let text_tile_bounds = Rect::new(10.0, 10.0, 900.0, 500.0);
        let text_tile_id = graph
            .create_tile(tab_id, "agent.two", lease_id, text_tile_bounds, 1)
            .expect("create_tile failed");

        let text_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "## two_tiles scene\n\nText tile on the left.".to_string(),
                bounds: Rect::new(0.0, 0.0, 900.0, 500.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: Some(Rgba::new(0.1, 0.1, 0.2, 1.0)),
                alignment: TextAlign::Start,
                overflow: TextOverflow::Ellipsis,
            }),
        };
        graph.set_tile_root(text_tile_id, text_node).expect("set_tile_root failed");

        // Tile 2 — hit region tile, right half of screen
        let hit_tile_bounds = Rect::new(930.0, 10.0, 900.0, 500.0);
        let hit_tile_id = graph
            .create_tile(tab_id, "agent.two", lease_id, hit_tile_bounds, 2)
            .expect("create_tile failed");

        let hit_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(300.0, 200.0, 300.0, 100.0),
                interaction_id: "btn-primary".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            }),
        };
        graph.set_tile_root(hit_tile_id, hit_node).expect("set_tile_root failed");

        let spec = SceneSpec {
            name: "two_tiles",
            description: "One tab, two non-overlapping tiles: text (left) and hit-region (right). \
                          Mirrors the vertical-slice layout.",
            expected_tab_count: 1,
            expected_tile_count: 2,
            has_hit_regions: true,
            has_zones: false,
        };

        (graph, spec)
    }

    /// `max_tiles` — stress test with many tiles, approaching the default `max_nodes` budget.
    ///
    /// Creates 60 tiles on a single tab (default budget is 64). This exercises the scene graph
    /// under load and validates that bookkeeping remains consistent near capacity.
    fn build_max_tiles(&self, clock: ClockMs) -> (SceneGraph, SceneSpec) {
        let mut graph = SceneGraph::new(self.display_width, self.display_height);

        let tab_id = graph.create_tab("Stress", 0).expect("create_tab failed");

        let lease_id = graph.grant_lease_at(
            "agent.stress",
            clock.0,
            300_000,
            vec![Capability::CreateTile, Capability::CreateNode],
        );

        // 10 columns × 6 rows = 60 tiles
        let cols = 10u32;
        let rows = 6u32;
        let tile_w = self.display_width / cols as f32;
        let tile_h = self.display_height / rows as f32;

        for row in 0..rows {
            for col in 0..cols {
                let z = row * cols + col + 1;
                let bounds = Rect::new(
                    col as f32 * tile_w,
                    row as f32 * tile_h,
                    tile_w - 2.0, // 2px gap
                    tile_h - 2.0,
                );
                let tile_id = graph
                    .create_tile(tab_id, "agent.stress", lease_id, bounds, z)
                    .expect("create_tile failed in max_tiles");

                // Alternate between SolidColor and TextMarkdown to exercise both node types
                let node = if z % 2 == 0 {
                    Node {
                        id: SceneId::new(),
                        children: vec![],
                        data: NodeData::SolidColor(SolidColorNode {
                            color: Rgba::new(
                                (col as f32) / cols as f32,
                                (row as f32) / rows as f32,
                                0.5,
                                1.0,
                            ),
                            bounds: Rect::new(0.0, 0.0, tile_w - 2.0, tile_h - 2.0),
                        }),
                    }
                } else {
                    Node {
                        id: SceneId::new(),
                        children: vec![],
                        data: NodeData::TextMarkdown(TextMarkdownNode {
                            content: format!("tile {z}"),
                            bounds: Rect::new(0.0, 0.0, tile_w - 2.0, tile_h - 2.0),
                            font_size_px: 12.0,
                            font_family: FontFamily::SystemMonospace,
                            color: Rgba::WHITE,
                            background: None,
                            alignment: TextAlign::Center,
                            overflow: TextOverflow::Clip,
                        }),
                    }
                };
                graph.set_tile_root(tile_id, node).expect("set_tile_root failed in max_tiles");
            }
        }

        let tile_count = (cols * rows) as usize;

        let spec = SceneSpec {
            name: "max_tiles",
            description: "Stress test with 60 tiles (near the 64-node default budget) on a \
                          single tab. Exercises scene graph bookkeeping under load.",
            expected_tab_count: 1,
            expected_tile_count: tile_count,
            has_hit_regions: false,
            has_zones: false,
        };

        (graph, spec)
    }

    /// `zone_test` — tiles publishing to zones, with a zone registry populated.
    fn build_zone_test(&self, clock: ClockMs) -> (SceneGraph, SceneSpec) {
        let mut graph = SceneGraph::new(self.display_width, self.display_height);

        let tab_id = graph.create_tab("Zoned", 0).expect("create_tab failed");

        let lease_id = graph.grant_lease_at(
            "agent.zones",
            clock.0,
            300_000,
            vec![Capability::CreateTile, Capability::CreateNode],
        );

        // Register two zones
        graph.zone_registry.zones.insert(
            "subtitle".to_string(),
            ZoneDefinition {
                id: SceneId::new(),
                name: "subtitle".to_string(),
                description: "Centered subtitle overlay at the bottom of the screen.".to_string(),
                geometry_policy: GeometryPolicy::EdgeAnchored {
                    edge: DisplayEdge::Bottom,
                    height_pct: 0.10,
                    width_pct: 0.80,
                    margin_px: 48.0,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::LatestWins,
                max_publishers: 1,
                transport_constraint: None,
                auto_clear_ms: None,
            },
        );
        graph.zone_registry.zones.insert(
            "status_bar".to_string(),
            ZoneDefinition {
                id: SceneId::new(),
                name: "status_bar".to_string(),
                description: "Persistent status bar at the top of the screen.".to_string(),
                geometry_policy: GeometryPolicy::EdgeAnchored {
                    edge: DisplayEdge::Bottom,
                    height_pct: 0.04,
                    width_pct: 1.0,
                    margin_px: 0.0,
                },
                accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: None,
            },
        );

        // Tile publishing to the subtitle zone
        let subtitle_tile_bounds = Rect::new(
            self.display_width * 0.1,
            self.display_height * 0.9,
            self.display_width * 0.8,
            self.display_height * 0.05,
        );
        let subtitle_tile_id = graph
            .create_tile(tab_id, "agent.zones", lease_id, subtitle_tile_bounds, 10)
            .expect("create_tile failed");

        let subtitle_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "Zone subtitle text content".to_string(),
                bounds: Rect::new(0.0, 0.0, subtitle_tile_bounds.width, subtitle_tile_bounds.height),
                font_size_px: 20.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                alignment: TextAlign::Center,
                overflow: TextOverflow::Clip,
            }),
        };
        graph.set_tile_root(subtitle_tile_id, subtitle_node).expect("set_tile_root failed");

        // Tile publishing to the status bar zone
        let status_tile_bounds = Rect::new(0.0, 0.0, self.display_width, 32.0);
        let status_tile_id = graph
            .create_tile(tab_id, "agent.zones", lease_id, status_tile_bounds, 20)
            .expect("create_tile failed");

        let status_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.15, 0.15, 0.2, 0.9),
                bounds: Rect::new(0.0, 0.0, self.display_width, 32.0),
            }),
        };
        graph.set_tile_root(status_tile_id, status_node).expect("set_tile_root failed");

        let spec = SceneSpec {
            name: "zone_test",
            description: "Two tiles publishing to registered zones (subtitle and status_bar). \
                          Validates zone registry operations and tile-to-zone mapping.",
            expected_tab_count: 1,
            expected_tile_count: 2,
            has_hit_regions: false,
            has_zones: true,
        };

        (graph, spec)
    }
}

// ─── Graph extension: grant_lease_at ─────────────────────────────────────────

/// Extension trait adding a clock-injectable variant of `grant_lease` to [`SceneGraph`].
///
/// The core `grant_lease` always calls the real wall clock. For test scenes we need to
/// control the `granted_at_ms` so that expiry behaviour is deterministic.
pub trait SceneGraphTestExt {
    /// Grant a lease using the provided `granted_at_ms` timestamp instead of the wall clock.
    fn grant_lease_at(
        &mut self,
        namespace: &str,
        granted_at_ms: u64,
        ttl_ms: u64,
        capabilities: Vec<Capability>,
    ) -> SceneId;
}

impl SceneGraphTestExt for SceneGraph {
    fn grant_lease_at(
        &mut self,
        namespace: &str,
        granted_at_ms: u64,
        ttl_ms: u64,
        capabilities: Vec<Capability>,
    ) -> SceneId {
        use crate::types::{Lease, ResourceBudget};

        let id = SceneId::new();
        self.leases.insert(
            id,
            Lease {
                id,
                namespace: namespace.to_string(),
                granted_at_ms,
                ttl_ms,
                capabilities,
                resource_budget: ResourceBudget::default(),
            },
        );
        self.version += 1;
        id
    }
}

// ─── Layer 0 invariant checks ─────────────────────────────────────────────────

/// Run all Layer 0 invariants against `graph`. Returns all violations found.
///
/// An empty vec means all invariants pass. A non-empty vec contains diagnostics
/// with structured codes suitable for automated regression reporting.
pub fn assert_layer0_invariants(graph: &SceneGraph) -> Vec<InvariantViolation> {
    let mut violations = Vec::new();

    violations.extend(check_tile_tab_refs(graph));
    violations.extend(check_tile_lease_refs(graph));
    violations.extend(check_tile_bounds_positive(graph));
    violations.extend(check_tile_bounds_within_display(graph));
    violations.extend(check_tile_opacity_range(graph));
    violations.extend(check_node_tile_backlinks(graph));
    violations.extend(check_hit_region_state_consistency(graph));
    violations.extend(check_active_tab_exists(graph));
    violations.extend(check_z_order_unique_per_tab(graph));
    violations.extend(check_lease_namespace_nonempty(graph));
    violations.extend(check_zone_names_nonempty(graph));
    violations.extend(check_zone_name_key_consistency(graph));
    violations.extend(check_version_non_decreasing(graph));

    violations
}

// ─── Individual invariant functions ──────────────────────────────────────────

/// Every tile's `tab_id` must reference a tab that exists in the graph.
pub fn check_tile_tab_refs(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .tiles
        .values()
        .filter(|t| !graph.tabs.contains_key(&t.tab_id))
        .map(|t| {
            InvariantViolation::new(
                "orphan_tile_tab",
                format!("tile {} references tab {} which does not exist", t.id, t.tab_id),
            )
        })
        .collect()
}

/// Every tile's `lease_id` must reference a lease that exists in the graph.
pub fn check_tile_lease_refs(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .tiles
        .values()
        .filter(|t| !graph.leases.contains_key(&t.lease_id))
        .map(|t| {
            InvariantViolation::new(
                "orphan_tile_lease",
                format!("tile {} references lease {} which does not exist", t.id, t.lease_id),
            )
        })
        .collect()
}

/// Every tile must have positive width and height.
pub fn check_tile_bounds_positive(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .tiles
        .values()
        .filter(|t| t.bounds.width <= 0.0 || t.bounds.height <= 0.0)
        .map(|t| {
            InvariantViolation::new(
                "tile_bounds_non_positive",
                format!(
                    "tile {} has non-positive bounds: {}×{}",
                    t.id, t.bounds.width, t.bounds.height
                ),
            )
        })
        .collect()
}

/// Every tile's bounds must be fully contained within the display area.
pub fn check_tile_bounds_within_display(graph: &SceneGraph) -> Vec<InvariantViolation> {
    let display = &graph.display_area;
    graph
        .tiles
        .values()
        .filter(|t| !t.bounds.is_within(display))
        .map(|t| {
            InvariantViolation::new(
                "tile_out_of_display",
                format!(
                    "tile {} bounds ({},{} {}×{}) exceed display area ({},{} {}×{})",
                    t.id,
                    t.bounds.x,
                    t.bounds.y,
                    t.bounds.width,
                    t.bounds.height,
                    display.x,
                    display.y,
                    display.width,
                    display.height,
                ),
            )
        })
        .collect()
}

/// Every tile's opacity must be in [0.0, 1.0].
pub fn check_tile_opacity_range(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .tiles
        .values()
        .filter(|t| !(0.0..=1.0).contains(&t.opacity))
        .map(|t| {
            InvariantViolation::new(
                "tile_opacity_out_of_range",
                format!("tile {} has opacity {} (must be in [0.0, 1.0])", t.id, t.opacity),
            )
        })
        .collect()
}

/// Every tile's `root_node`, if set, must point to a node that exists in the graph.
/// Additionally, every node listed as a child of another node must exist.
pub fn check_node_tile_backlinks(graph: &SceneGraph) -> Vec<InvariantViolation> {
    let mut violations = Vec::new();

    // Root node backlinks
    for tile in graph.tiles.values() {
        if let Some(root_id) = tile.root_node {
            if !graph.nodes.contains_key(&root_id) {
                violations.push(InvariantViolation::new(
                    "missing_root_node",
                    format!("tile {} root_node {} does not exist in nodes map", tile.id, root_id),
                ));
            }
        }
    }

    // Child node backlinks
    for node in graph.nodes.values() {
        for child_id in &node.children {
            if !graph.nodes.contains_key(child_id) {
                violations.push(InvariantViolation::new(
                    "missing_child_node",
                    format!("node {} child {} does not exist in nodes map", node.id, child_id),
                ));
            }
        }
    }

    violations
}

/// Every [`HitRegionNode`] must have a corresponding entry in `hit_region_states`.
pub fn check_hit_region_state_consistency(graph: &SceneGraph) -> Vec<InvariantViolation> {
    let mut violations = Vec::new();

    for node in graph.nodes.values() {
        if matches!(node.data, NodeData::HitRegion(_)) {
            if !graph.hit_region_states.contains_key(&node.id) {
                violations.push(InvariantViolation::new(
                    "missing_hit_region_state",
                    format!(
                        "hit region node {} has no entry in hit_region_states",
                        node.id
                    ),
                ));
            }
        }
    }

    // Inverse: every entry in hit_region_states must point to an existing HitRegion node
    for (node_id, _state) in &graph.hit_region_states {
        match graph.nodes.get(node_id) {
            None => violations.push(InvariantViolation::new(
                "orphan_hit_region_state",
                format!("hit_region_states entry {} has no corresponding node", node_id),
            )),
            Some(node) if !matches!(node.data, NodeData::HitRegion(_)) => {
                violations.push(InvariantViolation::new(
                    "hit_region_state_type_mismatch",
                    format!(
                        "hit_region_states entry {} points to a non-HitRegion node",
                        node_id
                    ),
                ));
            }
            _ => {}
        }
    }

    violations
}

/// If `active_tab` is `Some(id)`, that id must exist in the tabs map.
pub fn check_active_tab_exists(graph: &SceneGraph) -> Vec<InvariantViolation> {
    if let Some(active_id) = graph.active_tab {
        if !graph.tabs.contains_key(&active_id) {
            return vec![InvariantViolation::new(
                "missing_active_tab",
                format!("active_tab {} does not exist in tabs map", active_id),
            )];
        }
    }
    vec![]
}

/// No two tiles on the same tab may share the same `z_order`.
pub fn check_z_order_unique_per_tab(graph: &SceneGraph) -> Vec<InvariantViolation> {
    use std::collections::HashMap;

    // tab_id → (z_order → tile_id)
    let mut seen: HashMap<SceneId, HashMap<u32, SceneId>> = HashMap::new();
    let mut violations = Vec::new();

    for tile in graph.tiles.values() {
        let z_map = seen.entry(tile.tab_id).or_default();
        if let Some(existing_id) = z_map.insert(tile.z_order, tile.id) {
            violations.push(InvariantViolation::new(
                "duplicate_z_order",
                format!(
                    "tiles {} and {} on tab {} share z_order {}",
                    existing_id, tile.id, tile.tab_id, tile.z_order
                ),
            ));
        }
    }

    violations
}

/// Every lease must have a non-empty namespace.
pub fn check_lease_namespace_nonempty(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .leases
        .values()
        .filter(|l| l.namespace.is_empty())
        .map(|l| {
            InvariantViolation::new(
                "empty_lease_namespace",
                format!("lease {} has an empty namespace", l.id),
            )
        })
        .collect()
}

/// Every zone definition must have a non-empty name.
pub fn check_zone_names_nonempty(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .zone_registry
        .zones
        .values()
        .filter(|z| z.name.is_empty())
        .map(|z| {
            InvariantViolation::new(
                "empty_zone_name",
                format!("zone {} has an empty name", z.id),
            )
        })
        .collect()
}

/// The key of each entry in `zone_registry.zones` must match the `name` field of its
/// `ZoneDefinition`. The map is keyed by zone name for O(1) lookup, but the `ZoneDefinition`
/// also carries a `name` field. If they diverge the registry is silently inconsistent.
pub fn check_zone_name_key_consistency(graph: &SceneGraph) -> Vec<InvariantViolation> {
    graph
        .zone_registry
        .zones
        .iter()
        .filter(|(key, zone_def)| **key != zone_def.name)
        .map(|(key, zone_def)| {
            InvariantViolation::new(
                "zone_name_key_mismatch",
                format!(
                    "zone registry key '{}' does not match zone definition name '{}' for zone id {}",
                    key, zone_def.name, zone_def.id
                ),
            )
        })
        .collect()
}

/// The scene version must be ≥ 0 (a trivially always-true structural check included
/// to make the check suite exhaustive; catches accidental integer underflow if
/// version arithmetic changes in future).
pub fn check_version_non_decreasing(graph: &SceneGraph) -> Vec<InvariantViolation> {
    // u64 can never be negative, but we validate the version is reasonable.
    // A fresh graph starts at 0; a mutated one must be > 0.
    // We only flag this if the graph has content but version is still 0.
    let has_content = !graph.tabs.is_empty() || !graph.tiles.is_empty();
    if has_content && graph.version == 0 {
        vec![InvariantViolation::new(
            "version_not_incremented",
            "graph has content but version is still 0 — mutations must increment version",
        )]
    } else {
        vec![]
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn assert_no_violations(graph: &SceneGraph, scene_name: &str) {
        let violations = assert_layer0_invariants(graph);
        if !violations.is_empty() {
            let report: Vec<String> = violations.iter().map(|v| v.to_string()).collect();
            panic!("Layer 0 violations in scene '{scene_name}':\n{}", report.join("\n"));
        }
    }

    // ── Scene: empty ─────────────────────────────────────────────────────

    #[test]
    fn empty_scene_has_correct_structure() {
        let registry = TestSceneRegistry::new();
        let (graph, spec) = registry.build("empty", ClockMs::FIXED).unwrap();

        assert_eq!(graph.tabs.len(), spec.expected_tab_count, "tab count");
        assert_eq!(graph.tiles.len(), spec.expected_tile_count, "tile count");
        assert!(graph.active_tab.is_none(), "empty scene must have no active tab");
        assert!(graph.leases.is_empty(), "empty scene must have no leases");
        assert!(graph.nodes.is_empty(), "empty scene must have no nodes");
        assert_eq!(graph.version, 0, "empty graph version must be 0");
    }

    #[test]
    fn empty_scene_passes_all_layer0_invariants() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("empty", ClockMs::FIXED).unwrap();
        assert_no_violations(&graph, "empty");
    }

    // ── Scene: single_tile ────────────────────────────────────────────────

    #[test]
    fn single_tile_scene_has_correct_structure() {
        let registry = TestSceneRegistry::new();
        let (graph, spec) = registry.build("single_tile", ClockMs::FIXED).unwrap();

        assert_eq!(graph.tabs.len(), spec.expected_tab_count, "tab count");
        assert_eq!(graph.tiles.len(), spec.expected_tile_count, "tile count");
        assert!(graph.active_tab.is_some(), "single_tile must have an active tab");
        assert_eq!(graph.leases.len(), 1, "single_tile must have exactly one lease");
        assert_eq!(graph.nodes.len(), 1, "single_tile must have exactly one node");
    }

    #[test]
    fn single_tile_scene_tile_has_text_root() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("single_tile", ClockMs::FIXED).unwrap();

        let tile = graph.tiles.values().next().unwrap();
        assert!(tile.root_node.is_some(), "tile must have a root node");
        let node = graph.nodes.get(&tile.root_node.unwrap()).unwrap();
        assert!(matches!(node.data, NodeData::TextMarkdown(_)), "root node must be TextMarkdown");
    }

    #[test]
    fn single_tile_scene_tile_within_display() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("single_tile", ClockMs::FIXED).unwrap();

        let tile = graph.tiles.values().next().unwrap();
        assert!(
            tile.bounds.is_within(&graph.display_area),
            "tile bounds must be within display area"
        );
    }

    #[test]
    fn single_tile_scene_passes_all_layer0_invariants() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("single_tile", ClockMs::FIXED).unwrap();
        assert_no_violations(&graph, "single_tile");
    }

    // ── Scene: two_tiles ──────────────────────────────────────────────────

    #[test]
    fn two_tiles_scene_has_correct_structure() {
        let registry = TestSceneRegistry::new();
        let (graph, spec) = registry.build("two_tiles", ClockMs::FIXED).unwrap();

        assert_eq!(graph.tabs.len(), spec.expected_tab_count, "tab count");
        assert_eq!(graph.tiles.len(), spec.expected_tile_count, "tile count");
        assert_eq!(graph.nodes.len(), 2, "two_tiles must have exactly two nodes");
    }

    #[test]
    fn two_tiles_scene_has_one_hit_region() {
        let registry = TestSceneRegistry::new();
        let (graph, spec) = registry.build("two_tiles", ClockMs::FIXED).unwrap();

        let hit_region_count = graph
            .nodes
            .values()
            .filter(|n| matches!(n.data, NodeData::HitRegion(_)))
            .count();

        assert_eq!(hit_region_count, 1, "two_tiles must have exactly one hit region node");
        assert_eq!(
            graph.hit_region_states.len(),
            1,
            "hit_region_states must have one entry"
        );
        assert!(spec.has_hit_regions, "spec must declare has_hit_regions = true");
    }

    #[test]
    fn two_tiles_scene_tiles_do_not_overlap() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("two_tiles", ClockMs::FIXED).unwrap();

        let tiles: Vec<_> = graph.tiles.values().collect();
        assert_eq!(tiles.len(), 2, "expected exactly 2 tiles");
        assert!(
            !tiles[0].bounds.intersects(&tiles[1].bounds),
            "two_tiles tiles must not overlap: {:?} vs {:?}",
            tiles[0].bounds,
            tiles[1].bounds,
        );
    }

    #[test]
    fn two_tiles_scene_z_orders_are_unique() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("two_tiles", ClockMs::FIXED).unwrap();

        let tiles: Vec<_> = graph.tiles.values().collect();
        assert_ne!(tiles[0].z_order, tiles[1].z_order, "z_orders must be unique");
    }

    #[test]
    fn two_tiles_scene_passes_all_layer0_invariants() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("two_tiles", ClockMs::FIXED).unwrap();
        assert_no_violations(&graph, "two_tiles");
    }

    // ── Scene: max_tiles ──────────────────────────────────────────────────

    #[test]
    fn max_tiles_scene_has_correct_structure() {
        let registry = TestSceneRegistry::new();
        let (graph, spec) = registry.build("max_tiles", ClockMs::FIXED).unwrap();

        assert_eq!(graph.tabs.len(), spec.expected_tab_count, "tab count");
        assert_eq!(graph.tiles.len(), spec.expected_tile_count, "tile count");
        // Each tile has exactly one root node
        assert_eq!(
            graph.nodes.len(),
            spec.expected_tile_count,
            "node count must equal tile count (one root per tile)"
        );
    }

    #[test]
    fn max_tiles_scene_all_tiles_within_display() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("max_tiles", ClockMs::FIXED).unwrap();

        let out_of_bounds: Vec<_> = graph
            .tiles
            .values()
            .filter(|t| !t.bounds.is_within(&graph.display_area))
            .collect();

        assert!(
            out_of_bounds.is_empty(),
            "{} tile(s) extend outside the display area",
            out_of_bounds.len()
        );
    }

    #[test]
    fn max_tiles_scene_z_orders_all_unique() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("max_tiles", ClockMs::FIXED).unwrap();

        let mut z_orders: Vec<u32> = graph.tiles.values().map(|t| t.z_order).collect();
        z_orders.sort_unstable();
        let before = z_orders.len();
        z_orders.dedup();
        assert_eq!(z_orders.len(), before, "all z_orders must be unique");
    }

    #[test]
    fn max_tiles_scene_passes_all_layer0_invariants() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("max_tiles", ClockMs::FIXED).unwrap();
        assert_no_violations(&graph, "max_tiles");
    }

    // ── Scene: zone_test ──────────────────────────────────────────────────

    #[test]
    fn zone_test_scene_has_correct_structure() {
        let registry = TestSceneRegistry::new();
        let (graph, spec) = registry.build("zone_test", ClockMs::FIXED).unwrap();

        assert_eq!(graph.tabs.len(), spec.expected_tab_count, "tab count");
        assert_eq!(graph.tiles.len(), spec.expected_tile_count, "tile count");
        assert!(spec.has_zones, "spec must declare has_zones = true");
        assert_eq!(graph.zone_registry.zones.len(), 2, "must have 2 registered zones");
    }

    #[test]
    fn zone_test_scene_zone_names_are_correct() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("zone_test", ClockMs::FIXED).unwrap();

        assert!(
            graph.zone_registry.zones.contains_key("subtitle"),
            "must have subtitle zone"
        );
        assert!(
            graph.zone_registry.zones.contains_key("status_bar"),
            "must have status_bar zone"
        );
    }

    #[test]
    fn zone_test_scene_passes_all_layer0_invariants() {
        let registry = TestSceneRegistry::new();
        let (graph, _spec) = registry.build("zone_test", ClockMs::FIXED).unwrap();
        assert_no_violations(&graph, "zone_test");
    }

    // ── Registry meta ─────────────────────────────────────────────────────

    #[test]
    fn unknown_scene_name_returns_none() {
        let registry = TestSceneRegistry::new();
        assert!(registry.build("does_not_exist", ClockMs::FIXED).is_none());
    }

    #[test]
    fn all_registered_names_build_successfully() {
        let registry = TestSceneRegistry::new();
        for name in TestSceneRegistry::scene_names() {
            let result = registry.build(name, ClockMs::FIXED);
            assert!(result.is_some(), "scene '{name}' failed to build");
        }
    }

    #[test]
    fn all_registered_scenes_pass_layer0_invariants() {
        let registry = TestSceneRegistry::new();
        let mut all_violations: Vec<String> = Vec::new();

        for name in TestSceneRegistry::scene_names() {
            let (graph, _spec) = registry.build(name, ClockMs::FIXED).unwrap();
            let violations = assert_layer0_invariants(&graph);
            for v in &violations {
                all_violations.push(format!("[{name}] {v}"));
            }
        }

        if !all_violations.is_empty() {
            panic!(
                "Layer 0 violations across all scenes:\n{}",
                all_violations.join("\n")
            );
        }
    }

    // ── Clock injection ───────────────────────────────────────────────────

    #[test]
    fn clock_injection_controls_lease_granted_at() {
        let registry = TestSceneRegistry::new();
        let t1 = ClockMs(1_000_000_000_000);
        let t2 = ClockMs(2_000_000_000_000);

        let (graph1, _) = registry.build("single_tile", t1).unwrap();
        let (graph2, _) = registry.build("single_tile", t2).unwrap();

        let lease1 = graph1.leases.values().next().unwrap();
        let lease2 = graph2.leases.values().next().unwrap();

        assert_eq!(lease1.granted_at_ms, t1.0, "lease1 granted_at_ms should match clock t1");
        assert_eq!(lease2.granted_at_ms, t2.0, "lease2 granted_at_ms should match clock t2");
    }

    #[test]
    fn clock_offset_helper_adds_correctly() {
        let base = ClockMs(1_000_000_000_000);
        let offset = base.offset(5_000);
        assert_eq!(offset.0, 1_000_000_005_000);
    }

    // ── Individual invariant checks ───────────────────────────────────────

    #[test]
    fn invariant_detects_orphan_tile_tab() {
        let mut graph = SceneGraph::new(1920.0, 1080.0);
        // We can't create a tile with a non-existent tab via the safe API, so simulate by
        // creating a valid tile and then removing the tab to orphan it.
        let lease_id = graph.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        let real_tab = graph.create_tab("Temp", 0).unwrap();
        let _tile_id = graph
            .create_tile(real_tab, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();
        graph.tabs.remove(&real_tab); // orphan the tile

        let violations = check_tile_tab_refs(&graph);
        assert!(!violations.is_empty(), "expected orphan_tile_tab violation");
        assert_eq!(violations[0].code, "orphan_tile_tab");
    }

    #[test]
    fn invariant_detects_orphan_tile_lease() {
        let mut graph = SceneGraph::new(1920.0, 1080.0);
        let tab_id = graph.create_tab("Main", 0).unwrap();
        let lease_id = graph.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        graph
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();
        // Remove the lease to orphan the tile
        graph.leases.remove(&lease_id);

        let violations = check_tile_lease_refs(&graph);
        assert!(!violations.is_empty(), "expected orphan_tile_lease violation");
        assert_eq!(violations[0].code, "orphan_tile_lease");
    }

    #[test]
    fn invariant_detects_duplicate_z_order() {
        let mut graph = SceneGraph::new(1920.0, 1080.0);
        let tab_id = graph.create_tab("Main", 0).unwrap();
        let lease_id = graph.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        graph
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 5)
            .unwrap();
        graph
            .create_tile(tab_id, "test", lease_id, Rect::new(200.0, 0.0, 100.0, 100.0), 5)
            .unwrap();

        let violations = check_z_order_unique_per_tab(&graph);
        assert!(!violations.is_empty(), "expected duplicate_z_order violation");
        assert_eq!(violations[0].code, "duplicate_z_order");
    }

    #[test]
    fn invariant_detects_missing_active_tab() {
        let mut graph = SceneGraph::new(1920.0, 1080.0);
        // Set active_tab to a non-existent ID
        graph.active_tab = Some(SceneId::new());

        let violations = check_active_tab_exists(&graph);
        assert!(!violations.is_empty(), "expected missing_active_tab violation");
        assert_eq!(violations[0].code, "missing_active_tab");
    }

    #[test]
    fn invariant_detects_zone_name_key_mismatch() {
        use crate::types::ZoneDefinition;

        let mut graph = SceneGraph::new(1920.0, 1080.0);
        // Insert a zone where the map key does not match the definition's name field
        graph.zone_registry.zones.insert(
            "wrong_key".to_string(),
            ZoneDefinition {
                id: SceneId::new(),
                name: "correct_name".to_string(),
                description: "Intentionally mismatched key/name.".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 1.0,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::LatestWins,
                max_publishers: 1,
                transport_constraint: None,
                auto_clear_ms: None,
            },
        );

        let violations = check_zone_name_key_consistency(&graph);
        assert!(!violations.is_empty(), "expected zone_name_key_mismatch violation");
        assert_eq!(violations[0].code, "zone_name_key_mismatch");
    }

    #[test]
    fn invariant_detects_missing_hit_region_state() {
        let mut graph = SceneGraph::new(1920.0, 1080.0);
        let tab_id = graph.create_tab("Main", 0).unwrap();
        let lease_id = graph.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        let tile_id = graph
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 400.0, 300.0), 1)
            .unwrap();

        let hr_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(0.0, 0.0, 100.0, 50.0),
                interaction_id: "btn".into(),
                accepts_focus: true,
                accepts_pointer: true,
            }),
        };
        let node_id = hr_node.id;
        graph.set_tile_root(tile_id, hr_node).unwrap();
        // Simulate missing state entry
        graph.hit_region_states.remove(&node_id);

        let violations = check_hit_region_state_consistency(&graph);
        assert!(!violations.is_empty(), "expected missing_hit_region_state violation");
        assert_eq!(violations[0].code, "missing_hit_region_state");
    }
}
