//! # 8-Stage Frame Pipeline
//!
//! Implements the frame pipeline defined in RFC 0002 §3.2 and
//! `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`.
//!
//! ## Stage Overview
//!
//! | Stage | Name               | Thread     | Budget (p99) |
//! |-------|--------------------|------------|-------------|
//! | 1     | Input Drain        | Main       | < 500µs     |
//! | 2     | Local Feedback     | Main       | < 500µs     |
//! | 3     | Mutation Intake    | Compositor | < 1ms       |
//! | 4     | Scene Commit       | Compositor | < 1ms       |
//! | 5     | Layout Resolve     | Compositor | < 1ms       |
//! | 6     | Render Encode      | Compositor | < 4ms       |
//! | 7     | GPU Submit+Present | Comp+Main  | < 8ms       |
//! | 8     | Telemetry Emit     | Telemetry  | < 200µs     |
//!
//! ## Pipeline Overlap
//!
//! The pipeline supports temporal overlap: GPU work for frame N executes
//! concurrently with input drain for frame N+1. This is modelled at the
//! orchestration layer (HeadlessRuntime / windowed runtime) by submitting
//! the command buffer asynchronously and immediately starting the next
//! frame's Stage 1.
//!
//! ## ArcSwap Hit-Test Snapshot
//!
//! Stage 2 (Local Feedback) must read tile bounds without taking a mutex.
//! The compositor thread publishes a new [`HitTestSnapshot`] after Stage 4
//! via an [`arc_swap::ArcSwap`]. The main thread loads the snapshot with a
//! pointer-width atomic load (no mutex, no blocking).
//!
//! ## Stage 3 Mutation Intake — budget-gated mutation processing
//!
//! [`MutationIntakeStage`] wraps `BudgetEnforcer` to provide the Stage 3
//! integration described in runtime-kernel/spec.md. Budget checks are
//! measured to be < 100µs per mutation batch (spec §Performance Gates).
//!
//! ### Stage 3 responsibilities
//!
//! 1. **Admission gate**: check the mutation batch against the agent's budget
//!    envelope (`BudgetEnforcer::check_mutation`).
//! 2. **Delta accounting**: on `Allow`, apply the mutation delta to the
//!    resource counters (`BudgetEnforcer::apply_mutation_delta`).
//! 3. **Ladder tick**: once per frame, advance the enforcement ladder for all
//!    agents (`BudgetEnforcer::tick`), returning namespaces to revoke.
//! 4. **Post-revocation cleanup**: schedule resource cleanup for revoked
//!    sessions with the configured delay.
//!
//! ### Post-revocation cleanup (spec line 330, 423)
//!
//! When a session is revoked, the runtime must free all agent-owned textures
//! and node data after a configurable delay [0ms, 5000ms] (default 100ms).
//! The `PendingCleanup` type tracks revoked sessions awaiting cleanup, and
//! `MutationIntakeStage::drain_ready_cleanups` returns the namespaces whose
//! cleanup delay has elapsed.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::mutation::MutationBatch;
use tze_hud_scene::types::{Rect, ResourceBudget, SceneId};
use tze_hud_telemetry::FrameTelemetry;

use crate::budget::{BudgetCheckOutcome, BudgetEnforcer, BudgetTelemetrySink};

// ─── Budget constants (microseconds) ──────────────────────────────────────────

/// Stage 1 (Input Drain) p99 budget — 500µs.
pub const STAGE1_BUDGET_US: u64 = 500;
/// Stage 2 (Local Feedback) p99 budget — 500µs.
pub const STAGE2_BUDGET_US: u64 = 500;
/// Stages 1+2 combined p99 budget — 1ms.
pub const STAGE12_COMBINED_BUDGET_US: u64 = 1_000;
/// Stage 3 (Mutation Intake) p99 budget — 1ms.
pub const STAGE3_BUDGET_US: u64 = 1_000;
/// Stage 4 (Scene Commit) p99 budget — 1ms.
pub const STAGE4_BUDGET_US: u64 = 1_000;
/// Stage 5 (Layout Resolve) p99 budget — 1ms.
pub const STAGE5_BUDGET_US: u64 = 1_000;
/// Stage 6 (Render Encode) p99 budget — 4ms.
pub const STAGE6_BUDGET_US: u64 = 4_000;
/// Stage 7 (GPU Submit + Present) p99 budget — 8ms.
pub const STAGE7_BUDGET_US: u64 = 8_000;
/// Stage 8 (Telemetry Emit) p99 budget — 200µs.
pub const STAGE8_BUDGET_US: u64 = 200;
/// Total pipeline (Stage 1 start → Stage 7 end) p99 budget — 16.6ms.
pub const TOTAL_PIPELINE_BUDGET_US: u64 = 16_600;
/// Input-to-local-ack p99 budget — 4ms.
pub const INPUT_TO_LOCAL_ACK_BUDGET_US: u64 = 4_000;
/// Input-to-scene-commit p99 budget — 50ms (covers agent network round-trip).
pub const INPUT_TO_SCENE_COMMIT_BUDGET_US: u64 = 50_000;
/// Input-to-next-present p99 budget — 33ms (two frames at 60fps).
pub const INPUT_TO_NEXT_PRESENT_BUDGET_US: u64 = 33_000;

// ─── Post-revocation cleanup constants ───────────────────────────────────────

/// Default post-revocation cleanup delay in milliseconds (spec line 423).
pub const DEFAULT_POST_REVOCATION_CLEANUP_DELAY_MS: u64 = 100;

/// Minimum allowed cleanup delay (0ms = immediate cleanup).
pub const MIN_POST_REVOCATION_CLEANUP_DELAY_MS: u64 = 0;

/// Maximum allowed cleanup delay (spec line 330).
pub const MAX_POST_REVOCATION_CLEANUP_DELAY_MS: u64 = 5000;

// ─── Hit-Test Snapshot ────────────────────────────────────────────────────────

/// A snapshot of tile bounds used for lock-free hit-testing in Stage 2.
///
/// Published by the compositor thread after Stage 4 (Scene Commit) via
/// [`ArcSwap`]. The main thread loads this atomically (pointer-width load,
/// no mutex).
#[derive(Clone, Debug)]
pub struct HitTestSnapshot {
    /// Sorted (by z-order descending) list of (tile_id_bytes, bounds) pairs.
    /// Using raw bytes avoids a SceneId dependency in snapshot loading.
    pub tiles: Vec<TileBoundsEntry>,
}

/// One entry in the hit-test snapshot.
#[derive(Clone, Debug)]
pub struct TileBoundsEntry {
    /// Tile UUID bytes (128-bit).
    pub tile_id_bytes: [u8; 16],
    /// Tile bounds in display-space pixels.
    pub bounds: Rect,
    /// Z-order (higher = drawn on top / hit first).
    pub z_order: u32,
    /// Owner namespace (for dispatch routing).
    pub namespace: String,
}

