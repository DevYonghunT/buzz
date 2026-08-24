use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use super::project_git_diff::{
    current_changes_from_pinned_repo, CurrentRepoChangeStatus, CurrentRepoDiffInfo,
};
use super::project_git_exec::{build_git_auth_config, with_pinned_git_directory, GitAuthConfig};
use crate::app_state::AppState;
use crate::code_workspace::{
    code_thread_source, preflight_execution_root, revalidate_execution_root,
    with_execution_root_authority, CodeActiveTurnCheckpoint, CodeApprovalCheckpoint,
    CodeApprovalResponseInput, CodeBoundThreadOpenResult, CodeBoundThreadSummary, CodeEventBacklog,
    CodeEventCheckpoint, CodeEventEmitter, CodeModelSelection, CodeModelSelectionStore,
    CodeModelsListResult, CodePreparedWorktree, CodeRecoveryThread, CodeRepositoryDescriptor,
    CodeRepositoryInspectInput, CodeRuntime, CodeRuntimeEvent, CodeRuntimeEventBacklog,
    CodeRuntimeProbe, CodeRuntimeStatus, CodeThreadBinding, CodeThreadBindingLookupInput,
    CodeThreadBindingRecoverInput, CodeThreadBindingScope, CodeThreadBindingStore,
    CodeThreadChangeStatus, CodeThreadChangedFile, CodeThreadChanges, CodeThreadChangesInput,
    CodeThreadListInput, CodeThreadPreparation, CodeThreadPreparationListInput,
    CodeThreadPreparationOperation, CodeThreadResumeInput, CodeThreadStartError,
    CodeThreadStartInput, CodeThreadsPage, CodeTurnInterruptInput, CodeTurnStartInput,
    CodeTurnSteerInput, CodeTurnSummary, CodeWorkspaceEvent, CodeWorktreeDescriptor,
    CodeWorktreePrepareCommandInput, CodeWorktreeRemovalContext, CodeWorktreeRemovalReceipt,
    CodeWorktreeRemoveInput, CodeWorktreeStatus, CODE_WORKSPACE_EVENT,
};

const MAX_EVENT_SCOPE_CACHE: usize = 512;

#[derive(Default)]
struct EventScopeCache {
    by_thread_id: HashMap<String, CodeThreadBindingScope>,
    insertion_order: VecDeque<String>,
}

impl EventScopeCache {
    fn get(&self, thread_id: &str) -> Option<CodeThreadBindingScope> {
        self.by_thread_id.get(thread_id).cloned()
    }

    fn insert(&mut self, thread_id: String, scope: CodeThreadBindingScope) {
        if self.by_thread_id.contains_key(&thread_id) {
            return;
        }
        if self.by_thread_id.len() >= MAX_EVENT_SCOPE_CACHE {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.by_thread_id.remove(&oldest);
            }
        }
        self.insertion_order.push_back(thread_id.clone());
        self.by_thread_id.insert(thread_id, scope);
    }
}

fn scope_live_event(
    store: &CodeThreadBindingStore,
    cache: &Mutex<EventScopeCache>,
    event: CodeRuntimeEvent,
) -> Option<CodeWorkspaceEvent> {
    let thread_id = event.thread_id.clone()?;
    let cached_scope = cache.lock().ok().and_then(|cache| cache.get(&thread_id));
    let scope = match cached_scope {
        Some(scope) => scope,
        None => {
            // Only successful lookups are cached. A notification can race the
            // thread/start response and durable binding commit; retrying misses
            // lets the first post-commit event establish the positive cache.
            let scope = store.lookup_thread_id(&thread_id).ok()??.scope();
            if let Ok(mut cache) = cache.lock() {
                cache.insert(thread_id, scope.clone());
            }
            scope
        }
    };
    event.into_scoped(scope)
}

