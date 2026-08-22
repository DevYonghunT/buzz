//! Thin Tauri facade for exact bound-thread archive lifecycle mutations.

use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::code_workspace::{
    canonical_workspace_root, revalidate_execution_root, CodeAuthoritativeThreadGraph,
    CodePendingForkExpectation, CodeRpcDeliveryError, CodeThreadBinding,
    CodeThreadBindingLifecycle, CodeThreadBindingLookupInput, CodeThreadBindingStore,
    CodeThreadLifecycleClaim, CodeThreadLifecycleDirtyCheckpoint, CodeThreadLifecycleGraphProof,
    CodeThreadLifecycleInput, CodeThreadLifecycleMutationResult, CodeThreadLifecycleStatus,
    CodeThreadMembership, CodeThreadPreparation, CodeThreadPreparationOperation, CodeThreadSummary,
    CodeWorktreeDescriptor,
};

/// Result of a full-store lifecycle reconciliation. A warning means graph
/// evidence failed, but every stable binding was durably moved to `unknown`.
pub(crate) type CodeLifecycleReconciliation = Result<Option<String>, String>;

#[tauri::command]
/// Archive one exact stable-active leaf thread without deleting its binding,
/// worktree reservation, or local changes.
pub async fn code_thread_archive(
    input: CodeThreadLifecycleInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let terminal_manager = state.code_terminal_manager.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        archive_thread_native(
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
    .map_err(|error| format!("Codex thread archive task failed: {error}"))?
}

#[tauri::command]
/// Restore one exact stable-archived thread at its persisted execution root.
pub async fn code_thread_unarchive(
    input: CodeThreadLifecycleInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        unarchive_thread_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("Codex thread unarchive task failed: {error}"))?
}

fn archive_thread_native(
    input: CodeThreadLifecycleInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    input.validate()?;
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = lifecycle_lookup(&input);
    let initial = require_exact_lifecycle(
        &store,
        &lookup,
        CodeThreadLifecycleStatus::Active,
        "archived",
    )?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    require_lifecycle_authority(lifecycle_authority)?;
    let execution_root = revalidate_binding_root(&initial.binding, nest_root)?;
    let activity = runtime.ensure_thread_idle(&input.thread_id)?;
    validate_reported_root(&activity.cwd, &execution_root, "archive idle proof")?;
    terminal_manager.terminate_owner(&input.scope, &input.thread_id)?;

    let dirty_checkpoint =
        lifecycle_dirty_checkpoint(runtime, &input.thread_id, lifecycle_authority)?;
    let proof =
        authoritative_graph_proof_or_mark_unknown(&store, &lookup, runtime, lifecycle_authority)?;
    let reconciled = reconcile_one(&store, &lookup, &initial.binding, proof.graph())
        .inspect_err(|_| invalidate_lifecycle_authority(lifecycle_authority))?;
    clear_lifecycle_dirty(
        runtime,
        &input.thread_id,
        dirty_checkpoint,
        lifecycle_authority,
    )?;
    if reconciled.status != CodeThreadLifecycleStatus::Active {
        return Err(
            "SchoolX Code archive was refused because authoritative membership did not match the active lifecycle"
                .to_string(),
        );
    }
    if proof.graph().ensure_leaf(&input.thread_id)? != CodeThreadMembership::Active {
        return Err("Only an authoritative active thread can be archived".to_string());
    }

    let claim = store.begin_archive(&lookup)?;
    if let Err(error) = revalidate_binding_root(&reconciled.binding, nest_root) {
        return rollback_unsent_error(
            &store,
            &claim,
            format!("SchoolX Code execution root changed before archive RPC: {error}"),
        );
    }
    let completion = match runtime.thread_archive_guarded(&input, proof) {
        Ok(completion) => completion,
        Err(error) => return handle_delivery_error(&store, &claim, "archive", error),
    };
    let completed = match runtime.complete_thread_archive_lifecycle(
        &input.thread_id,
        completion,
        || store.complete_lifecycle_transition(&claim),
    ) {
        Ok(completed) => completed,
        Err(error) => {
            invalidate_lifecycle_authority(lifecycle_authority);
            return mark_unknown_error(
                &store,
                &claim,
                format!(
                    "Codex thread archive succeeded, but its durable completion could not be proven: {error}"
                ),
            );
        }
    };
    if completed.status != CodeThreadLifecycleStatus::Archived {
        return Err("SchoolX Code archive did not commit an archived lifecycle".to_string());
    }
    Ok(lifecycle_result(completed, None))
}

