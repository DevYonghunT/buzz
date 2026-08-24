use super::*;

#[test]
fn turn_start_response_marks_active_before_delayed_notification() -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let token = bridge.begin_turn_start(7, "thread-1")?;
    bridge.complete_turn_start(
        7,
        "thread-1",
        token,
        "turn-1",
        CodePinnedTurnStatus::InProgress,
    )?;
    let before_notification = bridge.activity_snapshot(7, "thread-1")?;
    assert!(before_notification.active_or_starting);

    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: "turn/started".to_string(),
            payload: json!({ "turn": { "status": "inProgress" } }),
        },
    );
    assert!(bridge.activity_snapshot(7, "thread-1")?.active_or_starting);
    Ok(())
}
#[test]
fn completion_before_turn_start_response_cannot_resurrect_the_turn() -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let token = bridge.begin_turn_start(7, "thread-1")?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: "turn/completed".to_string(),
            payload: json!({ "turn": { "status": "completed" } }),
        },
    );
    bridge.complete_turn_start(
        7,
        "thread-1",
        token,
        "turn-1",
        CodePinnedTurnStatus::InProgress,
    )?;
    assert!(!bridge.activity_snapshot(7, "thread-1")?.active_or_starting);

    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: "turn/started".to_string(),
            payload: json!({ "turn": { "status": "inProgress" } }),
        },
    );
    assert!(!bridge.activity_snapshot(7, "thread-1")?.active_or_starting);
    bridge.reset(8, noop_emitter())?;
    assert!(!bridge.activity_snapshot(8, "thread-1")?.active_or_starting);
    Ok(())
}
#[test]
fn thread_close_before_turn_start_response_cannot_resurrect_the_turn() -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let token = bridge.begin_turn_start(7, "thread-1")?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/closed".to_string(),
            payload: json!({ "threadId": "thread-1" }),
        },
    );
    assert!(bridge
        .complete_turn_start(
            7,
            "thread-1",
            token,
            "turn-1",
            CodePinnedTurnStatus::InProgress,
        )
        .is_err());
    assert!(!bridge.activity_snapshot(7, "thread-1")?.active_or_starting);
    Ok(())
}

#[test]
fn inflight_terminal_proof_survives_global_tombstone_eviction() -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let token = bridge.begin_turn_start(7, "thread-target")?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-target".to_string()),
            turn_id: Some("turn-target".to_string()),
            item_id: None,
            kind: "turn/completed".to_string(),
            payload: json!({ "turn": { "status": "completed" } }),
        },
    );
    for index in 0..MAX_TURN_TOMBSTONES {
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-other".to_string()),
                turn_id: Some(format!("turn-other-{index}")),
                item_id: None,
                kind: "turn/completed".to_string(),
                payload: json!({ "turn": { "status": "completed" } }),
            },
        );
    }
    bridge.complete_turn_start(
        7,
        "thread-target",
        token,
        "turn-target",
        CodePinnedTurnStatus::InProgress,
    )?;
    assert!(
        !bridge
            .activity_snapshot(7, "thread-target")?
            .active_or_starting
    );
    Ok(())
}
