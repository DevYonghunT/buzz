use super::*;

pub(super) fn update_active_turn_checkpoint(
    inner: &mut EventBridgeInner,
    event: &CodeRuntimeEvent,
) {
    match event.kind.as_str() {
        "turn/started" => {
            let (Some(thread_id), Some(turn_id)) =
                (event.thread_id.as_ref(), event.turn_id.as_ref())
            else {
                return;
            };
            let status = event
                .payload
                .get("turn")
                .and_then(|turn| turn.get("status"))
                .and_then(Value::as_str)
                .filter(|status| !status.is_empty())
                .unwrap_or("inProgress");
            let key = (thread_id.clone(), turn_id.clone());
            match CodePinnedTurnStatus::parse(status) {
                Ok(CodePinnedTurnStatus::InProgress) if !inner.turn_tombstones.contains(&key) => {
                    inner.uncertain_turn_threads.remove(thread_id);
                    inner.active_turns.insert(
                        key,
                        CodeRuntimeActiveTurnCheckpoint {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            status: status.to_string(),
                            started_sequence: event.sequence,
                        },
                    );
                }
                Ok(terminal) => {
                    inner.active_turns.remove(&key);
                    if terminal.is_terminal() {
                        insert_turn_tombstone(inner, key);
                    }
                }
                Err(_) => {
                    inner.uncertain_turn_threads.insert(thread_id.clone());
                }
            }
            bump_thread_activity(inner, thread_id);
        }
        "turn/completed" => {
            if let (Some(thread_id), Some(turn_id)) =
                (event.thread_id.as_ref(), event.turn_id.as_ref())
            {
                if let Some(inflight) = inner.inflight_turn_starts.get_mut(thread_id) {
                    if !inflight.terminal_overflow
                        && !inflight.terminal_turn_ids.contains(turn_id)
                        && inflight.terminal_turn_ids.len() >= MAX_INFLIGHT_TERMINAL_TURNS
                    {
                        inflight.terminal_turn_ids.clear();
                        inflight.terminal_overflow = true;
                    }
                    if !inflight.terminal_overflow {
                        inflight.terminal_turn_ids.insert(turn_id.clone());
                    }
                }
                let key = (thread_id.clone(), turn_id.clone());
                inner.active_turns.remove(&key);
                inner.uncertain_turn_threads.remove(thread_id);
                insert_turn_tombstone(inner, key);
                bump_thread_activity(inner, thread_id);
            }
        }
        "thread/closed" => {
            if let Some(thread_id) = event.thread_id.as_ref() {
                record_topology_change(inner, thread_id, TopologyChangeKind::Closed);
                if let Some(inflight) = inner.inflight_turn_starts.get_mut(thread_id) {
                    inflight.thread_closed = true;
                }
                let completed = inner
                    .active_turns
                    .keys()
                    .filter(|(candidate, _)| candidate == thread_id)
                    .cloned()
                    .collect::<Vec<_>>();
                inner
                    .active_turns
                    .retain(|(candidate, _), _| candidate != thread_id);
                inner.uncertain_turn_threads.remove(thread_id);
                for key in completed {
                    insert_turn_tombstone(inner, key);
                }
                bump_thread_activity(inner, thread_id);
            }
        }
        "thread/archived" | "thread/unarchived" => {
            if let Some(thread_id) = event.thread_id.as_ref() {
                let signal = if event.kind == "thread/archived" {
                    CodeThreadLifecycleSignal::Archived
                } else {
                    CodeThreadLifecycleSignal::Unarchived
                };
                mark_lifecycle_dirty(inner, thread_id, signal);
            }
        }
        "thread/started" => {
            if let Some(thread_id) = event.thread_id.as_ref() {
                record_topology_change(inner, thread_id, TopologyChangeKind::Started);
            }
        }
        _ => {}
    }
}

pub(super) fn ensure_event_generation(
    inner: &EventBridgeInner,
    generation: u64,
) -> Result<(), String> {
    if inner.generation == generation {
        Ok(())
    } else {
        Err("Codex thread activity belongs to a stale runtime generation".to_string())
    }
}

pub(super) fn lifecycle_checkpoint_locked(
    inner: &EventBridgeInner,
    generation: u64,
    thread_id: &str,
) -> CodeThreadLifecycleDirtyCheckpoint {
    let thread_dirty = inner.lifecycle_dirty_revisions.get(thread_id).copied();
    let dirty = thread_dirty.is_some()
        || inner.lifecycle_generation_dirty && !inner.lifecycle_clean_threads.contains(thread_id);
    CodeThreadLifecycleDirtyCheckpoint {
        generation,
        thread_id: thread_id.to_string(),
        boundary_revision: inner.lifecycle_boundary_revision,
        graph_revision: inner.next_lifecycle_revision,
        thread_dirty,
        dirty,
    }
}

