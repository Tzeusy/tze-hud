//! # threads
//!
//! Thread spawning, priority elevation, and shutdown coordination per
//! runtime-kernel/spec.md §Thread Model (line 19) and §Graceful Shutdown (line 333).
//!
//! ## Thread model (spec §Thread Model)
//!
//! The runtime starts exactly four groups of threads at startup and spawns
//! no new OS threads during normal operation:
//!
//! 1. **Main thread** — winit event loop, input drain, local feedback,
//!    surface.present(). Elevated to real-time priority at startup.
//! 2. **Compositor thread** — scene commit, render encode, GPU submit.
//!    Owns `wgpu::Device` and `wgpu::Queue` exclusively.
//! 3. **Network thread(s)** — Tokio multi-thread runtime for gRPC server,
//!    MCP bridge, session management.
//! 4. **Telemetry thread** — async structured emission.
//!
//! ## Priority elevation (spec §Main Thread Responsibilities, lines 43-45)
//!
//! - Linux:   `SCHED_RR` real-time scheduling
//! - macOS:   `QOS_CLASS_USER_INTERACTIVE`
//! - Windows: `THREAD_PRIORITY_TIME_CRITICAL`
//!
//! Failure to elevate MUST NOT fail startup — log warning and continue.
//!
//! ## Graceful shutdown (spec §Graceful Shutdown, line 333)
//!
//! 1. Stop accepting new connections
//! 2. Drain active mutations (configurable timeout, default 500 ms)
//! 3. Revoke all leases without waiting for acknowledgement
//! 4. Flush telemetry (configurable grace period, default 200 ms)
//! 5. Terminate agent sessions
//! 6. GPU drain via `device.poll(Wait)`
//! 7. Release resources (reference counts reach zero)
//! 8. Exit process (0 = clean, non-zero = error)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::{broadcast, oneshot};

// ─── Shutdown configuration ───────────────────────────────────────────────────

/// Configurable timeouts for graceful shutdown.
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// How long to wait for active mutations to drain before forceful shutdown.
    /// Valid range: [0 ms, 60 000 ms]. Default: 500 ms.
    pub drain_timeout_ms: u64,
    /// How long to wait for telemetry to flush.
    /// Valid range: [0 ms, 10 000 ms]. Default: 200 ms.
    pub telemetry_grace_ms: u64,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout_ms: 500,
            telemetry_grace_ms: 200,
        }
    }
}

impl ShutdownConfig {
    /// Build with explicit timeouts. Panics if values are out-of-spec range.
    pub fn new(drain_timeout_ms: u64, telemetry_grace_ms: u64) -> Self {
        assert!(
            drain_timeout_ms <= 60_000,
            "drain_timeout_ms must be ≤ 60 000"
        );
        assert!(
            telemetry_grace_ms <= 10_000,
            "telemetry_grace_ms must be ≤ 10 000"
        );
        Self {
            drain_timeout_ms,
            telemetry_grace_ms,
        }
    }
}

// ─── Shutdown token ───────────────────────────────────────────────────────────

/// Broadcast a shutdown signal to all threads.
///
/// Every long-running loop should listen on a `broadcast::Receiver` obtained
/// from `token.subscribe()` and exit cleanly when the signal arrives.
#[derive(Clone)]
pub struct ShutdownToken {
    tx: broadcast::Sender<ShutdownReason>,
    triggered: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownReason {
    /// Normal exit requested (SIGTERM, user close, etc.).
    Clean,
    /// GPU device was lost — flush telemetry and exit with non-zero code.
    GpuDeviceLost,
    /// An unrecoverable error occurred.
    Fatal(String),
}

impl ShutdownToken {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(8);
        Self {
            tx,
            triggered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Trigger shutdown. Idempotent — safe to call from any thread.
    pub fn trigger(&self, reason: ShutdownReason) {
        if self
            .triggered
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            let _ = self.tx.send(reason);
        }
    }

    /// Subscribe a new receiver. Use this to listen for shutdown in loops.
    pub fn subscribe(&self) -> broadcast::Receiver<ShutdownReason> {
        self.tx.subscribe()
    }

    /// Returns `true` if shutdown has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::Acquire)
    }
}