fn scope_event_backlog(
    backlog: CodeRuntimeEventBacklog,
    scope: CodeThreadBindingScope,
    bound_thread_ids: &HashSet<String>,
) -> CodeEventBacklog {
    let checkpoint = backlog.checkpoint.map(|checkpoint| {
        let active_turns = checkpoint
            .active_turns
            .into_iter()
            .filter(|turn| bound_thread_ids.contains(&turn.thread_id))
            .map(|turn| CodeActiveTurnCheckpoint {
                thread_id: turn.thread_id,
                turn_id: turn.turn_id,
                status: turn.status,
                started_sequence: turn.started_sequence,
            })
            .collect();
        let pending_approvals = checkpoint
            .pending_approvals
            .into_iter()
            .filter(|approval| {
                approval
                    .event
                    .thread_id
                    .as_ref()
                    .is_some_and(|thread_id| bound_thread_ids.contains(thread_id))
            })
            .filter_map(|approval| {
                Some(CodeApprovalCheckpoint {
                    event: approval.event.into_scoped(scope.clone())?,
                    respondable: approval.respondable,
                })
            })
            .collect();
        CodeEventCheckpoint {
            runtime_generation: checkpoint.runtime_generation,
            sequence_watermark: checkpoint.sequence_watermark,
            active_turns,
            pending_approvals,
        }
    });
    let events = backlog
        .events
        .into_iter()
        .filter(|event| {
            event
                .thread_id
                .as_ref()
                .is_some_and(|thread_id| bound_thread_ids.contains(thread_id))
        })
        .filter_map(|event| event.into_scoped(scope.clone()))
        .collect();
    CodeEventBacklog {
        runtime_generation: backlog.runtime_generation,
        latest_sequence: backlog.latest_sequence,
        truncated: backlog.truncated,
        checkpoint,
        events,
    }
}

#[tauri::command]
/// Discover the canonical Codex executable and installed version.
pub async fn code_runtime_probe(state: State<'_, AppState>) -> Result<CodeRuntimeProbe, String> {
    let runtime = state.code_runtime.clone();
    tauri::async_runtime::spawn_blocking(move || runtime.probe())
        .await
        .map_err(|error| format!("Codex probe task failed: {error}"))
}

#[tauri::command]
/// Start and initialize app-server, installing the normalized Tauri emitter.
pub async fn code_runtime_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeRuntimeStatus, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_code_runtime_after_removal_recovery(
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
            move |store| {
                let cache = Mutex::new(EventScopeCache::default());
                Arc::new(move |event| {
                    if let Some(event) = scope_live_event(&store, &cache, event) {
                        let _ = app.emit(CODE_WORKSPACE_EVENT, event);
                    }
                })
            },
        )
    })
    .await
    .map_err(|error| format!("Codex start task failed: {error}"))?
}

pub(crate) fn start_code_runtime_after_removal_recovery(
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &AtomicBool,
    emitter_factory: impl FnOnce(CodeThreadBindingStore) -> CodeEventEmitter,
) -> Result<CodeRuntimeStatus, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let emitter = emitter_factory(store.clone());
    if let Some(status) = runtime.replace_emitter_if_ready(Arc::clone(&emitter))? {
        return Ok(status);
    }

    crate::commands::code_thread_lifecycle::invalidate_lifecycle_authority(lifecycle_authority);
    crate::code_workspace::git_write::recover_startup_journals(&store, app_data_dir, nest_root)?;
    let status = runtime.start(emitter)?;
    match super::code_thread_lifecycle::reconcile_all_thread_lifecycles(
        &store,
        runtime,
        lifecycle_authority,
    ) {
        Ok(None) => Ok(status),
        Ok(Some(warning)) => {
            let stop_error = runtime.stop().err();
            super::code_thread_lifecycle::invalidate_lifecycle_authority(lifecycle_authority);
            Err(match stop_error {
                Some(stop_error) => {
                    format!("{warning}; Codex runtime cleanup also failed: {stop_error}")
                }
                None => warning,
            })
        }
        Err(error) => {
            let stop_error = runtime.stop().err();
            Err(match stop_error {
                Some(stop_error) => format!(
                    "SchoolX Code lifecycle reconciliation failed: {error}; Codex runtime cleanup also failed: {stop_error}"
                ),
                None => format!("SchoolX Code lifecycle reconciliation failed: {error}"),
            })
        }
    }
}

#[tauri::command]
/// Stop the whole app-server process group and invalidate pending approvals.
pub async fn code_runtime_stop(state: State<'_, AppState>) -> Result<CodeRuntimeStatus, String> {
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_bindings(&binding_lock)?;
        super::code_thread_lifecycle::invalidate_lifecycle_authority(&lifecycle_authority);
        runtime.stop()
    })
    .await
    .map_err(|error| format!("Codex stop task failed: {error}"))?
}

#[tauri::command]
/// Read the current runtime state without starting a process.
pub fn code_runtime_status(state: State<'_, AppState>) -> Result<CodeRuntimeStatus, String> {
    state.code_runtime.status()
}

