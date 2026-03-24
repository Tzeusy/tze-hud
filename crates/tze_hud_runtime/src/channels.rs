//! # channels
//!
//! Bounded inter-thread communication channels per spec §Channel Topology.
//!
//! All inter-thread communication uses bounded channels. No unbounded queues.
//!
//! ## Channel inventory
//!
//! | Channel | Type | Semantics |
//! |---------|------|-----------|
//! | InputEvent | ring buffer | drop-oldest on full |
//! | SceneLocalPatch | ring buffer | drop-oldest on full |
//! | SceneEventEphemeral | ring buffer | drop-oldest on full |
//! | TelemetryRecord | ring buffer | drop-oldest on full |
//! | SceneEventTransactional | backpressure | block sender (capacity 256) |
//! | SceneEventStateStream | coalesce-key | replace by (tile_id, event_kind) key |
//! | FrameReadySignal | tokio::sync::watch | latest-value wins |
//!
//! ## Implementation strategy
//!
//! - Ring buffers: `Arc<Mutex<VecDeque<T>>>` with fixed capacity. On push,
//!   if full, drop the oldest element and count the overflow in telemetry.
//! - Backpressure channel: `tokio::sync::mpsc` with fixed capacity. The async
//!   `send().await` applies natural backpressure — the sender blocks when full.
//! - Coalesce-key channel: a keyed store (`HashMap<(TileId, EventKind), V>`)
//!   plus a `tokio::sync::Notify` for wakeup. Pending entries for the same
//!   key are replaced, so intermediate states are skipped, not dropped.
//! - FrameReadySignal: `tokio::sync::watch` — the receiver always sees the
//!   most-recent value.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, watch, Mutex, Notify};
use tze_hud_scene::types::SceneId;

// ─── Channel capacities ───────────────────────────────────────────────────────

/// Capacity for InputEvent ring buffer.
pub const INPUT_EVENT_CAPACITY: usize = 256;

/// Capacity for SceneLocalPatch ring buffer.
pub const SCENE_LOCAL_PATCH_CAPACITY: usize = 128;

/// Capacity for SceneEventEphemeral ring buffer.
pub const SCENE_EVENT_EPHEMERAL_CAPACITY: usize = 256;

/// Capacity for TelemetryRecord ring buffer.
pub const TELEMETRY_RECORD_CAPACITY: usize = 512;

/// Capacity for SceneEventTransactional backpressure channel.
pub const SCENE_EVENT_TRANSACTIONAL_CAPACITY: usize = 256;

/// Capacity for SceneEventStateStream coalesce-key channel.
pub const SCENE_EVENT_STATE_STREAM_CAPACITY: usize = 512;

// ─── Overflow counter ─────────────────────────────────────────────────────────

/// Shared atomic overflow counter for ring buffers.
/// Incremented when a ring buffer drops its oldest entry to make room.
#[derive(Clone, Debug, Default)]
pub struct OverflowCounters {
    pub input_event: Arc<AtomicU64>,
    pub scene_local_patch: Arc<AtomicU64>,
    pub scene_event_ephemeral: Arc<AtomicU64>,
    pub telemetry_record: Arc<AtomicU64>,
}

impl OverflowCounters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total drops across all ring buffers.
    pub fn total_drops(&self) -> u64 {
        self.input_event.load(Ordering::Relaxed)
            + self.scene_local_patch.load(Ordering::Relaxed)
            + self.scene_event_ephemeral.load(Ordering::Relaxed)
            + self.telemetry_record.load(Ordering::Relaxed)
    }
}

// ─── Ring buffer ─────────────────────────────────────────────────────────────

/// A bounded ring buffer that drops the oldest entry when full.
///
/// All operations are O(1). The overflow counter is incremented each time
/// an element is evicted.
pub struct RingBuffer<T> {
    inner: Arc<Mutex<RingBufferInner<T>>>,
    overflow: Arc<AtomicU64>,
}

struct RingBufferInner<T> {
    buf: VecDeque<T>,
    capacity: usize,
}

