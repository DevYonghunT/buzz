//! Thin Tauri facade for exact bound-thread terminal sessions.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::code_workspace::{
    revalidate_execution_root, CodeTerminalEvent, CodeTerminalOpenInput, CodeTerminalResizeInput,
    CodeTerminalSession, CodeTerminalStdinInput, CodeTerminalTerminateInput, CodeThreadBinding,
    CodeThreadBindingLookupInput, CodeThreadBindingStore, CodeWorktreeDescriptor,
};

#[tauri::command]
/// Open a native user shell only at an exact persisted thread binding root.
pub async fn code_terminal_open(
    input: CodeTerminalOpenInput,
    on_event: Channel<CodeTerminalEvent>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeTerminalSession, String> {
    if state.shutdown_started.load(Ordering::Acquire) {
        return Err("SchoolX Code terminal cannot open during app shutdown".to_string());
    }
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let manager = state.code_terminal_manager.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        open_terminal_native(
            input,
            on_event,
            &app_data_dir,
            &nest_root,
            TerminalOpenContext {
                runtime: &runtime,
                manager: &manager,
                binding_lock: &binding_lock,
                lifecycle_authority: &lifecycle_authority,
            },
        )
    })
    .await
    .map_err(|error| format!("SchoolX Code terminal open task failed: {error}"))?
}

#[tauri::command]
/// Resize one native terminal after exact owner validation.
pub async fn code_terminal_resize(
    input: CodeTerminalResizeInput,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.code_terminal_manager.clone();
    tauri::async_runtime::spawn_blocking(move || manager.resize(input))
        .await
        .map_err(|error| format!("SchoolX Code terminal resize task failed: {error}"))?
}

#[tauri::command]
/// Write raw stdin bytes after exact terminal owner validation.
pub async fn code_terminal_stdin(
    input: CodeTerminalStdinInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let runtime = state.code_runtime.clone();
    let manager = state.code_terminal_manager.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        stdin_terminal_native(
            input,
            &app_data_dir,
            &runtime,
            &manager,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("SchoolX Code terminal stdin task failed: {error}"))?
}

#[tauri::command]
/// Terminate and reap one exact native terminal owner.
pub async fn code_terminal_terminate(
    input: CodeTerminalTerminateInput,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.code_terminal_manager.clone();
    tauri::async_runtime::spawn_blocking(move || manager.terminate(input))
        .await
        .map_err(|error| format!("SchoolX Code terminal terminate task failed: {error}"))?
}

struct TerminalOpenContext<'a> {
    runtime: &'a crate::code_workspace::CodeRuntime,
    manager: &'a crate::code_workspace::CodeTerminalManager,
    binding_lock: &'a Mutex<()>,
    lifecycle_authority: &'a std::sync::atomic::AtomicBool,
}

fn open_terminal_native(
    input: CodeTerminalOpenInput,
    on_event: Channel<CodeTerminalEvent>,
    app_data_dir: &Path,
    nest_root: &Path,
    context: TerminalOpenContext<'_>,
) -> Result<CodeTerminalSession, String> {
    let _guard = lock_bindings(context.binding_lock)?;
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
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        context.runtime,
        context.lifecycle_authority,
        &binding.codex_thread_id,
    )?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    let thread_id = binding.codex_thread_id;
    context
        .runtime
        .with_thread_lifecycle_admission(&thread_id, lifecycle_checkpoint, || {
            context.manager.open(input, &execution_root, on_event)
        })
}

#[cfg(test)]
pub(crate) fn open_terminal_for_test(
    input: CodeTerminalOpenInput,
    on_event: Channel<CodeTerminalEvent>,
    app_data_dir: &Path,
    nest_root: &Path,
    context: (
        &crate::code_workspace::CodeRuntime,
        &crate::code_workspace::CodeTerminalManager,
        &Mutex<()>,
        &std::sync::atomic::AtomicBool,
    ),
) -> Result<CodeTerminalSession, String> {
    let (runtime, manager, binding_lock, lifecycle_authority) = context;
    open_terminal_native(
        input,
        on_event,
        app_data_dir,
        nest_root,
        TerminalOpenContext {
            runtime,
            manager,
            binding_lock,
            lifecycle_authority,
        },
    )
}

fn stdin_terminal_native(
    input: CodeTerminalStdinInput,
    app_data_dir: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    manager: &crate::code_workspace::CodeTerminalManager,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let lookup = CodeThreadBindingLookupInput {
        scope: input.scope.clone(),
        codex_thread_id: input.thread_id.clone(),
    };
    let binding = store.require_active_binding(&lookup)?;
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        runtime,
        lifecycle_authority,
        &binding.codex_thread_id,
    )?;
    let thread_id = binding.codex_thread_id;
    runtime
        .with_thread_lifecycle_admission(&thread_id, lifecycle_checkpoint, || manager.stdin(input))
}

fn lock_bindings(lock: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    lock.lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())
}

fn revalidate_binding_root(
    binding: &CodeThreadBinding,
    nest_root: &Path,
) -> Result<PathBuf, String> {
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity.clone(),
        execution_root: binding.execution_root.clone(),
        base_ref: binding.base_ref.clone(),
        worktree_id: binding.worktree_id.clone(),
    };
    Ok(PathBuf::from(
        revalidate_execution_root(&descriptor, nest_root)?
            .descriptor
            .execution_root,
    ))
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
