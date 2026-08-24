use super::*;

impl EventBridge {
    pub(super) fn new() -> Self {
        Self {
            inner: Mutex::new(EventBridgeInner {
                generation: 0,
                next_sequence: 1,
                backlog: VecDeque::new(),
                active_turns: HashMap::new(),
                inflight_turn_starts: HashMap::new(),
                uncertain_turn_threads: HashSet::new(),
                turn_tombstones: HashSet::new(),
                turn_tombstone_order: VecDeque::new(),
                thread_activity_revisions: HashMap::new(),
                next_activity_revision: 0,
                next_turn_start_token: 0,
                lifecycle_generation_dirty: true,
                lifecycle_clean_threads: HashSet::new(),
                lifecycle_dirty_revisions: HashMap::new(),
                lifecycle_boundary_revision: 0,
                next_lifecycle_revision: 0,
                topology_changes: VecDeque::new(),
                topology_boundary_revision: 0,
                next_topology_revision: 0,
                emitter: Arc::new(|_| {}),
            }),
        }
    }

    pub(super) fn reset(&self, generation: u64, emitter: CodeEventEmitter) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        inner.generation = generation;
        inner.next_sequence = 1;
        inner.backlog.clear();
        clear_event_activity(&mut inner);
        reset_lifecycle_dirty_boundary(&mut inner);
        inner.emitter = emitter;
        Ok(())
    }

    pub(super) fn clear_activity(&self, generation: u64) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        if inner.generation == generation {
            clear_event_activity(&mut inner);
            advance_lifecycle_dirty_boundary(&mut inner);
        }
        Ok(())
    }

    pub(super) fn replace_emitter(&self, emitter: CodeEventEmitter) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|error| error.to_string())?
            .emitter = emitter;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn publish(&self, generation: u64, draft: CodeWorkspaceEventDraft) {
        let publication = self
            .inner
            .lock()
            .ok()
            .and_then(|mut inner| publish_locked(&mut inner, generation, draft));
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
    }

    pub(super) fn insert_approval_and_publish(
        &self,
        approvals: &PendingApprovalStore,
        generation: u64,
        request_id: Value,
        method: &str,
        params: Option<Value>,
    ) -> Result<bool, String> {
        let publication = {
            let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
            ensure_event_generation(&inner, generation)?;
            let Some(draft) = approvals.insert_request(generation, request_id, method, params)?
            else {
                return Ok(false);
            };
            publish_locked(&mut inner, generation, draft)
        };
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
        Ok(true)
    }

    pub(super) fn publish_notification(
        &self,
        approvals: &PendingApprovalStore,
        generation: u64,
        method: &str,
        raw_params: Option<&Value>,
        draft: CodeWorkspaceEventDraft,
    ) -> Result<(), String> {
        let publication = {
            let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
            ensure_event_generation(&inner, generation)?;
            if method == "serverRequest/resolved" {
                if let Some(params) = raw_params {
                    approvals.resolve_notification(generation, params);
                }
            }
            if method == "turn/completed" {
                if let (Some(thread_id), Some(turn_id)) =
                    (draft.thread_id.as_deref(), draft.turn_id.as_deref())
                {
                    approvals.clear_turn(generation, thread_id, turn_id);
                }
            }
            publish_locked(&mut inner, generation, draft)
        };
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
        Ok(())
    }

    pub(super) fn snapshot(
        &self,
        approvals: &PendingApprovalStore,
        requested_generation: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Result<CodeRuntimeEventBacklog, String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        let generation_changed =
            requested_generation.is_some_and(|value| value != inner.generation);
        let full_replay_requested =
            generation_changed || after_sequence.is_none() || after_sequence == Some(0);
        let after_sequence = if generation_changed {
            0
        } else {
            after_sequence.unwrap_or_default()
        };
        let oldest_sequence = inner.backlog.front().map(|event| event.sequence);
        let truncated = generation_changed
            || oldest_sequence.is_some_and(|oldest| after_sequence.saturating_add(1) < oldest);
        let events = inner
            .backlog
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect();
        let latest_sequence = inner.next_sequence.saturating_sub(1);
        let checkpoint = if full_replay_requested || truncated {
            let mut active_turns = inner.active_turns.values().cloned().collect::<Vec<_>>();
            active_turns.sort_by(|left, right| {
                left.started_sequence
                    .cmp(&right.started_sequence)
                    .then_with(|| left.thread_id.cmp(&right.thread_id))
                    .then_with(|| left.turn_id.cmp(&right.turn_id))
            });
            let pending_approvals = approvals
                .checkpoint_events(inner.generation)?
                .into_iter()
                .map(|(draft, respondable)| CodeRuntimeApprovalCheckpoint {
                    event: CodeRuntimeEvent {
                        runtime_generation: inner.generation,
                        sequence: latest_sequence,
                        thread_id: draft.thread_id,
                        turn_id: draft.turn_id,
                        item_id: draft.item_id,
                        kind: draft.kind,
                        payload: draft.payload,
                    },
                    respondable,
                })
                .collect();
            Some(CodeRuntimeEventCheckpoint {
                runtime_generation: inner.generation,
                sequence_watermark: latest_sequence,
                active_turns,
                pending_approvals,
            })
        } else {
            None
        };
        Ok(CodeRuntimeEventBacklog {
            runtime_generation: inner.generation,
            latest_sequence,
            truncated,
            checkpoint,
            events,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.backlog.len())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn begin_turn_start(&self, generation: u64, thread_id: &str) -> Result<u64, String> {
        protocol::validate_id("turn-start thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        begin_turn_start_locked(&mut inner, thread_id)
    }

    pub(super) fn fail_turn_start(
        &self,
        generation: u64,
        thread_id: &str,
        token: u64,
        uncertain_delivery: bool,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        fail_turn_start_locked(&mut inner, thread_id, token, uncertain_delivery)
    }

    pub(super) fn complete_turn_start(
        &self,
        generation: u64,
        thread_id: &str,
        token: u64,
        turn_id: &str,
        status: CodePinnedTurnStatus,
    ) -> Result<(), String> {
        protocol::validate_id("started turn", turn_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        let inflight = remove_matching_turn_start(&mut inner, thread_id, token)?;
        inner.uncertain_turn_threads.remove(thread_id);
        let key = (thread_id.to_string(), turn_id.to_string());
        if inflight.thread_closed {
            inner.active_turns.remove(&key);
            insert_turn_tombstone(&mut inner, key);
            bump_thread_activity(&mut inner, thread_id);
            return Err("Codex thread closed before the turn/start response completed".to_string());
        }
        if inflight.terminal_overflow {
            inner.active_turns.remove(&key);
            inner.uncertain_turn_threads.insert(thread_id.to_string());
            bump_thread_activity(&mut inner, thread_id);
            return Err("Codex turn/start terminal ordering exceeded its safety limit".to_string());
        }
        if status.is_terminal() || inflight.terminal_turn_ids.contains(turn_id) {
            inner.active_turns.remove(&key);
            insert_turn_tombstone(&mut inner, key);
        } else if !inner.turn_tombstones.contains(&key) {
            let started_sequence = inner.next_sequence.saturating_sub(1);
            inner
                .active_turns
                .entry(key)
                .or_insert_with(|| CodeRuntimeActiveTurnCheckpoint {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    status: status.as_str().to_string(),
                    started_sequence,
                });
        }
        bump_thread_activity(&mut inner, thread_id);
        Ok(())
    }

    pub(super) fn mark_thread_uncertain(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<(), String> {
        protocol::validate_id("uncertain turn thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        inner.uncertain_turn_threads.insert(thread_id.to_string());
        bump_thread_activity(&mut inner, thread_id);
        Ok(())
    }

    pub(super) fn reconcile_thread_summary(
        &self,
        generation: u64,
        thread: &CodeThreadSummary,
    ) -> Result<(), String> {
        protocol::validate_id("resumed thread", &thread.id)?;
        let status_value = thread
            .status
            .clone()
            .ok_or_else(|| "Codex resumed thread omitted its status".to_string())?;
        let status: CodePinnedThreadStatus = serde_json::from_value(status_value)
            .map_err(|error| format!("invalid Codex resumed thread status: {error}"))?;
        let mut turns = Vec::with_capacity(thread.turns.len());
        for turn in &thread.turns {
            protocol::validate_id("resumed turn", &turn.id)?;
            turns.push((turn.id.clone(), CodePinnedTurnStatus::parse(&turn.status)?));
        }

        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        if inner.inflight_turn_starts.contains_key(&thread.id) {
            inner.uncertain_turn_threads.insert(thread.id.clone());
            bump_thread_activity(&mut inner, &thread.id);
            return Err("Codex thread was resumed during an in-flight turn/start".to_string());
        }
        inner
            .active_turns
            .retain(|(candidate, _), _| candidate != &thread.id);
        inner.uncertain_turn_threads.remove(&thread.id);
        let mut in_progress = 0_usize;
        for (turn_id, turn_status) in turns {
            let key = (thread.id.clone(), turn_id.clone());
            if turn_status == CodePinnedTurnStatus::InProgress {
                in_progress = in_progress.saturating_add(1);
                if !inner.turn_tombstones.contains(&key) {
                    let started_sequence = inner.next_sequence.saturating_sub(1);
                    inner.active_turns.insert(
                        key,
                        CodeRuntimeActiveTurnCheckpoint {
                            thread_id: thread.id.clone(),
                            turn_id,
                            status: turn_status.as_str().to_string(),
                            started_sequence,
                        },
                    );
                }
            } else {
                insert_turn_tombstone(&mut inner, key);
            }
        }
        let contradictory_idle = status.proves_quiescent() && in_progress > 0;
        if status.is_active() && in_progress == 0
            || matches!(status, CodePinnedThreadStatus::SystemError)
            || contradictory_idle
        {
            inner.uncertain_turn_threads.insert(thread.id.clone());
        }
        bump_thread_activity(&mut inner, &thread.id);
        if contradictory_idle {
            return Err("Codex resumed thread reported idle with an in-progress turn".to_string());
        }
        Ok(())
    }

    pub(super) fn activity_snapshot(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<ThreadActivitySnapshot, String> {
        protocol::validate_id("activity thread", thread_id)?;
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        Ok(ThreadActivitySnapshot {
            revision: inner
                .thread_activity_revisions
                .get(thread_id)
                .copied()
                .unwrap_or_default(),
            active_or_starting: inner
                .active_turns
                .keys()
                .any(|(candidate, _)| candidate == thread_id)
                || inner.inflight_turn_starts.contains_key(thread_id),
            uncertain: inner.uncertain_turn_threads.contains(thread_id),
        })
    }

    pub(super) fn confirm_authoritative_idle(
        &self,
        generation: u64,
        thread_id: &str,
        expected_revision: u64,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        let revision = inner
            .thread_activity_revisions
            .get(thread_id)
            .copied()
            .unwrap_or_default();
        let blocking = inner
            .active_turns
            .keys()
            .any(|(candidate, _)| candidate == thread_id)
            || inner.inflight_turn_starts.contains_key(thread_id);
        if revision != expected_revision || blocking {
            return Err("Codex thread activity changed during the idle proof".to_string());
        }
        if inner.uncertain_turn_threads.remove(thread_id) {
            bump_thread_activity(&mut inner, thread_id);
        }
        Ok(())
    }

    pub(super) fn lifecycle_dirty_checkpoint(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<CodeThreadLifecycleDirtyCheckpoint, String> {
        protocol::validate_id("lifecycle-dirty thread", thread_id)?;
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        Ok(lifecycle_checkpoint_locked(&inner, generation, thread_id))
    }

    pub(super) fn topology_checkpoint(&self, generation: u64) -> Result<(u64, u64), String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        Ok((
            inner.topology_boundary_revision,
            inner.next_topology_revision,
        ))
    }

    pub(super) fn confirm_topology_checkpoint(
        &self,
        generation: u64,
        boundary_revision: u64,
        revision: u64,
    ) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        if inner.topology_boundary_revision != boundary_revision
            || inner.next_topology_revision != revision
        {
            return Err(
                "Codex thread topology changed during the authoritative graph scan".to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn clear_lifecycle_dirty(
        &self,
        generation: u64,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<(), String> {
        protocol::validate_id("lifecycle-dirty thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        if checkpoint.generation != generation
            || checkpoint.thread_id != thread_id
            || checkpoint.boundary_revision != inner.lifecycle_boundary_revision
            || checkpoint.graph_revision != inner.next_lifecycle_revision
            || checkpoint.thread_dirty != inner.lifecycle_dirty_revisions.get(thread_id).copied()
        {
            return Err("Codex thread lifecycle changed during durable reconciliation".to_string());
        }
        mark_lifecycle_clean_locked(&mut inner, thread_id)
    }

    #[cfg(test)]
    pub(super) fn mark_new_thread_lifecycle_clean(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<(), String> {
        protocol::validate_id("new lifecycle thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        validate_new_thread_lifecycle_locked(&inner, thread_id)?;
        ensure_lifecycle_clean_capacity_locked(&inner, thread_id)?;
        inner.lifecycle_clean_threads.insert(thread_id.to_string());
        Ok(())
    }

    pub(super) fn mutation_response_checkpoint(
        &self,
        receipt: LifecycleWriteReceipt,
    ) -> Result<CodeThreadLifecycleMutationCompletion, String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, receipt.generation)?;
        let expected_signals = validate_expected_topology_changes_locked(
            &inner,
            receipt.topology_boundary_revision,
            receipt.topology_revision,
            &receipt.thread_id,
            receipt.expected,
        )?;
        if expected_signals > 1 {
            return Err("Codex emitted duplicate lifecycle completion signals".to_string());
        }
        if inner.lifecycle_boundary_revision != receipt.lifecycle_boundary_revision {
            return Err(
                "Codex lifecycle boundary changed after the guarded mutation write".to_string(),
            );
        }
        let thread_dirty = inner
            .lifecycle_dirty_revisions
            .get(&receipt.thread_id)
            .copied();
        Ok(CodeThreadLifecycleMutationCompletion {
            generation: receipt.generation,
            thread_id: receipt.thread_id,
            expected: receipt.expected,
            lifecycle_boundary_revision: inner.lifecycle_boundary_revision,
            topology_boundary_revision: inner.topology_boundary_revision,
            topology_revision: inner.next_topology_revision,
            thread_dirty,
            expected_signal_seen: expected_signals == 1,
        })
    }

    pub(super) fn complete_lifecycle_mutation<T>(
        &self,
        thread_id: &str,
        completion: CodeThreadLifecycleMutationCompletion,
        expected: CodeThreadLifecycleSignal,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("lifecycle completion thread", thread_id)?;
        if completion.thread_id != thread_id || completion.expected != expected {
            return Err(
                "Codex lifecycle completion proof does not match the requested mutation"
                    .to_string(),
            );
        }
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, completion.generation)?;
        if inner.lifecycle_boundary_revision != completion.lifecycle_boundary_revision {
            return Err(
                "Codex lifecycle boundary changed during durable mutation commit".to_string(),
            );
        }
        let expected_signals = validate_expected_topology_changes_locked(
            &inner,
            completion.topology_boundary_revision,
            completion.topology_revision,
            thread_id,
            expected,
        )?;
        if usize::from(completion.expected_signal_seen).saturating_add(expected_signals) > 1 {
            return Err("Codex emitted duplicate lifecycle completion signals".to_string());
        }
        let current_dirty = inner.lifecycle_dirty_revisions.get(thread_id).copied();
        if current_dirty != completion.thread_dirty
            && !current_dirty.is_some_and(|dirty| dirty.signal == expected)
        {
            return Err(
                "Codex lifecycle signal conflicted with the durable mutation commit".to_string(),
            );
        }
        ensure_lifecycle_clean_capacity_locked(&inner, thread_id)?;
        let committed = commit()?;
        inner.lifecycle_dirty_revisions.remove(thread_id);
        inner.lifecycle_clean_threads.insert(thread_id.to_string());
        Ok(committed)
    }
}
