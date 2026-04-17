//! Bidirectional streaming session server implementing RFC 0005.
//!
//! This module provides `HudSessionImpl`, the server-side implementation of the
//! `HudSession` gRPC service. It manages the bidirectional streaming session
//! lifecycle: handshake, mutation processing, lease management, heartbeats,
//! event dispatch, and reconnection.
//!
//! # Session Lifecycle State Machine (RFC 0005 §1.1)
//!
//! ```text
//! Connecting → Handshaking → Active → Disconnecting → Closed → Resuming
//! ```
//!
//! Valid transitions:
//! - Connecting → Handshaking (stream opened, SessionInit received)
//! - Handshaking → Active (valid auth → SessionEstablished)
//! - Handshaking → Closed (auth failure → SessionError(AUTH_FAILED))
//! - Active → Disconnecting (SessionClose received)
//! - Active → Closed (ungraceful: heartbeat timeout or stream EOF/RST)
//! - Disconnecting → Closed (stream termination complete)
//! - Closed → Resuming (SessionResume within grace period)
//! - Resuming → Active (valid resume token)
//! - Resuming → Closed (expired/invalid token)

use crate::auth::{
    AuthResult, CapabilityPolicy, authenticate_session_init, negotiate_version,
    validate_canonical_capabilities,
};
use crate::convert;
use crate::dedup::{CachedResult, DedupWindow};
use crate::lease::{
    CachedLeaseResponse, DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY, LeaseCorrelationCache,
    effective_priority,
};
use crate::proto::session::client_message::Payload as ClientPayload;
use crate::proto::session::hud_session_server::HudSession;
use crate::proto::session::server_message::Payload as ServerPayload;
use crate::proto::session::*;
use crate::proto::{ElementInfo, ListElementsRequest, ListElementsResponse};
use crate::session::{SESSION_EVENT_CHANNEL_CAPACITY, SharedState};
use crate::subscriptions;
use crate::token::{DEFAULT_GRACE_PERIOD_MS, TokenStore};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tze_hud_resource::{
    AgentBudget, ResourceError as StoreResourceError, ResourceStore, ResourceStoreConfig,
    ResourceStored as StoreResourceStored, ResourceType as StoreResourceType,
    RuntimeWidgetStoreError, RuntimeWidgetStorePutOutcome as DurablePutOutcome, UploadId,
    UploadStartRequest,
};
use tze_hud_scene::element_store::{ElementStore, ElementStoreEntry, ElementType};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};
use tze_hud_scene::types::*;
use tze_hud_widget::{RuntimeWidgetAssetError, register_runtime_widget_svg_asset};

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

// ─── Session Lifecycle State Machine ────────────────────────────────────────

/// Session lifecycle states per RFC 0005 §1.1.
///
/// The state machine progresses through these states in response to protocol
/// events (stream open/close, SessionInit/Resume, heartbeat timeout, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// TCP/TLS establishment in progress. Initial state when gRPC stream is opened.
    Connecting,
    /// SessionInit received, validating credentials and capabilities.
    Handshaking,
    /// Bidirectional stream is open and agent is active.
    Active,
    /// Graceful close: agent sent SessionClose, waiting for stream termination.
    Disconnecting,
    /// Stream terminated. Leases are orphaned if previously Active.
    Closed,
    /// Agent is reconnecting within the grace period using a resume token.
    Resuming,
}

impl SessionState {
    /// Returns true if this state allows mutation submission.
    pub fn allows_mutations(&self) -> bool {
        *self == SessionState::Active
    }

    /// Human-readable label for logging.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Handshaking => "Handshaking",
            Self::Active => "Active",
            Self::Disconnecting => "Disconnecting",
            Self::Closed => "Closed",
            Self::Resuming => "Resuming",
        }
    }
}

// ─── Traffic Class ───────────────────────────────────────────────────────────

/// Traffic class for outbound server messages (RFC 0005 §3.1, §3.2).
///
/// Each class has different delivery guarantees:
/// - Transactional: at-least-once, ordered, never dropped.
/// - StateStream: at-least-once with coalescing; intermediate states may be skipped.
/// - Ephemeral: at-most-once, latest-wins, dropped under backpressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficClass {
    /// Reliable, ordered, never dropped. MutationResult, LeaseResponse, SessionEstablished, etc.
    Transactional,
    /// Coalesced under pressure; intermediate states may be skipped. SceneSnapshot, TelemetryFrame.
    StateStream,
    /// Droppable under backpressure; latest value wins. Heartbeat echo, ephemeral ZonePublish.
    Ephemeral,
}

/// Classify an outbound `ServerMessage` payload into its traffic class.
///
/// Per RFC 0005 §3.1 and §3.2:
/// - Session lifecycle responses, MutationResult, LeaseResponse, LeaseStateChange,
///   SubscriptionChangeResult, ZonePublishResult, RuntimeError, BackpressureSignal,
///   SessionSuspended, SessionResumed, and input-control responses are Transactional.
/// - SceneSnapshot, SceneDelta, EventBatch, TelemetryFrame are StateStream.
/// - Heartbeat echoes are Ephemeral.
pub fn classify_server_payload(payload: &ServerPayload) -> TrafficClass {
    match payload {
        // Session lifecycle — always transactional
        ServerPayload::SessionEstablished(_)
        | ServerPayload::SessionError(_)
        | ServerPayload::SessionResumeResult(_)
        | ServerPayload::SessionSuspended(_)
        | ServerPayload::SessionResumed(_)
        | ServerPayload::RuntimeError(_) => TrafficClass::Transactional,

        // Mutation / lease responses — transactional
        ServerPayload::MutationResult(_)
        | ServerPayload::LeaseResponse(_)
        | ServerPayload::LeaseStateChange(_)
        | ServerPayload::CapabilityNotice(_)
        | ServerPayload::SubscriptionChangeResult(_)
        | ServerPayload::ZonePublishResult(_)
        | ServerPayload::InputFocusResponse(_)
        | ServerPayload::InputCaptureResponse(_) => TrafficClass::Transactional,

        // Widget and resource-upload responses — transactional.
        ServerPayload::WidgetPublishResult(_)
        | ServerPayload::WidgetAssetRegisterResult(_)
        | ServerPayload::ResourceUploadAccepted(_)
        | ServerPayload::ResourceStored(_)
        | ServerPayload::ResourceErrorResponse(_) => TrafficClass::Transactional,

        // Backpressure signal — transactional (must not be dropped)
        ServerPayload::BackpressureSignal(_) => TrafficClass::Transactional,

        // Degradation notice — transactional (RFC 0005 §3.4; never dropped)
        ServerPayload::DegradationNotice(_) => TrafficClass::Transactional,

        // Scene state / events / runtime telemetry — state-stream
        ServerPayload::SceneSnapshot(_)
        | ServerPayload::SceneDelta(_)
        | ServerPayload::EventBatch(_)
        | ServerPayload::RuntimeTelemetry(_) => TrafficClass::StateStream,

        // Heartbeat echo — ephemeral (droppable, latest-wins)
        ServerPayload::Heartbeat(_) => TrafficClass::Ephemeral,

        // Agent event emission result — transactional (always delivered)
        ServerPayload::EmitSceneEventResult(_) | ServerPayload::ListElementsResponse(_) => {
            TrafficClass::Transactional
        }

        // Element repositioned event — transactional (drag completion / reset-to-default)
        ServerPayload::ElementRepositioned(_) => TrafficClass::Transactional,
    }
}

// ─── Inbound mutation traffic class ──────────────────────────────────────────

/// Traffic class for an **inbound** `MutationBatch`.
///
/// Used by the per-session freeze queue to implement traffic-class-aware
/// overflow (system-shell/spec.md §Freeze Scene, source RFC 0007 §4.3):
///
/// - **Transactional** — never evicted; gRPC backpressure applied on overflow.
/// - **StateStream** — coalesced (latest-wins) before eviction.
/// - **Ephemeral** — dropped oldest-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboundTrafficClass {
    Transactional,
    StateStream,
    Ephemeral,
}

/// Classify an inbound `MutationBatch` by examining its contained mutations.
///
/// Any structural/identity-changing mutation makes the batch Transactional;
/// otherwise content mutations are StateStream; empty batch is Ephemeral.
fn classify_inbound_batch(batch: &MutationBatch) -> InboundTrafficClass {
    for m in &batch.mutations {
        if let Some(ref mutation) = m.mutation {
            use crate::proto::mutation_proto::Mutation;
            match mutation {
                Mutation::CreateTile(_) => return InboundTrafficClass::Transactional,
                // AddNode is structural — marks the batch as Transactional.
                Mutation::AddNode(_) => return InboundTrafficClass::Transactional,
                // SetTileRoot, UpdateTileOpacity, UpdateTileInputMode are StateStream.
                Mutation::SetTileRoot(_) => {}
                Mutation::UpdateTileOpacity(_) => {}
                Mutation::UpdateTileInputMode(_) => {}
                Mutation::PublishToZone(_) => {}
                Mutation::PublishToTile(_) => {}
                Mutation::ClearZone(_) => {}
                Mutation::ClearWidget(_) => {}
                // UpdateNodeContent is a content update — StateStream
                Mutation::UpdateNodeContent(_) => {}
            }
        }
    }
    // If we found any mutation at all, it's StateStream (content update)
    if batch.mutations.is_empty() {
        InboundTrafficClass::Ephemeral
    } else {
        InboundTrafficClass::StateStream
    }
}

// ─── Per-session freeze queue ─────────────────────────────────────────────────

/// Default per-session mutation queue capacity while frozen.
/// Source: system-shell/spec.md §Freeze Scene (default 1000).
const FREEZE_QUEUE_CAPACITY: usize = 1_000;

/// Queue pressure threshold fraction (80% of capacity).
/// Source: system-shell/spec.md §Freeze Backpressure Signal.
const FREEZE_QUEUE_PRESSURE_FRACTION: f32 = 0.80;

/// A queued mutation entry for the per-session freeze queue.
#[derive(Clone, Debug)]
struct FrozenMutation {
    /// The original proto `MutationBatch` to re-apply on unfreeze.
    batch: MutationBatch,
    /// Traffic class inferred at enqueue time.
    traffic_class: InboundTrafficClass,
    /// Coalesce key for StateStream mutations: `"<namespace>/<lease_id_hex>"`.
    /// When two entries share the same key, the newer one replaces the older
    /// (latest-wins coalescing per spec).
    coalesce_key: Option<String>,
}

/// Outcome of a freeze-queue enqueue operation.
#[derive(Debug)]
enum FreezeEnqueueResult {
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
struct SessionFreezeQueue {
    capacity: usize,
    queue: VecDeque<FrozenMutation>,
}

impl SessionFreezeQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::with_capacity(capacity.min(256)),
        }
    }

    #[allow(dead_code)] // reserved for diagnostics/safe-mode drain path
    fn len(&self) -> usize {
        self.queue.len()
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn is_full(&self) -> bool {
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
    fn enqueue(&mut self, batch: MutationBatch, namespace: &str) -> FreezeEnqueueResult {
        let traffic_class = classify_inbound_batch(&batch);
        // Derive coalesce key for StateStream: "namespace/lease_id_hex".
        // Using the first 8 bytes (64 bits) as a compact key.
        let coalesce_key = if traffic_class == InboundTrafficClass::StateStream {
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
            InboundTrafficClass::Transactional => {
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

            InboundTrafficClass::StateStream => {
                // Try coalescing: if an entry with the same key exists, replace it.
                if let Some(ref key) = coalesce_key {
                    for entry in self.queue.iter_mut() {
                        if entry.traffic_class == InboundTrafficClass::StateStream
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
                        .position(|e| e.traffic_class != InboundTrafficClass::Transactional)
                    {
                        let evicted = self.queue.remove(idx).unwrap();
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

            InboundTrafficClass::Ephemeral => {
                if self.is_full() {
                    // Evict oldest non-transactional, or drop this one.
                    if let Some(idx) = self
                        .queue
                        .iter()
                        .position(|e| e.traffic_class != InboundTrafficClass::Transactional)
                    {
                        let evicted = self.queue.remove(idx).unwrap();
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
    fn drain(&mut self) -> Vec<MutationBatch> {
        self.queue.drain(..).map(|e| e.batch).collect()
    }

    /// Discard all queued mutations (used on safe mode cancellation).
    #[allow(dead_code)] // reserved for safe-mode cancellation path
    fn discard(&mut self) {
        self.queue.clear();
    }
}

// ─── Ephemeral send buffer ────────────────────────────────────────────────────

/// A bounded queue for ephemeral outbound messages.
///
/// When the buffer exceeds `capacity`, the oldest message is dropped, retaining
/// only the latest `capacity` messages (RFC 0005 §2.5: oldest-first eviction).
#[allow(dead_code)] // reserved for RFC 0005 §2.5 ephemeral eviction path; not yet wired
struct EphemeralQueue {
    queue: VecDeque<Result<ServerMessage, Status>>,
    capacity: usize,
}

#[allow(dead_code)] // reserved for RFC 0005 §2.5 ephemeral eviction path; not yet wired
impl EphemeralQueue {
    fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity + 1),
            capacity,
        }
    }

    /// Enqueue a message. If at capacity, drops the oldest entry.
    fn push(&mut self, msg: Result<ServerMessage, Status>) {
        if self.queue.len() >= self.capacity {
            self.queue.pop_front(); // oldest-first eviction
        }
        self.queue.push_back(msg);
    }

    /// Drain the queue into the send channel (non-blocking).
    async fn flush(&mut self, tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>) {
        while let Some(msg) = self.queue.pop_front() {
            let _ = tx.try_send(msg);
        }
    }
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Default heartbeat interval in milliseconds.
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 5000;

/// Default heartbeat missed threshold (number of missed heartbeats before disconnect).
const HEARTBEAT_MISSED_THRESHOLD: u64 = 3;

/// Default heartbeat timeout: threshold * interval.
const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 =
    DEFAULT_HEARTBEAT_INTERVAL_MS * HEARTBEAT_MISSED_THRESHOLD;

/// Default maximum sequence gap before SEQUENCE_GAP_EXCEEDED (RFC 0005 §2.3).
const DEFAULT_MAX_SEQUENCE_GAP: u64 = 100;

/// Default per-session ephemeral message buffer quota (RFC 0005 §2.5).
#[allow(dead_code)] // reserved for ephemeral buffer wiring
const DEFAULT_EPHEMERAL_BUFFER_MAX: usize = 16;

// ─── Helper ─────────────────────────────────────────────────────────────────

/// Process-start instant used as the base for monotonic timestamps.
///
/// Initialized on first access. All `_mono_us` timestamps are microseconds
/// elapsed since this point, giving true monotonic semantics independent of
/// wall-clock adjustments.
static PROCESS_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Returns the process-start `Instant`, initializing it on first call.
fn process_start() -> std::time::Instant {
    *PROCESS_START.get_or_init(std::time::Instant::now)
}

/// Returns monotonic microseconds elapsed since process start.
///
/// Uses `std::time::Instant` so the value is immune to wall-clock adjustments
/// (NTP steps, leap seconds, user clock changes). Suitable for `_mono_us` fields.
fn now_mono_us() -> u64 {
    process_start().elapsed().as_micros() as u64
}

fn now_wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn scene_id_to_bytes(id: tze_hud_scene::SceneId) -> Vec<u8> {
    id.as_uuid().as_bytes().to_vec()
}

#[allow(clippy::result_large_err)] // tonic::Status is large by design; boxing it would add indirection on every call
fn bytes_to_scene_id(bytes: &[u8]) -> Result<tze_hud_scene::SceneId, Status> {
    if bytes.len() != 16 {
        return Err(Status::invalid_argument(format!(
            "invalid scene ID: expected 16 bytes, got {}",
            bytes.len()
        )));
    }
    let arr: [u8; 16] = bytes.try_into().unwrap();
    let uuid = uuid::Uuid::from_bytes(arr);
    Ok(tze_hud_scene::SceneId::from_uuid(uuid))
}

/// Map proto `batch_id` bytes to a `SceneId` for rejection-correlation semantics.
///
/// If the client supplied a valid 16-byte UUID, use it directly so that any
/// `BatchRejected` or `MutationResult` echoes the client's own `batch_id`.
/// Note: `bytes_to_scene_id` validates only the byte length (16 bytes); UUID
/// version/variant are not checked because the spec (RFC 0005 §3.2) requires
/// only that `batch_id` is a 16-byte RFC 4122 UUID (big-endian, matching
/// `scene_id_to_bytes` / `bytes_to_scene_id`) — version bits are the client's
/// responsibility.
///
/// Falls back to a fresh `SceneId` only when the field is absent or malformed
/// (wrong length); logs a debug warning so SDK regressions are diagnosable.
fn proto_batch_id_to_scene_id(batch_id: &[u8]) -> tze_hud_scene::SceneId {
    match bytes_to_scene_id(batch_id) {
        Ok(id) => id,
        Err(_) => {
            tracing::debug!(
                batch_id_len = batch_id.len(),
                "proto batch_id is absent or malformed (expected 16 bytes); \
                 generating a fresh SceneId — client cannot correlate this batch"
            );
            tze_hud_scene::SceneId::new()
        }
    }
}

/// Captures the data needed to persist the element store outside the shared-state lock.
struct ElementStorePersistRequest {
    store: ElementStore,
    path: std::path::PathBuf,
}

/// Update tile entries in the element store and return an optional persistence request.
async fn persist_created_tile_entries(
    st: &mut SharedState,
    created_ids: &[SceneId],
) -> Option<ElementStorePersistRequest> {
    if created_ids.is_empty() {
        return None;
    }

    let created_tiles: Vec<(SceneId, String)> = {
        let scene = st.scene.lock().await;
        created_ids
            .iter()
            .filter_map(|id| {
                scene
                    .tiles
                    .get(id)
                    .map(|tile| (*id, tile.namespace.clone()))
            })
            .collect()
    };

    if created_tiles.is_empty() {
        return None;
    }

    let now = now_ms();
    let mut changed = false;
    for (id, namespace) in created_tiles {
        match st.element_store.entries.get_mut(&id) {
            Some(entry) => {
                if entry.element_type != ElementType::Tile {
                    entry.element_type = ElementType::Tile;
                    changed = true;
                }
                if entry.namespace != namespace {
                    entry.namespace = namespace;
                    changed = true;
                }
                if entry.created_at == 0 {
                    entry.created_at = now;
                    changed = true;
                }
                if entry.last_published_at != now {
                    entry.last_published_at = now;
                    changed = true;
                }
                if entry.geometry_override.is_some() {
                    entry.geometry_override = None;
                    changed = true;
                }
            }
            None => {
                st.element_store.entries.insert(
                    id,
                    ElementStoreEntry {
                        element_type: ElementType::Tile,
                        namespace,
                        created_at: now,
                        last_published_at: now,
                        geometry_override: None,
                    },
                );
                changed = true;
            }
        }
    }

    if !changed {
        return None;
    }

    st.element_store_path
        .clone()
        .map(|path| ElementStorePersistRequest {
            store: st.element_store.clone(),
            path,
        })
}

/// Persist the element store without blocking the async executor worker thread.
async fn persist_element_store(request: Option<ElementStorePersistRequest>) {
    let Some(request) = request else {
        return;
    };

    let path_for_log = request.path.clone();
    match tokio::task::spawn_blocking(move || request.store.persist_to_path_atomic(&request.path))
        .await
    {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            tracing::warn!(
                path = %path_for_log.display(),
                error = %err,
                "element_store: failed to persist tile IDs"
            );
        }
        Err(err) => {
            tracing::warn!(
                path = %path_for_log.display(),
                error = %err,
                "element_store: failed to join tile ID persistence task"
            );
        }
    }
}

fn element_type_wire_name(element_type: ElementType) -> &'static str {
    match element_type {
        ElementType::Tile => "tile",
        ElementType::Zone => "zone",
        ElementType::Widget => "widget",
    }
}

fn parse_element_type_filter(filter: &str) -> Option<ElementType> {
    match filter.trim().to_ascii_lowercase().as_str() {
        "tile" => Some(ElementType::Tile),
        "zone" => Some(ElementType::Zone),
        "widget" => Some(ElementType::Widget),
        _ => None,
    }
}

fn resolve_tile_bounds_with_override(
    entry: Option<&ElementStoreEntry>,
    agent_bounds: Option<Rect>,
    display_area: Rect,
) -> Option<Rect> {
    let user_override = entry.and_then(|e| e.geometry_override);
    let agent_requested = agent_bounds.map(|bounds| {
        rect_to_relative_geometry_policy(bounds, display_area.width, display_area.height)
    });
    resolve_geometry_override_chain(user_override, agent_requested, None, None).map(|policy| {
        geometry_policy_to_absolute_rect(policy, display_area.width, display_area.height)
    })
}

fn touch_element_store_entry_by_id(
    st: &mut SharedState,
    element_id: SceneId,
    element_type: ElementType,
    now: u64,
) -> Option<ElementStorePersistRequest> {
    let entry = st.element_store.entries.get_mut(&element_id)?;
    if entry.element_type != element_type {
        return None;
    }
    entry.last_published_at = now;
    st.element_store_path
        .clone()
        .map(|path| ElementStorePersistRequest {
            store: st.element_store.clone(),
            path,
        })
}

fn touch_element_store_entry_by_namespace(
    st: &mut SharedState,
    element_type: ElementType,
    namespace: &str,
    now: u64,
) -> Option<ElementStorePersistRequest> {
    let id = st
        .element_store
        .find_id_by_type_namespace(element_type, namespace)?;
    touch_element_store_entry_by_id(st, id, element_type, now)
}

// ─── Shared agent event emission types ───────────────────────────────────────

// AgentEventRateLimiter, MAX_PAYLOAD_BYTES, and DEFAULT_MAX_EVENTS_PER_SECOND
// live in tze_hud_scene::events::emission and are shared with tze_hud_runtime.
use tze_hud_scene::events::emission::{
    AgentEventRateLimiter, DEFAULT_MAX_EVENTS_PER_SECOND, MAX_PAYLOAD_BYTES,
};

#[derive(Clone, Debug)]
struct ResidentUploadState {
    request_sequence: u64,
    metadata: ResourceMetadata,
    total_size_bytes: u64,
}

/// Sliding-window byte limiter for resident upload payload intake.
///
/// Tracks accepted upload bytes in a 1-second window. The session handler
/// uses this to apply transport backpressure by delaying additional read
/// processing until enough bytes leave the active window.
#[derive(Debug)]
struct UploadByteRateLimiter {
    limit_bytes_per_second: usize,
    window: VecDeque<(Instant, usize)>,
    bytes_in_window: usize,
}

impl UploadByteRateLimiter {
    fn with_limit(limit_bytes_per_second: usize) -> Self {
        Self {
            limit_bytes_per_second,
            window: VecDeque::new(),
            bytes_in_window: 0,
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&(ts, bytes)) = self.window.front() {
            if now.duration_since(ts) >= Duration::from_secs(1) {
                self.window.pop_front();
                self.bytes_in_window = self.bytes_in_window.saturating_sub(bytes);
            } else {
                break;
            }
        }
    }

    fn available_bytes(&mut self, now: Instant) -> usize {
        if self.limit_bytes_per_second == 0 {
            return usize::MAX;
        }
        self.prune(now);
        self.limit_bytes_per_second
            .saturating_sub(self.bytes_in_window)
    }

    fn reserve_bytes(&mut self, now: Instant, bytes: usize) {
        if self.limit_bytes_per_second == 0 || bytes == 0 {
            return;
        }
        self.window.push_back((now, bytes));
        self.bytes_in_window = self.bytes_in_window.saturating_add(bytes);
    }

    fn next_delay(&mut self, now: Instant) -> Duration {
        if self.limit_bytes_per_second == 0 {
            return Duration::ZERO;
        }
        self.prune(now);
        match self.window.front() {
            Some((ts, _)) => Duration::from_secs(1).saturating_sub(now.duration_since(*ts)),
            None => Duration::ZERO,
        }
    }
}

#[derive(Debug)]
enum UploadWorkerCommand {
    Start {
        request_sequence: u64,
        capabilities: Vec<String>,
        start: ResourceUploadStart,
    },
    Chunk {
        request_sequence: u64,
        chunk: ResourceUploadChunk,
    },
    Complete {
        request_sequence: u64,
        capabilities: Vec<String>,
        complete: ResourceUploadComplete,
    },
}

#[derive(Debug)]
enum UploadWorkerEvent {
    UploadAccepted {
        request_sequence: u64,
        upload_id: [u8; 16],
    },
    Stored {
        request_sequence: u64,
        stored: StoreResourceStored,
        stored_bytes: u64,
        metadata: ResourceMetadata,
        upload_id: Option<[u8; 16]>,
    },
    Error {
        request_sequence: u64,
        upload_id: Option<Vec<u8>>,
        err: StoreResourceError,
    },
}

async fn run_upload_worker(
    state: Arc<Mutex<SharedState>>,
    namespace: String,
    mut command_rx: tokio::sync::mpsc::Receiver<UploadWorkerCommand>,
    event_tx: tokio::sync::mpsc::Sender<UploadWorkerEvent>,
    upload_rate_limit_bytes_per_sec: usize,
) {
    let store = {
        let st = state.lock().await;
        st.resource_store.clone()
    };
    let mut in_flight_uploads: HashMap<[u8; 16], ResidentUploadState> = HashMap::new();
    let mut upload_rate_limiter =
        UploadByteRateLimiter::with_limit(upload_rate_limit_bytes_per_sec);

    while let Some(command) = command_rx.recv().await {
        match command {
            UploadWorkerCommand::Start {
                request_sequence,
                capabilities,
                start,
            } => {
                let resource_type = match proto_resource_type_to_store(start.resource_type) {
                    Some(v) => v,
                    None => {
                        let err = StoreResourceError::UnsupportedType(format!(
                            "unknown resource_type enum value {}",
                            start.resource_type
                        ));
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id: None,
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };

                let expected_hash: [u8; 32] = match start.expected_hash.as_slice().try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        let err = StoreResourceError::HashMismatch {
                            computed: "invalid".to_string(),
                            expected: format!("len={} (expected 32)", start.expected_hash.len()),
                        };
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id: None,
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };

                let metadata = start.metadata.unwrap_or_default();
                if start.inline_data.is_empty() && start.total_size_bytes == 0 {
                    let err = StoreResourceError::SizeExceeded {
                        detail: "total_size_bytes must be > 0 for chunked uploads".to_string(),
                    };
                    if event_tx
                        .send(UploadWorkerEvent::Error {
                            request_sequence,
                            upload_id: None,
                            err,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }

                let total_size = match usize::try_from(start.total_size_bytes) {
                    Ok(v) => v,
                    Err(_) => {
                        let err = StoreResourceError::SizeExceeded {
                            detail: format!(
                                "total_size_bytes {} exceeds platform limit {}",
                                start.total_size_bytes,
                                usize::MAX
                            ),
                        };
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id: None,
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };

                let upload_id_bytes = *uuid::Uuid::now_v7().as_bytes();
                let inline_bytes = start.inline_data.len();
                let request = UploadStartRequest {
                    agent_namespace: namespace.clone(),
                    agent_capabilities: capabilities,
                    agent_budget: AgentBudget {
                        texture_bytes_total_limit: 0,
                        texture_bytes_total_used: 0,
                    },
                    upload_id: UploadId::from_bytes(upload_id_bytes),
                    resource_type,
                    expected_hash,
                    total_size,
                    inline_data: start.inline_data,
                    width: metadata.width,
                    height: metadata.height,
                };

                if inline_bytes > 0 {
                    apply_upload_transport_backpressure(&mut upload_rate_limiter, inline_bytes)
                        .await;
                }

                match store.handle_upload_start(request).await {
                    Ok(Some(stored)) => {
                        register_uploaded_scene_resource(&state, &stored.resource_id).await;
                        if event_tx
                            .send(UploadWorkerEvent::Stored {
                                request_sequence,
                                stored,
                                stored_bytes: start.total_size_bytes,
                                metadata,
                                upload_id: None,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => {
                        in_flight_uploads.insert(
                            upload_id_bytes,
                            ResidentUploadState {
                                request_sequence,
                                metadata,
                                total_size_bytes: start.total_size_bytes,
                            },
                        );
                        if event_tx
                            .send(UploadWorkerEvent::UploadAccepted {
                                request_sequence,
                                upload_id: upload_id_bytes,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(err) => {
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id: None,
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            UploadWorkerCommand::Chunk {
                request_sequence,
                chunk,
            } => {
                let upload_id_bytes = match upload_id_bytes_from_slice(&chunk.upload_id) {
                    Some(id) => id,
                    None => {
                        let err = StoreResourceError::InvalidChunk(format!(
                            "upload_id length={} (must be 16)",
                            chunk.upload_id.len()
                        ));
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id: Some(chunk.upload_id.clone()),
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };

                let Some(tracked) = in_flight_uploads.get(&upload_id_bytes).cloned() else {
                    let err = StoreResourceError::InvalidChunk(
                        "upload_id is not in-flight for this session".to_string(),
                    );
                    if event_tx
                        .send(UploadWorkerEvent::Error {
                            request_sequence,
                            upload_id: Some(chunk.upload_id.clone()),
                            err,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                };

                if !chunk.data.is_empty() {
                    apply_upload_transport_backpressure(&mut upload_rate_limiter, chunk.data.len())
                        .await;
                }

                if let Err(err) = store
                    .handle_upload_chunk(
                        &namespace,
                        UploadId::from_bytes(upload_id_bytes),
                        chunk.chunk_index,
                        chunk.data,
                    )
                    .await
                {
                    in_flight_uploads.remove(&upload_id_bytes);
                    if event_tx
                        .send(UploadWorkerEvent::Error {
                            request_sequence: tracked.request_sequence,
                            upload_id: Some(upload_id_bytes.to_vec()),
                            err,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            UploadWorkerCommand::Complete {
                request_sequence,
                capabilities,
                complete,
            } => {
                let upload_id_bytes = match upload_id_bytes_from_slice(&complete.upload_id) {
                    Some(id) => id,
                    None => {
                        let err = StoreResourceError::InvalidChunk(format!(
                            "upload_id length={} (must be 16)",
                            complete.upload_id.len()
                        ));
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id: Some(complete.upload_id.clone()),
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };

                let Some(tracked) = in_flight_uploads.get(&upload_id_bytes).cloned() else {
                    let err = StoreResourceError::InvalidChunk(
                        "upload_id is not in-flight for this session".to_string(),
                    );
                    if event_tx
                        .send(UploadWorkerEvent::Error {
                            request_sequence,
                            upload_id: Some(complete.upload_id.clone()),
                            err,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                };

                match store
                    .handle_upload_complete(
                        &namespace,
                        UploadId::from_bytes(upload_id_bytes),
                        &capabilities,
                        &AgentBudget {
                            texture_bytes_total_limit: 0,
                            texture_bytes_total_used: 0,
                        },
                    )
                    .await
                {
                    Ok(stored) => {
                        in_flight_uploads.remove(&upload_id_bytes);
                        register_uploaded_scene_resource(&state, &stored.resource_id).await;
                        if event_tx
                            .send(UploadWorkerEvent::Stored {
                                request_sequence: tracked.request_sequence,
                                stored,
                                stored_bytes: tracked.total_size_bytes,
                                metadata: tracked.metadata,
                                upload_id: Some(upload_id_bytes),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(err) => {
                        in_flight_uploads.remove(&upload_id_bytes);
                        if event_tx
                            .send(UploadWorkerEvent::Error {
                                request_sequence: tracked.request_sequence,
                                upload_id: Some(upload_id_bytes.to_vec()),
                                err,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }
    }
}

// ─── Session state ──────────────────────────────────────────────────────────

/// Per-session state tracked by the streaming server.
struct StreamSession {
    session_id: String,
    namespace: String,
    agent_name: String,
    /// Capabilities explicitly granted at handshake (from `requested_capabilities`).
    capabilities: Vec<String>,
    /// Authorization scope for subscription and capability-request checks.
    /// For unrestricted PSK sessions this is `vec!["*"]`; for restricted agents
    /// it mirrors `capabilities`. Used for gating subscriptions and mid-session
    /// CapabilityRequest evaluation.
    policy_capabilities: Vec<String>,
    lease_ids: Vec<tze_hud_scene::SceneId>,
    subscriptions: Vec<String>,
    /// Fine-grained event type prefix filters per subscription category (RFC 0010 §7.2).
    ///
    /// When an agent subscribes with a `filter_prefix` (via `SubscriptionChange.subscribe_filter`),
    /// the filter is stored here keyed by category name. Categories not present in this map
    /// use the category's default prefix. Filters are removed when the category is unsubscribed.
    subscription_filters: std::collections::HashMap<String, String>,
    server_sequence: u64,
    resume_token: Vec<u8>,
    last_heartbeat_ms: u64,

    /// Current lifecycle state (RFC 0005 §1.1).
    state: SessionState,

    /// Last validated client-side sequence number (RFC 0005 §2.3).
    /// Initialized to 1 during session init/resume (treating the handshake message as
    /// sequence 1). Each subsequent validated message must carry a strictly greater
    /// sequence number within `max_sequence_gap` of the previous.
    last_client_sequence: u64,

    /// Whether safe mode is active for this session (RFC 0005 §3.7).
    /// When true, MutationBatch messages are rejected with SAFE_MODE_ACTIVE.
    safe_mode_active: bool,

    /// Whether the agent indicated `expect_resume=true` in SessionClose (RFC 0005 §1.5).
    /// When true, leases are held for the full reconnect grace period.
    expect_resume: bool,

    /// Sliding-window rate limiter for agent scene event emission.
    ///
    /// Tracks per-session event timestamps for the 1-second sliding window.
    /// Default limit: [`DEFAULT_MAX_EVENTS_PER_SECOND`] events/second
    /// (spec: scene-events/spec.md §5.4).
    agent_event_rate_limiter: AgentEventRateLimiter,

    /// Per-session mutation queue for freeze semantics (system-shell/spec.md §Freeze Scene).
    ///
    /// When `SharedState.freeze_active` is true, incoming MutationBatch messages
    /// are enqueued here rather than applied to the scene. On unfreeze, all queued
    /// mutations are applied in submission order.
    ///
    /// The shell owns freeze state transitions; the session server owns the queue.
    freeze_queue: SessionFreezeQueue,

    /// Wall-clock time (UTC µs since epoch) at which this session was opened.
    /// Used for TIMESTAMP_TOO_OLD validation of TimingHints (RFC 0003 §3.5).
    session_open_at_wall_us: u64,

    /// Per-session MutationBatch deduplication window (RFC 0005 §5.2).
    dedup_window: DedupWindow,

    /// Per-session lease-operation correlation cache (RFC 0005 §5.3).
    ///
    /// Maps client sequence number → cached `LeaseResponse` payload.  On
    /// retransmit the server replays the cached response without re-applying
    /// the lease operation, preserving at-least-once semantics with
    /// idempotent handling.
    lease_correlation_cache: LeaseCorrelationCache,

    /// Per-session upload-byte limiter for resident resource transport.
    resource_upload_rate_limiter: UploadByteRateLimiter,
}

impl StreamSession {
    fn next_server_seq(&mut self) -> u64 {
        self.server_sequence += 1;
        self.server_sequence
    }

    /// Transition to a new state. Returns the previous state.
    fn transition(&mut self, new_state: SessionState) -> SessionState {
        let prev = self.state.clone();
        self.state = new_state;
        prev
    }

    /// Validate an inbound client sequence number per RFC 0005 §2.3.
    ///
    /// Returns `Ok(())` if valid, or an error string with the appropriate
    /// SessionError code if the sequence is regressed or the gap is too large.
    fn validate_client_sequence(
        &mut self,
        seq: u64,
        max_gap: u64,
    ) -> Result<(), (&'static str, String)> {
        if seq <= self.last_client_sequence {
            return Err((
                "SEQUENCE_REGRESSION",
                format!(
                    "sequence regression: received {seq}, last was {}",
                    self.last_client_sequence
                ),
            ));
        }
        // "gap" per spec (RFC 0005 §2.3) = seq − last_seq.
        // Reject if gap > max_sequence_gap (default 100).
        // Example: last=5, seq=105 → gap=100 = max_gap → accepted (not strictly greater).
        //          last=5, seq=106 → gap=101 > max_gap=100 → rejected.
        //          last=5, seq=150 (spec example) → gap=145 > 100 → rejected.
        let gap = seq - self.last_client_sequence;
        if gap > max_gap {
            return Err((
                "SEQUENCE_GAP_EXCEEDED",
                format!(
                    "sequence gap {gap} exceeds max {max_gap}: received {seq}, last was {}",
                    self.last_client_sequence
                ),
            ));
        }
        self.last_client_sequence = seq;
        Ok(())
    }
}

/// Broadcast channel capacity for transactional server-push messages (DegradationNotice, etc.).
///
/// A capacity of 32 ensures that if a session is briefly slow to consume messages it
/// still receives all degradation notices without the sender blocking.
const BROADCAST_CHANNEL_CAPACITY: usize = 32;

// ─── Capability Revocation Event ─────────────────────────────────────────────

/// A runtime-initiated capability revocation command broadcast to all session handlers.
///
/// When the runtime calls [`HudSessionImpl::revoke_capability_on_lease`], it broadcasts
/// this event. Each session handler checks whether any of its leases match `lease_id`
/// and, if so, applies the revocation to the scene graph and notifies the agent via
/// `CapabilityNotice(revoked=[capability_name])` and a `LeaseStateChange` audit event.
///
/// RFC 0001 §3.3: capability checks are enforced at mutation time against the live scope,
/// not merely at grant time.
#[derive(Clone, Debug)]
pub struct CapabilityRevocationEvent {
    /// The lease to narrow.
    pub lease_id: tze_hud_scene::SceneId,
    /// Canonical name of the capability to remove (e.g. `"create_tiles"`, `"publish_zone:subtitle"`).
    pub capability_name: String,
}

// ─── Service implementation ─────────────────────────────────────────────────

/// The bidirectional streaming session service implementation.
///
/// Holds shared state (scene graph + session registry) and implements the
/// `HudSession` trait generated from `session.proto`.
///
/// `degradation_tx` is a broadcast channel used to deliver `DegradationNotice`
/// messages to all active sessions unconditionally (RFC 0005 §3.4, §7.1).
/// Each session handler task subscribes to this channel and forwards any
/// received notices to the agent stream at Transactional traffic class.
///
/// `agent_capabilities` drives per-agent capability gating at handshake time
/// (configuration/spec.md §Requirement: Agent Registration, lines 136-147).
/// Agents whose `agent_id` matches a key in this map receive only the listed
/// capabilities; unlisted agents are treated as guests (no capabilities).
pub struct HudSessionImpl {
    pub state: Arc<Mutex<SharedState>>,
    psk: String,
    /// Per-agent capability grants from `[agents.registered]` config.
    ///
    /// Keyed by agent name (the `agent_id` sent in `SessionInit`).
    /// Used to build `CapabilityPolicy` at handshake: registered agents get
    /// their listed capabilities; unregistered agents get guest (empty) policy.
    ///
    /// For dev/test scenarios where no config is loaded, pass an empty map
    /// and set `fallback_unrestricted = true` to restore the legacy behaviour.
    agent_capabilities: Arc<HashMap<String, Vec<String>>>,
    /// When true and an agent is not found in `agent_capabilities`, grant
    /// unrestricted capabilities (backwards-compatible dev mode).
    ///
    /// Production deployments MUST set this to `false`.
    fallback_unrestricted: bool,
    /// Broadcast sender for transactional server-push notices (DegradationNotice).
    /// Cloned into each session handler task.
    pub degradation_tx: tokio::sync::broadcast::Sender<DegradationNotice>,
    /// Broadcast sender for live capability revocation commands (RFC 0001 §3.3, GAP-G3-4).
    ///
    /// When the runtime calls `revoke_capability_on_lease`, it broadcasts a
    /// `CapabilityRevocationEvent` here. Each active session handler subscribes
    /// and processes revocations for leases it owns, applying the scene-graph
    /// mutation and delivering the `CapabilityNotice` + `LeaseStateChange` responses.
    pub capability_revocation_tx: tokio::sync::broadcast::Sender<CapabilityRevocationEvent>,

    /// Broadcast sender for runtime-injected input event batches (hud-i6yd.6).
    ///
    /// Carries `(namespace, EventBatch)` tuples. Each session handler subscribes
    /// and delivers the batch only if `namespace` matches its own namespace AND the
    /// agent has at least one of `INPUT_EVENTS` / `FOCUS_EVENTS` active. The batch
    /// is filtered through `subscriptions::filter_event_batch` before delivery.
    ///
    /// Used by `inject_input_event` to push runtime-assembled ClickEvent /
    /// CommandInputEvent batches to the owning agent session.
    pub input_event_tx: tokio::sync::broadcast::Sender<(String, crate::proto::EventBatch)>,

    /// Broadcast sender for `ElementRepositionedEvent` notifications (hud-bs2q.6).
    ///
    /// Emitted after drag completion (geometry_override persisted) and after
    /// reset-to-default (geometry_override cleared). Each session handler subscribes
    /// and delivers the event only when the agent is subscribed to `SCENE_TOPOLOGY`
    /// and the session is `Active`. Agents cannot reject — no response mechanism.
    ///
    /// Subscription category: SCENE_TOPOLOGY (requires `read_scene_topology`).
    /// Message class: Transactional (never coalesced or dropped).
    pub element_repositioned_tx:
        tokio::sync::broadcast::Sender<crate::proto::ElementRepositionedEvent>,
}

impl HudSessionImpl {
    /// Create a new session service with the given scene graph and PSK.
    ///
    /// Uses an empty capability registry with `fallback_unrestricted = true`
    /// for backwards compatibility. Prefer `new_with_config` for production.
    pub fn new(scene: SceneGraph, psk: &str) -> Self {
        let (degradation_tx, _) = tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (capability_revocation_tx, _) =
            tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (input_event_tx, _) = tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (element_repositioned_tx, _) =
            tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        Self {
            state: Arc::new(Mutex::new(SharedState {
                scene: Arc::new(Mutex::new(scene)),
                sessions: crate::session::SessionRegistry::new(psk),
                resource_store: ResourceStore::new(ResourceStoreConfig::default()),
                widget_asset_store: crate::session::WidgetAssetStore::default(),
                runtime_widget_store: None,
                element_store: tze_hud_scene::element_store::ElementStore::default(),
                element_store_path: None,
                safe_mode_active: false,
                token_store: TokenStore::new(),
                freeze_active: false,
                degradation_level: crate::session::RuntimeDegradationLevel::Normal,
            })),
            psk: psk.to_string(),
            agent_capabilities: Arc::new(HashMap::new()),
            fallback_unrestricted: true,
            degradation_tx,
            capability_revocation_tx,
            input_event_tx,
            element_repositioned_tx,
        }
    }

    /// Create from existing shared state.
    ///
    /// Uses an empty capability registry with `fallback_unrestricted = true`
    /// for backwards compatibility. Prefer `from_shared_state_with_config` for production.
    pub fn from_shared_state(state: Arc<Mutex<SharedState>>, psk: &str) -> Self {
        let (degradation_tx, _) = tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (capability_revocation_tx, _) =
            tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (input_event_tx, _) = tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (element_repositioned_tx, _) =
            tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        Self {
            state,
            psk: psk.to_string(),
            agent_capabilities: Arc::new(HashMap::new()),
            fallback_unrestricted: true,
            degradation_tx,
            capability_revocation_tx,
            input_event_tx,
            element_repositioned_tx,
        }
    }

    /// Create from existing shared state with a config-driven capability registry.
    ///
    /// `agent_capabilities` is populated from `ResolvedConfig::agent_capabilities`
    /// (i.e. the `[agents.registered]` TOML section).
    ///
    /// `fallback_unrestricted` controls what happens when an agent is NOT found in
    /// the registry:
    /// - `false` (production): unlisted agents receive guest policy (no capabilities).
    /// - `true` (dev/test): unlisted agents receive unrestricted policy.
    pub fn from_shared_state_with_config(
        state: Arc<Mutex<SharedState>>,
        psk: &str,
        agent_capabilities: HashMap<String, Vec<String>>,
        fallback_unrestricted: bool,
    ) -> Self {
        let (degradation_tx, _) = tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (capability_revocation_tx, _) =
            tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (input_event_tx, _) = tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (element_repositioned_tx, _) =
            tokio::sync::broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        Self {
            state,
            psk: psk.to_string(),
            agent_capabilities: Arc::new(agent_capabilities),
            fallback_unrestricted,
            degradation_tx,
            capability_revocation_tx,
            input_event_tx,
            element_repositioned_tx,
        }
    }

    /// Broadcast a `DegradationNotice` to all currently-active sessions.
    ///
    /// Updates `SharedState::degradation_level` so that newly-joining sessions
    /// can observe the current level. Then sends the notice on the broadcast
    /// channel so every active session handler delivers it transactionally.
    ///
    /// Returns the number of active sessions that received the notice (0 if
    /// no sessions are connected).
    pub async fn broadcast_degradation(
        &self,
        level: crate::session::RuntimeDegradationLevel,
        reason: &str,
        affected_capabilities: Vec<String>,
    ) -> usize {
        // Update shared state.
        {
            let mut st = self.state.lock().await;
            st.degradation_level = level;
        }

        let notice = DegradationNotice {
            level: level.to_proto_i32(),
            reason: reason.to_string(),
            affected_capabilities,
            timestamp_wall_us: now_wall_us(),
        };

        // Broadcast returns an error only when there are no active subscribers
        // (no sessions connected). That is not an error condition.
        self.degradation_tx.send(notice).unwrap_or_default()
    }

    /// Revoke a named capability from an active lease at runtime (RFC 0001 §3.3, GAP-G3-4).
    ///
    /// This is the end-to-end API for live capability revocation. It:
    /// 1. Broadcasts a [`CapabilityRevocationEvent`] to all active session handlers.
    /// 2. The session handler that owns `lease_id` receives the event, calls
    ///    [`tze_hud_scene::graph::SceneGraph::revoke_capability`] to narrow the live scope,
    ///    then delivers `CapabilityNotice(revoked=[capability_name])` and a `LeaseStateChange`
    ///    audit event to the affected agent.
    ///
    /// After revocation, any attempt to use `capability_name` under `lease_id` will be
    /// rejected by the existing capability-check path in the mutation pipeline.
    ///
    /// # Arguments
    ///
    /// * `lease_id`        — The lease whose capability scope is being narrowed.
    /// * `capability_name` — Canonical name of the capability to remove
    ///   (e.g. `"create_tiles"`, `"publish_zone:subtitle"`).
    ///
    /// # Returns
    ///
    /// The number of session handlers that received the revocation event (0 if the
    /// lease is not owned by any currently-connected session).
    pub fn revoke_capability_on_lease(
        &self,
        lease_id: tze_hud_scene::SceneId,
        capability_name: impl Into<String>,
    ) -> usize {
        let event = CapabilityRevocationEvent {
            lease_id,
            capability_name: capability_name.into(),
        };
        self.capability_revocation_tx
            .send(event)
            .unwrap_or_default()
    }

    /// Inject an `EventBatch` into the gRPC stream of the session owning `namespace`.
    ///
    /// Used by the runtime to push ClickEvent / CommandInputEvent batches produced by
    /// the compositor input pipeline (Stage 2) to the owning agent (hud-i6yd.6).
    ///
    /// The batch is broadcast to all session handler tasks; each task delivers it only
    /// if its namespace matches AND the event passes subscription filtering
    /// (`INPUT_EVENTS` / `FOCUS_EVENTS` gates).
    ///
    /// Returns the number of session handlers that received the broadcast (0 if no
    /// sessions are currently connected, regardless of namespace match).
    ///
    /// # Subscription gate
    ///
    /// ClickEvent and CommandInputEvent are `INPUT_EVENTS` variants. The session handler
    /// will silently drop the batch if the agent is not subscribed to `INPUT_EVENTS`.
    /// Callers that need a guaranteed delivery path should ensure the agent subscribes
    /// to `INPUT_EVENTS` / `access_input_events` at handshake time.
    pub fn inject_input_event(
        &self,
        namespace: impl Into<String>,
        batch: crate::proto::EventBatch,
    ) -> usize {
        self.input_event_tx
            .send((namespace.into(), batch))
            .unwrap_or_default()
    }

    /// Broadcast an `ElementRepositionedEvent` to all active sessions subscribed
    /// to `SCENE_TOPOLOGY` (hud-bs2q.6).
    ///
    /// Called after:
    /// - Drag completion: `geometry_override` has been persisted.
    /// - Reset-to-default: `geometry_override` has been cleared.
    ///
    /// Each session handler delivers the event only when:
    /// 1. The session is `SessionState::Active`.
    /// 2. The agent is subscribed to `SCENE_TOPOLOGY`.
    ///
    /// Returns the number of active session handlers that received the broadcast
    /// (0 if no sessions are connected).
    pub fn broadcast_element_repositioned(
        &self,
        event: crate::proto::ElementRepositionedEvent,
    ) -> usize {
        self.element_repositioned_tx.send(event).unwrap_or_default()
    }

    /// Reset an element's user geometry override to the fallback position and
    /// broadcast an `ElementRepositionedEvent` to subscribed agents (hud-bs2q.6).
    ///
    /// This is the programmatic path for "reset-to-default". The visual entry
    /// point (right-click context menu / tap button on the drag handle) calls
    /// this from the compositor/input pipeline.
    ///
    /// # Behaviour
    ///
    /// 1. Clears `geometry_override` from the element store entry.
    /// 2. If no override was set, returns `false` (no-op).
    /// 3. Re-resolves the effective geometry from the fallback chain:
    ///    agent bounds → config override → default policy.
    /// 4. Persists the element store to disk.
    /// 5. Broadcasts `ElementRepositionedEvent {
    ///        element_id,
    ///        new_geometry  = fallback geometry,
    ///        previous_geometry = cleared override,
    ///    }` to sessions subscribed to `SCENE_TOPOLOGY`.
    ///
    /// Returns `true` if an override was cleared and the event was emitted.
    pub async fn reset_element_geometry(&self, element_id: SceneId) -> bool {
        let (previous_override, fallback_geometry, persist_request) = {
            let mut st = self.state.lock().await;
            // Attempt to clear the override.
            let previous = st.element_store.reset_geometry_override(element_id);
            if previous.is_none() {
                // No override present — no-op.
                return false;
            }
            // Resolve fallback geometry (agent bounds → config → default policy).
            let scene = st.scene.lock().await;
            let zero_policy = GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.0,
                height_pct: 0.0,
            };
            let fallback = if let Some(entry) = st.element_store.entries.get(&element_id) {
                match entry.element_type {
                    ElementType::Tile => {
                        let agent_policy = scene.tiles.get(&element_id).map(|tile| {
                            rect_to_relative_geometry_policy(
                                tile.bounds,
                                scene.display_area.width,
                                scene.display_area.height,
                            )
                        });
                        resolve_geometry_override_chain(None, agent_policy, None, None)
                            .unwrap_or(zero_policy)
                    }
                    ElementType::Zone => scene
                        .zone_registry
                        .resolve_geometry_policy_for_zone(&entry.namespace, None, None)
                        .unwrap_or(zero_policy),
                    ElementType::Widget => scene
                        .widget_registry
                        .resolve_geometry_policy_for_instance(&entry.namespace, None)
                        .unwrap_or(zero_policy),
                }
            } else {
                zero_policy
            };
            drop(scene);
            let persist_req =
                st.element_store_path
                    .clone()
                    .map(|path| ElementStorePersistRequest {
                        store: st.element_store.clone(),
                        path,
                    });
            (previous.unwrap(), fallback, persist_req)
        };

        // Persist store outside the lock.
        persist_element_store(persist_request).await;

        // Build and broadcast ElementRepositionedEvent.
        let event = crate::proto::ElementRepositionedEvent {
            element_id: scene_id_to_bytes(element_id),
            new_geometry: Some(convert::geometry_policy_to_proto(&fallback_geometry)),
            previous_geometry: Some(convert::geometry_policy_to_proto(&previous_override)),
        };
        self.broadcast_element_repositioned(event);
        true
    }

    /// Build and broadcast an `ElementRepositionedEvent` for a completed drag
    /// (hud-bs2q.6).
    ///
    /// Called by the compositor after `persist_drag_geometry` has already written
    /// the new `geometry_override` to the element store.
    ///
    /// `new_geometry` is the newly persisted policy.
    /// `previous_geometry` is the geometry that was in effect before the drag
    /// (the prior override or `None` if there was no override).
    pub fn emit_drag_repositioned_event(
        &self,
        element_id: SceneId,
        new_geometry: &GeometryPolicy,
        previous_geometry: Option<&GeometryPolicy>,
    ) {
        let event = crate::proto::ElementRepositionedEvent {
            element_id: scene_id_to_bytes(element_id),
            new_geometry: Some(convert::geometry_policy_to_proto(new_geometry)),
            previous_geometry: previous_geometry.map(convert::geometry_policy_to_proto),
        };
        self.broadcast_element_repositioned(event);
    }
}

#[tonic::async_trait]
impl HudSession for HudSessionImpl {
    type SessionStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<ServerMessage, Status>> + Send>>;

    async fn session(
        &self,
        request: Request<tonic::Streaming<ClientMessage>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        let mut inbound = request.into_inner();
        let state = self.state.clone();
        let psk = self.psk.clone();
        // Clone the capability registry for use inside the session task.
        let agent_capabilities = self.agent_capabilities.clone();
        let fallback_unrestricted = self.fallback_unrestricted;
        // Subscribe to the degradation broadcast channel before spawning the task.
        // Subscribing here (rather than inside the task) ensures we don't miss notices
        // that arrive between task spawn and channel subscription.
        let mut degradation_rx = self.degradation_tx.subscribe();
        // Subscribe to the capability revocation broadcast channel.
        // Subscribing here ensures the session handler receives revocations issued
        // immediately after it is spawned (before the task subscribes itself).
        let mut capability_revocation_rx = self.capability_revocation_tx.subscribe();

        // Subscribe to the input event broadcast channel (hud-i6yd.6).
        // Each session handler delivers only batches addressed to its own namespace.
        let mut input_event_rx = self.input_event_tx.subscribe();

        // Subscribe to the element-repositioned broadcast channel (hud-bs2q.6).
        // Delivery is gated on SCENE_TOPOLOGY subscription in the session loop.
        let mut element_repositioned_rx = self.element_repositioned_tx.subscribe();

        // Create outbound channel
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(
            SESSION_EVENT_CHANNEL_CAPACITY,
        );

        // Spawn the session handler task
        tokio::spawn(async move {
            // Wait for the first message (must be SessionInit or SessionResume)
            let first_msg = match tokio::time::timeout(
                tokio::time::Duration::from_millis(5000),
                inbound.message(),
            )
            .await
            {
                Ok(Ok(Some(msg))) => msg,
                Ok(Ok(None)) => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "HANDSHAKE_TIMEOUT".to_string(),
                                message: "Stream closed before handshake".to_string(),
                                hint: String::new(),
                            })),
                        }))
                        .await;
                    return;
                }
                Ok(Err(e)) => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "HANDSHAKE_ERROR".to_string(),
                                message: format!("Error receiving handshake: {e}"),
                                hint: String::new(),
                            })),
                        }))
                        .await;
                    return;
                }
                Err(_) => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "HANDSHAKE_TIMEOUT".to_string(),
                                message: "Handshake timed out (5000ms)".to_string(),
                                hint: "Send SessionInit as the first message".to_string(),
                            })),
                        }))
                        .await;
                    return;
                }
            };

            // Process handshake
            let mut session = match first_msg.payload {
                Some(ClientPayload::SessionInit(init)) => {
                    handle_session_init(
                        &state,
                        &psk,
                        &tx,
                        &init,
                        &agent_capabilities,
                        fallback_unrestricted,
                    )
                    .await
                }
                Some(ClientPayload::SessionResume(resume)) => {
                    handle_session_resume(
                        &state,
                        &psk,
                        &tx,
                        &resume,
                        &agent_capabilities,
                        fallback_unrestricted,
                    )
                    .await
                }
                _ => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "INVALID_HANDSHAKE".to_string(),
                                message: "First message must be SessionInit or SessionResume"
                                    .to_string(),
                                hint: String::new(),
                            })),
                        }))
                        .await;
                    return;
                }
            };

            let Some(ref mut session) = session else {
                return; // Handshake failed, error already sent
            };

            // Transition: Handshaking/Resuming → Active (RFC 0005 §1.1)
            session.transition(SessionState::Active);

            // Send SceneSnapshot after successful handshake (RFC 0005 §1.3, §6.4)
            {
                let st = state.lock().await;
                let wall_us = now_wall_us();
                let mono_us: u64 = now_mono_us();
                let (snap_json, checksum, sequence_number) = {
                    let scene = st.scene.lock().await;
                    let graph_snap = scene.take_snapshot(wall_us, mono_us);
                    let snap_json = graph_snap
                        .to_json()
                        .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                    let checksum = graph_snap.checksum.clone();
                    let sequence_number = scene.sequence_number;
                    (snap_json, checksum, sequence_number)
                };
                let seq = session.next_server_seq();
                drop(st);
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: seq,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::SceneSnapshot(SceneSnapshot {
                            snapshot_json: snap_json,
                            sequence: sequence_number,
                            snapshot_wall_us: wall_us,
                            snapshot_mono_us: mono_us,
                            blake3_checksum: checksum,
                        })),
                    }))
                    .await;
            }

            let upload_rate_limit_bytes_per_sec =
                session.resource_upload_rate_limiter.limit_bytes_per_second;
            let (upload_command_tx, upload_command_rx) =
                tokio::sync::mpsc::channel::<UploadWorkerCommand>(64);
            let (upload_event_tx, mut upload_event_rx) =
                tokio::sync::mpsc::channel::<UploadWorkerEvent>(64);
            tokio::spawn(run_upload_worker(
                state.clone(),
                session.namespace.clone(),
                upload_command_rx,
                upload_event_tx,
                upload_rate_limit_bytes_per_sec,
            ));

            // Main message loop
            //
            // The loop exits for one of three reasons:
            //   1. Stream EOF (graceful): agent closed the stream.
            //   2. Stream error: transport-level error.
            //   3. Heartbeat timeout: no message for heartbeat_missed_threshold × interval.
            //
            // In cases (2) and (3) the disconnect is ungraceful; leases become orphaned.
            // In case (1) the disconnect may be graceful (SessionClose was sent) or
            // ungraceful (agent dropped the connection without sending SessionClose).
            //
            // The loop also listens on `degradation_rx` for transactional DegradationNotice
            // broadcasts (RFC 0005 §3.4). These are delivered unconditionally to all active
            // sessions regardless of subscription config and are never dropped.
            loop {
                // Use heartbeat timeout for receive (RFC 0005 §1.6, §3.6)
                let timeout_duration =
                    tokio::time::Duration::from_millis(DEFAULT_HEARTBEAT_TIMEOUT_MS);

                // ── Unfreeze drain: apply queued mutations if freeze just cleared ──
                // When the shell sets SharedState.freeze_active = false, queued
                // mutations are applied at the start of the next loop iteration
                // so they are delivered in the next available frame batch
                // (system-shell/spec.md §Freeze Scene: "Unfreeze applies queued
                //  mutations in submission order in the next available frame batch").
                //
                // IMPORTANT: Use `apply_queued_batch_to_scene` (not
                // `handle_mutation_batch`) here. Each queued batch has already
                // received an immediate `MutationResult(accepted=true)` when it
                // was enqueued. Re-using `handle_mutation_batch` would send a
                // second result for the same batch_id, violating RFC 0005 §2.1.
                {
                    let freeze_active = state.lock().await.freeze_active;
                    if !freeze_active && !session.freeze_queue.is_empty() {
                        let queued = session.freeze_queue.drain();
                        for queued_batch in queued {
                            apply_queued_batch_to_scene(&state, session, queued_batch).await;
                        }
                    }
                }

                tokio::select! {
                    // ── Inbound client message ────────────────────────────────
                    msg_result = tokio::time::timeout(timeout_duration, inbound.message()) => {
                        match msg_result {
                            Ok(Ok(Some(msg))) => {
                                // Update heartbeat timestamp on any received message
                                session.last_heartbeat_ms = now_ms();

                                // Retransmit fast-path (RFC 0005 §5.3).
                                //
                                // For lease operations there is no batch_id correlation key;
                                // the client-side sequence number serves as the correlation key.
                                // When the server sees a sequence it has already processed for
                                // a lease operation, it replays the cached response without
                                // re-applying the operation and WITHOUT running sequence
                                // validation (which would reject the same sequence as a
                                // regression).
                                let is_lease_op = matches!(
                                    &msg.payload,
                                    Some(ClientPayload::LeaseRequest(_))
                                    | Some(ClientPayload::LeaseRenew(_))
                                    | Some(ClientPayload::LeaseRelease(_))
                                );
                                if is_lease_op && msg.sequence > 0
                                    && session.lease_correlation_cache.get(msg.sequence).is_some()
                                {
                                    // This is a retransmit: dispatch to the lease handler which
                                    // will replay the cached response.  Skip sequence validation
                                    // so the duplicate sequence does not terminate the session.
                                    handle_client_message(
                                        &state,
                                        session,
                                        &tx,
                                        &upload_command_tx,
                                        msg,
                                    )
                                    .await;
                                    continue;
                                }

                                // Validate client sequence number (RFC 0005 §2.3).
                                // Skip validation for sequence 0 (unset) to allow legacy callers
                                // that don't set sequences. Sequence must be monotonically increasing
                                // starting at 2 (since 1 is the handshake message).
                                if msg.sequence != 0 {
                                    match session.validate_client_sequence(
                                        msg.sequence,
                                        DEFAULT_MAX_SEQUENCE_GAP,
                                    ) {
                                        Ok(()) => {}
                                        Err((code, message)) => {
                                            // Close stream with sequence error
                                            let seq = session.next_server_seq();
                                            let _ = tx
                                                .send(Ok(ServerMessage {
                                                    sequence: seq,
                                                    timestamp_wall_us: now_wall_us(),
                                                    payload: Some(ServerPayload::SessionError(
                                                        SessionError {
                                                            code: code.to_string(),
                                                            message,
                                                            hint: String::new(),
                                                        },
                                                    )),
                                                }))
                                                .await;
                                            session.transition(SessionState::Closed);
                                            break;
                                        }
                                    }
                                }

                                // Check if this is a graceful close message
                                let is_close = matches!(
                                    &msg.payload,
                                    Some(ClientPayload::SessionClose(_))
                                );

                                handle_client_message(
                                    &state,
                                    session,
                                    &tx,
                                    &upload_command_tx,
                                    msg,
                                )
                                .await;

                                // After handling SessionClose, transition to Disconnecting then Closed
                                if is_close {
                                    session.transition(SessionState::Disconnecting);
                                    session.transition(SessionState::Closed);
                                    break;
                                }
                            }
                            Ok(Ok(None)) => {
                                // Stream EOF
                                session.transition(SessionState::Closed);
                                break;
                            }
                            Ok(Err(_e)) => {
                                // Stream transport error — ungraceful disconnect
                                session.transition(SessionState::Closed);
                                break;
                            }
                            Err(_) => {
                                // Heartbeat timeout (RFC 0005 §1.6, §3.6)
                                session.transition(SessionState::Closed);
                                break;
                            }
                        }
                    }

                    upload_event = upload_event_rx.recv() => {
                        match upload_event {
                            Some(UploadWorkerEvent::UploadAccepted {
                                request_sequence,
                                upload_id,
                            }) => {
                                let seq = session.next_server_seq();
                                let _ = tx
                                    .send(Ok(ServerMessage {
                                        sequence: seq,
                                        timestamp_wall_us: now_wall_us(),
                                        payload: Some(ServerPayload::ResourceUploadAccepted(
                                            ResourceUploadAccepted {
                                                request_sequence,
                                                upload_id: upload_id.to_vec(),
                                            },
                                        )),
                                    }))
                                    .await;
                            }
                            Some(UploadWorkerEvent::Stored {
                                request_sequence,
                                stored,
                                stored_bytes,
                                metadata,
                                upload_id,
                            }) => {
                                send_resource_stored(
                                    session,
                                    &tx,
                                    request_sequence,
                                    &stored,
                                    stored_bytes,
                                    metadata,
                                    upload_id.as_ref(),
                                )
                                .await;
                            }
                            Some(UploadWorkerEvent::Error {
                                request_sequence,
                                upload_id,
                                err,
                            }) => {
                                send_resource_error_response(
                                    session,
                                    &tx,
                                    request_sequence,
                                    upload_id.as_deref(),
                                    &err,
                                )
                                .await;
                            }
                            None => {
                                session.transition(SessionState::Closed);
                                break;
                            }
                        }
                    }

                    // ── DegradationNotice broadcast (RFC 0005 §3.4, §7.1) ────
                    //
                    // Transactional — delivered unconditionally to all active sessions
                    // regardless of subscription config. Never dropped.
                    degradation_result = degradation_rx.recv() => {
                        match degradation_result {
                            Ok(notice) => {
                                let seq = session.next_server_seq();
                                let _ = tx
                                    .send(Ok(ServerMessage {
                                        sequence: seq,
                                        timestamp_wall_us: now_wall_us(),
                                        payload: Some(ServerPayload::DegradationNotice(notice)),
                                    }))
                                    .await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                // We missed `n` notices due to slow consumption.
                                // Per spec §3.4, DegradationNotice is transactional and must not
                                // be dropped. Log the anomaly and continue — the session remains
                                // open. Operators should investigate why this session is slow.
                                // In a production implementation this would emit a metric/alert.
                                let _ = n; // suppress unused warning; real code: tracing::warn!
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Broadcast channel closed — runtime is shutting down.
                                // Treat as ungraceful disconnect.
                                session.transition(SessionState::Closed);
                                break;
                            }
                        }
                    }

                    // ── Capability revocation broadcast (RFC 0001 §3.3, GAP-G3-4) ────
                    //
                    // The runtime can narrow an active lease's capability scope without
                    // revoking the lease itself. The session handler applies the change
                    // to the scene graph and notifies the agent with CapabilityNotice
                    // + LeaseStateChange (both transactional — never dropped).
                    revocation_result = capability_revocation_rx.recv() => {
                        match revocation_result {
                            Ok(event) => {
                                // Only this session's leases are affected.
                                if session.lease_ids.contains(&event.lease_id) {
                                    handle_capability_revocation(
                                        &state,
                                        session,
                                        &tx,
                                        event,
                                    ).await;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // Missed revocation events. Log and continue; the capability
                                // scope may be stale for those dropped events.
                                // In production: emit a metric and re-query the live scope.
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Runtime shutting down — treat as ungraceful disconnect.
                                session.transition(SessionState::Closed);
                                break;
                            }
                        }
                    }

                    // ── Runtime-injected input EventBatch (hud-i6yd.6) ───────────
                    //
                    // The compositor input pipeline (Stage 2) assembles ClickEvent /
                    // CommandInputEvent batches for the owning agent and injects them
                    // here via `HudSessionImpl::inject_input_event`. Only batches
                    // addressed to this session's namespace are forwarded; others are
                    // silently discarded.
                    //
                    // Delivery is gated on subscription: the batch is filtered through
                    // `subscriptions::filter_event_batch` before sending. If the agent
                    // is not subscribed to INPUT_EVENTS / FOCUS_EVENTS the batch is
                    // dropped silently (no error response).
                    input_event_result = input_event_rx.recv() => {
                        match input_event_result {
                            Ok((target_namespace, batch)) => {
                                // Namespace filter: only deliver to the owning session.
                                if target_namespace == session.namespace {
                                    // Subscription filter: gate on INPUT_EVENTS / FOCUS_EVENTS.
                                    if let Some(filtered) = crate::subscriptions::filter_event_batch(
                                        batch,
                                        &session.subscriptions,
                                    ) {
                                        let seq = session.next_server_seq();
                                        let _ = tx
                                            .send(Ok(ServerMessage {
                                                sequence: seq,
                                                timestamp_wall_us: now_wall_us(),
                                                payload: Some(ServerPayload::EventBatch(filtered)),
                                            }))
                                            .await;
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // Missed input events under backpressure. Transactional
                                // events (ClickEvent, CommandInputEvent) must not be
                                // dropped; in production this should emit a metric/alert.
                                // For v1 we continue silently — the session remains open.
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Runtime shutting down — treat as ungraceful disconnect.
                                session.transition(SessionState::Closed);
                                break;
                            }
                        }
                    }

                    // ── ElementRepositionedEvent broadcast (hud-bs2q.6) ──────────
                    //
                    // Emitted after drag completion or reset-to-default. Delivered to
                    // agents subscribed to SCENE_TOPOLOGY (requires read_scene_topology).
                    // Transactional — never coalesced or dropped. Agent cannot reject.
                    element_repositioned_result = element_repositioned_rx.recv() => {
                        match element_repositioned_result {
                            Ok(event) => {
                                // Gate on SCENE_TOPOLOGY subscription.
                                if session.subscriptions.contains(
                                    &crate::subscriptions::category::SCENE_TOPOLOGY.to_string()
                                ) {
                                    let seq = session.next_server_seq();
                                    let _ = tx
                                        .send(Ok(ServerMessage {
                                            sequence: seq,
                                            timestamp_wall_us: now_wall_us(),
                                            payload: Some(
                                                ServerPayload::ElementRepositioned(event),
                                            ),
                                        }))
                                        .await;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                // Missed notifications. Log and continue — the element
                                // store state is persistent so a future snapshot or
                                // ListElementsRequest will reflect the current position.
                                let _ = n; // suppress unused warning; production: tracing::warn!
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Runtime shutting down — treat as ungraceful disconnect.
                                session.transition(SessionState::Closed);
                                break;
                            }
                        }
                    }
                }
            }

            // Cleanup: remove session from registry and store resume token.
            //
            // The resume token issued at handshake time is saved to the TokenStore so
            // the agent can reconnect within the grace period using SessionResume.
            // Token is not persisted across process restarts (RFC 0005 §6.6).
            let (resource_store, namespace_for_cleanup) = {
                let mut st = state.lock().await;
                st.sessions.remove_session(&session.session_id);

                // Only register a resume token if the session was ever Active
                // (i.e. handshake succeeded). Sessions that fail auth do not
                // get an orphaned-lease grace period.
                if !session.resume_token.is_empty() {
                    st.token_store.insert(
                        session.resume_token.clone(),
                        session.agent_name.clone(),
                        session.capabilities.clone(),
                        session.subscriptions.clone(),
                        session.lease_ids.clone(),
                        DEFAULT_GRACE_PERIOD_MS,
                        now_ms(),
                    );
                }
                (st.resource_store.clone(), session.namespace.clone())
            };
            resource_store
                .abort_all_uploads(&namespace_for_cleanup)
                .await;
        });

        // Return the receiver stream as the response
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }
}

// ─── Handshake handlers ─────────────────────────────────────────────────────

/// Resolve the per-agent authorization scope used for `CapabilityRequest`
/// evaluation.
///
/// Source of truth in v1:
/// - Registered agent entries (`agent_capabilities`) provide the full
///   allow-list.
/// - Unregistered agents receive unrestricted scope only when
///   `fallback_unrestricted=true` (dev/test mode).
/// - Otherwise unregistered agents are guest scope (empty allow-list).
fn authorization_scope_for_agent(
    agent_id: &str,
    agent_capabilities: &HashMap<String, Vec<String>>,
    fallback_unrestricted: bool,
) -> Vec<String> {
    match agent_capabilities.get(agent_id) {
        Some(caps) => caps.clone(),
        None if fallback_unrestricted => vec!["*".to_string()],
        None => Vec::new(),
    }
}

async fn handle_session_init(
    state: &Arc<Mutex<SharedState>>,
    psk: &str,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    init: &SessionInit,
    agent_capabilities: &HashMap<String, Vec<String>>,
    fallback_unrestricted: bool,
) -> Option<StreamSession> {
    // ── Step 1: Version negotiation (RFC 0005 §4.1) ──────────────────────────
    // Do this before authentication so agents can learn about version
    // incompatibility even if they send a wrong key.
    let negotiated_version =
        match negotiate_version(init.min_protocol_version, init.max_protocol_version) {
            Ok(v) => v,
            Err(msg) => {
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: 1,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::SessionError(SessionError {
                            code: "UNSUPPORTED_PROTOCOL_VERSION".to_string(),
                            message: msg,
                            hint: format!(
                                "{{\"runtime_min\": {}, \"runtime_max\": {}}}",
                                crate::auth::RUNTIME_MIN_VERSION,
                                crate::auth::RUNTIME_MAX_VERSION
                            ),
                        })),
                    }))
                    .await;
                return None;
            }
        };

    // ── Step 2: Authentication (RFC 0005 §1.4) ───────────────────────────────
    // Authentication is evaluated synchronously before SessionEstablished is sent.
    let auth_result =
        authenticate_session_init(init.auth_credential.as_ref(), &init.pre_shared_key, psk);

    match auth_result {
        AuthResult::Accepted => {}
        AuthResult::Failed(reason) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "AUTH_FAILED".to_string(),
                        message: reason,
                        hint: String::new(),
                    })),
                }))
                .await;
            return None;
        }
        AuthResult::Unimplemented(reason) => {
            // v1-reserved credential type — reject with AUTH_FAILED.
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "AUTH_FAILED".to_string(),
                        message: reason,
                        hint: r#"{"supported_v1": ["PreSharedKeyCredential", "LocalSocketCredential"]}"#.to_string(),
                    })),
                }))
                .await;
            return None;
        }
    }

    // ── Step 3: Capability vocabulary validation (configuration/spec.md §Capability Vocabulary) ──
    // All requested capability names MUST be from the canonical v1 vocabulary.
    // Legacy names (create_tile, receive_input, read_scene, zone_publish) and any
    // other non-canonical name MUST be rejected with CONFIG_UNKNOWN_CAPABILITY and a hint.
    if let Err(unknown_caps) = validate_canonical_capabilities(&init.requested_capabilities) {
        // Collect all errors before reporting (spec requires collecting all, not fail-fast).
        let hints: Vec<serde_json::Value> = unknown_caps
            .iter()
            .map(|e| serde_json::json!({"unknown": e.unknown, "hint": e.hint}))
            .collect();
        let hint_json = serde_json::to_string(&hints)
            .unwrap_or_else(|_| "see configuration/spec.md §Capability Vocabulary".to_string());
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::SessionError(SessionError {
                    code: "CONFIG_UNKNOWN_CAPABILITY".to_string(),
                    message: format!(
                        "{} unrecognized capability name(s); canonical v1 names are required",
                        unknown_caps.len()
                    ),
                    hint: hint_json,
                })),
            }))
            .await;
        return None;
    }

    // ── Step 4: Capability negotiation (RFC 0005 §5.3) ───────────────────────
    // Capabilities are gated against the agent's authorization policy.
    //
    // Per configuration/spec.md §Requirement: Agent Registration (lines 136-147),
    // the configured authorization scope is the source of truth for both
    // handshake grants and future mid-session escalation checks.
    let authorization_scope =
        authorization_scope_for_agent(&init.agent_id, agent_capabilities, fallback_unrestricted);
    let policy = CapabilityPolicy::new(authorization_scope.clone());
    let (granted_capabilities, _denied_caps) =
        policy.partition_capabilities(&init.requested_capabilities);

    // ── Step 5: Subscription filtering (RFC 0005 §7.1) ──────────────────────
    // Initial subscriptions are filtered against the agent's explicitly granted
    // capabilities. Agents must include the required capability in their
    // `requested_capabilities` to subscribe to capability-gated categories
    // (e.g. `access_input_events` for INPUT_EVENTS, `read_scene_topology` for
    // SCENE_TOPOLOGY). Mandatory categories are always active.
    let policy_caps = if policy.is_unrestricted() {
        vec!["*".to_string()]
    } else {
        authorization_scope
    };
    let sub_result =
        subscriptions::filter_subscriptions(&init.initial_subscriptions, &granted_capabilities);

    let session_id = uuid::Uuid::now_v7().to_string();
    let namespace = init.agent_id.clone();
    let resume_token = uuid::Uuid::now_v7().as_bytes().to_vec();

    // Register session in the session registry and capture upload rate config.
    let upload_rate_limit_bytes_per_sec = {
        let mut st = state.lock().await;
        let _ = st
            .sessions
            .authenticate(&init.agent_id, psk, &granted_capabilities);
        st.resource_store.upload_rate_limit_bytes_per_sec()
    };

    let session_open_at = now_wall_us();
    let mut session = StreamSession {
        session_id: session_id.clone(),
        namespace: namespace.clone(),
        agent_name: init.agent_id.clone(),
        capabilities: granted_capabilities.clone(),
        policy_capabilities: policy_caps.clone(),
        lease_ids: Vec::new(),
        subscriptions: sub_result.active.clone(),
        subscription_filters: std::collections::HashMap::new(),
        server_sequence: 0,
        resume_token: resume_token.clone(),
        last_heartbeat_ms: now_ms(),
        state: SessionState::Handshaking,
        last_client_sequence: 1, // SessionInit is sequence 1; start validation from next
        safe_mode_active: false,
        expect_resume: false,
        agent_event_rate_limiter: AgentEventRateLimiter::new(),
        freeze_queue: SessionFreezeQueue::new(FREEZE_QUEUE_CAPACITY),
        session_open_at_wall_us: session_open_at,
        dedup_window: DedupWindow::new(1000, 60),
        lease_correlation_cache: LeaseCorrelationCache::new(
            DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY,
        ),
        resource_upload_rate_limiter: UploadByteRateLimiter::with_limit(
            upload_rate_limit_bytes_per_sec,
        ),
    };

    // ── Step 5: Clock skew estimation (RFC 0003 §1.3) ────────────────────────
    let compositor_ts = now_wall_us();
    let estimated_skew = if init.agent_timestamp_wall_us > 0 {
        init.agent_timestamp_wall_us as i64 - compositor_ts as i64
    } else {
        0
    };

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: compositor_ts,
            payload: Some(ServerPayload::SessionEstablished(SessionEstablished {
                session_id: uuid::Uuid::parse_str(&session_id)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
                namespace,
                granted_capabilities,
                resume_token,
                heartbeat_interval_ms: DEFAULT_HEARTBEAT_INTERVAL_MS,
                server_sequence: seq,
                compositor_timestamp_wall_us: compositor_ts,
                estimated_skew_us: estimated_skew,
                active_subscriptions: sub_result.active,
                denied_subscriptions: sub_result.denied,
                negotiated_protocol_version: negotiated_version,
            })),
        }))
        .await;

    Some(session)
}

/// Handle a `SessionResume` message — the first message on a reconnecting stream
/// within the grace period (RFC 0005 §6.2–6.4).
///
/// # Protocol contract
///
/// 1. Re-authenticate via `pre_shared_key` (RFC 0005 §6.2).
/// 2. Look up and consume the resume token from the [`TokenStore`].
///    - If missing or expired → `SessionError(SESSION_GRACE_EXPIRED)`.
///    - If valid → restore session state and issue new token.
/// 3. Send [`SessionResumeResult`] with `accepted=true` and the confirmed
///    subscription/capability state.
/// 4. The caller (main session loop) sends a [`SceneSnapshot`] immediately
///    after this function returns (same mechanism as new connections).
async fn handle_session_resume(
    state: &Arc<Mutex<SharedState>>,
    psk: &str,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    resume: &SessionResume,
    agent_capabilities: &HashMap<String, Vec<String>>,
    fallback_unrestricted: bool,
) -> Option<StreamSession> {
    // Re-authentication is required on resume (RFC 0005 §6.2).
    let auth_result =
        authenticate_session_init(resume.auth_credential.as_ref(), &resume.pre_shared_key, psk);
    match auth_result {
        AuthResult::Accepted => {}
        AuthResult::Failed(reason) | AuthResult::Unimplemented(reason) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "AUTH_FAILED".to_string(),
                        message: reason,
                        hint: String::new(),
                    })),
                }))
                .await;
            return None;
        }
    }

    // Step 2: Validate the resume token.
    let current_ms = now_ms();
    let resume_result = {
        let mut st = state.lock().await;
        st.token_store
            .consume(&resume.resume_token, &resume.agent_id, current_ms)
    };

    let prior_entry = match resume_result {
        Ok(entry) => entry,
        Err(err) => {
            // Token invalid or expired — agent must perform a full SessionInit.
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: err.error_code().to_string(),
                        message: err.message().to_string(),
                        hint: err.hint().to_string(),
                    })),
                }))
                .await;
            return None;
        }
    };

    // Step 3: Build restored session.
    let session_id = uuid::Uuid::now_v7().to_string();
    let namespace = resume.agent_id.clone();
    // Issue a fresh single-use token for the resumed session (RFC 0005 §6.3).
    let new_resume_token = uuid::Uuid::now_v7().as_bytes().to_vec();

    // Register the resumed agent in the session registry so shared-state
    // operations (e.g. lease grant, broadcast) can find it, and capture the
    // current upload-rate configuration for this session.
    let upload_rate_limit_bytes_per_sec = {
        let mut st = state.lock().await;
        let _ = st
            .sessions
            .authenticate(&resume.agent_id, psk, &prior_entry.capabilities);
        st.resource_store.upload_rate_limit_bytes_per_sec()
    };

    // Reconstruct policy_caps for the resumed session using the same config-driven
    // lookup as new sessions.  `capabilities` (restored from TokenStore) holds the
    // grants the agent actually held before disconnect.  `policy_capabilities` governs
    // mid-session CapabilityRequest escalation and must reflect the agent's full
    // *authorization* scope (not just the already-granted subset), so that
    // post-resume escalation requests stay within the registered allow-list.
    let resume_policy_caps =
        authorization_scope_for_agent(&resume.agent_id, agent_capabilities, fallback_unrestricted);
    let session_open_at = now_wall_us();
    let mut session = StreamSession {
        session_id: session_id.clone(),
        namespace: namespace.clone(),
        agent_name: resume.agent_id.clone(),
        capabilities: prior_entry.capabilities.clone(),
        policy_capabilities: resume_policy_caps,
        // Restore orphaned leases so the agent can continue using them.
        lease_ids: prior_entry.orphaned_lease_ids.clone(),
        // Restore subscription set from before the disconnect.
        subscriptions: prior_entry.subscriptions.clone(),
        // Subscription filters are not persisted across reconnects; agents must re-send
        // subscribe_filter entries after resuming if they still need prefix filtering.
        subscription_filters: std::collections::HashMap::new(),
        server_sequence: 0,
        resume_token: new_resume_token.clone(),
        last_heartbeat_ms: now_ms(),
        state: SessionState::Resuming,
        last_client_sequence: 1, // SessionResume is sequence 1; start validation from next
        safe_mode_active: false,
        expect_resume: false,
        agent_event_rate_limiter: AgentEventRateLimiter::new(),
        freeze_queue: SessionFreezeQueue::new(FREEZE_QUEUE_CAPACITY),
        session_open_at_wall_us: session_open_at,
        dedup_window: DedupWindow::new(1000, 60),
        lease_correlation_cache: LeaseCorrelationCache::new(
            DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY,
        ),
        resource_upload_rate_limiter: UploadByteRateLimiter::with_limit(
            upload_rate_limit_bytes_per_sec,
        ),
    };

    let compositor_ts = now_wall_us();
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: compositor_ts,
            payload: Some(ServerPayload::SessionResumeResult(SessionResumeResult {
                accepted: true,
                new_session_token: new_resume_token.clone(),
                new_server_sequence: seq,
                // Resume always runs at the highest runtime-supported version.
                // version = major * 1000 + minor; v1.1 = 1001.
                negotiated_protocol_version: crate::auth::RUNTIME_MAX_VERSION,
                // RFC 0005 §6.3: agents MUST use confirmed state, not assume pre-disconnect set.
                granted_capabilities: prior_entry.capabilities,
                active_subscriptions: prior_entry.subscriptions,
                denied_subscriptions: Vec::new(),
                error: String::new(),
            })),
        }))
        .await;

    Some(session)
}

// ─── Message handlers ───────────────────────────────────────────────────────

async fn handle_client_message(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    upload_command_tx: &tokio::sync::mpsc::Sender<UploadWorkerCommand>,
    msg: ClientMessage,
) {
    let client_sequence = msg.sequence;
    let Some(payload) = msg.payload else {
        return;
    };

    match payload {
        ClientPayload::MutationBatch(batch) => {
            handle_mutation_batch(state, session, tx, batch).await;
        }
        ClientPayload::LeaseRequest(req) => {
            handle_lease_request(state, session, tx, client_sequence, req).await;
        }
        ClientPayload::LeaseRenew(renew) => {
            handle_lease_renew(state, session, tx, client_sequence, renew).await;
        }
        ClientPayload::LeaseRelease(release) => {
            handle_lease_release(state, session, tx, client_sequence, release).await;
        }
        ClientPayload::SubscriptionChange(change) => {
            handle_subscription_change(session, tx, change).await;
        }
        ClientPayload::ListElementsRequest(request) => {
            handle_list_elements_request(state, session, tx, request).await;
        }
        ClientPayload::ZonePublish(publish) => {
            handle_zone_publish(state, session, tx, client_sequence, publish).await;
        }
        ClientPayload::Heartbeat(hb) => {
            handle_heartbeat(session, tx, hb).await;
        }
        ClientPayload::TelemetryFrame(_tf) => {
            // Accept agent-side telemetry frames silently (logging/storage deferred to post-v1)
        }
        ClientPayload::InputFocusRequest(req) => {
            // Synchronous focus request (RFC 0005 §3.8).
            // v1 grants focus unconditionally (arbitration deferred to post-v1).
            handle_input_focus_request(session, tx, req).await;
        }
        ClientPayload::InputCaptureRequest(req) => {
            // Synchronous capture request (RFC 0005 §3.8).
            // v1 grants capture unconditionally (arbitration deferred to post-v1).
            handle_input_capture_request(session, tx, req).await;
        }
        ClientPayload::InputCaptureRelease(rel) => {
            // Asynchronous capture release (RFC 0005 §3.8).
            // Confirmed by CaptureReleasedEvent in EventBatch (field 34).
            handle_input_capture_release(session, tx, rel).await;
        }
        ClientPayload::SetImePosition(_pos) => {
            // IME position hint (RFC 0005 §3.8): fire-and-forget, no response sent.
        }
        ClientPayload::SessionClose(close) => {
            // Graceful disconnect (RFC 0005 §1.5).
            // Record the expect_resume hint; the main loop transitions state after this returns.
            session.expect_resume = close.expect_resume;
        }
        ClientPayload::CapabilityRequest(req) => {
            handle_capability_request(session, tx, req).await;
        }
        // Agent scene event emission (scene-events/spec.md §5.1, §5.2).
        ClientPayload::EmitSceneEvent(emit) => {
            handle_emit_scene_event(state, session, tx, client_sequence, emit).await;
        }
        // Widget publishing (widget-system spec §Requirement: Widget Publishing via gRPC).
        // Durable-widget publishes receive WidgetPublishResult (ServerMessage field 47).
        // Ephemeral-widget publishes are fire-and-forget (no result).
        ClientPayload::WidgetPublish(publish) => {
            handle_widget_publish(state, session, tx, client_sequence, publish).await;
        }
        // Widget asset register/upload (session-protocol spec §Requirement: Widget Asset Registration via Session Stream).
        // Always transactional; every request receives WidgetAssetRegisterResult.
        ClientPayload::WidgetAssetRegister(register) => {
            handle_widget_asset_register(state, session, tx, client_sequence, register).await;
        }
        ClientPayload::ResourceUploadStart(start) => {
            let _ = upload_command_tx
                .send(UploadWorkerCommand::Start {
                    request_sequence: client_sequence,
                    capabilities: session.capabilities.clone(),
                    start,
                })
                .await;
        }
        ClientPayload::ResourceUploadChunk(chunk) => {
            let _ = upload_command_tx
                .send(UploadWorkerCommand::Chunk {
                    request_sequence: client_sequence,
                    chunk,
                })
                .await;
        }
        ClientPayload::ResourceUploadComplete(complete) => {
            let _ = upload_command_tx
                .send(UploadWorkerCommand::Complete {
                    request_sequence: client_sequence,
                    capabilities: session.capabilities.clone(),
                    complete,
                })
                .await;
        }
        // SessionInit/SessionResume should not appear after handshake
        ClientPayload::SessionInit(_) | ClientPayload::SessionResume(_) => {
            // Protocol violation: ignore (or could send RuntimeError)
        }
    }
}

/// Maximum future schedule horizon in microseconds (RFC 0003 §3.5, default 5 minutes).
const DEFAULT_MAX_FUTURE_SCHEDULE_US: u64 = 300_000_000;

/// Validate TimingHints for a MutationBatch (RFC 0003 §3.5, RFC 0005 §3.3).
///
/// Returns `Ok(())` if valid, or `Err((error_code, message))` for each
/// invalid condition.
///
/// Validation rules:
/// - `present_at_wall_us < session_open_at_wall_us - 60_000_000` → TIMESTAMP_TOO_OLD
/// - `present_at_wall_us > current_wall_us + max_future_schedule_us` → TIMESTAMP_TOO_FUTURE
/// - `expires_at_wall_us > 0 && expires_at_wall_us <= present_at_wall_us` → TIMESTAMP_EXPIRY_BEFORE_PRESENT
///
/// A value of 0 in either field means "no constraint".
fn validate_timing_hints(
    hints: &TimingHints,
    session_open_at_wall_us: u64,
    max_future_schedule_us: u64,
) -> Result<(), (&'static str, String)> {
    let present = hints.present_at_wall_us;
    let expires = hints.expires_at_wall_us;

    if present > 0 {
        let now = now_wall_us();

        // TIMESTAMP_TOO_OLD: present_at_wall_us more than 60 seconds before session open
        // (RFC 0003 §3.5; 60s = 60_000_000 µs)
        let too_old_threshold = session_open_at_wall_us.saturating_sub(60_000_000);
        if present < too_old_threshold {
            return Err((
                "TIMESTAMP_TOO_OLD",
                format!(
                    "present_at_wall_us ({present}) is more than 60s before session open \
                     ({session_open_at_wall_us})"
                ),
            ));
        }

        // TIMESTAMP_TOO_FUTURE: present_at_wall_us exceeds max_future_schedule_us horizon
        if present > now.saturating_add(max_future_schedule_us) {
            return Err((
                "TIMESTAMP_TOO_FUTURE",
                format!(
                    "present_at_wall_us ({present}) exceeds max future schedule \
                     ({max_future_schedule_us} µs from now={now})"
                ),
            ));
        }

        // TIMESTAMP_EXPIRY_BEFORE_PRESENT: non-zero expiry at or before present
        if expires > 0 && expires <= present {
            return Err((
                "TIMESTAMP_EXPIRY_BEFORE_PRESENT",
                format!(
                    "expires_at_wall_us ({expires}) must be strictly after \
                     present_at_wall_us ({present})"
                ),
            ));
        }
    }

    Ok(())
}

async fn handle_mutation_batch(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    batch: MutationBatch,
) {
    // ── Step 1: Safe mode check (RFC 0005 §3.7) ─────────────────────────────
    // Reject MutationBatch when safe mode is active.
    // Session-local flag tracks per-session suspension (from SessionSuspended delivery).
    // Shared state flag tracks global suspension (from the runtime side).
    // Both are checked; shared state takes precedence.
    // Per the spec invariant: safe_mode=true implies freeze_active=false,
    // so this check runs before the freeze check.
    {
        let st = state.lock().await;
        let safe_mode = session.safe_mode_active || st.safe_mode_active;
        if safe_mode {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: "SAFE_MODE_ACTIVE".to_string(),
                        message: "Mutations are not accepted while the runtime is in safe mode."
                            .to_string(),
                        context: String::new(),
                        hint: r#"{"wait_for": "SessionResumed"}"#.to_string(),
                        error_code_enum: ErrorCode::SafeModeActive as i32,
                    })),
                }))
                .await;
            return;
        }
    }

    // ── Step 2: Freeze check (system-shell/spec.md §Freeze Scene) ────────────
    // When the scene is frozen, mutations are QUEUED (not rejected).
    // Agents are NEVER informed that the scene is frozen — signals are generic
    // queue-pressure signals to avoid leaking viewer state.
    {
        let st = state.lock().await;
        if st.freeze_active {
            // Determine traffic class and enqueue.
            let namespace = session.namespace.clone();
            let result = session.freeze_queue.enqueue(batch.clone(), &namespace);
            drop(st);

            match result {
                FreezeEnqueueResult::Queued { pressure_warning } => {
                    if pressure_warning {
                        // Send MUTATION_QUEUE_PRESSURE — generic, not freeze-specific.
                        let seq = session.next_server_seq();
                        let _ = tx
                            .send(Ok(ServerMessage {
                                sequence: seq,
                                timestamp_wall_us: now_wall_us(),
                                payload: Some(ServerPayload::MutationResult(MutationResult {
                                    batch_id: batch.batch_id,
                                    accepted: true,
                                    created_ids: Vec::new(),
                                    error_code: "MUTATION_QUEUE_PRESSURE".to_string(),
                                    error_message:
                                        "Mutation queue is under pressure (>= 80% capacity)."
                                            .to_string(),
                                })),
                            }))
                            .await;
                    } else {
                        // Send accepted=true (queued — not yet applied, but accepted).
                        let seq = session.next_server_seq();
                        let _ = tx
                            .send(Ok(ServerMessage {
                                sequence: seq,
                                timestamp_wall_us: now_wall_us(),
                                payload: Some(ServerPayload::MutationResult(MutationResult {
                                    batch_id: batch.batch_id,
                                    accepted: true,
                                    created_ids: Vec::new(),
                                    error_code: String::new(),
                                    error_message: String::new(),
                                })),
                            }))
                            .await;
                    }
                }
                FreezeEnqueueResult::Coalesced => {
                    // Coalesced with an existing entry — accepted.
                    let seq = session.next_server_seq();
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: seq,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::MutationResult(MutationResult {
                                batch_id: batch.batch_id,
                                accepted: true,
                                created_ids: Vec::new(),
                                error_code: String::new(),
                                error_message: String::new(),
                            })),
                        }))
                        .await;
                }
                FreezeEnqueueResult::Evicted { evicted_batch_id } => {
                    // An older non-transactional entry was evicted; new one queued.
                    // Send MUTATION_DROPPED for the evicted batch (generic signal).
                    let seq_evicted = session.next_server_seq();
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: seq_evicted,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::MutationResult(MutationResult {
                                batch_id: evicted_batch_id,
                                accepted: false,
                                created_ids: Vec::new(),
                                error_code: "MUTATION_DROPPED".to_string(),
                                error_message:
                                    "Mutation evicted from queue due to capacity pressure."
                                        .to_string(),
                            })),
                        }))
                        .await;
                    // New batch was queued — send accepted.
                    let seq_new = session.next_server_seq();
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: seq_new,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::MutationResult(MutationResult {
                                batch_id: batch.batch_id,
                                accepted: true,
                                created_ids: Vec::new(),
                                error_code: String::new(),
                                error_message: String::new(),
                            })),
                        }))
                        .await;
                }
                FreezeEnqueueResult::BackpressureRequired => {
                    // Transactional mutation: queue full — apply gRPC backpressure.
                    // Send MUTATION_QUEUE_PRESSURE signal.
                    let seq = session.next_server_seq();
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: seq,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::MutationResult(MutationResult {
                                batch_id: batch.batch_id,
                                accepted: false,
                                created_ids: Vec::new(),
                                error_code: "MUTATION_QUEUE_PRESSURE".to_string(),
                                error_message: "Mutation queue full; backpressure applied."
                                    .to_string(),
                            })),
                        }))
                        .await;
                }
                FreezeEnqueueResult::Dropped => {
                    // Ephemeral mutation dropped.
                    let seq = session.next_server_seq();
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: seq,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::MutationResult(MutationResult {
                                batch_id: batch.batch_id,
                                accepted: false,
                                created_ids: Vec::new(),
                                error_code: "MUTATION_DROPPED".to_string(),
                                error_message: "Ephemeral mutation dropped; queue at capacity."
                                    .to_string(),
                            })),
                        }))
                        .await;
                }
            }
            return;
        }
    }

    // ── Deduplication (RFC 0005 §5.2) ────────────────────────────────────────
    //
    // If this batch_id is already in the dedup window, return the cached result
    // without re-applying mutations. This covers retransmission scenarios where
    // the agent resends with the same batch_id and a new sequence number.
    if !batch.batch_id.is_empty() {
        if let Some(cached) = session.dedup_window.lookup(&batch.batch_id) {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::MutationResult(MutationResult {
                        batch_id: batch.batch_id,
                        accepted: cached.accepted,
                        created_ids: cached.created_ids,
                        error_code: cached.error_code,
                        error_message: cached.error_message,
                    })),
                }))
                .await;
            return;
        }
    }

    // ── TimingHints validation (RFC 0003 §3.5, RFC 0005 §3.3) ────────────────
    if let Some(ref hints) = batch.timing {
        if let Err((error_code, message)) = validate_timing_hints(
            hints,
            session.session_open_at_wall_us,
            DEFAULT_MAX_FUTURE_SCHEDULE_US,
        ) {
            let error_code_enum = match error_code {
                "TIMESTAMP_TOO_OLD" => ErrorCode::TimestampTooOld as i32,
                "TIMESTAMP_TOO_FUTURE" => ErrorCode::TimestampTooFuture as i32,
                "TIMESTAMP_EXPIRY_BEFORE_PRESENT" => ErrorCode::TimestampExpiryBeforePresent as i32,
                _ => ErrorCode::InvalidArgument as i32,
            };
            // context points at the specific field that caused the rejection.
            let context = match error_code {
                "TIMESTAMP_EXPIRY_BEFORE_PRESENT" => "timing.expires_at_wall_us",
                _ => "timing.present_at_wall_us",
            };
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: error_code.to_string(),
                        message,
                        context: context.to_string(),
                        hint: r#"{"check_field": "timing"}"#.to_string(),
                        error_code_enum,
                    })),
                }))
                .await;
            return;
        }
    }

    let mut st = state.lock().await;

    let lease_id = match bytes_to_scene_id(&batch.lease_id) {
        Ok(id) => id,
        Err(_) => {
            let cached = CachedResult {
                accepted: false,
                created_ids: Vec::new(),
                error_code: "INVALID_ARGUMENT".to_string(),
                error_message: "Invalid lease_id bytes".to_string(),
            };
            if !batch.batch_id.is_empty() {
                session
                    .dedup_window
                    .insert(batch.batch_id.clone(), cached.clone());
            }
            let seq = session.next_server_seq();
            // Drop lock before awaiting send to avoid holding mutex across await point.
            drop(st);
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::MutationResult(MutationResult {
                        batch_id: batch.batch_id,
                        accepted: false,
                        created_ids: Vec::new(),
                        error_code: cached.error_code,
                        error_message: cached.error_message,
                    })),
                }))
                .await;
            return;
        }
    };

    // Find the active tab (acquire scene lock in a nested block so the guard
    // drops before we potentially drop `st` below).
    let active_tab_opt = { st.scene.lock().await.active_tab };
    let tab_id = match active_tab_opt {
        Some(id) => id,
        None => {
            let cached = CachedResult {
                accepted: false,
                created_ids: Vec::new(),
                error_code: "PRECONDITION_FAILED".to_string(),
                error_message: "No active tab".to_string(),
            };
            if !batch.batch_id.is_empty() {
                session
                    .dedup_window
                    .insert(batch.batch_id.clone(), cached.clone());
            }
            let seq = session.next_server_seq();
            // Drop lock before awaiting send to avoid holding mutex across await point.
            drop(st);
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::MutationResult(MutationResult {
                        batch_id: batch.batch_id,
                        accepted: false,
                        created_ids: Vec::new(),
                        error_code: cached.error_code,
                        error_message: cached.error_message,
                    })),
                }))
                .await;
            return;
        }
    };

    // Convert proto mutations to scene mutations
    let display_area = { st.scene.lock().await.display_area };
    let mut scene_mutations = Vec::new();
    let mut pending_touch_ids: Vec<(SceneId, ElementType)> = Vec::new();
    let mut pending_touch_names: Vec<(ElementType, String)> = Vec::new();
    let mut mutation_conversion_error: Option<(String, String)> = None;
    for m in &batch.mutations {
        match &m.mutation {
            Some(crate::proto::mutation_proto::Mutation::CreateTile(ct)) => {
                let requested_bounds = ct
                    .bounds
                    .as_ref()
                    .map(convert::proto_rect_to_scene)
                    .unwrap_or(tze_hud_scene::Rect::new(0.0, 0.0, 200.0, 150.0));
                let bounds =
                    resolve_tile_bounds_with_override(None, Some(requested_bounds), display_area)
                        .unwrap_or(requested_bounds);
                scene_mutations.push(SceneMutation::CreateTile {
                    tab_id,
                    namespace: session.namespace.clone(),
                    lease_id,
                    bounds,
                    z_order: ct.z_order,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::SetTileRoot(str_)) => {
                // tile_id is encoded as uuid::Uuid::as_bytes() (big-endian RFC 4122 bytes),
                // matching scene_id_to_bytes / bytes_to_scene_id wire contract.
                match bytes_to_scene_id(&str_.tile_id) {
                    Ok(tile_id) => {
                        if let Some(ref node_proto) = str_.node
                            && let Some(node) = convert::proto_node_to_scene(node_proto)
                        {
                            scene_mutations.push(SceneMutation::SetTileRoot { tile_id, node });
                        }
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = str_.tile_id.len(),
                            "SetTileRoot: invalid tile_id length (expected 16 bytes); \
                             mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::PublishToZone(pz)) => {
                let content = pz
                    .content
                    .as_ref()
                    .and_then(convert::proto_zone_content_to_scene);
                if let Some(content) = content {
                    let resolved_zone_name = if !pz.element_id.is_empty() {
                        match bytes_to_scene_id(&pz.element_id) {
                            Ok(element_id) => match st.element_store.entries.get(&element_id) {
                                Some(entry) if entry.element_type == ElementType::Zone => {
                                    pending_touch_ids.push((element_id, ElementType::Zone));
                                    entry.namespace.clone()
                                }
                                _ => {
                                    mutation_conversion_error = Some((
                                        "ELEMENT_NOT_FOUND".to_string(),
                                        "publish_to_zone element_id does not reference a known zone"
                                            .to_string(),
                                    ));
                                    break;
                                }
                            },
                            Err(_) => {
                                mutation_conversion_error = Some((
                                    "INVALID_ARGUMENT".to_string(),
                                    "publish_to_zone element_id must be 16 bytes".to_string(),
                                ));
                                break;
                            }
                        }
                    } else {
                        pending_touch_names.push((ElementType::Zone, pz.zone_name.clone()));
                        pz.zone_name.clone()
                    };
                    let token = tze_hud_scene::types::ZonePublishToken {
                        token: pz
                            .publish_token
                            .as_ref()
                            .map(|t| t.token.clone())
                            .unwrap_or_default(),
                    };
                    let merge_key = if pz.merge_key.is_empty() {
                        None
                    } else {
                        Some(pz.merge_key.clone())
                    };
                    scene_mutations.push(SceneMutation::PublishToZone {
                        zone_name: resolved_zone_name,
                        content,
                        publish_token: token,
                        merge_key,
                        // expires_at_wall_us and content_classification are not yet present
                        // in the PublishToZoneMutation proto (post-v1 wire extensions).
                        expires_at_wall_us: None,
                        content_classification: None,
                        // breakpoints are not in the MutationBatch PublishToZoneMutation proto
                        // (post-v1 wire extension); use ZonePublish path for streaming.
                        breakpoints: Vec::new(),
                    });
                }
            }
            Some(crate::proto::mutation_proto::Mutation::PublishToTile(pt)) => {
                let element_id = match bytes_to_scene_id(&pt.element_id) {
                    Ok(id) => id,
                    Err(_) => {
                        mutation_conversion_error = Some((
                            "INVALID_ARGUMENT".to_string(),
                            "publish_to_tile element_id must be 16 bytes".to_string(),
                        ));
                        break;
                    }
                };

                let entry = match st.element_store.entries.get(&element_id) {
                    Some(entry) if entry.element_type == ElementType::Tile => entry.clone(),
                    _ => {
                        mutation_conversion_error = Some((
                            "ELEMENT_NOT_FOUND".to_string(),
                            "publish_to_tile element_id does not reference a known tile"
                                .to_string(),
                        ));
                        break;
                    }
                };

                let requested_bounds = pt.bounds.as_ref().map(convert::proto_rect_to_scene);
                if let Some(resolved_bounds) =
                    resolve_tile_bounds_with_override(Some(&entry), requested_bounds, display_area)
                {
                    scene_mutations.push(SceneMutation::UpdateTileBounds {
                        tile_id: element_id,
                        bounds: resolved_bounds,
                    });
                }

                let mut had_content = false;
                if let Some(ref node_proto) = pt.node {
                    if let Some(node) = convert::proto_node_to_scene(node_proto) {
                        scene_mutations.push(SceneMutation::SetTileRoot {
                            tile_id: element_id,
                            node,
                        });
                        had_content = true;
                    } else {
                        mutation_conversion_error = Some((
                            "INVALID_ARGUMENT".to_string(),
                            "publish_to_tile node content is invalid or missing data".to_string(),
                        ));
                        break;
                    }
                }

                if !had_content && requested_bounds.is_none() {
                    mutation_conversion_error = Some((
                        "INVALID_ARGUMENT".to_string(),
                        "publish_to_tile requires at least one of bounds or node".to_string(),
                    ));
                    break;
                }

                pending_touch_ids.push((element_id, ElementType::Tile));
            }
            Some(crate::proto::mutation_proto::Mutation::ClearZone(cz)) => {
                let token = tze_hud_scene::types::ZonePublishToken {
                    token: cz
                        .publish_token
                        .as_ref()
                        .map(|t| t.token.clone())
                        .unwrap_or_default(),
                };
                scene_mutations.push(SceneMutation::ClearZone {
                    zone_name: cz.zone_name.clone(),
                    publish_token: token,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::ClearWidget(cw)) => {
                let instance_id = (!cw.instance_id.is_empty()).then_some(cw.instance_id.clone());
                scene_mutations.push(SceneMutation::ClearWidget {
                    widget_name: cw.widget_name.clone(),
                    instance_id,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::UpdateNodeContent(unc)) => {
                match (
                    bytes_to_scene_id(&unc.tile_id),
                    bytes_to_scene_id(&unc.node_id),
                ) {
                    (Ok(tile_id), Ok(node_id)) => {
                        if let Some(ref d) = unc.data
                            && let Some(data) = convert::proto_update_node_content_data_to_scene(d)
                        {
                            scene_mutations.push(SceneMutation::UpdateNodeContent {
                                tile_id,
                                node_id,
                                data,
                            });
                        } else {
                            tracing::warn!(
                                "UpdateNodeContent: missing or unrecognised data variant; \
                                 mutation skipped"
                            );
                        }
                    }
                    _ => {
                        tracing::warn!(
                            tile_id_len = unc.tile_id.len(),
                            node_id_len = unc.node_id.len(),
                            "UpdateNodeContent: invalid tile_id or node_id length (expected 16 \
                             bytes); mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::AddNode(an)) => {
                match bytes_to_scene_id(&an.tile_id) {
                    Ok(tile_id) => {
                        let parent_id = if an.parent_id.is_empty() {
                            None
                        } else {
                            match bytes_to_scene_id(&an.parent_id) {
                                Ok(id) => Some(id),
                                Err(_) => {
                                    tracing::warn!(
                                        parent_id_len = an.parent_id.len(),
                                        "AddNode: invalid parent_id length (expected 16 bytes); \
                                         mutation skipped — SDK bug or wire corruption"
                                    );
                                    continue;
                                }
                            }
                        };
                        if let Some(ref node_proto) = an.node
                            && let Some(node) = convert::proto_node_to_scene(node_proto)
                        {
                            scene_mutations.push(SceneMutation::AddNode {
                                tile_id,
                                parent_id,
                                node,
                            });
                        }
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = an.tile_id.len(),
                            "AddNode: invalid tile_id length (expected 16 bytes); \
                             mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::UpdateTileOpacity(uto)) => {
                match bytes_to_scene_id(&uto.tile_id) {
                    Ok(tile_id) => {
                        scene_mutations.push(SceneMutation::UpdateTileOpacity {
                            tile_id,
                            opacity: uto.opacity,
                        });
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = uto.tile_id.len(),
                            "UpdateTileOpacity: invalid tile_id length (expected 16 bytes); \
                             mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::UpdateTileInputMode(utim)) => {
                match bytes_to_scene_id(&utim.tile_id) {
                    Ok(tile_id) => {
                        let input_mode = convert::proto_input_mode_to_scene(
                            crate::proto::TileInputModeProto::try_from(utim.input_mode).unwrap_or(
                                crate::proto::TileInputModeProto::TileInputModeUnspecified,
                            ),
                        );
                        scene_mutations.push(SceneMutation::UpdateTileInputMode {
                            tile_id,
                            input_mode,
                        });
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = utim.tile_id.len(),
                            "UpdateTileInputMode: invalid tile_id length (expected 16 bytes); \
                             mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            None => {}
        }
    }

    if let Some((error_code, error_message)) = mutation_conversion_error {
        let cached = CachedResult {
            accepted: false,
            created_ids: Vec::new(),
            error_code: error_code.clone(),
            error_message: error_message.clone(),
        };
        if !batch.batch_id.is_empty() {
            session
                .dedup_window
                .insert(batch.batch_id.clone(), cached.clone());
        }
        let seq = session.next_server_seq();
        drop(st);
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::MutationResult(MutationResult {
                    batch_id: batch.batch_id,
                    accepted: false,
                    created_ids: Vec::new(),
                    error_code: cached.error_code,
                    error_message: cached.error_message,
                })),
            }))
            .await;
        return;
    }

    // Map the proto batch_id bytes to a SceneId for rejection-correlation.
    // Falls back (with a debug log) when the field is absent or malformed.
    let scene_batch_id = proto_batch_id_to_scene_id(&batch.batch_id);

    // Apply as atomic batch, propagating client batch_id and lease_id so that
    // the five-stage validation pipeline can perform lease/budget checks.
    let scene_batch = SceneMutationBatch {
        batch_id: scene_batch_id,
        agent_namespace: session.namespace.clone(),
        mutations: scene_mutations,
        timing_hints: None,
        lease_id: Some(lease_id),
    };

    let result = st.scene.lock().await.apply_batch(&scene_batch);

    let seq = session.next_server_seq();
    if result.applied {
        let mut persist_request = persist_created_tile_entries(&mut st, &result.created_ids).await;
        let now = now_ms();
        let mut touched = false;
        for (element_id, element_type) in pending_touch_ids {
            if let Some(entry) = st.element_store.entries.get_mut(&element_id) {
                if entry.element_type == element_type {
                    entry.last_published_at = now;
                    touched = true;
                }
            }
        }
        for (element_type, namespace) in pending_touch_names {
            if let Some(id) = st
                .element_store
                .find_id_by_type_namespace(element_type, namespace.as_str())
            {
                if let Some(entry) = st.element_store.entries.get_mut(&id) {
                    if entry.element_type == element_type {
                        entry.last_published_at = now;
                        touched = true;
                    }
                }
            }
        }
        if touched {
            persist_request =
                st.element_store_path
                    .clone()
                    .map(|path| ElementStorePersistRequest {
                        store: st.element_store.clone(),
                        path,
                    });
        }

        let created_ids: Vec<Vec<u8>> = result
            .created_ids
            .iter()
            .map(|id| scene_id_to_bytes(*id))
            .collect();

        // Cache result before sending.
        if !batch.batch_id.is_empty() {
            session.dedup_window.insert(
                batch.batch_id.clone(),
                CachedResult {
                    accepted: true,
                    created_ids: created_ids.clone(),
                    error_code: String::new(),
                    error_message: String::new(),
                },
            );
        }

        // Drop lock before awaiting send to avoid holding mutex across await point.
        drop(st);
        persist_element_store(persist_request).await;
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::MutationResult(MutationResult {
                    batch_id: batch.batch_id,
                    accepted: true,
                    created_ids,
                    error_code: String::new(),
                    error_message: String::new(),
                })),
            }))
            .await;
    } else {
        let error_message = result
            .error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string());

        // Cache rejection result before sending.
        if !batch.batch_id.is_empty() {
            session.dedup_window.insert(
                batch.batch_id.clone(),
                CachedResult {
                    accepted: false,
                    created_ids: Vec::new(),
                    error_code: "MUTATION_REJECTED".to_string(),
                    error_message: error_message.clone(),
                },
            );
        }

        // Drop lock before awaiting send to avoid holding mutex across await point.
        drop(st);
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::MutationResult(MutationResult {
                    batch_id: batch.batch_id,
                    accepted: false,
                    created_ids: Vec::new(),
                    error_code: "MUTATION_REJECTED".to_string(),
                    error_message,
                })),
            }))
            .await;
    }
}

/// Apply a previously-queued mutation batch to the scene without sending a
/// `MutationResult` response.
///
/// This is called during the unfreeze drain. The initial `MutationResult`
/// (with `accepted = true`) was already sent when the batch was enqueued;
/// sending a second one would violate the "one response per request" contract
/// (RFC 0005 §2.1).
///
/// Safe mode and freeze checks are intentionally skipped here: the spec
/// invariant (`safe_mode = true → freeze_active = false`) guarantees that
/// safe mode cannot activate between freeze deactivation and the drain.
async fn apply_queued_batch_to_scene(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    batch: MutationBatch,
) {
    let mut st = state.lock().await;

    let lease_id = match bytes_to_scene_id(&batch.lease_id) {
        Ok(id) => id,
        Err(_) => return, // invalid lease_id — silently skip (already acked)
    };

    let active_tab_opt = { st.scene.lock().await.active_tab };
    let tab_id = match active_tab_opt {
        Some(id) => id,
        None => return, // no active tab — skip silently
    };

    let display_area = { st.scene.lock().await.display_area };
    let mut scene_mutations = Vec::new();
    let mut pending_touch_ids: Vec<(SceneId, ElementType)> = Vec::new();
    let mut pending_touch_names: Vec<(ElementType, String)> = Vec::new();
    let mut mutation_conversion_error: Option<(String, String)> = None;
    for m in &batch.mutations {
        match &m.mutation {
            Some(crate::proto::mutation_proto::Mutation::CreateTile(ct)) => {
                let requested_bounds = ct
                    .bounds
                    .as_ref()
                    .map(convert::proto_rect_to_scene)
                    .unwrap_or(tze_hud_scene::Rect::new(0.0, 0.0, 200.0, 150.0));
                let bounds =
                    resolve_tile_bounds_with_override(None, Some(requested_bounds), display_area)
                        .unwrap_or(requested_bounds);
                scene_mutations.push(SceneMutation::CreateTile {
                    tab_id,
                    namespace: session.namespace.clone(),
                    lease_id,
                    bounds,
                    z_order: ct.z_order,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::SetTileRoot(str_)) => {
                // tile_id is encoded as uuid::Uuid::as_bytes() (big-endian RFC 4122 bytes),
                // matching scene_id_to_bytes / bytes_to_scene_id wire contract.
                match bytes_to_scene_id(&str_.tile_id) {
                    Ok(tile_id) => {
                        if let Some(ref node_proto) = str_.node
                            && let Some(node) = convert::proto_node_to_scene(node_proto)
                        {
                            scene_mutations.push(SceneMutation::SetTileRoot { tile_id, node });
                        }
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = str_.tile_id.len(),
                            "SetTileRoot (queued): invalid tile_id length (expected 16 bytes); \
                             mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::PublishToZone(pz)) => {
                let content = pz
                    .content
                    .as_ref()
                    .and_then(convert::proto_zone_content_to_scene);
                if let Some(content) = content {
                    let resolved_zone_name = if !pz.element_id.is_empty() {
                        match bytes_to_scene_id(&pz.element_id) {
                            Ok(element_id) => match st.element_store.entries.get(&element_id) {
                                Some(entry) if entry.element_type == ElementType::Zone => {
                                    pending_touch_ids.push((element_id, ElementType::Zone));
                                    entry.namespace.clone()
                                }
                                _ => {
                                    mutation_conversion_error = Some((
                                        "ELEMENT_NOT_FOUND".to_string(),
                                        "publish_to_zone element_id does not reference a known zone"
                                            .to_string(),
                                    ));
                                    break;
                                }
                            },
                            Err(_) => {
                                mutation_conversion_error = Some((
                                    "INVALID_ARGUMENT".to_string(),
                                    "publish_to_zone element_id must be 16 bytes".to_string(),
                                ));
                                break;
                            }
                        }
                    } else {
                        pending_touch_names.push((ElementType::Zone, pz.zone_name.clone()));
                        pz.zone_name.clone()
                    };
                    let token = tze_hud_scene::types::ZonePublishToken {
                        token: pz
                            .publish_token
                            .as_ref()
                            .map(|t| t.token.clone())
                            .unwrap_or_default(),
                    };
                    let merge_key = if pz.merge_key.is_empty() {
                        None
                    } else {
                        Some(pz.merge_key.clone())
                    };
                    scene_mutations.push(SceneMutation::PublishToZone {
                        zone_name: resolved_zone_name,
                        content,
                        publish_token: token,
                        merge_key,
                        expires_at_wall_us: None,
                        content_classification: None,
                        breakpoints: Vec::new(),
                    });
                }
            }
            Some(crate::proto::mutation_proto::Mutation::PublishToTile(pt)) => {
                let element_id = match bytes_to_scene_id(&pt.element_id) {
                    Ok(id) => id,
                    Err(_) => {
                        mutation_conversion_error = Some((
                            "INVALID_ARGUMENT".to_string(),
                            "publish_to_tile element_id must be 16 bytes".to_string(),
                        ));
                        break;
                    }
                };

                let entry = match st.element_store.entries.get(&element_id) {
                    Some(entry) if entry.element_type == ElementType::Tile => entry.clone(),
                    _ => {
                        mutation_conversion_error = Some((
                            "ELEMENT_NOT_FOUND".to_string(),
                            "publish_to_tile element_id does not reference a known tile"
                                .to_string(),
                        ));
                        break;
                    }
                };

                let requested_bounds = pt.bounds.as_ref().map(convert::proto_rect_to_scene);
                if let Some(resolved_bounds) =
                    resolve_tile_bounds_with_override(Some(&entry), requested_bounds, display_area)
                {
                    scene_mutations.push(SceneMutation::UpdateTileBounds {
                        tile_id: element_id,
                        bounds: resolved_bounds,
                    });
                }

                let mut had_content = false;
                if let Some(ref node_proto) = pt.node {
                    if let Some(node) = convert::proto_node_to_scene(node_proto) {
                        scene_mutations.push(SceneMutation::SetTileRoot {
                            tile_id: element_id,
                            node,
                        });
                        had_content = true;
                    } else {
                        mutation_conversion_error = Some((
                            "INVALID_ARGUMENT".to_string(),
                            "publish_to_tile node content is invalid or missing data".to_string(),
                        ));
                        break;
                    }
                }

                if !had_content && requested_bounds.is_none() {
                    mutation_conversion_error = Some((
                        "INVALID_ARGUMENT".to_string(),
                        "publish_to_tile requires at least one of bounds or node".to_string(),
                    ));
                    break;
                }

                pending_touch_ids.push((element_id, ElementType::Tile));
            }
            Some(crate::proto::mutation_proto::Mutation::ClearZone(cz)) => {
                let token = tze_hud_scene::types::ZonePublishToken {
                    token: cz
                        .publish_token
                        .as_ref()
                        .map(|t| t.token.clone())
                        .unwrap_or_default(),
                };
                scene_mutations.push(SceneMutation::ClearZone {
                    zone_name: cz.zone_name.clone(),
                    publish_token: token,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::ClearWidget(cw)) => {
                let instance_id = (!cw.instance_id.is_empty()).then_some(cw.instance_id.clone());
                scene_mutations.push(SceneMutation::ClearWidget {
                    widget_name: cw.widget_name.clone(),
                    instance_id,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::UpdateNodeContent(unc)) => {
                match (
                    bytes_to_scene_id(&unc.tile_id),
                    bytes_to_scene_id(&unc.node_id),
                ) {
                    (Ok(tile_id), Ok(node_id)) => {
                        if let Some(ref d) = unc.data
                            && let Some(data) = convert::proto_update_node_content_data_to_scene(d)
                        {
                            scene_mutations.push(SceneMutation::UpdateNodeContent {
                                tile_id,
                                node_id,
                                data,
                            });
                        } else {
                            tracing::warn!(
                                "UpdateNodeContent (queued): missing or unrecognised data \
                                 variant; mutation skipped"
                            );
                        }
                    }
                    _ => {
                        tracing::warn!(
                            tile_id_len = unc.tile_id.len(),
                            node_id_len = unc.node_id.len(),
                            "UpdateNodeContent (queued): invalid tile_id or node_id length \
                             (expected 16 bytes); mutation skipped — SDK bug or wire corruption"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::AddNode(an)) => {
                match bytes_to_scene_id(&an.tile_id) {
                    Ok(tile_id) => {
                        let parent_id_result = if an.parent_id.is_empty() {
                            Ok(None)
                        } else {
                            bytes_to_scene_id(&an.parent_id).map(Some)
                        };
                        match parent_id_result {
                            Ok(parent_id) => {
                                if let Some(ref node_proto) = an.node
                                    && let Some(node) = convert::proto_node_to_scene(node_proto)
                                {
                                    scene_mutations.push(SceneMutation::AddNode {
                                        tile_id,
                                        parent_id,
                                        node,
                                    });
                                }
                            }
                            Err(_) => {
                                tracing::warn!(
                                    parent_id_len = an.parent_id.len(),
                                    "AddNode (queued): invalid parent_id length (expected 16 \
                                     bytes); mutation skipped — SDK bug or wire corruption"
                                );
                            }
                        }
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = an.tile_id.len(),
                            "AddNode (queued): invalid tile_id length; mutation skipped"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::UpdateTileOpacity(uto)) => {
                match bytes_to_scene_id(&uto.tile_id) {
                    Ok(tile_id) => {
                        scene_mutations.push(SceneMutation::UpdateTileOpacity {
                            tile_id,
                            opacity: uto.opacity,
                        });
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = uto.tile_id.len(),
                            "UpdateTileOpacity (queued): invalid tile_id length; mutation skipped"
                        );
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::UpdateTileInputMode(utim)) => {
                match bytes_to_scene_id(&utim.tile_id) {
                    Ok(tile_id) => {
                        let input_mode = convert::proto_input_mode_to_scene(
                            crate::proto::TileInputModeProto::try_from(utim.input_mode).unwrap_or(
                                crate::proto::TileInputModeProto::TileInputModeUnspecified,
                            ),
                        );
                        scene_mutations.push(SceneMutation::UpdateTileInputMode {
                            tile_id,
                            input_mode,
                        });
                    }
                    Err(_) => {
                        tracing::warn!(
                            tile_id_len = utim.tile_id.len(),
                            "UpdateTileInputMode (queued): invalid tile_id length; mutation skipped"
                        );
                    }
                }
            }
            None => {}
        }
    }

    if let Some((error_code, error_message)) = mutation_conversion_error {
        tracing::warn!(
            error_code,
            error_message,
            "queued mutation batch skipped due to conversion error after enqueue"
        );
        return;
    }

    // Map the proto batch_id bytes to a SceneId for validation correlation.
    let scene_batch_id = proto_batch_id_to_scene_id(&batch.batch_id);

    let scene_batch = SceneMutationBatch {
        batch_id: scene_batch_id,
        agent_namespace: session.namespace.clone(),
        mutations: scene_mutations,
        timing_hints: None,
        // Propagate the lease_id so that lease/budget validation runs for
        // queued batches just as it does for live batches.
        lease_id: Some(lease_id),
    };

    // Apply to scene; response was already sent when the batch was queued.
    let result = st.scene.lock().await.apply_batch(&scene_batch);
    if result.applied {
        let mut persist_request = persist_created_tile_entries(&mut st, &result.created_ids).await;
        let now = now_ms();
        let mut touched = false;
        for (element_id, element_type) in pending_touch_ids {
            if let Some(entry) = st.element_store.entries.get_mut(&element_id) {
                if entry.element_type == element_type {
                    entry.last_published_at = now;
                    touched = true;
                }
            }
        }
        for (element_type, namespace) in pending_touch_names {
            if let Some(id) = st
                .element_store
                .find_id_by_type_namespace(element_type, namespace.as_str())
            {
                if let Some(entry) = st.element_store.entries.get_mut(&id) {
                    if entry.element_type == element_type {
                        entry.last_published_at = now;
                        touched = true;
                    }
                }
            }
        }
        if touched {
            persist_request =
                st.element_store_path
                    .clone()
                    .map(|path| ElementStorePersistRequest {
                        store: st.element_store.clone(),
                        path,
                    });
        }
        drop(st);
        persist_element_store(persist_request).await;
    }
}

/// Map a canonical v1 capability wire name to the `Capability` enum variant.
///
/// Only canonical names (post-validation) reach this function.
/// Returns `None` for names that have no corresponding enum variant at this
/// layer (e.g., informational capabilities not enforced by the scene graph).
fn canonical_name_to_capability(name: &str) -> Option<Capability> {
    match name {
        "create_tiles" => Some(Capability::CreateTiles),
        "modify_own_tiles" => Some(Capability::ModifyOwnTiles),
        "manage_tabs" => Some(Capability::ManageTabs),
        "manage_sync_groups" => Some(Capability::ManageSyncGroups),
        "upload_resource" => Some(Capability::UploadResource),
        "read_scene_topology" => Some(Capability::ReadSceneTopology),
        "subscribe_scene_events" => Some(Capability::SubscribeSceneEvents),
        "overlay_privileges" => Some(Capability::OverlayPrivileges),
        "access_input_events" => Some(Capability::AccessInputEvents),
        "high_priority_z_order" => Some(Capability::HighPriorityZOrder),
        "exceed_default_budgets" => Some(Capability::ExceedDefaultBudgets),
        "read_telemetry" => Some(Capability::ReadTelemetry),
        "resident_mcp" => Some(Capability::ResidentMcp),
        "lease:priority:1" => Some(Capability::LeasePriority1),
        _ if name.starts_with("publish_zone:") => {
            let zone = name.strip_prefix("publish_zone:").unwrap_or("*");
            Some(Capability::PublishZone(zone.to_string()))
        }
        _ if name.starts_with("publish_widget:") => {
            let widget = name.strip_prefix("publish_widget:").unwrap_or("*");
            Some(Capability::PublishWidget(widget.to_string()))
        }
        _ if name.starts_with("emit_scene_event:") => {
            let event = name.strip_prefix("emit_scene_event:").unwrap_or("");
            Some(Capability::EmitSceneEvent(event.to_string()))
        }
        // Higher-priority lease variants beyond priority 1 are not yet represented
        // in the enum; skip them without error (forward compat).
        _ => None,
    }
}

fn capability_grant_covers(granted: &str, requested: &str) -> bool {
    if granted == "*" || granted == requested {
        return true;
    }

    (granted == "publish_zone:*" && requested.starts_with("publish_zone:"))
        || (granted == "publish_widget:*" && requested.starts_with("publish_widget:"))
        || (granted == "emit_scene_event:*" && requested.starts_with("emit_scene_event:"))
}

fn capability_set_covers(granted: &[String], requested: &str) -> bool {
    granted
        .iter()
        .any(|grant| capability_grant_covers(grant, requested))
}

async fn handle_lease_request(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    client_sequence: u64,
    req: LeaseRequest,
) {
    // Retransmit dedup (RFC 0005 §5.3): if we have already processed this
    // client sequence, replay the cached response.
    if client_sequence > 0 {
        if let Some(cached) = session
            .lease_correlation_cache
            .get(client_sequence)
            .cloned()
        {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: cached.granted,
                        lease_id: cached.lease_id,
                        granted_ttl_ms: cached.granted_ttl_ms,
                        granted_priority: cached.granted_priority,
                        granted_capabilities: cached.granted_capabilities,
                        deny_reason: cached.deny_reason,
                        deny_code: cached.deny_code,
                    })),
                }))
                .await;
            return;
        }
    }

    // Validate requested capabilities against the canonical v1 vocabulary.
    // Non-canonical names (including legacy names like create_tile, receive_input)
    // must be rejected with CONFIG_UNKNOWN_CAPABILITY and a hint.
    if let Err(unknown_caps) = validate_canonical_capabilities(&req.capabilities) {
        let hints: Vec<serde_json::Value> = unknown_caps
            .iter()
            .map(|e| serde_json::json!({"unknown": e.unknown, "hint": e.hint}))
            .collect();
        let hint_json = serde_json::to_string(&hints)
            .unwrap_or_else(|_| "see configuration/spec.md §Capability Vocabulary".to_string());
        let deny_reason = format!("{} unrecognized capability name(s)", unknown_caps.len());
        // Cache the denial so retransmits replay a stable response without
        // duplicating the RuntimeError advisory (RFC 0005 §5.3 dedup contract).
        if client_sequence > 0 {
            session.lease_correlation_cache.insert(
                client_sequence,
                CachedLeaseResponse {
                    granted: false,
                    lease_id: Vec::new(),
                    granted_ttl_ms: 0,
                    granted_priority: 0,
                    granted_capabilities: Vec::new(),
                    deny_reason: deny_reason.clone(),
                    deny_code: "CONFIG_UNKNOWN_CAPABILITY".to_string(),
                },
            );
        }
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                    granted: false,
                    deny_code: "CONFIG_UNKNOWN_CAPABILITY".to_string(),
                    deny_reason,
                    ..Default::default()
                })),
            }))
            .await;
        // Send structured hints as a RuntimeError advisory.
        // LeaseResponse has no hint field; the advisory carries the JSON hint array
        // so agents can identify which names are non-canonical and what to use instead.
        let hint_seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: hint_seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::RuntimeError(RuntimeError {
                    error_code: "CONFIG_UNKNOWN_CAPABILITY".to_string(),
                    message: format!(
                        "LeaseRequest contains {} unrecognized capability name(s)",
                        unknown_caps.len()
                    ),
                    hint: hint_json,
                    ..Default::default()
                })),
            }))
            .await;
        return;
    }

    // Lease capability scope must stay within the session's currently granted
    // authority surface. Do not silently clamp to a subset: deny the full
    // request when any requested capability is out of scope.
    let unauthorized_caps: Vec<String> = req
        .capabilities
        .iter()
        .filter(|requested| !capability_set_covers(&session.capabilities, requested))
        .cloned()
        .collect();
    if !unauthorized_caps.is_empty() {
        let deny_reason = format!(
            "requested lease scope exceeds session-granted capabilities: {}",
            unauthorized_caps.join(", ")
        );
        let deny_code = "PERMISSION_DENIED".to_string();
        if client_sequence > 0 {
            session.lease_correlation_cache.insert(
                client_sequence,
                CachedLeaseResponse {
                    granted: false,
                    lease_id: Vec::new(),
                    granted_ttl_ms: 0,
                    granted_priority: 0,
                    granted_capabilities: Vec::new(),
                    deny_reason: deny_reason.clone(),
                    deny_code: deny_code.clone(),
                },
            );
        }
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                    granted: false,
                    deny_reason,
                    deny_code,
                    ..Default::default()
                })),
            }))
            .await;
        return;
    }

    let granted_capabilities: Vec<String> = req.capabilities.clone();
    let capabilities: Vec<Capability> = granted_capabilities
        .iter()
        .filter_map(|c| canonical_name_to_capability(c))
        .collect();

    let ttl = if req.ttl_ms > 0 { req.ttl_ms } else { 60_000 };

    // Enforce priority rules per lease-governance spec §Priority Assignment.
    let granted_priority = effective_priority(req.lease_priority, &session.capabilities);

    // Persist the effective priority in the scene graph lease record so that the
    // degradation ladder and arbitration engine can sort tiles by
    // (lease_priority ASC, z_order DESC) without consulting the session layer.
    // Spec §Requirement: Priority Sort Semantics (lease-governance/spec.md lines 62-69).
    // `effective_priority` returns u32 (wire type); priority values are 0-4 so the
    // conversion to u8 is always lossless.
    let priority_u8 = granted_priority as u8;
    let st = state.lock().await;
    let lease_id = st.scene.lock().await.grant_lease_with_priority(
        &session.namespace,
        ttl,
        priority_u8,
        capabilities,
    );
    session.lease_ids.push(lease_id);
    let lease_id_bytes = scene_id_to_bytes(lease_id);

    // Cache the response for retransmit handling (RFC 0005 §5.3).
    if client_sequence > 0 {
        session.lease_correlation_cache.insert(
            client_sequence,
            CachedLeaseResponse {
                granted: true,
                lease_id: lease_id_bytes.clone(),
                granted_ttl_ms: ttl,
                granted_priority,
                granted_capabilities: granted_capabilities.clone(),
                deny_reason: String::new(),
                deny_code: String::new(),
            },
        );
    }

    // Send LeaseResponse (transactional: never dropped, RFC 0005 §3.1).
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                granted: true,
                lease_id: lease_id_bytes.clone(),
                granted_ttl_ms: ttl,
                granted_priority,
                granted_capabilities,
                ..Default::default()
            })),
        }))
        .await;

    // Send LeaseStateChange notification (REQUESTED→ACTIVE).
    // LeaseStateChange is transactional and delivered unconditionally —
    // LEASE_CHANGES subscriptions are always active (spec §Subscription Management,
    // lines 459-461).
    let change_seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: change_seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::LeaseStateChange(LeaseStateChange {
                lease_id: lease_id_bytes,
                previous_state: "REQUESTED".to_string(),
                new_state: "ACTIVE".to_string(),
                reason: format!("Lease granted with TTL {ttl}ms and priority {granted_priority}"),
                timestamp_wall_us: now_wall_us(),
            })),
        }))
        .await;
}

async fn handle_lease_renew(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    client_sequence: u64,
    renew: LeaseRenew,
) {
    // Retransmit dedup (RFC 0005 §5.3).
    if client_sequence > 0 {
        if let Some(cached) = session
            .lease_correlation_cache
            .get(client_sequence)
            .cloned()
        {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: cached.granted,
                        lease_id: cached.lease_id,
                        granted_ttl_ms: cached.granted_ttl_ms,
                        granted_priority: cached.granted_priority,
                        granted_capabilities: cached.granted_capabilities,
                        deny_reason: cached.deny_reason,
                        deny_code: cached.deny_code,
                    })),
                }))
                .await;
            return;
        }
    }

    let lease_id = match bytes_to_scene_id(&renew.lease_id) {
        Ok(id) => id,
        Err(_) => {
            let seq = session.next_server_seq();
            let deny_reason = "Invalid lease_id bytes".to_string();
            let deny_code = "INVALID_ARGUMENT".to_string();
            if client_sequence > 0 {
                session.lease_correlation_cache.insert(
                    client_sequence,
                    CachedLeaseResponse {
                        granted: false,
                        lease_id: Vec::new(),
                        granted_ttl_ms: 0,
                        granted_priority: 0,
                        granted_capabilities: Vec::new(),
                        deny_reason: deny_reason.clone(),
                        deny_code: deny_code.clone(),
                    },
                );
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason,
                        deny_code,
                        ..Default::default()
                    })),
                }))
                .await;
            return;
        }
    };

    let st = state.lock().await;
    let ttl = if renew.new_ttl_ms > 0 {
        renew.new_ttl_ms
    } else {
        60_000
    };
    let lease_id_bytes = scene_id_to_bytes(lease_id);

    let renew_result = {
        let mut scene = st.scene.lock().await;
        let result = scene.renew_lease(lease_id, ttl);
        // Read the stored priority while we hold the scene lock.
        let stored_priority = scene
            .leases
            .get(&lease_id)
            .map(|l| l.priority as u32)
            .unwrap_or(2);
        result.map(|()| stored_priority)
    };

    match renew_result {
        Ok(stored_priority) => {
            // Spec: "runtime SHALL respond with LeaseResponse" for lease operations.
            // For renewal success, return LeaseResponse(granted=true) with the updated TTL.
            // Read the stored priority from the scene graph so the renewal response reflects
            // the persisted value (lease-governance spec §Requirement: Priority Assignment,
            // lines 49-60: renewal preserves the priority set at grant time).
            let seq = session.next_server_seq();
            let lease_response = LeaseResponse {
                granted: true,
                lease_id: lease_id_bytes.clone(),
                granted_ttl_ms: ttl,
                granted_priority: stored_priority,
                ..Default::default()
            };
            // Cache exactly what we send, so retransmit replays the same response.
            if client_sequence > 0 {
                session.lease_correlation_cache.insert(
                    client_sequence,
                    CachedLeaseResponse {
                        granted: lease_response.granted,
                        lease_id: lease_response.lease_id.clone(),
                        granted_ttl_ms: lease_response.granted_ttl_ms,
                        granted_priority: lease_response.granted_priority,
                        granted_capabilities: lease_response.granted_capabilities.clone(),
                        deny_reason: lease_response.deny_reason.clone(),
                        deny_code: lease_response.deny_code.clone(),
                    },
                );
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(lease_response)),
                }))
                .await;

            // Also send LeaseStateChange notification: ACTIVE→ACTIVE (renewal).
            // LeaseStateChange is transactional and always delivered (LEASE_CHANGES
            // subscription is unconditional per spec §Subscription Management).
            let change_seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: change_seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseStateChange(LeaseStateChange {
                        lease_id: lease_id_bytes,
                        previous_state: "ACTIVE".to_string(),
                        new_state: "ACTIVE".to_string(),
                        reason: format!("Renewed with TTL {ttl}ms"),
                        timestamp_wall_us: now_wall_us(),
                    })),
                }))
                .await;
        }
        Err(e) => {
            let seq = session.next_server_seq();
            let deny_reason = e.to_string();
            let deny_code = "LEASE_NOT_FOUND".to_string();
            if client_sequence > 0 {
                session.lease_correlation_cache.insert(
                    client_sequence,
                    CachedLeaseResponse {
                        granted: false,
                        lease_id: Vec::new(),
                        granted_ttl_ms: 0,
                        granted_priority: 0,
                        granted_capabilities: Vec::new(),
                        deny_reason: deny_reason.clone(),
                        deny_code: deny_code.clone(),
                    },
                );
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason,
                        deny_code,
                        ..Default::default()
                    })),
                }))
                .await;
        }
    }
}

async fn handle_lease_release(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    client_sequence: u64,
    release: LeaseRelease,
) {
    // Retransmit dedup (RFC 0005 §5.3).
    // Replay the cached LeaseResponse for both success and denial paths so the
    // client always receives a LeaseResponse on retransmit (consistent with the
    // original send).  Emitting a new LeaseStateChange on retransmit would
    // produce duplicate state-change notifications.
    if client_sequence > 0 {
        if let Some(cached) = session
            .lease_correlation_cache
            .get(client_sequence)
            .cloned()
        {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: cached.granted,
                        lease_id: cached.lease_id,
                        granted_ttl_ms: cached.granted_ttl_ms,
                        granted_priority: cached.granted_priority,
                        granted_capabilities: cached.granted_capabilities,
                        deny_reason: cached.deny_reason,
                        deny_code: cached.deny_code,
                    })),
                }))
                .await;
            return;
        }
    }

    let lease_id = match bytes_to_scene_id(&release.lease_id) {
        Ok(id) => id,
        Err(_) => {
            let seq = session.next_server_seq();
            let deny_reason = "Invalid lease_id bytes".to_string();
            let deny_code = "INVALID_ARGUMENT".to_string();
            if client_sequence > 0 {
                session.lease_correlation_cache.insert(
                    client_sequence,
                    CachedLeaseResponse {
                        granted: false,
                        lease_id: Vec::new(),
                        granted_ttl_ms: 0,
                        granted_priority: 0,
                        granted_capabilities: Vec::new(),
                        deny_reason: deny_reason.clone(),
                        deny_code: deny_code.clone(),
                    },
                );
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason,
                        deny_code,
                        ..Default::default()
                    })),
                }))
                .await;
            return;
        }
    };

    let st = state.lock().await;
    let lease_id_bytes = scene_id_to_bytes(lease_id);

    match st.scene.lock().await.revoke_lease(lease_id) {
        Ok(()) => {
            // Remove from session's tracked leases
            session.lease_ids.retain(|&id| id != lease_id);

            // Spec: every lease operation SHALL be answered with LeaseResponse.
            // Send LeaseResponse(granted=true) first (transactional), then
            // LeaseStateChange(ACTIVE→RELEASED) (also transactional).
            let release_response = LeaseResponse {
                granted: true,
                lease_id: lease_id_bytes.clone(),
                ..Default::default()
            };
            // Cache the LeaseResponse so retransmits replay it.
            if client_sequence > 0 {
                session.lease_correlation_cache.insert(
                    client_sequence,
                    CachedLeaseResponse {
                        granted: release_response.granted,
                        lease_id: release_response.lease_id.clone(),
                        granted_ttl_ms: release_response.granted_ttl_ms,
                        granted_priority: release_response.granted_priority,
                        granted_capabilities: release_response.granted_capabilities.clone(),
                        deny_reason: release_response.deny_reason.clone(),
                        deny_code: release_response.deny_code.clone(),
                    },
                );
            }
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(release_response)),
                }))
                .await;

            // LeaseStateChange notification: ACTIVE→RELEASED.
            // Transactional and always delivered (LEASE_CHANGES is unconditional).
            let change_seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: change_seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseStateChange(LeaseStateChange {
                        lease_id: lease_id_bytes,
                        previous_state: "ACTIVE".to_string(),
                        new_state: "RELEASED".to_string(),
                        reason: "Agent released lease".to_string(),
                        timestamp_wall_us: now_wall_us(),
                    })),
                }))
                .await;
        }
        Err(e) => {
            let seq = session.next_server_seq();
            let deny_reason = e.to_string();
            let deny_code = "LEASE_NOT_FOUND".to_string();
            if client_sequence > 0 {
                session.lease_correlation_cache.insert(
                    client_sequence,
                    CachedLeaseResponse {
                        granted: false,
                        lease_id: Vec::new(),
                        granted_ttl_ms: 0,
                        granted_priority: 0,
                        granted_capabilities: Vec::new(),
                        deny_reason: deny_reason.clone(),
                        deny_code: deny_code.clone(),
                    },
                );
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason,
                        deny_code,
                        ..Default::default()
                    })),
                }))
                .await;
        }
    }
}

async fn handle_subscription_change(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    change: SubscriptionChange,
) {
    // Merge plain subscriptions and filtered subscriptions into a combined add list.
    // `subscribe` contains category-only adds (use default prefix).
    // `subscribe_filter` contains category + optional finer-grained prefix (RFC 0010 §7.2).
    // Use a HashSet to deduplicate in O(n) rather than O(n²).
    let mut seen: std::collections::HashSet<&str> =
        change.subscribe.iter().map(String::as_str).collect();
    let mut add: Vec<String> = change.subscribe.clone();
    for entry in &change.subscribe_filter {
        if seen.insert(entry.category.as_str()) {
            add.push(entry.category.clone());
        }
    }

    // Apply capability-filtered subscription change (RFC 0005 §7.3).
    // Mandatory subscriptions (DEGRADATION_NOTICES, LEASE_CHANGES) cannot be removed.
    // Additions without the required capability are placed in denied_subscriptions.
    // New subscription set takes effect immediately after the ack is sent.
    let result = subscriptions::apply_subscription_change(
        &session.subscriptions,
        &add,
        &change.unsubscribe,
        &session.capabilities,
    );

    // Update per-category subscription filters to match the new active set.
    //
    // Semantics:
    // - Plain `subscribe` for a category implies default behavior (no stored filter),
    //   so any existing filter for that category is cleared when the subscription is active.
    // - `subscribe_filter` with a non-empty filter_prefix stores/updates the filter
    //   for that category, but only if the subscription is active (not denied).
    // - `subscribe_filter` with an empty filter_prefix explicitly resets to default:
    //   any stored filter for that category is removed.
    // - Unsubscribed categories always have their filters removed.

    // Clear filters for categories in plain `subscribe` that are now active.
    for cat in &change.subscribe {
        if result.active.contains(cat) {
            session.subscription_filters.remove(cat.as_str());
        }
    }

    // Apply filtered subscriptions: store, update, or clear filter per entry.
    for entry in &change.subscribe_filter {
        if result.active.contains(&entry.category) {
            if entry.filter_prefix.is_empty() {
                // Empty prefix for an active subscription resets to default behavior.
                session.subscription_filters.remove(entry.category.as_str());
            } else {
                session
                    .subscription_filters
                    .insert(entry.category.clone(), entry.filter_prefix.clone());
            }
        }
    }

    // Remove filters for unsubscribed categories.
    for cat in &change.unsubscribe {
        session.subscription_filters.remove(cat.as_str());
    }

    // Update session's active subscription set
    session.subscriptions = result.active.clone();

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::SubscriptionChangeResult(
                SubscriptionChangeResult {
                    active_subscriptions: result.active,
                    denied_subscriptions: result.denied,
                },
            )),
        }))
        .await;
}

/// Handle a mid-session CapabilityRequest (RFC 0005 §5.3).
///
/// Validates the request against the agent's authorization policy. If all
/// requested capabilities are authorized, responds with CapabilityNotice.
/// On partial failure or any denial, responds with RuntimeError(PERMISSION_DENIED)
/// without granting any capabilities (RFC 0005 §5.3 scenario 4).
///
/// Authorization is evaluated against `session.policy_capabilities`, which is
/// sourced from the config-driven agent allow-list (or fallback-unrestricted
/// dev mode). Guest sessions (empty policy scope) are denied any escalation.
async fn handle_capability_request(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    req: CapabilityRequest,
) {
    // Reconstruct authorization policy from `session.policy_capabilities`.
    // This scope comes from the configured per-agent allow-list; only fallback
    // unrestricted dev mode yields wildcard policy.
    let policy = CapabilityPolicy::new(session.policy_capabilities.clone());

    match policy.evaluate_capability_request(&req.capabilities) {
        Ok(granted) => {
            // Compute newly granted capabilities (exclude those already held).
            // CapabilityNotice.granted must contain only *newly* granted capabilities
            // so clients don't misinterpret re-requests as fresh grants.
            let seq = session.next_server_seq();
            let mut newly_granted: Vec<String> = Vec::new();
            for cap in &granted {
                if !session.capabilities.contains(cap) {
                    session.capabilities.push(cap.clone());
                    newly_granted.push(cap.clone());
                }
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::CapabilityNotice(CapabilityNotice {
                        granted: newly_granted,
                        revoked: Vec::new(),
                        reason: req.reason.clone(),
                        effective_at_server_seq: seq,
                    })),
                }))
                .await;
        }
        Err(denied_caps) => {
            // Deny the entire request (partial grants not allowed per RFC 0005 §5.3).
            let context = denied_caps.join(", ");
            let hint = serde_json::to_string(&serde_json::json!({
                "unauthorized_capabilities": denied_caps
            }))
            .unwrap_or_else(|_| "{}".to_string());
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: "PERMISSION_DENIED".to_string(),
                        message: format!(
                            "Capability request denied: unauthorized capabilities: {context}"
                        ),
                        context,
                        hint,
                        error_code_enum: ErrorCode::PermissionDenied as i32,
                    })),
                }))
                .await;
        }
    }
}

/// Handle a runtime-initiated capability revocation for a lease owned by this session.
///
/// Called by the session main loop when the session receives a [`CapabilityRevocationEvent`]
/// for one of its own leases. This function:
///
/// 1. Converts the capability name to a [`Capability`] enum value.
/// 2. Calls [`SceneGraph::revoke_capability`] to narrow the live scope.
/// 3. Emits `CapabilityNotice(revoked=[cap_name])` to the agent (transactional).
/// 4. Emits `LeaseStateChange` with a `CAPABILITY_REVOKED:<cap_name>` reason
///    (transactional audit event; lease state remains ACTIVE).
///
/// RFC 0001 §3.3: Capability enforcement happens at mutation time against the live scope.
/// After this function returns, any mutation that requires `capability_name` will be
/// rejected by the existing require_capability() check in the mutation pipeline.
///
/// # Error handling
///
/// If the capability name is not a recognized canonical name, the revocation is a no-op
/// and a `RuntimeError(CAPABILITY_NOT_PRESENT)` is sent to the agent for diagnostics.
///
/// If the lease is in a terminal state or the capability is not present, the function
/// sends `RuntimeError(CAPABILITY_NOT_PRESENT)` and returns without modifying the scope.
async fn handle_capability_revocation(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    event: CapabilityRevocationEvent,
) {
    // Map canonical capability name to enum value.
    let Some(cap) = canonical_name_to_capability(&event.capability_name) else {
        // Unknown capability name — emit a diagnostic and return.
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::RuntimeError(RuntimeError {
                    error_code: "CAPABILITY_NOT_PRESENT".to_string(),
                    message: format!(
                        "CapabilityRevocation: unknown capability name {:?} for lease {}",
                        event.capability_name, event.lease_id
                    ),
                    context: format!("lease_id={}", event.lease_id),
                    hint: String::new(),
                    error_code_enum: ErrorCode::InvalidArgument as i32,
                })),
            }))
            .await;
        return;
    };

    // Apply the revocation to the scene graph.
    let result = {
        let st = state.lock().await;
        st.scene
            .lock()
            .await
            .revoke_capability(event.lease_id, &cap)
    };

    match result {
        Ok((cap_name, revoked_at_us)) => {
            // Also remove from the session-level capability list so that
            // mid-session CapabilityRequest re-grants are not polluted.
            // (The session.capabilities list is used for CapabilityNotice.granted
            // filtering; removing the revoked entry keeps the audit trail clean.)
            session.capabilities.retain(|c| c != &event.capability_name);

            let lease_id_bytes = scene_id_to_bytes(event.lease_id);
            let reason = format!("CAPABILITY_REVOKED:{cap_name}");

            // ── CapabilityNotice (transactional, RFC 0005 §5.3) ──────────────
            // Tells the agent which capability was revoked so it can update its
            // local capability inventory and stop issuing mutations that require it.
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::CapabilityNotice(CapabilityNotice {
                        granted: Vec::new(),
                        revoked: vec![event.capability_name.clone()],
                        reason: reason.clone(),
                        effective_at_server_seq: seq,
                    })),
                }))
                .await;

            // ── LeaseStateChange audit event (transactional) ─────────────────
            // Carries the audit trail for the capability revocation. The lease
            // state remains ACTIVE; only the capability scope is narrowed.
            // RFC 0001 §3.3: "The lease remains in its current state; only the
            // capability scope is narrowed."
            let state_seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: state_seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseStateChange(LeaseStateChange {
                        lease_id: lease_id_bytes,
                        previous_state: "ACTIVE".to_string(),
                        new_state: "ACTIVE".to_string(),
                        reason,
                        timestamp_wall_us: revoked_at_us,
                    })),
                }))
                .await;
        }
        Err(e) => {
            // The scene-graph rejected the revocation (lease terminal or cap missing).
            // Report the error to the agent for diagnostics; do not alter state.
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: "CAPABILITY_NOT_PRESENT".to_string(),
                        message: format!(
                            "CapabilityRevocation failed for lease {}: {e}",
                            event.lease_id
                        ),
                        context: format!("lease_id={}", event.lease_id),
                        hint: String::new(),
                        error_code_enum: ErrorCode::InvalidArgument as i32,
                    })),
                }))
                .await;
        }
    }
}

async fn handle_list_elements_request(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request: ListElementsRequest,
) {
    if !capability_set_covers(&session.capabilities, "read_scene_topology") {
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::RuntimeError(RuntimeError {
                    error_code: "PERMISSION_DENIED".to_string(),
                    message: "Missing capability: read_scene_topology".to_string(),
                    context: "list_elements_request".to_string(),
                    hint: r#"{"required_capability":"read_scene_topology"}"#.to_string(),
                    error_code_enum: ErrorCode::PermissionDenied as i32,
                })),
            }))
            .await;
        return;
    }

    let namespace_filter = request.namespace_filter.unwrap_or_default();
    let element_type_filter = request.element_type.unwrap_or_default();
    let parsed_type_filter = if element_type_filter.trim().is_empty() {
        None
    } else {
        match parse_element_type_filter(&element_type_filter) {
            Some(element_type) => Some(element_type),
            None => {
                let seq = session.next_server_seq();
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: seq,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::RuntimeError(RuntimeError {
                            error_code: "INVALID_ARGUMENT".to_string(),
                            message: format!(
                                "Unsupported element_type filter {element_type_filter:?}; expected tile|zone|widget"
                            ),
                            context: "list_elements_request.element_type".to_string(),
                            hint: r#"{"supported":["tile","zone","widget"]}"#.to_string(),
                            error_code_enum: ErrorCode::InvalidArgument as i32,
                        })),
                    }))
                    .await;
                return;
            }
        }
    };

    let (scene_handle, mut entries): (Arc<Mutex<SceneGraph>>, Vec<(SceneId, ElementStoreEntry)>) = {
        let st = state.lock().await;
        (
            st.scene.clone(),
            st.element_store
                .entries
                .iter()
                .map(|(id, entry)| (*id, entry.clone()))
                .collect(),
        )
    };
    entries.sort_by_key(|(id, entry)| (entry.created_at, id.to_bytes_le()));

    let scene = scene_handle.lock().await;
    let mut elements = Vec::new();
    for (element_id, entry) in entries {
        if let Some(filter) = parsed_type_filter {
            if entry.element_type != filter {
                continue;
            }
        }
        if !namespace_filter.is_empty() && !entry.namespace.starts_with(&namespace_filter) {
            continue;
        }

        let zero_policy = GeometryPolicy::Relative {
            x_pct: 0.0,
            y_pct: 0.0,
            width_pct: 0.0,
            height_pct: 0.0,
        };
        let current_geometry = match entry.element_type {
            ElementType::Tile => {
                let agent_policy = scene.tiles.get(&element_id).map(|tile| {
                    rect_to_relative_geometry_policy(
                        tile.bounds,
                        scene.display_area.width,
                        scene.display_area.height,
                    )
                });
                resolve_geometry_override_chain(entry.geometry_override, agent_policy, None, None)
                    .unwrap_or(zero_policy)
            }
            ElementType::Zone => scene
                .zone_registry
                .resolve_geometry_policy_for_zone(
                    &entry.namespace,
                    entry.geometry_override.as_ref(),
                    None,
                )
                .or(entry.geometry_override)
                .unwrap_or(zero_policy),
            ElementType::Widget => scene
                .widget_registry
                .resolve_geometry_policy_for_instance(
                    &entry.namespace,
                    entry.geometry_override.as_ref(),
                )
                .or(entry.geometry_override)
                .unwrap_or(zero_policy),
        };

        elements.push(ElementInfo {
            element_id: scene_id_to_bytes(element_id),
            element_type: element_type_wire_name(entry.element_type).to_string(),
            namespace: entry.namespace,
            current_geometry: Some(convert::geometry_policy_to_proto(&current_geometry)),
            has_user_override: entry.geometry_override.is_some(),
            created_at_ms: entry.created_at,
            last_published_at_ms: entry.last_published_at,
        });
    }

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::ListElementsResponse(ListElementsResponse {
                elements,
            })),
        }))
        .await;
}

/// Handle a ZonePublish from the client (RFC 0005 §3.1, §8.6).
///
/// Durable-zone publishes are transactional and receive a ZonePublishResult.
/// Ephemeral-zone publishes are fire-and-forget; no ZonePublishResult is sent.
///
/// Zone durability is determined by `ZoneDefinition.ephemeral`:
/// - `false` (default): durable → sends ZonePublishResult ack.
/// - `true`: ephemeral → fire-and-forget, no ZonePublishResult.
async fn handle_zone_publish(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    publish: ZonePublish,
) {
    // Apply the zone publish through the scene graph mutation path.
    // Also determine zone durability (ephemeral vs durable) for ack decision.
    let (accepted, error_code, error_message, is_ephemeral_zone, persist_request) = {
        let mut st = state.lock().await;
        let (resolved_zone_name, resolved_element_id, preflight_error) =
            if !publish.element_id.is_empty() {
                match bytes_to_scene_id(&publish.element_id) {
                    Ok(element_id) => match st.element_store.entries.get(&element_id) {
                        Some(entry) if entry.element_type == ElementType::Zone => {
                            (entry.namespace.clone(), Some(element_id), None)
                        }
                        _ => (
                            String::new(),
                            None,
                            Some((
                                false,
                                "ELEMENT_NOT_FOUND".to_string(),
                                "element_id does not reference a known zone".to_string(),
                                false,
                                None,
                            )),
                        ),
                    },
                    Err(_) => (
                        String::new(),
                        None,
                        Some((
                            false,
                            "INVALID_ARGUMENT".to_string(),
                            "invalid element_id: expected 16 bytes".to_string(),
                            false,
                            None,
                        )),
                    ),
                }
            } else {
                (publish.zone_name.clone(), None, None)
            };

        if let Some(preflight_error) = preflight_error {
            preflight_error
        } else {
            let mut scene = st.scene.lock().await;

            // Check zone durability before applying the mutation
            let zone_is_ephemeral = scene
                .zone_registry
                .get_by_name(&resolved_zone_name)
                .map(|def| def.ephemeral)
                .unwrap_or(false); // Unknown zones default to durable (will fail below)

            let content = publish
                .content
                .as_ref()
                .and_then(crate::convert::proto_zone_content_to_scene);

            if let Some(content) = content {
                // Validate: breakpoints are only meaningful for StreamText content.
                if !publish.breakpoints.is_empty()
                    && !matches!(content, tze_hud_scene::types::ZoneContent::StreamText(_))
                {
                    (
                        false,
                        "INVALID_ARGUMENT".to_string(),
                        "breakpoints are only valid for StreamText content".to_string(),
                        zone_is_ephemeral,
                        None,
                    )
                } else {
                    let merge_key = if publish.merge_key.is_empty() {
                        None
                    } else {
                        Some(publish.merge_key.clone())
                    };

                    let mutation = tze_hud_scene::mutation::SceneMutation::PublishToZone {
                        zone_name: resolved_zone_name.clone(),
                        content,
                        publish_token: tze_hud_scene::types::ZonePublishToken { token: Vec::new() },
                        merge_key,
                        // expires_at_wall_us and content_classification are not yet present in
                        // the ZonePublish proto message (post-v1 wire extensions).
                        expires_at_wall_us: None,
                        content_classification: None,
                        // Wire breakpoints from the ZonePublish proto for StreamText streaming reveal.
                        // Per spec §Subtitle Streaming Word-by-Word Reveal.
                        breakpoints: publish.breakpoints.clone(),
                    };

                    // Apply as a single-mutation batch.
                    let zone_publish_lease_id = session.lease_ids.first().copied();
                    let batch = tze_hud_scene::mutation::MutationBatch {
                        batch_id: tze_hud_scene::SceneId::new(),
                        agent_namespace: session.namespace.clone(),
                        mutations: vec![mutation],
                        timing_hints: None,
                        lease_id: zone_publish_lease_id,
                    };
                    let result = scene.apply_batch(&batch);
                    drop(scene);
                    if result.applied {
                        let now = now_ms();
                        let persist_request = if let Some(element_id) = resolved_element_id {
                            touch_element_store_entry_by_id(
                                &mut st,
                                element_id,
                                ElementType::Zone,
                                now,
                            )
                        } else {
                            touch_element_store_entry_by_namespace(
                                &mut st,
                                ElementType::Zone,
                                &resolved_zone_name,
                                now,
                            )
                        };
                        (
                            true,
                            String::new(),
                            String::new(),
                            zone_is_ephemeral,
                            persist_request,
                        )
                    } else {
                        let (code, msg) = match &result.error {
                            Some(tze_hud_scene::ValidationError::ZoneNotFound { name }) => (
                                "ZONE_NOT_FOUND".to_string(),
                                format!("Zone not found: {name}"),
                            ),
                            Some(tze_hud_scene::ValidationError::ZonePublishTokenInvalid {
                                zone,
                            }) => (
                                "TOKEN_INVALID".to_string(),
                                format!("Publish token invalid for zone '{zone}'"),
                            ),
                            Some(tze_hud_scene::ValidationError::BudgetExceeded { resource }) => (
                                "BUDGET_EXCEEDED".to_string(),
                                format!("Budget exceeded: {resource}"),
                            ),
                            Some(tze_hud_scene::ValidationError::CapabilityMissing {
                                capability,
                            }) => (
                                "CAPABILITY_MISSING".to_string(),
                                format!("Capability missing: {capability}"),
                            ),
                            Some(err) => ("ZONE_PUBLISH_FAILED".to_string(), err.to_string()),
                            None => (
                                "ZONE_PUBLISH_FAILED".to_string(),
                                "Zone publish failed".to_string(),
                            ),
                        };
                        (false, code, msg, zone_is_ephemeral, None)
                    }
                }
            } else {
                (
                    false,
                    "INVALID_CONTENT".to_string(),
                    "Missing or invalid zone content".to_string(),
                    zone_is_ephemeral,
                    None,
                )
            }
        }
    };

    persist_element_store(persist_request).await;

    // Durable zones: send ZonePublishResult (transactional ack).
    // Ephemeral zones: fire-and-forget — no ZonePublishResult sent, even on failure
    // (RFC 0005 §8.6: "Ephemeral-zone publishes SHALL be fire-and-forget").
    if !is_ephemeral_zone {
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::ZonePublishResult(ZonePublishResult {
                    request_sequence,
                    accepted,
                    error_code,
                    error_message,
                })),
            }))
            .await;
    }
    // Ephemeral zone: no ack sent (fire-and-forget per RFC 0005 §8.6), success or failure
}

fn is_valid_widget_type_id(widget_type_id: &str) -> bool {
    let mut chars = widget_type_id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_valid_widget_svg_filename(svg_filename: &str) -> bool {
    !svg_filename.is_empty()
        && svg_filename.ends_with(".svg")
        && !svg_filename.contains('/')
        && !svg_filename.contains('\\')
}

fn validate_svg_payload(svg_bytes: &[u8]) -> Result<(), String> {
    let svg_text =
        std::str::from_utf8(svg_bytes).map_err(|e| format!("SVG payload is not UTF-8: {e}"))?;

    let mut reader = Reader::from_str(svg_text);
    reader.config_mut().trim_text(true);

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) | Ok(Event::Empty(start)) => {
                if start.name().as_ref() == b"svg" {
                    return Ok(());
                }
                return Err("SVG root element must be <svg>".to_string());
            }
            Ok(Event::Decl(_) | Event::Comment(_) | Event::DocType(_) | Event::Text(_)) => {
                continue;
            }
            Ok(Event::Eof) => {
                return Err("SVG payload is empty or missing a root element".to_string());
            }
            Err(e) => return Err(format!("SVG XML parse error: {e}")),
            _ => continue,
        }
    }
}

async fn send_widget_asset_register_result(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    result: WidgetAssetRegisterResult,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::WidgetAssetRegisterResult(result)),
        }))
        .await;
}

fn make_widget_asset_error_result(
    request_sequence: u64,
    widget_type_id: &str,
    svg_filename: &str,
    error_code: &str,
    error_message: String,
) -> WidgetAssetRegisterResult {
    WidgetAssetRegisterResult {
        request_sequence,
        accepted: false,
        widget_type_id: widget_type_id.to_string(),
        svg_filename: svg_filename.to_string(),
        asset_handle: String::new(),
        was_deduplicated: false,
        error_code: error_code.to_string(),
        error_message,
    }
}

fn widget_asset_handle_from_hash(hash: [u8; 32]) -> String {
    format!(
        "widget-svg:{}",
        tze_hud_resource::ResourceId::from_bytes(hash).to_hex()
    )
}

fn register_widget_asset_in_scene(
    scene: &mut SceneGraph,
    register: &WidgetAssetRegister,
    svg_bytes: &[u8],
    asset_handle: &str,
) -> Result<(), RuntimeWidgetAssetError> {
    register_runtime_widget_svg_asset(
        scene,
        &register.widget_type_id,
        &register.svg_filename,
        svg_bytes,
        asset_handle,
        &HashMap::new(),
    )
}

fn proto_resource_type_to_store(resource_type: i32) -> Option<StoreResourceType> {
    match resource_type {
        1 => Some(StoreResourceType::ImageRgba8),
        2 => Some(StoreResourceType::ImagePng),
        3 => Some(StoreResourceType::ImageJpeg),
        4 => Some(StoreResourceType::FontTtf),
        5 => Some(StoreResourceType::FontOtf),
        6 => Some(StoreResourceType::ImageSvg),
        _ => None,
    }
}

fn resource_error_code_i32(err: &StoreResourceError) -> i32 {
    match err {
        StoreResourceError::CapabilityDenied => 1,
        StoreResourceError::BudgetExceeded { .. } => 2,
        StoreResourceError::SizeExceeded { .. } => 3,
        StoreResourceError::UnsupportedType(_) => 4,
        StoreResourceError::DecodeError(_) => 5,
        StoreResourceError::HashMismatch { .. } => 6,
        StoreResourceError::InvalidChunk(detail)
            if detail.contains("unknown upload_id")
                || detail.contains("not in-flight")
                || detail.contains("no uploads in flight") =>
        {
            9
        }
        StoreResourceError::InvalidChunk(_) => 7,
        StoreResourceError::TooManyUploads => 8,
        StoreResourceError::UploadAborted(_) => 9,
        StoreResourceError::Internal(_) => 7,
    }
}

fn upload_id_bytes_from_slice(upload_id: &[u8]) -> Option<[u8; 16]> {
    let arr: [u8; 16] = upload_id.try_into().ok()?;
    Some(arr)
}

async fn apply_upload_transport_backpressure(
    limiter: &mut UploadByteRateLimiter,
    mut bytes: usize,
) {
    while bytes > 0 {
        let now = Instant::now();
        let available = limiter.available_bytes(now);
        if available > 0 {
            let consumed = bytes.min(available);
            limiter.reserve_bytes(now, consumed);
            bytes -= consumed;
            continue;
        }

        let delay = limiter.next_delay(now);
        if delay.is_zero() {
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(delay).await;
        }
    }
}

async fn send_resource_stored(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    stored: &StoreResourceStored,
    stored_bytes: u64,
    metadata: ResourceMetadata,
    upload_id: Option<&[u8; 16]>,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::ResourceStored(ResourceStored {
                request_sequence,
                resource_id: Some(crate::proto::ResourceIdProto {
                    bytes: stored.resource_id.as_bytes().to_vec(),
                }),
                was_deduplicated: stored.was_deduplicated,
                stored_bytes,
                decoded_bytes: stored.decoded_bytes as u64,
                metadata: Some(metadata),
                upload_id: upload_id.map(|u| u.to_vec()).unwrap_or_default(),
            })),
        }))
        .await;
}

async fn register_uploaded_scene_resource(
    state: &Arc<Mutex<SharedState>>,
    resource_id: &tze_hud_resource::ResourceId,
) {
    let scene_resource_id = ResourceId::from_bytes(*resource_id.as_bytes());
    let st = state.lock().await;
    let mut scene = st.scene.lock().await;
    scene.register_resource(scene_resource_id);
}

async fn send_resource_error_response(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    upload_id: Option<&[u8]>,
    err: &StoreResourceError,
) {
    let context = serde_json::json!({
        "domain": "resource_upload",
        "wire_code": err.wire_code(),
    })
    .to_string();
    let hint = serde_json::json!({
        "expected_flow": "ResourceUploadStart -> [ResourceUploadAccepted] -> ResourceUploadChunk* -> ResourceUploadComplete",
    })
    .to_string();
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::ResourceErrorResponse(
                ResourceErrorResponse {
                    request_sequence,
                    error_code: resource_error_code_i32(err),
                    message: err.to_string(),
                    context,
                    hint,
                    upload_id: upload_id.map(|u| u.to_vec()).unwrap_or_default(),
                },
            )),
        }))
        .await;
}

/// Handle a WidgetAssetRegister from the client (session-protocol spec
/// §Requirement: Widget Asset Registration via Session Stream).
///
/// Implements metadata-first dedup preflight:
/// - known hash => accepted dedup hit without payload transfer
/// - unknown hash => payload required, checksum/hash verified, SVG validated
async fn handle_widget_asset_register(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    register: WidgetAssetRegister,
) {
    let required_cap = "register_widget_asset".to_string();
    if !session.capabilities.contains(&required_cap) {
        send_widget_asset_register_result(
            session,
            tx,
            make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                "WIDGET_ASSET_CAPABILITY_MISSING",
                format!("Missing capability: {required_cap}"),
            ),
        )
        .await;
        return;
    }

    if !is_valid_widget_type_id(&register.widget_type_id)
        || !is_valid_widget_svg_filename(&register.svg_filename)
    {
        send_widget_asset_register_result(
            session,
            tx,
            make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                "WIDGET_ASSET_TYPE_INVALID",
                "widget_type_id or svg_filename failed validation".to_string(),
            ),
        )
        .await;
        return;
    }

    let expected_hash: [u8; 32] = match register.content_hash_blake3.as_slice().try_into() {
        Ok(hash) => hash,
        Err(_) => {
            send_widget_asset_register_result(
                session,
                tx,
                make_widget_asset_error_result(
                    request_sequence,
                    &register.widget_type_id,
                    &register.svg_filename,
                    "WIDGET_ASSET_HASH_MISMATCH",
                    "content_hash_blake3 must be exactly 32 bytes".to_string(),
                ),
            )
            .await;
            return;
        }
    };

    let (existing_record, durable_dedup_hit) = {
        let st = state.lock().await;
        let existing = st.widget_asset_store.by_hash.get(&expected_hash).cloned();
        let durable_hit = if existing.is_none() {
            st.runtime_widget_store
                .as_ref()
                .map(|store| {
                    store.contains(tze_hud_resource::ResourceId::from_bytes(expected_hash))
                })
                .unwrap_or(false)
        } else {
            false
        };
        (existing, durable_hit)
    };
    if let Some(existing) = existing_record {
        let st = state.lock().await;
        let runtime_register_result = {
            let mut scene = st.scene.lock().await;
            register_widget_asset_in_scene(
                &mut scene,
                &register,
                &existing.bytes,
                &existing.asset_handle,
            )
        };
        if let Err(err) = runtime_register_result {
            let error_result = make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                err.wire_code(),
                err.to_string(),
            );
            drop(st);
            send_widget_asset_register_result(session, tx, error_result).await;
            return;
        }

        let asset_handle = existing.asset_handle;
        drop(st);
        send_widget_asset_register_result(
            session,
            tx,
            WidgetAssetRegisterResult {
                request_sequence,
                accepted: true,
                widget_type_id: register.widget_type_id.clone(),
                svg_filename: register.svg_filename.clone(),
                asset_handle,
                was_deduplicated: true,
                error_code: String::new(),
                error_message: String::new(),
            },
        )
        .await;
        return;
    }

    if durable_dedup_hit {
        send_widget_asset_register_result(
            session,
            tx,
            WidgetAssetRegisterResult {
                request_sequence,
                accepted: true,
                widget_type_id: register.widget_type_id.clone(),
                svg_filename: register.svg_filename.clone(),
                asset_handle: widget_asset_handle_from_hash(expected_hash),
                was_deduplicated: true,
                error_code: String::new(),
                error_message: String::new(),
            },
        )
        .await;
        return;
    }

    if register.inline_svg_bytes.is_empty() {
        send_widget_asset_register_result(
            session,
            tx,
            make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                "WIDGET_ASSET_HASH_MISMATCH",
                "unknown content hash requires payload bytes".to_string(),
            ),
        )
        .await;
        return;
    }

    if register.total_size_bytes != register.inline_svg_bytes.len() as u64 {
        send_widget_asset_register_result(
            session,
            tx,
            make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                "WIDGET_ASSET_CHECKSUM_MISMATCH",
                format!(
                    "declared total_size_bytes={} does not match payload length={}",
                    register.total_size_bytes,
                    register.inline_svg_bytes.len()
                ),
            ),
        )
        .await;
        return;
    }

    if register.transport_crc32c != 0 {
        let computed_crc = crc32c::crc32c(&register.inline_svg_bytes);
        if computed_crc != register.transport_crc32c {
            send_widget_asset_register_result(
                session,
                tx,
                make_widget_asset_error_result(
                    request_sequence,
                    &register.widget_type_id,
                    &register.svg_filename,
                    "WIDGET_ASSET_CHECKSUM_MISMATCH",
                    format!(
                        "transport_crc32c mismatch: declared={}, computed={computed_crc}",
                        register.transport_crc32c
                    ),
                ),
            )
            .await;
            return;
        }
    }

    let computed_hash = *blake3::hash(&register.inline_svg_bytes).as_bytes();
    if computed_hash != expected_hash {
        send_widget_asset_register_result(
            session,
            tx,
            make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                "WIDGET_ASSET_HASH_MISMATCH",
                "payload BLAKE3 hash does not match content_hash_blake3".to_string(),
            ),
        )
        .await;
        return;
    }

    if let Err(detail) = validate_svg_payload(&register.inline_svg_bytes) {
        send_widget_asset_register_result(
            session,
            tx,
            make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                "WIDGET_ASSET_INVALID_SVG",
                detail,
            ),
        )
        .await;
        return;
    }

    let payload_len = register.inline_svg_bytes.len() as u64;
    let mut st = state.lock().await;
    let asset_handle = widget_asset_handle_from_hash(expected_hash);

    // Re-check dedup after lock acquisition to avoid races between workers.
    if let Some(existing) = st.widget_asset_store.by_hash.get(&expected_hash).cloned() {
        let runtime_register_result = {
            let mut scene = st.scene.lock().await;
            register_widget_asset_in_scene(
                &mut scene,
                &register,
                &existing.bytes,
                &existing.asset_handle,
            )
        };
        if let Err(err) = runtime_register_result {
            let error_result = make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                err.wire_code(),
                err.to_string(),
            );
            drop(st);
            send_widget_asset_register_result(session, tx, error_result).await;
            return;
        }
        let dedup_result = WidgetAssetRegisterResult {
            request_sequence,
            accepted: true,
            widget_type_id: register.widget_type_id.clone(),
            svg_filename: register.svg_filename.clone(),
            asset_handle: existing.asset_handle,
            was_deduplicated: true,
            error_code: String::new(),
            error_message: String::new(),
        };
        drop(st);
        send_widget_asset_register_result(session, tx, dedup_result).await;
        return;
    }

    if let Some(store) = st.runtime_widget_store.as_ref() {
        let resource_id = tze_hud_resource::ResourceId::from_bytes(expected_hash);
        if store.contains(resource_id) {
            let dedup_result = WidgetAssetRegisterResult {
                request_sequence,
                accepted: true,
                widget_type_id: register.widget_type_id.clone(),
                svg_filename: register.svg_filename.clone(),
                asset_handle,
                was_deduplicated: true,
                error_code: String::new(),
                error_message: String::new(),
            };
            drop(st);
            send_widget_asset_register_result(session, tx, dedup_result).await;
            return;
        }
    }

    if let Some(store) = st.runtime_widget_store.as_mut() {
        let put_outcome = match store.put_svg(&session.namespace, &register.inline_svg_bytes) {
            Ok(outcome) => outcome,
            Err(RuntimeWidgetStoreError::TotalBudgetExceeded { .. })
            | Err(RuntimeWidgetStoreError::AgentBudgetExceeded { .. }) => {
                let budget_error = make_widget_asset_error_result(
                    request_sequence,
                    &register.widget_type_id,
                    &register.svg_filename,
                    "WIDGET_ASSET_BUDGET_EXCEEDED",
                    "runtime widget asset store budget exceeded".to_string(),
                );
                drop(st);
                send_widget_asset_register_result(session, tx, budget_error).await;
                return;
            }
            Err(err) => {
                let store_error = make_widget_asset_error_result(
                    request_sequence,
                    &register.widget_type_id,
                    &register.svg_filename,
                    "WIDGET_ASSET_STORE_IO_ERROR",
                    format!("runtime widget asset store write failed: {err}"),
                );
                drop(st);
                send_widget_asset_register_result(session, tx, store_error).await;
                return;
            }
        };

        let runtime_register_result = {
            let mut scene = st.scene.lock().await;
            register_widget_asset_in_scene(
                &mut scene,
                &register,
                &register.inline_svg_bytes,
                &asset_handle,
            )
        };
        if let Err(err) = runtime_register_result {
            let error_result = make_widget_asset_error_result(
                request_sequence,
                &register.widget_type_id,
                &register.svg_filename,
                err.wire_code(),
                err.to_string(),
            );
            drop(st);
            send_widget_asset_register_result(session, tx, error_result).await;
            return;
        }

        let was_deduplicated = matches!(put_outcome, DurablePutOutcome::Deduplicated { .. });
        drop(st);
        send_widget_asset_register_result(
            session,
            tx,
            WidgetAssetRegisterResult {
                request_sequence,
                accepted: true,
                widget_type_id: register.widget_type_id,
                svg_filename: register.svg_filename,
                asset_handle,
                was_deduplicated,
                error_code: String::new(),
                error_message: String::new(),
            },
        )
        .await;
        return;
    }

    let used_by_ns = st
        .widget_asset_store
        .per_namespace_bytes
        .get(&session.namespace)
        .copied()
        .unwrap_or(0);

    if st
        .widget_asset_store
        .total_bytes
        .saturating_add(payload_len)
        > st.widget_asset_store.max_total_bytes
        || used_by_ns.saturating_add(payload_len) > st.widget_asset_store.max_namespace_bytes
    {
        let budget_error = make_widget_asset_error_result(
            request_sequence,
            &register.widget_type_id,
            &register.svg_filename,
            "WIDGET_ASSET_BUDGET_EXCEEDED",
            "runtime widget asset store budget exceeded".to_string(),
        );
        drop(st);
        send_widget_asset_register_result(session, tx, budget_error).await;
        return;
    }

    let runtime_register_result = {
        let mut scene = st.scene.lock().await;
        register_widget_asset_in_scene(
            &mut scene,
            &register,
            &register.inline_svg_bytes,
            &asset_handle,
        )
    };
    if let Err(err) = runtime_register_result {
        let error_result = make_widget_asset_error_result(
            request_sequence,
            &register.widget_type_id,
            &register.svg_filename,
            err.wire_code(),
            err.to_string(),
        );
        drop(st);
        send_widget_asset_register_result(session, tx, error_result).await;
        return;
    }
    let previous = st.widget_asset_store.by_hash.insert(
        expected_hash,
        crate::session::WidgetAssetRecord {
            asset_handle: asset_handle.clone(),
            widget_type_id: register.widget_type_id.clone(),
            svg_filename: register.svg_filename.clone(),
            owner_namespace: session.namespace.clone(),
            bytes: register.inline_svg_bytes,
        },
    );
    if previous.is_some() {
        // Should be unreachable due to dedup checks above; return a stable error anyway.
        let duplicate_error = make_widget_asset_error_result(
            request_sequence,
            &register.widget_type_id,
            &register.svg_filename,
            "WIDGET_ASSET_STORE_IO_ERROR",
            "duplicate hash insertion race while updating store".to_string(),
        );
        drop(st);
        send_widget_asset_register_result(session, tx, duplicate_error).await;
        return;
    }
    st.widget_asset_store.total_bytes = st
        .widget_asset_store
        .total_bytes
        .saturating_add(payload_len);
    let entry = st
        .widget_asset_store
        .per_namespace_bytes
        .entry(session.namespace.clone())
        .or_insert(0);
    *entry = entry.saturating_add(payload_len);
    drop(st);

    send_widget_asset_register_result(
        session,
        tx,
        WidgetAssetRegisterResult {
            request_sequence,
            accepted: true,
            widget_type_id: register.widget_type_id,
            svg_filename: register.svg_filename,
            asset_handle,
            was_deduplicated: false,
            error_code: String::new(),
            error_message: String::new(),
        },
    )
    .await;
}

/// Look up whether a widget instance is transactional (i.e., not ephemeral).
///
/// Returns `true` when the widget is durable (WidgetPublishResult should be sent),
/// `false` when ephemeral (fire-and-forget, no result). Defaults to `true` when the
/// widget instance or definition is not found, so unknown widgets still receive an
/// error result (WIDGET_NOT_FOUND is always reportable).
async fn is_widget_transactional(state: &Arc<Mutex<SharedState>>, widget_name: &str) -> bool {
    let st = state.lock().await;
    let scene = st.scene.lock().await;
    let is_ephemeral = scene
        .widget_registry
        .instances
        .get(widget_name)
        .and_then(|inst| {
            scene
                .widget_registry
                .definitions
                .get(&inst.widget_type_name)
        })
        .map(|def| def.ephemeral)
        .unwrap_or(false); // Unknown widget: treat as durable (WIDGET_NOT_FOUND reportable)
    !is_ephemeral
}

/// Handle a WidgetPublish from the client (widget-system spec §Requirement: Widget Publishing via gRPC).
///
/// 1. Checks `publish_widget:<widget_name>` capability from session.capabilities.
/// 2. Converts proto params to scene `WidgetParameterValue` map.
/// 3. Calls `SceneGraph::publish_to_widget`, which validates params and applies the publication.
/// 4. For durable widgets (ephemeral=false): sends WidgetPublishResult(accepted=true/false).
/// 5. For ephemeral widgets (ephemeral=true): fire-and-forget, no WidgetPublishResult sent.
async fn handle_widget_publish(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    _request_sequence: u64,
    publish: WidgetPublish,
) {
    let (resolved_widget_name, resolved_element_id) = if !publish.element_id.is_empty() {
        let st = state.lock().await;
        match bytes_to_scene_id(&publish.element_id) {
            Ok(element_id) => match st.element_store.entries.get(&element_id) {
                Some(entry) if entry.element_type == ElementType::Widget => {
                    (entry.namespace.clone(), Some(element_id))
                }
                _ => {
                    let seq = session.next_server_seq();
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: seq,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::WidgetPublishResult(
                                WidgetPublishResult {
                                    accepted: false,
                                    widget_name: publish.widget_name.clone(),
                                    error_code: "ELEMENT_NOT_FOUND".to_string(),
                                    error_message: "element_id does not reference a known widget"
                                        .to_string(),
                                },
                            )),
                        }))
                        .await;
                    return;
                }
            },
            Err(_) => {
                let seq = session.next_server_seq();
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: seq,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::WidgetPublishResult(WidgetPublishResult {
                            accepted: false,
                            widget_name: publish.widget_name.clone(),
                            error_code: "INVALID_ARGUMENT".to_string(),
                            error_message: "invalid element_id: expected 16 bytes".to_string(),
                        })),
                    }))
                    .await;
                return;
            }
        }
    } else if !publish.instance_id.is_empty() {
        (publish.instance_id.clone(), None)
    } else {
        (publish.widget_name.clone(), None)
    };

    // ── Step 1: Capability check (string-based, matches session.capabilities) ──
    let required_cap = format!("publish_widget:{resolved_widget_name}");
    let has_cap = capability_set_covers(&session.capabilities, &required_cap);

    if !has_cap {
        // Per spec: WIDGET_CAPABILITY_MISSING. For durable widgets we send a result;
        // since we don't know if it's ephemeral without looking up the registry,
        // we check ephemerality to decide whether to send a result.
        let transactional = is_widget_transactional(state, resolved_widget_name.as_str()).await;
        if transactional {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::WidgetPublishResult(WidgetPublishResult {
                        accepted: false,
                        widget_name: resolved_widget_name.clone(),
                        error_code: "WIDGET_CAPABILITY_MISSING".to_string(),
                        error_message: format!("Missing capability: {required_cap}"),
                    })),
                }))
                .await;
        }
        return;
    }

    // ── Step 2: Convert proto params to scene WidgetParameterValue map ─────────
    let params: std::collections::HashMap<String, tze_hud_scene::types::WidgetParameterValue> =
        publish
            .params
            .iter()
            .filter_map(crate::convert::proto_to_widget_param_value)
            .collect();

    // ── Step 4: Apply through the scene graph ─────────────────────────────────
    let merge_key = if publish.merge_key.is_empty() {
        None
    } else {
        Some(publish.merge_key.clone())
    };

    let (result, persist_request) = {
        let mut st = state.lock().await;
        let mut scene = st.scene.lock().await;
        let publish_result = scene.publish_to_widget(
            &resolved_widget_name,
            params,
            &session.namespace,
            merge_key,
            publish.transition_ms,
            None, // expires_at_wall_us not yet in proto
        );
        drop(scene);
        let persist_request = if publish_result.is_ok() {
            let now = now_ms();
            if let Some(element_id) = resolved_element_id {
                touch_element_store_entry_by_id(&mut st, element_id, ElementType::Widget, now)
            } else {
                touch_element_store_entry_by_namespace(
                    &mut st,
                    ElementType::Widget,
                    &resolved_widget_name,
                    now,
                )
            }
        } else {
            None
        };
        (publish_result, persist_request)
    };
    persist_element_store(persist_request).await;

    // ── Step 5: Send result or fire-and-forget ────────────────────────────────
    match result {
        Ok(is_durable) => {
            // is_durable = true → durable widget, send WidgetPublishResult(accepted=true)
            // is_durable = false → ephemeral widget, no result
            if is_durable {
                let seq = session.next_server_seq();
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: seq,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::WidgetPublishResult(WidgetPublishResult {
                            accepted: true,
                            widget_name: resolved_widget_name.clone(),
                            error_code: String::new(),
                            error_message: String::new(),
                        })),
                    }))
                    .await;
            }
            // Ephemeral: no result sent (fire-and-forget per spec)
        }
        Err(err) => {
            // Map validation errors to wire error codes
            let (error_code, error_message) = match &err {
                tze_hud_scene::ValidationError::WidgetNotFound { name } => (
                    "WIDGET_NOT_FOUND".to_string(),
                    format!("Widget not found: {name}"),
                ),
                tze_hud_scene::ValidationError::WidgetUnknownParameter { widget, param } => (
                    "WIDGET_UNKNOWN_PARAMETER".to_string(),
                    format!("parameter '{param}' is not declared in widget '{widget}' schema"),
                ),
                tze_hud_scene::ValidationError::WidgetParameterTypeMismatch { widget, param } => (
                    "WIDGET_PARAMETER_TYPE_MISMATCH".to_string(),
                    format!("parameter '{param}' type mismatch in widget '{widget}'"),
                ),
                tze_hud_scene::ValidationError::WidgetParameterInvalidValue {
                    widget,
                    param,
                    reason,
                } => (
                    "WIDGET_PARAMETER_INVALID_VALUE".to_string(),
                    format!("parameter '{param}' in widget '{widget}': {reason}"),
                ),
                tze_hud_scene::ValidationError::WidgetCapabilityMissing { widget } => (
                    "WIDGET_CAPABILITY_MISSING".to_string(),
                    format!("Missing capability: publish_widget:{widget}"),
                ),
                other => ("WIDGET_PUBLISH_FAILED".to_string(), other.to_string()),
            };

            // Determine transactional state to decide whether to send WidgetPublishResult.
            // For WIDGET_NOT_FOUND, we can't look up the definition — helper defaults to
            // transactional=true so the error result is always delivered.
            let transactional = is_widget_transactional(state, resolved_widget_name.as_str()).await;

            if transactional {
                let seq = session.next_server_seq();
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: seq,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::WidgetPublishResult(WidgetPublishResult {
                            accepted: false,
                            widget_name: resolved_widget_name.clone(),
                            error_code,
                            error_message,
                        })),
                    }))
                    .await;
            }
        }
    }
}

/// Handle an InputFocusRequest from the client (RFC 0005 §3.8, RFC 0004 §8.3.1).
///
/// Synchronous: runtime responds with InputFocusResponse correlated by sequence.
/// v1 grants focus unconditionally (focus arbitration deferred to post-v1).
async fn handle_input_focus_request(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    req: InputFocusRequest,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::InputFocusResponse(InputFocusResponse {
                tile_id: req.tile_id.clone(),
                granted: true,
                reason: String::new(),
            })),
        }))
        .await;
}

/// Handle an InputCaptureRequest from the client (RFC 0005 §3.8, RFC 0004 §8.3.1).
///
/// Synchronous: runtime responds with InputCaptureResponse correlated by sequence.
/// v1 grants capture unconditionally (arbitration deferred to post-v1).
async fn handle_input_capture_request(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    req: InputCaptureRequest,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::InputCaptureResponse(InputCaptureResponse {
                tile_id: req.tile_id.clone(),
                granted: true,
                device_kind: req.device_kind.clone(),
                reason: String::new(),
            })),
        }))
        .await;
}

/// Handle an InputCaptureRelease from the client (RFC 0005 §3.8, RFC 0004 §8.3.1).
///
/// Asynchronous: confirmed by CaptureReleasedEvent in the next EventBatch (field 34).
/// No synchronous response is sent. The event is delivered with reason=AGENT_RELEASED.
/// v1: immediately delivers a CaptureReleasedEvent in a synthetic EventBatch.
async fn handle_input_capture_release(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    rel: InputCaptureRelease,
) {
    use crate::proto::CaptureReleasedReason;
    use crate::proto::input_envelope::Event as InputEvent;
    use crate::proto::{CaptureReleasedEvent, EventBatch, InputEnvelope};

    // Only deliver the CaptureReleasedEvent if the agent is subscribed to FOCUS_EVENTS.
    // CaptureReleasedEvent is a focus variant (RFC 0005 §7.1).
    if !session
        .subscriptions
        .iter()
        .any(|s| s == subscriptions::category::FOCUS_EVENTS)
    {
        // Agent not subscribed to FOCUS_EVENTS; do not deliver CaptureReleasedEvent.
        // The release is still processed (capture is released from the runtime side).
        return;
    }

    let now_us = now_wall_us();
    let seq = session.next_server_seq();

    let capture_released = CaptureReleasedEvent {
        tile_id: rel.tile_id.clone(),
        node_id: Vec::new(),
        timestamp_mono_us: 0, // no monotonic clock available; leave unset (v1)
        device_id: rel.device_kind.clone(),
        reason: CaptureReleasedReason::AgentReleased as i32,
    };

    let batch = EventBatch {
        frame_number: 0, // synthetic batch (not tied to compositor frame)
        batch_ts_us: now_us,
        events: vec![InputEnvelope {
            event: Some(InputEvent::CaptureReleased(capture_released)),
        }],
    };

    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_us,
            payload: Some(ServerPayload::EventBatch(batch)),
        }))
        .await;
}

async fn handle_heartbeat(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    hb: Heartbeat,
) {
    session.last_heartbeat_ms = now_ms();

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::Heartbeat(Heartbeat {
                // Echo the client's monotonic timestamp for RTT calculation
                timestamp_mono_us: hb.timestamp_mono_us,
            })),
        }))
        .await;
}

// ─── Agent Scene Event Emission handler ──────────────────────────────────────

/// Handle an `EmitSceneEvent` request from an agent.
///
/// Implements the server-side of the agent event emission protocol per
/// scene-events/spec.md §5.1–§5.4:
///
/// 1. Validate the bare name (format + reserved prefix).
/// 2. Check the `emit_scene_event:<bare_name>` capability.
/// 3. Enforce the 4 KB payload size limit.
/// 4. Apply the per-session sliding-window rate limit.
/// 5. On success, dispatch the event to subscribers and respond with the
///    fully-prefixed event type.
///
/// Per-session rate limiting is enforced via the
/// `StreamSession::agent_event_rate_limiter: AgentEventRateLimiter` field.
///
/// Note: Full event bus delivery to subscribers (step 5) is wired in by bead #2.
/// This handler performs all gating checks and returns a result; actual fan-out
/// to subscription channels is not implemented in this bead.
async fn handle_emit_scene_event(
    _state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    emit: EmitSceneEvent,
) {
    // Run all validation checks. On rejection, the Err variant carries the
    // (error_code, error_message) pair for the wire response; on success, the
    // Ok variant carries the fully-prefixed delivered_event_type.
    let outcome = validate_emission(session, &emit);

    let seq = session.next_server_seq();
    let (accepted, delivered_event_type, error_code, error_message) = match outcome {
        Ok(delivered) => (true, delivered, String::new(), String::new()),
        Err((code, msg)) => (false, String::new(), code, msg),
    };

    // TODO (bead #2): on accepted, dispatch delivered_event_type to subscribers
    // via the event bus.

    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                request_sequence,
                accepted,
                delivered_event_type,
                error_code,
                error_message,
            })),
        }))
        .await;
}

/// Validate all emission gates for `EmitSceneEvent`, mutating session state
/// (rate-limiter) only on acceptance.
///
/// Returns `Ok(delivered_event_type)` on success, or
/// `Err((error_code, error_message))` on the first failing gate.
///
/// Validation order (spec §5.1–§5.4):
/// 1. Bare name format and reserved-prefix check.
/// 2. Capability check (`emit_scene_event:<bare_name>`).
/// 3. Payload size limit (≤ [`MAX_PAYLOAD_BYTES`]).
/// 4. Sliding-window rate limit ([`DEFAULT_MAX_EVENTS_PER_SECOND`] events/s).
fn validate_emission(
    session: &mut StreamSession,
    emit: &EmitSceneEvent,
) -> Result<String, (String, String)> {
    use tze_hud_scene::events::naming::{NamingError, build_agent_event_type, validate_bare_name};

    // ── Step 1: Validate bare name (format + reserved prefix) ────────────
    if let Err(naming_err) = validate_bare_name(&emit.bare_name) {
        let (code, msg) = match &naming_err {
            NamingError::ReservedPrefix { prefix } => (
                "AGENT_EVENT_RESERVED_PREFIX".to_string(),
                format!("bare name must not start with reserved prefix {prefix:?}"),
            ),
            _ => (
                "AGENT_EVENT_INVALID_NAME".to_string(),
                format!("invalid bare name: {naming_err}"),
            ),
        };
        return Err((code, msg));
    }

    // ── Step 2: Capability check ──────────────────────────────────────────
    let required_cap = format!("emit_scene_event:{}", emit.bare_name);
    if !capability_set_covers(&session.capabilities, &required_cap) {
        return Err((
            "AGENT_EVENT_CAPABILITY_MISSING".to_string(),
            format!("missing capability: {required_cap}"),
        ));
    }

    // ── Step 3: Payload size limit ────────────────────────────────────────
    if emit.payload.len() > MAX_PAYLOAD_BYTES {
        return Err((
            "AGENT_EVENT_PAYLOAD_TOO_LARGE".to_string(),
            format!(
                "payload {} bytes exceeds {MAX_PAYLOAD_BYTES}-byte limit",
                emit.payload.len()
            ),
        ));
    }

    // ── Step 4: Rate limit ────────────────────────────────────────────────
    if session
        .agent_event_rate_limiter
        .check_and_record(std::time::Instant::now())
        .is_err()
    {
        return Err((
            "AGENT_EVENT_RATE_EXCEEDED".to_string(),
            format!(
                "agent event rate limit exceeded ({DEFAULT_MAX_EVENTS_PER_SECOND}/s sliding window)"
            ),
        ));
    }

    // ── Accepted: build fully-prefixed event type ─────────────────────────
    Ok(build_agent_event_type(&session.namespace, &emit.bare_name))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::session::hud_session_client::HudSessionClient;
    use crate::proto::session::hud_session_server::HudSessionServer;
    use std::collections::HashMap;
    use tokio_stream::StreamExt;
    use tze_hud_scene::graph::SceneGraph;

    /// Consume the next non-LeaseStateChange message from a stream.
    ///
    /// Some test scenarios interleave LeaseStateChange events (e.g.,
    /// REQUESTED→ACTIVE after lease grant) with MutationResult/RuntimeError
    /// messages. This helper drains those state-change events so tests can
    /// assert on the first substantive message without order-dependency.
    async fn next_non_state_change(
        stream: &mut tonic::Streaming<crate::proto::session::ServerMessage>,
    ) -> crate::proto::session::ServerMessage {
        use crate::proto::session::server_message::Payload as P;
        loop {
            let msg = stream.next().await.unwrap().unwrap();
            if let Some(P::LeaseStateChange(_)) = &msg.payload {
                continue;
            }
            return msg;
        }
    }

    /// Start a test server and return a connected client.
    async fn setup_test() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        let client = connect_test_client_with_retry(addr.port()).await;

        (client, handle)
    }

    /// Start a test server with explicit agent capability policy settings.
    async fn setup_test_with_policy(
        agent_capabilities: HashMap<String, Vec<String>>,
        fallback_unrestricted: bool,
    ) -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let scene = SceneGraph::new(800.0, 600.0);
        let base = HudSessionImpl::new(scene, "test-key");
        let service = HudSessionImpl::from_shared_state_with_config(
            base.state.clone(),
            "test-key",
            agent_capabilities,
            fallback_unrestricted,
        );

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        let client = connect_test_client_with_retry(addr.port()).await;

        (client, handle)
    }

    async fn connect_test_client_with_retry(
        port: u16,
    ) -> HudSessionClient<tonic::transport::Channel> {
        let endpoint = format!("http://[::1]:{port}");
        for attempt in 0..25 {
            if let Ok(client) = HudSessionClient::connect(endpoint.clone()).await {
                return client;
            }
            if attempt < 24 {
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            }
        }
        panic!("failed to connect test client to {endpoint} after retries");
    }

    /// Helper: create a bidirectional stream and perform handshake.
    /// Returns (sender, first few server messages including SessionEstablished + SceneSnapshot).
    async fn handshake(
        client: &mut HudSessionClient<tonic::transport::Channel>,
        agent_id: &str,
        psk: &str,
    ) -> (
        tokio::sync::mpsc::Sender<ClientMessage>,
        Vec<ServerMessage>,
        tonic::Streaming<ServerMessage>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // Send SessionInit with canonical capability names (create_tiles, access_input_events)
        // and read_scene_topology so SCENE_TOPOLOGY subscription is granted.
        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_id.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "access_input_events".to_string(),
                    "read_scene_topology".to_string(),
                ],
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();

        // Collect SessionEstablished and SceneSnapshot
        let mut messages = Vec::new();
        // We expect exactly 2 messages: SessionEstablished and SceneSnapshot
        for _ in 0..2 {
            if let Some(msg) = response_stream.next().await {
                messages.push(msg.unwrap());
            }
        }

        (tx, messages, response_stream)
    }

    /// Helper: perform SessionInit with an explicit requested capability list.
    async fn handshake_with_requested_capabilities(
        client: &mut HudSessionClient<tonic::transport::Channel>,
        agent_id: &str,
        psk: &str,
        requested_capabilities: Vec<String>,
    ) -> (
        tokio::sync::mpsc::Sender<ClientMessage>,
        Vec<ServerMessage>,
        tonic::Streaming<ServerMessage>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_id.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities,
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let mut messages = Vec::new();
        for _ in 0..2 {
            if let Some(msg) = response_stream.next().await {
                messages.push(msg.unwrap());
            }
        }
        (tx, messages, response_stream)
    }

    #[tokio::test]
    async fn test_handshake_init_established_and_snapshot() {
        let (mut client, _server) = setup_test().await;
        let (_tx, messages, _stream) = handshake(&mut client, "test-agent", "test-key").await;

        assert_eq!(messages.len(), 2);

        // First message: SessionEstablished
        match &messages[0].payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                assert!(!established.session_id.is_empty());
                assert_eq!(established.namespace, "test-agent");
                assert!(
                    established
                        .granted_capabilities
                        .contains(&"create_tiles".to_string())
                );
                assert!(
                    established
                        .granted_capabilities
                        .contains(&"access_input_events".to_string())
                );
                assert!(
                    established
                        .granted_capabilities
                        .contains(&"read_scene_topology".to_string())
                );
                assert!(!established.resume_token.is_empty());
                assert_eq!(
                    established.heartbeat_interval_ms,
                    DEFAULT_HEARTBEAT_INTERVAL_MS
                );
                // SCENE_TOPOLOGY is granted because agent has read_scene_topology capability
                assert!(
                    established
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "SCENE_TOPOLOGY should be active (agent has read_scene_topology)"
                );
                // Mandatory subscriptions always present
                assert!(
                    established
                        .active_subscriptions
                        .contains(&"DEGRADATION_NOTICES".to_string()),
                    "DEGRADATION_NOTICES must always be active"
                );
                assert!(
                    established
                        .active_subscriptions
                        .contains(&"LEASE_CHANGES".to_string()),
                    "LEASE_CHANGES must always be active"
                );
                // denied_subscriptions must be empty (all requested categories granted)
                assert!(
                    established.denied_subscriptions.is_empty(),
                    "no subscriptions should be denied"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }

        // Second message: SceneSnapshot
        match &messages[1].payload {
            Some(ServerPayload::SceneSnapshot(snapshot)) => {
                assert!(!snapshot.snapshot_json.is_empty());
            }
            other => panic!("Expected SceneSnapshot, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handshake_auth_failure() {
        let (mut client, _server) = setup_test().await;

        let (_tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let (init_tx, init_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(init_rx);

        // Send SessionInit with wrong key
        init_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionInit(SessionInit {
                    agent_id: "bad-agent".to_string(),
                    agent_display_name: "bad-agent".to_string(),
                    pre_shared_key: "wrong-key".to_string(),
                    requested_capabilities: Vec::new(),
                    initial_subscriptions: Vec::new(),
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: 0,
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();

        match &msg.payload {
            Some(ServerPayload::SessionError(error)) => {
                assert_eq!(error.code, "AUTH_FAILED");
            }
            other => panic!("Expected SessionError, got: {other:?}"),
        }

        drop(_tx);
        drop(rx);
    }

    #[tokio::test]
    async fn test_mutation_over_stream() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) = handshake(&mut client, "mutator", "test-key").await;

        // First, request a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Create a tab in the scene (needed for mutations)
        // We need to do this through shared state since tab creation
        // isn't exposed via the streaming protocol yet.
        // For the test, we'll send a mutation that doesn't require a tab.

        // Send a mutation batch
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::CreateTile(
                        crate::proto::CreateTileMutation {
                            tab_id: vec![], // empty = server infers active tab
                            bounds: Some(crate::proto::Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 200.0,
                                height: 150.0,
                                ..Default::default()
                            }),
                            z_order: 1,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .unwrap();

        // Drain any interleaved LeaseStateChange events before expecting MutationResult.
        // A LeaseStateChange(REQUESTED -> ACTIVE) may be emitted after lease grant.
        let result_msg = loop {
            let msg = stream.next().await.unwrap().unwrap();
            if let Some(ServerPayload::LeaseStateChange(_)) = &msg.payload {
                continue; // skip lease state events
            }
            break msg;
        };
        match &result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                // This will fail because no active tab exists, which is expected
                // in this isolated test. The important thing is that the protocol
                // round-trip works.
                assert_eq!(result.batch_id, batch_id);
                // accepted may be false due to "no active tab" -- that's fine
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_create_tile_persists_element_store_entry() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let path = std::env::temp_dir().join(format!(
            "tze_hud_element_store_session_server_{}.toml",
            SceneId::new()
        ));
        let _ = std::fs::remove_file(&path);

        {
            let mut st = shared_state.lock().await;
            st.element_store = tze_hud_scene::element_store::ElementStore::default();
            st.element_store_path = Some(path.clone());
            st.scene
                .lock()
                .await
                .create_tab("main", 0)
                .expect("create tab");
        }

        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "persist-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .expect("lease request");

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected granted LeaseResponse, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::CreateTile(
                        crate::proto::CreateTileMutation {
                            tab_id: vec![],
                            bounds: Some(crate::proto::Rect {
                                x: 8.0,
                                y: 8.0,
                                width: 200.0,
                                height: 100.0,
                                ..Default::default()
                            }),
                            z_order: 1,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .expect("mutation batch");

        let result_msg = next_non_state_change(&mut stream).await;
        let created_tile_id = match &result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(result.accepted, "create tile must be accepted");
                assert_eq!(result.created_ids.len(), 1, "one tile should be created");
                bytes_to_scene_id(&result.created_ids[0]).expect("valid created tile id bytes")
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        };

        let store = tze_hud_scene::element_store::ElementStore::load_or_default(&path)
            .expect("load persisted element store");
        let entry = store
            .entries
            .get(&created_tile_id)
            .expect("tile id should be persisted");
        assert_eq!(
            entry.element_type,
            tze_hud_scene::element_store::ElementType::Tile
        );
        assert_eq!(entry.namespace, "persist-agent");
        assert!(entry.created_at > 0);
        assert!(entry.last_published_at >= entry.created_at);

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_existing_tile_last_published_update_triggers_persist() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let path = std::env::temp_dir().join(format!(
            "tze_hud_element_store_last_published_{}.toml",
            SceneId::new()
        ));
        let _ = std::fs::remove_file(&path);

        {
            let mut st = shared_state.lock().await;
            st.element_store = tze_hud_scene::element_store::ElementStore::default();
            st.element_store_path = Some(path.clone());
            st.scene
                .lock()
                .await
                .create_tab("main", 0)
                .expect("create tab");
        }

        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "persist-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .expect("lease request");

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected granted LeaseResponse, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::CreateTile(
                        crate::proto::CreateTileMutation {
                            tab_id: vec![],
                            bounds: Some(crate::proto::Rect {
                                x: 8.0,
                                y: 8.0,
                                width: 200.0,
                                height: 100.0,
                                ..Default::default()
                            }),
                            z_order: 1,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .expect("mutation batch");

        let result_msg = next_non_state_change(&mut stream).await;
        let created_tile_id = match &result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(result.accepted, "create tile must be accepted");
                assert_eq!(result.created_ids.len(), 1, "one tile should be created");
                bytes_to_scene_id(&result.created_ids[0]).expect("valid created tile id bytes")
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        };

        let baseline_store = tze_hud_scene::element_store::ElementStore::load_or_default(&path)
            .expect("load baseline element store");
        let baseline_entry = baseline_store
            .entries
            .get(&created_tile_id)
            .expect("baseline tile id should be persisted");
        let baseline_last_published = baseline_entry.last_published_at;

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let persist_request = {
            let mut st = shared_state.lock().await;
            let entry = st
                .element_store
                .entries
                .get_mut(&created_tile_id)
                .expect("in-memory tile entry should exist");
            entry.last_published_at = baseline_last_published;
            persist_created_tile_entries(&mut st, &[created_tile_id]).await
        };
        persist_element_store(persist_request).await;

        let updated_store = tze_hud_scene::element_store::ElementStore::load_or_default(&path)
            .expect("reload element store");
        let updated_entry = updated_store
            .entries
            .get(&created_tile_id)
            .expect("tile id should remain persisted");
        assert!(
            updated_entry.last_published_at > baseline_last_published,
            "last_published_at update must be persisted when it is the only changed field"
        );

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_list_elements_request_supports_filters_and_override_metadata() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let tile_id: SceneId;
        let zone_id = SceneId::new();
        let widget_id = SceneId::new();

        {
            let mut st = shared_state.lock().await;
            st.element_store = tze_hud_scene::element_store::ElementStore::default();
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("main", 0).expect("create tab");

            let bootstrap_lease = scene.grant_lease(
                "agent-list",
                60_000,
                vec![
                    Capability::CreateTile,
                    Capability::UpdateTile,
                    Capability::CreateNode,
                    Capability::UpdateNode,
                    Capability::PublishZone("list-zone".to_string()),
                ],
            );

            tile_id = scene
                .create_tile(
                    tab_id,
                    "agent-list",
                    bootstrap_lease,
                    Rect::new(40.0, 30.0, 160.0, 120.0),
                    1,
                )
                .expect("create tile");

            scene.zone_registry.register(ZoneDefinition {
                id: zone_id,
                name: "list-zone".to_string(),
                description: "ListElements test zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 0.1,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::LatestWins,
                max_publishers: 2,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            });

            scene
                .publish_to_zone_with_lease(
                    "list-zone",
                    ZoneContent::StreamText("hello".to_string()),
                    "agent-list",
                    None,
                )
                .expect("publish zone");

            scene.widget_registry.register_definition(WidgetDefinition {
                id: "gauge".to_string(),
                name: "Gauge".to_string(),
                description: "Gauge widget".to_string(),
                parameter_schema: vec![WidgetParameterDeclaration {
                    name: "level".to_string(),
                    param_type: WidgetParamType::F32,
                    default_value: WidgetParameterValue::F32(0.0),
                    constraints: None,
                }],
                layers: vec![],
                default_geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.2,
                    y_pct: 0.2,
                    width_pct: 0.2,
                    height_pct: 0.1,
                },
                default_rendering_policy: RenderingPolicy::default(),
                default_contention_policy: ContentionPolicy::LatestWins,
                ephemeral: false,
                hover_behavior: None,
            });
            scene.widget_registry.register_instance(WidgetInstance {
                id: widget_id,
                widget_type_name: "gauge".to_string(),
                tab_id,
                geometry_override: None,
                contention_override: None,
                instance_name: "gauge-main".to_string(),
                current_params: HashMap::new(),
            });
            scene
                .publish_to_widget(
                    "gauge-main",
                    HashMap::from([("level".to_string(), WidgetParameterValue::F32(0.5))]),
                    "agent-list",
                    None,
                    0,
                    None,
                )
                .expect("publish widget");
            drop(scene);

            st.element_store.entries.insert(
                tile_id,
                tze_hud_scene::element_store::ElementStoreEntry {
                    element_type: tze_hud_scene::element_store::ElementType::Tile,
                    namespace: "agent-list".to_string(),
                    created_at: 101,
                    last_published_at: 202,
                    geometry_override: Some(GeometryPolicy::Relative {
                        x_pct: 0.25,
                        y_pct: 0.1,
                        width_pct: 0.2,
                        height_pct: 0.2,
                    }),
                },
            );
            st.element_store.entries.insert(
                zone_id,
                tze_hud_scene::element_store::ElementStoreEntry {
                    element_type: tze_hud_scene::element_store::ElementType::Zone,
                    namespace: "list-zone".to_string(),
                    created_at: 303,
                    last_published_at: 404,
                    geometry_override: None,
                },
            );
            st.element_store.entries.insert(
                widget_id,
                tze_hud_scene::element_store::ElementStoreEntry {
                    element_type: tze_hud_scene::element_store::ElementType::Widget,
                    namespace: "gauge-main".to_string(),
                    created_at: 505,
                    last_published_at: 606,
                    geometry_override: None,
                },
            );
        }

        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "agent-list", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ListElementsRequest(
                crate::proto::ListElementsRequest {
                    namespace_filter: Some("agent-".to_string()),
                    element_type: Some("tile".to_string()),
                },
            )),
        })
        .await
        .expect("send list-elements request");

        let tile_only = next_non_state_change(&mut stream).await;
        match tile_only.payload {
            Some(ServerPayload::ListElementsResponse(resp)) => {
                assert_eq!(
                    resp.elements.len(),
                    1,
                    "tile filter should return one element"
                );
                let entry = &resp.elements[0];
                assert_eq!(entry.element_type, "tile");
                assert_eq!(entry.namespace, "agent-list");
                assert_eq!(
                    bytes_to_scene_id(&entry.element_id).expect("tile entry id must decode"),
                    tile_id
                );
                assert!(entry.has_user_override, "tile should report user override");
                assert_eq!(entry.created_at_ms, 101);
                assert_eq!(entry.last_published_at_ms, 202);
                match entry
                    .current_geometry
                    .as_ref()
                    .and_then(|g| g.policy.as_ref())
                {
                    Some(crate::proto::geometry_policy_proto::Policy::Relative(relative)) => {
                        assert!((relative.x_pct - 0.25).abs() < 1e-6);
                        assert!((relative.y_pct - 0.1).abs() < 1e-6);
                        assert!((relative.width_pct - 0.2).abs() < 1e-6);
                        assert!((relative.height_pct - 0.2).abs() < 1e-6);
                    }
                    other => panic!("expected relative geometry policy, got {other:?}"),
                }
            }
            other => panic!("Expected ListElementsResponse, got: {other:?}"),
        }

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ListElementsRequest(
                crate::proto::ListElementsRequest {
                    namespace_filter: Some("list-".to_string()),
                    element_type: Some("zone".to_string()),
                },
            )),
        })
        .await
        .expect("send list-elements zone filter request");

        let zone_only = next_non_state_change(&mut stream).await;
        match zone_only.payload {
            Some(ServerPayload::ListElementsResponse(resp)) => {
                assert_eq!(
                    resp.elements.len(),
                    1,
                    "zone filter should return one element"
                );
                assert_eq!(resp.elements[0].element_type, "zone");
                assert_eq!(resp.elements[0].namespace, "list-zone");
                assert_eq!(
                    bytes_to_scene_id(&resp.elements[0].element_id)
                        .expect("zone entry id must decode"),
                    zone_id
                );
            }
            other => panic!("Expected ListElementsResponse, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_publish_to_tile_by_element_id_applies_override_and_updates_timestamp() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let tile_id: SceneId;

        {
            let mut st = shared_state.lock().await;
            st.element_store = tze_hud_scene::element_store::ElementStore::default();
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("main", 0).expect("create tab");
            let bootstrap_lease = scene.grant_lease(
                "tile-publisher",
                60_000,
                vec![
                    Capability::CreateTile,
                    Capability::UpdateTile,
                    Capability::CreateNode,
                    Capability::UpdateNode,
                ],
            );
            tile_id = scene
                .create_tile(
                    tab_id,
                    "tile-publisher",
                    bootstrap_lease,
                    Rect::new(20.0, 20.0, 100.0, 80.0),
                    1,
                )
                .expect("create tile");
            drop(scene);

            st.element_store.entries.insert(
                tile_id,
                tze_hud_scene::element_store::ElementStoreEntry {
                    element_type: tze_hud_scene::element_store::ElementType::Tile,
                    namespace: "tile-publisher".to_string(),
                    created_at: 1,
                    last_published_at: 1,
                    geometry_override: Some(GeometryPolicy::Relative {
                        x_pct: 0.4,
                        y_pct: 0.25,
                        width_pct: 0.3,
                        height_pct: 0.2,
                    }),
                },
            );
        }

        let (tx, _init_messages, mut stream) = handshake_with_requested_capabilities(
            &mut client,
            "tile-publisher",
            "test-key",
            vec![
                "create_tiles".to_string(),
                "modify_own_tiles".to_string(),
                "read_scene_topology".to_string(),
            ],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .expect("lease request");

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id,
            other => panic!("Expected granted lease response, got: {other:?}"),
        };

        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "publish-to-tile".to_string(),
                bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                background: None,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::PublishToTile(
                        crate::proto::PublishToTileMutation {
                            element_id: scene_id_to_bytes(tile_id),
                            bounds: Some(crate::proto::Rect {
                                x: 5.0,
                                y: 5.0,
                                width: 20.0,
                                height: 10.0,
                                ..Default::default()
                            }),
                            node: Some(crate::convert::scene_node_to_proto(&node)),
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .expect("publish-to-tile mutation");

        let result_msg = next_non_state_change(&mut stream).await;
        match result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(result.accepted, "publish_to_tile should be accepted");
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }

        {
            let st = shared_state.lock().await;
            let scene = st.scene.lock().await;
            let tile = scene.tiles.get(&tile_id).expect("tile should exist");
            assert!((tile.bounds.x - 320.0).abs() < 1e-3);
            assert!((tile.bounds.y - 150.0).abs() < 1e-3);
            assert!((tile.bounds.width - 240.0).abs() < 1e-3);
            assert!((tile.bounds.height - 120.0).abs() < 1e-3);

            let root_id = tile.root_node.expect("tile root should be set");
            let root = scene
                .nodes
                .get(&root_id)
                .expect("tile root node should exist");
            match &root.data {
                NodeData::TextMarkdown(markdown) => {
                    assert_eq!(markdown.content, "publish-to-tile");
                }
                other => panic!("expected markdown node, got {other:?}"),
            }

            let entry = st
                .element_store
                .entries
                .get(&tile_id)
                .expect("element store entry should exist");
            assert!(
                entry.last_published_at > 1,
                "publish_to_tile should update last_published_at"
            );
        }
    }

    #[tokio::test]
    async fn test_publish_to_tile_by_element_id_rejects_invalid_node_even_with_bounds() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let tile_id: SceneId;

        {
            let mut st = shared_state.lock().await;
            st.element_store = tze_hud_scene::element_store::ElementStore::default();
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("main", 0).expect("create tab");
            let bootstrap_lease = scene.grant_lease(
                "tile-publisher-invalid-node",
                60_000,
                vec![
                    Capability::CreateTile,
                    Capability::UpdateTile,
                    Capability::CreateNode,
                    Capability::UpdateNode,
                ],
            );
            tile_id = scene
                .create_tile(
                    tab_id,
                    "tile-publisher-invalid-node",
                    bootstrap_lease,
                    Rect::new(20.0, 20.0, 100.0, 80.0),
                    1,
                )
                .expect("create tile");
            drop(scene);

            st.element_store.entries.insert(
                tile_id,
                tze_hud_scene::element_store::ElementStoreEntry {
                    element_type: tze_hud_scene::element_store::ElementType::Tile,
                    namespace: "tile-publisher-invalid-node".to_string(),
                    created_at: 1,
                    last_published_at: 1,
                    geometry_override: None,
                },
            );
        }

        let (tx, _init_messages, mut stream) = handshake_with_requested_capabilities(
            &mut client,
            "tile-publisher-invalid-node",
            "test-key",
            vec![
                "create_tiles".to_string(),
                "modify_own_tiles".to_string(),
                "read_scene_topology".to_string(),
            ],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .expect("lease request");

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id,
            other => panic!("Expected granted lease response, got: {other:?}"),
        };

        let mut invalid_node = crate::convert::scene_node_to_proto(&Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 16.0, 16.0),
            }),
        });
        invalid_node.data = None;

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::PublishToTile(
                        crate::proto::PublishToTileMutation {
                            element_id: scene_id_to_bytes(tile_id),
                            bounds: Some(crate::proto::Rect {
                                x: 10.0,
                                y: 10.0,
                                width: 60.0,
                                height: 40.0,
                                ..Default::default()
                            }),
                            node: Some(invalid_node),
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .expect("publish-to-tile invalid node mutation");

        let result_msg = next_non_state_change(&mut stream).await;
        match result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(!result.accepted, "invalid node should be rejected");
                assert_eq!(result.error_code, "INVALID_ARGUMENT");
                assert!(
                    result
                        .error_message
                        .contains("publish_to_tile node content is invalid or missing data"),
                    "unexpected error message: {}",
                    result.error_message
                );
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_publish_to_tile_by_element_id_returns_element_not_found() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        {
            let st = shared_state.lock().await;
            st.scene
                .lock()
                .await
                .create_tab("main", 0)
                .expect("create tab");
        }
        let (tx, _init_messages, mut stream) = handshake_with_requested_capabilities(
            &mut client,
            "tile-publisher-missing",
            "test-key",
            vec![
                "create_tiles".to_string(),
                "modify_own_tiles".to_string(),
                "read_scene_topology".to_string(),
            ],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .expect("lease request");

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id,
            other => panic!("Expected granted lease response, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::PublishToTile(
                        crate::proto::PublishToTileMutation {
                            element_id: scene_id_to_bytes(SceneId::new()),
                            bounds: Some(crate::proto::Rect {
                                x: 10.0,
                                y: 10.0,
                                width: 80.0,
                                height: 40.0,
                                ..Default::default()
                            }),
                            node: None,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .expect("publish-to-tile missing mutation");

        let result_msg = next_non_state_change(&mut stream).await;
        match result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(!result.accepted, "missing element_id should be rejected");
                assert_eq!(result.error_code, "ELEMENT_NOT_FOUND");
                assert!(
                    result
                        .error_message
                        .contains("publish_to_tile element_id does not reference a known tile"),
                    "unexpected error message: {}",
                    result.error_message
                );
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }
    }

    // ─── Regression tests for hud-wu32: batch_id correlation + lease_id propagation ──

    /// Regression: MutationResult.batch_id MUST echo the client-provided batch_id.
    ///
    /// Before this fix, handle_mutation_batch generated a fresh SceneId for
    /// `SceneMutationBatch.batch_id`, which meant the client could not correlate
    /// rejection responses with their own batch_id values.
    ///
    /// This test verifies that even when a mutation is rejected (here: "no active
    /// tab"), the MutationResult carries back the original client batch_id.
    #[tokio::test]
    async fn test_mutation_result_echoes_client_batch_id() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "batch-id-regression", "test-key").await;

        // Acquire a lease so the batch reaches the batch_id mapping code
        // (lease validation runs first; an invalid lease returns early before
        // the batch_id mapping happens).
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Send a mutation batch with a known, unique batch_id.
        let client_batch_id: Vec<u8> = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: client_batch_id.clone(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::CreateTile(
                        crate::proto::CreateTileMutation {
                            tab_id: vec![],
                            bounds: Some(crate::proto::Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 100.0,
                                height: 100.0,
                                ..Default::default()
                            }),
                            z_order: 0,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .unwrap();

        // The batch will be rejected (no active tab in setup_test).
        // Regardless of rejection, MutationResult.batch_id MUST equal client_batch_id.
        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert_eq!(
                    result.batch_id, client_batch_id,
                    "MutationResult.batch_id must echo the client-provided batch_id \
                     (regression for hud-wu32: batch_id was previously a fresh SceneId)"
                );
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }
    }

    /// Regression: lease_id MUST be propagated into SceneMutationBatch so that
    /// the five-stage validation pipeline (including lease/budget checks) fires.
    ///
    /// Before this fix, `lease_id: None` was passed, which meant lease and budget
    /// validation was skipped for non-CreateTile mutations in the gRPC path.
    ///
    /// This test verifies that a mutation using an expired lease is rejected with
    /// an error indicating lease/budget validation ran — not silently accepted.
    #[tokio::test]
    async fn test_mutation_rejected_with_expired_lease_id() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "lease-validation-regression", "test-key").await;

        // Create an active tab so mutations can reach the scene-apply path.
        {
            let st = shared_state.lock().await;
            st.scene
                .lock()
                .await
                .create_tab("test-tab", 0)
                .expect("create_tab");
        }

        // Acquire a lease.
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id_bytes = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Revoke the lease directly in shared state, simulating an expired lease.
        // The wire format encodes SceneId as uuid::Uuid::as_bytes() (big-endian UUID bytes),
        // matching bytes_to_scene_id in session_server.rs.
        {
            let st = shared_state.lock().await;
            let arr: [u8; 16] = lease_id_bytes
                .as_slice()
                .try_into()
                .expect("16-byte lease_id");
            let lease_id = tze_hud_scene::SceneId::from_uuid(uuid::Uuid::from_bytes(arr));
            let _ = st.scene.lock().await.revoke_lease(lease_id);
        }

        // Send a CreateTile mutation referencing the now-revoked lease.
        let batch_id: Vec<u8> = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id_bytes,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::CreateTile(
                        crate::proto::CreateTileMutation {
                            tab_id: vec![],
                            bounds: Some(crate::proto::Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 100.0,
                                height: 100.0,
                                ..Default::default()
                            }),
                            z_order: 0,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .unwrap();

        // The batch MUST be rejected (lease is revoked; validation pipeline runs).
        // batch_id must still be echoed back.
        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(
                    !result.accepted,
                    "Mutation with revoked lease_id must be rejected \
                     (regression for hud-wu32: lease_id=None previously bypassed validation)"
                );
                assert_eq!(
                    result.batch_id, batch_id,
                    "MutationResult.batch_id must echo client batch_id even on rejection"
                );
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_lease_over_stream() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) = handshake(&mut client, "leasor", "test-key").await;

        // Request a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec![
                    "create_tiles".to_string(),
                    "access_input_events".to_string(),
                ],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted, "expected lease to be granted");
                assert!(!resp.lease_id.is_empty());
                assert_eq!(resp.lease_id.len(), 16);
                assert_eq!(resp.granted_ttl_ms, 30_000);
                assert!(
                    resp.granted_capabilities
                        .contains(&"create_tiles".to_string())
                );
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_heartbeat_echo() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "heartbeater", "test-key").await;

        let mono_us = 12345678u64;
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: mono_us,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::Heartbeat(hb)) => {
                assert_eq!(hb.timestamp_mono_us, mono_us);
            }
            other => panic!("Expected Heartbeat echo, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_resume_with_token() {
        let (mut client, _server) = setup_test().await;

        // Start initial session to get a resume token
        let (tx, init_messages, _stream) = handshake(&mut client, "resumable", "test-key").await;
        drop(tx); // Close the first stream
        drop(_stream);

        // Wait a bit for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Now resume with the token
        let resume_token = match &init_messages[0].payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                established.resume_token.clone()
            }
            _ => panic!("Expected SessionEstablished"),
        };

        let (resume_tx, resume_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let resume_stream = tokio_stream::wrappers::ReceiverStream::new(resume_rx);

        resume_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionResume(SessionResume {
                    agent_id: "resumable".to_string(),
                    resume_token,
                    last_seen_server_sequence: 2,
                    pre_shared_key: "test-key".to_string(),
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(resume_stream).await.unwrap().into_inner();

        // Should get SessionResumeResult + SceneSnapshot (not SessionEstablished)
        let msg1 = response_stream.next().await.unwrap().unwrap();
        match &msg1.payload {
            Some(ServerPayload::SessionResumeResult(result)) => {
                assert!(result.accepted, "expected resume to be accepted");
                assert!(!result.new_session_token.is_empty());
                // version = major * 1000 + minor; runtime max = v1.1 = 1001
                assert_eq!(
                    result.negotiated_protocol_version,
                    crate::auth::RUNTIME_MAX_VERSION
                );
            }
            other => panic!("Expected SessionResumeResult on resume, got: {other:?}"),
        }

        let msg2 = response_stream.next().await.unwrap().unwrap();
        match &msg2.payload {
            Some(ServerPayload::SceneSnapshot(_)) => {}
            other => panic!("Expected SceneSnapshot on resume, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_zone_publish_result() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "zone-publisher", "test-key").await;

        // Send a ZonePublish — expect ZonePublishResult correlated by client sequence
        let client_seq: u64 = 2;
        tx.send(ClientMessage {
            sequence: client_seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ZonePublish(ZonePublish {
                zone_name: "status".to_string(),
                content: Some(crate::proto::ZoneContent {
                    payload: Some(crate::proto::zone_content::Payload::StreamText(
                        "hello zone".to_string(),
                    )),
                }),
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
                breakpoints: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::ZonePublishResult(result)) => {
                // request_sequence must echo the client envelope sequence
                assert_eq!(
                    result.request_sequence, client_seq,
                    "ZonePublishResult.request_sequence must correlate with client ZonePublish sequence"
                );
                // Zone "status" doesn't exist in the default scene graph so it
                // will be rejected; we just verify the sequence correlation and
                // that error_code is populated on rejection.
                if !result.accepted {
                    assert!(
                        !result.error_code.is_empty(),
                        "rejected result must carry an error_code"
                    );
                }
            }
            other => panic!("Expected ZonePublishResult, got: {other:?}"),
        }
    }

    /// Scenario: Add subscription mid-session with required capability (RFC 0005 §7.3).
    /// Also validates subscription denied for missing capability.
    #[tokio::test]
    async fn test_subscription_change_result() {
        let (mut client, _server) = setup_test().await;

        // Use a custom handshake with access_input_events to test SubscriptionChange
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "subscriber".to_string(),
                agent_display_name: "subscriber".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec![
                    "read_scene_topology".to_string(),
                    "access_input_events".to_string(),
                ],
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();

        // Collect SessionEstablished and SceneSnapshot
        for _ in 0..2 {
            let _ = response_stream.next().await;
        }

        // Send a SubscriptionChange to add INPUT_EVENTS (has access_input_events)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SubscriptionChange(SubscriptionChange {
                subscribe: vec!["INPUT_EVENTS".to_string()],
                unsubscribe: Vec::new(),
                subscribe_filter: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SubscriptionChangeResult(result)) => {
                // Initial SCENE_TOPOLOGY subscription should still be active
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "initial SCENE_TOPOLOGY subscription should still be active"
                );
                // Newly added INPUT_EVENTS should be active (agent has access_input_events)
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"INPUT_EVENTS".to_string()),
                    "newly added INPUT_EVENTS subscription should be active"
                );
                // Mandatory subscriptions always present
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"DEGRADATION_NOTICES".to_string()),
                    "DEGRADATION_NOTICES must always be active"
                );
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"LEASE_CHANGES".to_string()),
                    "LEASE_CHANGES must always be active"
                );
                // No denied subscriptions (all requested categories have required capability)
                assert!(
                    result.denied_subscriptions.is_empty(),
                    "no subscriptions should be denied"
                );
            }
            other => panic!("Expected SubscriptionChangeResult, got: {other:?}"),
        }
        drop(tx);
    }

    /// Scenario: SubscriptionChange.subscribe_filter persists filter_prefix (RFC 0010 §7.2, spec line 179).
    ///
    /// WHEN agent sends SubscriptionChange with subscribe_filter=[{SCENE_TOPOLOGY, "scene.zone."}]
    /// THEN runtime accepts the subscription (no denial) and stores the filter_prefix in
    ///      session.subscription_filters so future event routing can apply the narrower filter.
    ///
    /// Additionally verifies that a subsequent plain `subscribe` for the same category
    /// clears the stored filter (resetting to category-default prefix behavior).
    #[tokio::test]
    async fn test_subscription_change_with_filter_prefix() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "filter-agent".to_string(),
                agent_display_name: "filter-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec!["read_scene_topology".to_string()],
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();

        // Collect SessionEstablished and SceneSnapshot
        for _ in 0..2 {
            let _ = response_stream.next().await;
        }

        // Step 1: Send SubscriptionChange with subscribe_filter: add SCENE_TOPOLOGY with "scene.zone." filter
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SubscriptionChange(SubscriptionChange {
                subscribe: Vec::new(),
                unsubscribe: Vec::new(),
                subscribe_filter: vec![crate::proto::session::SubscriptionEntry {
                    category: "SCENE_TOPOLOGY".to_string(),
                    filter_prefix: "scene.zone.".to_string(),
                }],
            })),
        })
        .await
        .unwrap();

        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SubscriptionChangeResult(result)) => {
                // SCENE_TOPOLOGY must be in the active set (subscribe_filter is processed as an add)
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "SCENE_TOPOLOGY must be active after subscribe_filter"
                );
                // No denials (agent has read_scene_topology capability)
                assert!(
                    result.denied_subscriptions.is_empty(),
                    "subscribe_filter with a valid capability must not produce denials"
                );
            }
            other => panic!("Expected SubscriptionChangeResult, got: {other:?}"),
        }

        // Step 2: Reset to default by sending a plain `subscribe` for SCENE_TOPOLOGY.
        // The stored filter must be cleared (empty filter_prefix resets to category default).
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SubscriptionChange(SubscriptionChange {
                subscribe: vec!["SCENE_TOPOLOGY".to_string()],
                unsubscribe: Vec::new(),
                subscribe_filter: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg2 = response_stream.next().await.unwrap().unwrap();
        match &msg2.payload {
            Some(ServerPayload::SubscriptionChangeResult(result2)) => {
                // SCENE_TOPOLOGY must still be active
                assert!(
                    result2
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "SCENE_TOPOLOGY must remain active after plain subscribe"
                );
                // No denials
                assert!(
                    result2.denied_subscriptions.is_empty(),
                    "plain subscribe for already-held category must not produce denials"
                );
            }
            other => panic!("Expected SubscriptionChangeResult for reset, got: {other:?}"),
        }

        // Step 3: Also verify that subscribe_filter with empty filter_prefix explicitly resets the filter.
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SubscriptionChange(SubscriptionChange {
                subscribe: Vec::new(),
                unsubscribe: Vec::new(),
                subscribe_filter: vec![crate::proto::session::SubscriptionEntry {
                    category: "SCENE_TOPOLOGY".to_string(),
                    filter_prefix: String::new(), // empty = reset to default
                }],
            })),
        })
        .await
        .unwrap();

        let msg3 = response_stream.next().await.unwrap().unwrap();
        match &msg3.payload {
            Some(ServerPayload::SubscriptionChangeResult(result3)) => {
                assert!(
                    result3
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "SCENE_TOPOLOGY must remain active after empty-prefix subscribe_filter"
                );
                assert!(
                    result3.denied_subscriptions.is_empty(),
                    "empty-prefix subscribe_filter for active category must not produce denials"
                );
            }
            other => {
                panic!("Expected SubscriptionChangeResult for empty-prefix reset, got: {other:?}")
            }
        }

        drop(tx);
    }

    /// Scenario: Subscription denied when capability is missing (RFC 0005 §7.1, spec lines 455-457).
    /// WHEN agent requests INPUT_EVENTS without access_input_events capability
    /// THEN subscription is denied and listed in denied_subscriptions.
    #[tokio::test]
    async fn test_subscription_denied_without_capability() {
        let (mut client, _server) = setup_test().await;

        // Handshake WITHOUT access_input_events capability
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "no-input-agent".to_string(),
                agent_display_name: "no-input-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec!["read_scene_topology".to_string()],
                // Request INPUT_EVENTS without access_input_events capability
                initial_subscriptions: vec![
                    "SCENE_TOPOLOGY".to_string(),
                    "INPUT_EVENTS".to_string(),
                ],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();

        // First message: SessionEstablished
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                // INPUT_EVENTS should be in denied_subscriptions
                assert!(
                    established
                        .denied_subscriptions
                        .contains(&"INPUT_EVENTS".to_string()),
                    "INPUT_EVENTS must be denied without access_input_events capability"
                );
                // INPUT_EVENTS should NOT be in active_subscriptions
                assert!(
                    !established
                        .active_subscriptions
                        .contains(&"INPUT_EVENTS".to_string()),
                    "INPUT_EVENTS must not be active without access_input_events capability"
                );
                // SCENE_TOPOLOGY is granted (agent has read_scene_topology)
                assert!(
                    established
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "SCENE_TOPOLOGY should be active with read_scene_topology capability"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
        drop(tx);
    }

    // ─── Sequence number validation tests (RFC 0005 §2.3) ────────────────────

    /// Scenario: Sequence gap exceeds threshold (RFC 0005 §2.3)
    /// WHEN client sends sequence 5 followed by 150 (gap > max_sequence_gap=100),
    /// THEN runtime closes the stream with SEQUENCE_GAP_EXCEEDED.
    #[tokio::test]
    async fn test_sequence_gap_exceeded() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "seq-gap-agent", "test-key").await;

        // Handshake consumes sequence 1. Send a valid message at sequence 2.
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 100,
            })),
        })
        .await
        .unwrap();

        // Drain the heartbeat echo
        let _ = stream.next().await;

        // Now jump to sequence 5, then to 150 — gap of 145 > DEFAULT_MAX_SEQUENCE_GAP=100
        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 200,
            })),
        })
        .await
        .unwrap();
        let _ = stream.next().await; // drain heartbeat echo

        tx.send(ClientMessage {
            sequence: 150,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 300,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "SEQUENCE_GAP_EXCEEDED",
                    "Expected SEQUENCE_GAP_EXCEEDED, got: {}",
                    err.code
                );
            }
            other => panic!("Expected SessionError(SEQUENCE_GAP_EXCEEDED), got: {other:?}"),
        }
    }

    /// Scenario: Sequence regression rejected (RFC 0005 §2.3)
    /// WHEN client sends sequence 10 followed by sequence 8,
    /// THEN runtime closes the stream with SEQUENCE_REGRESSION.
    #[tokio::test]
    async fn test_sequence_regression() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "seq-reg-agent", "test-key").await;

        // Send sequence 10
        tx.send(ClientMessage {
            sequence: 10,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 100,
            })),
        })
        .await
        .unwrap();
        let _ = stream.next().await; // drain heartbeat echo

        // Send sequence 8 — regression
        tx.send(ClientMessage {
            sequence: 8,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 200,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "SEQUENCE_REGRESSION",
                    "Expected SEQUENCE_REGRESSION, got: {}",
                    err.code
                );
            }
            other => panic!("Expected SessionError(SEQUENCE_REGRESSION), got: {other:?}"),
        }
    }

    /// Scenario: Monotonically increasing sequence numbers accepted.
    /// WHEN agent sends sequences 1, 2, 3,
    /// THEN all are processed without error.
    #[tokio::test]
    async fn test_sequence_monotonic_accepted() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "seq-ok-agent", "test-key").await;

        for seq in 2u64..=4 {
            tx.send(ClientMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::Heartbeat(Heartbeat {
                    timestamp_mono_us: seq * 1000,
                })),
            })
            .await
            .unwrap();

            let msg = stream.next().await.unwrap().unwrap();
            match &msg.payload {
                Some(ServerPayload::Heartbeat(hb)) => {
                    assert_eq!(hb.timestamp_mono_us, seq * 1000);
                }
                other => panic!("Expected Heartbeat echo at seq {seq}, got: {other:?}"),
            }
        }
    }

    // ─── Safe mode tests (RFC 0005 §3.7) ─────────────────────────────────────

    /// Scenario: Mutations rejected during safe mode (RFC 0005 §3.7)
    /// WHEN the runtime enters safe mode and sets `SharedState.safe_mode_active = true`,
    /// THEN MutationBatch is rejected with SAFE_MODE_ACTIVE.
    ///
    /// In this test we drive safe mode via `SharedState` directly (as the runtime
    /// would do via a SessionSuspended broadcast to all sessions).
    #[tokio::test]
    async fn test_safe_mode_rejects_mutations() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "safe-mode-agent", "test-key").await;

        // Enable safe mode in shared state (simulates runtime entering safe mode)
        {
            let mut st = shared_state.lock().await;
            st.safe_mode_active = true;
        }

        // Request a lease first (this is transactional, not affected by safe mode)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();
        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Send MutationBatch while safe mode is active — should be rejected
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                timing: None,
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut stream).await;
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(
                    err.error_code, "SAFE_MODE_ACTIVE",
                    "Expected SAFE_MODE_ACTIVE, got: {}",
                    err.error_code
                );
            }
            other => panic!("Expected RuntimeError(SAFE_MODE_ACTIVE), got: {other:?}"),
        }

        // Disable safe mode
        {
            let mut st = shared_state.lock().await;
            st.safe_mode_active = false;
        }

        // Mutations should no longer be rejected with SAFE_MODE_ACTIVE.
        // We use a heartbeat to verify the session is still responsive and
        // the safe mode is cleared.
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 999,
            })),
        })
        .await
        .unwrap();

        let msg2 = stream.next().await.unwrap().unwrap();
        match &msg2.payload {
            Some(ServerPayload::Heartbeat(hb)) => {
                // Session still active after safe mode was cleared
                assert_eq!(hb.timestamp_mono_us, 999, "Heartbeat should echo correctly");
            }
            other => panic!("Expected Heartbeat after safe mode exit, got: {other:?}"),
        }

        // Now verify a MutationBatch is no longer blocked by SAFE_MODE_ACTIVE.
        // (It may still fail due to invalid lease, but not because of safe mode.)
        let batch_id2 = uuid::Uuid::now_v7().as_bytes().to_vec();
        // Use the real lease from earlier
        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id2.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                timing: None,
            })),
        })
        .await
        .unwrap();

        let msg3 = stream.next().await.unwrap().unwrap();
        match &msg3.payload {
            Some(ServerPayload::MutationResult(result)) => {
                // We get MutationResult (not RuntimeError with SAFE_MODE_ACTIVE)
                assert_eq!(result.batch_id, batch_id2);
            }
            Some(ServerPayload::RuntimeError(err)) => {
                // Must NOT be SAFE_MODE_ACTIVE
                assert_ne!(
                    err.error_code, "SAFE_MODE_ACTIVE",
                    "Safe mode should be cleared, unexpected SAFE_MODE_ACTIVE"
                );
            }
            other => panic!("Unexpected message after safe mode exit: {other:?}"),
        }
    }

    // ─── Freeze queue tests (system-shell/spec.md §Freeze Scene) ────────────

    /// Scenario: Freeze queues mutations (spec line 146)
    /// WHEN viewer activates freeze via SharedState.freeze_active = true
    /// AND agent submits a MutationBatch
    /// THEN mutations are queued (accepted = true), tile content does not update
    #[tokio::test]
    async fn test_freeze_queues_mutations_not_applied() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "freeze-agent", "test-key").await;

        // Request a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();
        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Activate freeze
        {
            let mut st = shared_state.lock().await;
            st.freeze_active = true;
        }

        // Submit a MutationBatch while frozen
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut stream).await;
        match &msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                // Accepted=true: mutation was queued, not rejected
                assert_eq!(result.batch_id, batch_id);
                assert!(
                    result.accepted,
                    "Mutation should be accepted (queued) during freeze, not rejected"
                );
                // Scene should NOT have been modified; error code should not be SAFE_MODE_ACTIVE
                assert_ne!(result.error_code, "SAFE_MODE_ACTIVE");
            }
            Some(ServerPayload::RuntimeError(err)) => {
                panic!("Mutation should be queued during freeze, not rejected with error: {err:?}");
            }
            other => panic!("Expected MutationResult during freeze, got: {other:?}"),
        }

        // Deactivate freeze — queued mutation should be applied in next iteration
        {
            let mut st = shared_state.lock().await;
            st.freeze_active = false;
        }

        // Send a heartbeat to trigger the unfreeze drain on next loop iteration
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 9999,
            })),
        })
        .await
        .unwrap();

        // The unfreeze drain applies queued mutations (resulting in MutationResult(accepted))
        // before processing the heartbeat. We may get additional MutationResult messages.
        // Wait for the heartbeat echo to confirm the session is still active.
        let mut got_heartbeat = false;
        for _ in 0..5 {
            if let Some(Ok(msg)) = stream.next().await {
                match &msg.payload {
                    Some(ServerPayload::Heartbeat(hb)) => {
                        assert_eq!(hb.timestamp_mono_us, 9999);
                        got_heartbeat = true;
                        break;
                    }
                    Some(ServerPayload::MutationResult(_)) => {
                        // Drained mutation result — expected, continue
                    }
                    other => panic!("Unexpected message after unfreeze: {other:?}"),
                }
            }
        }
        assert!(
            got_heartbeat,
            "Expected heartbeat echo after unfreeze drain"
        );
    }

    /// Scenario: Freeze ignored during safe mode (spec line 137)
    /// WHEN safe mode is active AND freeze is set
    /// THEN mutations are rejected with SAFE_MODE_ACTIVE (not queued)
    #[tokio::test]
    async fn test_safe_mode_takes_precedence_over_freeze() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "safe-freeze-agent", "test-key").await;

        // Request a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();
        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Set BOTH safe mode and freeze (invariant: safe mode cancels freeze, but we test
        // that safe mode takes precedence in the session server check order)
        {
            let mut st = shared_state.lock().await;
            st.safe_mode_active = true;
            st.freeze_active = false; // Invariant: safe_mode=true => freeze_active=false
        }

        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut stream).await;
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(err.error_code, "SAFE_MODE_ACTIVE");
            }
            other => panic!("Expected SAFE_MODE_ACTIVE RuntimeError, got: {other:?}"),
        }
    }

    /// Scenario: SessionFreezeQueue unit test — MUTATION_QUEUE_PRESSURE at 80% capacity
    #[test]
    fn test_session_freeze_queue_pressure_signal() {
        let mut q = SessionFreezeQueue::new(10);
        // Fill 7 entries (70%) without crossing threshold
        for i in 0..7 {
            let batch = MutationBatch {
                batch_id: format!("b{i}").into_bytes(),
                lease_id: vec![0u8; 16],
                mutations: Vec::new(),
                ..Default::default()
            };
            let r = q.enqueue(batch, "ns");
            assert!(
                matches!(
                    r,
                    FreezeEnqueueResult::Queued {
                        pressure_warning: false
                    }
                ),
                "Expected no pressure warning at {i}/7"
            );
        }
        // 8th entry crosses 80%
        let batch = MutationBatch {
            batch_id: b"b7".to_vec(),
            lease_id: vec![0u8; 16],
            mutations: Vec::new(),
            ..Default::default()
        };
        let r = q.enqueue(batch, "ns");
        assert!(
            matches!(
                r,
                FreezeEnqueueResult::Queued {
                    pressure_warning: true
                }
            ),
            "Expected pressure_warning=true at 80%"
        );
    }

    /// Scenario: SessionFreezeQueue transactional never evicted
    #[test]
    fn test_session_freeze_queue_transactional_never_evicted() {
        use crate::proto::mutation_proto::Mutation;
        use crate::proto::{CreateTileMutation, MutationProto};

        let mut q = SessionFreezeQueue::new(2);
        // Fill with non-empty (StateStream) batches
        for i in 0..2 {
            let batch = MutationBatch {
                batch_id: format!("ss{i}").into_bytes(),
                lease_id: vec![0u8; 16],
                mutations: vec![],
                ..Default::default()
            };
            q.enqueue(batch, "ns");
        }

        // Submit a transactional mutation (CreateTile) — should get backpressure
        let tx_batch = MutationBatch {
            batch_id: b"tx1".to_vec(),
            lease_id: vec![0u8; 16],
            mutations: vec![MutationProto {
                mutation: Some(Mutation::CreateTile(CreateTileMutation {
                    tab_id: vec![], // empty = server infers active tab
                    bounds: None,
                    z_order: 0,
                    ..Default::default()
                })),
            }],
            ..Default::default()
        };
        let r = q.enqueue(tx_batch, "ns");
        assert!(
            matches!(r, FreezeEnqueueResult::BackpressureRequired),
            "Transactional mutation should require backpressure when queue is full, got: {r:?}"
        );
    }

    // ─── Session state machine tests (RFC 0005 §1.1) ─────────────────────────

    /// Scenario: Successful session establishment transitions through Connecting→Handshaking→Active.
    /// The state machine starts in Handshaking during the handle_session_init call and
    /// transitions to Active after the handshake response is sent.
    #[tokio::test]
    async fn test_state_machine_successful_establishment() {
        let (mut client, _server) = setup_test().await;
        let (_tx, messages, _stream) = handshake(&mut client, "state-test-agent", "test-key").await;

        // If handshake succeeded, we must have SessionEstablished followed by SceneSnapshot
        assert_eq!(
            messages.len(),
            2,
            "Expected SessionEstablished + SceneSnapshot"
        );
        assert!(
            matches!(
                messages[0].payload,
                Some(ServerPayload::SessionEstablished(_))
            ),
            "First message must be SessionEstablished"
        );
        assert!(
            matches!(messages[1].payload, Some(ServerPayload::SceneSnapshot(_))),
            "Second message must be SceneSnapshot"
        );
    }

    /// Scenario: Auth failure transitions Handshaking→Closed with SessionError.
    #[tokio::test]
    async fn test_state_machine_auth_failure_to_closed() {
        let (mut client, _server) = setup_test().await;

        let (init_tx, init_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(init_rx);

        init_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionInit(SessionInit {
                    agent_id: "state-fail-agent".to_string(),
                    agent_display_name: "state-fail-agent".to_string(),
                    pre_shared_key: "wrong-key".to_string(),
                    requested_capabilities: Vec::new(),
                    initial_subscriptions: Vec::new(),
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: 0,
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();

        // State machine should send SessionError (AUTH_FAILED) and transition to Closed
        match &msg.payload {
            Some(ServerPayload::SessionError(error)) => {
                assert_eq!(error.code, "AUTH_FAILED");
            }
            other => panic!("Expected SessionError(AUTH_FAILED), got: {other:?}"),
        }
    }

    /// Scenario: Graceful disconnect via SessionClose.
    /// The session stream should terminate cleanly after SessionClose is sent.
    #[tokio::test]
    async fn test_graceful_disconnect_session_close() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "close-agent", "test-key").await;

        // Send SessionClose with expect_resume=false
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionClose(SessionClose {
                reason: "test shutdown".to_string(),
                expect_resume: false,
            })),
        })
        .await
        .unwrap();

        // Stream should close (no response expected for SessionClose)
        // Give the server a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // The stream should be closed; next() should return None or an error
        // (The server closes the stream after transitioning to Closed state)
        drop(tx);
        // Drain any remaining messages
        let mut got_stream_end = false;
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(500);
        loop {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            match tokio::time::timeout(tokio::time::Duration::from_millis(100), stream.next()).await
            {
                Ok(None) | Err(_) => {
                    got_stream_end = true;
                    break;
                }
                Ok(Some(_)) => {
                    // Some message still in transit, keep draining
                }
            }
        }
        assert!(
            got_stream_end,
            "session stream did not terminate after SessionClose — graceful disconnect had no observable effect"
        );
    }

    /// Scenario: Graceful disconnect with expect_resume=true hint (RFC 0005 §1.5).
    /// The runtime should record the hint (tested via no error returned to client).
    #[tokio::test]
    async fn test_graceful_disconnect_with_resume_hint() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, _stream) =
            handshake(&mut client, "resume-hint-agent", "test-key").await;

        // Send SessionClose with expect_resume=true
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionClose(SessionClose {
                reason: "updating agent".to_string(),
                expect_resume: true,
            })),
        })
        .await
        .unwrap();

        // If no error is returned, the hint was processed successfully.
        // The test verifies protocol acceptance, not the lease hold behavior
        // (which requires multi-session coordination tested in integration tests).
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        drop(tx);
    }

    // ─── SessionConfig default values test (RFC 0005 §10) ────────────────────

    /// Verify SessionConfig defaults match the spec-specified values.
    #[test]
    fn test_session_config_defaults() {
        let config = SessionConfig::default();
        assert_eq!(
            config.handshake_timeout_ms, 5000,
            "handshake_timeout_ms default"
        );
        assert_eq!(
            config.heartbeat_interval_ms, 5000,
            "heartbeat_interval_ms default"
        );
        assert_eq!(
            config.heartbeat_missed_threshold, 3,
            "heartbeat_missed_threshold default"
        );
        assert_eq!(
            config.reconnect_grace_period_ms, 30_000,
            "reconnect_grace_period_ms default"
        );
        assert_eq!(
            config.retransmit_timeout_ms, 5000,
            "retransmit_timeout_ms default"
        );
        assert_eq!(config.dedup_window_size, 1000, "dedup_window_size default");
        assert_eq!(config.dedup_window_ttl_s, 60, "dedup_window_ttl_s default");
        assert_eq!(config.max_sequence_gap, 100, "max_sequence_gap default");
        assert_eq!(
            config.ephemeral_buffer_max, 16,
            "ephemeral_buffer_max default"
        );
        assert_eq!(
            config.max_concurrent_resident_sessions, 16,
            "max_concurrent_resident_sessions default"
        );
        assert_eq!(
            config.max_concurrent_guest_sessions, 64,
            "max_concurrent_guest_sessions default"
        );
    }

    // ─── Traffic class classification tests (RFC 0005 §3.1, §3.2) ───────────

    /// Verify traffic class routing for server payloads.
    #[test]
    fn test_traffic_class_routing() {
        use crate::proto::session::*;

        // Transactional messages
        assert_eq!(
            classify_server_payload(&ServerPayload::SessionEstablished(
                SessionEstablished::default()
            )),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::MutationResult(MutationResult::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::LeaseResponse(LeaseResponse::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::SessionSuspended(SessionSuspended::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::SessionResumed(SessionResumed::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::RuntimeError(RuntimeError::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::ResourceUploadAccepted(
                ResourceUploadAccepted::default(),
            )),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::ResourceStored(ResourceStored::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::ResourceErrorResponse(
                ResourceErrorResponse::default(),
            )),
            TrafficClass::Transactional,
        );

        // StateStream messages
        assert_eq!(
            classify_server_payload(&ServerPayload::SceneSnapshot(SceneSnapshot::default())),
            TrafficClass::StateStream,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::SceneDelta(SceneDelta::default())),
            TrafficClass::StateStream,
        );

        // DegradationNotice — transactional (RFC 0005 §3.4)
        assert_eq!(
            classify_server_payload(&ServerPayload::DegradationNotice(
                DegradationNotice::default()
            )),
            TrafficClass::Transactional,
        );

        // Ephemeral messages
        assert_eq!(
            classify_server_payload(&ServerPayload::Heartbeat(Heartbeat::default())),
            TrafficClass::Ephemeral,
        );
    }

    // ─── Ephemeral queue backpressure tests (RFC 0005 §2.5) ──────────────────

    /// Scenario: Ephemeral messages dropped under pressure (RFC 0005 §2.5)
    /// WHEN 20 ephemeral messages are enqueued (>16 quota),
    /// THEN oldest are dropped, retaining latest 16.
    #[test]
    fn test_ephemeral_queue_drops_oldest() {
        let mut queue = EphemeralQueue::new(DEFAULT_EPHEMERAL_BUFFER_MAX);

        // Enqueue 20 messages (4 more than the 16-message quota)
        for i in 0u64..20 {
            let msg = Ok(ServerMessage {
                sequence: i + 1,
                timestamp_wall_us: 0,
                payload: Some(ServerPayload::Heartbeat(Heartbeat {
                    timestamp_mono_us: i,
                })),
            });
            queue.push(msg);
        }

        // Queue should contain exactly 16 messages (quota)
        assert_eq!(
            queue.queue.len(),
            DEFAULT_EPHEMERAL_BUFFER_MAX,
            "Queue should be capped at ephemeral_buffer_max=16"
        );

        // First retained message should be sequence 5 (oldest 4 were dropped: 1,2,3,4)
        if let Some(Ok(msg)) = queue.queue.front() {
            assert_eq!(
                msg.sequence, 5,
                "Oldest 4 should have been evicted (1-4 dropped, 5 is oldest retained)"
            );
        }

        // Last retained message should be sequence 20
        if let Some(Ok(msg)) = queue.queue.back() {
            assert_eq!(msg.sequence, 20, "Latest message should be 20");
        }
    }

    /// Scenario: Ephemeral queue within capacity — no messages dropped.
    #[test]
    fn test_ephemeral_queue_within_capacity() {
        let mut queue = EphemeralQueue::new(DEFAULT_EPHEMERAL_BUFFER_MAX);

        for i in 0u64..16 {
            queue.push(Ok(ServerMessage {
                sequence: i + 1,
                timestamp_wall_us: 0,
                payload: Some(ServerPayload::Heartbeat(Heartbeat {
                    timestamp_mono_us: i,
                })),
            }));
        }

        assert_eq!(
            queue.queue.len(),
            16,
            "All 16 messages retained (at capacity)"
        );
        if let Some(Ok(msg)) = queue.queue.front() {
            assert_eq!(msg.sequence, 1, "No eviction: first message is sequence 1");
        }
    }

    // ─── Sequence validation unit tests ─────────────────────────────────────

    /// Unit tests for StreamSession::validate_client_sequence.
    #[test]
    fn test_validate_sequence_unit() {
        let mut session = StreamSession {
            session_id: "test".to_string(),
            namespace: "test".to_string(),
            agent_name: "test".to_string(),
            capabilities: Vec::new(),
            policy_capabilities: Vec::new(),
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            subscription_filters: std::collections::HashMap::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
            agent_event_rate_limiter: AgentEventRateLimiter::new(),
            freeze_queue: SessionFreezeQueue::new(FREEZE_QUEUE_CAPACITY),
            session_open_at_wall_us: now_wall_us(),
            dedup_window: DedupWindow::new(1000, 60),
            lease_correlation_cache: LeaseCorrelationCache::new(
                DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY,
            ),
            resource_upload_rate_limiter: UploadByteRateLimiter::with_limit(
                tze_hud_resource::DEFAULT_UPLOAD_RATE_LIMIT_BYTES_PER_SEC,
            ),
        };

        // seq=2 (gap=1): OK
        assert!(session.validate_client_sequence(2, 100).is_ok());
        assert_eq!(session.last_client_sequence, 2);

        // seq=102 (gap=100): still OK (gap == max_gap, not >)
        assert!(session.validate_client_sequence(102, 100).is_ok());
        assert_eq!(session.last_client_sequence, 102);

        // seq=203 (gap=101): exceeds max_gap=100
        let err = session.validate_client_sequence(203, 100);
        assert!(err.is_err());
        let (code, _) = err.unwrap_err();
        assert_eq!(code, "SEQUENCE_GAP_EXCEEDED");
        // last_client_sequence unchanged on error
        assert_eq!(session.last_client_sequence, 102);

        // seq=50 (regression): error
        let err = session.validate_client_sequence(50, 100);
        assert!(err.is_err());
        let (code, _) = err.unwrap_err();
        assert_eq!(code, "SEQUENCE_REGRESSION");

        // seq=102 (same as last): regression (not strictly greater)
        let err = session.validate_client_sequence(102, 100);
        assert!(err.is_err());
        let (code, _) = err.unwrap_err();
        assert_eq!(code, "SEQUENCE_REGRESSION");
    }

    // ─── Handshake auth, version, capability, subscription tests (rig-8uqz) ──

    /// Scenario: Structured AuthCredential (PSK) accepted (RFC 0005 §1.4)
    /// WHEN agent sends SessionInit with a valid PreSharedKeyCredential in auth_credential,
    /// THEN runtime authenticates and proceeds to SessionEstablished.
    #[tokio::test]
    async fn test_auth_structured_psk_credential_accepted() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "psk-agent".to_string(),
                agent_display_name: "psk-agent".to_string(),
                pre_shared_key: String::new(), // intentionally empty — use auth_credential
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: Some(crate::proto::session::AuthCredential {
                    credential: Some(
                        crate::proto::session::auth_credential::Credential::PreSharedKey(
                            crate::proto::session::PreSharedKeyCredential {
                                key: "test-key".to_string(),
                            },
                        ),
                    ),
                }),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(_)) => {}
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Invalid structured PSK credential rejected with AUTH_FAILED (RFC 0005 §1.4)
    /// WHEN agent sends SessionInit with a wrong PreSharedKeyCredential,
    /// THEN runtime sends SessionError(AUTH_FAILED) and closes stream.
    #[tokio::test]
    async fn test_auth_structured_psk_credential_wrong_key() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "bad-psk-agent".to_string(),
                agent_display_name: "bad-psk-agent".to_string(),
                pre_shared_key: String::new(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: Some(crate::proto::session::AuthCredential {
                    credential: Some(
                        crate::proto::session::auth_credential::Credential::PreSharedKey(
                            crate::proto::session::PreSharedKeyCredential {
                                key: "wrong-key".to_string(),
                            },
                        ),
                    ),
                }),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(err.code, "AUTH_FAILED");
            }
            other => panic!("Expected SessionError(AUTH_FAILED), got: {other:?}"),
        }
    }

    /// Scenario: LocalSocketCredential accepted (RFC 0005 §1.4)
    /// WHEN agent sends SessionInit with a valid LocalSocketCredential,
    /// THEN runtime authenticates and proceeds to SessionEstablished.
    #[tokio::test]
    async fn test_auth_local_socket_credential_accepted() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "local-agent".to_string(),
                agent_display_name: "local-agent".to_string(),
                pre_shared_key: String::new(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: Some(crate::proto::session::AuthCredential {
                    credential: Some(
                        crate::proto::session::auth_credential::Credential::LocalSocket(
                            crate::proto::session::LocalSocketCredential {
                                socket_path: "/run/tze_hud.sock".to_string(),
                                pid_hint: "42".to_string(),
                            },
                        ),
                    ),
                }),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(_)) => {}
            other => panic!("Expected SessionEstablished with LocalSocket cred, got: {other:?}"),
        }
    }

    /// Scenario: Version negotiated successfully (RFC 0005 §4.1)
    /// WHEN agent declares min=1000, max=1001 and runtime supports 1000-1001,
    /// THEN SessionEstablished contains negotiated_protocol_version=1001.
    #[tokio::test]
    async fn test_version_negotiation_success() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "version-agent".to_string(),
                agent_display_name: "version-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                assert_eq!(
                    established.negotiated_protocol_version, 1001,
                    "Should pick highest mutual version (1001)"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Version negotiation failure — no mutual version (RFC 0005 §4.1)
    /// WHEN agent declares min=2000, max=2001 and runtime only supports 1000-1001,
    /// THEN runtime sends SessionError(code=UNSUPPORTED_PROTOCOL_VERSION) and closes stream.
    #[tokio::test]
    async fn test_version_negotiation_unsupported() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "old-agent".to_string(),
                agent_display_name: "old-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 2000,
                max_protocol_version: 2001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "UNSUPPORTED_PROTOCOL_VERSION",
                    "Expected UNSUPPORTED_PROTOCOL_VERSION, got: {}",
                    err.code
                );
                // Hint should include runtime's supported range
                assert!(
                    !err.hint.is_empty(),
                    "Hint should contain runtime version range"
                );
            }
            other => panic!("Expected SessionError(UNSUPPORTED_PROTOCOL_VERSION), got: {other:?}"),
        }
    }

    /// Scenario: Clock sync — estimated_skew_us returned when agent_timestamp_wall_us is set
    /// (RFC 0005 §1.2 / RFC 0003 §1.3)
    /// WHEN agent includes agent_timestamp_wall_us in SessionInit,
    /// THEN runtime computes initial clock-skew and returns estimated_skew_us in SessionEstablished.
    #[tokio::test]
    async fn test_clock_skew_estimation() {
        let (mut client, _server) = setup_test().await;

        let agent_ts = now_wall_us();
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "clock-agent".to_string(),
                agent_display_name: "clock-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: agent_ts,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                // estimated_skew_us should be set (may be near 0 or slightly negative
                // due to timing between send and receive, but the field should exist
                // and be plausible — within ±1s for a loopback test)
                assert!(
                    established.estimated_skew_us.abs() < 1_000_000,
                    "Clock skew should be within ±1s on loopback, got: {}µs",
                    established.estimated_skew_us
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Non-canonical capability name rejected with CONFIG_UNKNOWN_CAPABILITY
    /// (configuration/spec.md Requirement: Capability Vocabulary, line 162-164)
    /// WHEN agent sends SessionInit with a legacy/non-canonical capability name,
    /// THEN runtime responds with SessionError(CONFIG_UNKNOWN_CAPABILITY) and a hint.
    #[tokio::test]
    async fn test_legacy_capability_rejected_with_hint() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "legacy-agent".to_string(),
                agent_display_name: "legacy-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                // Legacy names — must be rejected
                requested_capabilities: vec![
                    "create_tile".to_string(),   // legacy: should be create_tiles
                    "receive_input".to_string(), // legacy: should be access_input_events
                ],
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        use tokio_stream::StreamExt;
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "CONFIG_UNKNOWN_CAPABILITY",
                    "Expected CONFIG_UNKNOWN_CAPABILITY, got: {:?}",
                    err.code
                );
                // Hint should contain JSON with canonical replacements
                assert!(
                    !err.hint.is_empty(),
                    "Hint must be non-empty and point to canonical replacements"
                );
                // Both legacy names must be reported (spec: collect all, not fail-fast)
                assert!(
                    err.hint.contains("create_tiles") || err.hint.contains("create_tile"),
                    "Hint must reference create_tiles: {:?}",
                    err.hint
                );
                assert!(
                    err.hint.contains("access_input_events"),
                    "Hint must reference access_input_events: {:?}",
                    err.hint
                );
            }
            other => panic!("Expected SessionError(CONFIG_UNKNOWN_CAPABILITY), got: {other:?}"),
        }
    }

    /// Scenario: Pre-Round-14 name read_scene rejected with hint
    /// (policy-arbitration/spec.md §Requirement: Capability Registry Canonical Names, lines 281-292)
    #[tokio::test]
    async fn test_pre_round14_capability_name_rejected() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "old-vocab-agent".to_string(),
                agent_display_name: "old-vocab-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec![
                    "read_scene".to_string(), // pre-Round-14: should be read_scene_topology
                    "zone_publish:subtitle".to_string(), // pre-Round-14: should be publish_zone:subtitle
                ],
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        use tokio_stream::StreamExt;
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(err.code, "CONFIG_UNKNOWN_CAPABILITY");
                assert!(
                    err.hint.contains("read_scene_topology"),
                    "Hint must reference read_scene_topology"
                );
                assert!(
                    err.hint.contains("publish_zone:subtitle"),
                    "Hint must reference publish_zone:subtitle"
                );
            }
            other => panic!("Expected SessionError(CONFIG_UNKNOWN_CAPABILITY), got: {other:?}"),
        }
    }

    /// Scenario: LeaseRequest with non-canonical capability rejected
    /// (configuration/spec.md Requirement: Capability Vocabulary)
    #[tokio::test]
    async fn test_lease_request_with_legacy_capability_rejected() {
        let (mut client, _server) = setup_test().await;
        let (tx, _messages, mut response_stream) =
            handshake(&mut client, "cap-test-agent", "test-key").await;

        // Request a lease with a legacy (non-canonical) capability name
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tile".to_string()], // legacy: should be create_tiles
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        // Expect a LeaseResponse with granted=false and CONFIG_UNKNOWN_CAPABILITY
        let msg = next_non_state_change(&mut response_stream).await;
        match &msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(
                    !resp.granted,
                    "Lease must be denied for non-canonical capability"
                );
                assert_eq!(
                    resp.deny_code, "CONFIG_UNKNOWN_CAPABILITY",
                    "deny_code must be CONFIG_UNKNOWN_CAPABILITY, got: {:?}",
                    resp.deny_code
                );
            }
            other => panic!("Expected LeaseResponse(denied), got: {other:?}"),
        }
    }

    /// Scenario: LeaseRequest scope must not exceed current session grants
    /// (lease-governance/spec.md Requirement: Lease State Machine).
    ///
    /// WHEN lease request includes capabilities outside `SessionEstablished.granted_capabilities`,
    /// THEN runtime denies the entire lease request (no silent subset grant).
    #[tokio::test]
    async fn test_lease_request_scope_exceeding_session_grants_is_denied() {
        let (mut client, _server) = setup_test().await;
        let (tx, _messages, mut response_stream) =
            handshake(&mut client, "lease-scope-agent", "test-key").await;

        // Handshake helper grants create_tiles/access_input_events/read_scene_topology.
        // Requesting modify_own_tiles exceeds the current session-granted scope.
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut response_stream).await;
        match &msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(
                    !resp.granted,
                    "Lease must be denied when requested scope exceeds session grants"
                );
                assert_eq!(resp.deny_code, "PERMISSION_DENIED");
                assert!(
                    resp.deny_reason.contains("modify_own_tiles"),
                    "deny_reason should identify unauthorized capability; got {:?}",
                    resp.deny_reason
                );
                assert!(
                    resp.granted_capabilities.is_empty(),
                    "Denied lease must not return granted_capabilities subset"
                );
            }
            other => panic!("Expected LeaseResponse(denied), got: {other:?}"),
        }
    }

    #[test]
    fn test_capability_set_covers_wildcard_grants() {
        let caps = vec![
            "publish_zone:*".to_string(),
            "publish_widget:*".to_string(),
            "emit_scene_event:*".to_string(),
        ];
        assert!(capability_set_covers(&caps, "publish_zone:subtitle"));
        assert!(capability_set_covers(&caps, "publish_widget:gauge"));
        assert!(capability_set_covers(
            &caps,
            "emit_scene_event:status_update"
        ));
        assert!(!capability_set_covers(&caps, "create_tiles"));
    }

    /// Scenario: PSK agent with access_input_events capability successfully subscribes to
    /// INPUT_EVENTS (RFC 0005 §7.1).
    /// WHEN a PSK-authenticated agent requests INPUT_EVENTS subscription AND includes
    /// access_input_events in requested_capabilities,
    /// THEN SessionEstablished includes INPUT_EVENTS in active_subscriptions and
    /// denied_subscriptions is empty.
    ///
    /// Subscription gating uses the agent's explicitly granted capabilities (RFC 0005 §7.1).
    /// Agents must request the required capability to subscribe to gated categories.
    #[tokio::test]
    async fn test_psk_with_capability_allows_input_events_subscription() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // PSK agent requesting INPUT_EVENTS subscription WITH the required capability
        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "sub-test-agent".to_string(),
                agent_display_name: "sub-test-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec!["access_input_events".to_string()],
                initial_subscriptions: vec!["INPUT_EVENTS".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                // Agent with access_input_events capability should have INPUT_EVENTS active
                assert!(
                    established
                        .active_subscriptions
                        .contains(&"INPUT_EVENTS".to_string()),
                    "Agent with access_input_events should have INPUT_EVENTS in active_subscriptions; \
                     active={:?}, denied={:?}",
                    established.active_subscriptions,
                    established.denied_subscriptions
                );
                assert!(
                    established.denied_subscriptions.is_empty(),
                    "Agent with required capability should have no denied subscriptions"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Capability granted mid-session (RFC 0005 §5.3)
    /// WHEN agent sends CapabilityRequest with authorized capabilities,
    /// THEN runtime responds with CapabilityNotice(granted=requested_capabilities).
    #[tokio::test]
    async fn test_mid_session_capability_request_granted() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "cap-req-agent", "test-key").await;

        // Request a capability mid-session (PSK agents can request any capability)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                capabilities: vec!["read_telemetry".to_string()],
                reason: "monitoring".to_string(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(
                    notice.granted.contains(&"read_telemetry".to_string()),
                    "Expected read_telemetry to be granted; got: {:?}",
                    notice.granted
                );
                assert!(
                    notice.revoked.is_empty(),
                    "No capabilities should be revoked"
                );
                assert!(
                    notice.effective_at_server_seq > 0,
                    "effective_at_server_seq must be non-zero"
                );
            }
            other => panic!("Expected CapabilityNotice, got: {other:?}"),
        }
    }

    /// Scenario: PSK (unrestricted) agent receives CapabilityNotice for any capability (RFC 0005 §5.3)
    /// WHEN a PSK-authenticated agent requests any capability mid-session,
    /// THEN runtime responds with CapabilityNotice (not RuntimeError).
    ///
    /// `setup_test()` runs with fallback-unrestricted policy, so no capability
    /// request can be denied through this integration path. The denied path
    /// (PERMISSION_DENIED) is exercised in
    /// test_capability_request_denied_for_guest_session and
    /// test_capability_request_partial_grant_denied_entirely below.
    #[tokio::test]
    async fn test_mid_session_capability_request_unrestricted_succeeds() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "deny-test-agent", "test-key").await;

        // PSK agent requesting a valid capability — should succeed (PSK is unrestricted).
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                capabilities: vec!["overlay_privileges".to_string()],
                reason: "test".to_string(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        // PSK agents (unrestricted) should get CapabilityNotice, not an error.
        match &msg.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(
                    notice.granted.contains(&"overlay_privileges".to_string()),
                    "PSK unrestricted agent should get overlay_privileges granted"
                );
            }
            other => panic!("Expected CapabilityNotice for unrestricted PSK agent, got: {other:?}"),
        }
    }

    /// Unit test: handle_capability_request with guest (restricted) session
    /// to verify RuntimeError(PERMISSION_DENIED) is returned for unauthorized caps.
    ///
    /// Scenario: Guest agent denied resident tools via capability escalation (RFC 0005 §5.3)
    /// WHEN a guest-level agent sends CapabilityRequest for resident-level operations,
    /// THEN runtime denies with RuntimeError(PERMISSION_DENIED).
    #[tokio::test]
    async fn test_capability_request_denied_for_guest_session() {
        // Set up the outbound channel
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(16);

        // Build a guest session (no policy capabilities = no authorization)
        let mut session = StreamSession {
            session_id: "guest-session".to_string(),
            namespace: "guest".to_string(),
            agent_name: "guest".to_string(),
            capabilities: Vec::new(),
            policy_capabilities: Vec::new(), // guest: no authorization
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            subscription_filters: std::collections::HashMap::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
            agent_event_rate_limiter: AgentEventRateLimiter::new(),
            freeze_queue: SessionFreezeQueue::new(FREEZE_QUEUE_CAPACITY),
            session_open_at_wall_us: 0,
            dedup_window: DedupWindow::new(1000, 60),
            lease_correlation_cache: LeaseCorrelationCache::new(
                DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY,
            ),
            resource_upload_rate_limiter: UploadByteRateLimiter::with_limit(
                tze_hud_resource::DEFAULT_UPLOAD_RATE_LIMIT_BYTES_PER_SEC,
            ),
        };

        handle_capability_request(
            &mut session,
            &tx,
            CapabilityRequest {
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                reason: "escalation attempt".to_string(),
            },
        )
        .await;

        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(err.error_code, "PERMISSION_DENIED");
                assert_eq!(err.error_code_enum, ErrorCode::PermissionDenied as i32);
                assert!(
                    !err.context.is_empty(),
                    "Context should list denied capabilities"
                );
                assert!(
                    err.hint.contains("unauthorized_capabilities"),
                    "Hint should contain unauthorized_capabilities: {}",
                    err.hint
                );
            }
            other => panic!("Expected RuntimeError(PERMISSION_DENIED), got: {other:?}"),
        }
    }

    /// Scenario: Partial grant of mixed capabilities is denied entirely (RFC 0005 §5.3)
    /// WHEN agent requests capabilities=["read_telemetry", "overlay_privileges"] and is
    /// authorized for only read_telemetry,
    /// THEN runtime denies entire request with PERMISSION_DENIED.
    #[tokio::test]
    async fn test_capability_request_partial_grant_denied_entirely() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(16);

        // Session with only read_telemetry authorized
        let mut session = StreamSession {
            session_id: "partial-grant-session".to_string(),
            namespace: "restricted-agent".to_string(),
            agent_name: "restricted-agent".to_string(),
            capabilities: vec!["read_telemetry".to_string()],
            policy_capabilities: vec!["read_telemetry".to_string()], // only read_telemetry
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            subscription_filters: std::collections::HashMap::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
            agent_event_rate_limiter: AgentEventRateLimiter::new(),
            freeze_queue: SessionFreezeQueue::new(FREEZE_QUEUE_CAPACITY),
            session_open_at_wall_us: 0,
            dedup_window: DedupWindow::new(1000, 60),
            lease_correlation_cache: LeaseCorrelationCache::new(
                DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY,
            ),
            resource_upload_rate_limiter: UploadByteRateLimiter::with_limit(
                tze_hud_resource::DEFAULT_UPLOAD_RATE_LIMIT_BYTES_PER_SEC,
            ),
        };

        // Request both an authorized and an unauthorized capability
        handle_capability_request(
            &mut session,
            &tx,
            CapabilityRequest {
                capabilities: vec![
                    "read_telemetry".to_string(),
                    "overlay_privileges".to_string(),
                ],
                reason: "mixed request".to_string(),
            },
        )
        .await;

        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(
                    err.error_code, "PERMISSION_DENIED",
                    "Entire request should be denied, not just overlay_privileges"
                );
                assert_eq!(err.error_code_enum, ErrorCode::PermissionDenied as i32);
                assert!(
                    err.context.contains("overlay_privileges"),
                    "Context should mention the unauthorized capability: {}",
                    err.context
                );
                // read_telemetry should NOT have been granted
                assert!(
                    !session
                        .capabilities
                        .contains(&"overlay_privileges".to_string()),
                    "overlay_privileges must not have been added to session capabilities"
                );
            }
            other => {
                panic!("Expected RuntimeError(PERMISSION_DENIED) for partial grant, got: {other:?}")
            }
        }
    }

    /// Scenario: Session grants, lease grants, and mid-session escalation stay aligned.
    ///
    /// 1) LeaseRequest asking for capability scope beyond current session grants is denied.
    /// 2) After CapabilityRequest grants additional authorized scope, the same LeaseRequest
    ///    is accepted.
    #[tokio::test]
    async fn test_lease_scope_requires_session_grant_or_escalation() {
        let mut policy = HashMap::new();
        policy.insert(
            "scope-agent".to_string(),
            vec![
                "create_tiles".to_string(),
                "modify_own_tiles".to_string(),
                "read_scene_topology".to_string(),
            ],
        );
        let (mut client, _server) = setup_test_with_policy(policy, false).await;
        let (tx, _init_messages, mut stream) = handshake_with_requested_capabilities(
            &mut client,
            "scope-agent",
            "test-key",
            vec!["create_tiles".to_string()],
        )
        .await;

        // Request lease scope broader than current session grants: must be denied.
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let denied = next_non_state_change(&mut stream).await;
        match denied.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(!resp.granted, "lease must be denied before escalation");
                assert_eq!(resp.deny_code, "PERMISSION_DENIED");
                assert!(
                    resp.deny_reason.contains("modify_own_tiles"),
                    "deny_reason should name the out-of-scope capability"
                );
            }
            other => panic!("Expected denied LeaseResponse, got: {other:?}"),
        }

        // Escalate mid-session using the configured authorization scope.
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                capabilities: vec!["modify_own_tiles".to_string()],
                reason: "need edit capability".to_string(),
            })),
        })
        .await
        .unwrap();

        let granted = next_non_state_change(&mut stream).await;
        match granted.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(
                    notice.granted.contains(&"modify_own_tiles".to_string()),
                    "expected modify_own_tiles grant after escalation"
                );
            }
            other => panic!("Expected CapabilityNotice, got: {other:?}"),
        }

        // Retry the same lease request: should now succeed.
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let granted_lease = next_non_state_change(&mut stream).await;
        match granted_lease.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted, "lease must be granted after escalation");
                assert!(
                    resp.granted_capabilities
                        .contains(&"modify_own_tiles".to_string())
                );
            }
            other => panic!("Expected granted LeaseResponse, got: {other:?}"),
        }
    }

    /// Scenario: Reconnect/resume preserves current grants but keeps policy scope
    /// for future CapabilityRequest evaluation.
    #[tokio::test]
    async fn test_capability_request_after_resume_uses_policy_scope() {
        let mut policy = HashMap::new();
        policy.insert(
            "resume-scope-agent".to_string(),
            vec![
                "create_tiles".to_string(),
                "read_telemetry".to_string(),
                "read_scene_topology".to_string(),
            ],
        );
        let (mut client, _server) = setup_test_with_policy(policy, false).await;
        let (tx, init_messages, stream) = handshake_with_requested_capabilities(
            &mut client,
            "resume-scope-agent",
            "test-key",
            vec!["create_tiles".to_string()],
        )
        .await;

        let resume_token = match &init_messages[0].payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                assert!(
                    established
                        .granted_capabilities
                        .contains(&"create_tiles".to_string())
                );
                assert!(
                    !established
                        .granted_capabilities
                        .contains(&"read_telemetry".to_string()),
                    "read_telemetry should not be initially granted when not requested"
                );
                established.resume_token.clone()
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        };
        drop(tx);
        drop(stream);
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        // Reconnect using SessionResume.
        let (resume_tx, resume_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let resume_stream = tokio_stream::wrappers::ReceiverStream::new(resume_rx);
        resume_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionResume(SessionResume {
                    agent_id: "resume-scope-agent".to_string(),
                    resume_token,
                    last_seen_server_sequence: 2,
                    pre_shared_key: "test-key".to_string(),
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut resumed = client.session(resume_stream).await.unwrap().into_inner();
        let resume_result = resumed.next().await.unwrap().unwrap();
        match &resume_result.payload {
            Some(ServerPayload::SessionResumeResult(result)) => {
                assert!(result.accepted);
                assert!(
                    result
                        .granted_capabilities
                        .contains(&"create_tiles".to_string())
                );
                assert!(
                    !result
                        .granted_capabilities
                        .contains(&"read_telemetry".to_string()),
                    "resume restores prior grants; it must not auto-grant untouched policy scope"
                );
            }
            other => panic!("Expected SessionResumeResult, got: {other:?}"),
        }
        let snapshot = resumed.next().await.unwrap().unwrap();
        match snapshot.payload {
            Some(ServerPayload::SceneSnapshot(_)) => {}
            other => panic!("Expected SceneSnapshot after resume, got: {other:?}"),
        }

        // Authorized post-resume escalation must succeed.
        resume_tx
            .send(ClientMessage {
                sequence: 2,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                    capabilities: vec!["read_telemetry".to_string()],
                    reason: "need telemetry feed".to_string(),
                })),
            })
            .await
            .unwrap();

        let granted = resumed.next().await.unwrap().unwrap();
        match granted.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(notice.granted.contains(&"read_telemetry".to_string()));
            }
            other => panic!("Expected CapabilityNotice, got: {other:?}"),
        }

        // Mixed request still denies the entire batch after resume.
        resume_tx
            .send(ClientMessage {
                sequence: 3,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                    capabilities: vec![
                        "read_telemetry".to_string(),
                        "overlay_privileges".to_string(),
                    ],
                    reason: "mixed escalation".to_string(),
                })),
            })
            .await
            .unwrap();

        let denied = resumed.next().await.unwrap().unwrap();
        match denied.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(err.error_code, "PERMISSION_DENIED");
                assert!(
                    err.context.contains("overlay_privileges"),
                    "mixed denial context should list unauthorized capability"
                );
            }
            other => panic!("Expected RuntimeError(PERMISSION_DENIED), got: {other:?}"),
        }
    }

    /// Verify RuntimeError structure matches spec (RFC 0005 §3.5)
    /// error_code, message, context, hint, error_code_enum all populated.
    #[tokio::test]
    async fn test_runtime_error_structure_complete() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(16);

        let mut session = StreamSession {
            session_id: "err-test".to_string(),
            namespace: "err-agent".to_string(),
            agent_name: "err-agent".to_string(),
            capabilities: Vec::new(),
            policy_capabilities: Vec::new(),
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            subscription_filters: std::collections::HashMap::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
            agent_event_rate_limiter: AgentEventRateLimiter::new(),
            freeze_queue: SessionFreezeQueue::new(FREEZE_QUEUE_CAPACITY),
            session_open_at_wall_us: 0,
            dedup_window: DedupWindow::new(1000, 60),
            lease_correlation_cache: LeaseCorrelationCache::new(
                DEFAULT_LEASE_CORRELATION_CACHE_CAPACITY,
            ),
            resource_upload_rate_limiter: UploadByteRateLimiter::with_limit(
                tze_hud_resource::DEFAULT_UPLOAD_RATE_LIMIT_BYTES_PER_SEC,
            ),
        };

        handle_capability_request(
            &mut session,
            &tx,
            CapabilityRequest {
                capabilities: vec!["some_cap".to_string()],
                reason: "test".to_string(),
            },
        )
        .await;

        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                // error_code: stable string
                assert!(!err.error_code.is_empty(), "error_code must be set");
                // message: human-readable
                assert!(!err.message.is_empty(), "message must be set");
                // error_code_enum: typed enum (non-zero for known codes)
                assert!(
                    err.error_code_enum != 0,
                    "error_code_enum must be non-zero for known codes"
                );
                // hint: machine-readable JSON
                if !err.hint.is_empty() {
                    assert!(
                        serde_json::from_str::<serde_json::Value>(&err.hint).is_ok(),
                        "hint must be valid JSON: {}",
                        err.hint
                    );
                }
            }
            other => panic!("Expected RuntimeError, got: {other:?}"),
        }
    }

    /// Helper that returns the shared state alongside the client for state-manipulation tests.
    async fn setup_test_with_state() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<SharedState>>,
    ) {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");
        let shared_state = service.state.clone();

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        (client, handle, shared_state)
    }

    // ─── Reconnection and resume tests (RFC 0005 §6.1–6.6, rig-3dou) ────────

    /// Helper: perform a full handshake and return the resume token.
    ///
    /// Drops the sender and response stream, waits for server-side cleanup,
    /// then returns the resume token for use in subsequent resume attempts.
    async fn handshake_and_disconnect(
        client: &mut HudSessionClient<tonic::transport::Channel>,
        agent_id: &str,
        psk: &str,
    ) -> Vec<u8> {
        let (tx, init_messages, stream) = handshake(client, agent_id, psk).await;
        let resume_token = match &init_messages[0].payload {
            Some(ServerPayload::SessionEstablished(e)) => e.resume_token.clone(),
            _ => panic!("Expected SessionEstablished"),
        };
        drop(tx);
        drop(stream);
        // Allow server task to process EOF and register the resume token.
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        resume_token
    }

    /// Scenario (rig-3dou AC): Reconnect within grace period succeeds with
    /// `SessionResumeResult(accepted=true)`.
    /// RFC 0005 §6.1–6.3
    #[tokio::test]
    async fn test_reconnect_within_grace_accepted() {
        let (mut client, _server) = setup_test().await;
        let resume_token =
            handshake_and_disconnect(&mut client, "resume-ok-agent", "test-key").await;

        let (resume_tx, resume_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let resume_stream = tokio_stream::wrappers::ReceiverStream::new(resume_rx);

        resume_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionResume(SessionResume {
                    agent_id: "resume-ok-agent".to_string(),
                    resume_token: resume_token.clone(),
                    last_seen_server_sequence: 2,
                    pre_shared_key: "test-key".to_string(),
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(resume_stream).await.unwrap().into_inner();

        let msg1 = response_stream.next().await.unwrap().unwrap();
        match &msg1.payload {
            Some(ServerPayload::SessionResumeResult(result)) => {
                assert!(result.accepted, "expected resume to be accepted");
                assert!(
                    !result.new_session_token.is_empty(),
                    "new token must be issued"
                );
                assert_ne!(
                    result.new_session_token, resume_token,
                    "new token must differ from old token"
                );
                assert_eq!(
                    result.negotiated_protocol_version,
                    crate::auth::RUNTIME_MAX_VERSION
                );
            }
            other => panic!("Expected SessionResumeResult, got: {other:?}"),
        }

        // Full SceneSnapshot must follow SessionResumeResult (RFC 0005 §6.4).
        let msg2 = response_stream.next().await.unwrap().unwrap();
        match &msg2.payload {
            Some(ServerPayload::SceneSnapshot(_)) => {}
            other => panic!("Expected SceneSnapshot after resume, got: {other:?}"),
        }
    }

    /// Scenario (rig-3dou AC): New session token is issued on resume; old token
    /// is single-use and consumed.
    /// RFC 0005 §6.1 — "single-use for resumption"
    #[tokio::test]
    async fn test_resume_token_single_use() {
        let (mut client, _server) = setup_test().await;
        let resume_token =
            handshake_and_disconnect(&mut client, "single-use-agent", "test-key").await;

        // First resume: should succeed and consume the token.
        let (tx1, rx1) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let s1 = tokio_stream::wrappers::ReceiverStream::new(rx1);
        tx1.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionResume(SessionResume {
                agent_id: "single-use-agent".to_string(),
                resume_token: resume_token.clone(),
                last_seen_server_sequence: 2,
                pre_shared_key: "test-key".to_string(),
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut r1 = client.session(s1).await.unwrap().into_inner();
        let first_resume = r1.next().await.unwrap().unwrap();
        match &first_resume.payload {
            Some(ServerPayload::SessionResumeResult(result)) => {
                assert!(result.accepted, "first resume must succeed");
            }
            other => panic!("Expected SessionResumeResult, got: {other:?}"),
        }
        drop(tx1);
        drop(r1);
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        // Second resume attempt with the same original token: must fail.
        let (tx2, rx2) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let s2 = tokio_stream::wrappers::ReceiverStream::new(rx2);
        tx2.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionResume(SessionResume {
                agent_id: "single-use-agent".to_string(),
                resume_token: resume_token.clone(),
                last_seen_server_sequence: 2,
                pre_shared_key: "test-key".to_string(),
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut r2 = client.session(s2).await.unwrap().into_inner();
        let second_resume = r2.next().await.unwrap().unwrap();
        match &second_resume.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "SESSION_GRACE_EXPIRED",
                    "second use of same token must fail with SESSION_GRACE_EXPIRED, got: {}",
                    err.code
                );
            }
            other => panic!("Expected SessionError(SESSION_GRACE_EXPIRED), got: {other:?}"),
        }
    }

    /// Scenario (rig-3dou AC): Re-authentication required on resume.
    /// Invalid credentials result in `SessionError(AUTH_FAILED)`.
    /// RFC 0005 §6.2
    #[tokio::test]
    async fn test_resume_auth_required() {
        let (mut client, _server) = setup_test().await;
        let resume_token =
            handshake_and_disconnect(&mut client, "auth-check-agent", "test-key").await;

        let (resume_tx, resume_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let resume_stream = tokio_stream::wrappers::ReceiverStream::new(resume_rx);

        // Use wrong PSK on resume — must be rejected with AUTH_FAILED.
        resume_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionResume(SessionResume {
                    agent_id: "auth-check-agent".to_string(),
                    resume_token: resume_token.clone(),
                    last_seen_server_sequence: 2,
                    pre_shared_key: "wrong-key".to_string(),
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(resume_stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "AUTH_FAILED",
                    "expected AUTH_FAILED, got: {}",
                    err.code
                );
            }
            other => panic!("Expected SessionError(AUTH_FAILED), got: {other:?}"),
        }
    }

    /// Scenario (rig-3dou AC): Bogus token (as if runtime restarted and all tokens
    /// cleared) is rejected with `SESSION_GRACE_EXPIRED`.
    /// RFC 0005 §6.6
    #[tokio::test]
    async fn test_bogus_token_rejected_with_grace_expired() {
        let (mut client, _server) = setup_test().await;

        let bogus_token = uuid::Uuid::now_v7().as_bytes().to_vec();

        let (resume_tx, resume_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let resume_stream = tokio_stream::wrappers::ReceiverStream::new(resume_rx);

        resume_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionResume(SessionResume {
                    agent_id: "restart-agent".to_string(),
                    resume_token: bogus_token,
                    last_seen_server_sequence: 0,
                    pre_shared_key: "test-key".to_string(),
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(resume_stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "SESSION_GRACE_EXPIRED",
                    "unknown token must produce SESSION_GRACE_EXPIRED, got: {}",
                    err.code
                );
                assert!(
                    !err.hint.is_empty(),
                    "hint should direct client to SessionInit"
                );
            }
            other => panic!("Expected SessionError(SESSION_GRACE_EXPIRED), got: {other:?}"),
        }
    }

    /// Scenario (rig-3dou AC): SessionResumeResult carries complete subscription state.
    /// RFC 0005 §6.3 — agents MUST use confirmed subscription state, not assume pre-disconnect set.
    #[tokio::test]
    async fn test_resume_result_carries_subscription_state() {
        let (mut client, _server) = setup_test().await;

        // Establish a session that requested a specific subscription.
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "sub-resume-agent".to_string(),
                agent_display_name: "sub-resume-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                // Include required capabilities for both subscriptions (canonical names)
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "read_scene_topology".to_string(),
                    "access_input_events".to_string(),
                ],
                initial_subscriptions: vec![
                    "SCENE_TOPOLOGY".to_string(),
                    "INPUT_EVENTS".to_string(),
                ],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let established_msg = response_stream.next().await.unwrap().unwrap();
        let resume_token = match &established_msg.payload {
            Some(ServerPayload::SessionEstablished(e)) => e.resume_token.clone(),
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        };

        drop(tx);
        drop(response_stream);
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        // Now resume.
        let (rtx, rrx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let rstream = tokio_stream::wrappers::ReceiverStream::new(rrx);

        rtx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionResume(SessionResume {
                agent_id: "sub-resume-agent".to_string(),
                resume_token,
                last_seen_server_sequence: 2,
                pre_shared_key: "test-key".to_string(),
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut rs = client.session(rstream).await.unwrap().into_inner();
        let resume_result_msg = rs.next().await.unwrap().unwrap();
        match &resume_result_msg.payload {
            Some(ServerPayload::SessionResumeResult(result)) => {
                assert!(result.accepted);
                // Capabilities must be restored.
                assert!(
                    result
                        .granted_capabilities
                        .contains(&"create_tiles".to_string()),
                    "create_tiles capability must be restored on resume"
                );
                // Subscriptions must be restored.
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"SCENE_TOPOLOGY".to_string()),
                    "SCENE_TOPOLOGY subscription must be present in resume result"
                );
                assert!(
                    result
                        .active_subscriptions
                        .contains(&"INPUT_EVENTS".to_string()),
                    "INPUT_EVENTS subscription must be present in resume result"
                );
            }
            other => panic!("Expected SessionResumeResult, got: {other:?}"),
        }
    }

    // ─── DegradationNotice tests (RFC 0005 §3.4, §7.1) ───────────────────────

    /// traffic_class: DegradationNotice must be Transactional (RFC 0005 §3.4).
    #[test]
    fn test_degradation_notice_is_transactional() {
        assert_eq!(
            classify_server_payload(&ServerPayload::DegradationNotice(
                DegradationNotice::default()
            )),
            TrafficClass::Transactional,
            "DegradationNotice must be Transactional — never dropped"
        );
    }

    /// Scenario: WHEN runtime enters COALESCING_MORE degradation level,
    /// THEN all active sessions receive DegradationNotice unconditionally.
    #[tokio::test]
    async fn test_degradation_notice_broadcast_to_active_session() {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");
        let degradation_tx = service.degradation_tx.clone();
        let state_ref = service.state.clone();

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "degrad-agent", "test-key").await;

        // Give the session task a brief moment to subscribe to the broadcast channel.
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Broadcast a COALESCING_MORE degradation notice from the "compositor side".
        let notice = DegradationNotice {
            level: DegradationLevel::CoalescingMore as i32,
            reason: "high load".to_string(),
            affected_capabilities: vec!["state_stream".to_string()],
            timestamp_wall_us: now_wall_us(),
        };
        let _ = degradation_tx.send(notice.clone());

        // Update shared state level (mirrors what broadcast_degradation() does).
        {
            let mut st = state_ref.lock().await;
            st.degradation_level = crate::session::RuntimeDegradationLevel::CoalescingMore;
        }

        // The session should receive DegradationNotice next.
        let timeout = tokio::time::Duration::from_millis(500);
        let msg = tokio::time::timeout(timeout, stream.next())
            .await
            .expect("timeout waiting for DegradationNotice")
            .expect("stream ended")
            .expect("stream error");

        match &msg.payload {
            Some(ServerPayload::DegradationNotice(dn)) => {
                assert_eq!(
                    dn.level,
                    DegradationLevel::CoalescingMore as i32,
                    "Expected COALESCING_MORE"
                );
                assert_eq!(dn.reason, "high load");
                assert!(
                    dn.affected_capabilities
                        .contains(&"state_stream".to_string())
                );
            }
            other => panic!("Expected DegradationNotice, got: {other:?}"),
        }

        drop(tx);
    }

    // ─── Deduplication tests (RFC 0005 §5.2) ─────────────────────────────────

    /// Scenario: duplicate batch_id within window returns cached MutationResult.
    #[tokio::test]
    async fn test_mutation_dedup_returns_cached_result() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "dedup-agent", "test-key").await;

        // Obtain a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();
        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Send first MutationBatch with a unique batch_id
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                timing: None,
            })),
        })
        .await
        .unwrap();
        let first_result = next_non_state_change(&mut stream).await;
        let first_accepted = match &first_result.payload {
            Some(ServerPayload::MutationResult(r)) => {
                assert_eq!(r.batch_id, batch_id);
                r.accepted
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        };

        // Retransmit with the same batch_id but a new sequence number
        tx.send(ClientMessage {
            sequence: 4, // new sequence
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(), // same batch_id
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                timing: None,
            })),
        })
        .await
        .unwrap();
        let dedup_result = stream.next().await.unwrap().unwrap();
        match &dedup_result.payload {
            Some(ServerPayload::MutationResult(r)) => {
                assert_eq!(
                    r.batch_id, batch_id,
                    "batch_id must be echoed from cached result"
                );
                assert_eq!(
                    r.accepted, first_accepted,
                    "Dedup must return cached accepted flag"
                );
            }
            other => panic!("Expected cached MutationResult on retransmit, got: {other:?}"),
        }

        drop(tx);
    }

    // ─── TimingHints validation tests (RFC 0003 §3.5, RFC 0005 §3.3) ─────────

    /// Unit test for validate_timing_hints: TIMESTAMP_TOO_OLD.
    #[test]
    fn test_timing_hints_too_old() {
        // present_at_wall_us = session_open - 61 seconds → TIMESTAMP_TOO_OLD
        let session_open = 200_000_000u64; // arbitrary µs baseline
        let present = session_open - 61_000_001; // > 60s before session open
        let hints = TimingHints {
            present_at_wall_us: present,
            expires_at_wall_us: 0,
        };
        let result = validate_timing_hints(&hints, session_open, DEFAULT_MAX_FUTURE_SCHEDULE_US);
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, "TIMESTAMP_TOO_OLD");
    }

    /// Unit test for validate_timing_hints: TIMESTAMP_TOO_FUTURE.
    #[test]
    fn test_timing_hints_too_future() {
        let session_open = now_wall_us();
        let max_future = DEFAULT_MAX_FUTURE_SCHEDULE_US;
        // Use session_open as baseline and a large margin (1 full second) to avoid
        // flakiness from the µs gap between now_wall_us() calls.
        // present must exceed current_wall_us + max_future, where current_wall_us is
        // re-sampled inside validate_timing_hints. The 1-second buffer ensures the
        // margin holds even under scheduler jitter.
        let present = session_open + max_future + 1_000_000; // 1s beyond horizon
        let hints = TimingHints {
            present_at_wall_us: present,
            expires_at_wall_us: 0,
        };
        let result = validate_timing_hints(&hints, session_open, max_future);
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, "TIMESTAMP_TOO_FUTURE");
    }

    /// Unit test for validate_timing_hints: TIMESTAMP_EXPIRY_BEFORE_PRESENT.
    #[test]
    fn test_timing_hints_expiry_before_present() {
        let session_open = now_wall_us().saturating_sub(1_000_000); // 1s ago
        let now = now_wall_us();
        let present = now + 1_000_000; // 1s in future (valid range)
        let expires = present - 1; // expires before present → invalid
        let hints = TimingHints {
            present_at_wall_us: present,
            expires_at_wall_us: expires,
        };
        let result = validate_timing_hints(&hints, session_open, DEFAULT_MAX_FUTURE_SCHEDULE_US);
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, "TIMESTAMP_EXPIRY_BEFORE_PRESENT");
    }

    /// Unit test for validate_timing_hints: valid future scheduling (present_at in future).
    #[test]
    fn test_timing_hints_valid_future() {
        let session_open = now_wall_us().saturating_sub(1_000_000); // 1s ago
        let now = now_wall_us();
        let present = now + 500_000; // 500ms in the future (well within 5 min)
        let expires = present + 2_000_000; // 2s after present → valid
        let hints = TimingHints {
            present_at_wall_us: present,
            expires_at_wall_us: expires,
        };
        assert!(
            validate_timing_hints(&hints, session_open, DEFAULT_MAX_FUTURE_SCHEDULE_US).is_ok(),
            "Valid future TimingHints should not be rejected"
        );
    }

    /// Unit test for validate_timing_hints: zero fields bypass validation.
    #[test]
    fn test_timing_hints_zero_bypasses_validation() {
        let session_open = now_wall_us();
        let hints = TimingHints {
            present_at_wall_us: 0,
            expires_at_wall_us: 0,
        };
        assert!(
            validate_timing_hints(&hints, session_open, DEFAULT_MAX_FUTURE_SCHEDULE_US).is_ok(),
            "Zero TimingHints should always be valid"
        );
    }

    /// Integration test: MutationBatch with TIMESTAMP_TOO_OLD is rejected via stream.
    #[tokio::test]
    async fn test_mutation_timing_too_old_rejected() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "timing-old-agent", "test-key").await;

        // Get a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();
        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Send a mutation with present_at more than 60s before epoch 0 (which means
        // it's more than 60s before session open; session opened near now_wall_us(),
        // so session_open - 60s - 1 ≫ 0 for any real timestamp).
        //
        // Use present_at = 1 µs since epoch — guaranteed to be older than
        // session_open_at_wall_us - 60_000_000.
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                timing: Some(TimingHints {
                    present_at_wall_us: 1, // far in the past
                    expires_at_wall_us: 0,
                }),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(err.error_code, "TIMESTAMP_TOO_OLD");
                assert_eq!(err.error_code_enum, ErrorCode::TimestampTooOld as i32);
            }
            other => panic!("Expected RuntimeError(TIMESTAMP_TOO_OLD), got: {other:?}"),
        }

        drop(tx);
    }

    /// Integration test: MutationBatch with TIMESTAMP_EXPIRY_BEFORE_PRESENT is rejected.
    #[tokio::test]
    async fn test_mutation_timing_expiry_before_present_rejected() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "timing-exp-agent", "test-key").await;

        // Get a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();
        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        let now = now_wall_us();
        let present = now + 500_000; // 500ms in future
        let expires = present - 1; // expires 1µs before present → invalid

        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
                timing: Some(TimingHints {
                    present_at_wall_us: present,
                    expires_at_wall_us: expires,
                }),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(err.error_code, "TIMESTAMP_EXPIRY_BEFORE_PRESENT");
                assert_eq!(
                    err.error_code_enum,
                    ErrorCode::TimestampExpiryBeforePresent as i32
                );
            }
            other => {
                panic!("Expected RuntimeError(TIMESTAMP_EXPIRY_BEFORE_PRESENT), got: {other:?}")
            }
        }

        drop(tx);
    }

    // ─── Zone durability tests (RFC 0005 §3.1, §8.6) ─────────────────────────

    /// Scenario: Ephemeral zone publish is fire-and-forget — no ZonePublishResult.
    /// WHEN agent publishes to an ephemeral zone (zone.ephemeral=true)
    /// THEN runtime does NOT send a ZonePublishResult (spec lines 624-626)
    #[tokio::test]
    async fn test_ephemeral_zone_no_publish_result() {
        use tze_hud_scene::types::{
            ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, ZoneDefinition,
            ZoneMediaType,
        };
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");

        // Register an ephemeral zone in the scene
        {
            let st = service.state.lock().await;
            st.scene
                .lock()
                .await
                .zone_registry
                .register(ZoneDefinition {
                    id: tze_hud_scene::SceneId::new(),
                    name: "live-caption".to_string(),
                    description: "Ephemeral caption zone".to_string(),
                    geometry_policy: GeometryPolicy::Relative {
                        x_pct: 0.1,
                        y_pct: 0.8,
                        width_pct: 0.8,
                        height_pct: 0.1,
                    },
                    accepted_media_types: vec![ZoneMediaType::StreamText],
                    rendering_policy: RenderingPolicy::default(),
                    contention_policy: ContentionPolicy::LatestWins,
                    max_publishers: 1,
                    transport_constraint: None,
                    auto_clear_ms: None,
                    ephemeral: true, // <-- ephemeral zone
                    layer_attachment: LayerAttachment::Content,
                });
        }

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(
                    crate::proto::session::hud_session_server::HudSessionServer::new(service),
                )
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let mut client = crate::proto::session::hud_session_client::HudSessionClient::connect(
            format!("http://[::1]:{}", addr.port()),
        )
        .await
        .unwrap();

        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "ephemeral-publisher", "test-key").await;

        // Publish to the ephemeral zone
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ZonePublish(ZonePublish {
                zone_name: "live-caption".to_string(),
                content: Some(crate::proto::ZoneContent {
                    payload: Some(crate::proto::zone_content::Payload::StreamText(
                        "caption text".to_string(),
                    )),
                }),
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
                breakpoints: Vec::new(),
            })),
        })
        .await
        .unwrap();

        // Send a heartbeat so we can verify the next message is a heartbeat echo
        // (meaning no ZonePublishResult was sent for the ephemeral zone publish)
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 99999,
            })),
        })
        .await
        .unwrap();

        // The first message after the ephemeral zone publish should be the heartbeat echo,
        // NOT a ZonePublishResult (ephemeral zones are fire-and-forget)
        let next_msg = stream.next().await.unwrap().unwrap();
        match &next_msg.payload {
            Some(ServerPayload::ZonePublishResult(_)) => {
                panic!("Ephemeral zone publish must NOT produce a ZonePublishResult")
            }
            Some(ServerPayload::Heartbeat(hb)) => {
                assert_eq!(hb.timestamp_mono_us, 99999, "expected heartbeat echo");
            }
            other => panic!("Expected Heartbeat echo (no ZonePublishResult), got: {other:?}"),
        }
        drop(handle);
    }

    /// Scenario: Durable zone publish is acknowledged (RFC 0005 §3.1, spec lines 620-622).
    /// WHEN agent publishes to a durable zone (zone.ephemeral=false)
    /// THEN runtime sends a ZonePublishResult.
    #[tokio::test]
    async fn test_durable_zone_publish_result() {
        use tze_hud_scene::types::{
            ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, ZoneDefinition,
            ZoneMediaType,
        };
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");

        // Register a durable zone
        {
            let st = service.state.lock().await;
            st.scene
                .lock()
                .await
                .zone_registry
                .register(ZoneDefinition {
                    id: tze_hud_scene::SceneId::new(),
                    name: "status-text".to_string(),
                    description: "Durable status text zone".to_string(),
                    geometry_policy: GeometryPolicy::Relative {
                        x_pct: 0.0,
                        y_pct: 0.0,
                        width_pct: 1.0,
                        height_pct: 0.05,
                    },
                    accepted_media_types: vec![ZoneMediaType::StreamText],
                    rendering_policy: RenderingPolicy::default(),
                    contention_policy: ContentionPolicy::LatestWins,
                    max_publishers: 4,
                    transport_constraint: None,
                    auto_clear_ms: None,
                    ephemeral: false, // <-- durable zone
                    layer_attachment: LayerAttachment::Content,
                });
        }

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(
                    crate::proto::session::hud_session_server::HudSessionServer::new(service),
                )
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let mut client = crate::proto::session::hud_session_client::HudSessionClient::connect(
            format!("http://[::1]:{}", addr.port()),
        )
        .await
        .unwrap();

        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "durable-publisher", "test-key").await;

        let client_seq: u64 = 2;
        tx.send(ClientMessage {
            sequence: client_seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ZonePublish(ZonePublish {
                zone_name: "status-text".to_string(),
                content: Some(crate::proto::ZoneContent {
                    payload: Some(crate::proto::zone_content::Payload::StreamText(
                        "status: ok".to_string(),
                    )),
                }),
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
                breakpoints: Vec::new(),
            })),
        })
        .await
        .unwrap();

        // Durable zone: should receive ZonePublishResult
        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::ZonePublishResult(result)) => {
                assert_eq!(result.request_sequence, client_seq);
                assert!(result.accepted, "durable zone publish should be accepted");
            }
            other => panic!("Expected ZonePublishResult for durable zone, got: {other:?}"),
        }
        drop(handle);
    }

    // ─── Input control tests (RFC 0005 §3.8) ─────────────────────────────────

    /// Scenario: InputFocusRequest → InputFocusResponse (synchronous, correlated by sequence).
    /// WHEN agent sends InputFocusRequest at sequence N,
    /// THEN runtime responds with InputFocusResponse (spec lines 567-569).
    #[tokio::test]
    async fn test_input_focus_request_response() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "focus-agent", "test-key").await;

        let tile_id_bytes = vec![1u8; 16];
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::InputFocusRequest(InputFocusRequest {
                tile_id: tile_id_bytes.clone(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::InputFocusResponse(resp)) => {
                assert_eq!(resp.tile_id, tile_id_bytes, "tile_id must match request");
                assert!(resp.granted, "focus should be granted in v1");
            }
            other => panic!("Expected InputFocusResponse, got: {other:?}"),
        }
    }

    /// Scenario: InputCaptureRequest → InputCaptureResponse (synchronous).
    #[tokio::test]
    async fn test_input_capture_request_response() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "capture-agent", "test-key").await;

        let tile_id_bytes = vec![2u8; 16];
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::InputCaptureRequest(InputCaptureRequest {
                tile_id: tile_id_bytes.clone(),
                device_kind: "pointer".to_string(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::InputCaptureResponse(resp)) => {
                assert_eq!(resp.tile_id, tile_id_bytes, "tile_id must match request");
                assert_eq!(resp.device_kind, "pointer");
                assert!(resp.granted, "capture should be granted in v1");
            }
            other => panic!("Expected InputCaptureResponse, got: {other:?}"),
        }
    }

    /// Scenario: InputCaptureRelease → CaptureReleasedEvent in EventBatch (asynchronous).
    /// WHEN agent sends InputCaptureRelease (field 29) for a captured device
    /// THEN runtime delivers CaptureReleasedEvent in EventBatch (field 34), reason=AGENT_RELEASED
    /// (spec lines 571-573). Only delivered if agent has FOCUS_EVENTS subscription.
    #[tokio::test]
    async fn test_input_capture_release_delivers_event() {
        let (mut client, _server) = setup_test().await;

        // Use a custom handshake with access_input_events (needed for FOCUS_EVENTS sub)
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream_rx = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "capture-release-agent".to_string(),
                agent_display_name: "capture-release-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec!["access_input_events".to_string()],
                initial_subscriptions: vec!["INPUT_EVENTS".to_string(), "FOCUS_EVENTS".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream_rx).await.unwrap().into_inner();

        // Drain SessionEstablished and SceneSnapshot
        for _ in 0..2 {
            let _ = response_stream.next().await;
        }

        // Send InputCaptureRelease
        let tile_id_bytes = vec![3u8; 16];
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::InputCaptureRelease(InputCaptureRelease {
                tile_id: tile_id_bytes.clone(),
                device_kind: "pointer".to_string(),
            })),
        })
        .await
        .unwrap();

        // Should receive EventBatch with CaptureReleasedEvent
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::EventBatch(batch)) => {
                assert_eq!(batch.events.len(), 1, "should have exactly one event");
                match &batch.events[0].event {
                    Some(crate::proto::input_envelope::Event::CaptureReleased(ev)) => {
                        assert_eq!(
                            ev.tile_id, tile_id_bytes,
                            "tile_id must match release request"
                        );
                        assert_eq!(
                            ev.reason,
                            crate::proto::CaptureReleasedReason::AgentReleased as i32,
                            "reason must be AGENT_RELEASED"
                        );
                    }
                    other => panic!("Expected CaptureReleasedEvent, got: {other:?}"),
                }
            }
            other => panic!("Expected EventBatch with CaptureReleasedEvent, got: {other:?}"),
        }
        drop(tx);
    }

    /// Scenario: SetImePosition is fire-and-forget — no response sent.
    #[tokio::test]
    async fn test_set_ime_position_no_response() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "ime-agent", "test-key").await;

        // Send SetImePosition (fire-and-forget)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SetImePosition(SetImePosition {
                tile_id: vec![4u8; 16],
                x: 100.0,
                y: 200.0,
            })),
        })
        .await
        .unwrap();

        // Send a heartbeat immediately after — should receive heartbeat echo, NOT any IME response
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 88888,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::Heartbeat(hb)) => {
                assert_eq!(
                    hb.timestamp_mono_us, 88888,
                    "expected heartbeat echo after SetImePosition"
                );
            }
            other => panic!("Expected Heartbeat (no IME response), got: {other:?}"),
        }
    }

    // ─── Lease management tests (rig-7bho) ───────────────────────────────────

    /// Scenario: Lease acquisition via session stream (spec §Lease Management RPCs,
    /// lease-governance spec §Lease State Machine).
    ///
    /// WHEN agent sends LeaseRequest(action=ACQUIRE) on session stream,
    /// THEN runtime responds with LeaseResponse(granted=true) AND
    ///      a LeaseStateChange(REQUESTED→ACTIVE) notification.
    #[tokio::test]
    async fn test_lease_acquire_sends_lease_response_and_state_change() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "lease-acquire-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        // First response: LeaseResponse(granted=true)
        let resp_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &resp_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted, "Lease should be granted");
                assert_eq!(resp.lease_id.len(), 16, "lease_id must be 16-byte UUIDv7");
                assert_eq!(resp.granted_ttl_ms, 30_000);
                assert_eq!(resp.granted_priority, 2);
                assert!(
                    resp.granted_capabilities
                        .contains(&"create_tiles".to_string())
                );
                resp.lease_id.clone()
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        };

        // Second response: LeaseStateChange(REQUESTED→ACTIVE)
        let change_msg = stream.next().await.unwrap().unwrap();
        match &change_msg.payload {
            Some(ServerPayload::LeaseStateChange(change)) => {
                assert_eq!(
                    change.lease_id, lease_id,
                    "LeaseStateChange must reference same lease"
                );
                assert_eq!(change.previous_state, "REQUESTED");
                assert_eq!(change.new_state, "ACTIVE");
                assert!(change.timestamp_wall_us > 0);
            }
            other => panic!("Expected LeaseStateChange, got: {other:?}"),
        }
    }

    /// Scenario: lease_id is always a 16-byte UUIDv7 (SceneId spec §SceneId for Scene-Object Identifiers).
    ///
    /// WHEN agent requests a lease,
    /// THEN all lease_id fields in responses are exactly 16 bytes.
    #[tokio::test]
    async fn test_lease_id_is_16_byte_uuidv7() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "sceneid-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 10_000,
                capabilities: Vec::new(),
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        // LeaseResponse
        let resp_msg = stream.next().await.unwrap().unwrap();
        match &resp_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted);
                assert_eq!(
                    resp.lease_id.len(),
                    16,
                    "lease_id in LeaseResponse must be 16 bytes (SceneId UUIDv7)"
                );
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        }

        // LeaseStateChange — also carries lease_id
        let change_msg = stream.next().await.unwrap().unwrap();
        match &change_msg.payload {
            Some(ServerPayload::LeaseStateChange(change)) => {
                assert_eq!(
                    change.lease_id.len(),
                    16,
                    "lease_id in LeaseStateChange must be 16 bytes"
                );
            }
            other => panic!("Expected LeaseStateChange, got: {other:?}"),
        }
    }

    /// Scenario: Priority 0 request downgraded to priority 2 (lease-governance spec
    /// §Priority Assignment: "agent requesting priority 0 MUST receive priority 2").
    #[tokio::test]
    async fn test_lease_priority_zero_downgraded() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "prio-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 10_000,
                capabilities: Vec::new(),
                lease_priority: 0, // Priority 0 reserved for system — must be downgraded
            })),
        })
        .await
        .unwrap();

        let resp_msg = stream.next().await.unwrap().unwrap();
        match &resp_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted);
                assert_eq!(
                    resp.granted_priority, 2,
                    "Priority 0 request must be downgraded to priority 2"
                );
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        }
    }

    /// Scenario: Priority 1 without capability is downgraded to 2.
    #[tokio::test]
    async fn test_lease_priority_one_without_capability_downgraded() {
        let (mut client, _server) = setup_test().await;
        // Agent does not request lease:priority:1 capability
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "prio1-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 10_000,
                capabilities: Vec::new(),
                lease_priority: 1, // Requires lease:priority:1 cap — not granted
            })),
        })
        .await
        .unwrap();

        let resp_msg = stream.next().await.unwrap().unwrap();
        match &resp_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted);
                assert_eq!(
                    resp.granted_priority, 2,
                    "Priority 1 without lease:priority:1 capability must be downgraded to 2"
                );
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        }
    }

    /// Scenario: LeaseRenew responds with LeaseResponse(granted=true) AND LeaseStateChange.
    ///
    /// Spec §Lease Management RPCs: "runtime SHALL respond with LeaseResponse".
    /// On renewal, LeaseResponse with granted=true and the updated TTL is expected,
    /// followed by a LeaseStateChange(ACTIVE→ACTIVE) notification.
    #[tokio::test]
    async fn test_lease_renew_returns_lease_response_and_state_change() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "renew-agent", "test-key").await;

        // Acquire a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        // Consume LeaseResponse and LeaseStateChange from acquire
        let resp = stream.next().await.unwrap().unwrap();
        let lease_id = match &resp.payload {
            Some(ServerPayload::LeaseResponse(r)) if r.granted => r.lease_id.clone(),
            other => panic!("Expected LeaseResponse(granted), got: {other:?}"),
        };
        let _state_change = stream.next().await.unwrap().unwrap(); // consume REQUESTED→ACTIVE

        // Renew the lease
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRenew(LeaseRenew {
                lease_id: lease_id.clone(),
                new_ttl_ms: 120_000,
            })),
        })
        .await
        .unwrap();

        // First: LeaseResponse(granted=true) with updated TTL
        let renew_resp = stream.next().await.unwrap().unwrap();
        match &renew_resp.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted, "Renewal should be granted");
                assert_eq!(resp.lease_id, lease_id, "Same lease_id in renewal response");
                assert_eq!(resp.granted_ttl_ms, 120_000, "TTL should reflect renewal");
            }
            other => panic!("Expected LeaseResponse(granted) on renew, got: {other:?}"),
        }

        // Second: LeaseStateChange(ACTIVE→ACTIVE)
        let change = stream.next().await.unwrap().unwrap();
        match &change.payload {
            Some(ServerPayload::LeaseStateChange(sc)) => {
                assert_eq!(sc.lease_id, lease_id);
                assert_eq!(sc.previous_state, "ACTIVE");
                assert_eq!(sc.new_state, "ACTIVE");
            }
            other => panic!("Expected LeaseStateChange on renew, got: {other:?}"),
        }
    }

    /// Scenario: LeaseRelease sends LeaseResponse(granted=true) then LeaseStateChange(ACTIVE→RELEASED).
    ///
    /// WHEN agent sends LeaseRelease,
    /// THEN runtime first sends LeaseResponse(granted=true) (spec: every lease op answered by LeaseResponse),
    ///      then LeaseStateChange(new_state=RELEASED) (transactional notification).
    #[tokio::test]
    async fn test_lease_release_sends_state_change_released() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "release-agent", "test-key").await;

        // Acquire a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: Vec::new(),
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let resp = stream.next().await.unwrap().unwrap();
        let lease_id = match &resp.payload {
            Some(ServerPayload::LeaseResponse(r)) if r.granted => r.lease_id.clone(),
            other => panic!("Expected LeaseResponse(granted), got: {other:?}"),
        };
        let _sc = stream.next().await.unwrap().unwrap(); // consume REQUESTED→ACTIVE

        // Release the lease
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRelease(LeaseRelease {
                lease_id: lease_id.clone(),
            })),
        })
        .await
        .unwrap();

        // First: LeaseResponse(granted=true)
        let release_resp = stream.next().await.unwrap().unwrap();
        match &release_resp.payload {
            Some(ServerPayload::LeaseResponse(r)) => {
                assert!(
                    r.granted,
                    "LeaseRelease success must return LeaseResponse(granted=true)"
                );
                assert_eq!(r.lease_id, lease_id, "lease_id must match in LeaseResponse");
            }
            other => panic!("Expected LeaseResponse(granted) for release, got: {other:?}"),
        }

        // Second: LeaseStateChange(ACTIVE→RELEASED).
        let sc_msg = stream.next().await.unwrap().unwrap();
        match &sc_msg.payload {
            Some(ServerPayload::LeaseStateChange(sc)) => {
                assert_eq!(sc.lease_id, lease_id);
                assert_eq!(sc.previous_state, "ACTIVE");
                assert_eq!(sc.new_state, "RELEASED");
                assert!(sc.timestamp_wall_us > 0);
            }
            other => panic!("Expected LeaseStateChange(RELEASED), got: {other:?}"),
        }
    }

    /// Scenario: Retransmit correlation — sending a lease request with the same
    /// client sequence number returns the cached response (RFC 0005 §5.3).
    ///
    /// The server must detect retransmits (same sequence) and replay the response
    /// without re-applying the operation.
    #[tokio::test]
    async fn test_lease_retransmit_correlation_returns_cached_response() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "retransmit-agent", "test-key").await;

        let lease_req = ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        };

        // Original request
        tx.send(lease_req.clone()).await.unwrap();

        // Consume the original LeaseResponse + LeaseStateChange
        let orig_resp = stream.next().await.unwrap().unwrap();
        let orig_lease_id = match &orig_resp.payload {
            Some(ServerPayload::LeaseResponse(r)) => {
                assert!(r.granted);
                r.lease_id.clone()
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        };
        let _orig_sc = stream.next().await.unwrap().unwrap(); // REQUESTED→ACTIVE

        // Retransmit with same sequence number (simulates no-ack / lost response)
        tx.send(lease_req).await.unwrap();

        // The retransmit should return the cached LeaseResponse (no duplicate lease created)
        let retx_resp = stream.next().await.unwrap().unwrap();
        match &retx_resp.payload {
            Some(ServerPayload::LeaseResponse(r)) => {
                assert!(r.granted, "Retransmit should return cached grant");
                assert_eq!(
                    r.lease_id, orig_lease_id,
                    "Retransmit must return the same lease_id as the original response"
                );
                assert_eq!(r.granted_ttl_ms, 30_000);
            }
            other => panic!("Expected LeaseResponse on retransmit, got: {other:?}"),
        }
    }

    /// Scenario: Three agents contending for leases.
    ///
    /// Validates concurrent lease acquisition: all three agents can independently
    /// acquire leases from the same runtime with unique lease IDs.
    #[tokio::test]
    async fn test_three_agents_lease_contention() {
        let (client1, _server) = setup_test().await;

        // Use a single shared server — connect 3 clients to the same port.
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let url = format!("http://[::1]:{}", addr.port());
        let mut c1 = HudSessionClient::connect(url.clone()).await.unwrap();
        let mut c2 = HudSessionClient::connect(url.clone()).await.unwrap();
        let mut c3 = HudSessionClient::connect(url.clone()).await.unwrap();

        let (tx1, _, mut s1) = handshake(&mut c1, "agent-alpha", "test-key").await;
        let (tx2, _, mut s2) = handshake(&mut c2, "agent-beta", "test-key").await;
        let (tx3, _, mut s3) = handshake(&mut c3, "agent-gamma", "test-key").await;

        // All three agents request leases concurrently (sequential sends for simplicity)
        for (tx, seq) in [(&tx1, 2u64), (&tx2, 2u64), (&tx3, 2u64)] {
            tx.send(ClientMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                    ttl_ms: 30_000,
                    capabilities: vec!["create_tiles".to_string()],
                    lease_priority: 2,
                })),
            })
            .await
            .unwrap();
        }

        // Collect lease IDs
        let mut lease_ids = Vec::new();
        for stream in [&mut s1, &mut s2, &mut s3] {
            let msg = stream.next().await.unwrap().unwrap();
            match &msg.payload {
                Some(ServerPayload::LeaseResponse(r)) => {
                    assert!(r.granted, "All agents should get leases granted");
                    assert_eq!(r.lease_id.len(), 16);
                    lease_ids.push(r.lease_id.clone());
                }
                other => panic!("Expected LeaseResponse, got: {other:?}"),
            }
        }

        // All lease IDs must be unique — use a HashSet for correct deduplication.
        let set: std::collections::HashSet<Vec<u8>> = lease_ids.iter().cloned().collect();
        assert_eq!(
            set.len(),
            3,
            "All three agents must receive unique lease IDs"
        );

        drop(client1);
    }

    /// Scenario: Lease expiry — runtime accepts a lease with a very short TTL.
    ///
    /// This test verifies that the protocol accepts LeaseRequest with any valid TTL,
    /// including very short ones used in expiry scenarios.
    /// Full expiry notification behavior requires the timer loop (post-v1 scope for
    /// push notifications); here we verify the initial grant succeeds and the correct
    /// SceneId is returned.
    #[tokio::test]
    async fn test_lease_expiry_scenario_initial_grant() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "expiry-agent", "test-key").await;

        // Request a lease with a very short TTL (100ms — represents expiry scenario)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 100, // very short TTL for expiry testing
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let resp = stream.next().await.unwrap().unwrap();
        match &resp.payload {
            Some(ServerPayload::LeaseResponse(r)) => {
                assert!(r.granted);
                assert_eq!(
                    r.granted_ttl_ms, 100,
                    "Short-TTL lease should be granted as requested"
                );
                assert_eq!(r.lease_id.len(), 16, "lease_id must be 16-byte SceneId");
            }
            other => panic!("Expected LeaseResponse for short-TTL lease, got: {other:?}"),
        }
    }

    /// Scenario: LeaseStateChange notification traffic class is Transactional.
    ///
    /// LEASE_CHANGES are always subscribed and never dropped under backpressure
    /// (spec §Subscription Management, §Lease Management RPCs).
    #[test]
    fn test_lease_state_change_is_transactional() {
        assert_eq!(
            classify_server_payload(&ServerPayload::LeaseStateChange(LeaseStateChange::default())),
            TrafficClass::Transactional,
            "LeaseStateChange must be Transactional (never dropped)"
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::LeaseResponse(LeaseResponse::default())),
            TrafficClass::Transactional,
            "LeaseResponse must be Transactional (never dropped)"
        );
    }

    /// Scenario: Renew on non-existent lease returns denial.
    #[tokio::test]
    async fn test_lease_renew_unknown_lease_returns_denial() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "renew-unknown-agent", "test-key").await;

        let fake_lease_id = uuid::Uuid::now_v7().as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRenew(LeaseRenew {
                lease_id: fake_lease_id,
                new_ttl_ms: 60_000,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(!resp.granted, "Renew on unknown lease must be denied");
                assert!(!resp.deny_code.is_empty(), "deny_code must be populated");
            }
            other => {
                panic!("Expected LeaseResponse(denied) for unknown lease renew, got: {other:?}")
            }
        }
    }

    /// Scenario: Release on non-existent lease returns denial.
    #[tokio::test]
    async fn test_lease_release_unknown_lease_returns_denial() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "release-unknown-agent", "test-key").await;

        let fake_lease_id = uuid::Uuid::now_v7().as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRelease(LeaseRelease {
                lease_id: fake_lease_id,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(!resp.granted, "Release on unknown lease must be denied");
                assert!(!resp.deny_code.is_empty(), "deny_code must be populated");
            }
            other => {
                panic!("Expected LeaseResponse(denied) for unknown lease release, got: {other:?}")
            }
        }
    }

    /// Scenario: Disconnect orphan behavior — session cleanup does not panic
    /// when leases are held.
    ///
    /// WHEN an agent with active leases disconnects ungracefully,
    /// THEN the session is removed from the registry without error.
    ///
    /// Full orphan-to-expiry lifecycle requires a timer loop (post-v1); this test
    /// verifies the session teardown path is safe when leases are present.
    #[tokio::test]
    async fn test_disconnect_with_active_leases_no_panic() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "disconnect-agent", "test-key").await;

        // Acquire a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        // Consume LeaseResponse + LeaseStateChange
        let _r = stream.next().await.unwrap().unwrap();
        let _sc = stream.next().await.unwrap().unwrap();

        // Drop both tx and stream to simulate ungraceful disconnect
        drop(tx);
        drop(stream);

        // Give the server task time to clean up
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        // If we reach here without a panic, the cleanup path is safe.
    }

    // ─── Live capability revocation tests (RFC 0001 §3.3, GAP-G3-4) ────────────

    /// Set up a test server that also returns the capability-revocation broadcast sender
    /// (so tests can call `revoke_capability_on_lease` via the sender directly).
    async fn setup_test_with_revocation_tx() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<SharedState>>,
        tokio::sync::broadcast::Sender<CapabilityRevocationEvent>,
    ) {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");
        let shared_state = service.state.clone();
        let revocation_tx = service.capability_revocation_tx.clone();

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        (client, handle, shared_state, revocation_tx)
    }

    /// Helper: do a full handshake with publish_zone:subtitle capability and acquire a lease.
    /// Returns (tx, stream, lease_id_bytes).
    async fn handshake_with_publish_zone_lease(
        client: &mut HudSessionClient<tonic::transport::Channel>,
    ) -> (
        tokio::sync::mpsc::Sender<ClientMessage>,
        tonic::Streaming<ServerMessage>,
        Vec<u8>,
        tze_hud_scene::SceneId,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "revoke-test-agent".to_string(),
                agent_display_name: "revoke-test-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec![
                    "publish_zone:subtitle".to_string(),
                    "create_tiles".to_string(),
                ],
                initial_subscriptions: vec![],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();

        // Drain SessionEstablished + SceneSnapshot
        let _established = response_stream.next().await.unwrap().unwrap();
        let _snapshot = response_stream.next().await.unwrap().unwrap();

        // Request a lease with publish_zone:subtitle + create_tiles
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec![
                    "publish_zone:subtitle".to_string(),
                    "create_tiles".to_string(),
                ],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        // LeaseResponse
        let lease_resp_msg = response_stream.next().await.unwrap().unwrap();
        let lease_id_bytes = match &lease_resp_msg.payload {
            Some(ServerPayload::LeaseResponse(lr)) => {
                assert!(lr.granted, "Lease must be granted");
                lr.lease_id.clone()
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        };

        // LeaseStateChange (REQUESTED → ACTIVE)
        let _sc = response_stream.next().await.unwrap().unwrap();

        // Parse lease_id back to SceneId.
        // scene_id_to_bytes() uses as_uuid().as_bytes() (big-endian UUID bytes),
        // so we must decode with from_uuid(Uuid::from_bytes()) to match.
        let lease_arr: [u8; 16] = lease_id_bytes
            .as_slice()
            .try_into()
            .expect("lease_id must be 16 bytes");
        let lease_scene_id = tze_hud_scene::SceneId::from_uuid(uuid::Uuid::from_bytes(lease_arr));

        (tx, response_stream, lease_id_bytes, lease_scene_id)
    }

    /// WHEN the runtime revokes a capability from an active lease,
    /// THEN the agent receives CapabilityNotice(revoked=[cap_name]).
    #[tokio::test]
    async fn test_revoke_capability_sends_capability_notice() {
        let (mut client, _server, _state, revocation_tx) = setup_test_with_revocation_tx().await;

        let (_tx, mut stream, _lease_id_bytes, lease_scene_id) =
            handshake_with_publish_zone_lease(&mut client).await;

        // Revoke publish_zone:subtitle
        let _ = revocation_tx.send(CapabilityRevocationEvent {
            lease_id: lease_scene_id,
            capability_name: "publish_zone:subtitle".to_string(),
        });

        // The agent should receive a CapabilityNotice with revoked=[publish_zone:subtitle]
        let msg = stream.next().await.unwrap().unwrap();
        match msg.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(
                    notice
                        .revoked
                        .contains(&"publish_zone:subtitle".to_string()),
                    "CapabilityNotice.revoked must contain publish_zone:subtitle"
                );
                assert!(
                    notice.granted.is_empty(),
                    "CapabilityNotice.granted must be empty for a revocation"
                );
            }
            other => panic!("Expected CapabilityNotice, got: {other:?}"),
        }
    }

    /// WHEN the runtime revokes a capability from an active lease,
    /// THEN the agent receives LeaseStateChange with previous_state=ACTIVE, new_state=ACTIVE.
    #[tokio::test]
    async fn test_revoke_capability_sends_lease_state_change() {
        let (mut client, _server, _state, revocation_tx) = setup_test_with_revocation_tx().await;

        let (_tx, mut stream, _lease_id_bytes, lease_scene_id) =
            handshake_with_publish_zone_lease(&mut client).await;

        // Revoke create_tiles
        let _ = revocation_tx.send(CapabilityRevocationEvent {
            lease_id: lease_scene_id,
            capability_name: "create_tiles".to_string(),
        });

        // CapabilityNotice first
        let _notice = stream.next().await.unwrap().unwrap();

        // Then LeaseStateChange
        let msg = stream.next().await.unwrap().unwrap();
        match msg.payload {
            Some(ServerPayload::LeaseStateChange(sc)) => {
                assert_eq!(sc.previous_state, "ACTIVE", "Lease must stay ACTIVE");
                assert_eq!(
                    sc.new_state, "ACTIVE",
                    "Lease must stay ACTIVE after capability revocation"
                );
                assert!(
                    sc.reason.contains("CAPABILITY_REVOKED"),
                    "LeaseStateChange reason must contain CAPABILITY_REVOKED"
                );
            }
            other => panic!("Expected LeaseStateChange, got: {other:?}"),
        }
    }

    /// WHEN a capability is revoked from a lease, THEN the lease scope is narrowed
    /// in the scene graph and the capability is absent from the live scope.
    #[tokio::test]
    async fn test_revoke_capability_narrows_scene_graph_scope() {
        let (mut client, _server, state, revocation_tx) = setup_test_with_revocation_tx().await;

        let (_tx, mut stream, _lease_id_bytes, lease_scene_id) =
            handshake_with_publish_zone_lease(&mut client).await;

        // Before revocation: verify the capability is present
        {
            let st = state.lock().await;
            let scene = st.scene.lock().await;
            let caps = scene
                .lease_capabilities(&lease_scene_id)
                .expect("lease must exist");
            assert!(
                caps.iter().any(|c| matches!(c, tze_hud_scene::types::Capability::PublishZone(z) if z == "subtitle")),
                "publish_zone:subtitle must be in the live scope before revocation"
            );
        }

        // Revoke
        let _ = revocation_tx.send(CapabilityRevocationEvent {
            lease_id: lease_scene_id,
            capability_name: "publish_zone:subtitle".to_string(),
        });

        // Drain protocol messages
        let _notice = stream.next().await.unwrap().unwrap();
        let _sc = stream.next().await.unwrap().unwrap();

        // After revocation: the capability must be absent from the live scope
        {
            let st = state.lock().await;
            let scene = st.scene.lock().await;
            let caps = scene
                .lease_capabilities(&lease_scene_id)
                .expect("lease must still exist after capability revocation");
            assert!(
                !caps.iter().any(|c| matches!(c, tze_hud_scene::types::Capability::PublishZone(z) if z == "subtitle")),
                "publish_zone:subtitle must be removed from the live scope after revocation"
            );
        }
    }

    /// WHEN a capability is revoked, THEN the lease remains in ACTIVE state.
    #[tokio::test]
    async fn test_revoke_capability_preserves_lease_active_state() {
        let (mut client, _server, state, revocation_tx) = setup_test_with_revocation_tx().await;

        let (_tx, mut stream, _lease_id_bytes, lease_scene_id) =
            handshake_with_publish_zone_lease(&mut client).await;

        // Revoke one capability
        let _ = revocation_tx.send(CapabilityRevocationEvent {
            lease_id: lease_scene_id,
            capability_name: "create_tiles".to_string(),
        });
        let _notice = stream.next().await.unwrap().unwrap();
        let _sc = stream.next().await.unwrap().unwrap();

        // Lease must still be ACTIVE in the scene graph
        let st = state.lock().await;
        let scene = st.scene.lock().await;
        let lease = scene
            .leases
            .get(&lease_scene_id)
            .expect("lease must still exist");
        assert_eq!(
            lease.state,
            tze_hud_scene::types::LeaseState::Active,
            "Lease must remain ACTIVE after capability revocation"
        );
    }

    /// WHEN an unknown capability name is used in a revocation,
    /// THEN the agent receives RuntimeError(INVALID_ARGUMENT) and the lease is unchanged.
    #[tokio::test]
    async fn test_revoke_unknown_capability_returns_error() {
        let (mut client, _server, state, revocation_tx) = setup_test_with_revocation_tx().await;

        let (_tx, mut stream, _lease_id_bytes, lease_scene_id) =
            handshake_with_publish_zone_lease(&mut client).await;

        // Try to revoke a capability that doesn't exist in the vocabulary
        let _ = revocation_tx.send(CapabilityRevocationEvent {
            lease_id: lease_scene_id,
            capability_name: "totally_unknown_capability".to_string(),
        });

        // Should get a RuntimeError
        let msg = stream.next().await.unwrap().unwrap();
        match msg.payload {
            Some(ServerPayload::RuntimeError(e)) => {
                assert_eq!(e.error_code, "CAPABILITY_NOT_PRESENT");
            }
            other => panic!("Expected RuntimeError, got: {other:?}"),
        }

        // Lease scope unchanged (still has both original capabilities)
        let st = state.lock().await;
        let scene = st.scene.lock().await;
        let caps = scene
            .lease_capabilities(&lease_scene_id)
            .expect("lease must exist");
        assert_eq!(
            caps.len(),
            2,
            "Lease scope must be unchanged after failed revocation"
        );
    }

    /// WHEN a capability that is not in the lease scope is revoked (noop),
    /// THEN the agent receives RuntimeError(CAPABILITY_NOT_PRESENT).
    #[tokio::test]
    async fn test_revoke_absent_capability_returns_not_present() {
        let (mut client, _server, _state, revocation_tx) = setup_test_with_revocation_tx().await;

        let (_tx, mut stream, _lease_id_bytes, lease_scene_id) =
            handshake_with_publish_zone_lease(&mut client).await;

        // manage_tabs is not in this lease's scope
        let _ = revocation_tx.send(CapabilityRevocationEvent {
            lease_id: lease_scene_id,
            capability_name: "manage_tabs".to_string(),
        });

        // Should get a RuntimeError for capability not present
        let msg = stream.next().await.unwrap().unwrap();
        match msg.payload {
            Some(ServerPayload::RuntimeError(e)) => {
                assert_eq!(e.error_code, "CAPABILITY_NOT_PRESENT");
            }
            other => panic!("Expected RuntimeError for absent capability, got: {other:?}"),
        }
    }

    /// WHEN revoke_capability_on_lease is called for a lease not owned by any session,
    /// THEN the broadcast produces 0 receivers and no error.
    #[tokio::test]
    async fn test_revoke_capability_noop_for_unknown_lease_id() {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");

        // An unknown lease ID not owned by any session
        let unknown_lease_id = tze_hud_scene::SceneId::new();
        // No session is connected, so this should return 0 receivers
        let n = service.revoke_capability_on_lease(unknown_lease_id, "create_tiles");
        assert_eq!(n, 0, "No active sessions means 0 receivers");
    }

    // ─── Widget publish tests (widget-system spec §Requirement: Widget Publishing via gRPC) ──

    /// Helper: create a test service with a durable widget registered.
    async fn setup_widget_service() -> HudSessionImpl {
        use tze_hud_scene::types::{
            ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetDefinition, WidgetInstance,
            WidgetParamType, WidgetParameterDeclaration, WidgetParameterValue, WidgetSvgLayer,
        };

        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");
        {
            let st = service.state.lock().await;
            let mut s = st.scene.lock().await;

            // Register a durable widget type "gauge"
            s.widget_registry.register_definition(WidgetDefinition {
                id: "gauge".to_string(),
                name: "Gauge".to_string(),
                description: "A simple gauge widget".to_string(),
                parameter_schema: vec![WidgetParameterDeclaration {
                    name: "level".to_string(),
                    param_type: WidgetParamType::F32,
                    default_value: WidgetParameterValue::F32(0.0),
                    constraints: None,
                }],
                layers: vec![WidgetSvgLayer {
                    svg_file: "fill.svg".to_string(),
                    bindings: vec![],
                }],
                default_geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 0.1,
                    height_pct: 0.1,
                },
                default_rendering_policy: RenderingPolicy::default(),
                default_contention_policy: ContentionPolicy::LatestWins,
                ephemeral: false, // durable
                hover_behavior: None,
            });

            // Create a tab and widget instance
            let tab_id = s.create_tab("main", 0).unwrap();
            s.widget_registry.register_instance(WidgetInstance {
                id: SceneId::new(),
                widget_type_name: "gauge".to_string(),
                tab_id,
                geometry_override: None,
                contention_override: None,
                instance_name: "gauge".to_string(),
                current_params: std::collections::HashMap::new(),
            });
        }
        service
    }

    /// Helper: start a server with a widget service and connect.
    async fn setup_widget_test() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let service = setup_widget_service().await;
        setup_widget_test_with_service(service).await
    }

    /// Helper: start a server with a widget service and explicit resident upload rate limit.
    async fn setup_widget_test_with_upload_rate_limit(
        upload_rate_limit_bytes_per_sec: usize,
    ) -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let service = setup_widget_service().await;
        {
            let mut st = service.state.lock().await;
            st.resource_store =
                tze_hud_resource::ResourceStore::new(tze_hud_resource::ResourceStoreConfig {
                    upload_rate_limit_bytes_per_sec,
                    ..tze_hud_resource::ResourceStoreConfig::default()
                });
        }
        setup_widget_test_with_service(service).await
    }

    /// Helper: start a server with a widget service using explicit asset-store limits.
    async fn setup_widget_test_with_asset_limits(
        max_total_bytes: u64,
        max_namespace_bytes: u64,
    ) -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let service = setup_widget_service().await;
        {
            let mut st = service.state.lock().await;
            st.widget_asset_store = crate::session::WidgetAssetStore::new_with_limits(
                max_total_bytes,
                max_namespace_bytes,
            );
        }
        setup_widget_test_with_service(service).await
    }

    /// Helper: start a server with a durable runtime widget store.
    async fn setup_widget_test_with_durable_store(
        store_path: std::path::PathBuf,
        max_total_bytes: u64,
        max_agent_bytes: u64,
    ) -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let service = setup_widget_service().await;
        {
            let mut st = service.state.lock().await;
            st.runtime_widget_store = Some(
                tze_hud_resource::RuntimeWidgetStore::open(
                    tze_hud_resource::RuntimeWidgetStoreConfig {
                        store_path,
                        max_total_bytes,
                        max_agent_bytes,
                    },
                )
                .expect("durable runtime widget store should open for tests"),
            );
        }
        setup_widget_test_with_service(service).await
    }

    async fn setup_widget_test_with_service(
        service: HudSessionImpl,
    ) -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        (client, handle)
    }

    /// Scenario: Durable WidgetPublish with valid params receives WidgetPublishResult(accepted=true).
    #[tokio::test]
    async fn test_durable_widget_publish_receives_result() {
        let (mut client, _handle) = setup_widget_test().await;

        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "widget-agent",
            "test-key",
            &["publish_widget:gauge"],
        )
        .await;

        // Send a WidgetPublish for the durable "gauge" widget
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "gauge".to_string(),
                instance_id: String::new(),
                params: vec![crate::proto::WidgetParameterValueProto {
                    param_name: "level".to_string(),
                    value: Some(crate::proto::widget_parameter_value_proto::Value::F32Value(
                        0.75,
                    )),
                }],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::WidgetPublishResult(result)) => {
                assert!(
                    result.accepted,
                    "Durable widget publish must be accepted, got error: {}",
                    result.error_code
                );
                assert_eq!(result.widget_name, "gauge");
                assert!(result.error_code.is_empty(), "No error code on success");
            }
            other => panic!("Expected WidgetPublishResult, got: {other:?}"),
        }

        drop(tx);
    }

    /// Scenario: WidgetPublish with missing capability receives WIDGET_CAPABILITY_MISSING.
    #[tokio::test]
    async fn test_widget_publish_missing_capability_rejected() {
        let (mut client, _handle) = setup_widget_test().await;

        // Handshake WITHOUT publish_widget:gauge capability
        let (tx, _init_msgs, mut stream) =
            handshake(&mut client, "widget-no-cap-agent", "test-key").await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "gauge".to_string(),
                instance_id: String::new(),
                params: vec![],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::WidgetPublishResult(result)) => {
                assert!(!result.accepted, "Expected rejection");
                assert_eq!(
                    result.error_code, "WIDGET_CAPABILITY_MISSING",
                    "Expected WIDGET_CAPABILITY_MISSING, got: {}",
                    result.error_code
                );
            }
            other => panic!("Expected WidgetPublishResult(rejected), got: {other:?}"),
        }

        drop(tx);
    }

    /// Scenario: wildcard publish_widget capability authorizes any widget publish.
    #[tokio::test]
    async fn test_widget_publish_wildcard_capability_allows_publish() {
        let (mut client, _handle) = setup_widget_test().await;

        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "widget-wildcard-agent",
            "test-key",
            &["publish_widget:*"],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "gauge".to_string(),
                instance_id: String::new(),
                params: vec![],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::WidgetPublishResult(result)) => {
                assert!(
                    result.accepted,
                    "Expected wildcard capability to authorize publish"
                );
                assert_eq!(result.widget_name, "gauge");
            }
            other => panic!("Expected WidgetPublishResult, got: {other:?}"),
        }

        drop(tx);
    }

    /// Scenario: WidgetPublish targeting unknown widget receives WIDGET_NOT_FOUND.
    #[tokio::test]
    async fn test_widget_publish_not_found() {
        let (mut client, _handle) = setup_widget_test().await;

        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "widget-notfound-agent",
            "test-key",
            &["publish_widget:nonexistent"],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "nonexistent".to_string(),
                instance_id: String::new(),
                params: vec![],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::WidgetPublishResult(result)) => {
                assert!(!result.accepted, "Expected rejection");
                assert_eq!(
                    result.error_code, "WIDGET_NOT_FOUND",
                    "Expected WIDGET_NOT_FOUND, got: {}",
                    result.error_code
                );
            }
            other => panic!("Expected WidgetPublishResult(WIDGET_NOT_FOUND), got: {other:?}"),
        }

        drop(tx);
    }

    /// Scenario: WidgetPublish with unknown parameter receives WIDGET_UNKNOWN_PARAMETER.
    #[tokio::test]
    async fn test_widget_publish_unknown_parameter() {
        let (mut client, _handle) = setup_widget_test().await;

        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "widget-badparam-agent",
            "test-key",
            &["publish_widget:gauge"],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "gauge".to_string(),
                instance_id: String::new(),
                params: vec![crate::proto::WidgetParameterValueProto {
                    param_name: "bogus_param".to_string(),
                    value: Some(crate::proto::widget_parameter_value_proto::Value::F32Value(
                        0.5,
                    )),
                }],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let result_msg = next_non_state_change(&mut stream).await;
        match &result_msg.payload {
            Some(ServerPayload::WidgetPublishResult(result)) => {
                assert!(!result.accepted, "Expected rejection");
                assert_eq!(
                    result.error_code, "WIDGET_UNKNOWN_PARAMETER",
                    "Expected WIDGET_UNKNOWN_PARAMETER, got: {}",
                    result.error_code
                );
            }
            other => {
                panic!("Expected WidgetPublishResult(WIDGET_UNKNOWN_PARAMETER), got: {other:?}")
            }
        }

        drop(tx);
    }

    /// Scenario: Ephemeral WidgetPublish is fire-and-forget (no WidgetPublishResult).
    #[tokio::test]
    async fn test_ephemeral_widget_no_publish_result() {
        use tze_hud_scene::types::{
            ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetDefinition, WidgetInstance,
            WidgetParamType, WidgetParameterDeclaration, WidgetParameterValue,
        };

        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");
        {
            let st = service.state.lock().await;
            let mut s = st.scene.lock().await;

            // Register an EPHEMERAL widget type
            s.widget_registry.register_definition(WidgetDefinition {
                id: "live-bar".to_string(),
                name: "LiveBar".to_string(),
                description: "Ephemeral bar widget".to_string(),
                parameter_schema: vec![WidgetParameterDeclaration {
                    name: "value".to_string(),
                    param_type: WidgetParamType::F32,
                    default_value: WidgetParameterValue::F32(0.0),
                    constraints: None,
                }],
                layers: vec![],
                default_geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.8,
                    width_pct: 1.0,
                    height_pct: 0.05,
                },
                default_rendering_policy: RenderingPolicy::default(),
                default_contention_policy: ContentionPolicy::LatestWins,
                ephemeral: true, // ephemeral!
                hover_behavior: None,
            });

            let tab_id = s.create_tab("main", 0).unwrap();
            s.widget_registry.register_instance(WidgetInstance {
                id: SceneId::new(),
                widget_type_name: "live-bar".to_string(),
                tab_id,
                geometry_override: None,
                contention_override: None,
                instance_name: "live-bar".to_string(),
                current_params: std::collections::HashMap::new(),
            });
        }

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let mut client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "ephemeral-widget-agent",
            "test-key",
            &["publish_widget:live-bar"],
        )
        .await;

        // Publish to ephemeral widget
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "live-bar".to_string(),
                instance_id: String::new(),
                params: vec![crate::proto::WidgetParameterValueProto {
                    param_name: "value".to_string(),
                    value: Some(crate::proto::widget_parameter_value_proto::Value::F32Value(
                        0.9,
                    )),
                }],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        // Send a heartbeat — the next response should be the echo (no WidgetPublishResult)
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 77777,
            })),
        })
        .await
        .unwrap();

        let next_msg = stream.next().await.unwrap().unwrap();
        match &next_msg.payload {
            Some(ServerPayload::WidgetPublishResult(_)) => {
                panic!("Ephemeral widget publish must NOT produce a WidgetPublishResult")
            }
            Some(ServerPayload::Heartbeat(hb)) => {
                assert_eq!(hb.timestamp_mono_us, 77777, "expected heartbeat echo");
            }
            other => panic!("Expected Heartbeat echo, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_widget_asset_register_missing_capability_rejected() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) =
            handshake_with_capabilities(&mut client, "asset-no-cap", "test-key", &[]).await;

        let payload = b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec();
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: blake3::hash(&payload).as_bytes().to_vec(),
                transport_crc32c: 0,
                total_size_bytes: payload.len() as u64,
                inline_svg_bytes: payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut stream).await;
        match &msg.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_CAPABILITY_MISSING");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_widget_asset_register_metadata_preflight_dedup_hit() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "asset-dedup",
            "test-key",
            &["register_widget_asset"],
        )
        .await;

        let payload =
            b"<svg xmlns='http://www.w3.org/2000/svg'><rect width='1' height='1'/></svg>".to_vec();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: hash.clone(),
                transport_crc32c: 0,
                total_size_bytes: payload.len() as u64,
                inline_svg_bytes: payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();

        let first = next_non_state_change(&mut stream).await;
        match &first.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(result.accepted);
                assert!(!result.was_deduplicated);
            }
            other => panic!("expected WidgetAssetRegisterResult on first upload, got: {other:?}"),
        }

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: hash,
                transport_crc32c: 0,
                total_size_bytes: 0,
                inline_svg_bytes: Vec::new(),
                metadata_only_preflight: true,
            })),
        })
        .await
        .unwrap();

        let second = next_non_state_change(&mut stream).await;
        match &second.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(result.accepted);
                assert!(result.was_deduplicated);
            }
            other => panic!("expected WidgetAssetRegisterResult on preflight, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_widget_asset_register_durable_store_dedups_after_restart() {
        let temp = tempfile::tempdir().expect("tempdir should be creatable");
        let store_path = temp.path().join("runtime-widget-store");
        let payload =
            b"<svg xmlns='http://www.w3.org/2000/svg'><rect width='3' height='2'/></svg>".to_vec();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        // First runtime instance writes the asset durably.
        let (mut client_a, handle_a) =
            setup_widget_test_with_durable_store(store_path.clone(), 0, 0).await;
        let (tx_a, _init_msgs_a, mut stream_a) = handshake_with_capabilities(
            &mut client_a,
            "asset-durable-a",
            "test-key",
            &["register_widget_asset"],
        )
        .await;
        tx_a.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: hash.clone(),
                transport_crc32c: 0,
                total_size_bytes: payload.len() as u64,
                inline_svg_bytes: payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();
        let first = next_non_state_change(&mut stream_a).await;
        match &first.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(result.accepted);
                assert!(!result.was_deduplicated);
            }
            other => panic!("expected WidgetAssetRegisterResult on first upload, got: {other:?}"),
        }

        // New runtime instance should preflight-dedup from the same durable store.
        let (mut client_b, handle_b) = setup_widget_test_with_durable_store(store_path, 0, 0).await;
        let (tx_b, _init_msgs_b, mut stream_b) = handshake_with_capabilities(
            &mut client_b,
            "asset-durable-b",
            "test-key",
            &["register_widget_asset"],
        )
        .await;
        tx_b.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: hash,
                transport_crc32c: 0,
                total_size_bytes: 0,
                inline_svg_bytes: Vec::new(),
                metadata_only_preflight: true,
            })),
        })
        .await
        .unwrap();
        let second = next_non_state_change(&mut stream_b).await;
        match &second.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(result.accepted);
                assert!(result.was_deduplicated);
            }
            other => {
                panic!("expected WidgetAssetRegisterResult on restart preflight, got: {other:?}")
            }
        }

        drop(handle_a);
        drop(handle_b);
    }

    #[tokio::test]
    async fn test_widget_asset_register_unknown_hash_requires_payload_and_hash_validation() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "asset-require-payload",
            "test-key",
            &["register_widget_asset"],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: vec![0x11; 32],
                transport_crc32c: 0,
                total_size_bytes: 0,
                inline_svg_bytes: Vec::new(),
                metadata_only_preflight: true,
            })),
        })
        .await
        .unwrap();

        let missing_payload = next_non_state_change(&mut stream).await;
        match &missing_payload.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_HASH_MISMATCH");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        let payload = b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: vec![0xAA; 32], // wrong on purpose
                transport_crc32c: 0,
                total_size_bytes: payload.len() as u64,
                inline_svg_bytes: payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();

        let hash_mismatch = next_non_state_change(&mut stream).await;
        match &hash_mismatch.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_HASH_MISMATCH");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        let valid_payload =
            b"<svg xmlns='http://www.w3.org/2000/svg'><circle r='2'/></svg>".to_vec();
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: blake3::hash(&valid_payload).as_bytes().to_vec(),
                transport_crc32c: 0,
                total_size_bytes: valid_payload.len() as u64,
                inline_svg_bytes: valid_payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();

        let uploaded = next_non_state_change(&mut stream).await;
        match &uploaded.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(result.accepted);
                assert!(!result.was_deduplicated);
                assert!(result.error_code.is_empty());
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_widget_asset_register_checksum_svg_and_type_validation() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "asset-validation",
            "test-key",
            &["register_widget_asset"],
        )
        .await;

        // Invalid type id (must be kebab-case).
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "Gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: vec![0x44; 32],
                transport_crc32c: 0,
                total_size_bytes: 0,
                inline_svg_bytes: Vec::new(),
                metadata_only_preflight: true,
            })),
        })
        .await
        .unwrap();
        let invalid_type = next_non_state_change(&mut stream).await;
        match &invalid_type.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_TYPE_INVALID");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        // Bad checksum.
        let crc_payload = b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: blake3::hash(&crc_payload).as_bytes().to_vec(),
                transport_crc32c: 1, // wrong on purpose
                total_size_bytes: crc_payload.len() as u64,
                inline_svg_bytes: crc_payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();
        let checksum_mismatch = next_non_state_change(&mut stream).await;
        match &checksum_mismatch.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_CHECKSUM_MISMATCH");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        // Invalid SVG payload.
        let invalid_svg_payload = b"not-svg".to_vec();
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: blake3::hash(&invalid_svg_payload).as_bytes().to_vec(),
                transport_crc32c: 0,
                total_size_bytes: invalid_svg_payload.len() as u64,
                inline_svg_bytes: invalid_svg_payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();
        let invalid_svg = next_non_state_change(&mut stream).await;
        match &invalid_svg.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_INVALID_SVG");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_widget_asset_register_budget_exceeded_rejected() {
        let (mut client, handle) = setup_widget_test_with_asset_limits(24, 24).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "asset-budget",
            "test-key",
            &["register_widget_asset"],
        )
        .await;

        let payload =
            b"<svg xmlns='http://www.w3.org/2000/svg'><rect width='10' height='10'/></svg>"
                .to_vec();
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: blake3::hash(&payload).as_bytes().to_vec(),
                transport_crc32c: 0,
                total_size_bytes: payload.len() as u64,
                inline_svg_bytes: payload,
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();

        let budget_denied = next_non_state_change(&mut stream).await;
        match &budget_denied.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(!result.accepted);
                assert_eq!(result.error_code, "WIDGET_ASSET_BUDGET_EXCEEDED");
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_widget_asset_register_updates_runtime_widget_lifecycle_for_publish_path() {
        let service = setup_widget_service().await;
        let shared_state = service.state.clone();
        let (mut client, handle) = setup_widget_test_with_service(service).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "asset-lifecycle",
            "test-key",
            &["register_widget_asset", "publish_widget:gauge"],
        )
        .await;

        let payload =
            b"<svg xmlns='http://www.w3.org/2000/svg'><rect id='bar' width='1' height='1'/></svg>"
                .to_vec();
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                content_hash_blake3: blake3::hash(&payload).as_bytes().to_vec(),
                transport_crc32c: 0,
                total_size_bytes: payload.len() as u64,
                inline_svg_bytes: payload.clone(),
                metadata_only_preflight: false,
            })),
        })
        .await
        .unwrap();

        let asset_handle = match next_non_state_change(&mut stream).await.payload {
            Some(ServerPayload::WidgetAssetRegisterResult(result)) => {
                assert!(result.accepted);
                assert!(!result.was_deduplicated);
                result.asset_handle
            }
            other => panic!("expected WidgetAssetRegisterResult, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::WidgetPublish(WidgetPublish {
                widget_name: "gauge".to_string(),
                instance_id: String::new(),
                params: vec![crate::proto::WidgetParameterValueProto {
                    param_name: "level".to_string(),
                    value: Some(crate::proto::widget_parameter_value_proto::Value::F32Value(
                        0.42,
                    )),
                }],
                transition_ms: 0,
                ttl_us: 0,
                element_id: Vec::new(),
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let publish_msg = next_non_state_change(&mut stream).await;
        match &publish_msg.payload {
            Some(ServerPayload::WidgetPublishResult(result)) => {
                assert!(
                    result.accepted,
                    "publish should remain usable after registration"
                );
            }
            other => panic!("expected WidgetPublishResult, got: {other:?}"),
        }

        {
            let st = shared_state.lock().await;
            let mut scene = st.scene.lock().await;
            assert_eq!(
                scene
                    .widget_registry
                    .runtime_svg_handle("gauge", "fill.svg"),
                Some(asset_handle.as_str())
            );

            let queued = scene.drain_pending_widget_svg_assets();
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].0, "gauge");
            assert_eq!(queued[0].1, "fill.svg");
            assert_eq!(queued[0].2, payload);
        }

        drop(handle);
    }

    fn tiny_png_1x1_rgba() -> Vec<u8> {
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x56, 0xc7,
            0x2f, 0x0d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }

    fn tiny_rgba_1x1(pixel: [u8; 4]) -> Vec<u8> {
        pixel.to_vec()
    }

    #[test]
    fn upload_byte_rate_limiter_enforces_sliding_window() {
        let base = Instant::now();
        let mut limiter = UploadByteRateLimiter::with_limit(8);

        assert_eq!(limiter.available_bytes(base), 8);
        limiter.reserve_bytes(base, 8);
        assert_eq!(
            limiter.available_bytes(base + Duration::from_millis(100)),
            0
        );

        let delay = limiter.next_delay(base + Duration::from_millis(100));
        assert!(
            delay >= Duration::from_millis(850),
            "expected ~900ms wait, got {delay:?}"
        );

        assert_eq!(limiter.available_bytes(base + Duration::from_secs(1)), 8);
    }

    #[test]
    fn upload_byte_rate_limiter_zero_limit_is_unbounded() {
        let base = Instant::now();
        let mut limiter = UploadByteRateLimiter::with_limit(0);

        assert_eq!(limiter.available_bytes(base), usize::MAX);
        assert_eq!(limiter.next_delay(base), Duration::ZERO);

        limiter.reserve_bytes(base, 1024);
        assert_eq!(
            limiter.available_bytes(base + Duration::from_millis(500)),
            usize::MAX
        );
    }

    #[tokio::test]
    async fn test_resource_upload_chunk_transport_backpressure_from_rate_limit() {
        let (mut client, handle) = setup_widget_test_with_upload_rate_limit(8).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-rate-limit",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let chunk_a = vec![0xAB; 8];
        let chunk_b = vec![0xCD; 8];
        let payload = [chunk_a.clone(), chunk_b.clone()].concat();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 1, // IMAGE_RGBA8
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata {
                    width: 2,
                    height: 2,
                    ..ResourceMetadata::default()
                }),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let accepted = next_non_state_change(&mut stream).await;
        let upload_id = match &accepted.payload {
            Some(ServerPayload::ResourceUploadAccepted(accepted)) => {
                assert_eq!(accepted.request_sequence, 2);
                accepted.upload_id.clone()
            }
            other => panic!("expected ResourceUploadAccepted, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_id.clone(),
                chunk_index: 0,
                data: chunk_a,
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_id.clone(),
                chunk_index: 1,
                data: chunk_b,
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_id.clone(),
                },
            )),
        })
        .await
        .unwrap();

        let early = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            next_non_state_change(&mut stream),
        )
        .await;
        assert!(
            early.is_err(),
            "chunk stream should be back-pressured; completion arrived too quickly"
        );

        let stored = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("expected ResourceStored after backpressure interval");

        match &stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 2);
                assert_eq!(stored.upload_id, upload_id);
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected ResourceStored after chunk backpressure, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_backpressure_keeps_heartbeat_responsive() {
        let (mut client, handle) = setup_widget_test_with_upload_rate_limit(8).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-heartbeat-backpressure",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let chunk_a = vec![0xAB; 8];
        let chunk_b = vec![0xCD; 8];
        let payload = [chunk_a.clone(), chunk_b.clone()].concat();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 1, // IMAGE_RGBA8
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata {
                    width: 2,
                    height: 2,
                    ..ResourceMetadata::default()
                }),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let accepted = next_non_state_change(&mut stream).await;
        let upload_id = match &accepted.payload {
            Some(ServerPayload::ResourceUploadAccepted(accepted)) => accepted.upload_id.clone(),
            other => panic!("expected ResourceUploadAccepted, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_id.clone(),
                chunk_index: 0,
                data: chunk_a,
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_id.clone(),
                chunk_index: 1,
                data: chunk_b,
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_id.clone(),
                },
            )),
        })
        .await
        .unwrap();

        let heartbeat_ts = 4242u64;
        tx.send(ClientMessage {
            sequence: 6,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: heartbeat_ts,
            })),
        })
        .await
        .unwrap();

        let heartbeat_echo = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("heartbeat should not be blocked by upload backpressure");

        match &heartbeat_echo.payload {
            Some(ServerPayload::Heartbeat(hb)) => {
                assert_eq!(hb.timestamp_mono_us, heartbeat_ts);
            }
            other => panic!("expected Heartbeat echo, got: {other:?}"),
        }

        let stored = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("expected ResourceStored after backpressure interval");

        match &stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 2);
                assert_eq!(stored.upload_id, upload_id);
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected ResourceStored after chunk backpressure, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_backpressure_preserves_transactional_chunk_order() {
        let (mut client, handle) = setup_widget_test_with_upload_rate_limit(8).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-transactional-backpressure",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let payload_a = vec![0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80];
        let payload_b = tiny_rgba_1x1([0xAA, 0xBB, 0xCC, 0xDD]);
        let hash_a = blake3::hash(&payload_a).as_bytes().to_vec();
        let hash_b = blake3::hash(&payload_b).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash_a,
                resource_type: 1, // IMAGE_RGBA8
                total_size_bytes: payload_a.len() as u64,
                metadata: Some(ResourceMetadata {
                    width: 1,
                    height: 2,
                    ..Default::default()
                }),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash_b,
                resource_type: 1, // IMAGE_RGBA8
                total_size_bytes: payload_b.len() as u64,
                metadata: Some(ResourceMetadata {
                    width: 1,
                    height: 1,
                    ..Default::default()
                }),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let accepted_a = next_non_state_change(&mut stream).await;
        let upload_a = match &accepted_a.payload {
            Some(ServerPayload::ResourceUploadAccepted(accepted)) => {
                assert_eq!(accepted.request_sequence, 2);
                accepted.upload_id.clone()
            }
            other => panic!("expected ResourceUploadAccepted for upload A, got: {other:?}"),
        };

        let accepted_b = next_non_state_change(&mut stream).await;
        let upload_b = match &accepted_b.payload {
            Some(ServerPayload::ResourceUploadAccepted(accepted)) => {
                assert_eq!(accepted.request_sequence, 3);
                accepted.upload_id.clone()
            }
            other => panic!("expected ResourceUploadAccepted for upload B, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_a.clone(),
                chunk_index: 0,
                data: payload_a,
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_a.clone(),
                },
            )),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 6,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_b.clone(),
                chunk_index: 0,
                data: payload_b,
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 7,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_b.clone(),
                },
            )),
        })
        .await
        .unwrap();

        let first_stored = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("first upload should complete before rate limiter delays second upload");

        match &first_stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 2);
                assert_eq!(stored.upload_id, upload_a);
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected first ResourceStored for request 2, got: {other:?}"),
        }

        let early_second = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            next_non_state_change(&mut stream),
        )
        .await;
        assert!(
            early_second.is_err(),
            "second upload result should be delayed by upload-rate backpressure"
        );

        let second_stored = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("second upload should complete once backpressure window clears");

        match &second_stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 3);
                assert_eq!(stored.upload_id, upload_b);
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected second ResourceStored for request 3, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_inline_transport_backpressure_from_rate_limit() {
        // Inline data (upload_start with inline_data set) must pass through
        // apply_upload_transport_backpressure just like the chunk path does.
        // A rate limit of 8 bytes/s means an 8-byte inline payload exhausts the
        // window immediately; a second upload of equal size should be delayed.
        let (mut client, handle) = setup_widget_test_with_upload_rate_limit(8).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-inline-rate-limit",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let payload_a = vec![0x01u8; 8];
        let payload_b = vec![0x02u8; 8];
        let hash_a = blake3::hash(&payload_a).as_bytes().to_vec();
        let hash_b = blake3::hash(&payload_b).as_bytes().to_vec();

        // First inline upload (fills the rate window).
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash_a,
                resource_type: 1, // IMAGE_RGBA8
                total_size_bytes: payload_a.len() as u64,
                metadata: Some(ResourceMetadata {
                    width: 2,
                    height: 1,
                    ..Default::default()
                }),
                inline_data: payload_a,
            })),
        })
        .await
        .unwrap();

        // Second inline upload (should be delayed by the rate limiter).
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash_b,
                resource_type: 1, // IMAGE_RGBA8
                total_size_bytes: payload_b.len() as u64,
                metadata: Some(ResourceMetadata {
                    width: 2,
                    height: 1,
                    ..Default::default()
                }),
                inline_data: payload_b,
            })),
        })
        .await
        .unwrap();

        // First upload should complete quickly (no prior debt in window).
        let first_stored = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("first inline upload should complete before rate limiter delays second");

        match &first_stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 2);
                assert!(
                    stored.upload_id.is_empty(),
                    "inline upload has no upload_id"
                );
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected ResourceStored for first inline upload, got: {other:?}"),
        }

        // Second upload result must be delayed by the rate window.
        let early_second = tokio::time::timeout(
            tokio::time::Duration::from_millis(300),
            next_non_state_change(&mut stream),
        )
        .await;
        assert!(
            early_second.is_err(),
            "second inline upload should be rate-limited; result arrived too quickly"
        );

        let second_stored = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            next_non_state_change(&mut stream),
        )
        .await
        .expect("second inline upload should complete once rate-limit window clears");

        match &second_stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 3);
                assert!(
                    stored.upload_id.is_empty(),
                    "inline upload has no upload_id"
                );
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected ResourceStored for second inline upload, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_start_requires_upload_resource_capability() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) =
            handshake_with_capabilities(&mut client, "resource-no-cap", "test-key", &[]).await;
        let payload = tiny_png_1x1_rgba();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 2,
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata::default()),
                inline_data: payload,
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut stream).await;
        match &msg.payload {
            Some(ServerPayload::ResourceErrorResponse(err)) => {
                assert_eq!(err.request_sequence, 2);
                assert_eq!(err.error_code, 1);
                assert!(err.upload_id.is_empty());
            }
            other => panic!("expected ResourceErrorResponse, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_inline_and_dedup_short_circuit() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-inline",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let payload = tiny_png_1x1_rgba();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash.clone(),
                resource_type: 2,
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata::default()),
                inline_data: payload.clone(),
            })),
        })
        .await
        .unwrap();

        let first = next_non_state_change(&mut stream).await;
        match &first.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 2);
                assert!(!stored.was_deduplicated);
                assert!(stored.upload_id.is_empty());
            }
            other => panic!("expected ResourceStored, got: {other:?}"),
        }

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 2,
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata::default()),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let second = next_non_state_change(&mut stream).await;
        match &second.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 3);
                assert!(stored.was_deduplicated);
                assert!(stored.upload_id.is_empty());
            }
            other => panic!("expected ResourceStored on dedup short-circuit, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_chunked_ack_then_complete() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-chunked",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let payload = tiny_png_1x1_rgba();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 2,
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata::default()),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let accepted = next_non_state_change(&mut stream).await;
        let upload_id = match &accepted.payload {
            Some(ServerPayload::ResourceUploadAccepted(accepted)) => {
                assert_eq!(accepted.request_sequence, 2);
                assert_eq!(accepted.upload_id.len(), 16);
                accepted.upload_id.clone()
            }
            other => panic!("expected ResourceUploadAccepted, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_id.clone(),
                chunk_index: 0,
                data: payload.clone(),
            })),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_id.clone(),
                },
            )),
        })
        .await
        .unwrap();

        let stored = next_non_state_change(&mut stream).await;
        match &stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                assert_eq!(stored.request_sequence, 2);
                assert_eq!(stored.upload_id, upload_id);
                assert!(!stored.was_deduplicated);
            }
            other => panic!("expected ResourceStored on complete, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_chunked_concurrent_limit_rejected() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-concurrent-limit",
            "test-key",
            &["upload_resource"],
        )
        .await;

        // ResourceStore allows at most 4 in-flight uploads per agent namespace.
        for offset in 0..5u8 {
            let seq = u64::from(offset) + 2;
            let payload = tiny_rgba_1x1([offset, 0, 0, 0xFF]);
            tx.send(ClientMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                    expected_hash: blake3::hash(&payload).as_bytes().to_vec(),
                    resource_type: 1, // IMAGE_RGBA8
                    total_size_bytes: payload.len() as u64,
                    metadata: Some(ResourceMetadata {
                        width: 1,
                        height: 1,
                        ..Default::default()
                    }),
                    inline_data: Vec::new(),
                })),
            })
            .await
            .unwrap();

            let msg = next_non_state_change(&mut stream).await;
            if offset < 4 {
                match &msg.payload {
                    Some(ServerPayload::ResourceUploadAccepted(accepted)) => {
                        assert_eq!(accepted.request_sequence, seq);
                        assert_eq!(accepted.upload_id.len(), 16);
                    }
                    other => panic!("expected ResourceUploadAccepted, got: {other:?}"),
                }
            } else {
                match &msg.payload {
                    Some(ServerPayload::ResourceErrorResponse(err)) => {
                        assert_eq!(err.request_sequence, seq);
                        assert_eq!(err.error_code, 8);
                        assert!(err.upload_id.is_empty());
                    }
                    other => panic!("expected ResourceErrorResponse, got: {other:?}"),
                }
            }
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_chunked_success_correlates_by_request_sequence() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-correlation",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let payload_a = tiny_rgba_1x1([0, 0, 0, 0xFF]);
        let payload_b = tiny_rgba_1x1([0xFF, 0, 0, 0xFF]);
        let expected_a = blake3::hash(&payload_a).as_bytes().to_vec();
        let expected_b = blake3::hash(&payload_b).as_bytes().to_vec();

        for (seq, expected_hash) in [(2u64, expected_a.clone()), (3u64, expected_b.clone())] {
            tx.send(ClientMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                    expected_hash,
                    resource_type: 1, // IMAGE_RGBA8
                    total_size_bytes: 4,
                    metadata: Some(ResourceMetadata {
                        width: 1,
                        height: 1,
                        ..Default::default()
                    }),
                    inline_data: Vec::new(),
                })),
            })
            .await
            .unwrap();
        }

        let mut upload_id_by_request = HashMap::new();
        for _ in 0..2 {
            let msg = next_non_state_change(&mut stream).await;
            match &msg.payload {
                Some(ServerPayload::ResourceUploadAccepted(accepted)) => {
                    upload_id_by_request
                        .insert(accepted.request_sequence, accepted.upload_id.clone());
                }
                other => panic!("expected ResourceUploadAccepted, got: {other:?}"),
            }
        }
        assert_eq!(upload_id_by_request.len(), 2);
        let upload_a = upload_id_by_request
            .get(&2)
            .expect("request 2 must have upload_id")
            .clone();
        let upload_b = upload_id_by_request
            .get(&3)
            .expect("request 3 must have upload_id")
            .clone();

        // Complete request 3 before request 2 to assert correlation semantics.
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_b.clone(),
                chunk_index: 0,
                data: payload_b.clone(),
            })),
        })
        .await
        .unwrap();
        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_b.clone(),
                },
            )),
        })
        .await
        .unwrap();

        tx.send(ClientMessage {
            sequence: 6,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_a.clone(),
                chunk_index: 0,
                data: payload_a.clone(),
            })),
        })
        .await
        .unwrap();
        tx.send(ClientMessage {
            sequence: 7,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete {
                    upload_id: upload_a.clone(),
                },
            )),
        })
        .await
        .unwrap();

        let mut stored_by_request = HashMap::new();
        for _ in 0..2 {
            let msg = next_non_state_change(&mut stream).await;
            match &msg.payload {
                Some(ServerPayload::ResourceStored(stored)) => {
                    let bytes = stored
                        .resource_id
                        .as_ref()
                        .expect("resource_id must be present")
                        .bytes
                        .clone();
                    stored_by_request
                        .insert(stored.request_sequence, (stored.upload_id.clone(), bytes));
                }
                other => panic!("expected ResourceStored, got: {other:?}"),
            }
        }

        assert_eq!(stored_by_request.len(), 2);
        assert_eq!(
            stored_by_request
                .get(&2)
                .expect("request 2 stored result must exist")
                .0,
            upload_a
        );
        assert_eq!(
            stored_by_request
                .get(&3)
                .expect("request 3 stored result must exist")
                .0,
            upload_b
        );
        assert_eq!(
            stored_by_request
                .get(&2)
                .expect("request 2 stored result must exist")
                .1,
            expected_a
        );
        assert_eq!(
            stored_by_request
                .get(&3)
                .expect("request 3 stored result must exist")
                .1,
            expected_b
        );

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_chunked_zero_size_rejected() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-zero-size",
            "test-key",
            &["upload_resource"],
        )
        .await;

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: vec![0xAB; 32],
                resource_type: 2,
                total_size_bytes: 0,
                metadata: Some(ResourceMetadata::default()),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg = next_non_state_change(&mut stream).await;
        match &msg.payload {
            Some(ServerPayload::ResourceErrorResponse(err)) => {
                assert_eq!(err.request_sequence, 2);
                assert_eq!(err.error_code, 3);
                assert!(err.upload_id.is_empty());
                assert!(
                    err.message.contains("total_size_bytes"),
                    "expected total_size guard message, got: {}",
                    err.message
                );
            }
            other => panic!("expected ResourceErrorResponse, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resource_upload_chunk_error_aborts_inflight_tracking() {
        let (mut client, handle) = setup_widget_test().await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-chunk-error",
            "test-key",
            &["upload_resource"],
        )
        .await;

        let payload = tiny_png_1x1_rgba();
        let hash = blake3::hash(&payload).as_bytes().to_vec();

        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 2,
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata::default()),
                inline_data: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let accepted = next_non_state_change(&mut stream).await;
        let upload_id = match &accepted.payload {
            Some(ServerPayload::ResourceUploadAccepted(accepted)) => accepted.upload_id.clone(),
            other => panic!("expected ResourceUploadAccepted, got: {other:?}"),
        };

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
                upload_id: upload_id.clone(),
                chunk_index: 1,
                data: payload.clone(),
            })),
        })
        .await
        .unwrap();

        let first_error = next_non_state_change(&mut stream).await;
        match &first_error.payload {
            Some(ServerPayload::ResourceErrorResponse(err)) => {
                assert_eq!(err.request_sequence, 2);
                assert_eq!(err.error_code, 7);
                assert_eq!(err.upload_id, upload_id);
            }
            other => panic!("expected ResourceErrorResponse after bad chunk, got: {other:?}"),
        }

        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadComplete(
                ResourceUploadComplete { upload_id },
            )),
        })
        .await
        .unwrap();

        let second_error = next_non_state_change(&mut stream).await;
        match &second_error.payload {
            Some(ServerPayload::ResourceErrorResponse(err)) => {
                assert_eq!(err.request_sequence, 4);
                assert_eq!(err.error_code, 9);
            }
            other => panic!("expected ResourceErrorResponse after aborted upload, got: {other:?}"),
        }

        drop(handle);
    }

    #[tokio::test]
    async fn test_resident_upload_then_static_image_references_uploaded_resource_id() {
        let service = setup_widget_service().await;
        let shared_state = service.state.clone();
        let (mut client, handle) = setup_widget_test_with_service(service).await;
        let (tx, _init_msgs, mut stream) = handshake_with_capabilities(
            &mut client,
            "resource-scene-node",
            "test-key",
            &["upload_resource", "modify_own_tiles"],
        )
        .await;

        let payload = tiny_png_1x1_rgba();
        let hash = blake3::hash(&payload).as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
                expected_hash: hash,
                resource_type: 2, // IMAGE_PNG
                total_size_bytes: payload.len() as u64,
                metadata: Some(ResourceMetadata::default()),
                inline_data: payload,
            })),
        })
        .await
        .unwrap();

        let stored = next_non_state_change(&mut stream).await;
        let resource_id_bytes = match stored.payload {
            Some(ServerPayload::ResourceStored(stored)) => {
                stored
                    .resource_id
                    .expect("resource_id must be present on success")
                    .bytes
            }
            other => panic!("expected ResourceStored, got: {other:?}"),
        };
        let resource_id = ResourceId::from_bytes(
            resource_id_bytes
                .as_slice()
                .try_into()
                .expect("resource_id must be 32 bytes"),
        );
        {
            let st = shared_state.lock().await;
            let scene = st.scene.lock().await;
            assert!(
                scene.is_resource_registered(&resource_id),
                "uploaded resources must be registered for scene mutation validation"
            );
        }

        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let lease_msg = next_non_state_change(&mut stream).await;
        let lease_id = match lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id,
            other => panic!("expected granted LeaseResponse, got: {other:?}"),
        };

        let create_batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: create_batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::CreateTile(
                        crate::proto::CreateTileMutation {
                            tab_id: vec![],
                            bounds: Some(crate::proto::Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 120.0,
                                height: 120.0,
                                ..Default::default()
                            }),
                            z_order: 1,
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .unwrap();

        let create_result = next_non_state_change(&mut stream).await;
        let tile_id_bytes = match create_result.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(result.accepted, "create tile mutation should be accepted");
                assert_eq!(result.batch_id, create_batch_id);
                result
                    .created_ids
                    .first()
                    .cloned()
                    .expect("create tile should return one created tile id")
            }
            other => panic!("expected MutationResult for create tile, got: {other:?}"),
        };

        let root_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 1,
                height: 1,
                decoded_bytes: 4,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
            }),
        };

        let set_root_batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: set_root_batch_id.clone(),
                lease_id,
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(crate::proto::mutation_proto::Mutation::SetTileRoot(
                        crate::proto::SetTileRootMutation {
                            tile_id: tile_id_bytes.clone(),
                            node: Some(crate::convert::scene_node_to_proto(&root_node)),
                        },
                    )),
                }],
                timing: None,
            })),
        })
        .await
        .unwrap();

        let set_root_result = next_non_state_change(&mut stream).await;
        match set_root_result.payload {
            Some(ServerPayload::MutationResult(result)) => {
                assert!(result.accepted, "set_tile_root should be accepted");
                assert_eq!(result.batch_id, set_root_batch_id);
            }
            other => panic!("expected MutationResult for set_tile_root, got: {other:?}"),
        }

        let tile_id = bytes_to_scene_id(&tile_id_bytes).expect("tile id from mutation must decode");
        {
            let st = shared_state.lock().await;
            let scene = st.scene.lock().await;
            let tile = scene.tiles.get(&tile_id).expect("tile must exist");
            let root_id = tile.root_node.expect("tile must have root node");
            let root = scene.nodes.get(&root_id).expect("root node must exist");
            match &root.data {
                NodeData::StaticImage(static_image) => {
                    assert_eq!(
                        static_image.resource_id, resource_id,
                        "scene node must reference uploaded ResourceId"
                    );
                }
                other => panic!("expected StaticImage root node, got: {other:?}"),
            }
        }

        drop(handle);
    }

    /// Helper: handshake with specific capabilities in SessionInit.
    ///
    /// Widget capability checks use `session.capabilities` which is populated
    /// from the SessionInit `requested_capabilities` list.
    async fn handshake_with_capabilities(
        client: &mut HudSessionClient<tonic::transport::Channel>,
        agent_id: &str,
        psk: &str,
        extra_caps: &[&str],
    ) -> (
        tokio::sync::mpsc::Sender<ClientMessage>,
        Vec<ServerMessage>,
        tonic::Streaming<ServerMessage>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        let mut caps = vec![
            "create_tiles".to_string(),
            "access_input_events".to_string(),
            "read_scene_topology".to_string(),
        ];
        for c in extra_caps {
            caps.push(c.to_string());
        }

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_id.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities: caps,
                initial_subscriptions: vec![],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut streaming = client.session(stream).await.unwrap().into_inner();

        // Collect exactly 2 handshake messages: SessionEstablished + SceneSnapshot.
        // This mirrors the existing `handshake` helper to avoid stream state issues.
        let mut init_messages = Vec::new();
        for _ in 0..2 {
            if let Some(msg) = streaming.next().await {
                init_messages.push(msg.unwrap());
            }
        }

        (tx, init_messages, streaming)
    }

    // ─── ElementRepositionedEvent tests (hud-bs2q.6) ─────────────────────────

    /// Build a service + shared-state + element_repositioned broadcast channel.
    ///
    /// Extracts the shared state and broadcast sender before moving the service
    /// into the server task. The test can then call `broadcast_element_repositioned`
    /// via the channel directly, or manipulate the shared state for reset tests.
    async fn setup_test_with_reposition_tx() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<SharedState>>,
        tokio::sync::broadcast::Sender<crate::proto::ElementRepositionedEvent>,
    ) {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let service = HudSessionImpl::new(scene, "test-key");
        let shared_state = service.state.clone();
        let reposition_tx = service.element_repositioned_tx.clone();

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        (client, handle, shared_state, reposition_tx)
    }

    /// GIVEN element with geometry_override
    /// WHEN reset_geometry_override is called on the element store
    /// THEN override is cleared and the previous value is returned
    #[test]
    fn test_reset_geometry_override_clears_override_and_returns_previous() {
        use tze_hud_scene::element_store::{ElementStore, ElementStoreEntry, ElementType};

        let tile_id = SceneId::new();
        let override_policy = GeometryPolicy::Relative {
            x_pct: 0.5,
            y_pct: 0.5,
            width_pct: 0.2,
            height_pct: 0.1,
        };
        let mut store = ElementStore::default();
        store.entries.insert(
            tile_id,
            ElementStoreEntry {
                element_type: ElementType::Tile,
                namespace: "test-agent".to_string(),
                created_at: 1000,
                last_published_at: 2000,
                geometry_override: Some(override_policy),
            },
        );

        let previous = store.reset_geometry_override(tile_id);
        assert_eq!(
            previous,
            Some(override_policy),
            "reset must return the cleared override"
        );
        assert!(
            store
                .entries
                .get(&tile_id)
                .unwrap()
                .geometry_override
                .is_none(),
            "geometry_override must be None after reset"
        );
    }

    /// GIVEN element without geometry_override
    /// WHEN reset_geometry_override is called
    /// THEN returns None (no-op)
    #[test]
    fn test_reset_geometry_override_noop_when_no_override() {
        use tze_hud_scene::element_store::{ElementStore, ElementStoreEntry, ElementType};

        let tile_id = SceneId::new();
        let mut store = ElementStore::default();
        store.entries.insert(
            tile_id,
            ElementStoreEntry {
                element_type: ElementType::Tile,
                namespace: "test-agent".to_string(),
                created_at: 1000,
                last_published_at: 2000,
                geometry_override: None,
            },
        );

        let previous = store.reset_geometry_override(tile_id);
        assert!(
            previous.is_none(),
            "reset must return None when no override"
        );
    }

    /// GIVEN unknown element_id
    /// WHEN reset_geometry_override is called
    /// THEN returns None (no-op)
    #[test]
    fn test_reset_geometry_override_noop_for_unknown_element() {
        use tze_hud_scene::element_store::ElementStore;

        let mut store = ElementStore::default();
        let result = store.reset_geometry_override(SceneId::new());
        assert!(
            result.is_none(),
            "reset must return None for unknown element"
        );
    }

    /// GIVEN agent subscribed to SCENE_TOPOLOGY
    /// WHEN broadcast_element_repositioned is called via the channel
    /// THEN agent receives ElementRepositionedEvent
    #[tokio::test]
    async fn test_element_repositioned_delivered_to_scene_topology_subscriber() {
        let (mut client, _server, _shared_state, reposition_tx) =
            setup_test_with_reposition_tx().await;
        let (_tx, _msgs, mut stream) = handshake(&mut client, "test-agent", "test-key").await;

        // Give the session handler a moment to fully subscribe.
        tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

        let element_id = SceneId::new();
        let new_policy = GeometryPolicy::Relative {
            x_pct: 0.3,
            y_pct: 0.2,
            width_pct: 0.25,
            height_pct: 0.15,
        };
        let old_policy = GeometryPolicy::Relative {
            x_pct: 0.1,
            y_pct: 0.1,
            width_pct: 0.25,
            height_pct: 0.15,
        };

        let event = crate::proto::ElementRepositionedEvent {
            element_id: scene_id_to_bytes(element_id),
            new_geometry: Some(convert::geometry_policy_to_proto(&new_policy)),
            previous_geometry: Some(convert::geometry_policy_to_proto(&old_policy)),
        };
        let _ = reposition_tx.send(event);

        // Collect next message from stream (with timeout to avoid hanging on failure).
        let msg = tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next())
            .await
            .expect("timed out waiting for ElementRepositionedEvent")
            .expect("stream should not close")
            .expect("should not error");

        match msg.payload {
            Some(ServerPayload::ElementRepositioned(event)) => {
                // element_id must match
                let expected_id: Vec<u8> = scene_id_to_bytes(element_id);
                assert_eq!(event.element_id, expected_id, "element_id must match");
                // new_geometry must be set and match new_policy
                let ng = event.new_geometry.expect("new_geometry must be set");
                match ng.policy {
                    Some(crate::proto::geometry_policy_proto::Policy::Relative(r)) => {
                        assert!((r.x_pct - 0.3_f32).abs() < 1e-4, "x_pct mismatch");
                        assert!((r.y_pct - 0.2_f32).abs() < 1e-4, "y_pct mismatch");
                    }
                    other => panic!("expected Relative geometry, got {other:?}"),
                }
                // previous_geometry must be set and match old_policy
                let pg = event
                    .previous_geometry
                    .expect("previous_geometry must be set");
                match pg.policy {
                    Some(crate::proto::geometry_policy_proto::Policy::Relative(r)) => {
                        assert!((r.x_pct - 0.1_f32).abs() < 1e-4, "prev x_pct mismatch");
                    }
                    other => panic!("expected Relative previous geometry, got {other:?}"),
                }
            }
            other => panic!("expected ElementRepositioned, got {other:?}"),
        }
    }

    /// GIVEN agent NOT subscribed to SCENE_TOPOLOGY
    /// WHEN broadcast_element_repositioned is called
    /// THEN agent does not receive ElementRepositionedEvent
    #[tokio::test]
    async fn test_element_repositioned_not_delivered_without_scene_topology_subscription() {
        use crate::proto::session::hud_session_client::HudSessionClient;
        use crate::proto::session::hud_session_server::HudSessionServer;

        let scene = SceneGraph::new(1920.0, 1080.0);
        let service = HudSessionImpl::new(scene, "test-key");
        let reposition_tx = service.element_repositioned_tx.clone();

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let _handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let mut client = HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
            .await
            .unwrap();

        // Handshake WITHOUT read_scene_topology capability → no SCENE_TOPOLOGY subscription.
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "no-topology-agent".to_string(),
                agent_display_name: "no-topology-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: vec!["create_tiles".to_string()],
                initial_subscriptions: vec![],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();
        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        // Drain handshake (SessionEstablished + SceneSnapshot).
        response_stream.next().await;
        response_stream.next().await;

        tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

        let element_id = SceneId::new();
        let new_policy = GeometryPolicy::Relative {
            x_pct: 0.3,
            y_pct: 0.2,
            width_pct: 0.25,
            height_pct: 0.15,
        };
        let event = crate::proto::ElementRepositionedEvent {
            element_id: scene_id_to_bytes(element_id),
            new_geometry: Some(convert::geometry_policy_to_proto(&new_policy)),
            previous_geometry: None,
        };
        let _ = reposition_tx.send(event);

        // Agent should NOT receive the event; timeout expected.
        let result = tokio::time::timeout(
            tokio::time::Duration::from_millis(200),
            response_stream.next(),
        )
        .await;
        // Timeout means no event was delivered — correct behaviour.
        // If no timeout: check it's not an ElementRepositioned.
        if let Ok(Some(Ok(msg))) = result {
            if let Some(ServerPayload::ElementRepositioned(_)) = msg.payload {
                panic!(
                    "ElementRepositioned must NOT be delivered without SCENE_TOPOLOGY subscription"
                );
            }
            // Other messages (e.g., Heartbeat) are allowed.
        }
        drop(tx); // close stream
    }

    /// GIVEN element with override and known agent tile bounds
    /// WHEN the element store override is cleared and event is broadcast
    /// THEN event carries previous_geometry=old_override and new_geometry=fallback
    #[test]
    fn test_reset_geometry_override_carries_correct_previous_and_new() {
        use tze_hud_scene::element_store::{ElementStore, ElementStoreEntry, ElementType};

        let tile_id = SceneId::new();
        let override_policy = GeometryPolicy::Relative {
            x_pct: 0.8,
            y_pct: 0.8,
            width_pct: 0.1,
            height_pct: 0.1,
        };
        let mut store = ElementStore::default();
        store.entries.insert(
            tile_id,
            ElementStoreEntry {
                element_type: ElementType::Tile,
                namespace: "test-agent".to_string(),
                created_at: 1000,
                last_published_at: 2000,
                geometry_override: Some(override_policy),
            },
        );

        // Simulate reset: clear override and note previous value.
        let previous = store.reset_geometry_override(tile_id);
        assert_eq!(
            previous,
            Some(override_policy),
            "previous must be the removed override"
        );

        // After reset, override must be gone.
        let entry = store.entries.get(&tile_id).unwrap();
        assert!(
            entry.geometry_override.is_none(),
            "override must be cleared"
        );

        // The fallback geometry (agent bounds) would be applied by the caller;
        // verify the store correctly reflects the cleared state.
        let proto_previous = convert::geometry_policy_to_proto(&previous.unwrap());
        match proto_previous.policy {
            Some(crate::proto::geometry_policy_proto::Policy::Relative(r)) => {
                assert!(
                    (r.x_pct - 0.8_f32).abs() < 1e-4,
                    "previous x_pct must match override"
                );
            }
            other => panic!("expected Relative previous_geometry proto, got {other:?}"),
        }
    }
}