impl HitTestSnapshot {
    /// Create an empty snapshot.
    pub fn empty() -> Self {
        Self { tiles: Vec::new() }
    }

    /// Build a snapshot from the current scene graph.
    pub fn from_scene(scene: &SceneGraph) -> Self {
        let mut tiles: Vec<TileBoundsEntry> = scene
            .tiles
            .values()
            .map(|t| TileBoundsEntry {
                tile_id_bytes: t.id.to_bytes_le(),
                bounds: t.bounds,
                z_order: t.z_order,
                namespace: t.namespace.clone(),
            })
            .collect();
        // Sort descending by z_order for hit-testing (highest z tested first)
        tiles.sort_unstable_by(|a, b| b.z_order.cmp(&a.z_order));
        Self { tiles }
    }

    /// Test whether a display-space point (x, y) hits any tile.
    /// Returns the first (highest z) tile entry that contains the point.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<&TileBoundsEntry> {
        self.tiles.iter().find(|t| {
            x >= t.bounds.x
                && x < t.bounds.x + t.bounds.width
                && y >= t.bounds.y
                && y < t.bounds.y + t.bounds.height
        })
    }
}

// ─── Frame Pipeline ───────────────────────────────────────────────────────────

/// The 8-stage frame pipeline orchestrator.
///
/// In the full windowed runtime, this object lives on the compositor thread
/// and coordinates with the main thread via channels and [`ArcSwap`].
///
/// In headless/test mode (`HeadlessRuntime`), all stages run sequentially in
/// the same async task so that tests remain deterministic and GPU-synchronous.
///
/// # Telemetry Overflow
///
/// The compositor thread must never block on the telemetry channel. If the
/// channel is full, the pipeline increments an atomic counter and drops the
/// record. This counter is included in every `FrameTelemetry` record.
pub struct FramePipeline {
    /// Shared hit-test snapshot, published after Stage 4.
    pub hit_test_snapshot: Arc<ArcSwap<HitTestSnapshot>>,
    /// Monotonically increasing frame counter.
    frame_number: u64,
    /// Cumulative telemetry overflow drops since process start.
    telemetry_overflow_count: Arc<AtomicU64>,
}

