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

fn prepare_worktree_native(
    input: CodeWorktreePrepareCommandInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
) -> Result<CodePreparedWorktree, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    store.ensure_preparation_capacity()?;
    input.scope.validate()?;
    let scope = input.scope.clone();
    let repository = preflight_execution_root(&input.repository_root, &input.base_ref)?;
    if scope.repository_identity != repository.repository_identity {
        return Err(
            "SchoolX Code preparation scope does not match the selected repository".to_string(),
        );
    }
    let prepared = crate::code_workspace::prepare_execution_root_with_merge_target(
        input.into_native(),
        nest_root,
    )?;
    let worktree = prepared.worktree;
    let preparation_id = uuid::Uuid::new_v4().hyphenated().to_string();
    store
        .create_preparation_with_merge_target(
            preparation_id.clone(),
            scope.clone(),
            &worktree.descriptor,
            prepared.merge_target_ref,
        )
        .map_err(|error| {
            format!(
                "SchoolX Code prepared execution root {} but could not journal it: {error}. The execution root was preserved",
                worktree.descriptor.execution_root
            )
        })?;
    Ok(CodePreparedWorktree {
        preparation_id,
        scope,
        worktree,
    })
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

fn remove_worktree_native(
    input: CodeWorktreeRemoveInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
    context: CodeWorktreeRemovalContext<'_>,
) -> Result<CodeWorktreeRemovalReceipt, String> {
    input.validate()?;
    let binding_guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    crate::code_workspace::remove_archived_worktree(
        &store,
        binding_guard,
        input,
        nest_root,
        context,
    )
}

