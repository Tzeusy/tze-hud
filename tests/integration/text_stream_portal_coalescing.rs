//! Text stream portal coalescing validation (hud-t98e.4).
//!
//! Focus: under rapid append pressure, coalescing may skip intermediate frames
//! but must keep a coherent retained transcript window.

use tze_hud_runtime::{EnqueueResult, FreezeQueue, MutationTrafficClass, QueuedMutation};

fn retained_snapshot(history: &[String], retain_lines: usize) -> String {
    let start = history.len().saturating_sub(retain_lines);
    history[start..].join("\n")
}

fn queue_portal_snapshot(
    queue: &mut FreezeQueue,
    sequence: usize,
    portal_key: &str,
    snapshot: String,
) -> EnqueueResult {
    queue.enqueue(QueuedMutation {
        batch_id: format!("portal-{sequence}").into_bytes(),
        original_batch_id: format!("portal-{sequence}").into_bytes(),
        traffic_class: MutationTrafficClass::StateStream,
        coalesce_key: Some(portal_key.to_string()),
        submitted_at_wall_us: sequence as u64,
        payload: snapshot.into_bytes(),
    })
}

#[test]
fn rapid_portal_appends_preserve_full_window_under_coalescing() {
    const APPEND_COUNT: usize = 100;

    let mut queue = FreezeQueue::new(8);
    let mut history = Vec::new();

    for seq in 0..APPEND_COUNT {
        history.push(format!("[{seq:03}] line {seq}"));
        let snapshot = retained_snapshot(&history, APPEND_COUNT);
        let outcome = queue_portal_snapshot(&mut queue, seq, "portal://pilot/rapid", snapshot);

        if seq == 0 {
            assert!(
                matches!(
                    outcome,
                    EnqueueResult::Queued | EnqueueResult::QueuedWithPressure
                ),
                "first append should queue normally"
            );
        } else {
            assert!(
                matches!(outcome, EnqueueResult::Coalesced),
                "subsequent rapid appends should coalesce into latest coherent snapshot"
            );
        }
    }

    let drained = queue.drain();
    assert_eq!(
        drained.len(),
        1,
        "coalescing should collapse queued snapshots"
    );

    let final_snapshot = String::from_utf8(drained[0].payload.clone()).expect("utf8 payload");
    let lines: Vec<&str> = final_snapshot.lines().collect();
    assert_eq!(
        lines.len(),
        APPEND_COUNT,
        "retained window must keep all appends"
    );
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            *line,
            format!("[{i:03}] line {i}"),
            "line order must remain coherent after coalescing"
        );
    }
}

#[test]
fn coalescing_with_bounded_tail_keeps_latest_window_in_order() {
    const APPEND_COUNT: usize = 100;
    const RETAIN_LINES: usize = 32;

    let mut queue = FreezeQueue::new(4);
    let mut history = Vec::new();

    for seq in 0..APPEND_COUNT {
        history.push(format!("[{seq:03}] item {seq}"));
        let snapshot = retained_snapshot(&history, RETAIN_LINES);
        let _ = queue_portal_snapshot(&mut queue, seq, "portal://pilot/tail", snapshot);
    }

    let drained = queue.drain();
    assert_eq!(drained.len(), 1);
    let final_snapshot = String::from_utf8(drained[0].payload.clone()).expect("utf8 payload");
    let lines: Vec<&str> = final_snapshot.lines().collect();
    assert_eq!(
        lines.len(),
        RETAIN_LINES,
        "final snapshot must keep bounded tail"
    );

    let first_expected = APPEND_COUNT - RETAIN_LINES;
    for (idx, line) in lines.iter().enumerate() {
        let seq = first_expected + idx;
        assert_eq!(*line, format!("[{seq:03}] item {seq}"));
    }
}

#[test]
fn intermediate_renders_can_skip_frames_but_final_state_is_complete() {
    const APPEND_COUNT: usize = 100;
    const RETAIN_LINES: usize = APPEND_COUNT;

    let mut queue = FreezeQueue::new(6);
    let mut history = Vec::new();
    let mut rendered_counts = Vec::new();

    for seq in 0..APPEND_COUNT {
        history.push(format!("[{seq:03}] token {seq}"));
        let snapshot = retained_snapshot(&history, RETAIN_LINES);
        let _ = queue_portal_snapshot(&mut queue, seq, "portal://pilot/frame", snapshot);

        // Simulate compositor cadence slower than append cadence.
        if seq % 17 == 16 {
            let drained = queue.drain();
            if let Some(last) = drained.last() {
                let snapshot = String::from_utf8(last.payload.clone()).expect("utf8 payload");
                rendered_counts.push(snapshot.lines().count());
            }
        }
    }

    let drained = queue.drain();
    if let Some(last) = drained.last() {
        let snapshot = String::from_utf8(last.payload.clone()).expect("utf8 payload");
        rendered_counts.push(snapshot.lines().count());
    }

    assert!(
        rendered_counts.iter().any(|count| *count < APPEND_COUNT),
        "intermediate render states should be allowed to show fewer entries"
    );
    assert_eq!(
        *rendered_counts.last().expect("at least one render"),
        APPEND_COUNT,
        "final committed snapshot must include the full retained window"
    );
    assert!(
        rendered_counts.windows(2).all(|pair| pair[1] >= pair[0]),
        "rendered snapshots must move forward monotonically"
    );
}
