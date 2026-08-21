use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::code_workspace::{
    canonical_workspace_root, revalidate_execution_root, CodeThreadBinding,
    CodeThreadBindingLookupInput, CodeThreadBindingStore, CodeThreadLifecycleStatus,
    CodeThreadRenameInput, CodeThreadSummary, CodeWorktreeDescriptor,
};

#[tauri::command]
/// Rename only a thread owned by the exact persisted SchoolX binding scope.
pub async fn code_thread_rename(
    input: CodeThreadRenameInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeThreadSummary, String> {
    input.validate()?;
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        rename_thread_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("Codex thread rename task failed: {error}"))?
}

fn rename_thread_native(
    input: CodeThreadRenameInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeThreadSummary, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let binding = require_binding(&store, &input)?;
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        runtime,
        lifecycle_authority,
        &binding.codex_thread_id,
    )?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    let expected_thread_id = binding.codex_thread_id.clone();
    let expected_name = input.name.clone();
    let renamed = runtime.thread_rename_guarded(&input, lifecycle_checkpoint)?;
    validate_renamed_thread(
        &renamed,
        &expected_thread_id,
        &execution_root,
        &expected_name,
    )?;
    Ok(renamed)
}

fn require_binding(
    store: &CodeThreadBindingStore,
    input: &CodeThreadRenameInput,
) -> Result<CodeThreadBinding, String> {
    let lookup = CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    };
    let snapshot = store.lookup_with_lifecycle(&lookup)?.ok_or_else(|| {
        "Codex thread is not bound to the requested SchoolX community, project, and repository"
            .to_string()
    })?;
    store.ensure_no_pending_worktree_removal(&lookup)?;
    if !matches!(
        snapshot.status,
        CodeThreadLifecycleStatus::Active | CodeThreadLifecycleStatus::Archived
    ) {
        return Err(
            "SchoolX Code thread cannot be renamed while its lifecycle is unsettled".to_string(),
        );
    }
    Ok(snapshot.binding)
}

fn lock_bindings(lock: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    lock.lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())
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

fn validate_renamed_thread(
    thread: &CodeThreadSummary,
    expected_thread_id: &str,
    expected_root: &str,
    expected_name: &str,
) -> Result<(), String> {
    if thread.id != expected_thread_id {
        return Err("Codex returned a different thread than SchoolX renamed".to_string());
    }
    let reported_root = thread
        .cwd
        .as_deref()
        .ok_or_else(|| "Renamed Codex thread did not report its execution root".to_string())?;
    if canonical_workspace_root(reported_root)? != expected_root {
        return Err(
            "Renamed Codex thread reported a workspace outside the persisted execution root"
                .to_string(),
        );
    }
    if thread.name.as_deref() != Some(expected_name) {
        return Err("Codex did not persist the exact requested thread name".to_string());
    }
    Ok(())
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
mod tests {
    use super::*;

    fn summary(cwd: String, name: Option<&str>) -> CodeThreadSummary {
        CodeThreadSummary {
            id: "thread-1".to_string(),
            session_id: None,
            forked_from_id: None,
            parent_thread_id: None,
            preview: None,
            ephemeral: false,
            model_provider: None,
            created_at: None,
            updated_at: None,
            cwd: Some(cwd),
            name: name.map(str::to_string),
            status: None,
            turns: Vec::new(),
        }
    }

    #[test]
    fn rename_result_must_match_thread_root_and_exact_name() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let canonical = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let renamed = summary(canonical.clone(), Some("Focused rename"));
        validate_renamed_thread(&renamed, "thread-1", &canonical, "Focused rename")?;

        assert!(
            validate_renamed_thread(&renamed, "thread-other", &canonical, "Focused rename")
                .is_err()
        );
        assert!(validate_renamed_thread(&renamed, "thread-1", &canonical, "Stale name").is_err());
        Ok(())
    }

    #[test]
    fn rename_input_is_strictly_bounded_before_rpc() {
        let scope = crate::code_workspace::CodeThreadBindingScope {
            community_id: "community-1".to_string(),
            project_dtag: "project-1".to_string(),
            repository_identity: "a".repeat(64),
        };
        let input = |name: String| CodeThreadRenameInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            name,
        };

        assert!(input("Focused rename".to_string()).validate().is_ok());
        assert!(input(" rename".to_string()).validate().is_err());
        assert!(input("rename\nnext".to_string()).validate().is_err());
        assert!(input("x".repeat(129)).validate().is_err());
        assert!(input("😀".repeat(128)).validate().is_ok());
    }
}
