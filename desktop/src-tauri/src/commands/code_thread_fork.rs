//! Thin Tauri facade for exact bound-thread fork creation and recovery.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::code_workspace::{
    code_thread_source, prepare_execution_root, revalidate_execution_root,
    CodeBoundThreadOpenResult, CodeExecutionMode, CodeRecoveryThread, CodeThreadBinding,
    CodeThreadBindingLookupInput, CodeThreadBindingRecoverInput, CodeThreadBindingStore,
    CodeThreadForkInput, CodeThreadPreparation, CodeThreadPreparationOperation,
    CodeThreadStartError, CodeThreadSummary, CodeWorktreeDescriptor, CodeWorktreePrepareInput,
    CodeWorktreeStatus,
};

#[tauri::command]
/// Fork one exact clean managed task into a fresh native-owned worktree.
pub async fn code_thread_fork(
    input: CodeThreadForkInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    let app_data_dir = code_app_data_dir(&app)
        .map_err(|error| CodeThreadStartError::simple("appDataUnavailable", error))?;
    let nest_root =
        code_nest_root().map_err(|error| CodeThreadStartError::simple("nestUnavailable", error))?;
    let runtime = state.code_runtime.clone();
    let terminal_manager = state.code_terminal_manager.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        fork_thread_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &terminal_manager,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| {
        CodeThreadStartError::simple(
            "threadForkTaskFailed",
            format!("Codex thread fork task failed: {error}"),
        )
    })?
}

fn fork_thread_native(
    input: CodeThreadForkInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    input
        .validate()
        .map_err(|error| CodeThreadStartError::simple("invalidForkInput", error))?;
    let _guard = lock_bindings(binding_lock)
        .map_err(|error| CodeThreadStartError::simple("bindingLockUnavailable", error))?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)
        .map_err(|error| CodeThreadStartError::simple("bindingStoreUnavailable", error))?;
    store
        .ensure_preparation_capacity()
        .map_err(|error| CodeThreadStartError::simple("preparationUnavailable", error))?;
    store
        .ensure_fork_source_available(&input.scope, &input.thread_id)
        .map_err(|error| CodeThreadStartError::simple("forkPreparationExists", error))?;
    let source = require_active_source(&store, &input)
        .map_err(|error| CodeThreadStartError::simple("sourceUnavailable", error))?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )
    .map_err(|error| CodeThreadStartError::simple("sourceGitRecoveryRequired", error))?;
    super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)
        .map_err(|error| CodeThreadStartError::simple("lifecycleAuthorityUnavailable", error))?;
    super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        runtime,
        lifecycle_authority,
        &input.thread_id,
    )
    .map_err(|error| CodeThreadStartError::simple("sourceLifecycleUnavailable", error))?;

    let source_status = require_clean_managed_source(&source, nest_root, None)
        .map_err(|error| CodeThreadStartError::simple("sourceWorktreeUnavailable", error))?;
    let activity = runtime
        .ensure_thread_idle(&source.codex_thread_id)
        .map_err(|error| CodeThreadStartError::simple("sourceThreadBusy", error))?;
    validate_reported_root(
        &activity.cwd,
        &source_status.descriptor.execution_root,
        "fork source idle proof",
    )
    .map_err(|error| CodeThreadStartError::simple("sourceThreadUnavailable", error))?;
    terminal_manager
        .terminate_owner(&input.scope, &input.thread_id)
        .map_err(|error| CodeThreadStartError::simple("sourceTerminalDrainFailed", error))?;
    let source_status =
        require_clean_managed_source(&source, nest_root, Some(&source_status.head_commit))
            .map_err(|error| CodeThreadStartError::simple("sourceWorktreeChanged", error))?;
    super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        runtime,
        lifecycle_authority,
        &input.thread_id,
    )
    .map_err(|error| CodeThreadStartError::simple("sourceLifecycleChanged", error))?;

    let destination = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: source_status.descriptor.execution_root.clone(),
            base_ref: source_status.head_commit.clone(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest_root,
    )
    .map_err(|error| CodeThreadStartError::simple("forkDestinationUnavailable", error))?;
    if destination.dirty
        || destination.head_commit != source_status.head_commit
        || destination.branch.is_some()
        || destination.descriptor.execution_root == source_status.descriptor.execution_root
        || destination.descriptor.worktree_id == source_status.descriptor.worktree_id
    {
        return Err(CodeThreadStartError::preserved_root(
            "forkDestinationInvalid",
            format!(
                "SchoolX Code did not create an isolated clean fork destination. The destination was preserved at {}",
                destination.descriptor.execution_root
            ),
            destination.descriptor.execution_root,
        ));
    }

    let preparation_id = uuid::Uuid::new_v4().hyphenated().to_string();
    let preparation = store
        .create_fork_preparation(
            preparation_id.clone(),
            input.scope.clone(),
            input.thread_id.clone(),
            &destination.descriptor,
        )
        .map_err(|error| {
            CodeThreadStartError::recovery(
                "forkPreparationJournalFailed",
                format!(
                    "SchoolX Code created fork destination {} but could not journal it: {error}. The destination was preserved",
                    destination.descriptor.execution_root
                ),
                preparation_id,
                None,
                Some(destination.descriptor.execution_root),
            )
        })?;
    submit_fork_preparation_locked(
        input,
        preparation,
        &store,
        nest_root,
        runtime,
        terminal_manager,
        lifecycle_authority,
    )
}