#[cfg(test)]
pub(crate) fn archive_thread_for_test(
    input: CodeThreadLifecycleInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    terminal_manager: &crate::code_workspace::CodeTerminalManager,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    archive_thread_native(
        input,
        app_data_dir,
        nest_root,
        runtime,
        terminal_manager,
        binding_lock,
        lifecycle_authority,
    )
}

fn unarchive_thread_native(
    input: CodeThreadLifecycleInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    input.validate()?;
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = lifecycle_lookup(&input);
    let initial = require_exact_lifecycle(
        &store,
        &lookup,
        CodeThreadLifecycleStatus::Archived,
        "active",
    )?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    require_lifecycle_authority(lifecycle_authority)?;
    let execution_root = revalidate_binding_root(&initial.binding, nest_root)?;
    let dirty_checkpoint =
        lifecycle_dirty_checkpoint(runtime, &input.thread_id, lifecycle_authority)?;

    let proof =
        authoritative_graph_proof_or_mark_unknown(&store, &lookup, runtime, lifecycle_authority)?;
    let reconciled = reconcile_one(&store, &lookup, &initial.binding, proof.graph())
        .inspect_err(|_| invalidate_lifecycle_authority(lifecycle_authority))?;
    clear_lifecycle_dirty(
        runtime,
        &input.thread_id,
        dirty_checkpoint,
        lifecycle_authority,
    )?;
    if reconciled.status != CodeThreadLifecycleStatus::Archived
        || proof.graph().membership(&input.thread_id) != Some(CodeThreadMembership::Archived)
    {
        return Err(
            "SchoolX Code unarchive was refused because authoritative membership did not match the archived lifecycle"
                .to_string(),
        );
    }

    let claim = store.begin_unarchive(&lookup)?;
    if let Err(error) = revalidate_binding_root(&reconciled.binding, nest_root) {
        return rollback_unsent_error(
            &store,
            &claim,
            format!("SchoolX Code execution root changed before unarchive RPC: {error}"),
        );
    }
    let guarded = match runtime.thread_unarchive_guarded(&input, proof) {
        Ok(guarded) => guarded,
        Err(error) => return handle_delivery_error(&store, &claim, "unarchive", error),
    };
    let thread = guarded.thread;
    if let Err(error) = validate_unarchived_thread(&thread, &input.thread_id, &execution_root) {
        return mark_unknown_error(
            &store,
            &claim,
            format!("Codex unarchive response could not be trusted: {error}"),
        );
    }
    let completed = match runtime.complete_thread_unarchive_lifecycle(
        &input.thread_id,
        guarded.completion,
        || store.complete_lifecycle_transition(&claim),
    ) {
        Ok(completed) => completed,
        Err(error) => {
            invalidate_lifecycle_authority(lifecycle_authority);
            return mark_unknown_error(
                &store,
                &claim,
                format!(
                    "Codex thread unarchive succeeded, but its durable completion could not be proven: {error}"
                ),
            );
        }
    };
    if completed.status != CodeThreadLifecycleStatus::Active {
        return Err("SchoolX Code unarchive did not commit an active lifecycle".to_string());
    }
    Ok(lifecycle_result(completed, Some(thread)))
}

