//! Headless runtime — runs the full 8-stage frame pipeline without a display server.
//! Suitable for testing, CI, and artifact generation.
//!
//! The headless runtime runs all pipeline stages sequentially in the same task
//! (no cross-thread signalling). This is correct for testing because the pipeline
//! contract (stage order, per-stage telemetry, ArcSwap snapshot, overflow counter)
//! is identical to the windowed runtime; only the thread assignment differs.
//!
//! # Headless Mode Parity
//!
//! Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
//! "Headless mode MUST use the same process, code path, and pipeline as
//! windowed mode.  The only difference SHALL be the render surface."
//!
//! `HeadlessRuntime` wires the full pipeline:
//! - `Compositor` (GPU device + wgpu render pipeline)
//! - `HeadlessSurface` (offscreen render target, `present()` is a no-op)
//! - `InputProcessor` (hit-test, local feedback)
//! - `TelemetryCollector` (per-frame telemetry)
//! - `SharedState` (scene + session registry)
//! - gRPC server (HudSession streaming — optional)
//!
//! # Software GPU
//!
//! `HeadlessRuntime::new` respects the `HEADLESS_FORCE_SOFTWARE` environment
//! variable (spec line 409).  When set to `1`, wgpu adapter selection uses
//! `force_fallback_adapter = true` (llvmpipe on Linux, WARP on Windows).
//!
//! # Session Limits & Hot-Connect
//!
//! Per spec Requirement: Session Limits (line 355): headless mode still runs
//! the gRPC session server — agents connect normally, session limits are
//! enforced identically.
//!
//! Per spec Requirement: Hot-Connect (line 346): agents connecting to headless
//! runtime receive the full scene snapshot (handled by `HudSessionImpl`).
//!
//! # grpc_port = 0
//!
//! Setting `grpc_port = 0` in `HeadlessConfig` disables the gRPC server.
//! Tests that don't exercise the session layer use this to skip server startup.

use crate::pipeline::{FramePipeline, HitTestSnapshot};
use crate::reload_triggers::{RuntimeServiceImpl, spawn_sighup_listener};
use crate::runtime_context::{FallbackPolicy, RuntimeContext};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tze_hud_compositor::{Compositor, HeadlessSurface};
use tze_hud_config::TzeHudConfig;
use tze_hud_input::InputProcessor;
use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
use tze_hud_protocol::proto::session::runtime_service_server::RuntimeServiceServer;
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::session_server::HudSessionImpl;
use tze_hud_scene::config::ConfigLoader;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_telemetry::{FrameTelemetry, TelemetryCollector};
use wgpu::TextureFormat;

/// Configuration for the headless runtime.
pub struct HeadlessConfig {
    /// Render target width in pixels.
    pub width: u32,
    /// Render target height in pixels.
    pub height: u32,
    /// gRPC server port.  Set to `0` to disable the gRPC server entirely
    /// (useful for tests that only need rendering, not session management).
    pub grpc_port: u16,
    /// Pre-shared key for session authentication.
    pub psk: String,
    /// Optional TOML config string to load.
    ///
    /// When `Some(toml)`, the runtime parses and validates it, building a
    /// `RuntimeContext` that drives per-agent capability grants and profile
    /// budgets.  If parsing or validation fails, falls back to headless-default
    /// with `fallback_unrestricted = false` (guest policy / fail-safe).
    ///
    /// When `None`:
    /// - Under `cfg(any(test, feature = "dev-mode"))`: a headless-profile
    ///   `RuntimeContext` is used with `fallback_unrestricted = true` (dev mode,
    ///   all agents get unrestricted capabilities).
    /// - In production builds (without `dev-mode` feature): startup **fails**
    ///   with an error. The runtime requires explicit config to enforce capability
    ///   governance.
    pub config_toml: Option<String>,
}

/// `HeadlessConfig::default()` is only available under `cfg(test)` or with the
/// `dev-mode` feature enabled, because the default sets `config_toml: None`
/// which bypasses config governance (unrestricted capability grants).
///
/// Production code must construct `HeadlessConfig` explicitly and supply a
/// `config_toml` value.
#[cfg(any(test, feature = "dev-mode"))]
impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            grpc_port: 50051,
            psk: "test-key".to_string(),
            config_toml: None,
        }
    }
}