#[cfg(test)]
pub(crate) fn fork_thread_for_test(
    input: CodeThreadForkInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    fork_thread_native(
        input,
        app_data_dir,
        nest_root,
        runtime,
        terminal_manager,
        binding_lock,
        lifecycle_authority,
    )
}

pub(super) fn open_fork_preparation_locked(
    input: &CodeThreadBindingRecoverInput,
    preparation: CodeThreadPreparation,
    store: &CodeThreadBindingStore,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeBoundThreadOpenResult, String> {
    if preparation.operation != CodeThreadPreparationOperation::Fork {
        return Err("SchoolX Code preparation is not a fork operation".to_string());
    }
    if input.model.is_some() {
        return Err("SchoolX Code fork recovery does not accept a model override".to_string());
    }
    match preparation.state {
        crate::code_workspace::bindings::CodeThreadPreparationState::Prepared => {
            let source_thread_id = preparation.source_thread_id.clone().ok_or_else(|| {
                "SchoolX Code fork preparation is missing its source thread".to_string()
            })?;
            submit_fork_preparation_locked(
                CodeThreadForkInput {
                    scope: input.scope.clone(),
                    thread_id: source_thread_id,
                },
                preparation,
                store,
                nest_root,
                runtime,
                terminal_manager,
                lifecycle_authority,
            )
            .map_err(|error| error.message)
        }
        crate::code_workspace::bindings::CodeThreadPreparationState::Starting => {
            recover_fork_preparation_locked(
                input,
                &preparation,
                store,
                nest_root,
                runtime,
                lifecycle_authority,
            )
        }
    }
}

fn submit_fork_preparation_locked(
    input: CodeThreadForkInput,
    preparation: CodeThreadPreparation,
    store: &CodeThreadBindingStore,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    let preparation_id = preparation.preparation_id.clone();
    if preparation.operation != CodeThreadPreparationOperation::Fork
        || preparation.source_thread_id.as_deref() != Some(input.thread_id.as_str())
        || preparation.scope() != input.scope
        || preparation.state
            != crate::code_workspace::bindings::CodeThreadPreparationState::Prepared
    {
        return Err(CodeThreadStartError::recovery(
            "forkPreparationMismatch",
            "SchoolX Code fork continuation did not match the exact prepared operation".to_string(),
            preparation_id,
            None,
            Some(preparation.execution_root),
        ));
    }
    let source = require_active_source(store, &input)
        .map_err(|error| fork_error("sourceUnavailable", error, &preparation, None))?;
    super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)
        .map_err(|error| fork_error("lifecycleAuthorityUnavailable", error, &preparation, None))?;
    terminal_manager
        .terminate_owner(&input.scope, &input.thread_id)
        .map_err(|error| fork_error("sourceTerminalDrainFailed", error, &preparation, None))?;
    let source_status =
        require_clean_managed_source(&source, nest_root, Some(&preparation.base_ref))
            .map_err(|error| fork_error("sourceWorktreeChanged", error, &preparation, None))?;
    let destination_status = require_clean_destination(&preparation, nest_root, &source_status)
        .map_err(|error| fork_error("forkDestinationChanged", error, &preparation, None))?;
    let activity = runtime
        .ensure_thread_idle(&input.thread_id)
        .map_err(|error| fork_error("sourceThreadBusy", error, &preparation, None))?;
    validate_reported_root(
        &activity.cwd,
        &source_status.descriptor.execution_root,
        "fork source idle proof",
    )
    .map_err(|error| fork_error("sourceThreadUnavailable", error, &preparation, None))?;
    let recovery_baseline = runtime
        .recovery_threads_at(&destination_status.descriptor.execution_root)
        .map_err(|error| fork_error("recoveryBaselineUnavailable", error, &preparation, None))?
        .into_iter()
        .map(|candidate| candidate.thread.id)
        .collect();
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        runtime,
        lifecycle_authority,
        &input.thread_id,
    )
    .map_err(|error| fork_error("lifecycleAuthorityUnavailable", error, &preparation, None))?;
    let claimed = store
        .claim_preparation_for_fork(
            &input.scope,
            &preparation_id,
            &input.thread_id,
            recovery_baseline,
        )
        .map_err(|error| fork_error("preparationUnavailable", error, &preparation, None))?;

    if let Err(error) = require_clean_managed_source(&source, nest_root, Some(&claimed.base_ref))
        .and_then(|status| require_clean_destination(&claimed, nest_root, &status).map(|_| status))
    {
        return rollback_unsent_fork(
            store,
            &claimed,
            format!("SchoolX Code fork roots changed before RPC: {error}"),
        );
    }
    let guarded = match runtime.thread_fork_guarded(
        &input,
        &claimed.execution_root,
        &preparation_id,
        lifecycle_checkpoint,
    ) {
        Ok(opened) => opened,
        Err(error) if error.definitely_not_sent() => {
            return rollback_unsent_fork(store, &claimed, error.into_message());
        }
        Err(error) => {
            return Err(fork_error(
                "threadForkUncertain",
                error.into_message(),
                &claimed,
                None,
            ));
        }
    };
    let opened = guarded.opened;
    let destination_thread_id = opened.thread.id.clone();
    if let Err(error) = validate_fork_opened(
        &opened,
        &input.thread_id,
        &claimed.execution_root,
        &preparation_id,
    ) {
        return Err(fork_error(
            "threadForkUncertain",
            format!("Codex fork response could not be trusted: {error}"),
            &claimed,
            Some(destination_thread_id),
        ));
    }
    let binding = runtime
        .commit_new_fork_lifecycle(
            &input.thread_id,
            &destination_thread_id,
            guarded.completion,
            || {
            store.commit_preparation_binding(
                &input.scope,
                &preparation_id,
                &destination_thread_id,
            )
            },
        )
        .map_err(|error| {
            super::code_thread_lifecycle::invalidate_lifecycle_authority(lifecycle_authority);
            fork_error(
                "bindingCommitFailed",
                format!(
                    "Codex thread forked, but its SchoolX Code binding could not be committed: {error}"
                ),
                &claimed,
                Some(destination_thread_id.clone()),
            )
        })?;
    Ok(CodeBoundThreadOpenResult {
        binding,
        thread: opened.thread,
        instruction_sources: opened.instruction_sources,
        model: opened.model,
        reasoning_effort: opened.reasoning_effort,
    })
}