#[tauri::command]
/// List the strict visible catalog and the last still-supported UX preference.
pub async fn code_models_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeModelsListResult, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let runtime = state.code_runtime.clone();
    let selection_lock = state.code_model_selection_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = runtime.model_catalog()?;
        let _guard = selection_lock
            .lock()
            .map_err(|_| "SchoolX Code model selection lock is unavailable".to_string())?;
        let recent_selection = match CodeModelSelectionStore::for_app_data_read_only(&app_data_dir)?
        {
            Some(store) => store.load()?,
            None => None,
        };
        Ok(CodeModelsListResult {
            runtime_generation: catalog.runtime_generation,
            recent_selection: catalog.reconcile_recent_selection(recent_selection),
            models: catalog.models,
        })
    })
    .await
    .map_err(|error| format!("Codex model list task failed: {error}"))?
}

#[tauri::command]
/// Validate and atomically persist the installation-global recent selection.
pub async fn code_model_selection_set(
    input: CodeModelSelection,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeModelSelection, String> {
    input.validate_shape()?;
    let app_data_dir = code_app_data_dir(&app)?;
    let runtime = state.code_runtime.clone();
    let selection_lock = state.code_model_selection_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = runtime.model_catalog()?;
        catalog.require_selection(&input)?;
        let _guard = selection_lock
            .lock()
            .map_err(|_| "SchoolX Code model selection lock is unavailable".to_string())?;
        CodeModelSelectionStore::for_app_data(&app_data_dir)?.save(&input)?;
        Ok(input)
    })
    .await
    .map_err(|error| format!("Codex model selection task failed: {error}"))?
}

#[tauri::command]
/// Replay only events owned by one exact persisted SchoolX thread scope.
pub async fn code_runtime_events(
    scope: CodeThreadBindingScope,
    runtime_generation: Option<u64>,
    after_sequence: Option<u64>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeEventBacklog, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let runtime = state.code_runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let backlog = runtime.events(runtime_generation, after_sequence)?;
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        let bound_thread_ids = store
            .list(&scope)?
            .into_iter()
            .map(|binding| binding.codex_thread_id)
            .collect();
        Ok(scope_event_backlog(backlog, scope, &bound_thread_ids))
    })
    .await
    .map_err(|error| format!("Codex event replay task failed: {error}"))?
}

#[tauri::command]
/// Inspect a repository and validate its base ref without changing local state.
pub async fn code_repository_inspect(
    input: CodeRepositoryInspectInput,
) -> Result<CodeRepositoryDescriptor, String> {
    tauri::async_runtime::spawn_blocking(move || {
        preflight_execution_root(&input.repository_root, &input.base_ref)
    })
    .await
    .map_err(|error| format!("SchoolX Code repository inspection task failed: {error}"))?
}

#[tauri::command]
/// Prepare either a detached managed worktree or the selected local checkout.
pub async fn code_worktree_prepare(
    input: CodeWorktreePrepareCommandInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodePreparedWorktree, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let binding_lock = state.code_thread_bindings_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        prepare_worktree_native(input, &app_data_dir, &nest_root, &binding_lock)
    })
    .await
    .map_err(|error| format!("SchoolX Code worktree preparation task failed: {error}"))?
}

#[tauri::command]
/// List unfinished native preparations for recovery in one exact scope.
pub async fn code_thread_preparations_list(
    input: CodeThreadPreparationListInput,
    app: AppHandle,
) -> Result<Vec<CodeThreadPreparation>, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        CodeThreadBindingStore::for_app_data(&app_data_dir)?.list_preparations(&input.scope)
    })
    .await
    .map_err(|error| format!("SchoolX Code preparation list task failed: {error}"))?
}

#[tauri::command]
/// Revalidate a prepared execution root without modifying or removing it.
pub async fn code_worktree_status(
    descriptor: CodeWorktreeDescriptor,
) -> Result<CodeWorktreeStatus, String> {
    let nest_root = code_nest_root()?;
    tauri::async_runtime::spawn_blocking(move || revalidate_execution_root(&descriptor, &nest_root))
        .await
        .map_err(|error| format!("SchoolX Code worktree status task failed: {error}"))?
}

