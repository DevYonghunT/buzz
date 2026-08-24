use super::*;

pub(super) fn start_thread_native(
    input: CodeThreadStartInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    let preparation_id = input.preparation_id.clone();
    let _guard = lock_bindings(binding_lock)
        .map_err(|error| CodeThreadStartError::simple("bindingLockUnavailable", error))?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)
        .map_err(|error| CodeThreadStartError::simple("bindingStoreUnavailable", error))?;
    input
        .scope
        .validate()
        .map_err(|error| CodeThreadStartError::simple("invalidScope", error))?;
    let preparation = store
        .prepared_preparation(&input.scope, &preparation_id)
        .map_err(|error| CodeThreadStartError::simple("preparationUnavailable", error))?;
    super::super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)
        .map_err(|error| CodeThreadStartError::simple("lifecycleAuthorityUnavailable", error))?;
    let status = revalidate_execution_root(&preparation.descriptor(), nest_root)
        .map_err(|error| CodeThreadStartError::simple("executionRootUnavailable", error))?;
    let execution_root = status.descriptor.execution_root;
    runtime
        .ensure_ready()
        .map_err(|error| CodeThreadStartError::simple("runtimeNotReady", error))?;
    // Validate every caller-controlled start field before the durable
    // preparation crosses the RPC boundary. After the claim, only a
    // transport-proven pre-write failure may restore it to `prepared`.
    input
        .rpc_params(&execution_root)
        .map_err(|error| CodeThreadStartError::simple("invalidStartInput", error))?;
    let recovery_thread_baseline = runtime
        .recovery_threads_at(&execution_root)
        .map_err(|error| CodeThreadStartError::simple("recoveryBaselineUnavailable", error))?
        .into_iter()
        .map(|candidate| candidate.thread.id)
        .collect();
    let preparation = store
        .claim_preparation_for_start(&input.scope, &preparation_id, recovery_thread_baseline)
        .map_err(|error| CodeThreadStartError::simple("preparationUnavailable", error))?;
    let opened = match runtime.thread_start_at(input, &execution_root) {
        Ok(opened) => opened,
        Err(error) => {
            let definitely_not_sent = error.definitely_not_sent();
            let message = error.into_message();
            if definitely_not_sent {
                match store.restore_preparation_after_unsent_start(&preparation) {
                    Ok(_) => {
                        return Err(CodeThreadStartError::recovery(
                            "threadStartNotSent",
                            format!(
                                "Codex thread start was not sent; the preparation was restored and can be retried: {message}"
                            ),
                            preparation_id.to_string(),
                            None,
                            Some(execution_root),
                        ));
                    }
                    Err(rollback_error) => {
                        return Err(CodeThreadStartError::recovery(
                            "startRollbackFailed",
                            format!(
                                "Codex thread start was not sent, but its preparation could not be restored: {message}; {rollback_error}"
                            ),
                            preparation_id.to_string(),
                            None,
                            Some(execution_root),
                        ));
                    }
                }
            }
            return Err(CodeThreadStartError::recovery(
                "threadStartUncertain",
                message,
                preparation_id.to_string(),
                None,
                Some(execution_root),
            ));
        }
    };
    let thread_id = opened.thread.id.clone();

    let commit_result = (|| {
        let expected_thread_source = code_thread_source(&preparation_id)?;
        if opened.thread_source.as_deref() != Some(expected_thread_source.as_str()) {
            return Err(
                "Codex returned a thread without the SchoolX Code source marker".to_string(),
            );
        }
        validate_thread_identity_and_root(
            &opened.thread.id,
            opened.thread.cwd.as_deref(),
            &thread_id,
            &execution_root,
        )?;
        let binding = runtime.commit_new_thread_lifecycle(&thread_id, || {
            store.commit_preparation_binding(&preparation.scope(), &preparation_id, &thread_id)
        })?;
        Ok(CodeBoundThreadOpenResult {
            binding,
            thread: opened.thread,
            instruction_sources: opened.instruction_sources,
            model: opened.model,
            reasoning_effort: opened.reasoning_effort,
        })
    })();

    commit_result.map_err(|error: String| {
        super::super::code_thread_lifecycle::invalidate_lifecycle_authority(lifecycle_authority);
        CodeThreadStartError::recovery(
            "bindingCommitFailed",
            format!(
                "Codex thread started, but its SchoolX Code binding could not be committed: {error}"
            ),
            preparation_id.to_string(),
            Some(thread_id),
            Some(execution_root),
        )
    })
}