fn recover_fork_preparation_locked(
    input: &CodeThreadBindingRecoverInput,
    preparation: &CodeThreadPreparation,
    store: &CodeThreadBindingStore,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeBoundThreadOpenResult, String> {
    super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)?;
    let source_thread_id = preparation
        .source_thread_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code fork preparation is missing its source thread".to_string())?;
    let source = store
        .lookup(&CodeThreadBindingLookupInput {
            scope: input.scope.clone(),
            codex_thread_id: source_thread_id.to_string(),
        })?
        .ok_or_else(|| "SchoolX Code fork source binding is unavailable".to_string())?;
    validate_managed_source_identity(&source, preparation)?;
    let destination = revalidate_execution_root(&preparation.descriptor(), nest_root)?;
    if destination.dirty
        || destination.head_commit != preparation.base_ref
        || destination.branch.is_some()
    {
        return Err(
            "SchoolX Code fork destination changed after the uncertain request".to_string(),
        );
    }
    let candidates = runtime.recovery_threads_at(&destination.descriptor.execution_root)?;
    let reserved_thread_ids = store.load()?.reserved_thread_ids();
    let candidate = select_fork_recovery_candidate(
        preparation,
        candidates,
        &reserved_thread_ids,
        &destination.descriptor.execution_root,
    )?;
    let destination_thread_id = candidate.thread.id.clone();
    store.ensure_thread_unbound(&destination_thread_id)?;
    let existing = runtime.recovery_thread_read(&destination_thread_id)?;
    validate_fork_candidate(
        preparation,
        &existing,
        &destination.descriptor.execution_root,
        false,
    )?;
    let lifecycle_checkpoint = runtime.thread_lifecycle_dirty_checkpoint(&destination_thread_id)?;
    let opened = runtime.thread_resume_recovery_at_guarded(
        crate::code_workspace::CodeThreadResumeInput {
            scope: input.scope.clone(),
            thread_id: destination_thread_id.clone(),
            model: None,
        },
        &destination.descriptor.execution_root,
        lifecycle_checkpoint,
    )?;
    validate_fork_opened(
        &opened,
        source_thread_id,
        &destination.descriptor.execution_root,
        &preparation.preparation_id,
    )?;
    let binding = runtime
        .commit_new_thread_lifecycle(&destination_thread_id, || {
            store.commit_preparation_binding(
                &input.scope,
                &input.preparation_id,
                &destination_thread_id,
            )
        })
        .inspect_err(|_| {
            super::code_thread_lifecycle::invalidate_lifecycle_authority(lifecycle_authority);
        })?;
    Ok(CodeBoundThreadOpenResult {
        binding,
        thread: opened.thread,
        instruction_sources: opened.instruction_sources,
        model: opened.model,
        reasoning_effort: opened.reasoning_effort,
    })
}