pub(super) fn validate_exact_lifecycle_checkpoint_locked(
    inner: &EventBridgeInner,
    generation: u64,
    thread_id: &str,
    checkpoint: &CodeThreadLifecycleDirtyCheckpoint,
) -> Result<(), String> {
    if checkpoint.generation != generation
        || checkpoint.thread_id != thread_id
        || checkpoint.boundary_revision != inner.lifecycle_boundary_revision
        || checkpoint.thread_dirty != inner.lifecycle_dirty_revisions.get(thread_id).copied()
        || checkpoint.dirty != lifecycle_checkpoint_locked(inner, generation, thread_id).dirty
    {
        return Err("Codex exact thread lifecycle changed before native admission".to_string());
    }
    Ok(())
}

pub(super) fn validate_expected_topology_changes_locked(
    inner: &EventBridgeInner,
    checkpoint_boundary_revision: u64,
    checkpoint_revision: u64,
    thread_id: &str,
    expected: CodeThreadLifecycleSignal,
) -> Result<usize, String> {
    if checkpoint_boundary_revision != inner.topology_boundary_revision
        || checkpoint_revision < inner.topology_boundary_revision
    {
        return Err("Codex topology history crossed its bounded safety boundary".to_string());
    }
    let mut expected_signals = 0_usize;
    for change in inner
        .topology_changes
        .iter()
        .filter(|change| change.revision > checkpoint_revision)
    {
        if change.thread_id != thread_id || change.kind != TopologyChangeKind::Lifecycle(expected) {
            return Err(
                "Codex topology changed incompatibly with the lifecycle mutation".to_string(),
            );
        }
        expected_signals = expected_signals.saturating_add(1);
    }
    Ok(expected_signals)
}

pub(super) fn mark_lifecycle_clean_locked(
    inner: &mut EventBridgeInner,
    thread_id: &str,
) -> Result<(), String> {
    ensure_lifecycle_clean_capacity_locked(inner, thread_id)?;
    inner.lifecycle_dirty_revisions.remove(thread_id);
    inner.lifecycle_clean_threads.insert(thread_id.to_string());
    Ok(())
}

pub(super) fn ensure_lifecycle_clean_capacity_locked(
    inner: &EventBridgeInner,
    thread_id: &str,
) -> Result<(), String> {
    if !inner.lifecycle_clean_threads.contains(thread_id)
        && inner.lifecycle_clean_threads.len() >= MAX_LIFECYCLE_DIRTY_THREADS
    {
        return Err(format!(
            "Codex lifecycle clean-thread limit of {MAX_LIFECYCLE_DIRTY_THREADS} was reached"
        ));
    }
    Ok(())
}

pub(super) fn validate_new_thread_lifecycle_locked(
    inner: &EventBridgeInner,
    thread_id: &str,
) -> Result<(), String> {
    if !inner.lifecycle_generation_dirty
        || inner.lifecycle_boundary_revision != 0
        || inner.lifecycle_dirty_revisions.contains_key(thread_id)
        || inner.lifecycle_clean_threads.contains(thread_id)
    {
        return Err(
            "Codex new thread crossed an unverified lifecycle boundary or notification".to_string(),
        );
    }
    Ok(())
}

pub(super) fn clear_event_activity(inner: &mut EventBridgeInner) {
    inner.active_turns.clear();
    inner.inflight_turn_starts.clear();
    inner.uncertain_turn_threads.clear();
    inner.turn_tombstones.clear();
    inner.turn_tombstone_order.clear();
    inner.thread_activity_revisions.clear();
    inner.next_activity_revision = 0;
    inner.next_turn_start_token = 0;
}

pub(super) fn reset_lifecycle_dirty_boundary(inner: &mut EventBridgeInner) {
    inner.lifecycle_generation_dirty = true;
    inner.lifecycle_clean_threads.clear();
    inner.lifecycle_dirty_revisions.clear();
    inner.lifecycle_boundary_revision = 0;
    inner.next_lifecycle_revision = 0;
    inner.topology_changes.clear();
    inner.topology_boundary_revision = 0;
    inner.next_topology_revision = 0;
}

pub(super) fn advance_lifecycle_dirty_boundary(inner: &mut EventBridgeInner) {
    inner.next_lifecycle_revision = inner.next_lifecycle_revision.saturating_add(1);
    inner.lifecycle_boundary_revision = inner.next_lifecycle_revision;
    inner.lifecycle_generation_dirty = true;
    inner.lifecycle_clean_threads.clear();
    inner.lifecycle_dirty_revisions.clear();
    advance_topology_boundary(inner);
}

pub(super) fn advance_topology_boundary(inner: &mut EventBridgeInner) {
    inner.next_topology_revision = inner.next_topology_revision.saturating_add(1);
    inner.topology_boundary_revision = inner.next_topology_revision;
    inner.topology_changes.clear();
}

