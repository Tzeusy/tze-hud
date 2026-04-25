//! # gpu_lock
//!
//! Windows GPU scheduling lock for tze_hud.exe.
//!
//! ## Background
//!
//! The tzehouse-windows box (`parrot-hen.ts.net`, RTX 3080) is dual-use:
//! - **Interactive `/user-test` sessions** — wgpu compositor running `tze_hud.exe`
//! - **Nightly real-decode CI** — GStreamer D3D11/NVDEC decoders
//!
//! Running both concurrently can cause DXGI device-lost errors, GPU OOM, or
//! corrupted decode test output. A file-based advisory lock provides mutual
//! exclusion. See `docs/design/tzehouse-windows-gpu-scheduling.md` for the full
//! policy.
//!
//! ## Lock file
//!
//! Path: `C:\ProgramData\tze_hud\gpu.lock`
//!
//! Format (line-delimited UTF-8 key=value):
//! ```text
//! SESSION_TYPE=interactive
//! PID=<process-id>
//! STARTED_AT=<RFC3339 UTC timestamp>
//! DESCRIPTION=tze_hud.exe interactive session
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tze_hud_runtime::gpu_lock::GpuLock;
//!
//! // Acquire near the top of main().
//! // Returns Ok(None) on non-Windows platforms (no-op).
//! // Returns Ok(Some(guard)) on Windows — hold the guard for the process lifetime.
//! // Returns Err if the lock is held by another live process.
//! let _gpu_lock_guard = GpuLock::acquire()?;
//! ```
//!
//! The returned [`GpuLockGuard`] releases the lock on Drop. A panic hook is
//! installed automatically so the lock is also released on unwind.
//!
//! ## Cross-platform
//!
//! On non-Windows targets, [`GpuLock::acquire`] is a no-op that returns
//! `Ok(None)`. All types compile on every platform; call sites need no cfg guards.
//!
//! ## Fail-safe
//!
//! If lock file I/O fails (permissions, missing directory, etc.), the function
//! logs a warning and continues startup — except when the lock is held by
//! another live process, which is a hard refusal.
//!
//! Design doc: `docs/design/tzehouse-windows-gpu-scheduling.md`

use std::fmt;

// ─── Error type ───────────────────────────────────────────────────────────────

/// Errors that cause `GpuLock::acquire` to refuse startup.
///
/// Transient I/O errors are logged as warnings but do not produce this error —
/// only a live conflicting lock does.
#[derive(Debug)]
pub struct GpuLockConflict {
    /// The SESSION_TYPE value in the existing lock file.
    pub session_type: String,
    /// The PID in the existing lock file.
    pub pid: u32,
    /// The STARTED_AT timestamp in the existing lock file.
    pub started_at: String,
    /// The DESCRIPTION value in the existing lock file.
    pub description: String,
}

impl fmt::Display for GpuLockConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[gpu-lock] GPU is already in use: SESSION_TYPE={} PID={} STARTED_AT={} — {}",
            self.session_type, self.pid, self.started_at, self.description
        )
    }
}

impl std::error::Error for GpuLockConflict {}

// ─── Public API (cross-platform) ─────────────────────────────────────────────

/// RAII guard that holds the GPU lock for the duration of `tze_hud.exe`.
///
/// Dropping this guard releases the lock atomically, verifying PID ownership
/// before deletion so a lock taken over by another process is never blown away.
///
/// On non-Windows platforms, the guard is a zero-size no-op struct.
#[derive(Debug)]
pub struct GpuLockGuard {
    #[cfg(target_os = "windows")]
    inner: windows_impl::GpuLockGuardInner,
    #[cfg(not(target_os = "windows"))]
    _private: (),
}

impl Drop for GpuLockGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        self.inner.release();
    }
}

/// The GPU lock manager.
///
/// Call [`GpuLock::acquire`] once at startup and hold the returned guard until
/// the process exits.
pub struct GpuLock;

impl GpuLock {
    /// Acquire the GPU lock for an interactive `tze_hud.exe` session.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(guard))` — lock acquired; hold the guard for the process lifetime.
    /// - `Ok(None)` — not Windows; no-op.
    /// - `Err(GpuLockConflict)` — lock held by another live process; refuse startup.
    ///
    /// Transient I/O errors (missing directory, permissions, etc.) are logged
    /// as warnings but do not produce `Err` — the runtime continues without a lock
    /// rather than refusing to start over a filesystem issue. The single exception
    /// is a live conflicting lock, which is a hard refusal.
    pub fn acquire() -> Result<Option<GpuLockGuard>, GpuLockConflict> {
        #[cfg(target_os = "windows")]
        {
            windows_impl::acquire().map(|inner| Some(GpuLockGuard { inner }))
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(None)
        }
    }
}