fn select_fork_recovery_candidate(
    preparation: &CodeThreadPreparation,
    candidates: Vec<CodeRecoveryThread>,
    reserved_thread_ids: &HashSet<String>,
    expected_root: &str,
) -> Result<CodeRecoveryThread, String> {
    let baseline = preparation
        .recovery_thread_baseline
        .as_ref()
        .ok_or_else(|| {
            "SchoolX Code fork preparation predates safe recovery and cannot be auto-bound"
                .to_string()
        })?;
    let mut eligible = Vec::new();
    for candidate in candidates {
        if reserved_thread_ids.contains(&candidate.thread.id)
            || baseline
                .binary_search_by(|thread_id| thread_id.as_str().cmp(candidate.thread.id.as_str()))
                .is_ok()
        {
            continue;
        }
        if validate_fork_candidate(preparation, &candidate, expected_root, false).is_ok() {
            eligible.push(candidate);
        }
    }
    match eligible.len() {
        1 => Ok(eligible.remove(0)),
        0 => Err(
            "SchoolX Code could not find the exact fork child; the destination remains reserved"
                .to_string(),
        ),
        count => Err(format!(
            "SchoolX Code found {count} possible fork children and refused an ambiguous binding"
        )),
    }
}

fn validate_fork_candidate(
    preparation: &CodeThreadPreparation,
    candidate: &CodeRecoveryThread,
    expected_root: &str,
    require_idle: bool,
) -> Result<(), String> {
    let source_thread_id = preparation
        .source_thread_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code fork preparation is missing its source thread".to_string())?;
    validate_fork_thread(
        &candidate.thread,
        source_thread_id,
        expected_root,
        require_idle,
    )?;
    if !candidate.ephemeral_present {
        return Err("Codex fork child omitted its ephemeral flag".to_string());
    }
    let expected_source = code_thread_source(&preparation.preparation_id)?;
    if candidate.thread_source.as_deref() != Some(expected_source.as_str()) {
        return Err("Codex fork child did not retain the exact preparation source".to_string());
    }
    validate_app_server_source(candidate.session_source.as_ref())
}