impl<T: Send + 'static> RingBuffer<T> {
    /// Create a new ring buffer with the given capacity.
    pub fn new(capacity: usize, overflow: Arc<AtomicU64>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RingBufferInner {
                buf: VecDeque::with_capacity(capacity),
                capacity,
            })),
            overflow,
        }
    }

    /// Push an element. If the buffer is full, drops the oldest element and
    /// increments the overflow counter.
    pub async fn push(&self, item: T) {
        let mut inner = self.inner.lock().await;
        if inner.buf.len() == inner.capacity {
            inner.buf.pop_front(); // drop oldest
            self.overflow.fetch_add(1, Ordering::Relaxed);
        }
        inner.buf.push_back(item);
    }

    /// Drain all pending items. Returns them in arrival order (oldest first).
    pub async fn drain_all(&self) -> Vec<T> {
        let mut inner = self.inner.lock().await;
        inner.buf.drain(..).collect()
    }

    /// Pop one item (oldest). Returns `None` if empty.
    pub async fn pop(&self) -> Option<T> {
        let mut inner = self.inner.lock().await;
        inner.buf.pop_front()
    }

    /// Current number of pending items.
    pub async fn len(&self) -> usize {
        self.inner.lock().await.buf.len()
    }

    /// Returns `true` when empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.buf.is_empty()
    }

    /// Configured capacity.
    pub async fn capacity(&self) -> usize {
        self.inner.lock().await.capacity
    }

    /// Clone the inner `Arc` so a second owner can read from the same buffer.
    pub fn clone_handle(&self) -> RingBuffer<T> {
        RingBuffer {
            inner: Arc::clone(&self.inner),
            overflow: Arc::clone(&self.overflow),
        }
    }
}

// ─── Backpressure channel (SceneEventTransactional) ──────────────────────────

/// A bounded, backpressure-applying channel.
///
/// The sender blocks (await) when the channel reaches capacity. Messages are
/// NEVER dropped — this is the guarantee for transactional events.
pub struct BackpressureSender<T> {
    tx: mpsc::Sender<T>,
}

pub struct BackpressureReceiver<T> {
    rx: mpsc::Receiver<T>,
}

impl<T: Send + 'static> BackpressureSender<T> {
    /// Send an item. Blocks (awaits) if the channel is at capacity.
    /// Returns `Err` only if the receiver has been dropped.
    pub async fn send(&self, item: T) -> Result<(), mpsc::error::SendError<T>> {
        self.tx.send(item).await
    }

    /// Non-blocking attempt to send. Returns `Err` immediately if full or closed.
    pub fn try_send(&self, item: T) -> Result<(), mpsc::error::TrySendError<T>> {
        self.tx.try_send(item)
    }
}

impl<T: Send + 'static> BackpressureReceiver<T> {
    /// Receive one item. Blocks (awaits) until an item is available.
    pub async fn recv(&mut self) -> Option<T> {
        self.rx.recv().await
    }

    /// Non-blocking receive. Returns `None` immediately if empty.
    pub fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

/// Create a bounded backpressure channel with the given capacity.
pub fn backpressure_channel<T: Send + 'static>(
    capacity: usize,
) -> (BackpressureSender<T>, BackpressureReceiver<T>) {
    let (tx, rx) = mpsc::channel(capacity);
    (BackpressureSender { tx }, BackpressureReceiver { rx })
}

// ─── Coalesce-key channel (SceneEventStateStream) ────────────────────────────

/// The key used to coalesce state-stream events.
/// Two events with the same (tile_id, event_kind) are merged: the newer
/// replaces the older.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateStreamKey {
    pub tile_id: SceneId,
    pub event_kind: StateStreamEventKind,
}

/// Discriminant for the kind of state-stream event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateStreamEventKind {
    TileUpdate,
    ScenePatch,
    LeaseUpdate,
    Custom(u32),
}

/// A state-stream event with its coalesce key embedded.
pub trait CoalesceKeyed {
    fn key(&self) -> StateStreamKey;
}

/// Coalesce-key channel writer side.
pub struct CoalesceKeySender<T> {
    inner: Arc<Mutex<CoalesceKeyInner<T>>>,
    notify: Arc<Notify>,
    capacity: usize,
}