#[tauri::command]
/// Safely remove one exact archived managed worktree using native-only proof.
pub async fn code_worktree_remove(
    input: CodeWorktreeRemoveInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeWorktreeRemovalReceipt, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let terminals = state.code_terminal_manager.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    let shutdown_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let shutdown_state = shutdown_app.state::<AppState>();
        remove_worktree_native(
            input,
            &app_data_dir,
            &nest_root,
            &binding_lock,
            CodeWorktreeRemovalContext {
                runtime: &runtime,
                terminals: &terminals,
                lifecycle_authority_ready: &lifecycle_authority,
                shutdown_started: &shutdown_state.shutdown_started,
            },
        )
    })
    .await
    .map_err(|error| format!("SchoolX Code worktree removal task failed: {error}"))?
}

#[tauri::command]
/// List only Codex threads durably bound to one exact SchoolX project scope.
pub async fn code_threads_list(
    input: CodeThreadListInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeThreadsPage, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_threads_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("Codex thread list task failed: {error}"))?
}

fn list_threads_native(
    input: CodeThreadListInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
    lifecycle_authority: &std::sync::atomic::AtomicBool,
) -> Result<CodeThreadsPage, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    input.scope.validate()?;
    let _reconciliation_warning = super::code_thread_lifecycle::reconcile_all_thread_lifecycles(
        &store,
        runtime,
        lifecycle_authority,
    )?;
    let bindings = store.list_with_lifecycle(&input.scope)?;
    let validated_bindings = if bindings.is_empty() {
        Vec::new()
    } else {
        with_execution_root_authority(|| {
            Ok(bindings
                .into_iter()
                .map(|snapshot| {
                    let execution_root = revalidate_binding_root(&snapshot.binding, nest_root);
                    (snapshot.binding, snapshot.status, execution_root)
                })
                .collect::<Vec<_>>())
        })?
    };
    let mut data = Vec::with_capacity(validated_bindings.len());
    for (binding, lifecycle, execution_root) in validated_bindings {
        let hydrated = (|| {
            let execution_root = execution_root?;
            let thread = if lifecycle == crate::code_workspace::CodeThreadLifecycleStatus::Active {
                runtime.thread_read(&binding.codex_thread_id)?
            } else {
                runtime.thread_read_with_turns(&binding.codex_thread_id)?
            };
            validate_thread_identity_and_root(
                &thread.id,
                thread.cwd.as_deref(),
                &binding.codex_thread_id,
                &execution_root,
            )?;
            Ok(thread)
        })();
        match hydrated {
            Ok(thread) => data.push(CodeBoundThreadSummary {
                binding,
                lifecycle,
                thread: Some(thread),
                unavailable: None,
            }),
            Err(error) => data.push(CodeBoundThreadSummary {
                binding,
                lifecycle,
                thread: None,
                unavailable: Some(error),
            }),
        }
    }
    Ok(CodeThreadsPage {
        data,
        next_cursor: None,
        backwards_cursor: None,
    })
}

#[tauri::command]
/// Read current changes only after resolving and revalidating one exact bound
/// thread's persisted execution root and immutable base commit.
pub async fn code_thread_changes(
    input: CodeThreadChangesInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeThreadChanges, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let auth = build_git_auth_config(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        thread_changes_native(input, &app_data_dir, &nest_root, &auth)
    })
    .await
    .map_err(|error| format!("SchoolX Code changes task failed: {error}"))?
}

fn thread_changes_native(
    input: CodeThreadChangesInput,
    app_data_dir: &Path,
    nest_root: &Path,
    auth: &GitAuthConfig,
) -> Result<CodeThreadChanges, String> {
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let binding = require_binding(&store, &input.scope, &input.thread_id)?;
    let diff = with_pinned_git_directory(Path::new(&binding.execution_root), |pinned| {
        let execution_root = revalidate_binding_root(&binding, nest_root)?;
        current_changes_from_pinned_repo(
            pinned,
            auth,
            &execution_root,
            &binding.repository_identity,
            &binding.base_ref,
        )
    })?;
    Ok(code_thread_changes_from_current_diff(diff))
}