/// Reconcile every persisted binding against one exhaustive runtime graph.
///
/// Graph failure is converted into durable `unknown` for every stable record
/// and returned as a warning. Persistence failure remains a hard error because
/// callers must not expose an unverified stable lifecycle.
pub(crate) fn reconcile_all_thread_lifecycles(
    store: &CodeThreadBindingStore,
    runtime: &crate::code_workspace::CodeRuntime,
    lifecycle_authority: &AtomicBool,
) -> CodeLifecycleReconciliation {
    invalidate_lifecycle_authority(lifecycle_authority);
    let index = store.load()?;
    let pending_forks = pending_fork_expectations(&index.preparations)?;
    let bindings = index.bindings;
    if bindings.is_empty() {
        lifecycle_authority.store(true, Ordering::Release);
        return Ok(None);
    }
    let dirty_checkpoints = lifecycle_dirty_checkpoints(runtime, &bindings)?;
    let deferred_active_thread_ids = deferred_active_thread_ids(store, &bindings)?;
    let graph = match runtime
        .authoritative_thread_graph(&deferred_active_thread_ids, &pending_forks)
    {
        Ok(graph) => graph,
        Err(error) => {
            mark_all_stable_unknown(store, &bindings)?;
            return Ok(Some(format!(
                "Codex lifecycle membership could not be proven; stable bindings were marked unknown: {error}"
            )));
        }
    };
    for binding in bindings {
        let lookup = CodeThreadBindingLookupInput {
            scope: binding.scope(),
            codex_thread_id: binding.codex_thread_id.clone(),
        };
        reconcile_one(store, &lookup, &binding, &graph)?;
    }
    for (thread_id, checkpoint) in dirty_checkpoints {
        runtime.clear_thread_lifecycle_dirty(&thread_id, checkpoint)?;
    }
    lifecycle_authority.store(true, Ordering::Release);
    Ok(None)
}

fn authoritative_graph_proof_or_mark_unknown(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    runtime: &crate::code_workspace::CodeRuntime,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeThreadLifecycleGraphProof, String> {
    let bindings = store
        .load()
        .inspect_err(|_| invalidate_lifecycle_authority(lifecycle_authority))?;
    let deferred_active_thread_ids = deferred_active_thread_ids(store, &bindings.bindings)
        .inspect_err(|_| invalidate_lifecycle_authority(lifecycle_authority))?;
    let pending_forks = pending_fork_expectations(&bindings.preparations)
        .inspect_err(|_| invalidate_lifecycle_authority(lifecycle_authority))?;
    match runtime.authoritative_thread_graph_for_lifecycle_admission(
        &deferred_active_thread_ids,
        &pending_forks,
        &lookup.codex_thread_id,
    ) {
        Ok(proof) => Ok(proof),
        Err(error) => {
            invalidate_lifecycle_authority(lifecycle_authority);
            match store.mark_stable_lifecycle_unknown(lookup) {
                Ok(_) => Err(format!(
                    "Codex authoritative thread graph failed; the binding was marked unknown: {error}"
                )),
                Err(persistence_error) => Err(format!(
                    "Codex authoritative thread graph failed and the binding could not be marked unknown: {error}; {persistence_error}"
                )),
            }
        }
    }
}

pub(crate) fn require_lifecycle_authority(lifecycle_authority: &AtomicBool) -> Result<(), String> {
    if lifecycle_authority.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err(
            "SchoolX Code lifecycle authority is unavailable until reconciliation succeeds"
                .to_string(),
        )
    }
}

pub(crate) fn clean_thread_lifecycle_checkpoint(
    runtime: &crate::code_workspace::CodeRuntime,
    lifecycle_authority: &AtomicBool,
    thread_id: &str,
) -> Result<CodeThreadLifecycleDirtyCheckpoint, String> {
    require_lifecycle_authority(lifecycle_authority)?;
    match runtime.thread_lifecycle_dirty_checkpoint(thread_id) {
        Ok(checkpoint) if !checkpoint.is_dirty() => Ok(checkpoint),
        Ok(_) => Err(
            "SchoolX Code thread lifecycle changed and must be reconciled before this operation"
                .to_string(),
        ),
        Err(error) => {
            invalidate_lifecycle_authority(lifecycle_authority);
            Err(format!(
                "SchoolX Code thread lifecycle authority could not be checked: {error}"
            ))
        }
    }
}

pub(crate) fn invalidate_lifecycle_authority(lifecycle_authority: &AtomicBool) {
    lifecycle_authority.store(false, Ordering::Release);
}

