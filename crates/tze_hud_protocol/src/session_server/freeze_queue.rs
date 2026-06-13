//! Per-session freeze queue for mutation buffering during scene freeze.
//!
//! Holds `SessionFreezeQueue`, `FrozenMutation`, and `FreezeEnqueueResult` —
//! the bounded mutation queue used when a session's scene is frozen.
//!
//! Extracted from `session_server/mod.rs` (SS-2 of the module split plan).
//! All logic is unchanged; only visibility modifiers were added.

use std::collections::VecDeque;

use crate::proto::session::MutationBatch;

use super::traffic::{TrafficClass, classify_inbound_batch};

// ─── Per-session freeze queue ─────────────────────────────────────────────────

/// Default per-session mutation queue capacity while frozen.
/// Source: system-shell/spec.md §Freeze Scene (default 1000).
pub(super) const FREEZE_QUEUE_CAPACITY: usize = 1_000;

/// Queue pressure threshold fraction (80% of capacity).
/// Source: system-shell/spec.md §Freeze Backpressure Signal.
const FREEZE_QUEUE_PRESSURE_FRACTION: f32 = 0.80;

/// A queued mutation entry for the per-session freeze queue.
#[derive(Clone, Debug)]
struct FrozenMutation {
    /// The original proto `MutationBatch` to re-apply on unfreeze.
    batch: MutationBatch,
    /// Traffic class inferred at enqueue time.
    traffic_class: TrafficClass,
    /// Coalesce key for StateStream mutations: `"<namespace>/<lease_id_hex>"`.
    /// When two entries share the same key, the newer one replaces the older
    /// (latest-wins coalescing per spec).
    coalesce_key: Option<String>,
}

/// Outcome of a freeze-queue enqueue operation.
#[derive(Debug)]
pub(super) enum FreezeEnqueueResult {
    /// Mutation queued (possibly with pressure warning).
    Queued { pressure_warning: bool },
    /// StateStream coalesced with existing entry.
    Coalesced,
    /// A non-transactional entry was evicted; caller sends MUTATION_DROPPED.
    Evicted { evicted_batch_id: Vec<u8> },
    /// Transactional mutation overflows queue; caller applies gRPC backpressure.
    BackpressureRequired,
    /// Ephemeral mutation dropped (queue full of transactional entries).
    Dropped,
}

/// Per-session bounded mutation queue used during freeze.
pub(super) struct SessionFreezeQueue {
    capacity: usize,
    queue: VecDeque<FrozenMutation>,
}

impl SessionFreezeQueue {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::with_capacity(capacity.min(256)),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub(super) fn is_full(&self) -> bool {
        self.queue.len() >= self.capacity
    }

    fn pressure_warning_threshold(&self) -> usize {
        (self.capacity as f32 * FREEZE_QUEUE_PRESSURE_FRACTION) as usize
    }

    fn crosses_pressure_threshold_after_add(&self, before_len: usize) -> bool {
        let threshold = self.pressure_warning_threshold();
        before_len < threshold && self.queue.len() >= threshold
    }

    /// Enqueue a mutation batch per traffic-class-aware overflow rules.
    pub(super) fn enqueue(&mut self, batch: MutationBatch, namespace: &str) -> FreezeEnqueueResult {
        let traffic_class = classify_inbound_batch(&batch);
        // Derive coalesce key for StateStream: "namespace/lease_id_hex".
        // Using the first 8 bytes (64 bits) as a compact key.
        let coalesce_key = if traffic_class == TrafficClass::StateStream {
            let prefix_len = batch.lease_id.len().min(8);
            let key_hex: String = batch.lease_id[..prefix_len]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            Some(format!("{namespace}/{key_hex}"))
        } else {
            None
        };

        let before_len = self.queue.len();

        match traffic_class {
            TrafficClass::Transactional => {
                if self.is_full() {
                    return FreezeEnqueueResult::BackpressureRequired;
                }
                self.queue.push_back(FrozenMutation {
                    batch,
                    traffic_class,
                    coalesce_key,
                });
                let warn = self.crosses_pressure_threshold_after_add(before_len);
                FreezeEnqueueResult::Queued {
                    pressure_warning: warn,
                }
            }

            TrafficClass::StateStream => {
                // Try coalescing: if an entry with the same key exists, replace it.
                if let Some(ref key) = coalesce_key {
                    for entry in self.queue.iter_mut() {
                        if entry.traffic_class == TrafficClass::StateStream
                            && entry.coalesce_key.as_deref() == Some(key.as_str())
                        {
                            *entry = FrozenMutation {
                                batch,
                                traffic_class,
                                coalesce_key,
                            };
                            return FreezeEnqueueResult::Coalesced;
                        }
                    }
                }

                if self.is_full() {
                    // Evict oldest non-transactional entry.
                    if let Some(idx) = self
                        .queue
                        .iter()
                        .position(|e| e.traffic_class != TrafficClass::Transactional)
                    {
                        // idx was just returned by position() on this VecDeque,
                        // so remove(idx) is guaranteed to succeed.
                        let evicted = self
                            .queue
                            .remove(idx)
                            .expect("idx is a valid VecDeque index from position()");
                        self.queue.push_back(FrozenMutation {
                            batch,
                            traffic_class,
                            coalesce_key,
                        });
                        return FreezeEnqueueResult::Evicted {
                            evicted_batch_id: evicted.batch.batch_id,
                        };
                    } else {
                        // All slots transactional → backpressure.
                        return FreezeEnqueueResult::BackpressureRequired;
                    }
                }

                self.queue.push_back(FrozenMutation {
                    batch,
                    traffic_class,
                    coalesce_key,
                });
                let warn = self.crosses_pressure_threshold_after_add(before_len);
                FreezeEnqueueResult::Queued {
                    pressure_warning: warn,
                }
            }

            TrafficClass::Ephemeral => {
                if self.is_full() {
                    // Evict oldest non-transactional, or drop this one.
                    if let Some(idx) = self
                        .queue
                        .iter()
                        .position(|e| e.traffic_class != TrafficClass::Transactional)
                    {
                        // idx was just returned by position() on this VecDeque,
                        // so remove(idx) is guaranteed to succeed.
                        let evicted = self
                            .queue
                            .remove(idx)
                            .expect("idx is a valid VecDeque index from position()");
                        self.queue.push_back(FrozenMutation {
                            batch,
                            traffic_class,
                            coalesce_key,
                        });
                        return FreezeEnqueueResult::Evicted {
                            evicted_batch_id: evicted.batch.batch_id,
                        };
                    } else {
                        return FreezeEnqueueResult::Dropped;
                    }
                }

                self.queue.push_back(FrozenMutation {
                    batch,
                    traffic_class,
                    coalesce_key,
                });
                let warn = self.crosses_pressure_threshold_after_add(before_len);
                FreezeEnqueueResult::Queued {
                    pressure_warning: warn,
                }
            }
        }
    }

    /// Drain the queue in submission order.
    pub(super) fn drain(&mut self) -> Vec<MutationBatch> {
        self.queue.drain(..).map(|e| e.batch).collect()
    }
}
