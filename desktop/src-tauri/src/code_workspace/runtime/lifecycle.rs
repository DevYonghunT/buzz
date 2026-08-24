use super::*;

impl Default for CodeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeInner {
                phase: CodeRuntimePhase::Stopped,
                generation: 0,
                probe: None,
                initialized: None,
                process: None,
                last_error: None,
            })),
            events: Arc::new(EventBridge::new()),
            approvals: Arc::new(PendingApprovalStore::default()),
            explicit_executable: None,
            #[cfg(test)]
            fail_next_fork_before_write: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_executable(path: PathBuf) -> Self {
        let mut runtime = Self::new();
        runtime.explicit_executable = Some(path);
        runtime
    }

    #[cfg(test)]
    pub(crate) fn fail_next_fork_before_write_for_test(&self) {
        self.fail_next_fork_before_write
            .store(true, Ordering::Release);
    }

    pub fn probe(&self) -> CodeRuntimeProbe {
        let probe = probe_codex(self.explicit_executable.as_deref());
        let egress_probe = probe.redacted_for_egress();
        if let Ok(mut inner) = self.inner.lock() {
            inner.probe = Some(probe.clone());
            if inner.process.is_none() {
                inner.phase = if probe.available {
                    CodeRuntimePhase::Stopped
                } else {
                    CodeRuntimePhase::NotInstalled
                };
                inner.last_error = egress_probe.error.clone();
            }
        }
        egress_probe
    }

    pub fn status(&self) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        Ok(status_from_inner(&inner, self.events.len()))
    }

    pub(crate) fn replace_emitter_if_ready(
        &self,
        emitter: CodeEventEmitter,
    ) -> Result<Option<CodeRuntimeStatus>, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Ok(None);
        }
        self.events.replace_emitter(emitter)?;
        Ok(Some(status_from_inner(&inner, self.events.len())))
    }

    pub(crate) fn start(&self, emitter: CodeEventEmitter) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase == CodeRuntimePhase::Ready {
            self.events.replace_emitter(emitter)?;
            return Ok(status_from_inner(&inner, self.events.len()));
        }
        if let Err(error) = stop_runtime_process(&mut inner) {
            let detail =
                format!("failed to verify shutdown of the previous Codex app-server: {error}");
            inner.phase = CodeRuntimePhase::Failed;
            inner.initialized = None;
            inner.last_error = Some(detail.clone());
            return Err(detail);
        }

        inner.generation = inner
            .generation
            .checked_add(1)
            .ok_or_else(|| "Codex runtime generation was exhausted".to_string())?;
        let generation = inner.generation;
        self.events.reset(generation, emitter)?;
        self.approvals.reset(generation);
        inner.phase = CodeRuntimePhase::Starting;
        inner.initialized = None;
        inner.last_error = None;

        let probe = probe_codex(self.explicit_executable.as_deref());
        inner.probe = Some(probe.clone());
        if !probe.available {
            let error = probe
                .redacted_for_egress()
                .error
                .unwrap_or_else(|| "Codex CLI is not available".to_string());
            inner.phase = CodeRuntimePhase::NotInstalled;
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        if let Err(error) = ensure_supported_codex_version(&probe) {
            inner.phase = CodeRuntimePhase::Failed;
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        let executable = probe
            .executable
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| "Codex probe returned no executable path".to_string())?;

        let mut process = match RuntimeProcess::spawn(
            executable,
            generation,
            Arc::clone(&self.events),
            Arc::clone(&self.approvals),
        ) {
            Ok(process) => process,
            Err(error) => {
                inner.phase = CodeRuntimePhase::Failed;
                inner.last_error = Some(error.clone());
                return Err(error);
            }
        };
        inner.phase = CodeRuntimePhase::Initializing;
        match process.initialize() {
            Ok(initialized) => {
                inner.initialized = Some(initialized);
                inner.phase = CodeRuntimePhase::Ready;
                inner.process = Some(process);
                Ok(status_from_inner(&inner, self.events.len()))
            }
            Err(error) => {
                let stderr = process.stderr_tail();
                inner.process = Some(process);
                let stop_error = stop_runtime_process(&mut inner).err();
                self.approvals.clear_generation(generation);
                let mut detail = if stderr.trim().is_empty() {
                    error
                } else {
                    format!("{error} ({})", first_line(&stderr))
                };
                if let Some(stop_error) = stop_error {
                    detail.push_str(&format!(
                        "; failed to verify app-server shutdown: {stop_error}"
                    ));
                }
                inner.phase = CodeRuntimePhase::Failed;
                inner.last_error = Some(detail.clone());
                Err(detail)
            }
        }
    }

    pub fn stop(&self) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        inner.phase = CodeRuntimePhase::Stopping;
        let generation = inner.generation;
        let result = stop_runtime_process(&mut inner);
        self.events.clear_activity(generation)?;
        self.approvals.clear_generation(generation);
        inner.initialized = None;
        if let Err(error) = result {
            inner.phase = CodeRuntimePhase::Failed;
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        inner.phase = CodeRuntimePhase::Stopped;
        inner.last_error = None;
        Ok(status_from_inner(&inner, self.events.len()))
    }

    pub(crate) fn events(
        &self,
        runtime_generation: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Result<CodeRuntimeEventBacklog, String> {
        self.events
            .snapshot(&self.approvals, runtime_generation, after_sequence)
    }

    pub(crate) fn thread_start_at(
        &self,
        input: CodeThreadStartInput,
        workspace_root: &str,
    ) -> Result<CodeThreadRpcOpenResult, CodeThreadStartRpcError> {
        let workspace_root =
            canonical_workspace_root(workspace_root).map_err(CodeThreadStartRpcError::NotSent)?;
        let params = input
            .rpc_params(&workspace_root)
            .map_err(CodeThreadStartRpcError::NotSent)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (_, pending) = self.begin_ready_request("thread/start", params, requirement)?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        protocol::parse_thread_open(result).map_err(CodeThreadStartRpcError::Uncertain)
    }

    /// Return a bounded, strict visible model catalog for one ready generation.
    pub(crate) fn model_catalog(&self) -> Result<CodeModelCatalogSnapshot, String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        let process = runtime
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        collect_model_catalog_from_process(runtime.generation, process)
    }

    /// Fork one exact stable-active source into a native-owned destination.
    /// Lifecycle, turn, and approval admission stay serialized through the
    /// JSON-RPC byte write; delivery after that point is deliberately sticky.
    pub(crate) fn thread_fork_guarded(
        &self,
        input: &CodeThreadForkInput,
        workspace_root: &str,
        preparation_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadForkGuardedResult, CodeRpcDeliveryError> {
        let workspace_root =
            canonical_workspace_root(workspace_root).map_err(CodeRpcDeliveryError::NotSent)?;
        let params = input
            .rpc_params(&workspace_root, preparation_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let (pending, completion) =
            self.begin_fork_request(&input.thread_id, checkpoint, params)?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        let opened =
            protocol::parse_thread_open(result).map_err(CodeRpcDeliveryError::Uncertain)?;
        Ok(CodeThreadForkGuardedResult { opened, completion })
    }

    pub(crate) fn ensure_ready(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn thread_resume_at(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (generation, pending) = self
            .begin_ready_request("thread/resume", params, requirement)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let resumed = match protocol::parse_thread_open(result) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = self
                    .events
                    .mark_thread_uncertain(generation, &input.thread_id);
                return Err(error);
            }
        };
        if resumed.thread.id != input.thread_id {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err("Codex returned a different thread while resuming".to_string());
        }
        if let Err(error) = self
            .events
            .reconcile_thread_summary(generation, &resumed.thread)
        {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err(error);
        }
        Ok(resumed)
    }

    /// Resume one durable active thread with lifecycle validation held through
    /// JSON-RPC byte admission.
    pub(crate) fn thread_resume_at_guarded(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (generation, pending) = self
            .begin_active_request(
                &input.thread_id,
                checkpoint,
                "thread/resume",
                params,
                requirement,
            )
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let resumed = match protocol::parse_thread_open(result) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = self
                    .events
                    .mark_thread_uncertain(generation, &input.thread_id);
                return Err(error);
            }
        };
        if resumed.thread.id != input.thread_id {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err("Codex returned a different thread while resuming".to_string());
        }
        if let Err(error) = self
            .events
            .reconcile_thread_summary(generation, &resumed.thread)
        {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err(error);
        }
        Ok(resumed)
    }

    /// Resume one unbound recovery candidate while the current-generation new
    /// thread seam remains free of an exact lifecycle signal through RPC write.
    pub(crate) fn thread_resume_recovery_at_guarded(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (generation, pending) = self
            .begin_recovery_resume_request(&input.thread_id, checkpoint, params, requirement)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let resumed = match protocol::parse_thread_open(result) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = self
                    .events
                    .mark_thread_uncertain(generation, &input.thread_id);
                return Err(error);
            }
        };
        if resumed.thread.id != input.thread_id {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err("Codex returned a different thread while resuming".to_string());
        }
        if let Err(error) = self
            .events
            .reconcile_thread_summary(generation, &resumed.thread)
        {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err(error);
        }
        Ok(resumed)
    }

    pub(crate) fn thread_read(&self, thread_id: &str) -> Result<CodeThreadSummary, String> {
        let params = protocol::thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_thread_read(result)
    }

    /// Read normalized timeline metadata with the pinned `includeTurns:true`
    /// contract for rows that cannot be resumed to hydrate their turns.
    pub(crate) fn thread_read_with_turns(
        &self,
        thread_id: &str,
    ) -> Result<CodeThreadSummary, String> {
        let params = authoritative_thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_thread_read(result)
    }

    /// Fetch a complete cwd-free active+archived snapshot from one runtime
    /// generation and validate its pinned ancestry graph.
    pub(crate) fn authoritative_thread_graph(
        &self,
        deferred_target_ids: &[String],
        pending_forks: &[CodePendingForkExpectation],
    ) -> Result<CodeAuthoritativeThreadGraph, String> {
        let deferred_targets = validate_deferred_targets(deferred_target_ids)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        let generation = inner.generation;
        let (topology_boundary_revision, topology_revision) =
            self.events.topology_checkpoint(generation)?;
        let process = inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        let graph = collect_authoritative_thread_graph_with_pending_forks(
            &deferred_targets,
            pending_forks,
            |method, params| process.request(method, params, REQUEST_TIMEOUT),
        )?;
        self.events.confirm_topology_checkpoint(
            generation,
            topology_boundary_revision,
            topology_revision,
        )?;
        Ok(graph)
    }

    /// Fetch one exhaustive graph after idle/terminal drain and retain the
    /// topology epoch that a guarded lifecycle write must consume.
    pub(crate) fn authoritative_thread_graph_for_lifecycle_admission(
        &self,
        deferred_target_ids: &[String],
        pending_forks: &[CodePendingForkExpectation],
        thread_id: &str,
    ) -> Result<CodeThreadLifecycleGraphProof, String> {
        protocol::validate_id("lifecycle graph target", thread_id)?;
        let deferred_targets = validate_deferred_targets(deferred_target_ids)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        let generation = inner.generation;
        let (topology_boundary_revision, topology_revision) =
            self.events.topology_checkpoint(generation)?;
        let process = inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        let graph = collect_authoritative_thread_graph_with_pending_forks(
            &deferred_targets,
            pending_forks,
            |method, params| process.request(method, params, REQUEST_TIMEOUT),
        )?;
        self.events.confirm_topology_checkpoint(
            generation,
            topology_boundary_revision,
            topology_revision,
        )?;
        Ok(CodeThreadLifecycleGraphProof {
            generation,
            thread_id: thread_id.to_string(),
            graph,
            topology_boundary_revision,
            topology_revision,
        })
    }

    /// Atomically consume an exhaustive leaf proof, native idle state, and the
    /// no-approval gate through the `thread/archive` JSON-RPC byte write.
    pub(crate) fn thread_archive_guarded(
        &self,
        input: &CodeThreadLifecycleInput,
        proof: CodeThreadLifecycleGraphProof,
    ) -> Result<CodeThreadLifecycleMutationCompletion, CodeRpcDeliveryError> {
        let params = input.rpc_params().map_err(CodeRpcDeliveryError::NotSent)?;
        let (pending, receipt) = self.begin_lifecycle_mutation(
            input,
            proof,
            CodeThreadLifecycleSignal::Archived,
            params,
        )?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        parse_thread_archive(result).map_err(CodeRpcDeliveryError::Uncertain)?;
        self.events
            .mutation_response_checkpoint(receipt)
            .map_err(CodeRpcDeliveryError::Uncertain)
    }

    /// Atomically consume an exhaustive membership proof and native gates
    /// through the `thread/unarchive` JSON-RPC byte write.
    pub(crate) fn thread_unarchive_guarded(
        &self,
        input: &CodeThreadLifecycleInput,
        proof: CodeThreadLifecycleGraphProof,
    ) -> Result<CodeThreadUnarchiveGuardedResult, CodeRpcDeliveryError> {
        let params = input.rpc_params().map_err(CodeRpcDeliveryError::NotSent)?;
        let (pending, receipt) = self.begin_lifecycle_mutation(
            input,
            proof,
            CodeThreadLifecycleSignal::Unarchived,
            params,
        )?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        let thread = parse_thread_unarchive(result).map_err(CodeRpcDeliveryError::Uncertain)?;
        let completion = self
            .events
            .mutation_response_checkpoint(receipt)
            .map_err(CodeRpcDeliveryError::Uncertain)?;
        Ok(CodeThreadUnarchiveGuardedResult { thread, completion })
    }

    /// Consume a successful archive completion only after the matching durable
    /// binding transition is committed.
    pub(crate) fn complete_thread_archive_lifecycle<T>(
        &self,
        thread_id: &str,
        completion: CodeThreadLifecycleMutationCompletion,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != completion.generation {
            return Err(
                "Codex archive completion belongs to an inactive runtime generation".to_string(),
            );
        }
        self.events.complete_lifecycle_mutation(
            thread_id,
            completion,
            CodeThreadLifecycleSignal::Archived,
            commit,
        )
    }

    /// Consume a successful unarchive completion only after the matching
    /// durable binding transition is committed.
    pub(crate) fn complete_thread_unarchive_lifecycle<T>(
        &self,
        thread_id: &str,
        completion: CodeThreadLifecycleMutationCompletion,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != completion.generation {
            return Err(
                "Codex unarchive completion belongs to an inactive runtime generation".to_string(),
            );
        }
        self.events.complete_lifecycle_mutation(
            thread_id,
            completion,
            CodeThreadLifecycleSignal::Unarchived,
            commit,
        )
    }

    /// Whether a current-generation approval is pending or response-reserved
    /// for one exact thread.
    #[cfg(test)]
    pub fn has_pending_approval(&self, thread_id: &str) -> Result<bool, String> {
        let generation = self.ready_generation()?;
        self.approvals.has_for_thread(generation, thread_id)
    }

    #[cfg(test)]
    pub(crate) fn insert_pending_approval_for_test(
        &self,
        generation: u64,
        request_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        let inserted = self.approvals.insert_request(
            generation,
            json!(request_id),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": thread_id,
                "turnId": format!("turn-{request_id}"),
                "itemId": format!("item-{request_id}"),
                "availableDecisions": ["accept", "decline"]
            })),
        )?;
        if inserted.is_none() {
            return Err("test approval request was not normalized".to_string());
        }
        Ok(())
    }

    /// Whether a lifecycle notification or runtime boundary requires durable
    /// reconciliation before this thread may use an active-only command.
    #[cfg(test)]
    pub(crate) fn is_thread_lifecycle_dirty(&self, thread_id: &str) -> Result<bool, String> {
        Ok(self
            .thread_lifecycle_dirty_checkpoint(thread_id)?
            .is_dirty())
    }

    /// Capture the exact generation/revision that a durable lifecycle
    /// reconciliation must cover before clearing the native dirty gate.
    pub(crate) fn thread_lifecycle_dirty_checkpoint(
        &self,
        thread_id: &str,
    ) -> Result<CodeThreadLifecycleDirtyCheckpoint, String> {
        let generation = self.ready_generation()?;
        self.events
            .lifecycle_dirty_checkpoint(generation, thread_id)
    }

    /// Clear one exact dirty gate only if no lifecycle notification or runtime
    /// boundary occurred since the supplied reconciliation checkpoint.
    pub(crate) fn clear_thread_lifecycle_dirty(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<(), String> {
        let generation = self.ready_generation()?;
        self.events
            .clear_lifecycle_dirty(generation, thread_id, checkpoint)
    }

    /// Atomically validate the current-generation creation seam, commit one
    /// durable binding, and make that exact new thread lifecycle-clean.
    pub(crate) fn commit_new_thread_lifecycle<T>(
        &self,
        thread_id: &str,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("new lifecycle thread", thread_id)?;
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready for a new thread commit".to_string());
        }
        let mut events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, runtime.generation)?;
        validate_new_thread_lifecycle_locked(&events, thread_id)?;
        ensure_lifecycle_clean_capacity_locked(&events, thread_id)?;
        let committed = commit()?;
        events.lifecycle_clean_threads.insert(thread_id.to_string());
        Ok(committed)
    }

    /// Atomically prove the fork source stayed lifecycle/activity-clean after
    /// request admission, commit the exact destination binding, and mark only
    /// that child as a newly clean thread.
    pub(crate) fn commit_new_fork_lifecycle<T>(
        &self,
        source_thread_id: &str,
        child_thread_id: &str,
        completion: CodeThreadForkCompletion,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("fork source thread", source_thread_id)?;
        protocol::validate_id("fork child thread", child_thread_id)?;
        if source_thread_id == child_thread_id {
            return Err("Codex fork child cannot reuse its source thread id".to_string());
        }
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready
            || runtime.generation != completion.generation
            || completion.source_thread_id != source_thread_id
        {
            return Err("Codex fork completion belongs to a stale source generation".to_string());
        }
        let mut events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, completion.generation)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            completion.generation,
            source_thread_id,
            &completion.lifecycle_checkpoint,
        )?;
        if completion.lifecycle_checkpoint.dirty
            || events
                .thread_activity_revisions
                .get(source_thread_id)
                .copied()
                .unwrap_or_default()
                != completion.activity_revision
            || events
                .active_turns
                .keys()
                .any(|(thread_id, _)| thread_id == source_thread_id)
            || events.inflight_turn_starts.contains_key(source_thread_id)
            || events.uncertain_turn_threads.contains(source_thread_id)
        {
            return Err(
                "Codex fork source changed after request admission; destination commit was refused"
                    .to_string(),
            );
        }
        validate_new_thread_lifecycle_locked(&events, child_thread_id)?;
        ensure_lifecycle_clean_capacity_locked(&events, child_thread_id)?;
        let committed = commit()?;
        events
            .lifecycle_clean_threads
            .insert(child_thread_id.to_string());
        Ok(committed)
    }

    /// Hold the exact native lifecycle barrier from checkpoint validation
    /// through one non-RPC action such as PTY spawn/registration or stdin ack.
    pub(crate) fn with_thread_lifecycle_admission<T>(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("active-only admission thread", thread_id)?;
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(
                "Codex active-only admission belongs to an inactive runtime generation".to_string(),
            );
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
            thread_id,
            &checkpoint,
        )?;
        if checkpoint.dirty {
            return Err(
                "Codex thread lifecycle is dirty and cannot admit an active-only operation"
                    .to_string(),
            );
        }
        action()
    }

    /// Prove the target quiescent with a strict `thread/read` while guarding
    /// against response/notification activity races and pending approvals.
    pub(crate) fn ensure_thread_idle(
        &self,
        thread_id: &str,
    ) -> Result<CodeAuthoritativeThreadActivity, String> {
        protocol::validate_id("archive target thread", thread_id)?;
        let generation = self.ready_generation()?;
        let before = self.events.activity_snapshot(generation, thread_id)?;
        if before.active_or_starting || before.uncertain {
            return Err("Codex thread has active, starting, or uncertain turn state".to_string());
        }
        if self.approvals.has_for_thread(generation, thread_id)? {
            return Err("Codex thread has a pending approval".to_string());
        }
        let params = authoritative_thread_read_params(thread_id)?;
        let result = self.request_ready_at_generation(generation, "thread/read", params)?;
        let activity = parse_authoritative_thread_read(result)?;
        if activity.id != thread_id {
            return Err("Codex authoritative read returned a different thread".to_string());
        }
        activity.ensure_quiescent()?;
        if self.approvals.has_for_thread(generation, thread_id)? {
            return Err("Codex thread gained a pending approval during the idle proof".to_string());
        }
        self.events
            .confirm_authoritative_idle(generation, thread_id, before.revision)?;
        Ok(activity)
    }

    /// Retain the exact runtime/activity/approval locks after an authoritative
    /// idle proof so no turn or approval can be admitted during a private
    /// physical-removal claim.
    pub(crate) fn lock_thread_idle_admission(
        &self,
        thread_id: &str,
    ) -> Result<CodeThreadIdleAdmissionGuard<'_>, String> {
        protocol::validate_id("removal admission thread", thread_id)?;
        let runtime = self.inner.lock().map_err(|error| error.to_string())?;
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err("Codex runtime is not ready for removal admission".to_string());
        }
        let generation = runtime.generation;
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, generation)?;
        if events
            .active_turns
            .keys()
            .any(|(candidate, _)| candidate == thread_id)
            || events.inflight_turn_starts.contains_key(thread_id)
            || events.uncertain_turn_threads.contains(thread_id)
        {
            return Err("Codex thread gained activity before removal admission".to_string());
        }
        let approvals = self
            .approvals
            .lock_without_thread_approval(generation, thread_id)?;
        Ok(CodeThreadIdleAdmissionGuard {
            _runtime: runtime,
            _events: events,
            _approvals: approvals,
        })
    }
}