fn validate_fork_opened(
    opened: &crate::code_workspace::CodeThreadRpcOpenResult,
    source_thread_id: &str,
    expected_root: &str,
    preparation_id: &str,
) -> Result<(), String> {
    validate_fork_thread(&opened.thread, source_thread_id, expected_root, true)?;
    if !opened.ephemeral_present {
        return Err("Codex fork response omitted its ephemeral flag".to_string());
    }
    let response_cwd = opened
        .response_cwd
        .as_deref()
        .ok_or_else(|| "Codex fork response did not report its destination cwd".to_string())?;
    validate_canonical_root(response_cwd, expected_root, "fork response cwd")?;
    let expected_source = code_thread_source(preparation_id)?;
    if opened.thread_source.as_deref() != Some(expected_source.as_str()) {
        return Err("Codex fork response did not retain the preparation source".to_string());
    }
    validate_app_server_source(opened.session_source.as_ref())
}

fn validate_fork_thread(
    thread: &CodeThreadSummary,
    source_thread_id: &str,
    expected_root: &str,
    require_idle: bool,
) -> Result<(), String> {
    if thread.id == source_thread_id {
        return Err("Codex fork returned the source thread id".to_string());
    }
    if thread.session_id.as_deref() != Some(thread.id.as_str()) {
        return Err("Codex fork child session id did not match its thread id".to_string());
    }
    if thread.forked_from_id.as_deref() != Some(source_thread_id)
        || thread.parent_thread_id.is_some()
    {
        return Err("Codex fork response ancestry did not match the bound source".to_string());
    }
    if thread.ephemeral {
        return Err("Codex fork response was unexpectedly ephemeral".to_string());
    }
    let cwd = thread
        .cwd
        .as_deref()
        .ok_or_else(|| "Codex fork child did not report its destination cwd".to_string())?;
    validate_canonical_root(cwd, expected_root, "fork child cwd")?;
    let status = thread
        .status
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|status| status.get("type"))
        .and_then(Value::as_str);
    let allowed = if require_idle {
        status == Some("idle")
    } else {
        matches!(status, Some("idle" | "notLoaded"))
    };
    if !allowed {
        return Err("Codex fork child did not report an allowed quiescent status".to_string());
    }
    Ok(())
}

fn validate_app_server_source(source: Option<&Value>) -> Result<(), String> {
    if source == Some(&Value::String("appServer".to_string())) {
        Ok(())
    } else {
        Err("Codex fork child did not inherit the SchoolX appServer source".to_string())
    }
}

fn require_active_source(
    store: &CodeThreadBindingStore,
    input: &CodeThreadForkInput,
) -> Result<CodeThreadBinding, String> {
    store.require_active_binding(&CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    })
}