/// Coalesce-key channel reader side.
pub struct CoalesceKeyReceiver<T> {
    inner: Arc<Mutex<CoalesceKeyInner<T>>>,
    notify: Arc<Notify>,
}

struct CoalesceKeyInner<T> {
    /// Pending entries by key — replacement is O(1) hash-map write.
    /// Insertion order is tracked to preserve roughly-FIFO ordering
    /// when draining, while still coalescing duplicates.
    map: HashMap<StateStreamKey, T>,
    /// Order in which keys first appeared (oldest first).
    order: VecDeque<StateStreamKey>,
    /// Max number of distinct keys before eviction. Stored here for
    /// introspection / future receiver-side capacity queries.
    #[allow(dead_code)]
    capacity: usize,
}

impl<T: CoalesceKeyed + Send + 'static> CoalesceKeySender<T> {
    /// Insert or replace a pending entry for the given key.
    ///
    /// If the map is at capacity and this is a new key, the oldest pending
    /// entry is evicted to make room (effectively applying back-pressure by
    /// discarding stale state — this matches the spec: "intermediate states
    /// skipped, not dropped").
    pub async fn send(&self, item: T) {
        let key = item.key();
        let mut inner = self.inner.lock().await;

        if inner.map.contains_key(&key) {
            // Replace in-place — order entry already exists.
            inner.map.insert(key, item);
        } else {
            // New key: evict oldest if at capacity.
            if inner.map.len() >= self.capacity {
                if let Some(oldest_key) = inner.order.pop_front() {
                    inner.map.remove(&oldest_key);
                }
            }
            inner.order.push_back(key.clone());
            inner.map.insert(key, item);
        }

        // Wake any waiting receiver.
        self.notify.notify_one();
    }

    /// Number of distinct pending keys.
    pub async fn pending(&self) -> usize {
        self.inner.lock().await.map.len()
    }
}

impl<T: CoalesceKeyed + Send + 'static> CoalesceKeyReceiver<T> {
    /// Drain all pending entries. Returns them oldest-key-first.
    pub async fn drain_all(&mut self) -> Vec<T> {
        let mut inner = self.inner.lock().await;
        let mut out = Vec::with_capacity(inner.order.len());
        while let Some(key) = inner.order.pop_front() {
            if let Some(v) = inner.map.remove(&key) {
                out.push(v);
            }
        }
        out
    }

    /// Wait until at least one entry is available, then drain all.
    pub async fn recv_batch(&mut self) -> Vec<T> {
        loop {
            // Check without holding the lock.
            let notify = Arc::clone(&self.notify);
            {
                let inner = self.inner.lock().await;
                if !inner.map.is_empty() {
                    drop(inner);
                    return self.drain_all().await;
                }
            }
            notify.notified().await;
        }
    }

    /// Number of distinct pending keys.
    pub async fn pending(&self) -> usize {
        self.inner.lock().await.map.len()
    }
}

/// Create a coalesce-key channel with the given capacity (max distinct keys).
pub fn coalesce_key_channel<T: CoalesceKeyed + Send + 'static>(
    capacity: usize,
) -> (CoalesceKeySender<T>, CoalesceKeyReceiver<T>) {
    let inner = Arc::new(Mutex::new(CoalesceKeyInner {
        map: HashMap::with_capacity(capacity),
        order: VecDeque::with_capacity(capacity),
        capacity,
    }));
    let notify = Arc::new(Notify::new());
    (
        CoalesceKeySender {
            inner: Arc::clone(&inner),
            notify: Arc::clone(&notify),
            capacity,
        },
        CoalesceKeyReceiver { inner, notify },
    )
}

// ─── FrameReadySignal ─────────────────────────────────────────────────────────

/// Signal from the compositor thread to the main thread that a frame is ready
/// to be presented. Uses `tokio::sync::watch` — only the latest value matters.
///
/// Value `true` means "compositor has submitted GPU work, call surface.present()".
/// Value `false` is the initial state (no frame ready yet).
pub type FrameReadyTx = watch::Sender<bool>;
pub type FrameReadyRx = watch::Receiver<bool>;