fn deferred_active_thread_ids(
    store: &CodeThreadBindingStore,
    bindings: &[CodeThreadBinding],
) -> Result<Vec<String>, String> {
    let mut thread_ids = Vec::new();
    for binding in bindings {
        let lookup = CodeThreadBindingLookupInput {
            scope: binding.scope(),
            codex_thread_id: binding.codex_thread_id.clone(),
        };
        if store.allows_deferred_active_membership_proof(&lookup)? {
            thread_ids.push(binding.codex_thread_id.clone());
        }
    }
    thread_ids.sort();
    thread_ids.dedup();
    Ok(thread_ids)
}

fn pending_fork_expectations(
    preparations: &[CodeThreadPreparation],
) -> Result<Vec<CodePendingForkExpectation>, String> {
    let mut expectations = Vec::new();
    for preparation in preparations {
        if preparation.operation != CodeThreadPreparationOperation::Fork
            || preparation.state
                != crate::code_workspace::bindings::CodeThreadPreparationState::Starting
        {
            continue;
        }
        expectations.push(CodePendingForkExpectation {
            preparation_id: preparation.preparation_id.clone(),
            source_thread_id: preparation.source_thread_id.clone().ok_or_else(|| {
                "SchoolX pending fork preparation is missing its source thread".to_string()
            })?,
            execution_root: preparation.execution_root.clone(),
            recovery_thread_baseline: preparation.recovery_thread_baseline.clone().ok_or_else(
                || "SchoolX pending fork preparation is missing its recovery baseline".to_string(),
            )?,
        });
    }
    Ok(expectations)
}

fn lifecycle_dirty_checkpoints(
    runtime: &crate::code_workspace::CodeRuntime,
    bindings: &[CodeThreadBinding],
) -> Result<Vec<(String, CodeThreadLifecycleDirtyCheckpoint)>, String> {
    let mut thread_ids = bindings
        .iter()
        .map(|binding| binding.codex_thread_id.clone())
        .collect::<Vec<_>>();
    thread_ids.sort();
    thread_ids.dedup();
    thread_ids
        .into_iter()
        .map(|thread_id| {
            runtime
                .thread_lifecycle_dirty_checkpoint(&thread_id)
                .map(|checkpoint| (thread_id, checkpoint))
        })
        .collect()
}

fn lifecycle_dirty_checkpoint(
    runtime: &crate::code_workspace::CodeRuntime,
    thread_id: &str,
    lifecycle_authority: &AtomicBool,
) -> Result<CodeThreadLifecycleDirtyCheckpoint, String> {
    runtime
        .thread_lifecycle_dirty_checkpoint(thread_id)
        .map_err(|error| {
            invalidate_lifecycle_authority(lifecycle_authority);
            format!("Codex lifecycle dirty checkpoint failed: {error}")
        })
}

fn clear_lifecycle_dirty(
    runtime: &crate::code_workspace::CodeRuntime,
    thread_id: &str,
    checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    lifecycle_authority: &AtomicBool,
) -> Result<(), String> {
    runtime
        .clear_thread_lifecycle_dirty(thread_id, checkpoint)
        .map_err(|error| {
            invalidate_lifecycle_authority(lifecycle_authority);
            format!("Codex lifecycle changed during durable reconciliation: {error}")
        })
}

fn reconcile_one(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    binding: &CodeThreadBinding,
    graph: &CodeAuthoritativeThreadGraph,
) -> Result<CodeThreadBindingLifecycle, String> {
    let root_matches = match graph.thread(&binding.codex_thread_id) {
        Some(thread) => match (
            canonical_workspace_root(&thread.cwd),
            canonical_workspace_root(&binding.execution_root),
        ) {
            (Ok(reported), Ok(expected)) => reported == expected,
            _ => false,
        },
        None => false,
    };
    let active = root_matches
        && graph.membership(&binding.codex_thread_id) == Some(CodeThreadMembership::Active);
    let archived = root_matches
        && graph.membership(&binding.codex_thread_id) == Some(CodeThreadMembership::Archived);
    store.reconcile_lifecycle_membership(lookup, active, archived)
}