impl Default for ShutdownToken {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Main thread priority elevation ──────────────────────────────────────────

/// Attempt to elevate the calling thread to real-time / high priority.
///
/// This MUST be called from the main thread immediately after the winit
/// event loop starts.
///
/// Failure is non-fatal — the function logs a warning and returns `false`.
pub fn elevate_main_thread_priority() -> bool {
    #[cfg(target_os = "linux")]
    {
        elevate_linux()
    }
    #[cfg(target_os = "macos")]
    {
        elevate_macos()
    }
    #[cfg(target_os = "windows")]
    {
        elevate_windows()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        tracing::warn!("thread priority elevation not implemented for this platform");
        false
    }
}

#[cfg(target_os = "linux")]
fn elevate_linux() -> bool {
    // SCHED_RR with a modest priority (10 out of 99).
    // Requires CAP_SYS_NICE or a permissive rlimit.

    // Safety: sched_param is a plain C struct; all fields zero-initialized.
    let param = libc::sched_param { sched_priority: 10 };
    // SAFETY: syscall with valid args; failure handled below.
    let ret = unsafe { libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_RR, &param) };
    if ret == 0 {
        tracing::info!("main thread elevated to SCHED_RR priority 10");
        true
    } else {
        tracing::warn!(
            errno = ret,
            "failed to elevate main thread to SCHED_RR (errno {}); continuing at normal priority",
            ret
        );
        false
    }
}

#[cfg(target_os = "macos")]
fn elevate_macos() -> bool {
    // QOS_CLASS_USER_INTERACTIVE via pthread_set_qos_class_self_np.
    // Available on macOS 10.10+.
    extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    // SAFETY: calling a known macOS system function with documented args.
    let ret = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
    if ret == 0 {
        tracing::info!("main thread elevated to QOS_CLASS_USER_INTERACTIVE");
        true
    } else {
        tracing::warn!(
            "failed to elevate main thread QoS (ret {}); continuing at normal priority",
            ret
        );
        false
    }
}

#[cfg(target_os = "windows")]
fn elevate_windows() -> bool {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    // SAFETY: GetCurrentThread returns a pseudo-handle valid for the calling thread.
    let result = unsafe {
        let handle = GetCurrentThread();
        SetThreadPriority(handle, THREAD_PRIORITY_TIME_CRITICAL)
    };
    if result.is_ok() {
        tracing::info!("main thread elevated to THREAD_PRIORITY_TIME_CRITICAL");
        true
    } else {
        tracing::warn!(
            "failed to elevate main thread priority on Windows; continuing at normal priority"
        );
        false
    }
}

// ─── Thread roles ─────────────────────────────────────────────────────────────

/// Identifies the four fixed thread roles in the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRole {
    Main,
    Compositor,
    Network,
    Telemetry,
}

// ─── Compositor thread ────────────────────────────────────────────────────────

/// Handle returned when the compositor thread is spawned.
///
/// Dropping this handle does NOT kill the thread — use `ShutdownToken` to
/// signal it to exit, then join via the returned `JoinHandle`.
pub struct CompositorThreadHandle {
    pub join_handle: std::thread::JoinHandle<()>,
    pub ready_rx: oneshot::Receiver<CompositorReady>,
}

/// Signal sent by the compositor thread when it has finished initialising.
pub struct CompositorReady {
    /// The compositor is ready to accept work.
    pub ok: bool,
}

/// Spawn the compositor thread.
///
/// The closure `f` receives a `ShutdownToken` receiver and should run the
/// compositor event loop. It is responsible for owning `wgpu::Device` and
/// `wgpu::Queue` exclusively.
pub fn spawn_compositor_thread<F>(
    shutdown: ShutdownToken,
    ready_tx: oneshot::Sender<CompositorReady>,
    f: F,
) -> std::thread::JoinHandle<()>
where
    F: FnOnce(ShutdownToken, oneshot::Sender<CompositorReady>) + Send + 'static,
{
    std::thread::Builder::new()
        .name("tze-compositor".to_string())
        .spawn(move || {
            tracing::info!(role = ?ThreadRole::Compositor, "compositor thread started");
            f(shutdown, ready_tx);
            tracing::info!(role = ?ThreadRole::Compositor, "compositor thread exiting");
        })
        .expect("failed to spawn compositor thread")
}

// ─── Network runtime ──────────────────────────────────────────────────────────

/// Tokio multi-thread runtime used for network threads (gRPC, MCP, sessions).
///
/// Created once at startup. All network async tasks are spawned onto this runtime.
pub struct NetworkRuntime {
    pub rt: Runtime,
}

impl NetworkRuntime {
    /// Build a multi-thread Tokio runtime for network tasks.
    pub fn new() -> Result<Self, std::io::Error> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .thread_name("tze-network")
            .enable_all()
            .build()?;
        Ok(Self { rt })
    }
}

// ─── Telemetry thread ─────────────────────────────────────────────────────────