impl FramePipeline {
    /// Create a new pipeline with an empty hit-test snapshot.
    pub fn new() -> Self {
        Self {
            hit_test_snapshot: Arc::new(ArcSwap::from_pointee(HitTestSnapshot::empty())),
            frame_number: 0,
            telemetry_overflow_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get the current telemetry overflow count.
    pub fn telemetry_overflow_count(&self) -> u64 {
        self.telemetry_overflow_count.load(Ordering::Relaxed)
    }

    // ── Stage 1: Input Drain ─────────────────────────────────────────────────

    /// Stage 1 (Input Drain) — Main thread.
    ///
    /// Drains pending OS input events, attaches hardware timestamps, and
    /// produces `InputEvent` records for Stage 2. This implementation is a
    /// timing harness; in the windowed runtime the actual drain is driven by
    /// the winit event loop callback.
    ///
    /// Returns the elapsed time in microseconds (for telemetry).
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 500µs. Must never block on downstream.
    pub fn stage1_input_drain<F>(&self, drain_fn: F) -> u64
    where
        F: FnOnce(),
    {
        let t0 = Instant::now();
        drain_fn();
        t0.elapsed().as_micros() as u64
    }

    // ── Stage 2: Local Feedback ───────────────────────────────────────────────

    /// Stage 2 (Local Feedback) — Main thread.
    ///
    /// Hit-tests input events against the current tile bounds snapshot
    /// (loaded from `ArcSwap` — no mutex), updates pressed/hovered local
    /// state flags.
    ///
    /// Returns the elapsed time in microseconds (for telemetry).
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 500µs. Uses ArcSwap for lock-free read.
    pub fn stage2_local_feedback<F>(&self, feedback_fn: F) -> u64
    where
        F: FnOnce(&HitTestSnapshot),
    {
        let t0 = Instant::now();
        // Load the snapshot with a pointer-width atomic (no mutex)
        let snapshot = self.hit_test_snapshot.load();
        feedback_fn(&snapshot);
        t0.elapsed().as_micros() as u64
    }

    // ── Stage 3: Mutation Intake ──────────────────────────────────────────────

    /// Stage 3 (Mutation Intake) — Compositor thread.
    ///
    /// Drains the `MutationBatch` channel and applies agent envelope limits.
    /// Each batch is an atomic unit; batches are never coalesced.
    ///
    /// Returns (elapsed_us, batches_consumed).
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 1ms.
    pub fn stage3_mutation_intake<F>(&self, intake_fn: F) -> (u64, u32)
    where
        F: FnOnce() -> u32,
    {
        let t0 = Instant::now();
        let batches = intake_fn();
        (t0.elapsed().as_micros() as u64, batches)
    }

    // ── Stage 4: Scene Commit ─────────────────────────────────────────────────

    /// Stage 4 (Scene Commit) — Compositor thread.
    ///
    /// Applies validated mutation batches with all-or-nothing semantics per
    /// batch. After commit, publishes an updated `HitTestSnapshot` via
    /// `ArcSwap`.
    ///
    /// Returns (elapsed_us, mutations_applied).
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 1ms.
    pub fn stage4_scene_commit<F>(&self, commit_fn: F) -> (u64, u32)
    where
        F: FnOnce() -> (u32, HitTestSnapshot),
    {
        let t0 = Instant::now();
        let (mutations, new_snapshot) = commit_fn();
        // Publish updated snapshot via ArcSwap (atomic pointer swap)
        self.hit_test_snapshot.store(Arc::new(new_snapshot));
        (t0.elapsed().as_micros() as u64, mutations)
    }

    // ── Stage 5: Layout Resolve ───────────────────────────────────────────────

    /// Stage 5 (Layout Resolve) — Compositor thread.
    ///
    /// Runs incremental layout: recomputes only tiles that changed this frame.
    /// Validates bounds, recomputes z-order stack, computes compositing regions.
    ///
    /// Returns (elapsed_us, tiles_recomputed).
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 1ms. Incremental (only changed tiles).
    pub fn stage5_layout_resolve<F>(&self, layout_fn: F) -> (u64, u32)
    where
        F: FnOnce() -> u32,
    {
        let t0 = Instant::now();
        let tiles_recomputed = layout_fn();
        (t0.elapsed().as_micros() as u64, tiles_recomputed)
    }

    // ── Stage 6: Render Encode ────────────────────────────────────────────────

    /// Stage 6 (Render Encode) — Compositor thread.
    ///
    /// Builds a `wgpu::CommandEncoder` from the `RenderFrame`. Issues draw
    /// calls for tile nodes (solid color, text, image), encodes alpha-blend
    /// passes, and encodes the chrome layer.
    ///
    /// **MUST NOT** submit the command buffer to the GPU queue (that is Stage 7).
    /// Single-threaded in v1 (Parallel Render Encoding is post-v1).
    ///
    /// Returns elapsed_us.
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 4ms. Single-threaded v1.
    pub fn stage6_render_encode<F>(&self, encode_fn: F) -> u64
    where
        F: FnOnce(),
    {
        let t0 = Instant::now();
        encode_fn();
        t0.elapsed().as_micros() as u64
    }

    // ── Stage 7: GPU Submit + Present ─────────────────────────────────────────

    /// Stage 7 (GPU Submit + Present) — Compositor thread (submit) + main thread (present).
    ///
    /// The compositor thread submits the encoded `CommandBuffer` to the wgpu
    /// queue and signals the main thread via `FrameReadySignal`. The main
    /// thread calls `surface.present()`.
    ///
    /// In headless mode `present()` is a no-op.
    ///
    /// Returns elapsed_us.
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 8ms combined.
    pub fn stage7_gpu_submit<F>(&self, submit_fn: F) -> u64
    where
        F: FnOnce(),
    {
        let t0 = Instant::now();
        submit_fn();
        t0.elapsed().as_micros() as u64
    }

    // ── Stage 8: Telemetry Emit ───────────────────────────────────────────────

    /// Stage 8 (Telemetry Emit) — Telemetry thread.
    ///
    /// Sends a `TelemetryRecord` to the telemetry thread via a non-blocking
    /// bounded channel. If the channel is full, the emitter drops a record
    /// (the drop policy — newest vs. oldest — is determined by the caller's
    /// `emit_fn` implementation) and returns `true` to signal overflow.
    /// `telemetry_overflow_count` is incremented on each overflow. Must never
    /// block the frame pipeline.
    ///
    /// Returns elapsed_us.
    ///
    /// **Spec**: RFC 0002 §3.2 — p99 < 200µs. Non-blocking; drop-on-full.
    pub fn stage8_telemetry_emit<F>(&self, emit_fn: F) -> u64
    where
        F: FnOnce() -> bool,
    {
        let t0 = Instant::now();
        let dropped = emit_fn();
        if dropped {
            // Overflow: increment the counter (non-blocking, relaxed ordering)
            self.telemetry_overflow_count.fetch_add(1, Ordering::Relaxed);
        }
        t0.elapsed().as_micros() as u64
    }

    // ── Full Sequential Pipeline (headless / test) ────────────────────────────

    /// Run all 8 stages sequentially and return a fully-populated `FrameTelemetry`.
    ///
    /// This is the headless/test entry-point that runs all stages in-process.
    /// The real windowed runtime orchestrates stages across threads via channels
    /// and signalling, but the logical order and telemetry contract are identical.
    ///
    /// Parameters are closures for each stage, enabling full customisation
    /// in tests:
    ///
    /// - `drain`:   Stage 1 — drain OS events
    /// - `feedback`: Stage 2 — apply local feedback given current snapshot
    /// - `intake`:  Stage 3 — drain mutation channel, return batch count
    /// - `commit`:  Stage 4 — commit mutations, return (mutation count, new snapshot)
    /// - `layout`:  Stage 5 — incremental layout, return tiles recomputed
    /// - `encode`:  Stage 6 — build CommandEncoder (no GPU submit)
    /// - `submit`:  Stage 7 — submit + present
    /// - `emit`:    Stage 8 — telemetry send; return `true` if overflow occurred
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame<Drain, Feedback, Intake, Commit, Layout, Encode, Submit, Emit>(
        &mut self,
        drain: Drain,
        feedback: Feedback,
        intake: Intake,
        commit: Commit,
        layout: Layout,
        encode: Encode,
        submit: Submit,
        emit: Emit,
    ) -> FrameTelemetry
    where
        Drain: FnOnce(),
        Feedback: FnOnce(&HitTestSnapshot),
        Intake: FnOnce() -> u32,
        Commit: FnOnce() -> (u32, HitTestSnapshot),
        Layout: FnOnce() -> u32,
        Encode: FnOnce(),
        Submit: FnOnce(),
        Emit: FnOnce() -> bool,
    {
        self.frame_number += 1;
        let frame_start = Instant::now();
        let mut telemetry = FrameTelemetry::new(self.frame_number);
        telemetry.telemetry_overflow_count = self.telemetry_overflow_count.load(Ordering::Relaxed);

        // Stage 1: Input Drain (Main thread)
        telemetry.stage1_input_drain_us = self.stage1_input_drain(drain);

        // Stage 2: Local Feedback (Main thread)
        telemetry.stage2_local_feedback_us = self.stage2_local_feedback(feedback);

        // Stage 3: Mutation Intake (Compositor thread)
        let (stage3_us, _batches) = self.stage3_mutation_intake(intake);
        telemetry.stage3_mutation_intake_us = stage3_us;

        // Stage 4: Scene Commit (Compositor thread)
        let (stage4_us, mutations) = self.stage4_scene_commit(commit);
        telemetry.stage4_scene_commit_us = stage4_us;
        telemetry.mutations_applied = mutations;

        // Stage 5: Layout Resolve (Compositor thread)
        let (stage5_us, tiles_recomputed) = self.stage5_layout_resolve(layout);
        telemetry.stage5_layout_resolve_us = stage5_us;
        telemetry.tiles_layout_recomputed = tiles_recomputed;

        // Stage 6: Render Encode (Compositor thread — single-threaded v1)
        telemetry.stage6_render_encode_us = self.stage6_render_encode(encode);

        // Stage 7: GPU Submit + Present (Compositor + Main thread)
        telemetry.stage7_gpu_submit_us = self.stage7_gpu_submit(submit);

        // Record total frame time (Stage 1 start → Stage 7 end)
        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;

        // Stage 8: Telemetry Emit (Telemetry thread — non-blocking)
        // Note: measured separately; does NOT add to frame_time_us
        telemetry.stage8_telemetry_emit_us = self.stage8_telemetry_emit(emit);

        // Update telemetry overflow count (may have incremented in Stage 8)
        telemetry.telemetry_overflow_count = self.telemetry_overflow_count.load(Ordering::Relaxed);

        // Sync legacy alias fields
        telemetry.sync_legacy_aliases();

        telemetry
    }

    /// Convenience: run a frame driven by the current scene graph.
    ///
    /// This is the simplified path used by [`super::headless::HeadlessRuntime`]:
    /// it derives all stage callbacks from the scene graph state and produces
    /// telemetry with per-stage timings. The compositor render step (Stages 6-7)
    /// is delegated to the provided closure.
    pub fn run_scene_frame<RenderEncode, GpuSubmit>(
        &mut self,
        render_encode: RenderEncode,
        gpu_submit: GpuSubmit,
        pending_mutations: Vec<MutationBatch>,
        scene_graph_mut: &mut SceneGraph,
    ) -> FrameTelemetry
    where
        RenderEncode: FnOnce(),
        GpuSubmit: FnOnce(),
    {
        self.frame_number += 1;
        let frame_start = Instant::now();
        let mut telemetry = FrameTelemetry::new(self.frame_number);
        telemetry.telemetry_overflow_count = self.telemetry_overflow_count.load(Ordering::Relaxed);

        // Stage 1: Input Drain (no-op in headless; events come via channel)
        let s1_start = Instant::now();
        // In headless mode there is no OS event queue to drain.
        // The timing harness still records the stage boundary.
        telemetry.stage1_input_drain_us = s1_start.elapsed().as_micros() as u64;

        // Stage 2: Local Feedback (no-op in headless; no winit pointer events)
        let s2_start = Instant::now();
        // Load snapshot lock-free (ArcSwap — no mutex)
        let _snapshot = self.hit_test_snapshot.load();
        telemetry.stage2_local_feedback_us = s2_start.elapsed().as_micros() as u64;

        // Stage 3: Mutation Intake — drain / count the pending batch list.
        // Per spec §3.2: Stage 3 drains the MutationBatch channel and applies
        // agent envelope limits. apply_batch() (commit work) belongs to Stage 4.
        let s3_start = Instant::now();
        let batch_count = pending_mutations.len() as u32;
        telemetry.stage3_mutation_intake_us = s3_start.elapsed().as_micros() as u64;

        // Stage 4: Scene Commit — apply batches (all-or-nothing per batch) and
        // publish updated hit-test snapshot via ArcSwap.
        let s4_start = Instant::now();
        let mut mutations_applied = 0u32;
        // Each batch is validated and applied independently (no coalescing)
        for batch in &pending_mutations {
            let result = scene_graph_mut.apply_batch(batch);
            if result.applied {
                // Count each mutation in the batch that was applied
                mutations_applied += batch.mutations.len() as u32;
            }
        }
        let new_snapshot = HitTestSnapshot::from_scene(scene_graph_mut);
        self.hit_test_snapshot.store(Arc::new(new_snapshot));
        telemetry.stage4_scene_commit_us = s4_start.elapsed().as_micros() as u64;
        telemetry.mutations_applied = mutations_applied;

        // Stage 5: Layout Resolve — incremental layout for changed tiles
        let s5_start = Instant::now();
        // In the headless path the compositor handles tile visibility ordering.
        // We record the count of tiles visible in the current scene as a proxy
        // for "tiles that went through layout" (full layout in v1).
        let tiles_visible = scene_graph_mut.visible_tiles().len() as u32;
        telemetry.stage5_layout_resolve_us = s5_start.elapsed().as_micros() as u64;
        telemetry.tiles_layout_recomputed = if batch_count > 0 { tiles_visible } else { 0 };

        // Update visible scene counters
        telemetry.tile_count = tiles_visible;
        telemetry.node_count = scene_graph_mut.node_count() as u32;
        telemetry.active_leases = scene_graph_mut.leases.len() as u32;

        // Stage 6: Render Encode (single-threaded, compositor thread)
        let s6_start = Instant::now();
        render_encode();
        telemetry.stage6_render_encode_us = s6_start.elapsed().as_micros() as u64;

        // Stage 7: GPU Submit + Present
        let s7_start = Instant::now();
        gpu_submit();
        telemetry.stage7_gpu_submit_us = s7_start.elapsed().as_micros() as u64;

        // Record total frame time (Stage 1 start → Stage 7 end)
        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;

        // Stage 8: Telemetry Emit (non-blocking; records stage 8 time separately)
        let s8_start = Instant::now();
        // In the headless path, the caller (HeadlessRuntime) records the telemetry
        // directly. We record only the boundary overhead here.
        telemetry.stage8_telemetry_emit_us = s8_start.elapsed().as_micros() as u64;

        // Update overflow count
        telemetry.telemetry_overflow_count = self.telemetry_overflow_count.load(Ordering::Relaxed);

        // Sync legacy alias fields for backward compatibility
        telemetry.sync_legacy_aliases();

        telemetry
    }
}

impl Default for FramePipeline {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Pending cleanup ─────────────────────────────────────────────────────────

/// A revoked session awaiting post-revocation resource cleanup.
#[derive(Debug)]
pub struct PendingCleanup {
    /// Agent namespace whose resources must be freed.
    pub namespace: String,
    /// The revocation event timestamp.
    pub revoked_at: Instant,
    /// Delay before cleanup should run.
    pub cleanup_delay: Duration,
}

impl PendingCleanup {
    /// Returns `true` if the cleanup delay has elapsed relative to `now`.
    pub fn is_ready(&self, now: Instant) -> bool {
        now.duration_since(self.revoked_at) >= self.cleanup_delay
    }
}

// ─── Intake result ────────────────────────────────────────────────────────────

/// Outcome of `MutationIntakeStage::check_and_apply`.
#[derive(Debug)]
pub struct IntakeResult {
    /// Whether the mutation batch was allowed, rejected, or caused revocation.
    pub outcome: BudgetCheckOutcome,
    /// Wall time taken by the budget check (nanoseconds).
    /// Must be < 100,000 ns (100µs) per spec.
    pub check_duration_ns: u64,
}

impl IntakeResult {
    /// Returns `true` if the budget check took longer than 100µs.
    ///
    /// Callers should emit a warning or telemetry event when this is true.
    pub fn exceeded_latency_budget(&self) -> bool {
        self.check_duration_ns > 100_000
    }
}

// ─── Stage 3 Mutation Intake ─────────────────────────────────────────────────

/// Stage 3 (Mutation Intake) of the frame pipeline.
///
/// Owns a `BudgetEnforcer` and a queue of pending post-revocation cleanups.
/// The compositor thread calls this exclusively; no locking is required.
pub struct MutationIntakeStage {
    /// The budget enforcer — one per runtime, shared across all agent sessions.
    pub enforcer: BudgetEnforcer,
    /// Post-revocation cleanup delay (clamped to [0, 5000] ms).
    pub cleanup_delay: Duration,
    /// Queue of revoked sessions awaiting resource cleanup.
    pending_cleanups: VecDeque<PendingCleanup>,
}

impl MutationIntakeStage {
    /// Construct with default cleanup delay (100ms).
    pub fn new() -> Self {
        Self::with_cleanup_delay(Duration::from_millis(DEFAULT_POST_REVOCATION_CLEANUP_DELAY_MS))
    }

    /// Construct with a custom cleanup delay (clamped to [0ms, 5000ms]).
    pub fn with_cleanup_delay(delay: Duration) -> Self {
        // Clamp in Duration-space first to avoid u128→u64 truncation for extreme
        // values like Duration::MAX (as_millis() returns u128, which would wrap
        // when cast to u64 before clamping).
        let clamped = delay.clamp(
            Duration::from_millis(MIN_POST_REVOCATION_CLEANUP_DELAY_MS),
            Duration::from_millis(MAX_POST_REVOCATION_CLEANUP_DELAY_MS),
        );
        Self {
            enforcer: BudgetEnforcer::new(),
            cleanup_delay: clamped,
            pending_cleanups: VecDeque::new(),
        }
    }

    // ── Session management ─────────────────────────────────────────────────

    /// Register a new agent session with the given budget.
    ///
    /// Must be called before any `check_and_apply` calls for this namespace.
    pub fn register_session(
        &mut self,
        session_id: SceneId,
        namespace: String,
        budget: ResourceBudget,
    ) {
        self.enforcer.register_session(session_id, namespace, budget);
    }

    /// Remove a session from budget tracking (on clean disconnect).
    ///
    /// Unlike revocation, clean disconnect does not schedule cleanup — the
    /// caller is expected to release resources synchronously at this point.
    pub fn remove_session(&mut self, namespace: &str) {
        self.enforcer.remove_session(namespace);
    }

    // ── Mutation intake ────────────────────────────────────────────────────

    /// Run the budget check for a pending mutation batch (Stage 3 intake gate).
    ///
    /// If the check returns `Allow`, the resource delta is immediately applied
    /// to the agent's tracked counters.
    ///
    /// # Parameters
    /// - `namespace`: agent's registered namespace
    /// - `delta_tiles`: proposed change in tile count (negative for deletions)
    /// - `delta_texture_bytes`: proposed change in texture memory (negative for deletions)
    /// - `max_nodes_in_batch`: maximum node count across all tiles in this batch
    /// - `now`: current monotonic timestamp
    /// - `sink`: telemetry sink for violation events
    ///
    /// # Returns
    /// `IntakeResult` with the check outcome and wall-time measurement.
    pub fn check_and_apply(
        &mut self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
        max_nodes_in_batch: u32,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> IntakeResult {
        let start = Instant::now();
        let outcome = self.enforcer.check_mutation(
            namespace,
            delta_tiles,
            delta_texture_bytes,
            max_nodes_in_batch,
            now,
            sink,
        );
        let check_duration_ns = start.elapsed().as_nanos() as u64;

        // On Allow, apply the delta to tracked counters.
        if matches!(outcome, BudgetCheckOutcome::Allow) {
            self.enforcer
                .apply_mutation_delta(namespace, delta_tiles, delta_texture_bytes);
        }

        // On Revoke, schedule post-revocation cleanup.
        if matches!(outcome, BudgetCheckOutcome::Revoke(_)) {
            self.schedule_cleanup(namespace.to_string(), now);
        }

        IntakeResult {
            outcome,
            check_duration_ns,
        }
    }

    // ── Enforcement ladder ─────────────────────────────────────────────────

    /// Advance the enforcement ladder for all agents (call once per frame).
    ///
    /// Returns namespaces that were revoked this tick. Callers must tear down
    /// those sessions and may call `schedule_cleanup` for each.
    pub fn tick(
        &mut self,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> Vec<String> {
        let revoked = self.enforcer.tick(now, sink);
        // Schedule post-revocation cleanup for each newly-revoked session.
        for ns in &revoked {
            self.schedule_cleanup(ns.clone(), now);
        }
        revoked
    }

    // ── Post-revocation cleanup ────────────────────────────────────────────

    /// Schedule post-revocation cleanup for a namespace.
    ///
    /// If the namespace already has a pending cleanup entry, the duplicate is
    /// silently dropped — cleanup is idempotent and the existing entry is
    /// sufficient. This prevents `tick()` from accumulating unbounded duplicate
    /// entries when a revoked session persists in the budget enforcer.
    pub fn schedule_cleanup(&mut self, namespace: String, revoked_at: Instant) {
        if self.pending_cleanups.iter().any(|c| c.namespace == namespace) {
            return; // already scheduled; drop the duplicate
        }
        self.pending_cleanups.push_back(PendingCleanup {
            namespace,
            revoked_at,
            cleanup_delay: self.cleanup_delay,
        });
    }

    /// Drain all cleanups whose delay has elapsed.
    ///
    /// Returns the namespaces ready for resource cleanup. The caller is
    /// responsible for freeing textures, node data, and any other
    /// agent-owned state (reducing the agent's resource footprint to zero).
    ///
    /// The drain scans the entire queue rather than stopping at the first
    /// non-ready entry, since `schedule_cleanup` is public and callers may
    /// enqueue entries with arbitrary `revoked_at` timestamps.
    pub fn drain_ready_cleanups(&mut self, now: Instant) -> Vec<String> {
        let mut ready = Vec::new();
        self.pending_cleanups.retain(|cleanup| {
            if cleanup.is_ready(now) {
                ready.push(cleanup.namespace.clone());
                false // remove from queue
            } else {
                true // keep
            }
        });
        ready
    }

    /// Number of sessions awaiting post-revocation cleanup.
    pub fn pending_cleanup_count(&self) -> usize {
        self.pending_cleanups.len()
    }

    // ── Invariant violation reporting ──────────────────────────────────────

    /// Report an invariant violation. Returns `true` if the session was revoked
    /// (> 10 violations triggers immediate critical revocation per spec line 326).
    ///
    /// If revoked, a cleanup is automatically scheduled.
    pub fn report_invariant_violation(
        &mut self,
        namespace: &str,
        now: Instant,
        sink: &mut dyn BudgetTelemetrySink,
    ) -> bool {
        let revoked = self.enforcer.report_invariant_violation(namespace, now, sink);
        if revoked {
            self.schedule_cleanup(namespace.to_string(), now);
        }
        revoked
    }
}

impl Default for MutationIntakeStage {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use crate::budget::{CollectingTelemetrySink, NoopTelemetrySink};
    use tze_hud_scene::types::BudgetViolation;

    fn tight_budget() -> ResourceBudget {
        ResourceBudget {
            max_tiles: 2,
            max_texture_bytes: 1024,
            max_update_rate_hz: 5.0,
            max_nodes_per_tile: 4,
        }
    }

    fn make_stage() -> (MutationIntakeStage, CollectingTelemetrySink) {
        let mut stage = MutationIntakeStage::new();
        let sink = CollectingTelemetrySink::default();
        stage.register_session(SceneId::new(), "agent-a".to_string(), tight_budget());
        (stage, sink)
    }

    // ─── FramePipeline tests ──────────────────────────────────────────────

    /// Verify that all 8 stages execute in the correct order (1→8) in a single frame.
    ///
    /// The closures append their stage number to a shared log; the test asserts
    /// the log is exactly [1, 2, 3, 4, 5, 6, 7, 8].
    #[test]
    fn test_stage_ordering_enforced_1_through_8() {
        let log: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let mut pipeline = FramePipeline::new();

        let l = log.clone();
        let empty_scene = SceneGraph::new(800.0, 600.0);
        let mut scene = SceneGraph::new(800.0, 600.0);

        let telemetry = pipeline.run_frame(
            || l.lock().unwrap().push(1), // Stage 1: Input Drain
            |_snapshot| l.lock().unwrap().push(2), // Stage 2: Local Feedback
            || { l.lock().unwrap().push(3); 0 }, // Stage 3: Mutation Intake
            || { l.lock().unwrap().push(4); (0, HitTestSnapshot::from_scene(&empty_scene)) }, // Stage 4
            || { l.lock().unwrap().push(5); 0 }, // Stage 5: Layout Resolve
            || l.lock().unwrap().push(6), // Stage 6: Render Encode
            || l.lock().unwrap().push(7), // Stage 7: GPU Submit
            || { l.lock().unwrap().push(8); false }, // Stage 8: Telemetry
        );

        let stages = log.lock().unwrap().clone();
        assert_eq!(
            stages,
            vec![1, 2, 3, 4, 5, 6, 7, 8],
            "stages must execute in order 1→8; got: {stages:?}"
        );
        assert_eq!(telemetry.frame_number, 1, "first frame should be frame 1");
        // All per-stage telemetry fields exist in the returned struct
        let _ = telemetry.stage1_input_drain_us;
        let _ = telemetry.stage2_local_feedback_us;
        let _ = telemetry.stage3_mutation_intake_us;
        let _ = telemetry.stage4_scene_commit_us;
        let _ = telemetry.stage5_layout_resolve_us;
        let _ = telemetry.stage6_render_encode_us;
        let _ = telemetry.stage7_gpu_submit_us;
        let _ = telemetry.stage8_telemetry_emit_us;
        let _ = scene;
    }

    /// Verify that frame_time_us covers Stage 1 start through Stage 7 end,
    /// and that Stage 8 is excluded from the total budget.
    #[test]
    fn test_frame_time_covers_stages_1_through_7_only() {
        let mut pipeline = FramePipeline::new();
        let empty_scene = SceneGraph::new(800.0, 600.0);

        let telemetry = pipeline.run_frame(
            || {},
            |_| {},
            || 0,
            || (0, HitTestSnapshot::from_scene(&empty_scene)),
            || 0,
            || {},
            || {},
            || false,
        );

        // frame_time_us must be >= sum of stage 1-7 times
        let stage_sum = telemetry.stage1_input_drain_us
            + telemetry.stage2_local_feedback_us
            + telemetry.stage3_mutation_intake_us
            + telemetry.stage4_scene_commit_us
            + telemetry.stage5_layout_resolve_us
            + telemetry.stage6_render_encode_us
            + telemetry.stage7_gpu_submit_us;

        assert!(
            telemetry.frame_time_us >= stage_sum,
            "frame_time_us ({}) must be >= sum of stage 1-7 timings ({})",
            telemetry.frame_time_us,
            stage_sum
        );
    }

    /// Verify that the hit-test snapshot is published by Stage 4 and is
    /// readable by Stage 2 in the *next* frame without taking a mutex.
    #[test]
    fn test_arc_swap_snapshot_published_by_stage4() {
        let mut pipeline = FramePipeline::new();

        // Initial snapshot is empty
        {
            let snap = pipeline.hit_test_snapshot.load();
            assert!(snap.tiles.is_empty(), "initial snapshot should be empty");
        }

        // Build a scene with one tile, run a frame that commits it
        let mut scene = SceneGraph::new(800.0, 600.0);
        let tab = scene.create_tab("Main", 0).unwrap();
        let lease = scene.grant_lease("agent", 60_000, vec![]);
        scene
            .create_tile(tab, "agent", lease, Rect::new(10.0, 20.0, 100.0, 50.0), 1)
            .unwrap();

        // Frame 1: stage 4 publishes the new snapshot
        let scene_for_commit = scene.clone();
        pipeline.run_frame(
            || {},
            |_| {},
            || 0,
            move || {
                let snap = HitTestSnapshot::from_scene(&scene_for_commit);
                (0, snap)
            },
            || 0,
            || {},
            || {},
            || false,
        );

        // After Stage 4 published, the snapshot is visible immediately (ArcSwap)
        let snap = pipeline.hit_test_snapshot.load();
        assert_eq!(snap.tiles.len(), 1, "snapshot should contain the committed tile");
        assert_eq!(snap.tiles[0].bounds.x, 10.0);
        assert_eq!(snap.tiles[0].bounds.y, 20.0);
    }

    /// Verify that Stage 2 reads the snapshot lock-free (can call from any thread).
    #[test]
    fn test_stage2_hit_test_uses_arc_swap_no_mutex() {
        let pipeline = FramePipeline::new();
        let mut scene = SceneGraph::new(800.0, 600.0);
        let tab = scene.create_tab("Main", 0).unwrap();
        let lease = scene.grant_lease("agent", 60_000, vec![]);
        scene
            .create_tile(tab, "agent", lease, Rect::new(50.0, 50.0, 200.0, 100.0), 1)
            .unwrap();
        let snap = HitTestSnapshot::from_scene(&scene);
        pipeline.hit_test_snapshot.store(Arc::new(snap));

        // Stage 2 can be called from any thread — no mutex acquisition
        let mut hit_found = false;
        let elapsed_us = pipeline.stage2_local_feedback(|snapshot| {
            hit_found = snapshot.hit_test(100.0, 80.0).is_some();
        });

        assert!(hit_found, "should hit the tile at (100, 80)");
        assert!(elapsed_us < STAGE2_BUDGET_US * 100, "stage 2 overhead should be minimal");
    }

    /// Verify telemetry overflow counter increments when Stage 8 returns true.
    #[test]
    fn test_telemetry_overflow_counter_increments() {
        let mut pipeline = FramePipeline::new();
        let empty_scene = SceneGraph::new(800.0, 600.0);
        assert_eq!(pipeline.telemetry_overflow_count(), 0);

        // Simulate a frame where telemetry channel is full (emit returns true = overflow)
        let telemetry = pipeline.run_frame(
            || {},
            |_| {},
            || 0,
            || (0, HitTestSnapshot::from_scene(&empty_scene)),
            || 0,
            || {},
            || {},
            || true, // overflow!
        );

        assert_eq!(
            pipeline.telemetry_overflow_count(),
            1,
            "overflow counter should increment"
        );
        assert_eq!(
            telemetry.telemetry_overflow_count, 1,
            "telemetry record should reflect overflow count"
        );
    }

    /// Verify that all stage timing fields are populated in the telemetry record.
    #[test]
    fn test_all_per_stage_telemetry_fields_populated() {
        let mut pipeline = FramePipeline::new();
        let empty_scene = SceneGraph::new(800.0, 600.0);

        let telemetry = pipeline.run_frame(
            || {},
            |_| {},
            || 0,
            || (0, HitTestSnapshot::from_scene(&empty_scene)),
            || 0,
            || {},
            || {},
            || false,
        );

        // All stage fields should exist (values may be 0 on fast machines, that's OK)
        // The important thing is the struct has the fields and they don't panic.
        let _: u64 = telemetry.stage1_input_drain_us;
        let _: u64 = telemetry.stage2_local_feedback_us;
        let _: u64 = telemetry.stage3_mutation_intake_us;
        let _: u64 = telemetry.stage4_scene_commit_us;
        let _: u64 = telemetry.stage5_layout_resolve_us;
        let _: u64 = telemetry.stage6_render_encode_us;
        let _: u64 = telemetry.stage7_gpu_submit_us;
        let _: u64 = telemetry.stage8_telemetry_emit_us;
        let _: u64 = telemetry.frame_time_us;
        let _: u64 = telemetry.telemetry_overflow_count;
    }

    /// Verify that legacy alias fields are synced from per-stage fields.
    #[test]
    fn test_legacy_aliases_synced() {
        let mut pipeline = FramePipeline::new();
        let empty_scene = SceneGraph::new(800.0, 600.0);

        let telemetry = pipeline.run_frame(
            || {},
            |_| {},
            || 0,
            || (0, HitTestSnapshot::from_scene(&empty_scene)),
            || 0,
            || {},
            || {},
            || false,
        );

        assert_eq!(telemetry.input_drain_us, telemetry.stage1_input_drain_us);
        assert_eq!(telemetry.scene_commit_us, telemetry.stage4_scene_commit_us);
        assert_eq!(telemetry.render_encode_us, telemetry.stage6_render_encode_us);
        assert_eq!(telemetry.gpu_submit_us, telemetry.stage7_gpu_submit_us);
    }

    /// Verify the HitTestSnapshot correctly builds from a scene and performs hit-testing.
    #[test]
    fn test_hit_test_snapshot_from_scene() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab = scene.create_tab("Main", 0).unwrap();
        let lease = scene.grant_lease("agent", 60_000, vec![]);
        scene
            .create_tile(tab, "agent", lease, Rect::new(100.0, 100.0, 300.0, 200.0), 1)
            .unwrap();
        scene
            .create_tile(tab, "agent", lease, Rect::new(200.0, 150.0, 100.0, 100.0), 2)
            .unwrap();

        let snap = HitTestSnapshot::from_scene(&scene);
        assert_eq!(snap.tiles.len(), 2);
        // Tiles sorted by z_order descending (z=2 first)
        assert_eq!(snap.tiles[0].z_order, 2);
        assert_eq!(snap.tiles[1].z_order, 1);

        // Hit the higher-z tile
        let hit = snap.hit_test(250.0, 175.0);
        assert!(hit.is_some(), "should hit a tile at (250, 175)");
        assert_eq!(hit.unwrap().z_order, 2, "should hit the higher-z tile");

        // Hit outside all tiles
        assert!(snap.hit_test(0.0, 0.0).is_none(), "should not hit anything at (0,0)");
    }

    /// Budget constants should match the spec values.
    #[test]
    fn test_budget_constants_match_spec() {
        assert_eq!(STAGE1_BUDGET_US, 500);
        assert_eq!(STAGE2_BUDGET_US, 500);
        assert_eq!(STAGE12_COMBINED_BUDGET_US, 1_000);
        assert_eq!(STAGE3_BUDGET_US, 1_000);
        assert_eq!(STAGE4_BUDGET_US, 1_000);
        assert_eq!(STAGE5_BUDGET_US, 1_000);
        assert_eq!(STAGE6_BUDGET_US, 4_000);
        assert_eq!(STAGE7_BUDGET_US, 8_000);
        assert_eq!(STAGE8_BUDGET_US, 200);
        assert_eq!(TOTAL_PIPELINE_BUDGET_US, 16_600);
    }

    // ─── MutationIntakeStage tests ────────────────────────────────────────

    // ─── Budget check latency ──────────────────────────────────────────────

    #[test]
    fn test_check_latency_under_100us() {
        let (mut stage, mut sink) = make_stage();

        let result = stage.check_and_apply("agent-a", 1, 512, 2, Instant::now(), &mut sink);

        assert!(
            !result.exceeded_latency_budget(),
            "budget check took {}ns, expected < 100µs (100,000ns)",
            result.check_duration_ns
        );
    }

    #[test]
    fn test_check_latency_100_iterations_all_under_100us() {
        let mut stage = MutationIntakeStage::new();
        let mut sink = NoopTelemetrySink;
        stage.register_session(SceneId::new(), "bench-agent".to_string(), tight_budget());

        let mut over_budget = 0;
        for _ in 0..100 {
            let result =
                stage.check_and_apply("bench-agent", 0, 0, 0, Instant::now(), &mut sink);
            if result.exceeded_latency_budget() {
                over_budget += 1;
            }
            // Reset tile count so the agent doesn't get rejected.
            if let Some(state) = stage.enforcer.agent_state_mut("bench-agent") {
                state.tile_count = 0;
                state.texture_bytes_used = 0;
            }
        }
        // Allow up to 1 outlier (scheduler jitter).
        assert!(
            over_budget <= 1,
            "{} of 100 iterations exceeded 100µs latency budget",
            over_budget
        );
    }

    // ─── Delta accounting ──────────────────────────────────────────────────

    #[test]
    fn test_delta_applied_on_allow() {
        let (mut stage, mut sink) = make_stage();

        let result = stage.check_and_apply("agent-a", 1, 512, 2, Instant::now(), &mut sink);
        assert!(matches!(result.outcome, BudgetCheckOutcome::Allow));

        let state = stage.enforcer.agent_state("agent-a").unwrap();
        assert_eq!(state.tile_count, 1);
        assert_eq!(state.texture_bytes_used, 512);
    }

    #[test]
    fn test_delta_not_applied_on_reject() {
        let (mut stage, mut sink) = make_stage();

        // Request 3 tiles > max_tiles(2)
        let result = stage.check_and_apply("agent-a", 3, 0, 0, Instant::now(), &mut sink);
        assert!(matches!(result.outcome, BudgetCheckOutcome::Reject(_)));

        let state = stage.enforcer.agent_state("agent-a").unwrap();
        assert_eq!(state.tile_count, 0, "rejected mutation should not update counters");
    }

    // ─── Post-revocation cleanup scheduling ────────────────────────────────

    #[test]
    fn test_cleanup_scheduled_on_revoke() {
        let (mut stage, mut sink) = make_stage();

        // Force a critical OOM revocation
        let result = stage.check_and_apply(
            "agent-a",
            0,
            3 * 1024 * 1024 * 1024_i64, // 3 GiB OOM attempt
            0,
            Instant::now(),
            &mut sink,
        );
        assert!(matches!(result.outcome, BudgetCheckOutcome::Revoke(_)));
        assert_eq!(stage.pending_cleanup_count(), 1);
    }

    #[test]
    fn test_duplicate_cleanup_not_scheduled() {
        // Force a revocation, then schedule cleanup manually a second time.
        // Only one cleanup entry should exist (dedup guard).
        let (mut stage, mut sink) = make_stage();
        let _ = stage.check_and_apply(
            "agent-a",
            0,
            3 * 1024 * 1024 * 1024_i64,
            0,
            Instant::now(),
            &mut sink,
        );
        assert_eq!(stage.pending_cleanup_count(), 1);

        // Schedule again manually — should be a no-op.
        stage.schedule_cleanup("agent-a".to_string(), Instant::now());
        assert_eq!(
            stage.pending_cleanup_count(),
            1,
            "duplicate cleanup for same namespace should not be added"
        );
    }

    #[test]
    fn test_cleanup_not_ready_before_delay() {
        let (mut stage, mut sink) = make_stage();

        // Force a revocation
        let _ = stage.check_and_apply(
            "agent-a",
            0,
            3 * 1024 * 1024 * 1024_i64,
            0,
            Instant::now(),
            &mut sink,
        );

        // Immediately drain — should not be ready yet (100ms default delay)
        let ready = stage.drain_ready_cleanups(Instant::now());
        assert!(
            ready.is_empty(),
            "cleanup should not be ready immediately (100ms delay)"
        );
    }

    #[test]
    fn test_cleanup_ready_after_delay() {
        // Use 0ms cleanup delay so the cleanup is immediately ready.
        let mut stage = MutationIntakeStage::with_cleanup_delay(Duration::from_millis(0));
        let mut sink = CollectingTelemetrySink::default();
        stage.register_session(SceneId::new(), "agent-b".to_string(), tight_budget());

        // Force a revocation
        let _ = stage.check_and_apply(
            "agent-b",
            0,
            3 * 1024 * 1024 * 1024_i64,
            0,
            Instant::now(),
            &mut sink,
        );

        // With 0ms delay, cleanup is immediately ready.
        let ready = stage.drain_ready_cleanups(Instant::now());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0], "agent-b");
        assert_eq!(stage.pending_cleanup_count(), 0);
    }

    // ─── Cleanup delay clamping ────────────────────────────────────────────

    #[test]
    fn test_cleanup_delay_clamped_to_max() {
        let stage = MutationIntakeStage::with_cleanup_delay(Duration::from_secs(9999));
        assert_eq!(
            stage.cleanup_delay,
            Duration::from_millis(MAX_POST_REVOCATION_CLEANUP_DELAY_MS)
        );
    }

    #[test]
    fn test_cleanup_delay_extreme_duration_clamped_safely() {
        // Duration::MAX would overflow u64 if cast via as_millis() as u64.
        // Verify it is clamped to MAX_POST_REVOCATION_CLEANUP_DELAY_MS without panic.
        let stage = MutationIntakeStage::with_cleanup_delay(Duration::MAX);
        assert_eq!(
            stage.cleanup_delay,
            Duration::from_millis(MAX_POST_REVOCATION_CLEANUP_DELAY_MS)
        );
    }

    #[test]
    fn test_cleanup_delay_zero_allowed() {
        let stage = MutationIntakeStage::with_cleanup_delay(Duration::from_millis(0));
        assert_eq!(stage.cleanup_delay, Duration::from_millis(0));
    }

    // ─── Tick-based cleanup scheduling ────────────────────────────────────

    #[test]
    fn test_tick_schedules_cleanup_for_revoked_sessions() {
        let mut stage = MutationIntakeStage::with_cleanup_delay(Duration::from_millis(0));
        let mut sink = CollectingTelemetrySink::default();
        stage.register_session(SceneId::new(), "agent-c".to_string(), tight_budget());

        // Force agent into throttled state past the revocation threshold (30s)
        {
            let state = stage.enforcer.agent_state_mut("agent-c").unwrap();
            state.tile_count = 3; // over budget
            state.budget_state = crate::budget::BudgetState::Throttled {
                throttled_since: Instant::now() - Duration::from_secs(35),
            };
        }

        let revoked = stage.tick(Instant::now(), &mut sink);
        assert!(revoked.contains(&"agent-c".to_string()));
        assert_eq!(stage.pending_cleanup_count(), 1);
    }

    // ─── Invariant violation fast path ────────────────────────────────────

    #[test]
    fn test_invariant_violation_schedules_cleanup_on_revoke() {
        let mut stage = MutationIntakeStage::with_cleanup_delay(Duration::from_millis(0));
        let mut sink = CollectingTelemetrySink::default();
        stage.register_session(SceneId::new(), "agent-d".to_string(), tight_budget());

        let mut revoked = false;
        for _ in 0..=10 {
            revoked = stage.report_invariant_violation("agent-d", Instant::now(), &mut sink);
        }
        assert!(revoked, "10 invariant violations should trigger revocation");
        assert_eq!(stage.pending_cleanup_count(), 1);
    }

    // ─── Default max tile enforcement (spec line 313) ─────────────────────

    #[test]
    fn test_default_max_tiles_8_rejects_9th_tile() {
        // Agent with default budget (max_tiles = 8)
        let mut stage = MutationIntakeStage::new();
        let mut sink = CollectingTelemetrySink::default();
        stage.register_session(SceneId::new(), "default-agent".to_string(), ResourceBudget::default());

        // Place agent at exactly 8 tiles (simulating 8 accepted tile creations).
        {
            let state = stage.enforcer.agent_state_mut("default-agent").unwrap();
            state.tile_count = 8;
        }

        // Attempt to create a 9th tile — must be rejected.
        let result = stage.check_and_apply(
            "default-agent",
            1, // +1 tile
            0,
            0,
            Instant::now(),
            &mut sink,
        );
        assert!(
            matches!(
                result.outcome,
                BudgetCheckOutcome::Reject(BudgetViolation::TileCountExceeded { .. })
            ),
            "9th tile creation with default budget (max_tiles=8) must be rejected"
        );
    }
}