pub(super) fn recover_thread_binding_native(
    input: CodeThreadBindingRecoverInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeBoundThreadOpenResult, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let preparation = store.preparation(&input.scope, &input.preparation_id)?;
    super::super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)?;
    if preparation.operation == CodeThreadPreparationOperation::Fork {
        return super::super::code_thread_fork::open_fork_preparation_locked(
            &input,
            preparation,
            &store,
            nest_root,
            runtime,
            terminal_manager,
            lifecycle_authority,
        );
    }
    let preparation = store.starting_preparation(&input.scope, &input.preparation_id)?;
    let execution_root = revalidate_execution_root(&preparation.descriptor(), nest_root)?
        .descriptor
        .execution_root;
    let candidates = runtime.recovery_threads_at(&execution_root)?;
    let reserved_thread_ids = store.load()?.reserved_thread_ids();
    let candidate = select_recovery_candidate(
        &preparation,
        candidates,
        &reserved_thread_ids,
        &execution_root,
    )?;
    let thread_id = candidate.thread.id.clone();
    store.ensure_thread_unbound(&thread_id)?;
    let existing = runtime.recovery_thread_read(&thread_id)?;
    validate_recovery_source(&preparation, &existing)?;
    validate_thread_identity_and_root(
        &existing.thread.id,
        existing.thread.cwd.as_deref(),
        &thread_id,
        &execution_root,
    )?;
    let resume = CodeThreadResumeInput {
        scope: input.scope.clone(),
        thread_id: thread_id.clone(),
        model: input.model,
    };
    let lifecycle_checkpoint = runtime.thread_lifecycle_dirty_checkpoint(&thread_id)?;
    let opened =
        runtime.thread_resume_recovery_at_guarded(resume, &execution_root, lifecycle_checkpoint)?;
    validate_recovery_source(
        &preparation,
        &CodeRecoveryThread {
            thread: opened.thread.clone(),
            thread_source: opened.thread_source.clone(),
            session_source: opened.session_source.clone(),
            ephemeral_present: opened.ephemeral_present,
        },
    )?;
    validate_thread_identity_and_root(
        &opened.thread.id,
        opened.thread.cwd.as_deref(),
        &thread_id,
        &execution_root,
    )?;
    let binding = runtime
        .commit_new_thread_lifecycle(&thread_id, || {
            store.commit_preparation_binding(&input.scope, &input.preparation_id, &thread_id)
        })
        .inspect_err(|_| {
            super::super::code_thread_lifecycle::invalidate_lifecycle_authority(
                lifecycle_authority,
            );
        })?;
    Ok(CodeBoundThreadOpenResult {
        binding,
        thread: opened.thread,
        instruction_sources: opened.instruction_sources,
        model: opened.model,
        reasoning_effort: opened.reasoning_effort,
    })
}

pub(super) fn resume_thread_native(
    input: CodeThreadResumeInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeBoundThreadOpenResult, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    };
    let binding = store.require_active_binding(&lookup)?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    let lifecycle_checkpoint =
        super::super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
            runtime,
            lifecycle_authority,
            &binding.codex_thread_id,
        )?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    let expected_thread_id = binding.codex_thread_id.clone();
    let opened = runtime.thread_resume_at_guarded(input, &execution_root, lifecycle_checkpoint)?;
    validate_thread_identity_and_root(
        &opened.thread.id,
        opened.thread.cwd.as_deref(),
        &expected_thread_id,
        &execution_root,
    )?;
    Ok(CodeBoundThreadOpenResult {
        binding,
        thread: opened.thread,
        instruction_sources: opened.instruction_sources,
        model: opened.model,
        reasoning_effort: opened.reasoning_effort,
    })
}

pub(super) fn start_turn_native(
    input: CodeTurnStartInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeTurnSummary, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    };
    let binding = store.require_active_binding(&lookup)?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    let lifecycle_checkpoint =
        super::super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
            runtime,
            lifecycle_authority,
            &binding.codex_thread_id,
        )?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    runtime.turn_start_at_guarded(input, &execution_root, lifecycle_checkpoint)
}

pub(super) fn steer_turn_native(
    input: CodeTurnSteerInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeTurnSummary, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    };
    let binding = store.require_active_binding(&lookup)?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    let lifecycle_checkpoint =
        super::super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
            runtime,
            lifecycle_authority,
            &binding.codex_thread_id,
        )?;
    revalidate_binding_root(&binding, nest_root)?;
    runtime.turn_steer_guarded(input, lifecycle_checkpoint)
}

#[cfg(test)]
pub(crate) fn start_turn_for_test(
    input: CodeTurnStartInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeTurnSummary, String> {
    start_turn_native(
        input,
        app_data_dir,
        nest_root,
        runtime,
        binding_lock,
        lifecycle_authority,
    )
}

#[cfg(test)]
pub(crate) fn steer_turn_for_test(
    input: CodeTurnSteerInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeTurnSummary, String> {
    steer_turn_native(
        input,
        app_data_dir,
        nest_root,
        runtime,
        binding_lock,
        lifecycle_authority,
    )
}