// ─── Windows implementation ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::GpuLockConflict;
    use std::path::{Path, PathBuf};

    const LOCK_DIR: &str = r"C:\ProgramData\tze_hud";
    const LOCK_FILE: &str = r"C:\ProgramData\tze_hud\gpu.lock";
    const LOCK_TMP: &str = r"C:\ProgramData\tze_hud\gpu.lock.tmp";

    /// Inner guard state: the lock file path and the PID that wrote it.
    ///
    /// The struct holds the PID (not re-read from disk on drop) so release
    /// is correct even if the file is corrupt at that point.
    #[derive(Debug)]
    pub(super) struct GpuLockGuardInner {
        lock_path: PathBuf,
        owner_pid: u32,
    }

    impl GpuLockGuardInner {
        /// Release the lock. Verifies PID ownership before deletion.
        pub(super) fn release(&self) {
            release_lock(&self.lock_path, self.owner_pid);
        }
    }

    // ── Lock content helpers ──────────────────────────────────────────────────

    /// Parse the key=value pairs in the lock file. Returns None on any error.
    fn parse_lock_file(path: &Path) -> Option<std::collections::HashMap<String, String>> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut map = std::collections::HashMap::new();
        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.trim().to_string(), value.to_string());
            }
        }
        if map.is_empty() { None } else { Some(map) }
    }

    /// Return an RFC 3339 UTC timestamp for "right now".
    ///
    /// Uses `GetSystemTime` from the `windows` crate (already a dependency of
    /// `tze_hud_runtime` on Windows). Falls back to a fixed placeholder string
    /// if the call fails — the placeholder is still parseable key=value.
    fn utc_now_rfc3339() -> String {
        use windows::Win32::System::SystemInformation::GetSystemTime;

        // SAFETY: GetSystemTime returns the current UTC SYSTEMTIME and has no
        // preconditions for callers.
        let st = unsafe { GetSystemTime() };

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond,
        )
    }

    /// Write the lock file content (SESSION_TYPE, PID, STARTED_AT, DESCRIPTION).
    ///
    /// Uses a write-to-tmp-then-rename pattern for best-effort atomicity on NTFS.
    fn write_lock(session_type: &str, pid: u32, started_at: &str, description: &str) -> bool {
        let content = format!(
            "SESSION_TYPE={session_type}\nPID={pid}\nSTARTED_AT={started_at}\nDESCRIPTION={description}\n"
        );
        // Write to tmp first, then rename.
        if let Err(e) = std::fs::write(LOCK_TMP, &content) {
            tracing::warn!(
                error = %e,
                "[gpu-lock] Failed to write lock tmp file {LOCK_TMP}; continuing without lock"
            );
            return false;
        }
        if let Err(e) = std::fs::rename(LOCK_TMP, LOCK_FILE) {
            tracing::warn!(
                error = %e,
                "[gpu-lock] Failed to rename {LOCK_TMP} → {LOCK_FILE}; continuing without lock"
            );
            let _ = std::fs::remove_file(LOCK_TMP);
            return false;
        }
        true
    }

    /// Check whether the process identified by `pid` is currently running.
    ///
    /// Uses `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION`. A successful
    /// open means the process exists (it may be a zombie, but for our purposes
    /// that is still "alive" — the CI job is still accounting for the PID).
    fn pid_is_alive(pid: u32) -> bool {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        // SAFETY: OpenProcess with a well-known access mask is safe. The resulting
        // handle (if any) is closed immediately via CloseHandle.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
        match handle {
            Ok(h) => {
                // SAFETY: `h` is a valid handle returned by OpenProcess.
                unsafe { CloseHandle(h).ok() };
                true
            }
            Err(_) => false,
        }
    }

    // ── Lock release helper (shared by Drop and panic hook) ───────────────────

    fn release_lock(lock_path: &Path, owner_pid: u32) {
        tracing::info!("[gpu-lock] Releasing {}", lock_path.display());

        // Re-read the lock file to verify PID ownership before deletion.
        match parse_lock_file(lock_path) {
            None => {
                // Lock file already absent or unreadable — nothing to do.
                tracing::info!("[gpu-lock] Lock file absent or unreadable; nothing to release.");
            }
            Some(fields) => {
                let file_pid: u32 = fields.get("PID").and_then(|v| v.parse().ok()).unwrap_or(0);
                if file_pid != owner_pid {
                    tracing::warn!(
                        our_pid = owner_pid,
                        lock_pid = file_pid,
                        "[gpu-lock] Lock PID mismatch — another process now holds the lock. Not deleting."
                    );
                    return;
                }
                if let Err(e) = std::fs::remove_file(lock_path) {
                    tracing::warn!(
                        error = %e,
                        "[gpu-lock] Failed to remove lock file; it will become stale."
                    );
                } else {
                    tracing::info!(pid = owner_pid, "[gpu-lock] Lock released.");
                }
            }
        }
    }

    // ── Main acquire logic ────────────────────────────────────────────────────

    /// Acquire the interactive GPU lock.
    pub(super) fn acquire() -> Result<GpuLockGuardInner, GpuLockConflict> {
        let pid = std::process::id();

        tracing::info!("[gpu-lock] Checking {} ...", LOCK_FILE);

        // Ensure lock directory exists (non-fatal on failure).
        let lock_dir = Path::new(LOCK_DIR);
        if !lock_dir.exists() {
            tracing::info!("[gpu-lock] Creating lock directory: {LOCK_DIR}");
            if let Err(e) = std::fs::create_dir_all(lock_dir) {
                tracing::warn!(
                    error = %e,
                    "[gpu-lock] Could not create lock directory; proceeding without lock"
                );
                // Return a guard that will silently no-op on drop (file won't exist).
                return Ok(GpuLockGuardInner {
                    lock_path: PathBuf::from(LOCK_FILE),
                    owner_pid: pid,
                });
            }
        }

        let lock_path = Path::new(LOCK_FILE);

        // Check for an existing lock.
        if lock_path.exists() {
            match parse_lock_file(lock_path) {
                None => {
                    // Unreadable or corrupt — treat as absent. Log and continue.
                    tracing::warn!(
                        "[gpu-lock] Lock file exists but is unreadable/corrupt; treating as absent."
                    );
                    let _ = std::fs::remove_file(lock_path);
                }
                Some(fields) => {
                    let existing_pid: u32 =
                        fields.get("PID").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let existing_type = fields
                        .get("SESSION_TYPE")
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string());
                    let existing_started = fields
                        .get("STARTED_AT")
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string());
                    let existing_desc = fields.get("DESCRIPTION").cloned().unwrap_or_default();

                    tracing::info!(
                        session_type = %existing_type,
                        pid = existing_pid,
                        started_at = %existing_started,
                        "[gpu-lock] Lock file found: SESSION_TYPE={} PID={} STARTED_AT={}",
                        existing_type, existing_pid, existing_started,
                    );
                    if !existing_desc.is_empty() {
                        tracing::info!("[gpu-lock] Description: {}", existing_desc);
                    }

                    if existing_pid > 0 && pid_is_alive(existing_pid) {
                        // A live process holds the lock. Refuse startup.
                        tracing::error!(
                            existing_pid,
                            existing_session_type = %existing_type,
                            "[gpu-lock] Process {} is alive. GPU is in use. Refusing startup.",
                            existing_pid
                        );
                        return Err(GpuLockConflict {
                            session_type: existing_type,
                            pid: existing_pid,
                            started_at: existing_started,
                            description: existing_desc,
                        });
                    } else {
                        // Stale lock (dead PID or PID=0) — remove and take over.
                        tracing::warn!(
                            stale_pid = existing_pid,
                            "[gpu-lock] Process {} is no longer running. Lock is stale. Removing.",
                            existing_pid
                        );
                        if let Err(e) = std::fs::remove_file(lock_path) {
                            tracing::warn!(
                                error = %e,
                                "[gpu-lock] Could not remove stale lock; proceeding without lock"
                            );
                            return Ok(GpuLockGuardInner {
                                lock_path: PathBuf::from(LOCK_FILE),
                                owner_pid: pid,
                            });
                        }
                    }
                }
            }
        }

        // Acquire: write the interactive lock.
        let started_at = utc_now_rfc3339();
        let description = "tze_hud.exe interactive session";

        if write_lock("interactive", pid, &started_at, description) {
            tracing::info!(
                session_type = "interactive",
                pid,
                started_at = %started_at,
                "[gpu-lock] Lock acquired (SESSION_TYPE=interactive PID={} STARTED_AT={}).",
                pid, started_at,
            );
        }
        // Whether or not write succeeded, return the guard — it will only delete
        // the file on drop if it was actually written.

        // Install a panic hook so unwind also releases the lock.
        install_panic_hook(pid);

        Ok(GpuLockGuardInner {
            lock_path: PathBuf::from(LOCK_FILE),
            owner_pid: pid,
        })
    }

    // ── Panic hook ────────────────────────────────────────────────────────────

    /// Install a panic hook that releases the GPU lock before unwinding.
    ///
    /// This supplements Drop: if the runtime panics on a thread where the guard
    /// has not yet been dropped (or where panic = abort), the hook fires and
    /// deletes the lock file, preventing it from becoming a permanent stale lock.
    ///
    /// Idempotent: called once at acquire time.
    fn install_panic_hook(owner_pid: u32) {
        let lock_path = PathBuf::from(LOCK_FILE);
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(
                "[gpu-lock] Panic detected — releasing GPU lock before unwind. {:?}",
                info
            );
            release_lock(&lock_path, owner_pid);
            prev_hook(info);
        }));
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// On non-Windows platforms, acquire must succeed and return None (no-op).
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn acquire_returns_none_on_non_windows() {
        let result = GpuLock::acquire();
        assert!(
            result.is_ok(),
            "acquire must not error on non-Windows: {result:?}"
        );
        assert!(
            result.unwrap().is_none(),
            "acquire must return None on non-Windows (no-op)"
        );
    }

    /// GpuLockConflict Display includes the session type and PID.
    #[test]
    fn gpu_lock_conflict_display_includes_pid_and_type() {
        let conflict = GpuLockConflict {
            session_type: "ci".to_string(),
            pid: 12345,
            started_at: "2026-04-25T18:00:00Z".to_string(),
            description: "nightly run".to_string(),
        };
        let s = conflict.to_string();
        assert!(s.contains("12345"), "Display should include PID: {s}");
        assert!(s.contains("ci"), "Display should include session type: {s}");
    }
}