/// Create a FrameReadySignal pair (sender lives on compositor thread, receiver
/// on main thread).
pub fn frame_ready_channel() -> (FrameReadyTx, FrameReadyRx) {
    watch::channel(false)
}

// ─── Convenience bundle ───────────────────────────────────────────────────────

/// Placeholder types for the per-channel message payloads.
/// Real message types come from `tze_hud_scene` and `tze_hud_telemetry`;
/// the channel module just wires the topology.

/// An OS input event drained from the winit event loop.
#[derive(Debug, Clone)]
pub struct InputEvent {
    /// Monotonic timestamp of when the event was received (ns since epoch).
    pub timestamp_ns: u64,
    /// Raw event kind (simplified for the topology scaffold).
    pub kind: InputEventKind,
}

#[derive(Debug, Clone)]
pub enum InputEventKind {
    KeyPress { key: u32 },
    KeyRelease { key: u32 },
    PointerMove { x: f32, y: f32 },
    PointerPress { x: f32, y: f32, button: u8 },
    PointerRelease { x: f32, y: f32, button: u8 },
    Resize { width: u32, height: u32 },
    CloseRequested,
}

/// A local patch applied to the scene for immediate visual feedback
/// (e.g., hit-region press state).
#[derive(Debug, Clone)]
pub struct SceneLocalPatch {
    pub node_id: SceneId,
    pub patch: LocalPatchKind,
}

#[derive(Debug, Clone)]
pub enum LocalPatchKind {
    SetHovered(bool),
    SetPressed(bool),
}

/// An ephemeral scene event (hover, cursor trails, interim speech).
/// May be dropped if the ring buffer is full — latest wins.
#[derive(Debug, Clone)]
pub struct SceneEventEphemeral {
    pub tile_id: SceneId,
    pub kind: EphemeralEventKind,
}

#[derive(Debug, Clone)]
pub enum EphemeralEventKind {
    CursorMove { x: f32, y: f32 },
    InterimSpeech { text: String },
}

/// A transactional scene event (create tile, grant lease, switch tab).
/// MUST NOT be dropped — backpressure channel enforces this.
#[derive(Debug, Clone)]
pub struct SceneEventTransactional {
    pub seq: u64,
    pub kind: TransactionalEventKind,
}

#[derive(Debug, Clone)]
pub enum TransactionalEventKind {
    CreateTile { tile_id: SceneId },
    DestroyTile { tile_id: SceneId },
    GrantLease { agent_id: String, duration_ms: u64 },
    RevokeAllLeases,
}

/// A state-stream event. Intermediate states with the same key are replaced.
#[derive(Debug, Clone)]
pub struct SceneEventStateStream {
    pub tile_id: SceneId,
    pub event_kind: StateStreamEventKind,
    pub payload: StateStreamPayload,
}

#[derive(Debug, Clone)]
pub enum StateStreamPayload {
    TileUpdate { z_index: u32, opacity: f32 },
    ScenePatch { patch_seq: u64 },
    LeaseUpdate { remaining_ms: u64 },
    Custom { tag: u32, data: Vec<u8> },
}

impl CoalesceKeyed for SceneEventStateStream {
    fn key(&self) -> StateStreamKey {
        StateStreamKey {
            tile_id: self.tile_id,
            event_kind: self.event_kind.clone(),
        }
    }
}

/// A structured telemetry record emitted per frame or per session event.
#[derive(Debug, Clone)]
pub struct TelemetryRecord {
    pub frame_number: u64,
    pub frame_time_us: u64,
    pub overflow_drops: u64,
}

// ─── Full channel set (created once at startup) ───────────────────────────────

/// The complete set of inter-thread channels created at runtime startup.
///
/// Callers split these apart and hand each side to the appropriate thread.
pub struct ChannelSet {
    // Input events: main → compositor
    pub input_tx: RingBuffer<InputEvent>,
    pub input_rx: RingBuffer<InputEvent>,

    // Scene local patches: main → compositor (immediate local feedback)
    pub local_patch_tx: RingBuffer<SceneLocalPatch>,
    pub local_patch_rx: RingBuffer<SceneLocalPatch>,

