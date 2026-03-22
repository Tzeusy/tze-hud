//! Telemetry collector — gathers per-frame records and produces session summaries.

use crate::record::{FrameTelemetry, SessionSummary};
use std::sync::mpsc;

/// Non-blocking telemetry collector.
/// The compositor sends records via a channel; the collector aggregates them.
pub struct TelemetryCollector {
    records: Vec<FrameTelemetry>,
    summary: SessionSummary,
    /// Optional channel receiver for async collection.
    receiver: Option<mpsc::Receiver<FrameTelemetry>>,
}

/// Handle for sending telemetry from the frame pipeline.
pub struct TelemetrySender {
    sender: mpsc::SyncSender<FrameTelemetry>,
}

impl TelemetrySender {
    /// Non-blocking send. Drops the record if the channel is full.
    pub fn send(&self, record: FrameTelemetry) {
        let _ = self.sender.try_send(record);
    }
}

/// Create a telemetry channel pair.
pub fn telemetry_channel(capacity: usize) -> (TelemetrySender, TelemetryCollector) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    (
        TelemetrySender { sender },
        TelemetryCollector {
            records: Vec::new(),
            summary: SessionSummary::new(),
            receiver: Some(receiver),
        },
    )
}

impl TelemetryCollector {
    /// Create a collector without a channel (for direct use).
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            summary: SessionSummary::new(),
            receiver: None,
        }
    }

    /// Record a frame telemetry entry directly.
    pub fn record(&mut self, frame: FrameTelemetry) {
        self.summary.total_frames += 1;
        self.summary.frame_time.record(frame.frame_time_us);
        self.records.push(frame);
    }

    /// Drain any pending records from the channel.
    pub fn drain_channel(&mut self) {
        if let Some(rx) = &self.receiver {
            let mut pending = Vec::new();
            while let Ok(record) = rx.try_recv() {
                pending.push(record);
            }
            for record in pending {
                self.record(record);
            }
        }
    }

    /// Get the current session summary.
    pub fn summary(&self) -> &SessionSummary {
        &self.summary
    }

    /// Get a mutable reference to the session summary (for recording latencies).
    pub fn summary_mut(&mut self) -> &mut SessionSummary {
        &mut self.summary
    }

    /// Get all recorded frames.
    pub fn records(&self) -> &[FrameTelemetry] {
        &self.records
    }

    /// Emit session summary as JSON.
    pub fn emit_json(&self) -> Result<String, serde_json::Error> {
        self.summary.to_json()
    }
}

impl Default for TelemetryCollector {
    fn default() -> Self {
        Self::new()
    }
}