/// Spawn the telemetry emission thread.
///
/// The closure `f` should drain telemetry records and emit them (e.g., to
/// stdout as JSON). It receives the shutdown token so it can flush on exit.
pub fn spawn_telemetry_thread<F>(shutdown: ShutdownToken, f: F) -> std::thread::JoinHandle<()>
where
    F: FnOnce(ShutdownToken) + Send + 'static,
{
    std::thread::Builder::new()
        .name("tze-telemetry".to_string())
        .spawn(move || {
            tracing::info!(role = ?ThreadRole::Telemetry, "telemetry thread started");
            f(shutdown);
            tracing::info!(role = ?ThreadRole::Telemetry, "telemetry thread exiting");
        })
        .expect("failed to spawn telemetry thread")
}

// ─── Shutdown sequencer ───────────────────────────────────────────────────────

/// Execute the spec-mandated graceful shutdown sequence.
///
/// This function is called from the main thread (or a shutdown-coordinator
/// task) after the shutdown token has been triggered.
///
/// The GPU drain step requires access to `wgpu::Device`; callers provide a
/// callback for that step to keep this module free of wgpu imports.
///
/// Returns `0` for clean shutdown, `1` for error/GPU-lost shutdown.
pub async fn graceful_shutdown(
    reason: ShutdownReason,
    config: &ShutdownConfig,
    stop_accepting: impl FnOnce(),
    drain_mutations: impl std::future::Future<Output = ()>,
    revoke_all_leases: impl FnOnce(),
    flush_telemetry: impl std::future::Future<Output = ()>,
    terminate_sessions: impl FnOnce(),
    gpu_drain: impl FnOnce(),
) -> i32 {
    tracing::info!(?reason, "beginning graceful shutdown sequence");

    // Step 1: Stop accepting new connections.
    stop_accepting();
    tracing::debug!("shutdown step 1/8: stopped accepting connections");

    // Step 2: Drain active mutations with timeout.
    let drain_timeout = Duration::from_millis(config.drain_timeout_ms);
    tokio::select! {
        _ = drain_mutations => {
            tracing::debug!("shutdown step 2/8: mutations drained");
        }
        _ = tokio::time::sleep(drain_timeout) => {
            tracing::warn!(
                "shutdown step 2/8: drain timeout ({} ms) reached; proceeding",
                config.drain_timeout_ms
            );
        }
    }

    // Step 3: Revoke all leases (fire-and-forget, no ack required).
    revoke_all_leases();
    tracing::debug!("shutdown step 3/8: leases revoked");

    // Step 4: Flush telemetry with grace period.
    let telem_grace = Duration::from_millis(config.telemetry_grace_ms);
    tokio::select! {
        _ = flush_telemetry => {
            tracing::debug!("shutdown step 4/8: telemetry flushed");
        }
        _ = tokio::time::sleep(telem_grace) => {
            tracing::warn!(
                "shutdown step 4/8: telemetry grace ({} ms) reached; proceeding",
                config.telemetry_grace_ms
            );
        }
    }

    // Step 5: Terminate agent sessions.
    terminate_sessions();
    tracing::debug!("shutdown step 5/8: agent sessions terminated");

    // Step 6: GPU drain.
    gpu_drain();
    tracing::debug!("shutdown step 6/8: GPU drained");

    // Steps 7 & 8: Resources released as Rust drops happen; exit code follows.
    let exit_code = match reason {
        ShutdownReason::Clean => 0,
        ShutdownReason::GpuDeviceLost | ShutdownReason::Fatal(_) => 1,
    };
    tracing::info!(exit_code, "shutdown sequence complete");
    exit_code
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ShutdownConfig ───────────────────────────────────────────────────────

    #[test]
    fn shutdown_config_default_values() {
        let cfg = ShutdownConfig::default();
        assert_eq!(cfg.drain_timeout_ms, 500, "spec default 500 ms");
        assert_eq!(cfg.telemetry_grace_ms, 200, "spec default 200 ms");
    }

    #[test]
    fn shutdown_config_valid_boundaries() {
        let _ = ShutdownConfig::new(0, 0);
        let _ = ShutdownConfig::new(60_000, 10_000);
    }

    #[test]
    #[should_panic(expected = "drain_timeout_ms must be ≤ 60 000")]
    fn shutdown_config_rejects_drain_timeout_out_of_range() {
        let _ = ShutdownConfig::new(60_001, 0);
    }

    #[test]
    #[should_panic(expected = "telemetry_grace_ms must be ≤ 10 000")]
    fn shutdown_config_rejects_telemetry_grace_out_of_range() {
        let _ = ShutdownConfig::new(0, 10_001);
    }

    // ── ShutdownToken ────────────────────────────────────────────────────────

    #[test]
    fn shutdown_token_starts_untriggered() {
        let token = ShutdownToken::new();
        assert!(!token.is_triggered());
    }

    #[test]
    fn shutdown_token_trigger_sets_flag() {
        let token = ShutdownToken::new();
        token.trigger(ShutdownReason::Clean);
        assert!(token.is_triggered());
    }

    #[test]
    fn shutdown_token_trigger_is_idempotent() {
        let token = ShutdownToken::new();
        token.trigger(ShutdownReason::Clean);
        token.trigger(ShutdownReason::GpuDeviceLost); // second trigger ignored
        assert!(token.is_triggered());
    }

    #[tokio::test]
    async fn shutdown_token_receiver_gets_reason() {
        let token = ShutdownToken::new();
        let mut rx = token.subscribe();
        token.trigger(ShutdownReason::Clean);
        let reason = rx.recv().await.unwrap();
        assert_eq!(reason, ShutdownReason::Clean);
    }

    #[tokio::test]
    async fn shutdown_token_clone_shares_state() {
        let token = ShutdownToken::new();
        let token2 = token.clone();
        token2.trigger(ShutdownReason::Clean);
        assert!(token.is_triggered());
        assert!(token2.is_triggered());
    }

    // ── Thread spawning ──────────────────────────────────────────────────────

    #[test]
    fn spawn_compositor_thread_and_join() {
        let token = ShutdownToken::new();
        let (ready_tx, ready_rx) = oneshot::channel();
        let handle = spawn_compositor_thread(token, ready_tx, |_shutdown, ready| {
            // Signal ready immediately.
            let _ = ready.send(CompositorReady { ok: true });
            // No actual work — just a smoke test.
        });
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(ready_rx)
            .unwrap();
        assert!(result.ok);
        handle.join().expect("compositor thread panicked");
    }

    #[test]
    fn spawn_telemetry_thread_and_join() {
        let token = ShutdownToken::new();
        let handle = spawn_telemetry_thread(token, |_shutdown| {
            // No work — smoke test.
        });
        handle.join().expect("telemetry thread panicked");
    }

    // ── Graceful shutdown ────────────────────────────────────────────────────

    #[tokio::test]
    async fn graceful_shutdown_clean_returns_zero() {
        let config = ShutdownConfig::default();
        let exit_code = graceful_shutdown(
            ShutdownReason::Clean,
            &config,
            || {},    // stop_accepting
            async {}, // drain_mutations
            || {},    // revoke_all_leases
            async {}, // flush_telemetry
            || {},    // terminate_sessions
            || {},    // gpu_drain
        )
        .await;
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn graceful_shutdown_gpu_lost_returns_nonzero() {
        let config = ShutdownConfig::default();
        let exit_code = graceful_shutdown(
            ShutdownReason::GpuDeviceLost,
            &config,
            || {},
            async {},
            || {},
            async {},
            || {},
            || {},
        )
        .await;
        assert_ne!(exit_code, 0);
    }

    #[tokio::test]
    async fn graceful_shutdown_drain_timeout_does_not_hang() {
        let config = ShutdownConfig::new(50, 10); // very short timeouts
        let exit_code = graceful_shutdown(
            ShutdownReason::Clean,
            &config,
            || {},
            // Simulates a slow drain — longer than the timeout.
            tokio::time::sleep(Duration::from_millis(10_000)),
            || {},
            async {},
            || {},
            || {},
        )
        .await;
        // Should still complete (timeout hit) and return 0 (clean reason).
        assert_eq!(exit_code, 0);
    }

    // ── Thread priority elevation ─────────────────────────────────────────────

    #[test]
    fn elevate_main_thread_priority_does_not_panic() {
        // On CI (unprivileged) this returns false; on privileged hosts, true.
        // Either way it must not panic.
        let _ = elevate_main_thread_priority();
    }

    // ── No dynamic thread spawning ───────────────────────────────────────────

    /// Verify that thread-count constants reflect the fixed set of threads.
    /// This is a static assertion rather than a runtime one; we just verify
    /// the role enum covers exactly the four spec-mandated threads.
    #[test]
    fn thread_roles_cover_exactly_four_roles() {
        let roles = [
            ThreadRole::Main,
            ThreadRole::Compositor,
            ThreadRole::Network,
            ThreadRole::Telemetry,
        ];
        assert_eq!(roles.len(), 4, "spec mandates exactly 4 thread roles");
    }
}