fn require_clean_managed_source(
    binding: &CodeThreadBinding,
    nest_root: &Path,
    expected_head: Option<&str>,
) -> Result<CodeWorktreeStatus, String> {
    if binding.execution_mode != CodeExecutionMode::Worktree || binding.worktree_id.is_none() {
        return Err("SchoolX Code can only fork a managed source worktree".to_string());
    }
    let status = revalidate_execution_root(&binding_descriptor(binding), nest_root)?;
    if status.dirty {
        return Err("SchoolX Code source worktree has uncommitted changes".to_string());
    }
    if expected_head.is_some_and(|expected| status.head_commit != expected) {
        return Err("SchoolX Code source HEAD changed after fork preparation".to_string());
    }
    Ok(status)
}

fn require_clean_destination(
    preparation: &CodeThreadPreparation,
    nest_root: &Path,
    source_status: &CodeWorktreeStatus,
) -> Result<CodeWorktreeStatus, String> {
    let destination = revalidate_execution_root(&preparation.descriptor(), nest_root)?;
    if destination.dirty
        || destination.head_commit != preparation.base_ref
        || destination.branch.is_some()
    {
        return Err("SchoolX Code fork destination changed after preparation".to_string());
    }
    if destination.descriptor.execution_root == source_status.descriptor.execution_root
        || destination.descriptor.worktree_id == source_status.descriptor.worktree_id
    {
        return Err("SchoolX Code fork destination shares its source worktree".to_string());
    }
    Ok(destination)
}

fn validate_managed_source_identity(
    source: &CodeThreadBinding,
    preparation: &CodeThreadPreparation,
) -> Result<(), String> {
    if source.execution_mode != CodeExecutionMode::Worktree
        || source.repository_identity != preparation.repository_identity
        || source.execution_root == preparation.execution_root
        || source.worktree_id == preparation.worktree_id
    {
        return Err(
            "SchoolX Code fork source no longer matches its destination journal".to_string(),
        );
    }
    Ok(())
}

fn validate_reported_root(
    reported_root: &str,
    expected_root: &str,
    label: &str,
) -> Result<(), String> {
    validate_canonical_root(reported_root, expected_root, label)
}

fn validate_canonical_root(
    reported_root: &str,
    expected_root: &str,
    label: &str,
) -> Result<(), String> {
    let canonical = crate::code_workspace::canonical_workspace_root(reported_root)?;
    if canonical == expected_root {
        Ok(())
    } else {
        Err(format!("Codex {label} escaped the native execution root"))
    }
}

fn binding_descriptor(binding: &CodeThreadBinding) -> CodeWorktreeDescriptor {
    CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity.clone(),
        execution_root: binding.execution_root.clone(),
        base_ref: binding.base_ref.clone(),
        worktree_id: binding.worktree_id.clone(),
    }
}

fn rollback_unsent_fork(
    store: &CodeThreadBindingStore,
    claimed: &CodeThreadPreparation,
    message: String,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    match store.restore_preparation_after_unsent_fork(claimed) {
        Ok(_) => Err(fork_error(
            "threadForkNotSent",
            format!(
                "Codex thread fork was not sent; the exact destination was restored and can be continued: {message}"
            ),
            claimed,
            None,
        )),
        Err(rollback_error) => Err(fork_error(
            "forkRollbackFailed",
            format!(
                "Codex thread fork was not sent, but its preparation could not be restored: {message}; {rollback_error}"
            ),
            claimed,
            None,
        )),
    }
}

fn fork_error(
    code: &str,
    message: String,
    preparation: &CodeThreadPreparation,
    thread_id: Option<String>,
) -> CodeThreadStartError {
    CodeThreadStartError::recovery(
        code,
        message,
        preparation.preparation_id.clone(),
        thread_id,
        Some(preparation.execution_root.clone()),
    )
}

fn code_app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve SchoolX app-data directory: {error}"))
}

fn code_nest_root() -> Result<PathBuf, String> {
    crate::managed_agents::nest_dir()
        .ok_or_else(|| "failed to resolve the active SchoolX nest directory".to_string())
}

#[cfg(test)]
#[path = "code_thread_fork/tests.rs"]
mod tests;
fn lock_bindings(lock: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    lock.lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())
}