impl HeadlessConfig {
    /// Build a `RuntimeContext` from the optional TOML config.
    ///
    /// Returns `Ok((context, fallback_unrestricted))` on success.
    ///
    /// If `config_toml` is `None`:
    /// - Under `cfg(any(test, feature = "dev-mode"))`: returns a headless-default
    ///   context with `fallback_unrestricted = true` (all agents allowed, dev-mode).
    /// - In production builds: returns `Err` — config is required to enforce
    ///   capability governance.
    ///
    /// If `config_toml` is `Some(toml)` but parsing, validation, or freezing
    /// fails, logs warnings and falls back to the headless default with
    /// `fallback_unrestricted = false` (guest policy / fail-safe). A caller
    /// that supplied a config string intended to run with restricted capabilities;
    /// silently upgrading to unrestricted on parse failure would be a security
    /// regression.
    pub fn build_runtime_context(
        &self,
    ) -> Result<(RuntimeContext, bool), Box<dyn std::error::Error>> {
        match &self.config_toml {
            None => {
                #[cfg(any(test, feature = "dev-mode"))]
                {
                    Ok((RuntimeContext::headless_default(), true))
                }
                #[cfg(not(any(test, feature = "dev-mode")))]
                {
                    return Err(
                        "HeadlessConfig: config_toml is None but the `dev-mode` feature is not \
                         enabled. Production builds require an explicit config to enforce \
                         capability governance. Supply a TOML config string via \
                         `HeadlessConfig { config_toml: Some(toml), .. }` or enable the \
                         `dev-mode` feature for development use."
                            .into(),
                    );
                }
            }
            Some(toml) => match TzeHudConfig::parse(toml) {
                Err(e) => {
                    tracing::warn!(
                        parse_error = %e.message,
                        line = e.line,
                        column = e.column,
                        "HeadlessConfig: TOML parse error; using headless-default RuntimeContext (guest fallback)"
                    );
                    Ok((RuntimeContext::headless_default(), false))
                }
                Ok(mut loader) => {
                    loader.normalize();
                    let errors = loader.validate();
                    if !errors.is_empty() {
                        tracing::warn!(
                            error_count = errors.len(),
                            "HeadlessConfig: config validation errors; using headless-default RuntimeContext (guest fallback)"
                        );
                        return Ok((RuntimeContext::headless_default(), false));
                    }
                    match loader.freeze() {
                        Ok(resolved) => {
                            tracing::info!(
                                profile = %resolved.profile.name,
                                agent_count = resolved.agent_capabilities.len(),
                                "HeadlessConfig: loaded RuntimeContext from config"
                            );
                            let ctx = RuntimeContext::from_config(resolved, FallbackPolicy::Guest);
                            Ok((ctx, false))
                        }
                        Err(errors) => {
                            tracing::warn!(
                                error_count = errors.len(),
                                "HeadlessConfig: config freeze errors; using headless-default RuntimeContext (guest fallback)"
                            );
                            Ok((RuntimeContext::headless_default(), false))
                        }
                    }
                }
            },
        }
    }
}

/// The headless runtime instance.
///
/// Owns all runtime state: GPU compositor, offscreen surface, input processor,
/// telemetry collector, and scene/session state.
/// Includes the frame pipeline orchestrator (ArcSwap hit-test snapshot,
/// overflow counter, per-stage telemetry).
pub struct HeadlessRuntime {
    pub compositor: Compositor,
    pub surface: HeadlessSurface,
    pub input_processor: InputProcessor,
    pub telemetry: TelemetryCollector,
    pub state: Arc<Mutex<SharedState>>,
    pub config: HeadlessConfig,
    /// The 8-stage frame pipeline orchestrator.
    pub pipeline: FramePipeline,
    /// Immutable runtime context built from validated config at startup.
    ///
    /// Holds profile budgets and per-agent capability grants.
    pub runtime_context: Arc<RuntimeContext>,
    /// Whether unknown agents get unrestricted capabilities (true = dev mode).
    fallback_unrestricted: bool,
}