fn mark_all_stable_unknown(
    store: &CodeThreadBindingStore,
    bindings: &[CodeThreadBinding],
) -> Result<(), String> {
    for binding in bindings {
        let lookup = CodeThreadBindingLookupInput {
            scope: binding.scope(),
            codex_thread_id: binding.codex_thread_id.clone(),
        };
        let snapshot = store.lookup_with_lifecycle(&lookup)?.ok_or_else(|| {
            "SchoolX Code lifecycle reconciliation lost a persisted binding".to_string()
        })?;
        if matches!(
            snapshot.status,
            CodeThreadLifecycleStatus::Active | CodeThreadLifecycleStatus::Archived
        ) {
            store.mark_stable_lifecycle_unknown(&lookup)?;
        }
    }
    Ok(())
}

fn require_exact_lifecycle(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    required: CodeThreadLifecycleStatus,
    opposite_label: &str,
) -> Result<CodeThreadBindingLifecycle, String> {
    let snapshot = store.lookup_with_lifecycle(lookup)?.ok_or_else(|| {
        "Codex thread is not bound to the requested SchoolX community, project, and repository"
            .to_string()
    })?;
    if snapshot.status != required {
        return Err(format!(
            "SchoolX Code thread must be stably {required:?} before it can become {opposite_label}"
        ));
    }
    Ok(snapshot)
}

fn lifecycle_lookup(input: &CodeThreadLifecycleInput) -> CodeThreadBindingLookupInput {
    CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    }
}

fn lifecycle_result(
    snapshot: CodeThreadBindingLifecycle,
    thread: Option<CodeThreadSummary>,
) -> CodeThreadLifecycleMutationResult {
    CodeThreadLifecycleMutationResult {
        binding: snapshot.binding,
        lifecycle: snapshot.status,
        thread,
    }
}

fn handle_delivery_error(
    store: &CodeThreadBindingStore,
    claim: &CodeThreadLifecycleClaim,
    operation: &str,
    error: CodeRpcDeliveryError,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    let definitely_not_sent = error.definitely_not_sent();
    let message = error.into_message();
    if definitely_not_sent {
        rollback_unsent_error(
            store,
            claim,
            format!("Codex thread {operation} was not sent: {message}"),
        )
    } else {
        mark_unknown_error(
            store,
            claim,
            format!("Codex thread {operation} delivery is uncertain: {message}"),
        )
    }
}

fn rollback_unsent_error(
    store: &CodeThreadBindingStore,
    claim: &CodeThreadLifecycleClaim,
    message: String,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    match store.rollback_lifecycle_after_unsent(claim) {
        Ok(_) => Err(message),
        Err(rollback_error) => Err(format!(
            "{message}; the exact lifecycle rollback failed: {rollback_error}"
        )),
    }
}

fn mark_unknown_error(
    store: &CodeThreadBindingStore,
    claim: &CodeThreadLifecycleClaim,
    message: String,
) -> Result<CodeThreadLifecycleMutationResult, String> {
    match store.mark_lifecycle_unknown(claim) {
        Ok(_) => Err(message),
        Err(persistence_error) => Err(format!(
            "{message}; the uncertain lifecycle could not be persisted: {persistence_error}"
        )),
    }
}

fn validate_unarchived_thread(
    thread: &CodeThreadSummary,
    expected_thread_id: &str,
    expected_root: &str,
) -> Result<(), String> {
    if thread.id != expected_thread_id {
        return Err("Codex returned a different thread while unarchiving".to_string());
    }
    let reported_root = thread
        .cwd
        .as_deref()
        .ok_or_else(|| "Unarchived Codex thread did not report its execution root".to_string())?;
    validate_reported_root(reported_root, expected_root, "unarchive response")
}

fn validate_reported_root(
    reported_root: &str,
    expected_root: &str,
    boundary: &str,
) -> Result<(), String> {
    if canonical_workspace_root(reported_root)? != expected_root {
        return Err(format!(
            "Codex {boundary} reported a workspace outside the persisted execution root"
        ));
    }
    Ok(())
}

fn revalidate_binding_root(
    binding: &CodeThreadBinding,
    nest_root: &Path,
) -> Result<String, String> {
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity.clone(),
        execution_root: binding.execution_root.clone(),
        base_ref: binding.base_ref.clone(),
        worktree_id: binding.worktree_id.clone(),
    };
    Ok(revalidate_execution_root(&descriptor, nest_root)?
        .descriptor
        .execution_root)
}

fn lock_bindings(lock: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    lock.lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())
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
mod tests;
