use super::*;

impl CodeRuntime {
    /// Set one validated title and then read back authoritative thread metadata.
    #[cfg(test)]
    pub(crate) fn thread_rename(
        &self,
        input: &CodeThreadRenameInput,
    ) -> Result<CodeThreadSummary, String> {
        let params = input.rpc_params()?;
        let result = self.request_ready("thread/name/set", params)?;
        protocol::parse_thread_name_set(result)?;
        self.thread_read(&input.thread_id)
    }

    /// Rename one stable active or archived thread with exact lifecycle state
    /// held through the `thread/name/set` JSON-RPC byte write.
    pub(crate) fn thread_rename_guarded(
        &self,
        input: &CodeThreadRenameInput,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadSummary, String> {
        let params = input.rpc_params()?;
        let (_, pending) = self
            .begin_active_request(
                &input.thread_id,
                checkpoint,
                "thread/name/set",
                params,
                None,
            )
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        protocol::parse_thread_name_set(result)?;
        self.thread_read(&input.thread_id)
    }

    /// Discover every persisted or currently-loaded Codex thread at one exact
    /// native execution root. This is intentionally not a general frontend
    /// listing API: it exists only to reconcile an ambiguous `thread/start`.
    pub(crate) fn recovery_threads_at(
        &self,
        workspace_root: &str,
    ) -> Result<Vec<CodeRecoveryThread>, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let mut candidates = HashMap::<String, CodeRecoveryThread>::new();

        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        for _ in 0..MAX_RECOVERY_PAGES {
            let params = protocol::recovery_thread_list_params(&workspace_root, cursor.as_deref())?;
            let page =
                protocol::parse_recovery_thread_list(self.request_ready("thread/list", params)?)?;
            for candidate in page.data {
                validate_recovery_candidate_root(&candidate, &workspace_root)?;
                if candidates
                    .insert(candidate.thread.id.clone(), candidate)
                    .is_some()
                {
                    return Err(
                        "Codex recovery thread list contained a duplicate thread id".to_string()
                    );
                }
                if candidates.len() > MAX_RECOVERY_THREADS {
                    return Err(format!(
                        "Codex recovery exceeded the {MAX_RECOVERY_THREADS}-thread safety limit"
                    ));
                }
            }
            match page.next_cursor {
                Some(next_cursor) => {
                    if !seen_cursors.insert(next_cursor.clone()) {
                        return Err("Codex recovery pagination repeated a cursor".to_string());
                    }
                    cursor = Some(next_cursor);
                }
                None => {
                    cursor = None;
                    break;
                }
            }
        }
        if cursor.is_some() {
            return Err(format!(
                "Codex recovery exceeded the {MAX_RECOVERY_PAGES}-page safety limit"
            ));
        }

        // A newly-created empty 0.145 thread can remain deferred in memory and
        // therefore be absent from `thread/list`. Include loaded IDs and read
        // their metadata before deciding whether the start produced a thread.
        let mut loaded_ids = Vec::new();
        let mut loaded_cursor = None;
        let mut seen_loaded_cursors = HashSet::new();
        for _ in 0..MAX_RECOVERY_PAGES {
            let params = protocol::loaded_thread_list_params(loaded_cursor.as_deref())?;
            let page = protocol::parse_loaded_thread_list(
                self.request_ready("thread/loaded/list", params)?,
            )?;
            loaded_ids.extend(page.data);
            if loaded_ids.len() > MAX_RECOVERY_THREADS {
                return Err(format!(
                    "Codex loaded-thread recovery exceeded the {MAX_RECOVERY_THREADS}-thread safety limit"
                ));
            }
            match page.next_cursor {
                Some(next_cursor) => {
                    if !seen_loaded_cursors.insert(next_cursor.clone()) {
                        return Err(
                            "Codex loaded-thread recovery pagination repeated a cursor".to_string()
                        );
                    }
                    loaded_cursor = Some(next_cursor);
                }
                None => {
                    loaded_cursor = None;
                    break;
                }
            }
        }
        if loaded_cursor.is_some() {
            return Err(format!(
                "Codex loaded-thread recovery exceeded the {MAX_RECOVERY_PAGES}-page safety limit"
            ));
        }

        let mut seen_loaded_ids = HashSet::new();
        for thread_id in loaded_ids {
            if !seen_loaded_ids.insert(thread_id.clone()) || candidates.contains_key(&thread_id) {
                continue;
            }
            let params = protocol::recovery_thread_read_params(&thread_id)?;
            let candidate =
                protocol::parse_recovery_thread_read(self.request_ready("thread/read", params)?)?;
            if candidate.thread.id != thread_id {
                return Err(
                    "Codex returned a different thread during loaded-thread recovery".to_string(),
                );
            }
            if recovery_candidate_matches_root(&candidate, &workspace_root)? {
                candidates.insert(thread_id, candidate);
            }
        }

        let mut candidates = candidates.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.thread.id.cmp(&right.thread.id));
        Ok(candidates)
    }

    pub(crate) fn recovery_thread_read(
        &self,
        thread_id: &str,
    ) -> Result<CodeRecoveryThread, String> {
        let params = protocol::recovery_thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_recovery_thread_read(result)
    }

    #[cfg(test)]
    pub(crate) fn turn_start_at(
        &self,
        input: CodeTurnStartInput,
        workspace_root: &str,
    ) -> Result<CodeTurnSummary, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let selection = turn_selection(input.model.as_deref(), input.effort.as_deref())?;
        let (generation, token, pending) = {
            let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
            refresh_process_health(&mut runtime, &self.approvals, &self.events);
            if runtime.phase != CodeRuntimePhase::Ready {
                return Err("Codex app-server is not ready".to_string());
            }
            let generation = runtime.generation;
            let process = runtime
                .process
                .as_ref()
                .ok_or_else(|| "Codex app-server is not running".to_string())?;
            if let Some(selection) = selection.as_ref() {
                collect_model_catalog_from_process(generation, process)?
                    .require_selection(selection)?;
            }
            let mut events = self
                .events
                .inner
                .lock()
                .map_err(|error| error.to_string())?;
            ensure_event_generation(&events, generation)?;
            let token = begin_turn_start_locked(&mut events, &input.thread_id)?;
            let pending = match process.begin_request_with_delivery("turn/start", params) {
                Ok(pending) => pending,
                Err(error) => {
                    let uncertain_delivery = !error.definitely_not_sent();
                    let message = error.into_message();
                    if let Err(cleanup_error) = fail_turn_start_locked(
                        &mut events,
                        &input.thread_id,
                        token,
                        uncertain_delivery,
                    ) {
                        return Err(format!(
                            "{message}; turn/start state cleanup failed: {cleanup_error}"
                        ));
                    }
                    return Err(message);
                }
            };
            (generation, token, pending)
        };
        let result = match pending.wait(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(error) => {
                let uncertain_delivery = !error.definitely_not_sent();
                let message = error.into_message();
                if let Err(cleanup_error) = self.events.fail_turn_start(
                    generation,
                    &input.thread_id,
                    token,
                    uncertain_delivery,
                ) {
                    return Err(format!(
                        "{message}; turn/start state cleanup failed: {cleanup_error}"
                    ));
                }
                return Err(message);
            }
        };
        let turn = match protocol::parse_turn_start(result) {
            Ok(turn) => turn,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        let status = match CodePinnedTurnStatus::parse(&turn.status) {
            Ok(status) => status,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        self.events
            .complete_turn_start(generation, &input.thread_id, token, &turn.id, status)?;
        Ok(turn)
    }

    /// Start a turn with lifecycle validation, native active-turn reservation,
    /// and JSON-RPC byte admission serialized under one event barrier.
    pub(crate) fn turn_start_at_guarded(
        &self,
        input: CodeTurnStartInput,
        workspace_root: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeTurnSummary, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let selection = turn_selection(input.model.as_deref(), input.effort.as_deref())?;
        let (generation, token, pending) = {
            let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
            refresh_process_health(&mut runtime, &self.approvals, &self.events);
            if runtime.phase != CodeRuntimePhase::Ready
                || runtime.generation != checkpoint.generation
            {
                return Err(
                    "Codex turn/start belongs to an inactive runtime generation".to_string()
                );
            }
            let process = runtime
                .process
                .as_ref()
                .ok_or_else(|| "Codex app-server is not running".to_string())?;
            if let Some(selection) = selection.as_ref() {
                collect_model_catalog_from_process(checkpoint.generation, process)?
                    .require_selection(selection)?;
            }
            let mut events = self
                .events
                .inner
                .lock()
                .map_err(|error| error.to_string())?;
            ensure_event_generation(&events, checkpoint.generation)?;
            validate_exact_lifecycle_checkpoint_locked(
                &events,
                checkpoint.generation,
                &input.thread_id,
                &checkpoint,
            )?;
            if checkpoint.dirty {
                return Err(
                    "Codex thread lifecycle is dirty and cannot admit turn/start".to_string(),
                );
            }
            let token = begin_turn_start_locked(&mut events, &input.thread_id)?;
            let pending = match process.begin_request_with_delivery("turn/start", params) {
                Ok(pending) => pending,
                Err(error) => {
                    let uncertain_delivery = !error.definitely_not_sent();
                    let message = error.into_message();
                    if let Err(cleanup_error) = fail_turn_start_locked(
                        &mut events,
                        &input.thread_id,
                        token,
                        uncertain_delivery,
                    ) {
                        return Err(format!(
                            "{message}; turn/start state cleanup failed: {cleanup_error}"
                        ));
                    }
                    return Err(message);
                }
            };
            (checkpoint.generation, token, pending)
        };
        let result = match pending.wait(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(error) => {
                let message = error.into_message();
                if let Err(cleanup_error) =
                    self.events
                        .fail_turn_start(generation, &input.thread_id, token, true)
                {
                    return Err(format!(
                        "{message}; turn/start state cleanup failed: {cleanup_error}"
                    ));
                }
                return Err(message);
            }
        };
        let turn = match protocol::parse_turn_start(result) {
            Ok(turn) => turn,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        let status = match CodePinnedTurnStatus::parse(&turn.status) {
            Ok(status) => status,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        self.events
            .complete_turn_start(generation, &input.thread_id, token, &turn.id, status)?;
        Ok(turn)
    }

    #[cfg(test)]
    pub fn turn_steer(&self, input: CodeTurnSteerInput) -> Result<CodeTurnSummary, String> {
        let params = input.rpc_params()?;
        let result = self.request_ready("turn/steer", params)?;
        protocol::parse_turn_steer(result)
    }

    /// Steer an active turn only while the exact bound thread remains in the
    /// lifecycle state covered by `checkpoint` through JSON-RPC byte admission.
    pub(crate) fn turn_steer_guarded(
        &self,
        input: CodeTurnSteerInput,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeTurnSummary, String> {
        let params = input.rpc_params()?;
        let (_, pending) = self
            .begin_active_request(&input.thread_id, checkpoint, "turn/steer", params, None)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        protocol::parse_turn_steer(result)
    }

    pub fn turn_interrupt(&self, input: CodeTurnInterruptInput) -> Result<(), String> {
        let params = input.rpc_params()?;
        self.request_ready("turn/interrupt", params)?;
        let generation = self
            .inner
            .lock()
            .map_err(|error| error.to_string())?
            .generation;
        self.approvals
            .clear_turn(generation, &input.thread_id, &input.turn_id);
        Ok(())
    }

    #[cfg(test)]
    pub fn approval_respond(&self, input: CodeApprovalResponseInput) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready || inner.generation != input.runtime_generation {
            return Err("Codex approval belongs to an inactive runtime generation".to_string());
        }
        let process = inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        respond_to_pending_approval(&self.approvals, &input, |request_id, result| {
            process.respond(request_id, result)
        })
    }

    /// Respond to one approval with exact lifecycle validation and response
    /// byte admission serialized against native lifecycle notifications.
    pub(crate) fn approval_respond_guarded(
        &self,
        input: CodeApprovalResponseInput,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<(), String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready
            || runtime.generation != input.runtime_generation
            || runtime.generation != checkpoint.generation
        {
            return Err("Codex approval belongs to an inactive runtime generation".to_string());
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, checkpoint.generation)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            &input.thread_id,
            &checkpoint,
        )?;
        if checkpoint.dirty {
            return Err(
                "Codex thread lifecycle is dirty and cannot admit an approval response".to_string(),
            );
        }
        let process = runtime
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        respond_to_pending_approval(&self.approvals, &input, |request_id, result| {
            process.respond(request_id, result)
        })
    }

    pub(super) fn begin_lifecycle_mutation(
        &self,
        input: &CodeThreadLifecycleInput,
        proof: CodeThreadLifecycleGraphProof,
        expected: CodeThreadLifecycleSignal,
        params: Value,
    ) -> Result<(PendingRuntimeRequest, LifecycleWriteReceipt), CodeRpcDeliveryError> {
        if proof.thread_id != input.thread_id {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex lifecycle graph proof belongs to a different thread".to_string(),
            ));
        }
        let membership = proof
            .graph
            .ensure_leaf(&proof.thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let expected_membership = match expected {
            CodeThreadLifecycleSignal::Archived => CodeThreadMembership::Active,
            CodeThreadLifecycleSignal::Unarchived => CodeThreadMembership::Archived,
        };
        if membership != expected_membership {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex authoritative graph membership does not authorize the lifecycle mutation"
                    .to_string(),
            ));
        }
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != proof.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex lifecycle graph proof belongs to an inactive runtime generation".to_string(),
            ));
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, proof.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        if events.topology_boundary_revision != proof.topology_boundary_revision
            || events.next_topology_revision != proof.topology_revision
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread topology changed after the exhaustive leaf proof".to_string(),
            ));
        }
        let lifecycle = lifecycle_checkpoint_locked(&events, proof.generation, &proof.thread_id);
        if lifecycle.dirty {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread lifecycle is not durably reconciled for mutation admission"
                    .to_string(),
            ));
        }
        if events
            .active_turns
            .keys()
            .any(|(thread_id, _)| thread_id == &proof.thread_id)
            || events.inflight_turn_starts.contains_key(&proof.thread_id)
            || events.uncertain_turn_threads.contains(&proof.thread_id)
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread gained active or uncertain turn state before lifecycle mutation"
                    .to_string(),
            ));
        }
        let _approval_guard = self
            .approvals
            .lock_without_thread_approval(proof.generation, &proof.thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        let method = match expected {
            CodeThreadLifecycleSignal::Archived => "thread/archive",
            CodeThreadLifecycleSignal::Unarchived => "thread/unarchive",
        };
        let pending = process.begin_request_with_delivery(method, params)?;
        let receipt = LifecycleWriteReceipt {
            generation: proof.generation,
            thread_id: proof.thread_id,
            expected,
            lifecycle_boundary_revision: events.lifecycle_boundary_revision,
            topology_boundary_revision: events.topology_boundary_revision,
            topology_revision: events.next_topology_revision,
        };
        Ok((pending, receipt))
    }

    pub(super) fn begin_active_request(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        method: &str,
        params: Value,
        model_requirement: Option<CodeModelCatalogRequirement<'_>>,
    ) -> Result<(u64, PendingRuntimeRequest), CodeRpcDeliveryError> {
        protocol::validate_id("active-only request thread", thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex active-only request belongs to an inactive runtime generation".to_string(),
            ));
        }
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        if let Some(requirement) = model_requirement {
            let catalog = collect_model_catalog_from_process(checkpoint.generation, process)
                .map_err(CodeRpcDeliveryError::NotSent)?;
            requirement
                .validate(&catalog)
                .map_err(CodeRpcDeliveryError::NotSent)?;
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, checkpoint.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )
        .map_err(CodeRpcDeliveryError::NotSent)?;
        if checkpoint.dirty {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread lifecycle is dirty and cannot admit an active-only request"
                    .to_string(),
            ));
        }
        let pending = process.begin_request_with_delivery(method, params)?;
        Ok((checkpoint.generation, pending))
    }

    pub(super) fn begin_fork_request(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        params: Value,
    ) -> Result<(PendingRuntimeRequest, CodeThreadForkCompletion), CodeRpcDeliveryError> {
        protocol::validate_id("fork source thread", thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex fork belongs to an inactive runtime generation".to_string(),
            ));
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, checkpoint.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )
        .map_err(CodeRpcDeliveryError::NotSent)?;
        if checkpoint.dirty {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread lifecycle is dirty and cannot admit thread/fork".to_string(),
            ));
        }
        if events
            .active_turns
            .keys()
            .any(|(active_thread_id, _)| active_thread_id == thread_id)
            || events.inflight_turn_starts.contains_key(thread_id)
            || events.uncertain_turn_threads.contains(thread_id)
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex source thread gained active or uncertain turn state before fork".to_string(),
            ));
        }
        let _approval_guard = self
            .approvals
            .lock_without_thread_approval(checkpoint.generation, thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        let activity_revision = events
            .thread_activity_revisions
            .get(thread_id)
            .copied()
            .unwrap_or_default();
        #[cfg(test)]
        if self
            .fail_next_fork_before_write
            .swap(false, Ordering::AcqRel)
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "injected Codex fork failure before byte admission".to_string(),
            ));
        }
        let pending = process.begin_request_with_delivery("thread/fork", params)?;
        Ok((
            pending,
            CodeThreadForkCompletion {
                generation: checkpoint.generation,
                source_thread_id: thread_id.to_string(),
                lifecycle_checkpoint: checkpoint,
                activity_revision,
            },
        ))
    }

    pub(super) fn begin_recovery_resume_request(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        params: Value,
        model_requirement: Option<CodeModelCatalogRequirement<'_>>,
    ) -> Result<(u64, PendingRuntimeRequest), CodeRpcDeliveryError> {
        protocol::validate_id("recovery resume thread", thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex recovery resume belongs to an inactive runtime generation".to_string(),
            ));
        }
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        if let Some(requirement) = model_requirement {
            let catalog = collect_model_catalog_from_process(checkpoint.generation, process)
                .map_err(CodeRpcDeliveryError::NotSent)?;
            requirement
                .validate(&catalog)
                .map_err(CodeRpcDeliveryError::NotSent)?;
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, checkpoint.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )
        .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_new_thread_lifecycle_locked(&events, thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let pending = process.begin_request_with_delivery("thread/resume", params)?;
        Ok((checkpoint.generation, pending))
    }

    pub(super) fn begin_ready_request(
        &self,
        method: &str,
        params: Value,
        model_requirement: Option<CodeModelCatalogRequirement<'_>>,
    ) -> Result<(u64, PendingRuntimeRequest), CodeRpcDeliveryError> {
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex app-server is not ready".to_string(),
            ));
        }
        let generation = runtime.generation;
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        if let Some(requirement) = model_requirement {
            let catalog = collect_model_catalog_from_process(generation, process)
                .map_err(CodeRpcDeliveryError::NotSent)?;
            requirement
                .validate(&catalog)
                .map_err(CodeRpcDeliveryError::NotSent)?;
        }
        let pending = process.begin_request_with_delivery(method, params)?;
        Ok((generation, pending))
    }

    pub(super) fn ready_generation(&self) -> Result<u64, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        if inner.process.is_none() {
            return Err("Codex app-server is not running".to_string());
        }
        Ok(inner.generation)
    }

    pub(super) fn request_ready(&self, method: &str, params: Value) -> Result<Value, String> {
        let generation = self.ready_generation()?;
        self.request_ready_at_generation(generation, method, params)
    }

    pub(super) fn request_ready_at_generation(
        &self,
        generation: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        self.request_ready_with_delivery_at_generation(generation, method, params)
            .map_err(CodeRpcDeliveryError::into_message)
    }

    pub(super) fn request_ready_with_delivery_at_generation(
        &self,
        generation: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, CodeRpcDeliveryError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready || inner.generation != generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex app-server runtime generation changed".to_string(),
            ));
        }
        inner
            .process
            .as_ref()
            .ok_or_else(|| {
                CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
            })?
            .request_with_delivery(method, params, REQUEST_TIMEOUT)
    }
}
