use std::path::Path;
use std::sync::Mutex;

use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::code_workspace::{
    list_worktree_inventory, CodeThreadBindingStore, CodeWorktreeInventoryRow,
    CodeWorktreesListInput,
};

use super::code_workspace::{code_app_data_dir, code_nest_root};

#[tauri::command]
/// List only exact-scope roots owned by durable managed bindings or unfinished
/// managed preparations, without changing Git, filesystem, or binding state.
pub async fn code_worktrees_list(
    input: CodeWorktreesListInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<CodeWorktreeInventoryRow>, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let binding_lock = state.code_thread_bindings_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_worktrees_native(input, &app_data_dir, &nest_root, &binding_lock)
    })
    .await
    .map_err(|error| format!("SchoolX Code worktree inventory task failed: {error}"))?
}

fn list_worktrees_native(
    input: CodeWorktreesListInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
) -> Result<Vec<CodeWorktreeInventoryRow>, String> {
    let _guard = binding_lock
        .lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())?;
    let Some(store) = CodeThreadBindingStore::for_app_data_read_only(app_data_dir)? else {
        input.scope.validate()?;
        return Ok(Vec::new());
    };
    list_worktree_inventory(&store, nest_root, &input.scope)
}