    // Ephemeral events: network → compositor (droppable)
    pub ephemeral_tx: RingBuffer<SceneEventEphemeral>,
    pub ephemeral_rx: RingBuffer<SceneEventEphemeral>,

    // Telemetry records: compositor → telemetry thread
    pub telemetry_tx: RingBuffer<TelemetryRecord>,
    pub telemetry_rx: RingBuffer<TelemetryRecord>,

    // Transactional events: network → compositor (must not drop)
    pub transactional_tx: BackpressureSender<SceneEventTransactional>,
    pub transactional_rx: BackpressureReceiver<SceneEventTransactional>,

    // State-stream events: network → compositor (coalesce by key)
    pub state_stream_tx: CoalesceKeySender<SceneEventStateStream>,
    pub state_stream_rx: CoalesceKeyReceiver<SceneEventStateStream>,

    // Frame-ready signal: compositor → main thread
    pub frame_ready_tx: FrameReadyTx,
    pub frame_ready_rx: FrameReadyRx,

    /// Overflow counters for all ring buffers — sharable snapshot for telemetry.
    pub overflow: OverflowCounters,
}

impl ChannelSet {
    /// Create the full channel set with spec-mandated capacities.
    pub fn new() -> Self {
        let overflow = OverflowCounters::new();

        let input_buf = RingBuffer::new(
            INPUT_EVENT_CAPACITY,
            Arc::clone(&overflow.input_event),
        );
        let local_patch_buf = RingBuffer::new(
            SCENE_LOCAL_PATCH_CAPACITY,
            Arc::clone(&overflow.scene_local_patch),
        );
        let ephemeral_buf = RingBuffer::new(
            SCENE_EVENT_EPHEMERAL_CAPACITY,
            Arc::clone(&overflow.scene_event_ephemeral),
        );
        let telemetry_buf = RingBuffer::new(
            TELEMETRY_RECORD_CAPACITY,
            Arc::clone(&overflow.telemetry_record),
        );

        let (transactional_tx, transactional_rx) =
            backpressure_channel(SCENE_EVENT_TRANSACTIONAL_CAPACITY);
        let (state_stream_tx, state_stream_rx) =
            coalesce_key_channel(SCENE_EVENT_STATE_STREAM_CAPACITY);
        let (frame_ready_tx, frame_ready_rx) = frame_ready_channel();

        ChannelSet {
            input_tx: input_buf.clone_handle(),
            input_rx: input_buf,
            local_patch_tx: local_patch_buf.clone_handle(),
            local_patch_rx: local_patch_buf,
            ephemeral_tx: ephemeral_buf.clone_handle(),
            ephemeral_rx: ephemeral_buf,
            telemetry_tx: telemetry_buf.clone_handle(),
            telemetry_rx: telemetry_buf,
            transactional_tx,
            transactional_rx,
            state_stream_tx,
            state_stream_rx,
            frame_ready_tx,
            frame_ready_rx,
            overflow,
        }
    }
}

impl Default for ChannelSet {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Ring buffer ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ring_buffer_drop_oldest_when_full() {
        let overflow = Arc::new(AtomicU64::new(0));
        let buf: RingBuffer<u32> = RingBuffer::new(3, Arc::clone(&overflow));

        buf.push(1).await;
        buf.push(2).await;
        buf.push(3).await;
        // Buffer full — push 4 should evict 1
        buf.push(4).await;

        assert_eq!(overflow.load(Ordering::Relaxed), 1, "one drop counted");
        let items = buf.drain_all().await;
        assert_eq!(items, vec![2, 3, 4], "oldest evicted");
    }

    #[tokio::test]
    async fn ring_buffer_no_drop_below_capacity() {
        let overflow = Arc::new(AtomicU64::new(0));
        let buf: RingBuffer<u32> = RingBuffer::new(4, Arc::clone(&overflow));

        buf.push(10).await;
        buf.push(20).await;

        assert_eq!(overflow.load(Ordering::Relaxed), 0);
        assert_eq!(buf.len().await, 2);
    }

