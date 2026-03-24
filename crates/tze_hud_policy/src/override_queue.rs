//! # OverrideCommandQueue — Level 0 Human Override SPSC Queue
//!
//! Implements the bounded SPSC override command queue specified in
//! policy-arbitration/spec.md (lines 48-59).
//!
//! ## Key Properties
//!
//! - **SPSC**: single producer (input thread), single consumer (compositor thread).
//! - **Bounded**: capacity 16 (spec §1.1, §11.1).
//! - **Local, instant**: override commands are processed before any `MutationBatch` intake.
//! - **Cannot be vetoed**: no policy level may intercept, delay, or block override commands.
//!
//! ## Purity constraint
//!
//! `OverrideCommandQueue` is a queue (not pure). It is **not** part of `PolicyContext`.
//! The compositor thread drains it into `OverrideState` before constructing `PolicyContext`.
//! The policy evaluation functions (`frame.rs`, `event.rs`, `stack.rs`) are pure and only
//! read the resulting `OverrideState`.

use std::collections::VecDeque;

/// Maximum number of override commands that can be queued at once (spec §11.1).
pub const OVERRIDE_QUEUE_CAPACITY: usize = 16;

/// A human override command issued from the physical input layer.
///
/// All commands arrive on the input thread and are drained by the compositor thread
/// before `MutationBatch` intake (spec §3.2, §11.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverrideCommand {
    /// Dismiss the tile identified by the given tile ID string.
    ///
    /// Level 0 wins over Level 3 Security: the tile is dismissed regardless of lease
    /// priority or capabilities (spec lines 57-59).
    Dismiss {
        /// Opaque tile identifier. The compositor resolves this to a scene node.
        tile_id: String,
    },

    /// Enter safe mode immediately.
    ///
    /// Triggered by `Ctrl+Shift+Escape` (or equivalent hardware button).
    /// Safe mode suspends all agent mutations and renders chrome in software fallback.
    SafeMode,

    /// Freeze the scene.
    ///
    /// Agent mutations are queued (not rejected), resource budgets are paused,
    /// and the degradation ladder does not advance (spec §6.1, §6.2).
    Freeze,

    /// Mute all audio output from agent tiles.
    Mute,
}

/// Single-producer single-consumer override command queue.
///
/// **Capacity**: `OVERRIDE_QUEUE_CAPACITY` (16).
///
/// **Producer**: input thread — calls `push`.
/// **Consumer**: compositor thread — calls `drain` before any `MutationBatch` intake.
///
/// If the queue is full when `push` is called, the command is silently dropped.
/// In practice this should not occur under normal operation; the compositor drains
/// the queue every frame (< 16.6ms), and 16 commands per frame is far above the
/// expected human input rate.
///
/// # Thread Safety Note
///
/// This implementation uses a `VecDeque` and is **not** thread-safe by itself.
/// In the compositor thread model, the producer and consumer are on different threads.
/// The caller is responsible for providing appropriate synchronization (e.g., a `Mutex`
/// or a lock-free SPSC ring buffer from an external crate). This struct models the
/// *logical* SPSC contract; production code should wrap it or replace it with a
/// lock-free variant (e.g., `crossbeam::queue::ArrayQueue`).
#[derive(Debug)]
pub struct OverrideCommandQueue {
    inner: VecDeque<OverrideCommand>,
}

impl OverrideCommandQueue {
    /// Create a new empty override command queue.
    pub fn new() -> Self {
        Self {
            inner: VecDeque::with_capacity(OVERRIDE_QUEUE_CAPACITY),
        }
    }

    /// Push an override command from the input thread.
    ///
    /// Returns `true` if the command was enqueued, `false` if the queue was full
    /// and the command was dropped.
    pub fn push(&mut self, cmd: OverrideCommand) -> bool {
        if self.inner.len() >= OVERRIDE_QUEUE_CAPACITY {
            return false;
        }
        self.inner.push_back(cmd);
        true
    }

    /// Pop the next override command (FIFO order).
    ///
    /// Returns `None` if the queue is empty.
    pub fn pop(&mut self) -> Option<OverrideCommand> {
        self.inner.pop_front()
    }

    /// Drain all pending commands into a `Vec`.
    ///
    /// Called by the compositor thread at the start of each frame, **before** any
    /// `MutationBatch` intake (spec §11.1). The commands are processed in FIFO order
    /// (input-event order, per spec §2.2 within-level conflict resolution for Level 0).
    pub fn drain(&mut self) -> Vec<OverrideCommand> {
        self.inner.drain(..).collect()
    }

    /// Current queue depth.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns true if the queue is at capacity (next push will be dropped).
    pub fn is_full(&self) -> bool {
        self.inner.len() >= OVERRIDE_QUEUE_CAPACITY
    }
}

impl Default for OverrideCommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_capacity_is_16() {
        assert_eq!(OVERRIDE_QUEUE_CAPACITY, 16);
    }

    #[test]
    fn test_push_and_pop_fifo_order() {
        let mut q = OverrideCommandQueue::new();
        q.push(OverrideCommand::Freeze);
        q.push(OverrideCommand::Mute);
        q.push(OverrideCommand::SafeMode);

        assert_eq!(q.pop(), Some(OverrideCommand::Freeze));
        assert_eq!(q.pop(), Some(OverrideCommand::Mute));
        assert_eq!(q.pop(), Some(OverrideCommand::SafeMode));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn test_drain_returns_all_in_fifo_order() {
        let mut q = OverrideCommandQueue::new();
        q.push(OverrideCommand::SafeMode);
        q.push(OverrideCommand::Freeze);
        q.push(OverrideCommand::Dismiss { tile_id: "tile_1".to_string() });

        let drained = q.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0], OverrideCommand::SafeMode);
        assert_eq!(drained[1], OverrideCommand::Freeze);
        assert_eq!(drained[2], OverrideCommand::Dismiss { tile_id: "tile_1".to_string() });
        assert!(q.is_empty());
    }

    #[test]
    fn test_push_drops_when_full() {
        let mut q = OverrideCommandQueue::new();
        // Fill to capacity
        for _ in 0..OVERRIDE_QUEUE_CAPACITY {
            assert!(q.push(OverrideCommand::Mute));
        }
        assert!(q.is_full());

        // Next push is dropped
        let accepted = q.push(OverrideCommand::SafeMode);
        assert!(!accepted);
        assert_eq!(q.len(), OVERRIDE_QUEUE_CAPACITY);
    }

    #[test]
    fn test_empty_drain() {
        let mut q = OverrideCommandQueue::new();
        let drained = q.drain();
        assert!(drained.is_empty());
    }

    #[test]
    fn test_safe_mode_command_enqueues() {
        let mut q = OverrideCommandQueue::new();
        q.push(OverrideCommand::SafeMode);
        assert_eq!(q.len(), 1);
        assert!(!q.is_full());
    }

    /// WHEN the viewer presses Ctrl+Shift+Escape while mutation intake is in progress
    /// THEN override command is processed before any pending mutations in the current batch.
    ///
    /// This test verifies that SafeMode is at the head of a drained queue when pushed first.
    #[test]
    fn test_safe_mode_is_first_in_drain_when_pushed_first() {
        let mut q = OverrideCommandQueue::new();
        q.push(OverrideCommand::SafeMode);
        q.push(OverrideCommand::Dismiss { tile_id: "t1".to_string() });

        let cmds = q.drain();
        assert_eq!(cmds[0], OverrideCommand::SafeMode);
    }
}
