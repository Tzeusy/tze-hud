//! Headless runtime — runs the full 8-stage frame pipeline without a display server.
//! Suitable for testing, CI, and artifact generation.
//!
//! The headless runtime runs all pipeline stages sequentially in the same task
//! (no cross-thread signalling). This is correct for testing because the pipeline
//! contract (stage order, per-stage telemetry, ArcSwap snapshot, overflow counter)
//! is identical to the windowed runtime; only the thread assignment differs.

use crate::pipeline::{FramePipeline, HitTestSnapshot};
use tze_hud_compositor::{Compositor, HeadlessSurface};
use tze_hud_input::InputProcessor;
use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::session_server::HudSessionImpl;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_telemetry::{TelemetryCollector, FrameTelemetry};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Configuration for the headless runtime.
pub struct HeadlessConfig {
    pub width: u32,
    pub height: u32,
    pub grpc_port: u16,
    pub psk: String,
}

impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            grpc_port: 50051,
            psk: "test-key".to_string(),
        }
    }
}

/// The headless runtime instance.
///
/// Owns the frame pipeline (including the ArcSwap hit-test snapshot),
/// the compositor, and the telemetry collector.
pub struct HeadlessRuntime {
    pub compositor: Compositor,
    pub surface: HeadlessSurface,
    pub input_processor: InputProcessor,
    pub telemetry: TelemetryCollector,
    pub state: Arc<Mutex<SharedState>>,
    pub config: HeadlessConfig,
    /// The 8-stage frame pipeline orchestrator.
    pub pipeline: FramePipeline,
}

impl HeadlessRuntime {
    /// Create a new headless runtime.
    pub async fn new(config: HeadlessConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let compositor = Compositor::new_headless(config.width, config.height).await?;
        let surface = HeadlessSurface::new(&compositor.device, config.width, config.height);

        let scene = SceneGraph::new(config.width as f32, config.height as f32);
        let sessions = tze_hud_protocol::session::SessionRegistry::new(&config.psk);
        let state = Arc::new(Mutex::new(SharedState {
            scene,
            sessions,
            safe_mode_active: false,
            token_store: tze_hud_protocol::token::TokenStore::new(),
            freeze_active: false,
            degradation_level: tze_hud_protocol::session::RuntimeDegradationLevel::Normal,
        }));

        Ok(Self {
            compositor,
            surface,
            input_processor: InputProcessor::new(),
            telemetry: TelemetryCollector::new(),
            state,
            config,
            pipeline: FramePipeline::new(),
        })
    }

    /// Get a reference to the shared state (scene + sessions).
    pub fn shared_state(&self) -> &Arc<Mutex<SharedState>> {
        &self.state
    }

    /// Run one frame through the full 8-stage pipeline.
    ///
    /// Stages 1-8 run sequentially in the calling task (no cross-thread signalling
    /// in headless mode). Per-stage telemetry is recorded in the returned
    /// `FrameTelemetry`.
    ///
    /// The compositor renders the current scene to the headless surface.
    pub async fn render_frame(&mut self) -> FrameTelemetry {
        let frame_start = Instant::now();
        let state = self.state.lock().await;
        let scene = &state.scene;

        // ── Capture compositor render work upfront ────────────────────────────
        // The headless pipeline runs all stages in-process. The compositor's
        // render_frame() handles stages 6 (encode) and 7 (submit) internally.
        // We split timing manually below.

        // Stage 1: Input Drain (headless — no OS event queue to drain)
        let s1_start = Instant::now();
        let stage1_us = s1_start.elapsed().as_micros() as u64;

        // Stage 2: Local Feedback (load ArcSwap snapshot — no mutex)
        // Per spec §3.2: Stage 2 reads the snapshot published after Stage 4 of the
        // *previous* frame. It must not publish a new snapshot (that is Stage 4's job).
        let s2_start = Instant::now();
        let _snap_ref = self.pipeline.hit_test_snapshot.load();
        let stage2_us = s2_start.elapsed().as_micros() as u64;

        // Stage 3: Mutation Intake (headless — mutations committed before render)
        let s3_start = Instant::now();
        let stage3_us = s3_start.elapsed().as_micros() as u64;

        // Stage 4: Scene Commit (headless — scene already committed)
        let s4_start = Instant::now();
        let new_snap = HitTestSnapshot::from_scene(scene);
        self.pipeline.hit_test_snapshot.store(Arc::new(new_snap));
        let stage4_us = s4_start.elapsed().as_micros() as u64;

        // Stage 5: Layout Resolve
        let s5_start = Instant::now();
        let tiles_visible = scene.visible_tiles().len() as u32;
        let stage5_us = s5_start.elapsed().as_micros() as u64;

        // Stages 6 + 7: Render Encode + GPU Submit (handled by Compositor::render_frame)
        let compositor_telemetry = self.compositor.render_frame(scene, &self.surface);
        // Total frame time from Compositor covers encode + submit
        let stage6_us = compositor_telemetry.render_encode_us;
        let stage7_us = compositor_telemetry.gpu_submit_us;

        // Total frame time: stage 1 start → stage 7 end
        let frame_time_us = frame_start.elapsed().as_micros() as u64;

        // Build the per-stage telemetry record (outside the Stage 8 timed region)
        let mut telemetry = FrameTelemetry::new(compositor_telemetry.frame_number);
        telemetry.stage1_input_drain_us = stage1_us;
        telemetry.stage2_local_feedback_us = stage2_us;
        telemetry.stage3_mutation_intake_us = stage3_us;
        telemetry.stage4_scene_commit_us = stage4_us;
        telemetry.stage5_layout_resolve_us = stage5_us;
        telemetry.stage6_render_encode_us = stage6_us;
        telemetry.stage7_gpu_submit_us = stage7_us;
        telemetry.frame_time_us = frame_time_us;
        telemetry.tile_count = tiles_visible;
        telemetry.node_count = compositor_telemetry.node_count;
        telemetry.active_leases = compositor_telemetry.active_leases;
        telemetry.mutations_applied = compositor_telemetry.mutations_applied;
        telemetry.hit_region_updates = compositor_telemetry.hit_region_updates;
        telemetry.telemetry_overflow_count =
            self.pipeline.telemetry_overflow_count();
        telemetry.sync_legacy_aliases();

        drop(state);

        // Stage 8: Telemetry Emit — non-blocking record into collector.
        // Timer wraps the actual emit so the measurement reflects its cost.
        let s8_start = Instant::now();
        self.telemetry.record(telemetry.clone());
        telemetry.stage8_telemetry_emit_us = s8_start.elapsed().as_micros() as u64;

        telemetry
    }

    /// Read back pixels from the last rendered frame.
    pub fn read_pixels(&self) -> Vec<u8> {
        self.surface.read_pixels(&self.compositor.device)
    }

    /// Start the gRPC server in the background, serving the HudSession streaming service.
    /// Returns the server task handle.
    pub async fn start_grpc_server(
        &self,
    ) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
        let addr = format!("[::1]:{}", self.config.grpc_port).parse()?;

        let service = HudSessionImpl::from_shared_state(
            self.state.clone(),
            &self.config.psk,
        );

        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve(addr)
                .await
                .expect("gRPC server failed");
        });

        // Give the server a moment to bind
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Ok(handle)
    }
}