    #[tokio::test]
    async fn ring_buffer_clone_handle_shares_state() {
        let overflow = Arc::new(AtomicU64::new(0));
        let buf: RingBuffer<u32> = RingBuffer::new(8, Arc::clone(&overflow));
        let reader = buf.clone_handle();

        buf.push(42).await;
        let item = reader.pop().await;
        assert_eq!(item, Some(42));
    }

    #[tokio::test]
    async fn ring_buffer_capacity_assertion_input_event() {
        let overflow = Arc::new(AtomicU64::new(0));
        let buf: RingBuffer<InputEvent> = RingBuffer::new(INPUT_EVENT_CAPACITY, overflow);
        assert_eq!(buf.capacity().await, INPUT_EVENT_CAPACITY);
        assert_eq!(INPUT_EVENT_CAPACITY, 256);
    }

    #[tokio::test]
    async fn ring_buffer_capacity_assertion_ephemeral() {
        let overflow = Arc::new(AtomicU64::new(0));
        let buf: RingBuffer<SceneEventEphemeral> =
            RingBuffer::new(SCENE_EVENT_EPHEMERAL_CAPACITY, overflow);
        assert_eq!(buf.capacity().await, SCENE_EVENT_EPHEMERAL_CAPACITY);
    }

    #[tokio::test]
    async fn ring_buffer_capacity_assertion_telemetry() {
        let overflow = Arc::new(AtomicU64::new(0));
        let buf: RingBuffer<TelemetryRecord> =
            RingBuffer::new(TELEMETRY_RECORD_CAPACITY, overflow);
        assert_eq!(buf.capacity().await, TELEMETRY_RECORD_CAPACITY);
    }

    // ── Backpressure channel ─────────────────────────────────────────────────

    #[tokio::test]
    async fn backpressure_channel_capacity_assertion() {
        let (tx, _rx): (
            BackpressureSender<SceneEventTransactional>,
            BackpressureReceiver<SceneEventTransactional>,
        ) = backpressure_channel(SCENE_EVENT_TRANSACTIONAL_CAPACITY);
        assert_eq!(SCENE_EVENT_TRANSACTIONAL_CAPACITY, 256);
        // Fill the channel to capacity using try_send
        for i in 0..SCENE_EVENT_TRANSACTIONAL_CAPACITY {
            let ev = SceneEventTransactional {
                seq: i as u64,
                kind: TransactionalEventKind::RevokeAllLeases,
            };
            tx.try_send(ev).expect("should not be full yet");
        }
        // One more should fail (channel full, not dropped!)
        let ev = SceneEventTransactional {
            seq: 9999,
            kind: TransactionalEventKind::RevokeAllLeases,
        };
        assert!(tx.try_send(ev).is_err(), "channel must apply backpressure");
    }

