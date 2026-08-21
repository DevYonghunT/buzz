use std::path::Path;

use tauri::{AppHandle, State};

use super::code_workspace::{code_app_data_dir, code_nest_root};
use super::project_git_diff::{
    current_changes_from_pinned_repo, CurrentRepoChangeStatus, CurrentRepoDiffInfo,
};
use super::project_git_exec::{build_git_auth_config, with_pinned_git_directory};
use crate::app_state::AppState;
use crate::code_workspace::{
    self, revalidate_execution_root, CodeExecutionMode, CodeGitAcknowledgeInput,
    CodeGitAcknowledgeReceipt, CodeGitChangeFile, CodeGitChangeSet, CodeGitChangeStatus,
    CodeGitCommitInput, CodeGitCommitReceipt, CodeGitIndexMutationInput,
    CodeGitIndexMutationReceipt, CodeGitReconcileInput, CodeGitReconcileResult, CodeGitStatus,
    CodeGitStatusInput, CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingStore,
    CodeThreadLifecycleStatus, CodeWorktreeDescriptor,
};

#[tauri::command]
/// Read one authoritative task/staged/unstaged snapshot for an exact bound thread.
pub async fn code_thread_git_status(
    input: CodeGitStatusInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeGitStatus, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let _binding_guard = state
        .code_thread_bindings_lock
        .lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())?;
    let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
    let lookup = lookup(&input);
    let runtime_generation = state.code_runtime.status()?.generation;
    let snapshot = store
        .lookup_with_lifecycle(&lookup)?
        .ok_or_else(|| "Codex thread is not bound to the requested SchoolX scope".to_string())?;
    if snapshot.status != CodeThreadLifecycleStatus::Active {
        return code_workspace::git_write::blocked_status(
            &state.code_git_write,
            input,
            runtime_generation,
            "Git writes require a stable active thread lifecycle".to_string(),
        );
    }
    if snapshot.binding.execution_mode != CodeExecutionMode::Worktree {
        return code_workspace::git_write::blocked_status(
            &state.code_git_write,
            input,
            runtime_generation,
            "Git writes are read-only for local checkout bindings".to_string(),
        );
    }
    if let Some(status) = code_workspace::git_write::recovery_required_status(
        &state.code_git_write,
        &app_data_dir,
        &input,
        runtime_generation,
    )? {
        return Ok(status);
    }
    revalidate(&snapshot.binding)?;
    let task = task_changes(&snapshot.binding, &state)?;
    let activity_blocker = state
        .code_runtime
        .ensure_thread_idle(&input.thread_id)
        .err()
        .or_else(|| {
            state
                .code_terminal_manager
                .ensure_owner_absent(&input.scope, &input.thread_id)
                .err()
        });
    code_workspace::git_write::status(
        &state.code_git_write,
        input,
        code_workspace::git_write::GitWriteContext {
            app_data_dir,
            binding: snapshot.binding,
            runtime_generation,
            task,
            activity_blocker,
        },
    )
}