impl HeadlessRuntime {
    /// Create a new headless runtime.
    ///
    /// Respects `HEADLESS_FORCE_SOFTWARE=1` — when set, the wgpu adapter
    /// selection uses `force_fallback_adapter = true` (spec line 211).
    ///
    /// If `config.config_toml` is provided, the runtime builds an immutable
    /// `RuntimeContext` from the loaded config (profile budgets, per-agent
    /// capability grants). If config is present but fails to parse/validate,
    /// falls back to headless-default with `fallback_unrestricted = false`
    /// (guest policy).
    ///
    /// If `config.config_toml` is `None`:
    /// - Under `cfg(any(test, feature = "dev-mode"))`: uses headless-default
    ///   with `fallback_unrestricted = true` (dev mode, all agents unrestricted).
    /// - In production builds (without `dev-mode`): returns `Err` immediately —
    ///   the runtime refuses to start without an explicit config.
    pub async fn new(config: HeadlessConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // Build RuntimeContext at startup from loaded config (or default).
        // Returns Err if config_toml is None in a production build (dev-mode not enabled).
        let (runtime_ctx, fallback_unrestricted) = config.build_runtime_context()?;
        let runtime_context = Arc::new(runtime_ctx);

        let mut compositor = Compositor::new_headless(config.width, config.height).await?;
        let surface = HeadlessSurface::new(&compositor.device, config.width, config.height);

        // ── Initialize text renderer ──────────────────────────────────────
        // HeadlessSurface always uses Rgba8UnormSrgb. Must be called here so
        // glyphon text rendering is active for all runtime paths (not just tests).
        compositor.init_text_renderer(TextureFormat::Rgba8UnormSrgb);
        tracing::debug!("headless: text renderer initialized");

        let scene = Arc::new(Mutex::new(SceneGraph::new(
            config.width as f32,
            config.height as f32,
        )));
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
            runtime_context,
            fallback_unrestricted,
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
    /// The compositor renders the current scene to the headless surface via
    /// `render_frame_headless()`, which includes the `copy_to_buffer` step so
    /// that `read_pixels()` returns actual rendered pixel data after this call.
    pub async fn render_frame(&mut self) -> FrameTelemetry {
        let frame_start = Instant::now();
        let state = self.state.lock().await;
        // Clone the Arc so we can release the SharedState lock before rendering.
        let scene_arc = state.scene.clone();
        drop(state);
        let mut scene_guard = scene_arc.lock().await;
        // Per timing-model/spec.md §Expiration Policy: expired zone
        // publications MUST be cleared before the next frame.
        scene_guard.drain_expired_zone_publications();
        let scene = &*scene_guard;

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
        // input_to_scene_commit: wall time from frame_start to end of Stage 4.
        // Measured as elapsed since frame_start (not a sum of stage durations)
        // so that any inter-stage overhead is included in the measurement.
        let input_to_scene_commit_us = frame_start.elapsed().as_micros() as u64;

        // Stage 5: Layout Resolve
        let s5_start = Instant::now();
        let tiles_visible = scene.visible_tiles().len() as u32;
        let stage5_us = s5_start.elapsed().as_micros() as u64;

        // Stages 6 + 7: Render Encode + GPU Submit (handled by Compositor::render_frame_headless)
        // Must use render_frame_headless (not render_frame) so copy_to_buffer is called
        // before queue.submit(), making read_pixels() return actual rendered pixel data.
        let compositor_telemetry = self.compositor.render_frame_headless(scene, &self.surface);
        // Total frame time from Compositor covers encode + submit
        let stage6_us = compositor_telemetry.render_encode_us;
        let stage7_us = compositor_telemetry.gpu_submit_us;

        // Total frame time: stage 1 start → stage 7 end
        let frame_time_us = frame_start.elapsed().as_micros() as u64;
        // input_to_next_present: time from frame start (proxy for input event
        // arrival) to Stage 7 completion (GPU present). Equals total frame time
        // since the frame pipeline starts at the input drain boundary.
        let input_to_next_present_us = frame_time_us;

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
        // ── Split input latency fields ─────────────────────────────────────
        // input_to_local_ack_us is populated externally by the input processor
        // (via input_processor.process() → InputProcessResult::local_ack_us) and
        // recorded into the summary by callers. The FrameTelemetry field is left
        // at 0 here because headless render_frame() has no input event to ack.
        //
        // input_to_scene_commit_us and input_to_next_present_us are gated on
        // mutations_applied > 0, keeping the documented semantics ("0 when no
        // input/agent response occurred this frame") consistent across runtimes.
        let had_scene_commit = compositor_telemetry.mutations_applied > 0;
        telemetry.input_to_local_ack_us = 0; // populated by input_processor callers
        telemetry.input_to_scene_commit_us = if had_scene_commit {
            input_to_scene_commit_us
        } else {
            0
        };
        telemetry.input_to_next_present_us = if had_scene_commit {
            input_to_next_present_us
        } else {
            0
        };
        telemetry.tile_count = tiles_visible;
        telemetry.node_count = compositor_telemetry.node_count;
        telemetry.active_leases = compositor_telemetry.active_leases;
        telemetry.mutations_applied = compositor_telemetry.mutations_applied;
        telemetry.hit_region_updates = compositor_telemetry.hit_region_updates;
        telemetry.telemetry_overflow_count = self.pipeline.telemetry_overflow_count();
        telemetry.sync_legacy_aliases();

        // Stage 8: Telemetry Emit — non-blocking record into collector.
        // Timer wraps the actual emit so the measurement reflects its cost.
        let s8_start = Instant::now();
        self.telemetry.record(telemetry.clone());
        telemetry.stage8_telemetry_emit_us = s8_start.elapsed().as_micros() as u64;

        telemetry
    }

    /// Read back pixels from the last rendered frame.
    ///
    /// Returns RGBA8 data (width × height × 4 bytes).  Blocks until GPU is idle.
    ///
    /// Per spec line 208: "pixel readback MUST be on-demand via copy_texture_to_buffer."
    pub fn read_pixels(&self) -> Vec<u8> {
        self.surface.read_pixels(&self.compositor.device)
    }

    /// Start the gRPC server in the background, serving the HudSession streaming service.
    ///
    /// Per spec Requirement: Session Limits (line 355): headless runtime still
    /// runs the gRPC server — agents connect normally.
    ///
    /// Per spec Requirement: Hot-Connect (line 346): the `HudSessionImpl` sends
    /// a full scene snapshot to each agent on connect.
    ///
    /// Returns the server task handle.  The caller must retain it to keep the
    /// server running.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `config.grpc_port == 0` (gRPC server disabled) or if
    /// the gRPC server fails to bind.
    pub async fn start_grpc_server(
        &self,
    ) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
        if self.config.grpc_port == 0 {
            return Err("start_grpc_server: grpc_port = 0 (gRPC server disabled)".into());
        }
        let addr = format!("[::1]:{}", self.config.grpc_port).parse()?;

        // Wire config-driven capability registry into the session service.
        // Snapshot the agent capability map from RuntimeContext (one-time clone at startup).
        // Per configuration/spec.md §Requirement: Agent Registration (lines 136-147):
        // registered agents get their configured capability set; unlisted agents get
        // guest policy (no capabilities) unless fallback_unrestricted is true (dev mode).
        let agent_caps = self.runtime_context.snapshot_agent_capabilities();

        let service = HudSessionImpl::from_shared_state_with_config(
            self.state.clone(),
            &self.config.psk,
            agent_caps,
            self.fallback_unrestricted,
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

    /// Start the `RuntimeService` gRPC server in the background.
    ///
    /// Satisfies RFC 0006 §9 (v1-mandatory) gRPC trigger requirement.
    ///
    /// The `RuntimeService.ReloadConfig` RPC accepts a TOML string, validates it,
    /// and atomically applies the hot-reloadable sections via `RuntimeContext`.
    ///
    /// The server binds to `addr` (e.g. `"[::1]:50052"`). It is independent of
    /// the `HudSession` gRPC server — you may co-host them on the same port via
    /// `tonic::transport::Server::builder().add_service(...).add_service(...)`.
    ///
    /// Returns the server task handle. The caller must retain it to keep the
    /// server running (dropping it keeps the task alive; use `.abort()` to stop).
    pub async fn start_runtime_service_server(
        &self,
        addr: std::net::SocketAddr,
    ) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
        let svc = RuntimeServiceImpl::new(Arc::clone(&self.runtime_context));
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(RuntimeServiceServer::new(svc))
                .serve(addr)
                .await
                .expect("RuntimeService gRPC server failed");
        });