    #[tokio::test]
    async fn backpressure_channel_sends_and_receives() {
        let (tx, mut rx) = backpressure_channel::<SceneEventTransactional>(4);
        let ev = SceneEventTransactional {
            seq: 1,
            kind: TransactionalEventKind::RevokeAllLeases,
        };
        tx.send(ev).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.seq, 1);
    }

    // ── Coalesce-key channel ─────────────────────────────────────────────────

    fn make_state_stream_event(tile_id: SceneId, kind: StateStreamEventKind) -> SceneEventStateStream {
        SceneEventStateStream {
            tile_id,
            event_kind: kind,
            payload: StateStreamPayload::ScenePatch { patch_seq: 0 },
        }
    }

    #[tokio::test]
    async fn coalesce_key_channel_capacity_assertion() {
        assert_eq!(SCENE_EVENT_STATE_STREAM_CAPACITY, 512);
    }

    #[tokio::test]
    async fn coalesce_key_replaces_pending_entry_for_same_key() {
        let (tx, mut rx) = coalesce_key_channel::<SceneEventStateStream>(32);
        let tile_id = SceneId::new();

        // Send two events with the same key — second should replace first.
        let ev1 = SceneEventStateStream {
            tile_id,
            event_kind: StateStreamEventKind::TileUpdate,
            payload: StateStreamPayload::TileUpdate {
                z_index: 1,
                opacity: 0.5,
            },
        };
        let ev2 = SceneEventStateStream {
            tile_id,
            event_kind: StateStreamEventKind::TileUpdate,
            payload: StateStreamPayload::TileUpdate {
                z_index: 1,
                opacity: 0.9, // this is the one that should survive
            },
        };

        tx.send(ev1).await;
        tx.send(ev2).await;

        let batch = rx.drain_all().await;
        assert_eq!(batch.len(), 1, "coalesced to one entry");
        match &batch[0].payload {
            StateStreamPayload::TileUpdate { opacity, .. } => {
                assert!(
                    (*opacity - 0.9).abs() < 0.001,
                    "latest value survives, got {opacity}"
                );
            }
            _ => panic!("unexpected payload"),
        }
    }

    #[tokio::test]
    async fn coalesce_key_different_keys_are_preserved() {
        let (tx, mut rx) = coalesce_key_channel::<SceneEventStateStream>(32);
        let tile_a = SceneId::new();
        let tile_b = SceneId::new();

        tx.send(make_state_stream_event(tile_a, StateStreamEventKind::TileUpdate))
            .await;
        tx.send(make_state_stream_event(tile_b, StateStreamEventKind::ScenePatch))
            .await;

        let batch = rx.drain_all().await;
        assert_eq!(batch.len(), 2, "two different keys → two entries");
    }

    #[tokio::test]
    async fn coalesce_key_evicts_oldest_when_at_capacity() {
        let capacity = 4;
        let (tx, mut rx) = coalesce_key_channel::<SceneEventStateStream>(capacity);

        let ids: Vec<SceneId> = (0..capacity + 1).map(|_| SceneId::new()).collect();
        for id in &ids[..capacity] {
            tx.send(make_state_stream_event(*id, StateStreamEventKind::TileUpdate))
                .await;
        }
        // Adding one more new key at capacity should evict the oldest.
        tx.send(make_state_stream_event(ids[capacity], StateStreamEventKind::TileUpdate))
            .await;

        let batch = rx.drain_all().await;
        assert_eq!(
            batch.len(),
            capacity,
            "still at capacity after eviction (one old entry replaced)"
        );
        // The first key should be absent (evicted).
        let found_first = batch.iter().any(|e| e.tile_id == ids[0]);
        assert!(!found_first, "oldest entry was evicted");
    }

    // ── FrameReadySignal ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn frame_ready_signal_latest_wins() {
        let (tx, mut rx) = frame_ready_channel();

        // Send two values in quick succession; receiver should see latest.
        tx.send(true).unwrap();
        tx.send(false).unwrap();

        assert!(!*rx.borrow_and_update(), "latest value is false");
    }

    #[tokio::test]
    async fn frame_ready_signal_initial_state() {
        let (_tx, rx) = frame_ready_channel();
        assert!(!*rx.borrow(), "initial state is false");
    }

    // ── ChannelSet construction ──────────────────────────────────────────────

    #[tokio::test]
    async fn channel_set_new_initializes_all_channels() {
        let cs = ChannelSet::new();
        assert_eq!(cs.input_tx.capacity().await, INPUT_EVENT_CAPACITY);
        assert_eq!(cs.local_patch_tx.capacity().await, SCENE_LOCAL_PATCH_CAPACITY);
        assert_eq!(cs.ephemeral_tx.capacity().await, SCENE_EVENT_EPHEMERAL_CAPACITY);
        assert_eq!(cs.telemetry_tx.capacity().await, TELEMETRY_RECORD_CAPACITY);
    }

    #[tokio::test]
    async fn overflow_counters_total_drops_aggregates_all_buffers() {
        let cs = ChannelSet::new();
        // Fill input ring buffer beyond capacity
        for i in 0..(INPUT_EVENT_CAPACITY + 5) as u64 {
            cs.input_tx
                .push(InputEvent {
                    timestamp_ns: i,
                    kind: InputEventKind::KeyPress { key: 0 },
                })
                .await;
        }
        assert_eq!(cs.overflow.input_event.load(Ordering::Relaxed), 5);
        assert!(cs.overflow.total_drops() >= 5);
    }
}