#[tauri::command]
/// Stage one whole file selected by a native-issued opaque snapshot coordinate.
pub async fn code_thread_git_stage(
    input: CodeGitIndexMutationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeGitIndexMutationReceipt, String> {
    let scope = input.scope.clone();
    let thread_id = input.thread_id.clone();
    with_mutation_clearance(&app, &state, &scope, &thread_id, |app_data, binding| {
        code_workspace::git_write::stage(&state.code_git_write, app_data, binding, input)
    })
}

#[tauri::command]
/// Unstage one whole file selected by a native-issued opaque snapshot coordinate.
pub async fn code_thread_git_unstage(
    input: CodeGitIndexMutationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeGitIndexMutationReceipt, String> {
    let scope = input.scope.clone();
    let thread_id = input.thread_id.clone();
    with_mutation_clearance(&app, &state, &scope, &thread_id, |app_data, binding| {
        code_workspace::git_write::unstage(&state.code_git_write, app_data, binding, input)
    })
}

#[tauri::command]
/// Commit exactly the staged tree through a detached-HEAD compare-and-swap.
pub async fn code_thread_git_commit(
    input: CodeGitCommitInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeGitCommitReceipt, String> {
    let scope = input.scope.clone();
    let thread_id = input.thread_id.clone();
    with_mutation_clearance(&app, &state, &scope, &thread_id, |app_data, binding| {
        code_workspace::git_write::commit(&state.code_git_write, app_data, binding, input)
    })
}

#[tauri::command]
/// Read the durable response-loss state for one exact bound thread.
pub async fn code_thread_git_reconcile(
    input: CodeGitReconcileInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeGitReconcileResult, String> {
    let scope = input.scope.clone();
    let thread_id = input.thread_id.clone();
    with_recovery_clearance(&app, &state, &scope, &thread_id, |app_data, binding| {
        code_workspace::git_write::reconcile(&state.code_git_write, app_data, binding, input)
    })
}

#[tauri::command]
/// Acknowledge a completed mutation only after its authoritative post-state snapshot.
pub async fn code_thread_git_acknowledge(
    input: CodeGitAcknowledgeInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeGitAcknowledgeReceipt, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let _binding_guard = state
        .code_thread_bindings_lock
        .lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())?;
    let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    };
    let _binding = store.require_active_binding(&lookup)?;
    code_workspace::git_write::acknowledge(&state.code_git_write, &app_data_dir, input)
}

fn with_recovery_clearance<T>(
    app: &AppHandle,
    state: &State<'_, AppState>,
    scope: &crate::code_workspace::CodeThreadBindingScope,
    thread_id: &str,
    recovery: impl FnOnce(&Path, &CodeThreadBinding) -> Result<T, String>,
) -> Result<T, String> {
    let app_data_dir = code_app_data_dir(app)?;
    let _binding_guard = state
        .code_thread_bindings_lock
        .lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())?;
    let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: scope.clone(),
        codex_thread_id: thread_id.to_string(),
    };
    let binding = store.require_active_binding(&lookup)?;
    store.ensure_no_pending_worktree_removal(&lookup)?;
    store.ensure_fork_source_available(scope, thread_id)?;
    if binding.execution_mode != CodeExecutionMode::Worktree {
        return Err("Git recovery requires an active managed worktree".to_string());
    }
    revalidate(&binding)?;
    state.code_runtime.ensure_thread_idle(thread_id)?;
    state
        .code_terminal_manager
        .ensure_owner_absent(scope, thread_id)?;
    let _runtime_guard = state.code_runtime.lock_thread_idle_admission(thread_id)?;
    state
        .code_terminal_manager
        .ensure_owner_absent(scope, thread_id)?;
    recovery(&app_data_dir, &binding)
}

fn with_mutation_clearance<T>(
    app: &AppHandle,
    state: &State<'_, AppState>,
    scope: &crate::code_workspace::CodeThreadBindingScope,
    thread_id: &str,
    mutation: impl FnOnce(&Path, &CodeThreadBinding) -> Result<T, String>,
) -> Result<T, String> {
    let app_data_dir = code_app_data_dir(app)?;
    let nest_root = code_nest_root()?;
    with_mutation_clearance_native(
        &app_data_dir,
        &nest_root,
        scope,
        thread_id,
        GitMutationClearanceContext {
            runtime: &state.code_runtime,
            terminal_manager: &state.code_terminal_manager,
            binding_lock: &state.code_thread_bindings_lock,
        },
        mutation,
    )
}

struct GitMutationClearanceContext<'a> {
    runtime: &'a crate::code_workspace::CodeRuntime,
    terminal_manager: &'a crate::code_workspace::CodeTerminalManager,
    binding_lock: &'a std::sync::Mutex<()>,
}

