//! Headless runtime — runs the full pipeline without a display server.
//! Suitable for testing, CI, and artifact generation.

use tze_hud_compositor::{Compositor, HeadlessSurface};
use tze_hud_input::InputProcessor;
use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::session_server::HudSessionImpl;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_telemetry::{TelemetryCollector, FrameTelemetry};
use std::sync::Arc;
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
pub struct HeadlessRuntime {
    pub compositor: Compositor,
    pub surface: HeadlessSurface,
    pub input_processor: InputProcessor,
    pub telemetry: TelemetryCollector,
    pub state: Arc<Mutex<SharedState>>,
    pub config: HeadlessConfig,
}

impl HeadlessRuntime {
    /// Create a new headless runtime.
    pub async fn new(config: HeadlessConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let compositor = Compositor::new_headless(config.width, config.height).await?;
        let surface = HeadlessSurface::new(&compositor.device, config.width, config.height);

        let scene = SceneGraph::new(config.width as f32, config.height as f32);
        let sessions = tze_hud_protocol::session::SessionRegistry::new(&config.psk);
        let state = Arc::new(Mutex::new(SharedState { scene, sessions, safe_mode_active: false }));

        Ok(Self {
            compositor,
            surface,
            input_processor: InputProcessor::new(),
            telemetry: TelemetryCollector::new(),
            state,
            config,
        })
    }

    /// Get a reference to the shared state (scene + sessions).
    pub fn shared_state(&self) -> &Arc<Mutex<SharedState>> {
        &self.state
    }

    /// Run one frame: render the current scene to the headless surface.
    pub async fn render_frame(&mut self) -> FrameTelemetry {
        let state = self.state.lock().await;
        let telemetry = self.compositor.render_frame(&state.scene, &self.surface);
        drop(state);
        self.telemetry.record(telemetry.clone());
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
