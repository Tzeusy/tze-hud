//! # Scene Snapshot Performance Benchmark — [rig-bav0]
//!
//! Measures `SceneGraph::take_snapshot` latency against the v1 spec requirement:
//!
//! > Serialization MUST complete in < 1ms for 100 tiles with 1000 nodes.
//! > Source: scene-graph/spec.md line 285, RFC 0001 §10.
//!
//! ## Benchmark groups
//!
//! - `snapshot/100_tiles_1000_nodes` — primary spec requirement case.
//!   100 tiles (10 nodes each), with zone registry populated.
//!
//! - `snapshot/empty_scene` — baseline with no tiles or nodes.
//!
//! - `snapshot/max_tabs_stress` — 25 test-scene `max_tiles_stress` build.
//!
//! ## Interpretation
//!
//! Criterion reports median and p99 wall-clock time. The < 1ms requirement
//! applies to single-core 3GHz-equivalent reference hardware. On faster
//! machines the threshold will be proportionally lower. The benchmark is
//! informational — it does not fail the build. Use it to detect regressions.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use tze_hud_scene::{
    Capability, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode,
    graph::SceneGraph,
    test_scenes::{ClockMs, TestSceneRegistry},
};

const WALL_US: u64 = 1_735_689_600_000_000;
const MONO_US: u64 = 12_345_678;

// ─── Scene builders ───────────────────────────────────────────────────────────

/// Build a scene with `tile_count` tiles, each with `nodes_per_tile` SolidColor nodes.
///
/// Node layout: the first node is the tile root; subsequent nodes are added as
/// children of the root via `add_node_to_tile`. This produces a flat (1-level-deep)
/// tree per tile.
fn build_dense_scene(tile_count: usize, nodes_per_tile: usize) -> SceneGraph {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Bench", 0).unwrap();
    let lease_id = scene.grant_lease(
        "agent.bench",
        600_000,
        vec![
            Capability::CreateTile,
            Capability::CreateNode,
            Capability::UpdateNode,
        ],
    );

    let cols = 10usize;
    for i in 0..tile_count {
        let col = (i % cols) as f32;
        let row = (i / cols) as f32;
        let tile_bounds = Rect::new(col * 192.0, row * 108.0, 190.0, 106.0);

        let tile_id = scene
            .create_tile(tab_id, "agent.bench", lease_id, tile_bounds, i as u32)
            .unwrap();

        let node_size = 190.0 / nodes_per_tile.max(1) as f32;

        // Root node (occupies full tile)
        let root_id = SceneId::new();
        let root_node = Node {
            id: root_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                bounds: Rect::new(0.0, 0.0, 190.0, 106.0),
            }),
        };
        scene.set_tile_root(tile_id, root_node).unwrap();

        // Additional child nodes added to the tile (up to MAX_NODES_PER_TILE)
        let extra_nodes = nodes_per_tile.saturating_sub(1).min(63);
        for j in 0..extra_nodes {
            let child = Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::SolidColor(SolidColorNode {
                    color: Rgba::new(j as f32 / nodes_per_tile as f32, 0.3, 0.7, 1.0),
                    bounds: Rect::new(j as f32 * node_size, 0.0, node_size - 1.0, 50.0),
                }),
            };
            // Add as child of root
            scene
                .add_node_to_tile(tile_id, Some(root_id), child)
                .unwrap();
        }
    }

    scene
}

// ─── Benchmarks ───────────────────────────────────────────────────────────────

fn bench_snapshot_100_tiles_1000_nodes(c: &mut Criterion) {
    // Primary spec requirement: 100 tiles with 1000 nodes total (10 nodes/tile)
    let scene = build_dense_scene(100, 10);
    assert!(scene.tile_count() == 100, "expected 100 tiles");
    assert!(
        scene.node_count() >= 100,
        "expected at least 100 nodes (root nodes)"
    );

    let mut group = c.benchmark_group("snapshot");
    group.bench_function(
        BenchmarkId::new("100_tiles_1000_nodes", "take_snapshot"),
        |b| {
            b.iter(|| black_box(scene.take_snapshot(black_box(WALL_US), black_box(MONO_US))));
        },
    );
    group.finish();
}

fn bench_snapshot_empty_scene(c: &mut Criterion) {
    let scene = SceneGraph::new(1920.0, 1080.0);

    let mut group = c.benchmark_group("snapshot");
    group.bench_function(BenchmarkId::new("empty_scene", "take_snapshot"), |b| {
        b.iter(|| black_box(scene.take_snapshot(black_box(WALL_US), black_box(MONO_US))));
    });
    group.finish();
}

fn bench_snapshot_max_tiles_stress(c: &mut Criterion) {
    let registry = TestSceneRegistry::new();
    let (scene, _) = registry.build("max_tiles_stress", ClockMs::FIXED).unwrap();

    let mut group = c.benchmark_group("snapshot");
    group.bench_function(BenchmarkId::new("max_tiles_stress", "take_snapshot"), |b| {
        b.iter(|| black_box(scene.take_snapshot(black_box(WALL_US), black_box(MONO_US))));
    });
    group.finish();
}

fn bench_snapshot_with_zones(c: &mut Criterion) {
    let registry = TestSceneRegistry::new();
    let (scene, _) = registry
        .build("zone_publish_subtitle", ClockMs::FIXED)
        .unwrap();

    let mut group = c.benchmark_group("snapshot");
    group.bench_function(
        BenchmarkId::new("zone_publish_subtitle", "take_snapshot"),
        |b| {
            b.iter(|| black_box(scene.take_snapshot(black_box(WALL_US), black_box(MONO_US))));
        },
    );
    group.finish();
}

criterion_group!(
    benches,
    bench_snapshot_100_tiles_1000_nodes,
    bench_snapshot_empty_scene,
    bench_snapshot_max_tiles_stress,
    bench_snapshot_with_zones,
);
criterion_main!(benches);
