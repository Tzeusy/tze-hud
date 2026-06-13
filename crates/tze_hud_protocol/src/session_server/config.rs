// ─── Session Configuration ───────────────────────────────────────────────────

/// Runtime-configurable parameters for session management (RFC 0005 §10).
///
/// All fields correspond to spec-defined configuration parameters with their
/// documented defaults.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Maximum time (ms) to wait for SessionInit after stream open. Default: 5000.
    pub handshake_timeout_ms: u64,
    /// Interval (ms) at which the client must send Heartbeat. Default: 5000.
    pub heartbeat_interval_ms: u64,
    /// Number of consecutive missed heartbeats before ungraceful disconnect. Default: 3.
    pub heartbeat_missed_threshold: u64,
    /// Grace period (ms) to hold orphaned leases after disconnect. Default: 30000.
    pub reconnect_grace_period_ms: u64,
    /// Timeout (ms) before retransmitting unacknowledged transactional messages. Default: 5000.
    pub retransmit_timeout_ms: u64,
    /// Per-session deduplication window size (unique batch_id values). Default: 1000.
    pub dedup_window_size: usize,
    /// Per-session deduplication window TTL (seconds). Default: 60.
    pub dedup_window_ttl_s: u64,
    /// Maximum sequence gap before SEQUENCE_GAP_EXCEEDED. Default: 100.
    pub max_sequence_gap: u64,
    /// Per-session ephemeral message buffer quota (oldest dropped beyond this). Default: 16.
    pub ephemeral_buffer_max: usize,
    /// Maximum concurrent resident sessions. Default: 16.
    pub max_concurrent_resident_sessions: usize,
    /// Maximum concurrent guest sessions. Default: 64.
    pub max_concurrent_guest_sessions: usize,
    /// Maximum future schedule horizon in microseconds (RFC 0003 §3.5). Default: 300_000_000 (5 min).
    pub max_future_schedule_us: u64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            handshake_timeout_ms: 5000,
            heartbeat_interval_ms: 5000,
            heartbeat_missed_threshold: 3,
            reconnect_grace_period_ms: 30_000,
            retransmit_timeout_ms: 5000,
            dedup_window_size: 1000,
            dedup_window_ttl_s: 60,
            max_sequence_gap: 100,
            ephemeral_buffer_max: 16,
            max_concurrent_resident_sessions: 16,
            max_concurrent_guest_sessions: 64,
            max_future_schedule_us: 300_000_000, // 5 minutes in microseconds
        }
    }
}