fn with_mutation_clearance_native<T>(
    app_data_dir: &Path,
    nest_root: &Path,
    scope: &crate::code_workspace::CodeThreadBindingScope,
    thread_id: &str,
    context: GitMutationClearanceContext<'_>,
    mutation: impl FnOnce(&Path, &CodeThreadBinding) -> Result<T, String>,
) -> Result<T, String> {
    let _binding_guard = context
        .binding_lock
        .lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: scope.clone(),
        codex_thread_id: thread_id.to_string(),
    };
    let binding = store.require_active_binding(&lookup)?;
    store.ensure_no_pending_worktree_removal(&lookup)?;
    store.ensure_fork_source_available(scope, thread_id)?;
    if binding.execution_mode != CodeExecutionMode::Worktree {
        return Err("Git writes require an active managed worktree".to_string());
    }
    revalidate_at(&binding, nest_root)?;
    context.runtime.ensure_thread_idle(thread_id)?;
    context
        .terminal_manager
        .ensure_owner_absent(scope, thread_id)?;
    let _runtime_guard = context.runtime.lock_thread_idle_admission(thread_id)?;
    context
        .terminal_manager
        .ensure_owner_absent(scope, thread_id)?;
    mutation(app_data_dir, &binding)
}

#[cfg(test)]
pub(crate) fn with_mutation_clearance_for_test<T>(
    app_data_dir: &Path,
    nest_root: &Path,
    activity: (
        &crate::code_workspace::CodeRuntime,
        &crate::code_workspace::CodeTerminalManager,
        &std::sync::Mutex<()>,
    ),
    scope: &crate::code_workspace::CodeThreadBindingScope,
    thread_id: &str,
    mutation: impl FnOnce(&Path, &CodeThreadBinding) -> Result<T, String>,
) -> Result<T, String> {
    with_mutation_clearance_native(
        app_data_dir,
        nest_root,
        scope,
        thread_id,
        GitMutationClearanceContext {
            runtime: activity.0,
            terminal_manager: activity.1,
            binding_lock: activity.2,
        },
        mutation,
    )
}

fn lookup(input: &CodeGitStatusInput) -> CodeThreadBindingLookupInput {
    CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    }
}

fn revalidate(binding: &CodeThreadBinding) -> Result<(), String> {
    revalidate_at(binding, &code_nest_root()?)
}

fn revalidate_at(binding: &CodeThreadBinding, nest_root: &Path) -> Result<(), String> {
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity.clone(),
        execution_root: binding.execution_root.clone(),
        base_ref: binding.base_ref.clone(),
        worktree_id: binding.worktree_id.clone(),
    };
    revalidate_execution_root(&descriptor, nest_root)?;
    Ok(())
}

fn task_changes(
    binding: &CodeThreadBinding,
    state: &State<'_, AppState>,
) -> Result<CodeGitChangeSet, String> {
    let auth = build_git_auth_config(state)?;
    let diff = with_pinned_git_directory(Path::new(&binding.execution_root), |pinned| {
        current_changes_from_pinned_repo(
            pinned,
            &auth,
            &binding.execution_root,
            &binding.repository_identity,
            &binding.base_ref,
        )
    })?;
    Ok(convert_task(diff))
}

fn convert_task(diff: CurrentRepoDiffInfo) -> CodeGitChangeSet {
    let files = diff
        .files
        .into_iter()
        .map(|file| CodeGitChangeFile {
            file_id: String::new(),
            path: file.path,
            status: match file.status {
                CurrentRepoChangeStatus::Added => CodeGitChangeStatus::Added,
                CurrentRepoChangeStatus::Modified => CodeGitChangeStatus::Modified,
                CurrentRepoChangeStatus::Deleted => CodeGitChangeStatus::Deleted,
                CurrentRepoChangeStatus::TypeChanged => CodeGitChangeStatus::TypeChanged,
                CurrentRepoChangeStatus::Unmerged => CodeGitChangeStatus::Unmerged,
                CurrentRepoChangeStatus::Untracked => CodeGitChangeStatus::Untracked,
            },
            binary: file.binary,
            additions: file.additions,
            deletions: file.deletions,
            patch: file.patch,
            truncated: file.truncated,
        })
        .collect();
    CodeGitChangeSet {
        files,
        total_files: diff.total_files,
        files_truncated: diff.files_truncated,
        additions: diff.additions,
        deletions: diff.deletions,
    }
}

#[cfg(test)]
mod gate_tests;