#[cfg(test)]
pub(crate) fn remove_worktree_for_test(
    input: CodeWorktreeRemoveInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
    context: (
        &crate::code_workspace::CodeRuntime,
        &crate::code_workspace::CodeTerminalManager,
        &std::sync::atomic::AtomicBool,
        &std::sync::atomic::AtomicBool,
    ),
) -> Result<CodeWorktreeRemovalReceipt, String> {
    remove_worktree_native(
        input,
        app_data_dir,
        nest_root,
        binding_lock,
        CodeWorktreeRemovalContext {
            runtime: context.0,
            terminals: context.1,
            lifecycle_authority_ready: context.2,
            shutdown_started: context.3,
        },
    )
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

fn start_thread_native(
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
    super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)
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
        super::code_thread_lifecycle::invalidate_lifecycle_authority(lifecycle_authority);
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

fn recover_thread_binding_native(
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
    super::code_thread_lifecycle::require_lifecycle_authority(lifecycle_authority)?;
    if preparation.operation == CodeThreadPreparationOperation::Fork {
        return super::code_thread_fork::open_fork_preparation_locked(
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

fn resume_thread_native(
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
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
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

fn start_turn_native(
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
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
        runtime,
        lifecycle_authority,
        &binding.codex_thread_id,
    )?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    runtime.turn_start_at_guarded(input, &execution_root, lifecycle_checkpoint)
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

fn steer_turn_native(
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
    let lifecycle_checkpoint = super::code_thread_lifecycle::clean_thread_lifecycle_checkpoint(
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::code_workspace::bindings::{CodeExecutionMode, CodeThreadPreparationState};
    use crate::code_workspace::{
        CodeRuntimeActiveTurnCheckpoint, CodeRuntimeApprovalCheckpoint, CodeRuntimeEventCheckpoint,
    };
    use serde_json::json;
    use std::fs;
    use std::process::Command;

    fn scope() -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: "community-a".to_string(),
            project_dtag: "project-a".to_string(),
            repository_identity: "a".repeat(64),
        }
    }

    fn runtime_event(thread_id: Option<&str>, sequence: u64) -> CodeRuntimeEvent {
        CodeRuntimeEvent {
            runtime_generation: 4,
            sequence,
            thread_id: thread_id.map(str::to_string),
            turn_id: Some("turn-a".to_string()),
            item_id: None,
            kind: "turn/started".to_string(),
            payload: json!({ "sequence": sequence }),
        }
    }

    fn local_binding(execution_root: &Path, thread_id: &str) -> CodeThreadBinding {
        let scope = scope();
        let execution_root = execution_root
            .canonicalize()
            .unwrap_or_else(|_| execution_root.to_path_buf());
        CodeThreadBinding {
            community_id: scope.community_id,
            project_dtag: scope.project_dtag,
            repository_identity: scope.repository_identity,
            codex_thread_id: thread_id.to_string(),
            execution_mode: CodeExecutionMode::Local,
            execution_root: execution_root.to_string_lossy().into_owned(),
            base_ref: "b".repeat(40),
            worktree_id: None,
        }
    }

    #[test]
    fn current_diff_mapping_preserves_status_binary_and_completeness_contract() {
        let statuses = [
            CurrentRepoChangeStatus::Added,
            CurrentRepoChangeStatus::Modified,
            CurrentRepoChangeStatus::Deleted,
            CurrentRepoChangeStatus::TypeChanged,
            CurrentRepoChangeStatus::Unmerged,
            CurrentRepoChangeStatus::Untracked,
        ];
        let diff = CurrentRepoDiffInfo {
            files: statuses
                .into_iter()
                .enumerate()
                .map(
                    |(index, status)| crate::commands::project_git_diff::CurrentRepoDiffFileInfo {
                        path: format!("file-{index}"),
                        status,
                        binary: index == 1,
                        additions: index,
                        deletions: index.saturating_add(1),
                        patch: format!("patch-{index}"),
                        truncated: index == 2,
                    },
                )
                .collect(),
            total_files: 9,
            files_truncated: true,
            additions: 15,
            deletions: 21,
        };

        let mapped = code_thread_changes_from_current_diff(diff);
        assert_eq!(mapped.total_files, 9);
        assert!(mapped.files_truncated);
        assert_eq!(mapped.additions, 15);
        assert_eq!(mapped.deletions, 21);
        assert!(mapped.commit_body.is_none());
        assert_eq!(
            mapped
                .files
                .iter()
                .map(|file| file.status)
                .collect::<Vec<_>>(),
            vec![
                CodeThreadChangeStatus::Added,
                CodeThreadChangeStatus::Modified,
                CodeThreadChangeStatus::Deleted,
                CodeThreadChangeStatus::TypeChanged,
                CodeThreadChangeStatus::Unmerged,
                CodeThreadChangeStatus::Untracked,
            ]
        );
        assert!(mapped.files[1].binary);
        assert!(mapped.files[2].truncated);
    }

    fn recovery_preparation(
        execution_root: &str,
        baseline: Option<Vec<&str>>,
    ) -> CodeThreadPreparation {
        let scope = scope();
        CodeThreadPreparation {
            preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
            community_id: scope.community_id,
            project_dtag: scope.project_dtag,
            repository_identity: scope.repository_identity,
            execution_mode: CodeExecutionMode::Local,
            execution_root: execution_root.to_string(),
            base_ref: "b".repeat(40),
            worktree_id: None,
            operation: crate::code_workspace::CodeThreadPreparationOperation::Start,
            source_thread_id: None,
            state: crate::code_workspace::bindings::CodeThreadPreparationState::Starting,
            recovery_thread_baseline: baseline
                .map(|thread_ids| thread_ids.into_iter().map(str::to_string).collect()),
            merge_target_ref: None,
        }
    }

    fn recovery_candidate(
        thread_id: &str,
        execution_root: Option<&str>,
        thread_source: Option<&str>,
    ) -> CodeRecoveryThread {
        CodeRecoveryThread {
            thread: crate::code_workspace::CodeThreadSummary {
                id: thread_id.to_string(),
                session_id: None,
                forked_from_id: None,
                parent_thread_id: None,
                preview: None,
                ephemeral: false,
                model_provider: None,
                created_at: None,
                updated_at: None,
                cwd: execution_root.map(str::to_string),
                name: None,
                status: None,
                turns: Vec::new(),
            },
            thread_source: thread_source.map(str::to_string),
            session_source: None,
            ephemeral_present: false,
        }
    }

    #[cfg(unix)]
    pub(crate) struct FakeCodex {
        _directory: tempfile::TempDir,
        pub(crate) executable: PathBuf,
    }

    #[cfg(unix)]
    impl FakeCodex {
        pub(crate) fn started_marker(&self) -> PathBuf {
            self.executable.with_file_name("codex.started")
        }

        pub(crate) fn created_marker(&self) -> PathBuf {
            self.executable.with_file_name("codex.created")
        }

        pub(crate) fn terminal_drained_marker(&self) -> PathBuf {
            self.executable.with_file_name("codex.terminal-drained")
        }

        pub(crate) fn mark_created(&self) -> Result<(), String> {
            fs::write(self.created_marker(), b"created").map_err(|error| error.to_string())
        }

        pub(crate) fn request_approval_on_read(&self) -> Result<(), String> {
            fs::write(
                self.executable.with_file_name("codex.request-approval"),
                b"pending",
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn block_turn_start(&self) -> Result<(), String> {
            fs::write(
                self.executable.with_file_name("codex.block-turn-start"),
                b"block",
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn turn_start_admitted_marker(&self) -> PathBuf {
            self.executable.with_file_name("codex.turn-start-admitted")
        }

        pub(crate) fn release_turn_start(&self) -> Result<(), String> {
            fs::write(
                self.executable.with_file_name("codex.release-turn-start"),
                b"release",
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn fail_turn_start(&self) -> Result<(), String> {
            fs::write(
                self.executable.with_file_name("codex.fail-turn-start"),
                b"fail",
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn spawn_descendant_after_terminal_drain(&self) -> Result<(), String> {
            fs::write(
                self.executable
                    .with_file_name("codex.spawn-descendant-after-terminal"),
                b"spawn",
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn fail_archive_response(&self) -> Result<(), String> {
            fs::write(
                self.executable.with_file_name("codex.fail-archive"),
                b"fail",
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn fail_archive_commit(&self, code_dir: &Path) -> Result<(), String> {
            fs::write(
                self.executable.with_file_name("codex.fail-archive-commit"),
                code_dir.to_string_lossy().as_bytes(),
            )
            .map_err(|error| error.to_string())
        }

        pub(crate) fn recorded_requests(&self) -> Result<Vec<serde_json::Value>, String> {
            let contents = fs::read_to_string(self.executable.with_file_name("codex.requests"))
                .map_err(|error| error.to_string())?;
            contents
                .lines()
                .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
                .collect()
        }
    }

    #[cfg(unix)]
    fn shell_double_quoted_json(value: &serde_json::Value) -> Result<String, String> {
        serde_json::to_string(value)
            .map_err(|error| error.to_string())
            .map(|value| {
                value
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('$', "\\$")
                    .replace('`', "\\`")
            })
    }

    #[cfg(unix)]
    pub(crate) fn stateful_fake_codex(
        execution_root: &str,
        thread_source: &str,
        thread_id: &str,
        uncertain_start: bool,
    ) -> Result<FakeCodex, String> {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let executable = directory.path().join("codex");
        let thread = |id: &str| {
            json!({
                "id": id,
                "sessionId": "session-phase1c",
                "cliVersion": "0.145.0",
                "preview": "Phase 1C fixture",
                "ephemeral": false,
                "modelProvider": "openai",
                "createdAt": 1723600000,
                "updatedAt": 1723600001,
                "cwd": execution_root,
                "source": "appServer",
                "status": { "type": "idle" },
                "threadSource": thread_source,
                "turns": []
            })
        };
        let baseline_page = shell_double_quoted_json(&json!({
            "data": [thread("thread-before")],
            "nextCursor": null
        }))?;
        let recovery_page = shell_double_quoted_json(&json!({
            "data": [thread("thread-before"), thread(thread_id)],
            "nextCursor": null
        }))?;
        let descendant_page = shell_double_quoted_json(&json!({
            "data": [
                thread("thread-before"),
                thread(thread_id),
                {
                    "id": "thread-descendant",
                    "sessionId": "session-descendant",
                    "cliVersion": "0.145.0",
                    "preview": "Late descendant fixture",
                    "ephemeral": false,
                    "modelProvider": "openai",
                    "createdAt": 1723600002,
                    "updatedAt": 1723600003,
                    "cwd": execution_root,
                    "source": { "subAgent": "review" },
                    "status": { "type": "idle" },
                    "parentThreadId": thread_id,
                    "turns": []
                }
            ],
            "nextCursor": null
        }))?;
        let empty_page = shell_double_quoted_json(&json!({
            "data": [],
            "nextCursor": null,
            "backwardsCursor": null
        }))?;
        let archived_page = shell_double_quoted_json(&json!({
            "data": [thread(thread_id)],
            "nextCursor": null,
            "backwardsCursor": null
        }))?;
        let empty_loaded = shell_double_quoted_json(&json!({
            "data": [],
            "nextCursor": null
        }))?;
        let opened = shell_double_quoted_json(&json!({
            "thread": thread(thread_id),
            "instructionSources": [],
            "model": "gpt-test",
            "reasoningEffort": "high"
        }))?;
        let read = shell_double_quoted_json(&json!({ "thread": thread(thread_id) }))?;
        let archived_notification = shell_double_quoted_json(&json!({
            "method": "thread/archived",
            "params": { "threadId": thread_id }
        }))?;
        let unarchived_notification = shell_double_quoted_json(&json!({
            "method": "thread/unarchived",
            "params": { "threadId": thread_id }
        }))?;
        let approval_request = shell_double_quoted_json(&json!({
            "id": "approval-command",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": thread_id,
                "turnId": "turn-approval",
                "itemId": "item-command",
                "startedAtMs": 1723600011000_u64,
                "command": "cargo test",
                "cwd": execution_root,
                "reason": "Run the focused tests"
            }
        }))?;
        let turn = shell_double_quoted_json(&json!({
            "turn": { "id": "turn-phase1c", "status": "inProgress" }
        }))?;
        let start_reply = if uncertain_start {
            "printf '%s\\n' \"{\\\"id\\\":$request_id,\\\"error\\\":{\\\"code\\\":-32000,\\\"message\\\":\\\"simulated uncertain start\\\"}}\""
                .to_string()
        } else {
            format!("printf '%s\\n' \"{{\\\"id\\\":$request_id,\\\"result\\\":{opened}}}\"")
        };
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
: > "$0.started"
IFS= read -r initialize
printf '%s\n' '{{"id":1,"result":{{"userAgent":"codex-phase1c","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}}}'
IFS= read -r initialized
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
        *'"method":"thread/list"'*)
      case "$line" in
        *'"archived":true'*)
          if [ -f "$0.archived" ]; then
            printf '%s\n' "{{\"id\":$request_id,\"result\":{archived_page}}}"
          else
            printf '%s\n' "{{\"id\":$request_id,\"result\":{empty_page}}}"
          fi
          ;;
        *)
          if [ -f "$0.archived" ]; then
            printf '%s\n' "{{\"id\":$request_id,\"result\":{baseline_page}}}"
          elif [ -f "$0.created" ]; then
            if [ -f "$0.spawn-descendant-after-terminal" ] && [ -f "$0.terminal-drained" ]; then
              printf '%s\n' "{{\"id\":$request_id,\"result\":{descendant_page}}}"
            else
              printf '%s\n' "{{\"id\":$request_id,\"result\":{recovery_page}}}"
            fi
          else
            printf '%s\n' "{{\"id\":$request_id,\"result\":{baseline_page}}}"
          fi
          ;;
      esac
      ;;
    *'"method":"thread/loaded/list"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{empty_loaded}}}"
      ;;
    *'"method":"thread/start"'*)
      : > "$0.created"
      {start_reply}
      ;;
    *'"method":"thread/read"'*)
      if [ -f "$0.request-approval" ]; then
        rm -f "$0.request-approval"
        printf '%s\n' "{approval_request}"
      fi
      printf '%s\n' "{{\"id\":$request_id,\"result\":{read}}}"
      ;;
    *'"method":"thread/archive"'*)
      if [ ! -f "$0.terminal-drained" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32001,\"message\":\"terminal was not drained before archive\"}}}}"
      elif [ -f "$0.fail-archive" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32002,\"message\":\"simulated uncertain archive\"}}}}"
      else
        : > "$0.archived"
        if [ -f "$0.fail-archive-commit" ]; then
          commit_dir=$(cat "$0.fail-archive-commit")
          chmod 500 "$commit_dir"
        fi
        printf '%s\n' "{archived_notification}"
        printf '%s\n' "{{\"id\":$request_id,\"result\":{{}}}}"
      fi
      ;;
    *'"method":"thread/unarchive"'*)
      rm -f "$0.archived"
      printf '%s\n' "{unarchived_notification}"
      printf '%s\n' "{{\"id\":$request_id,\"result\":{read}}}"
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{opened}}}"
      ;;
    *'"method":"turn/start"'*)
      if [ -f "$0.block-turn-start" ]; then
        : > "$0.turn-start-admitted"
        while [ ! -f "$0.release-turn-start" ]; do
          sleep 0.01
        done
      fi
      if [ -f "$0.fail-turn-start" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32003,\"message\":\"simulated uncertain turn start\"}}}}"
      else
        printf '%s\n' "{{\"id\":$request_id,\"result\":{turn}}}"
      fi
      ;;
    *)
      printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32601,\"message\":\"unexpected method\"}}}}"
      ;;
  esac
done
"#
        );
        fs::write(&executable, script).map_err(|error| error.to_string())?;
        let mut permissions = fs::metadata(&executable)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).map_err(|error| error.to_string())?;
        Ok(FakeCodex {
            _directory: directory,
            executable,
        })
    }

    pub(crate) struct TestRepository {
        _directory: tempfile::TempDir,
        root: PathBuf,
    }

    fn test_git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
        let executable = crate::managed_agents::resolve_command("git")
            .ok_or_else(|| "git executable was not found".to_string())?;
        let output = Command::new(executable)
            .arg("--no-pager")
            .args(args)
            .current_dir(cwd)
            .env_remove("GIT_NO_REPLACE_OBJECTS")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|error| format!("failed to run test git: {error}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }

    fn test_git_line(cwd: &Path, args: &[&str]) -> Result<String, String> {
        String::from_utf8(test_git(cwd, args)?)
            .map(|output| output.trim_end_matches(['\r', '\n']).to_string())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn create_test_repository() -> Result<TestRepository, String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = directory.path().join("repository");
        fs::create_dir(&root).map_err(|error| error.to_string())?;
        test_git(&root, &["init", "--initial-branch=main"])?;
        fs::write(root.join("README.md"), "phase 1c\n").map_err(|error| error.to_string())?;
        test_git(&root, &["add", "README.md"])?;
        test_git(
            &root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "phase 1c fixture",
            ],
        )?;
        Ok(TestRepository {
            _directory: directory,
            root,
        })
    }

    pub(crate) fn phase1c_scope(repository_identity: String) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: "community-phase1c".to_string(),
            project_dtag: "project-phase1c".to_string(),
            repository_identity,
        }
    }

    pub(crate) fn persisted_local_binding(
        repository: &TestRepository,
        thread_id: &str,
    ) -> Result<(CodeThreadBindingScope, CodeThreadBinding), String> {
        let descriptor = preflight_execution_root(&repository.root.to_string_lossy(), "HEAD")?;
        let base_ref = String::from_utf8(test_git(&repository.root, &["rev-parse", "HEAD"])?)
            .map_err(|error| error.to_string())?
            .trim()
            .to_string();
        let scope = phase1c_scope(descriptor.repository_identity);
        let binding = CodeThreadBinding {
            community_id: scope.community_id.clone(),
            project_dtag: scope.project_dtag.clone(),
            repository_identity: scope.repository_identity.clone(),
            codex_thread_id: thread_id.to_string(),
            execution_mode: CodeExecutionMode::Local,
            execution_root: descriptor.repository_root,
            base_ref,
            worktree_id: None,
        };
        Ok((scope, binding))
    }

    pub(crate) fn method_count(requests: &[serde_json::Value], method: &str) -> usize {
        requests
            .iter()
            .filter(|request| request["method"] == method)
            .count()
    }

    #[test]
    fn live_events_drop_unbound_threads_and_retry_misses_after_commit() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let execution_root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
        let cache = Mutex::new(EventScopeCache::default());

        assert!(scope_live_event(&store, &cache, runtime_event(Some("thread-a"), 1)).is_none());
        assert!(scope_live_event(&store, &cache, runtime_event(None, 2)).is_none());

        store.upsert(local_binding(execution_root.path(), "thread-a"))?;
        let event = scope_live_event(&store, &cache, runtime_event(Some("thread-a"), 3))
            .ok_or_else(|| "expected the post-commit event to be scoped".to_string())?;

        assert_eq!(event.scope, scope());
        assert_eq!(event.thread_id.as_deref(), Some("thread-a"));
        assert_eq!(event.sequence, 3);
        assert_eq!(
            cache
                .lock()
                .map_err(|error| error.to_string())?
                .by_thread_id
                .len(),
            1
        );
        Ok(())
    }

    #[test]
    fn replay_filters_global_backlog_to_the_requested_bound_thread_ids() {
        let mut approval_in_scope = runtime_event(Some("thread-a"), 4);
        approval_in_scope.runtime_generation = 9;
        approval_in_scope.item_id = Some("approval-a".to_string());
        approval_in_scope.kind = "item/fileChange/requestApproval".to_string();
        approval_in_scope.payload = json!({
            "requestId": "approval-a",
            "approvalKind": "fileChange",
            "request": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "itemId": "approval-a"
            }
        });
        let mut approval_other_scope = approval_in_scope.clone();
        approval_other_scope.thread_id = Some("thread-other-scope".to_string());
        let backlog = CodeRuntimeEventBacklog {
            runtime_generation: 9,
            latest_sequence: 4,
            truncated: true,
            checkpoint: Some(CodeRuntimeEventCheckpoint {
                runtime_generation: 9,
                sequence_watermark: 4,
                active_turns: vec![
                    CodeRuntimeActiveTurnCheckpoint {
                        thread_id: "thread-a".to_string(),
                        turn_id: "turn-a".to_string(),
                        status: "inProgress".to_string(),
                        started_sequence: 1,
                    },
                    CodeRuntimeActiveTurnCheckpoint {
                        thread_id: "thread-other-scope".to_string(),
                        turn_id: "turn-other".to_string(),
                        status: "inProgress".to_string(),
                        started_sequence: 2,
                    },
                ],
                pending_approvals: vec![
                    CodeRuntimeApprovalCheckpoint {
                        event: approval_in_scope,
                        respondable: true,
                    },
                    CodeRuntimeApprovalCheckpoint {
                        event: approval_other_scope,
                        respondable: false,
                    },
                ],
            }),
            events: vec![
                runtime_event(Some("thread-a"), 1),
                runtime_event(Some("thread-other-scope"), 2),
                runtime_event(None, 3),
            ],
        };
        let bound_thread_ids = HashSet::from(["thread-a".to_string()]);

        let scoped = scope_event_backlog(backlog, scope(), &bound_thread_ids);

        assert_eq!(scoped.runtime_generation, 9);
        assert_eq!(scoped.latest_sequence, 4);
        assert!(scoped.truncated);
        assert_eq!(scoped.events.len(), 1);
        assert_eq!(scoped.events[0].scope, scope());
        assert_eq!(scoped.events[0].thread_id.as_deref(), Some("thread-a"));
        let checkpoint = scoped
            .checkpoint
            .expect("truncated scoped replay should retain its checkpoint");
        assert_eq!(checkpoint.runtime_generation, 9);
        assert_eq!(checkpoint.sequence_watermark, 4);
        assert_eq!(checkpoint.active_turns.len(), 1);
        assert_eq!(checkpoint.active_turns[0].thread_id, "thread-a");
        assert_eq!(checkpoint.pending_approvals.len(), 1);
        assert_eq!(
            checkpoint.pending_approvals[0].event.thread_id.as_deref(),
            Some("thread-a")
        );
        assert!(checkpoint.pending_approvals[0].respondable);
    }

    #[test]
    fn reported_thread_root_must_match_the_authoritative_root() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let other = tempfile::tempdir().map_err(|error| error.to_string())?;
        let expected = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();

        validate_thread_identity_and_root(
            "thread-a",
            Some(&root.path().to_string_lossy()),
            "thread-a",
            &expected,
        )?;
        assert!(validate_thread_identity_and_root(
            "thread-a",
            Some(&other.path().to_string_lossy()),
            "thread-a",
            &expected,
        )
        .is_err());
        assert!(
            validate_thread_identity_and_root("thread-b", None, "thread-a", &expected,).is_err()
        );
        Ok(())
    }

    #[test]
    fn recovery_candidate_excludes_baseline_bound_and_wrong_source() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let preparation = recovery_preparation(&root, Some(vec!["thread-before"]));
        let source = code_thread_source(&preparation.preparation_id)?;
        let candidates = vec![
            recovery_candidate("thread-before", Some(&root), Some(&source)),
            recovery_candidate("thread-bound", None, Some(&source)),
            recovery_candidate("thread-foreign", Some(&root), Some("other-client")),
            recovery_candidate(
                "thread-other-preparation",
                Some(&root),
                Some("schoolx-code/77f11a1d-0274-4d40-9b0c-e406e51c64fb"),
            ),
            recovery_candidate("thread-created", Some(&root), Some(&source)),
        ];
        let bound = HashSet::from(["thread-bound".to_string()]);

        let selected = select_recovery_candidate(&preparation, candidates, &bound, &root)?;
        assert_eq!(selected.thread.id, "thread-created");
        Ok(())
    }

    #[test]
    fn recovery_candidate_requires_exactly_one_and_a_durable_baseline() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let preparation = recovery_preparation(&root, Some(Vec::new()));
        let source = code_thread_source(&preparation.preparation_id)?;
        assert!(select_recovery_candidate(
            &preparation,
            vec![recovery_candidate("foreign", Some(&root), None)],
            &HashSet::new(),
            &root,
        )
        .is_err());
        let ambiguous = select_recovery_candidate(
            &preparation,
            vec![
                recovery_candidate("created-a", Some(&root), Some(&source)),
                recovery_candidate("created-b", Some(&root), Some(&source)),
            ],
            &HashSet::new(),
            &root,
        )
        .expect_err("multiple recovery candidates must fail closed");
        assert!(ambiguous.contains("2 possible recovery threads"));

        let legacy = recovery_preparation(&root, None);
        let legacy_error = select_recovery_candidate(
            &legacy,
            vec![recovery_candidate("created", Some(&root), None)],
            &HashSet::new(),
            &root,
        )
        .expect_err("legacy starting preparation must fail closed");
        assert!(legacy_error.contains("predates safe native recovery"));
        Ok(())
    }

    #[test]
    fn recovered_thread_must_retain_the_exact_preparation_source() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let preparation = recovery_preparation(&root, Some(Vec::new()));
        let source = code_thread_source(&preparation.preparation_id)?;

        validate_recovery_source(
            &preparation,
            &recovery_candidate("thread-created", Some(&root), Some(&source)),
        )?;
        assert!(validate_recovery_source(
            &preparation,
            &recovery_candidate(
                "thread-created",
                Some(&root),
                Some("schoolx-code/77f11a1d-0274-4d40-9b0c-e406e51c64fb"),
            ),
        )
        .is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_reads_exact_local_checkout_without_mutating_it() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let literal_magic_path = ":(glob)*.txt";
        fs::write(
            repository.root.join(literal_magic_path),
            "literal baseline\n",
        )
        .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("decoy.txt"), "decoy baseline\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("binary.dat"), [0_u8, 1, 2])
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("deleted.txt"), "deleted baseline\n")
            .map_err(|error| error.to_string())?;
        fs::write(
            repository.root.join("type-change.txt"),
            "regular baseline\n",
        )
        .map_err(|error| error.to_string())?;
        fs::write(
            repository.root.join(".gitattributes"),
            "filtered.txt filter=schoolx-phase1f\n",
        )
        .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("filtered.txt"), "filtered baseline\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "-A"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "literal pathspec fixture",
            ],
        )?;
        let (scope, binding) = persisted_local_binding(&repository, "thread-changes")?;
        CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding.clone())?;
        test_git(
            &repository.root,
            &[
                "config",
                "filter.schoolx-phase1f.clean",
                "sh -c 'touch filter-marker; cat'",
            ],
        )?;
        test_git(
            &repository.root,
            &["config", "filter.schoolx-phase1f.required", "true"],
        )?;

        fs::write(repository.root.join("README.md"), "phase 1f changed\n")
            .map_err(|error| error.to_string())?;
        fs::write(
            repository.root.join(literal_magic_path),
            "literal changed\n",
        )
        .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("decoy.txt"), "decoy changed\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("binary.dat"), [0_u8, 1, 3])
            .map_err(|error| error.to_string())?;
        fs::remove_file(repository.root.join("deleted.txt")).map_err(|error| error.to_string())?;
        fs::remove_file(repository.root.join("type-change.txt"))
            .map_err(|error| error.to_string())?;
        symlink("README.md", repository.root.join("type-change.txt"))
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("filtered.txt"), "filtered changed\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("staged.txt"), "staged\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "staged.txt"])?;
        fs::write(repository.root.join("untracked.txt"), "one\ntwo\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("untracked.bin"), [0_u8, 4, 5])
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("empty.txt"), b"").map_err(|error| error.to_string())?;
        let before = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
        let filter_marker = repository.root.join("filter-marker");
        if filter_marker.exists() {
            fs::remove_file(&filter_marker).map_err(|error| error.to_string())?;
        }
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let limited = crate::commands::project_git_diff::current_changes_from_repo_with_limit(
            &repository.root,
            &auth,
            &binding.base_ref,
            &binding.repository_identity,
            3,
        )?;
        assert_eq!(limited.total_files, 11);
        assert_eq!(limited.files.len(), 3);
        assert!(limited.files_truncated);
        assert_eq!(
            limited
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![":(glob)*.txt", "README.md", "binary.dat"]
        );
        assert_eq!(
            limited.additions,
            limited
                .files
                .iter()
                .map(|file| file.additions)
                .sum::<usize>()
        );
        assert_eq!(
            limited.deletions,
            limited
                .files
                .iter()
                .map(|file| file.deletions)
                .sum::<usize>()
        );

        let changes = thread_changes_native(
            CodeThreadChangesInput {
                scope: scope.clone(),
                thread_id: "thread-changes".to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )?;

        assert_eq!(
            changes
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                ":(glob)*.txt",
                "README.md",
                "binary.dat",
                "decoy.txt",
                "deleted.txt",
                "empty.txt",
                "filtered.txt",
                "staged.txt",
                "type-change.txt",
                "untracked.bin",
                "untracked.txt"
            ]
        );
        assert_eq!(changes.total_files, 11);
        assert!(!changes.files_truncated);
        let literal_patch = changes
            .files
            .iter()
            .find(|file| file.path == literal_magic_path)
            .map(|file| file.patch.as_str())
            .ok_or_else(|| "literal pathspec change was missing".to_string())?;
        assert!(literal_patch.contains("+literal changed"));
        assert!(!literal_patch.contains("+decoy changed"));
        assert!(!filter_marker.exists());
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "README.md")
            .is_some_and(|file| {
                file.status == CodeThreadChangeStatus::Modified
                    && !file.binary
                    && file.additions == 1
                    && file.deletions == 1
            }));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "staged.txt")
            .is_some_and(|file| file.status == CodeThreadChangeStatus::Added && !file.binary));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "deleted.txt")
            .is_some_and(|file| file.status == CodeThreadChangeStatus::Deleted && !file.binary));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "type-change.txt")
            .is_some_and(|file| {
                file.status == CodeThreadChangeStatus::TypeChanged && !file.binary
            }));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "binary.dat")
            .is_some_and(|file| {
                file.status == CodeThreadChangeStatus::Modified
                    && file.binary
                    && file.additions == 0
                    && file.deletions == 0
            }));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "untracked.txt")
            .is_some_and(|file| {
                file.status == CodeThreadChangeStatus::Untracked
                    && !file.binary
                    && file.patch.contains("+one")
                    && file.additions == 2
            }));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "untracked.bin")
            .is_some_and(|file| {
                file.status == CodeThreadChangeStatus::Untracked
                    && file.binary
                    && file.additions == 0
                    && file.deletions == 0
            }));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "empty.txt")
            .is_some_and(|file| {
                file.status == CodeThreadChangeStatus::Untracked
                    && !file.binary
                    && file.additions == 0
                    && file.deletions == 0
            }));
        assert_eq!(
            changes.additions,
            changes
                .files
                .iter()
                .map(|file| file.additions)
                .sum::<usize>()
        );
        assert_eq!(
            changes.deletions,
            changes
                .files
                .iter()
                .map(|file| file.deletions)
                .sum::<usize>()
        );
        assert_eq!(
            test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?,
            before
        );

        let mut wrong_scope = scope.clone();
        wrong_scope.community_id = "other-community".to_string();
        assert!(thread_changes_native(
            CodeThreadChangesInput {
                scope: wrong_scope,
                thread_id: "thread-changes".to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )
        .is_err());
        assert!(thread_changes_native(
            CodeThreadChangesInput {
                scope,
                thread_id: "other-thread".to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )
        .is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_ignores_replace_refs_for_immutable_base() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let (scope, binding) = persisted_local_binding(&repository, "thread-replaced-base-object")?;
        let base_commit = binding.base_ref.clone();
        CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding)?;

        fs::write(repository.root.join("README.md"), "replacement view\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "README.md"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "replacement commit fixture",
            ],
        )?;
        let replacement_commit = test_git_line(&repository.root, &["rev-parse", "HEAD"])?;
        assert_ne!(replacement_commit, base_commit);

        test_git(&repository.root, &["checkout", "--detach", &base_commit])?;
        fs::write(repository.root.join("README.md"), "replacement view\n")
            .map_err(|error| error.to_string())?;
        test_git(
            &repository.root,
            &["replace", &base_commit, &replacement_commit],
        )?;

        let unprotected_diff = test_git(
            &repository.root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                &base_commit,
                "--",
                "README.md",
            ],
        )?;
        assert!(
            unprotected_diff.is_empty(),
            "Git replacement fixture did not hide the immutable-base change"
        );
        let literal_diff = String::from_utf8(test_git(
            &repository.root,
            &[
                "--no-replace-objects",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                &base_commit,
                "--",
                "README.md",
            ],
        )?)
        .map_err(|error| error.to_string())?;
        assert!(literal_diff.contains("-phase 1c"));
        assert!(literal_diff.contains("+replacement view"));

        let status_before = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let changes = thread_changes_native(
            CodeThreadChangesInput {
                scope,
                thread_id: "thread-replaced-base-object".to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )?;

        assert_eq!(changes.files.len(), 1);
        assert!(changes.files.first().is_some_and(|file| {
            file.path == "README.md"
                && file.additions == 1
                && file.deletions == 1
                && file.patch.contains("-phase 1c")
                && file.patch.contains("+replacement view")
        }));
        assert_eq!(changes.additions, 1);
        assert_eq!(changes.deletions, 1);
        assert_eq!(
            test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?,
            status_before
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_reads_persisted_managed_worktree_from_immutable_base() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let repository_root = repository.root.to_string_lossy().into_owned();
        let inspected = preflight_execution_root(&repository_root, "main")?;
        let scope = phase1c_scope(inspected.repository_identity.clone());
        let base_commit = test_git_line(&repository.root, &["rev-parse", "main"])?;
        let binding_lock = Mutex::new(());

        let prepared = prepare_worktree_native(
            CodeWorktreePrepareCommandInput {
                scope: scope.clone(),
                repository_root,
                base_ref: "main".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            app_data.path(),
            nest.path(),
            &binding_lock,
        )?;
        assert_eq!(prepared.worktree.repository, inspected);
        assert_eq!(
            prepared.worktree.descriptor.execution_mode,
            CodeExecutionMode::Worktree
        );
        assert_eq!(
            prepared.worktree.descriptor.repository_identity,
            scope.repository_identity
        );
        assert_eq!(prepared.worktree.descriptor.base_ref, base_commit);
        let worktree_id = prepared
            .worktree
            .descriptor
            .worktree_id
            .as_deref()
            .ok_or_else(|| "managed preparation did not issue a worktree id".to_string())?;
        let execution_root = PathBuf::from(&prepared.worktree.descriptor.execution_root);
        let expected_root = nest
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join("WORKTREES")
            .join(&scope.repository_identity)
            .join(worktree_id);
        assert_eq!(execution_root, expected_root);
        let git_entry =
            fs::symlink_metadata(execution_root.join(".git")).map_err(|error| error.to_string())?;
        assert!(git_entry.is_file());
        assert!(!git_entry.file_type().is_symlink());

        let source_common_dir = PathBuf::from(test_git_line(
            &repository.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
        let managed_common_dir = PathBuf::from(test_git_line(
            &execution_root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
        assert_eq!(managed_common_dir, source_common_dir);
        assert_eq!(
            crate::code_workspace::repository_identity(&managed_common_dir)?,
            scope.repository_identity
        );

        let thread_id = "thread-managed-changes";
        let binding = {
            let _guard = lock_bindings(&binding_lock)?;
            let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
            store.claim_preparation_for_start(&scope, &prepared.preparation_id, Vec::new())?;
            store.commit_preparation_binding(&scope, &prepared.preparation_id, thread_id)?
        };
        let reloaded_store = CodeThreadBindingStore::for_app_data(app_data.path())?;
        let reloaded = reloaded_store.load()?;
        assert!(reloaded.preparations.is_empty());
        assert_eq!(reloaded.bindings, vec![binding.clone()]);
        let (_, target_ref) = reloaded_store
            .binding_merge_authority(&CodeThreadBindingLookupInput {
                scope: scope.clone(),
                codex_thread_id: thread_id.to_string(),
            })?
            .ok_or_else(|| "committed managed binding disappeared".to_string())?;
        assert_eq!(target_ref.as_deref(), Some("refs/heads/main"));
        assert_eq!(binding.execution_mode, CodeExecutionMode::Worktree);
        assert_eq!(binding.execution_root, execution_root.to_string_lossy());
        assert_eq!(binding.repository_identity, scope.repository_identity);
        assert_eq!(binding.base_ref, base_commit);
        assert_eq!(binding.worktree_id.as_deref(), Some(worktree_id));

        fs::write(repository.root.join("README.md"), "advanced main\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "README.md"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "advance mutable main",
            ],
        )?;
        let advanced_main = test_git_line(&repository.root, &["rev-parse", "main"])?;
        assert_ne!(advanced_main, base_commit);

        fs::write(execution_root.join("README.md"), "advanced main\n")
            .map_err(|error| error.to_string())?;
        fs::write(
            execution_root.join("managed-only.txt"),
            "managed one\nmanaged two\n",
        )
        .map_err(|error| error.to_string())?;
        fs::write(
            repository.root.join("source-decoy.txt"),
            "must not appear in managed Changes\n",
        )
        .map_err(|error| error.to_string())?;
        let source_status_before = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
        let managed_status_before = test_git(&execution_root, &["status", "--porcelain=v1", "-z"])?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;

        let changes = thread_changes_native(
            CodeThreadChangesInput {
                scope: scope.clone(),
                thread_id: thread_id.to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )?;

        assert_eq!(
            changes
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["README.md", "managed-only.txt"]
        );
        assert!(changes
            .files
            .iter()
            .all(|file| file.path != "source-decoy.txt"));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "README.md")
            .is_some_and(|file| {
                file.additions == 1
                    && file.deletions == 1
                    && file.patch.contains("-phase 1c")
                    && file.patch.contains("+advanced main")
            }));
        assert!(changes
            .files
            .iter()
            .find(|file| file.path == "managed-only.txt")
            .is_some_and(|file| {
                file.additions == 2
                    && file.deletions == 0
                    && file.patch.contains("+managed one")
                    && file.patch.contains("+managed two")
            }));
        assert_eq!(changes.additions, 3);
        assert_eq!(changes.deletions, 1);
        let source_status_after = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
        let managed_status_after = test_git(&execution_root, &["status", "--porcelain=v1", "-z"])?;
        assert_eq!(source_status_after, source_status_before);
        assert_eq!(managed_status_after, managed_status_before);

        let mut wrong_scope = scope;
        wrong_scope.project_dtag = "other-project".to_string();
        assert!(thread_changes_native(
            CodeThreadChangesInput {
                scope: wrong_scope,
                thread_id: thread_id.to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )
        .is_err());
        assert!(thread_changes_native(
            CodeThreadChangesInput {
                scope: binding.scope(),
                thread_id: "other-thread".to_string(),
            },
            app_data.path(),
            nest.path(),
            &auth,
        )
        .is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_root_swap_after_pin_fails_closed() -> Result<(), String> {
        let repository = create_test_repository()?;
        let (_, binding) = persisted_local_binding(&repository, "thread-swap-after-pin")?;
        fs::write(
            repository.root.join("README.md"),
            "original pinned change\n",
        )
        .map_err(|error| error.to_string())?;
        let moved = repository.root.with_file_name("moved-pinned-repository");
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;

        let result = crate::commands::project_git_diff::current_changes_from_repo_after_pin(
            &repository.root,
            &auth,
            &binding.base_ref,
            &binding.repository_identity,
            || {
                fs::rename(&repository.root, &moved).map_err(|error| error.to_string())?;
                fs::create_dir(&repository.root).map_err(|error| error.to_string())?;
                test_git(&repository.root, &["init", "--initial-branch=main"])?;
                fs::write(
                    repository.root.join("replacement.txt"),
                    "must not be read\n",
                )
                .map_err(|error| error.to_string())?;
                Ok(())
            },
        );

        let error = result
            .err()
            .ok_or_else(|| "root replacement unexpectedly produced a diff".to_string())?;
        assert!(error.contains("moved or was replaced"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn pinned_git_directory_rejects_same_base_dot_git_replacement() -> Result<(), String> {
        let repository = create_test_repository()?;
        let (_, binding) = persisted_local_binding(&repository, "thread-dot-git-swap")?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let original_git = repository.root.join(".git-original");
        let replacement = repository.root.with_file_name("replacement-clone");

        let result = crate::commands::project_git_diff::current_changes_from_repo_after_pin(
            &repository.root,
            &auth,
            &binding.base_ref,
            &binding.repository_identity,
            || {
                fs::rename(repository.root.join(".git"), &original_git)
                    .map_err(|error| error.to_string())?;
                let original_git_value = original_git.to_string_lossy().into_owned();
                let replacement_value = replacement.to_string_lossy().into_owned();
                test_git(
                    repository
                        .root
                        .parent()
                        .ok_or_else(|| "test repository had no parent".to_string())?,
                    &[
                        "clone",
                        "--no-hardlinks",
                        "--no-checkout",
                        &original_git_value,
                        &replacement_value,
                    ],
                )?;
                fs::rename(replacement.join(".git"), repository.root.join(".git"))
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        );

        let error = result
            .err()
            .ok_or_else(|| "same-base .git replacement was accepted".to_string())?;
        assert!(error.contains(".git entry"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn pinned_managed_gitfile_replacement_is_rejected() -> Result<(), String> {
        let repository = create_test_repository()?;
        let linked_root = repository.root.with_file_name("linked-worktree");
        let linked_value = linked_root.to_string_lossy().into_owned();
        test_git(
            &repository.root,
            &["worktree", "add", "--detach", &linked_value, "HEAD"],
        )?;
        let descriptor = preflight_execution_root(&linked_value, "HEAD")?;
        let head_output = test_git(&linked_root, &["rev-parse", "HEAD"])?;
        let base_ref = String::from_utf8(head_output)
            .map_err(|error| error.to_string())?
            .trim()
            .to_string();
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let original_gitfile = linked_root.join(".git-original");

        let result = crate::commands::project_git_diff::current_changes_from_repo_after_pin(
            &linked_root,
            &auth,
            &base_ref,
            &descriptor.repository_identity,
            || {
                let contents =
                    fs::read(linked_root.join(".git")).map_err(|error| error.to_string())?;
                fs::rename(linked_root.join(".git"), &original_gitfile)
                    .map_err(|error| error.to_string())?;
                fs::write(linked_root.join(".git"), contents).map_err(|error| error.to_string())?;
                Ok(())
            },
        );

        let error = result
            .err()
            .ok_or_else(|| "managed gitfile replacement was accepted".to_string())?;
        assert!(error.contains(".git entry"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_classifies_a_real_index_conflict_as_unmerged() -> Result<(), String> {
        let repository = create_test_repository()?;
        fs::write(repository.root.join("conflict.txt"), "base\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "conflict.txt"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "conflict base",
            ],
        )?;
        let (_, binding) = persisted_local_binding(&repository, "thread-conflict")?;

        test_git(&repository.root, &["checkout", "-b", "divergent"])?;
        fs::write(repository.root.join("conflict.txt"), "theirs\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "conflict.txt"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "divergent change",
            ],
        )?;

        test_git(&repository.root, &["checkout", "main"])?;
        fs::write(repository.root.join("conflict.txt"), "ours\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "conflict.txt"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "main change",
            ],
        )?;
        assert!(test_git(&repository.root, &["merge", "divergent"]).is_err());

        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let changes = crate::commands::project_git_diff::current_changes_from_repo_with_limit(
            &repository.root,
            &auth,
            &binding.base_ref,
            &binding.repository_identity,
            10,
        )?;

        assert_eq!(changes.total_files, 1);
        assert!(!changes.files_truncated);
        assert!(changes.files.first().is_some_and(|file| {
            file.path == "conflict.txt"
                && file.status
                    == crate::commands::project_git_diff::CurrentRepoChangeStatus::Unmerged
        }));
        assert!(String::from_utf8(test_git(
            &repository.root,
            &["diff", "--name-only", "--diff-filter=U"]
        )?)
        .map_err(|error| error.to_string())?
        .contains("conflict.txt"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_retries_when_an_untracked_file_disappears_before_open() -> Result<(), String>
    {
        let repository = create_test_repository()?;
        let (_, binding) = persisted_local_binding(&repository, "thread-untracked-disappears")?;
        let disappearing = repository.root.join("disappearing.txt");
        fs::write(&disappearing, "temporary\n").map_err(|error| error.to_string())?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let mut hook_calls = 0;

        let changes =
            crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
                &repository.root,
                &auth,
                &binding.base_ref,
                &binding.repository_identity,
                || {
                    hook_calls += 1;
                    if hook_calls == 1 {
                        fs::remove_file(&disappearing).map_err(|error| error.to_string())?;
                    }
                    Ok(())
                },
            )?;

        assert_eq!(hook_calls, 2);
        assert_eq!(changes.total_files, 0);
        assert!(changes.files.is_empty());
        assert!(!changes.files_truncated);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_retries_one_drifted_inventory_once() -> Result<(), String> {
        let repository = create_test_repository()?;
        let (_, binding) = persisted_local_binding(&repository, "thread-transient-drift")?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let mut hook_calls = 0;

        let changes =
            crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
                &repository.root,
                &auth,
                &binding.base_ref,
                &binding.repository_identity,
                || {
                    hook_calls += 1;
                    if hook_calls == 1 {
                        fs::write(repository.root.join("appeared.txt"), "appeared\n")
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(())
                },
            )?;

        assert_eq!(hook_calls, 2);
        assert_eq!(changes.total_files, 1);
        assert!(!changes.files_truncated);
        assert!(changes.files.first().is_some_and(|file| {
            file.path == "appeared.txt"
                && file.status
                    == crate::commands::project_git_diff::CurrentRepoChangeStatus::Untracked
        }));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn thread_changes_reports_clear_error_after_repeated_inventory_drift() -> Result<(), String> {
        let repository = create_test_repository()?;
        let (_, binding) = persisted_local_binding(&repository, "thread-repeated-drift")?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let mut hook_calls = 0;

        let error =
            crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
                &repository.root,
                &auth,
                &binding.base_ref,
                &binding.repository_identity,
                || {
                    hook_calls += 1;
                    fs::write(
                        repository.root.join(format!("drift-{hook_calls}.txt")),
                        "drift\n",
                    )
                    .map_err(|error| error.to_string())
                },
            )
            .expect_err("two consecutive inventory drifts must fail closed");

        assert_eq!(hook_calls, 2);
        assert_eq!(
            error,
            "SchoolX Code Changes changed during inspection; retry after the workspace settles"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn untracked_ancestor_symlink_swap_cannot_read_outside_root() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let repository = create_test_repository()?;
        let (_, binding) = persisted_local_binding(&repository, "thread-untracked-swap")?;
        let subdirectory = repository.root.join("sub");
        fs::create_dir(&subdirectory).map_err(|error| error.to_string())?;
        fs::write(subdirectory.join("file.txt"), "inside repository\n")
            .map_err(|error| error.to_string())?;
        let moved_subdirectory = repository.root.join("original-sub");
        let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
        let sentinel = "outside-sentinel-must-not-be-read";
        fs::write(outside.path().join("file.txt"), sentinel).map_err(|error| error.to_string())?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;

        let result =
            crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
                &repository.root,
                &auth,
                &binding.base_ref,
                &binding.repository_identity,
                || {
                    fs::rename(&subdirectory, &moved_subdirectory)
                        .map_err(|error| error.to_string())?;
                    symlink(outside.path(), &subdirectory).map_err(|error| error.to_string())?;
                    Ok(())
                },
            );

        assert!(result.is_err());
        assert!(!format!("{result:?}").contains(sentinel));
        Ok(())
    }

    #[test]
    fn thread_changes_rejects_missing_or_replaced_execution_root() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let (scope, binding) = persisted_local_binding(&repository, "thread-replaced")?;
        CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding)?;
        let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
        let original = repository.root.with_file_name("original-repository");
        fs::rename(&repository.root, &original).map_err(|error| error.to_string())?;

        let input = CodeThreadChangesInput {
            scope,
            thread_id: "thread-replaced".to_string(),
        };
        assert!(
            thread_changes_native(input.clone(), app_data.path(), nest.path(), &auth,).is_err()
        );

        fs::create_dir(&repository.root).map_err(|error| error.to_string())?;
        test_git(&repository.root, &["init", "--initial-branch=main"])?;
        fs::write(repository.root.join("README.md"), "replacement\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "README.md"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "replacement fixture",
            ],
        )?;
        assert!(thread_changes_native(input, app_data.path(), nest.path(), &auth).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn native_binding_survives_process_restart_list_resume_and_turn() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let repository_root = repository.root.to_string_lossy().into_owned();
        let repository_descriptor = preflight_execution_root(&repository_root, "HEAD")?;
        let scope = phase1c_scope(repository_descriptor.repository_identity);
        let binding_lock = Mutex::new(());
        let lifecycle_authority = std::sync::atomic::AtomicBool::new(true);

        let prepared = prepare_worktree_native(
            CodeWorktreePrepareCommandInput {
                scope: scope.clone(),
                repository_root,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Local,
            },
            app_data.path(),
            nest.path(),
            &binding_lock,
        )?;
        let preparation_id = prepared.preparation_id.clone();
        let execution_root = prepared.worktree.descriptor.execution_root.clone();
        let thread_source = code_thread_source(&preparation_id)?;
        let fake = stateful_fake_codex(&execution_root, &thread_source, "thread-phase1c", false)?;
        let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());

        let first = runtime.start(Arc::new(|_| {}))?;
        let opened = start_thread_native(
            CodeThreadStartInput {
                scope: scope.clone(),
                preparation_id: preparation_id.clone(),
                model: None,
            },
            app_data.path(),
            nest.path(),
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )
        .map_err(|error| error.message)?;
        assert_eq!(opened.binding.codex_thread_id, "thread-phase1c");
        assert_eq!(opened.binding.execution_root, execution_root);

        let reloaded = CodeThreadBindingStore::for_app_data(app_data.path())?.load()?;
        assert!(reloaded.preparations.is_empty());
        assert_eq!(reloaded.bindings, vec![opened.binding.clone()]);

        runtime.stop()?;
        let second = runtime.start(Arc::new(|_| {}))?;
        assert!(second.generation > first.generation);
        crate::commands::code_thread_lifecycle::invalidate_lifecycle_authority(
            &lifecycle_authority,
        );

        let page = list_threads_native(
            CodeThreadListInput {
                scope: scope.clone(),
            },
            app_data.path(),
            nest.path(),
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )?;
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].binding, opened.binding);
        assert_eq!(
            page.data[0]
                .thread
                .as_ref()
                .and_then(|thread| thread.cwd.as_deref()),
            Some(execution_root.as_str())
        );
        assert!(page.data[0].unavailable.is_none());

        let resumed = resume_thread_native(
            CodeThreadResumeInput {
                scope: scope.clone(),
                thread_id: "thread-phase1c".to_string(),
                model: None,
            },
            app_data.path(),
            nest.path(),
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )?;
        assert_eq!(resumed.binding, opened.binding);
        assert_eq!(resumed.thread.cwd.as_deref(), Some(execution_root.as_str()));

        let turn = start_turn_native(
            CodeTurnStartInput {
                scope,
                thread_id: "thread-phase1c".to_string(),
                prompt: "Run the Phase 1C tests".to_string(),
                model: None,
                effort: None,
            },
            app_data.path(),
            nest.path(),
            &runtime,
            &binding_lock,
            &lifecycle_authority,
        )?;
        assert_eq!(turn.id, "turn-phase1c");
        runtime.stop()?;

        let requests = fake.recorded_requests()?;
        assert_eq!(method_count(&requests, "thread/list"), 3);
        assert_eq!(method_count(&requests, "thread/loaded/list"), 2);
        assert_eq!(method_count(&requests, "thread/start"), 1);
        assert_eq!(method_count(&requests, "thread/read"), 1);
        assert_eq!(method_count(&requests, "thread/resume"), 1);
        assert_eq!(method_count(&requests, "turn/start"), 1);
        for request in &requests {
            let params = &request["params"];
            assert!(params.get("scope").is_none());
            assert!(params.get("workspaceRoot").is_none());
            assert!(params.get("runtimeWorkspaceRoots").is_none());
        }
        let turn_request = requests
            .iter()
            .find(|request| request["method"] == "turn/start")
            .ok_or_else(|| "turn/start fixture request was not recorded".to_string())?;
        assert_eq!(turn_request["params"]["cwd"], execution_root);
        assert_eq!(
            turn_request["params"]["sandboxPolicy"]["writableRoots"],
            json!([execution_root])
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn uncertain_start_survives_store_reload_and_exact_recovery() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let repository_root = repository.root.to_string_lossy().into_owned();
        let repository_descriptor = preflight_execution_root(&repository_root, "HEAD")?;
        let scope = phase1c_scope(repository_descriptor.repository_identity);
        let binding_lock = Mutex::new(());
        let lifecycle_authority = std::sync::atomic::AtomicBool::new(true);
        let prepared = prepare_worktree_native(
            CodeWorktreePrepareCommandInput {
                scope: scope.clone(),
                repository_root,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Local,
            },
            app_data.path(),
            nest.path(),
            &binding_lock,
        )?;
        let preparation_id = prepared.preparation_id.clone();
        let execution_root = prepared.worktree.descriptor.execution_root.clone();
        let thread_source = code_thread_source(&preparation_id)?;
        let fake = stateful_fake_codex(&execution_root, &thread_source, "thread-recovered", true)?;

        let first_runtime =
            crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
        first_runtime.start(Arc::new(|_| {}))?;
        let error = start_thread_native(
            CodeThreadStartInput {
                scope: scope.clone(),
                preparation_id: preparation_id.clone(),
                model: None,
            },
            app_data.path(),
            nest.path(),
            &first_runtime,
            &binding_lock,
            &lifecycle_authority,
        )
        .expect_err("the fixture must leave thread/start uncertain");
        assert_eq!(error.code, "threadStartUncertain");
        assert!(fake.created_marker().is_file());

        let claimed = CodeThreadBindingStore::for_app_data(app_data.path())?.load()?;
        assert!(claimed.bindings.is_empty());
        assert_eq!(claimed.preparations.len(), 1);
        assert_eq!(
            claimed.preparations[0].state,
            CodeThreadPreparationState::Starting
        );
        assert_eq!(
            claimed.preparations[0].recovery_thread_baseline,
            Some(vec!["thread-before".to_string()])
        );
        first_runtime.stop()?;
        drop(first_runtime);

        // Reconstruct both the process owner and filesystem-backed store to
        // model the native restart boundary rather than retaining memory state.
        let reloaded = CodeThreadBindingStore::for_app_data(app_data.path())?.load()?;
        assert_eq!(
            reloaded.preparations[0].state,
            CodeThreadPreparationState::Starting
        );
        let second_runtime =
            crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
        second_runtime.start(Arc::new(|_| {}))?;
        let terminal_manager = crate::code_workspace::CodeTerminalManager::new();
        let recovered = recover_thread_binding_native(
            CodeThreadBindingRecoverInput {
                scope: scope.clone(),
                preparation_id: preparation_id.clone(),
                model: None,
            },
            app_data.path(),
            nest.path(),
            &second_runtime,
            &terminal_manager,
            &binding_lock,
            &lifecycle_authority,
        )?;
        assert_eq!(recovered.binding.codex_thread_id, "thread-recovered");
        assert_eq!(recovered.binding.execution_root, execution_root);
        assert_eq!(recovered.thread.id, "thread-recovered");
        assert_eq!(
            recovered.thread.cwd.as_deref(),
            Some(execution_root.as_str())
        );
        second_runtime.stop()?;

        let committed = CodeThreadBindingStore::for_app_data(app_data.path())?.load()?;
        assert!(committed.preparations.is_empty());
        assert_eq!(committed.bindings, vec![recovered.binding]);

        let requests = fake.recorded_requests()?;
        assert_eq!(method_count(&requests, "thread/start"), 1);
        assert_eq!(method_count(&requests, "thread/list"), 2);
        assert_eq!(method_count(&requests, "thread/loaded/list"), 2);
        assert_eq!(method_count(&requests, "thread/read"), 1);
        assert_eq!(method_count(&requests, "thread/resume"), 1);
        Ok(())
    }

    #[test]
    fn recovery_candidate_requires_a_reported_exact_canonical_root() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let other = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let other = other
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let preparation = recovery_preparation(&root, Some(Vec::new()));
        let source = code_thread_source(&preparation.preparation_id)?;

        assert!(select_recovery_candidate(
            &preparation,
            vec![recovery_candidate("missing-root", None, Some(&source),)],
            &HashSet::new(),
            &root,
        )
        .is_err());
        assert!(select_recovery_candidate(
            &preparation,
            vec![recovery_candidate(
                "wrong-root",
                Some(&other),
                Some(&source),
            )],
            &HashSet::new(),
            &root,
        )
        .is_err());
        Ok(())
    }
}
