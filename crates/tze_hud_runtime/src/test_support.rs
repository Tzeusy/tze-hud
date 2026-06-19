//! Test-only shared helpers for runtime-lib tests.

use tokio::sync::{Mutex, MutexGuard};

/// Serializes tests that construct real headless wgpu compositor state.
///
/// On headless Linux with llvmpipe/wgpu, concurrent headless compositor/runtime
/// construction can wedge the default parallel libtest harness. Keep this guard
/// scoped to tests that create real GPU-backed headless runtime objects.
static HEADLESS_RUNTIME_MUTEX: Mutex<()> = Mutex::const_new(());

pub(crate) async fn lock_headless_runtime() -> MutexGuard<'static, ()> {
    HEADLESS_RUNTIME_MUTEX.lock().await
}