fn code_thread_changes_from_current_diff(diff: CurrentRepoDiffInfo) -> CodeThreadChanges {
    CodeThreadChanges {
        files: diff
            .files
            .into_iter()
            .map(|file| CodeThreadChangedFile {
                path: file.path,
                status: match file.status {
                    CurrentRepoChangeStatus::Added => CodeThreadChangeStatus::Added,
                    CurrentRepoChangeStatus::Modified => CodeThreadChangeStatus::Modified,
                    CurrentRepoChangeStatus::Deleted => CodeThreadChangeStatus::Deleted,
                    CurrentRepoChangeStatus::TypeChanged => CodeThreadChangeStatus::TypeChanged,
                    CurrentRepoChangeStatus::Unmerged => CodeThreadChangeStatus::Unmerged,
                    CurrentRepoChangeStatus::Untracked => CodeThreadChangeStatus::Untracked,
                },
                binary: file.binary,
                additions: file.additions,
                deletions: file.deletions,
                patch: file.patch,
                truncated: file.truncated,
            })
            .collect(),
        total_files: diff.total_files,
        files_truncated: diff.files_truncated,
        additions: diff.additions,
        deletions: diff.deletions,
        commit_body: None,
    }
}

#[tauri::command]
/// Start a Codex thread and atomically bind its native execution root.
pub async fn code_thread_start(
    input: CodeThreadStartInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeBoundThreadOpenResult, CodeThreadStartError> {
    let preparation_id = input.preparation_id.clone();
    let app_data_dir = code_app_data_dir(&app)
        .map_err(|error| CodeThreadStartError::simple("appDataUnavailable", error))?;
    let nest_root =
        code_nest_root().map_err(|error| CodeThreadStartError::simple("nestUnavailable", error))?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_thread_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| {
        CodeThreadStartError::recovery(
            "threadStartTaskFailed",
            format!("Codex thread start task failed: {error}"),
            preparation_id,
            None,
            None,
        )
    })?
}

#[tauri::command]
/// Discover and reconcile the thread created by a durable `starting` preparation.
pub async fn code_thread_binding_recover(
    input: CodeThreadBindingRecoverInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeBoundThreadOpenResult, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let terminal_manager = state.code_terminal_manager.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        recover_thread_binding_native(
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
    .map_err(|error| format!("SchoolX Code binding recovery task failed: {error}"))?
}

#[tauri::command]
/// Resume a Codex thread only at its persisted, revalidated execution root.
pub async fn code_thread_resume(
    input: CodeThreadResumeInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeBoundThreadOpenResult, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        resume_thread_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("Codex thread resume task failed: {error}"))?
}

#[tauri::command]
/// Start one user turn at the thread's persisted execution root.
pub async fn code_turn_start(
    input: CodeTurnStartInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeTurnSummary, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_turn_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("Codex turn start task failed: {error}"))?
}

#[tauri::command]
/// Add input to an active turn after revalidating its persisted binding.
pub async fn code_turn_steer(
    input: CodeTurnSteerInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeTurnSummary, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        steer_turn_native(
            input,
            &app_data_dir,
            &nest_root,
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
    })
    .await
    .map_err(|error| format!("Codex turn steer task failed: {error}"))?
}

#[tauri::command]
/// Interrupt an exact bound turn even when its execution root is unavailable.
pub async fn code_turn_interrupt(
    input: CodeTurnInterruptInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_bindings(&binding_lock)?;
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        let binding = require_binding(&store, &input.scope, &input.thread_id)?;
        let lookup = CodeThreadBindingLookupInput {
            scope: input.scope.clone(),
            codex_thread_id: binding.codex_thread_id,
        };
        store.ensure_no_pending_worktree_removal(&lookup)?;
        runtime.turn_interrupt(input)
    })
    .await
    .map_err(|error| format!("Codex turn interrupt task failed: {error}"))?
}

#[tauri::command]
/// Respond once to an approval after validating its persisted thread scope.
pub async fn code_approval_respond(
    input: CodeApprovalResponseInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = input
        .approves_execution()
        .then(code_nest_root)
        .transpose()?;
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    let lifecycle_authority = state.code_lifecycle_authority_ready.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_bindings(&binding_lock)?;
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        let lookup = CodeThreadBindingLookupInput {
            scope: input.scope.clone(),
            codex_thread_id: input.thread_id.clone(),
        };
        let binding = store.require_active_binding(&lookup)?;
        let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
            &runtime,
            &lifecycle_authority,
            &binding.codex_thread_id,
        )?;
        if let Some(nest_root) = nest_root.as_deref() {
            revalidate_binding_root(&binding, nest_root)?;
        }
        runtime.approval_respond_guarded(input, lifecycle_checkpoint)
    })
    .await
    .map_err(|error| format!("Codex approval response task failed: {error}"))?
}