        // Give the server a moment to bind.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Ok(handle)
    }

    /// Install a SIGHUP listener that hot-reloads config from `config_path`.
    ///
    /// Satisfies RFC 0006 §9 (v1-mandatory) SIGHUP trigger requirement.
    ///
    /// On Unix: spawns a Tokio task that waits for SIGHUP signals and reloads
    /// the config file at `config_path`, applying hot-reloadable sections via
    /// `RuntimeContext`.
    ///
    /// On non-Unix (Windows): this is a no-op stub — use `ReloadConfig` gRPC instead.
    ///
    /// Returns the task handle. Dropping the handle keeps the task running.
    pub fn start_sighup_listener(
        &self,
        config_path: impl Into<String> + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        spawn_sighup_listener(Arc::clone(&self.runtime_context), config_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that grpc_port = 0 does not start a server by default.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_headless_runtime_no_grpc() {
        let config = HeadlessConfig {
            width: 64,
            height: 64,
            grpc_port: 0,
            psk: "test".to_string(),
            config_toml: None,
        };
        let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");
        // render_frame should succeed without a gRPC server
        let telemetry = runtime.render_frame().await;
        assert!(telemetry.frame_time_us > 0, "frame time must be non-zero");
    }

    /// Verify that read_pixels returns the correct buffer size after a render.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_headless_read_pixels_buffer_size() {
        let config = HeadlessConfig {
            width: 128,
            height: 96,
            grpc_port: 0,
            psk: "test".to_string(),
            config_toml: None,
        };
        let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");
        runtime.render_frame().await;
        let pixels = runtime.read_pixels();
        assert_eq!(
            pixels.len(),
            128 * 96 * 4,
            "pixel buffer must be width * height * 4 bytes (RGBA8)"
        );
    }

    /// Verify that read_pixels returns actual rendered content after render_frame().
    ///
    /// Regression test for the bug where render_frame() called compositor.render_frame()
    /// instead of render_frame_headless(), causing copy_to_buffer to be skipped and
    /// read_pixels() to return stale/zero data.
    ///
    /// The clear color in render_frame_headless is (r:0.05, g:0.05, b:0.1, a:1.0) in
    /// linear space, which in Rgba8UnormSrgb encoding is approximately (48, 48, 80, 255).
    /// The alpha channel MUST be 255 (fully opaque). Any all-zero pixel data indicates
    /// the copy_to_buffer step was skipped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_headless_read_pixels_content_after_render() {
        let config = HeadlessConfig {
            width: 64,
            height: 64,
            grpc_port: 0,
            psk: "test".to_string(),
            config_toml: None,
        };
        let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");
        runtime.render_frame().await;
        let pixels = runtime.read_pixels();

        assert_eq!(
            pixels.len(),
            64 * 64 * 4,
            "pixel buffer must be width * height * 4 bytes"
        );

        // Verify pixels are not all zero — all-zero means copy_to_buffer was skipped
        // (the bug this test guards against: using render_frame instead of render_frame_headless).
        let all_zero = pixels.iter().all(|&b| b == 0);
        assert!(
            !all_zero,
            "read_pixels() returned all-zero data: copy_to_buffer was likely skipped. \
             render_frame() must call render_frame_headless() for correct pixel readback."
        );

        // Verify alpha channel is 255 (fully opaque clear color).
        // RGBA8 layout: [R, G, B, A, R, G, B, A, ...]
        let alpha_nonopaque = pixels.chunks(4).any(|px| px[3] != 255);
        assert!(
            !alpha_nonopaque,
            "expected fully-opaque pixels (alpha=255) from clear color render, got non-255 alpha"
        );
    }

    /// Verify that the gRPC server starts and the runtime remains functional
    /// after startup.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_headless_grpc_server_starts() {
        // Bind to port 0 to get an ephemeral port, then release the listener
        // before tonic binds. This avoids hardcoded ports that may conflict in CI.
        let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
        let free_port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = HeadlessConfig {
            width: 64,
            height: 64,
            grpc_port: free_port,
            psk: "test".to_string(),
            config_toml: None,
        };
        let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");
        let _server = runtime.start_grpc_server().await.expect("server start");

        // Render a frame while the server is running
        let telemetry = runtime.render_frame().await;
        assert!(telemetry.frame_time_us > 0);

        // Server task is still running (not panicked)
        assert!(
            !_server.is_finished(),
            "gRPC server task should still be running"
        );
        _server.abort();
    }

    /// Verify that config_toml = None produces a headless-default RuntimeContext
    /// with fallback_unrestricted = true (dev mode).
    ///
    /// This path is only reachable under cfg(test) or the `dev-mode` feature;
    /// in production builds, `build_runtime_context()` returns `Err` when
    /// `config_toml` is `None`.
    #[test]
    fn test_build_runtime_context_none_is_dev_mode() {
        let config = HeadlessConfig {
            width: 64,
            height: 64,
            grpc_port: 0,
            psk: "test".to_string(),
            config_toml: None,
        };
        let (ctx, fallback) = config
            .build_runtime_context()
            .expect("build_runtime_context with None should succeed under cfg(test)");
        assert_eq!(ctx.profile.name, "headless");
        assert!(
            fallback,
            "config_toml = None should set fallback_unrestricted = true"
        );
    }

    /// Verify that a valid TOML config produces a proper RuntimeContext with
    /// per-agent capabilities and fallback_unrestricted = false.
    ///
    /// This test verifies the config-driven capability gating path
    /// (configuration/spec.md §Requirement: Agent Registration, lines 136-147).
    #[test]
    fn test_build_runtime_context_from_valid_toml() {
        let toml = r#"
[runtime]
profile = "headless"

[[tabs]]
name = "Main"
default_tab = true

[agents.registered.weather-agent]
capabilities = ["create_tiles", "modify_own_tiles", "read_scene_topology"]

[agents.registered.monitor-agent]
capabilities = ["read_telemetry", "read_scene_topology"]
"#;

        let config = HeadlessConfig {
            width: 64,
            height: 64,
            grpc_port: 0,
            psk: "test".to_string(),
            config_toml: Some(toml.to_string()),
        };

        let (ctx, fallback) = config
            .build_runtime_context()
            .expect("build_runtime_context should succeed with valid TOML");
        assert_eq!(ctx.profile.name, "headless");
        assert!(
            !fallback,
            "valid config should set fallback_unrestricted = false"
        );

        // weather-agent gets its configured capabilities
        let policy = ctx.capability_policy_for("weather-agent");
        assert!(
            !policy.is_unrestricted(),
            "registered agent should not be unrestricted"
        );
        assert!(
            policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_ok(),
            "weather-agent should be granted create_tiles"
        );
        assert!(
            policy
                .evaluate_capability_request(&["read_telemetry".to_string()])
                .is_err(),
            "weather-agent should be denied read_telemetry (not in its list)"
        );

        // monitor-agent gets its configured capabilities
        let monitor_policy = ctx.capability_policy_for("monitor-agent");
        assert!(
            monitor_policy
                .evaluate_capability_request(&["read_telemetry".to_string()])
                .is_ok(),
            "monitor-agent should be granted read_telemetry"
        );
        assert!(
            monitor_policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_err(),
            "monitor-agent should be denied create_tiles"
        );

        // unknown agent gets guest policy (no capabilities) since fallback = Guest
        let guest_policy = ctx.capability_policy_for("unknown-agent");
        assert!(
            !guest_policy.is_unrestricted(),
            "unknown agent should get guest policy"
        );
        assert!(
            guest_policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_err(),
            "unknown agent should be denied all capabilities"
        );
    }

    /// Verify that a malformed TOML falls back to the guest (fail-safe) policy, NOT dev mode.
    ///
    /// When a config string is provided but cannot be parsed, the caller intended
    /// to run with restricted capabilities. Silently upgrading to unrestricted on
    /// parse failure would be a security regression.
    #[test]
    fn test_build_runtime_context_malformed_toml_falls_back_to_guest() {
        let config = HeadlessConfig {
            width: 64,
            height: 64,
            grpc_port: 0,
            psk: "test".to_string(),
            config_toml: Some("this is not valid TOML %%%".to_string()),
        };
        let (ctx, fallback) = config.build_runtime_context().expect(
            "build_runtime_context should return Ok (guest fallback) even for malformed TOML",
        );
        assert_eq!(ctx.profile.name, "headless");
        assert!(
            !fallback,
            "malformed TOML should fall back to guest policy (fallback_unrestricted = false), not dev mode"
        );
    }
}