pub(super) fn mark_lifecycle_dirty(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    signal: CodeThreadLifecycleSignal,
) {
    if !inner.lifecycle_dirty_revisions.contains_key(thread_id)
        && inner.lifecycle_dirty_revisions.len() >= MAX_LIFECYCLE_DIRTY_THREADS
    {
        advance_lifecycle_dirty_boundary(inner);
    }
    inner.next_lifecycle_revision = inner.next_lifecycle_revision.saturating_add(1);
    record_topology_change(inner, thread_id, TopologyChangeKind::Lifecycle(signal));
    inner.lifecycle_dirty_revisions.insert(
        thread_id.to_string(),
        LifecycleDirtyRevision {
            revision: inner.next_lifecycle_revision,
            signal,
        },
    );
    inner.lifecycle_clean_threads.remove(thread_id);
}

pub(super) fn record_topology_change(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    kind: TopologyChangeKind,
) {
    inner.next_topology_revision = inner.next_topology_revision.saturating_add(1);
    if inner.topology_changes.len() == MAX_TOPOLOGY_CHANGES {
        if let Some(evicted) = inner.topology_changes.pop_front() {
            inner.topology_boundary_revision = evicted.revision;
        }
    }
    inner.topology_changes.push_back(TopologyChange {
        revision: inner.next_topology_revision,
        thread_id: thread_id.to_string(),
        kind,
    });
}

pub(super) fn publish_locked(
    inner: &mut EventBridgeInner,
    generation: u64,
    draft: CodeWorkspaceEventDraft,
) -> Option<(CodeEventEmitter, CodeRuntimeEvent)> {
    if inner.generation != generation {
        return None;
    }
    let sequence = inner.next_sequence;
    inner.next_sequence = inner.next_sequence.saturating_add(1);
    let event = CodeRuntimeEvent {
        runtime_generation: generation,
        sequence,
        thread_id: draft.thread_id,
        turn_id: draft.turn_id,
        item_id: draft.item_id,
        kind: draft.kind,
        payload: draft.payload,
    };
    update_active_turn_checkpoint(inner, &event);
    if inner.backlog.len() == MAX_NOTIFICATION_BACKLOG {
        inner.backlog.pop_front();
    }
    inner.backlog.push_back(event.clone());
    Some((Arc::clone(&inner.emitter), event))
}

pub(super) fn remove_matching_turn_start(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    token: u64,
) -> Result<InflightTurnStart, String> {
    if inner
        .inflight_turn_starts
        .get(thread_id)
        .map(|inflight| inflight.token)
        != Some(token)
    {
        return Err("Codex turn/start ordering token is no longer current".to_string());
    }
    inner
        .inflight_turn_starts
        .remove(thread_id)
        .ok_or_else(|| "Codex turn/start ordering state disappeared".to_string())
}

pub(super) fn begin_turn_start_locked(
    inner: &mut EventBridgeInner,
    thread_id: &str,
) -> Result<u64, String> {
    if inner.inflight_turn_starts.contains_key(thread_id)
        || inner
            .active_turns
            .keys()
            .any(|(candidate, _)| candidate == thread_id)
        || inner.uncertain_turn_threads.contains(thread_id)
    {
        return Err("Codex thread already has active or uncertain turn state".to_string());
    }
    let token = inner
        .next_turn_start_token
        .checked_add(1)
        .ok_or_else(|| "Codex turn/start ordering token was exhausted".to_string())?;
    inner.next_turn_start_token = token;
    inner.inflight_turn_starts.insert(
        thread_id.to_string(),
        InflightTurnStart {
            token,
            terminal_turn_ids: HashSet::new(),
            terminal_overflow: false,
            thread_closed: false,
        },
    );
    bump_thread_activity(inner, thread_id);
    Ok(token)
}

pub(super) fn fail_turn_start_locked(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    token: u64,
    uncertain_delivery: bool,
) -> Result<(), String> {
    let _inflight = remove_matching_turn_start(inner, thread_id, token)?;
    if uncertain_delivery {
        inner.uncertain_turn_threads.insert(thread_id.to_string());
    }
    bump_thread_activity(inner, thread_id);
    Ok(())
}

pub(super) fn bump_thread_activity(inner: &mut EventBridgeInner, thread_id: &str) {
    inner.next_activity_revision = inner.next_activity_revision.saturating_add(1);
    inner
        .thread_activity_revisions
        .insert(thread_id.to_string(), inner.next_activity_revision);
}

pub(super) fn insert_turn_tombstone(inner: &mut EventBridgeInner, key: (String, String)) {
    if !inner.turn_tombstones.insert(key.clone()) {
        return;
    }
    if inner.turn_tombstone_order.len() == MAX_TURN_TOMBSTONES {
        if let Some(expired) = inner.turn_tombstone_order.pop_front() {
            inner.turn_tombstones.remove(&expired);
        }
    }
    inner.turn_tombstone_order.push_back(key);
}