pub(crate) fn code_app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve SchoolX app-data directory: {error}"))
}

pub(crate) fn code_nest_root() -> Result<PathBuf, String> {
    crate::managed_agents::nest_dir()
        .ok_or_else(|| "failed to resolve the active SchoolX nest directory".to_string())
}

fn lock_bindings(lock: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    lock.lock()
        .map_err(|_| "SchoolX Code binding lock is unavailable".to_string())
}

fn require_binding(
    store: &CodeThreadBindingStore,
    scope: &CodeThreadBindingScope,
    thread_id: &str,
) -> Result<CodeThreadBinding, String> {
    let lookup = CodeThreadBindingLookupInput {
        scope: scope.clone(),
        codex_thread_id: thread_id.to_string(),
    };
    store.lookup(&lookup)?.ok_or_else(|| {
        "Codex thread is not bound to the requested SchoolX community, project, and repository"
            .to_string()
    })
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

fn validate_thread_identity_and_root(
    actual_thread_id: &str,
    reported_root: Option<&str>,
    expected_thread_id: &str,
    expected_root: &str,
) -> Result<(), String> {
    if actual_thread_id != expected_thread_id {
        return Err("Codex returned a different thread than SchoolX requested".to_string());
    }
    let reported_root = reported_root
        .ok_or_else(|| "Codex thread did not report its execution root".to_string())?;
    let canonical_reported = crate::code_workspace::canonical_workspace_root(reported_root)?;
    if canonical_reported != expected_root {
        return Err("Codex reported a workspace outside the persisted execution root".to_string());
    }
    Ok(())
}

fn select_recovery_candidate(
    preparation: &CodeThreadPreparation,
    candidates: Vec<CodeRecoveryThread>,
    reserved_thread_ids: &HashSet<String>,
    expected_root: &str,
) -> Result<CodeRecoveryThread, String> {
    let baseline = preparation
        .recovery_thread_baseline
        .as_ref()
        .ok_or_else(|| {
            "SchoolX Code preparation predates safe native recovery and cannot be auto-bound"
                .to_string()
        })?;
    let expected_thread_source = code_thread_source(&preparation.preparation_id)?;
    let mut eligible = Vec::new();
    for candidate in candidates {
        if reserved_thread_ids.contains(&candidate.thread.id) {
            continue;
        }
        validate_thread_identity_and_root(
            &candidate.thread.id,
            candidate.thread.cwd.as_deref(),
            &candidate.thread.id,
            expected_root,
        )?;
        if baseline
            .binary_search_by(|thread_id| thread_id.as_str().cmp(candidate.thread.id.as_str()))
            .is_ok()
            || candidate.thread_source.as_deref() != Some(expected_thread_source.as_str())
        {
            continue;
        }
        eligible.push(candidate);
    }

    match eligible.len() {
        1 => Ok(eligible.remove(0)),
        0 => Err(
            "SchoolX Code could not find an unbound thread created after this preparation was claimed; Codex 0.145 may not persist an empty thread across a runtime restart, so the preparation remains reserved"
                .to_string(),
        ),
        count => Err(format!(
            "SchoolX Code found {count} possible recovery threads and refused an ambiguous binding"
        )),
    }
}

fn validate_recovery_source(
    preparation: &CodeThreadPreparation,
    candidate: &CodeRecoveryThread,
) -> Result<(), String> {
    if preparation.recovery_thread_baseline.is_none() {
        return Err(
            "SchoolX Code preparation predates safe native recovery and cannot be auto-bound"
                .to_string(),
        );
    }
    let expected_thread_source = code_thread_source(&preparation.preparation_id)?;
    let valid = candidate.thread_source.as_deref() == Some(expected_thread_source.as_str());
    if valid {
        Ok(())
    } else {
        Err("Codex recovery thread source did not match the native preparation".to_string())
    }
}

mod native_threads;
mod native_worktrees;

use native_threads::{
    recover_thread_binding_native, resume_thread_native, start_thread_native, start_turn_native,
    steer_turn_native,
};
use native_worktrees::{prepare_worktree_native, remove_worktree_native};

#[cfg(test)]
pub(crate) use native_threads::{start_turn_for_test, steer_turn_for_test};
#[cfg(test)]
pub(crate) use native_worktrees::remove_worktree_for_test;

#[cfg(test)]
pub(crate) mod tests;
